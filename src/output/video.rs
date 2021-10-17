use std::{
    fs,
    path::Path,
    process::{exit, Command, Stdio},
    str::FromStr,
};

use crate::input::{get_video_frame_count, PixelFormat, VideoDimensions};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Film,
    Anime,
    Fast,
}

impl Default for Profile {
    fn default() -> Self {
        Profile::Film
    }
}

impl FromStr for Profile {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_ref() {
            "film" => Profile::Film,
            "anime" => Profile::Anime,
            "fast" => Profile::Fast,
            _ => {
                return Err("Invalid profile given".to_owned());
            }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compat {
    Dxva,
    Normal,
    None,
}

impl FromStr for Compat {
    type Err = &'static str;
    fn from_str(input: &str) -> std::result::Result<Self, <Self as std::str::FromStr>::Err> {
        Ok(match input.to_lowercase().as_str() {
            "dxva" => Compat::Dxva,
            "normal" => Compat::Normal,
            "none" => Compat::None,
            _ => {
                return Err("Valid compat values are 'dxva', 'normal', or 'none'");
            }
        })
    }
}

fn create_lossless(input: &Path, dimensions: VideoDimensions) -> Result<(), String> {
    let lossless_filename = input.with_extension("lossless.mkv");
    let mut needs_encode = true;
    if lossless_filename.exists() {
        if let Ok(lossless_frames) = get_video_frame_count(&lossless_filename) {
            if lossless_frames == dimensions.frames {
                needs_encode = false;
                eprintln!("Lossless already exists");
            }
        }
    }

    if needs_encode {
        // Print the info once
        Command::new("vspipe")
            .arg("-i")
            .arg(input)
            .arg("-")
            .status()
            .map_err(|e| format!("Failed to execute vspipe -i: {}", e))?;

        let filename = input.file_name().unwrap().to_str().unwrap();
        let pipe = if filename.ends_with(".vpy") {
            Command::new("vspipe")
                .arg("-c")
                .arg("y4m")
                .arg(input)
                .arg("-")
                .stdout(Stdio::piped())
                .spawn()
                .unwrap()
        } else {
            panic!("Unrecognized input type");
        };
        let mut command = Command::new("nice");
        let status = command
            .arg("ffmpeg")
            .arg("-hide_banner")
            .arg("-loglevel")
            .arg("level+error")
            .arg("-stats")
            .arg("-y")
            .arg("-i")
            .arg("-")
            .arg("-vcodec")
            .arg("ffv1")
            .arg("-level")
            .arg("3")
            .arg("-threads")
            .arg("8")
            .arg("-slices")
            .arg("12")
            .arg(&lossless_filename)
            .stdin(pipe.stdout.unwrap())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|e| format!("Failed to execute ffmpeg: {}", e))?;
        if !status.success() {
            return Err(format!(
                "Failed to execute ffmpeg: Exited with code {:x}",
                status.code().unwrap()
            ));
        }

        if let Ok(lossless_frames) = get_video_frame_count(&lossless_filename) {
            if lossless_frames != dimensions.frames {
                return Err("Incomlete lossless encode".to_string());
            }
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn convert_video_av1an(
    input: &Path,
    encoder: Encoder,
    crf: u8,
    speed: Option<u8>,
    dimensions: VideoDimensions,
    profile: Profile,
    is_hdr: bool,
    keep_lossless: bool,
    lossless_only: bool,
    compat: Compat,
    grain: u8,
) -> Result<(), String> {
    create_lossless(input, dimensions)?;
    if lossless_only {
        exit(0);
    }

    let fps = (dimensions.fps.0 as f32 / dimensions.fps.1 as f32).round() as u32;

    let mut command = Command::new("nice");
    command
        .arg("av1an")
        .arg("-i")
        .arg(input.with_extension("lossless.mkv"))
        .arg("-e")
        .arg(encoder.get_av1an_name())
        .arg("-v")
        .arg(&encoder.get_args_string(crf, speed, dimensions, profile, is_hdr, compat, grain))
        .arg("--sc-method")
        .arg("standard")
        .arg("-x")
        .arg(
            match profile {
                Profile::Film | Profile::Fast => fps * 10,
                Profile::Anime => fps * 15,
            }
            .to_string(),
        )
        .arg("--min-scene-len")
        .arg(
            match profile {
                Profile::Film | Profile::Fast => fps,
                Profile::Anime => fps / 2,
            }
            .to_string(),
        )
        .arg("-w")
        .arg(
            if encoder.has_tiling() && dimensions.width >= 1200 {
                num_cpus::get() / 2 + num_cpus::get() / 8
            } else if encoder.tons_of_lookahead() {
                if dimensions.width >= 1440 {
                    std::cmp::min(6, num_cpus::get())
                } else if dimensions.width >= 1024 {
                    std::cmp::min(10, num_cpus::get())
                } else {
                    num_cpus::get()
                }
            } else {
                num_cpus::get()
            }
            .to_string(),
        )
        .arg("--pix-format")
        .arg(match (dimensions.bit_depth, dimensions.pixel_format) {
            (8, PixelFormat::Yuv420) => "yuv420p".to_string(),
            (8, PixelFormat::Yuv422) => "yuv422p".to_string(),
            (8, PixelFormat::Yuv444) => "yuv444p".to_string(),
            (bd, PixelFormat::Yuv420) => format!("yuv420p{}le", bd),
            (bd, PixelFormat::Yuv422) => format!("yuv422p{}le", bd),
            (bd, PixelFormat::Yuv444) => format!("yuv444p{}le", bd),
        })
        .arg("-r")
        .arg("--verbose")
        .arg("-o")
        .arg(input.with_extension("out.mkv"));
    let status = command
        .status()
        .map_err(|e| format!("Failed to execute av1an: {}", e))?;

    if status.success() {
        if !keep_lossless {
            let _ = fs::remove_file(input.with_extension("lossless.mkv"));
        }
        Ok(())
    } else {
        Err(format!(
            "Failed to execute av1an: Exited with code {:x}",
            status.code().unwrap()
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoder {
    Aom,
    Rav1e,
    X264,
    X265,
}

impl Encoder {
    pub fn get_av1an_name(&self) -> &str {
        match self {
            Encoder::Aom => "aom",
            Encoder::Rav1e => "rav1e",
            Encoder::X264 => "x264",
            Encoder::X265 => "x265",
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn get_args_string(
        &self,
        crf: u8,
        speed: Option<u8>,
        dimensions: VideoDimensions,
        profile: Profile,
        is_hdr: bool,
        compat: Compat,
        grain: u8,
    ) -> String {
        match self {
            Encoder::Aom => build_aom_args_string(crf, speed, dimensions, profile, is_hdr, grain),
            Encoder::Rav1e => build_rav1e_args_string(crf, speed, dimensions, is_hdr),
            Encoder::X264 => build_x264_args_string(crf, dimensions, profile, compat),
            Encoder::X265 => build_x265_args_string(crf, dimensions, profile),
        }
    }

    pub fn has_tiling(&self) -> bool {
        *self == Encoder::Aom || *self == Encoder::Rav1e
    }

    pub fn tons_of_lookahead(&self) -> bool {
        *self == Encoder::X264
    }
}

fn build_aom_args_string(
    crf: u8,
    speed: Option<u8>,
    dimensions: VideoDimensions,
    profile: Profile,
    is_hdr: bool,
    grain: u8,
) -> String {
    format!(
        " --cpu-used={} --end-usage=q --cq-level={} --lag-in-frames=48 --enable-fwd-kf=1 \
         --deltaq-mode={} --enable-chroma-deltaq=1 --quant-b-adapt=1 --enable-qm=1 --qm-min=0 \
         --min-q=1 --enable-keyframe-filtering=0 --arnr-strength=4 --arnr-maxframes={} \
         --sharpness=2 --enable-dnl-denoising=0 --denoise-noise-level={} \
         --disable-trellis-quant=0 --tune=image_perceptual_quality --tile-columns={} \
         --tile-rows=0 --threads=4 --row-mt=0 --color-primaries={} --transfer-characteristics={} \
         --matrix-coefficients={} --disable-kf --kf-max-dist=9999 ",
        speed.unwrap_or(4),
        crf,
        if is_hdr { 5 } else { 1 },
        if profile == Profile::Anime { 15 } else { 7 },
        grain,
        if dimensions.width >= 1200 { 1 } else { 0 },
        if is_hdr {
            "bt2020"
        } else if dimensions.height >= 576 {
            "bt709"
        } else {
            "bt601"
        },
        if is_hdr {
            "bt2020-10bit"
        } else if dimensions.height >= 576 {
            "bt709"
        } else {
            "bt601"
        },
        if is_hdr {
            "bt2020ncl"
        } else if dimensions.height >= 576 {
            "bt709"
        } else {
            "bt601"
        }
    )
}

fn build_rav1e_args_string(
    crf: u8,
    speed: Option<u8>,
    dimensions: VideoDimensions,
    is_hdr: bool,
) -> String {
    format!(
        " --speed={} --quantizer={} --tile-cols={} --tile-rows=0 --primaries={} --transfer={} \
         --matrix={} --no-scene-detection --keyint 0 --min-keyint 0 ",
        speed.unwrap_or(4),
        crf,
        if dimensions.width >= 1200 { 1 } else { 0 },
        if is_hdr {
            "BT2020"
        } else if dimensions.height >= 576 {
            "BT709"
        } else {
            "BT601"
        },
        if is_hdr {
            "BT2020_10Bit"
        } else if dimensions.height >= 576 {
            "BT709"
        } else {
            "BT601"
        },
        if is_hdr {
            "BT2020NCL"
        } else if dimensions.height >= 576 {
            "BT709"
        } else {
            "BT601"
        }
    )
}

fn build_x265_args_string(crf: u8, dimensions: VideoDimensions, profile: Profile) -> String {
    let deblock = match profile {
        Profile::Film => -3,
        Profile::Anime | Profile::Fast => -1,
    };
    format!(
        " --crf {} --preset slow --bframes {} --keyint -1 --min-keyint 1 --no-scenecut \
         --limit-sao --deblock {} --psy-rd {} --psy-rdoq {} --aq-mode 3 --aq-strength {} \
         --colormatrix {} --colorprim {} --transfer {} --output-depth {} --frame-threads 1 ",
        crf,
        match profile {
            Profile::Film => 5,
            Profile::Anime => 8,
            Profile::Fast => 3,
        },
        format!("{}:{}", deblock, deblock),
        match profile {
            Profile::Film => "1.5",
            Profile::Anime | Profile::Fast => "1.0",
        },
        match profile {
            Profile::Film => "4.0",
            Profile::Anime | Profile::Fast => "1.5",
        },
        match profile {
            Profile::Film => "0.8",
            Profile::Anime | Profile::Fast => "0.7",
        },
        dimensions.colorspace.to_string(),
        dimensions.colorspace.to_string(),
        dimensions.colorspace.to_string(),
        dimensions.bit_depth.to_string()
    )
}

fn build_x264_args_string(
    crf: u8,
    dimensions: VideoDimensions,
    profile: Profile,
    compat: Compat,
) -> String {
    format!(
        " --crf {} --preset {} --bframes {} --psy-rd {} --deblock {} --merange {} --rc-lookahead \
         250 --aq-mode 3 --aq-strength {} -i 1 -I infinite --no-scenecut --qcomp {} --ipratio \
         1.30 --pbratio 1.20 --no-fast-pskip --no-dct-decimate --colormatrix {} --colorprim {} \
         --transfer {} --output-depth {} {} {} --threads 4 ",
        crf,
        if profile == Profile::Fast {
            "faster"
        } else {
            "veryslow"
        },
        match profile {
            Profile::Film => 5,
            Profile::Anime => 8,
            Profile::Fast => 3,
        },
        match profile {
            Profile::Film => format!("{:.1}:{:.1}", 1.0, 0.0),
            Profile::Anime => format!("{:.1}:{:.1}", 0.7, 0.0),
            Profile::Fast => format!("{:.1}:{:.1}", 0.0, 0.0),
        },
        match profile {
            Profile::Film => format!("{}:{}", -3, -3),
            Profile::Anime => format!("{}:{}", -2, -1),
            Profile::Fast => format!("{}:{}", 0, 0),
        },
        if dimensions.width > 1440 {
            48
        } else if dimensions.width > 1024 {
            32
        } else {
            24
        },
        match profile {
            Profile::Film => 0.8,
            Profile::Anime => 0.7,
            Profile::Fast => 0.7,
        },
        match profile {
            Profile::Film | Profile::Fast => 0.75,
            Profile::Anime => 0.65,
        },
        dimensions.colorspace,
        dimensions.colorspace,
        dimensions.colorspace,
        dimensions.bit_depth,
        match compat {
            Compat::Dxva => {
                "--level 4.1 --vbv-maxrate 50000 --vbv-bufsize 78125"
            }
            Compat::Normal => "",
            Compat::None => {
                "--open-gop"
            }
        },
        match dimensions.pixel_format {
            PixelFormat::Yuv422 => {
                "--profile high422 --output-csp i422"
            }
            PixelFormat::Yuv444 => {
                "--profile high444 --output-csp i444"
            }
            _ => "",
        }
    )
}
