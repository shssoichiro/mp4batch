use std::{
    fmt::Display,
    path::Path,
    process::{Command, Stdio},
    str::FromStr,
};

use ansi_term::Colour::{Green, Yellow};
use anyhow::Result;

use crate::{
    absolute_path,
    input::{get_video_frame_count, PixelFormat, VideoDimensions},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoOutput {
    pub encoder: VideoEncoder,
    pub output_ext: String,
    pub bit_depth: Option<u8>,
    pub resolution: Option<(u32, u32)>,
}

impl Default for VideoOutput {
    fn default() -> Self {
        VideoOutput {
            encoder: VideoEncoder::X264 {
                crf: 18,
                profile: Profile::Film,
                compat: false,
            },
            output_ext: "mkv".to_string(),
            bit_depth: None,
            resolution: None,
        }
    }
}

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
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_ref() {
            "film" => Profile::Film,
            "anime" => Profile::Anime,
            "fast" => Profile::Fast,
            _ => {
                return Err("Unrecognized profile");
            }
        })
    }
}

impl Display for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        write!(
            f,
            "{}",
            match self {
                Profile::Film => "film",
                Profile::Anime => "anime",
                Profile::Fast => "fast",
            }
        )
    }
}

pub fn create_lossless(input: &Path, dimensions: VideoDimensions) -> Result<()> {
    let lossless_filename = input.with_extension("lossless.mkv");
    if lossless_filename.exists() {
        if let Ok(lossless_frames) = get_video_frame_count(&lossless_filename) {
            if lossless_frames == dimensions.frames {
                eprintln!(
                    "{} {}",
                    Green.bold().paint("[Success]"),
                    Green.paint("Lossless already exists"),
                );
                return Ok(());
            }
        }
    }

    // Print the info once
    Command::new("vspipe")
        .arg("-i")
        .arg(input)
        .arg("-")
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to execute vspipe -i: {}", e))?;

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
        .arg("libx264")
        .arg("-preset")
        .arg("ultrafast")
        .arg("-qp")
        .arg("0")
        .arg(&lossless_filename)
        .stdin(pipe.stdout.unwrap())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to execute ffmpeg: {}", e))?;
    if !status.success() {
        anyhow::bail!(
            "Failed to execute ffmpeg: Exited with code {:x}",
            status.code().unwrap()
        );
    }

    if let Ok(lossless_frames) = get_video_frame_count(&lossless_filename) {
        if lossless_frames != dimensions.frames {
            anyhow::bail!("Incomlete lossless encode");
        }
    }

    eprintln!(
        "{} {}",
        Green.bold().paint("[Success]"),
        Green.paint("Finished encoding lossless"),
    );

    Ok(())
}

pub fn convert_video_av1an(
    vpy_input: &Path,
    output: &Path,
    encoder: VideoEncoder,
    dimensions: VideoDimensions,
) -> Result<()> {
    if dimensions.width % 8 != 0 {
        eprintln!(
            "{} {} {} {}",
            Yellow.bold().paint("[Warning]"),
            Yellow.paint("Width"),
            Yellow.paint(dimensions.width.to_string()),
            Yellow.paint("is not divisble by 8")
        );
    }
    if dimensions.height % 8 != 0 {
        eprintln!(
            "{} {} {} {}",
            Yellow.bold().paint("[Warning]"),
            Yellow.paint("Height"),
            Yellow.paint(dimensions.height.to_string()),
            Yellow.paint("is not divisble by 8")
        );
    }

    if output.exists() && get_video_frame_count(output)? == dimensions.frames {
        return Ok(());
    }

    let fps = (dimensions.fps.0 as f32 / dimensions.fps.1 as f32).round() as u32;
    // We may not actually split tiles at this point,
    // but we want to make sure we don't run out of memory
    let tiles = if dimensions.height >= 1600 { 2 } else { 1 }
        * if dimensions.width >= 1600 { 2 } else { 1 };
    let workers = std::cmp::max(num_cpus::get() / tiles, 1);
    let mut command = Command::new("nice");
    command
        .arg("av1an")
        .arg("-i")
        .arg(absolute_path(vpy_input).unwrap())
        .arg("-e")
        .arg(encoder.get_av1an_name())
        .arg("-v")
        .arg(&encoder.get_args_string(dimensions, workers))
        .arg("--sc-method")
        .arg("standard")
        .arg("-x")
        .arg(
            match encoder {
                VideoEncoder::Aom { profile, .. }
                | VideoEncoder::Rav1e { profile, .. }
                | VideoEncoder::X264 { profile, .. }
                | VideoEncoder::X265 { profile, .. } => match profile {
                    Profile::Film | Profile::Fast => fps * 10,
                    Profile::Anime => fps * 15,
                },
            }
            .to_string(),
        )
        .arg("--min-scene-len")
        .arg(
            match encoder {
                VideoEncoder::Aom { profile, .. }
                | VideoEncoder::Rav1e { profile, .. }
                | VideoEncoder::X264 { profile, .. }
                | VideoEncoder::X265 { profile, .. } => match profile {
                    Profile::Film | Profile::Fast => fps,
                    Profile::Anime => fps / 2,
                },
            }
            .to_string(),
        )
        .arg("-w")
        .arg(workers.to_string())
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
        .arg(absolute_path(output).unwrap());
    if dimensions.height > 1080 {
        command.arg("--sc-downscale-height").arg("1080");
    }
    if num_cpus::get() % workers == 0 && encoder.has_tiling() {
        command
            .arg("--set-thread-affinity")
            .arg((num_cpus::get() / workers).to_string());
    }
    if let VideoEncoder::Aom { grain, .. } = encoder {
        if grain > 0 {
            command.arg("--photon-noise").arg(grain.to_string());
        }
    }
    let status = command
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to execute av1an: {}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "Failed to execute av1an: Exited with code {:x}",
            status.code().unwrap()
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoEncoder {
    Aom {
        crf: i16,
        speed: u8,
        profile: Profile,
        is_hdr: bool,
        grain: u8,
        compat: bool,
    },
    Rav1e {
        crf: i16,
        speed: u8,
        profile: Profile,
        is_hdr: bool,
    },
    X264 {
        crf: i16,
        profile: Profile,
        compat: bool,
    },
    X265 {
        crf: i16,
        profile: Profile,
        compat: bool,
        is_hdr: bool,
    },
}

impl VideoEncoder {
    pub const fn supported_encoders() -> &'static [&'static str] {
        &["aom", "rav1e", "x264", "x265"]
    }

    pub const fn get_av1an_name(&self) -> &str {
        match self {
            VideoEncoder::Aom { .. } => "aom",
            VideoEncoder::Rav1e { .. } => "rav1e",
            VideoEncoder::X264 { .. } => "x264",
            VideoEncoder::X265 { .. } => "x265",
        }
    }

    pub fn get_args_string(&self, dimensions: VideoDimensions, workers: usize) -> String {
        match self {
            VideoEncoder::Aom {
                crf,
                speed,
                profile,
                is_hdr,
                ..
            } => build_aom_args_string(*crf, *speed, dimensions, *profile, *is_hdr, workers),
            VideoEncoder::Rav1e {
                crf, speed, is_hdr, ..
            } => build_rav1e_args_string(*crf, *speed, dimensions, *is_hdr),
            VideoEncoder::X264 {
                crf,
                profile,
                compat,
            } => build_x264_args_string(*crf, dimensions, *profile, *compat),
            VideoEncoder::X265 {
                crf,
                profile,
                compat,
                is_hdr,
                ..
            } => build_x265_args_string(*crf, dimensions, *profile, *compat, *is_hdr),
        }
    }

    pub const fn has_tiling(&self) -> bool {
        matches!(self, VideoEncoder::Aom { .. } | VideoEncoder::Rav1e { .. })
    }
}

fn build_aom_args_string(
    crf: i16,
    speed: u8,
    dimensions: VideoDimensions,
    profile: Profile,
    is_hdr: bool,
    workers: usize,
) -> String {
    format!(
        " --cpu-used={} --cq-level={} --end-usage=q --tune-content={} --lag-in-frames=64 \
         --enable-fwd-kf=1 --aq-mode=1 --deltaq-mode={} --enable-chroma-deltaq=1 \
         --quant-b-adapt=1 --enable-qm=1 --min-q=1 --arnr-strength=1 --arnr-maxframes=3 \
         --sharpness=2 --enable-dnl-denoising=0 --disable-trellis-quant=0 --enable-dual-filter=0 \
         --tune=image_perceptual_quality --tile-columns={} --tile-rows={} --threads=64 \
         --row-mt={} --color-primaries={} --transfer-characteristics={} --matrix-coefficients={} \
         -b {} --disable-kf ",
        speed,
        crf,
        if profile == Profile::Anime {
            "animation"
        } else {
            "psy"
        },
        if is_hdr { 5 } else { 1 },
        if dimensions.width >= 1936 { 1 } else { 0 },
        if dimensions.height >= 1936 { 1 } else { 0 },
        if workers >= num_cpus::get() { 0 } else { 1 },
        if is_hdr {
            "bt2020"
        } else if dimensions.height > 576 {
            "bt709"
        } else {
            "bt601"
        },
        if is_hdr {
            "smpte2084"
        } else if dimensions.height > 576 {
            "bt709"
        } else {
            "bt601"
        },
        if is_hdr {
            "bt2020ncl"
        } else if dimensions.height > 576 {
            "bt709"
        } else {
            "bt601"
        },
        dimensions.bit_depth
    )
}

fn build_rav1e_args_string(
    crf: i16,
    speed: u8,
    dimensions: VideoDimensions,
    is_hdr: bool,
) -> String {
    // TODO: Add proper HDR metadata
    format!(
        " --speed={} --quantizer={} --tile-cols={} --tile-rows={} --primaries={} --transfer={} \
         --matrix={} --no-scene-detection --keyint 0 --rdo-lookahead-frames 40 ",
        speed,
        crf,
        if dimensions.width >= 1600 { 2 } else { 1 },
        if dimensions.height >= 1600 { 2 } else { 1 },
        if is_hdr {
            "BT2020"
        } else if dimensions.height > 576 {
            "BT709"
        } else {
            "BT601"
        },
        if is_hdr {
            "SMPTE2084"
        } else if dimensions.height > 576 {
            "BT709"
        } else {
            "BT601"
        },
        if is_hdr {
            "BT2020NCL"
        } else if dimensions.height > 576 {
            "BT709"
        } else {
            "BT601"
        }
    )
}

fn build_x265_args_string(
    crf: i16,
    dimensions: VideoDimensions,
    profile: Profile,
    compat: bool,
    is_hdr: bool,
) -> String {
    if is_hdr {
        todo!("Implement HDR support for x265");
    }

    let deblock = match profile {
        Profile::Film => -2,
        Profile::Anime | Profile::Fast => -1,
    };
    format!(
        " --crf {} --preset slow --bframes {} --keyint -1 --min-keyint 1 --no-scenecut {} \
         --deblock {}:{} --psy-rd {} --psy-rdoq {} --aq-mode 3 --aq-strength {} --rc-lookahead 60 \
         --lookahead-slices 1 --lookahead-threads 1 --weightb --colormatrix {} --colorprim {} \
         --transfer {} --output-depth {} --frame-threads 1 --y4m {} {} ",
        crf,
        match profile {
            Profile::Film => 5,
            Profile::Anime => 8,
            Profile::Fast => 3,
        },
        if (profile == Profile::Anime && crf >= 17) || crf >= 19 {
            "--limit-sao"
        } else {
            "--no-sao"
        },
        deblock,
        deblock,
        match profile {
            Profile::Film => "1.5",
            Profile::Anime | Profile::Fast => "1.0",
        },
        match profile {
            Profile::Film => "3.0",
            Profile::Anime | Profile::Fast => "1.5",
        },
        match profile {
            Profile::Film => "0.8",
            Profile::Anime | Profile::Fast => "0.7",
        },
        dimensions.colorspace,
        dimensions.colorspace,
        dimensions.colorspace,
        dimensions.bit_depth,
        if compat {
            if dimensions.bit_depth == 10 {
                "--profile main10 --level-idc 5.1"
            } else {
                "--profile main --level-idc 5.1"
            }
        } else {
            ""
        },
        if is_hdr { "--hdr10-opt" } else { "" }
    )
}

fn build_x264_args_string(
    crf: i16,
    dimensions: VideoDimensions,
    profile: Profile,
    compat: bool,
) -> String {
    format!(
        " --crf {} --preset {} --bframes {} --psy-rd {} --deblock {} --merange {} --rc-lookahead \
         96 --aq-mode 3 --aq-strength {} -i 1 -I infinite --no-scenecut --qcomp {} --ipratio 1.30 \
         --pbratio 1.20 --no-fast-pskip --no-dct-decimate --colormatrix {} --colorprim {} \
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
        if compat {
            "--level 4.1 --vbv-maxrate 50000 --vbv-bufsize 78125"
        } else {
            ""
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
