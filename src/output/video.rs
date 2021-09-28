use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    str::FromStr,
};

use crate::input::{get_video_frame_count, ColorSpace, PixelFormat, VideoDimensions};

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

#[derive(Debug, Clone, Copy)]
struct X264Settings {
    pub profile: Profile,
    pub crf: u8,
    pub b_frames: u8,
    pub psy_rd: (f32, f32),
    pub deblock: (i8, i8),
    pub aq_strength: f32,
    pub min_keyint: usize,
    pub max_keyint: usize,
    pub qcomp: f32,
    pub merange: u8,
    pub bit_depth: u8,
    pub pixel_format: PixelFormat,
    pub colorspace: ColorSpace,
}

impl X264Settings {
    pub fn new(crf: u8, profile: Profile, dimensions: VideoDimensions) -> Self {
        let fps = dimensions.fps.0 as f32 / dimensions.fps.1 as f32;
        X264Settings {
            profile,
            crf,
            b_frames: match profile {
                Profile::Film => 5,
                Profile::Anime => 8,
                Profile::Fast => 3,
            },
            psy_rd: match profile {
                Profile::Film => (1.0, 0.0),
                Profile::Anime => (0.7, 0.0),
                Profile::Fast => (0.0, 0.0),
            },
            deblock: match profile {
                Profile::Film => (-3, -3),
                Profile::Anime => (-2, -1),
                Profile::Fast => (0, 0),
            },
            aq_strength: match profile {
                Profile::Film => 0.8,
                Profile::Anime => 0.7,
                Profile::Fast => 0.7,
            },
            min_keyint: match profile {
                Profile::Film | Profile::Fast => fps.round() as usize,
                Profile::Anime => fps.round() as usize / 2,
            },
            max_keyint: match profile {
                Profile::Film | Profile::Fast => fps.round() as usize * 10,
                Profile::Anime => fps.round() as usize * 30 / 2,
            },
            qcomp: match profile {
                Profile::Film | Profile::Fast => 0.75,
                Profile::Anime => 0.65,
            },
            merange: if dimensions.width > 1440 {
                48
            } else if dimensions.width > 1024 {
                32
            } else {
                24
            },
            bit_depth: dimensions.bit_depth,
            pixel_format: dimensions.pixel_format,
            colorspace: dimensions.colorspace,
        }
    }

    pub fn apply_to_command<'a>(&self, command: &'a mut Command) -> &'a mut Command {
        command
            .arg("--crf")
            .arg(self.crf.to_string())
            .arg("--preset")
            .arg(if self.profile == Profile::Fast {
                "faster"
            } else {
                "veryslow"
            })
            .arg("--level")
            .arg("4.1")
            .arg("--bframes")
            .arg(self.b_frames.to_string())
            .arg("--psy-rd")
            .arg(format!("{:.2}:{:.2}", self.psy_rd.0, self.psy_rd.1))
            .arg("--deblock")
            .arg(format!("{}:{}", self.deblock.0, self.deblock.1))
            .arg("--merange")
            .arg(self.merange.to_string())
            .arg("--rc-lookahead")
            .arg("250")
            .arg("--aq-mode")
            .arg("3")
            .arg("--aq-strength")
            .arg(format!("{:.2}", self.aq_strength))
            .arg("-i")
            .arg(self.min_keyint.to_string())
            .arg("-I")
            .arg(self.max_keyint.to_string())
            .arg("--qcomp")
            .arg(format!("{:.2}", self.qcomp))
            .arg("--ipratio")
            .arg("1.30")
            .arg("--pbratio")
            .arg("1.20")
            .arg("--no-fast-pskip")
            .arg("--no-dct-decimate")
            .arg("--vbv-maxrate")
            .arg("50000")
            .arg("--vbv-bufsize")
            .arg("78125")
            .arg("--colormatrix")
            .arg(self.colorspace.to_string())
            .arg("--colorprim")
            .arg(self.colorspace.to_string())
            .arg("--transfer")
            .arg(self.colorspace.to_string())
            .arg("--output-depth")
            .arg(self.bit_depth.to_string());
        match self.pixel_format {
            PixelFormat::Yuv422 => {
                command
                    .arg("--profile")
                    .arg("high422")
                    .arg("--output-csp")
                    .arg("i422");
            }
            PixelFormat::Yuv444 => {
                command
                    .arg("--profile")
                    .arg("high444")
                    .arg("--output-csp")
                    .arg("i444");
            }
            _ => (),
        }
        command
    }
}

pub fn convert_video_x264(
    input: &Path,
    profile: Profile,
    crf: u8,
    dimensions: VideoDimensions,
) -> Result<(), String> {
    let settings = X264Settings::new(crf, profile, dimensions);
    let mut command = Command::new("x264");
    settings
        .apply_to_command(&mut command)
        .arg("--output")
        .arg(input.with_extension("out.mkv"))
        .arg("-");
    let filename = input.file_name().unwrap().to_str().unwrap();
    let pipe = if filename.ends_with(".vpy") {
        Command::new("vspipe")
            .arg("--y4m")
            .arg(input)
            .arg("-")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap()
    } else {
        panic!("Unrecognized input type");
    };
    command
        .arg("--frames")
        .arg(format!("{}", dimensions.frames))
        .arg("--stdin")
        .arg("y4m")
        .arg("-")
        .stdin(pipe.stdout.unwrap())
        .stderr(Stdio::inherit());
    let status = command
        .status()
        .map_err(|e| format!("Failed to execute x264: {}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Failed to execute x264: Exited with code {:x}",
            status.code().unwrap()
        ))
    }
}

pub fn convert_video_x265(
    input: &Path,
    profile: Profile,
    crf: u8,
    dimensions: VideoDimensions,
) -> Result<(), String> {
    let filename = input.file_name().unwrap().to_str().unwrap();
    let pipe = if filename.ends_with(".vpy") {
        Command::new("vspipe")
            .arg("--y4m")
            .arg(input)
            .arg("-")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap()
    } else {
        panic!("Unrecognized input type");
    };

    let fps = dimensions.fps.0 as f32 / dimensions.fps.1 as f32;
    let deblock = match profile {
        Profile::Anime | Profile::Fast => -1,
        Profile::Film => -3,
    };
    let status = Command::new("x265")
        .arg("-")
        .arg("--crf")
        .arg(crf.to_string())
        .arg("--preset")
        .arg("slow")
        .arg("--bframes")
        .arg(match profile {
            Profile::Film => "5",
            Profile::Anime => "8",
            Profile::Fast => "3",
        })
        .arg("--min-keyint")
        .arg(
            match profile {
                Profile::Film | Profile::Fast => fps.round() as usize,
                Profile::Anime => fps.round() as usize / 2,
            }
            .to_string(),
        )
        .arg("--keyint")
        .arg(
            match profile {
                Profile::Film | Profile::Fast => fps.round() as usize * 10,
                Profile::Anime => fps.round() as usize * 30 / 2,
            }
            .to_string(),
        )
        .arg("--limit-sao")
        .arg("--deblock")
        .arg(format!("{}:{}", deblock, deblock))
        .arg("--psy-rd")
        .arg(match profile {
            Profile::Film => "1.5",
            Profile::Anime | Profile::Fast => "1.0",
        })
        .arg("--psy-rdoq")
        .arg(match profile {
            Profile::Film => "4.0",
            Profile::Anime | Profile::Fast => "1.5",
        })
        .arg("--aq-mode")
        .arg("3")
        .arg("--aq-strength")
        .arg(match profile {
            Profile::Film => "0.8",
            Profile::Anime | Profile::Fast => "0.7",
        })
        .arg("--colormatrix")
        .arg(dimensions.colorspace.to_string())
        .arg("--colorprim")
        .arg(dimensions.colorspace.to_string())
        .arg("--transfer")
        .arg(dimensions.colorspace.to_string())
        .arg("--output-depth")
        .arg(dimensions.bit_depth.to_string())
        .arg("--vbv-maxrate")
        .arg("30000")
        .arg("--vbv-bufsize")
        .arg("60000")
        .arg("--output")
        .arg(input.with_extension("out.265"))
        .arg("--frames")
        .arg(format!("{}", dimensions.frames))
        .arg("--y4m")
        .stdin(pipe.stdout.unwrap())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("Failed to execute x264: {}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Failed to execute x264: Exited with code {:x}",
            status.code().unwrap()
        ))
    }
}

#[allow(clippy::too_many_arguments)]
pub fn convert_video_av1<P: AsRef<Path>>(
    input: P,
    crf: u8,
    speed: Option<u8>,
    dimensions: VideoDimensions,
    profile: Profile,
    is_hdr: bool,
    use_lossless: bool,
    keep_lossless: bool,
) -> Result<(), String> {
    let fps = (dimensions.fps.0 as f32 / dimensions.fps.1 as f32).round() as u32;
    if use_lossless {
        let lossless_filename = input.as_ref().with_extension("lossless.mkv");
        let mut needs_encode = true;
        if lossless_filename.exists() {
            if let Ok(lossless_frames) = get_video_frame_count(&lossless_filename) {
                if lossless_frames == dimensions.frames {
                    needs_encode = false;
                }
            }
        }

        if needs_encode {
            // Print the info once
            Command::new("vspipe")
                .arg("-i")
                .arg(input.as_ref())
                .arg("-")
                .status()
                .map_err(|e| format!("Failed to execute vspipe -i: {}", e))?;

            let filename = input.as_ref().file_name().unwrap().to_str().unwrap();
            let pipe = if filename.ends_with(".vpy") {
                Command::new("vspipe")
                    .arg("--y4m")
                    .arg(input.as_ref())
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
    }

    let mut command = Command::new("nice");
    command
        .arg("av1an")
        .arg("-i")
        .arg(if use_lossless {
            input.as_ref().with_extension("lossless.mkv")
        } else {
            input.as_ref().to_path_buf()
        })
        .arg("-e")
        .arg("aom")
        .arg("-v")
        .arg(&format!(
            " --cpu-used={} --end-usage=q --cq-level={} --lag-in-frames=35 --enable-fwd-kf=1 \
             --qm-min=5 --quant-b-adapt=1 --enable-keyframe-filtering=0 --tile-columns={} \
             --tile-rows=0 --threads=4 --row-mt=0 --color-primaries={} \
             --transfer-characteristics={} --matrix-coefficients={} --disable-kf ",
            speed.unwrap_or(4),
            crf,
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
        ))
        .arg("--sc-method")
        .arg("standard")
        .arg("--sc-downscale-height")
        .arg("720")
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
            if dimensions.width >= 1200 {
                num_cpus::get() / 2 + num_cpus::get() / 8
            } else {
                num_cpus::get()
            }
            .to_string(),
        )
        .arg("-r")
        .arg("--verbose")
        .arg("-o")
        .arg(input.as_ref().with_extension("out.mkv"));
    let status = command
        .status()
        .map_err(|e| format!("Failed to execute av1an: {}", e))?;

    if status.success() {
        if !keep_lossless {
            let _ = fs::remove_file(input.as_ref().with_extension("lossless.mkv"));
        }
        // FIXME: This is still too slow.
        // let _ = Command::new("aomstats")
        //     .arg(input.as_ref().with_extension("out.mkv"))
        //     .stdout(Stdio::inherit())
        //     .stderr(Stdio::inherit())
        //     .status();
        Ok(())
    } else {
        Err(format!(
            "Failed to execute av1an: Exited with code {:x}",
            status.code().unwrap()
        ))
    }
}

#[allow(clippy::too_many_arguments)]
pub fn convert_video_av1an_rav1e<P: AsRef<Path>>(
    input: P,
    crf: u8,
    speed: Option<u8>,
    dimensions: VideoDimensions,
    profile: Profile,
    is_hdr: bool,
    use_lossless: bool,
    keep_lossless: bool,
) -> Result<(), String> {
    let fps = (dimensions.fps.0 as f32 / dimensions.fps.1 as f32).round() as u32;
    if use_lossless {
        let lossless_filename = input.as_ref().with_extension("lossless.mkv");
        let mut needs_encode = true;
        if lossless_filename.exists() {
            if let Ok(lossless_frames) = get_video_frame_count(&lossless_filename) {
                if lossless_frames == dimensions.frames {
                    needs_encode = false;
                }
            }
        }

        if needs_encode {
            // Print the info once
            Command::new("vspipe")
                .arg("-i")
                .arg(input.as_ref())
                .arg("-")
                .status()
                .unwrap();

            let filename = input.as_ref().file_name().unwrap().to_str().unwrap();
            let pipe = if filename.ends_with(".vpy") {
                Command::new("vspipe")
                    .arg("--y4m")
                    .arg(input.as_ref())
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
        }
    }

    let mut command = Command::new("nice");
    command
        .arg("av1an")
        .arg("-i")
        .arg(if use_lossless {
            input.as_ref().with_extension("lossless.mkv")
        } else {
            input.as_ref().to_path_buf()
        })
        .arg("-e")
        .arg("rav1e")
        .arg("-v")
        .arg(&format!(
            " --speed={} --quantizer={} --tile-cols={} --tile-rows=0 --primaries={} --transfer={} \
             --matrix={} --no-scene-detection ",
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
        ))
        .arg("--sc-method")
        .arg("standard")
        .arg("--sc-downscale-height")
        .arg("720")
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
            if dimensions.width >= 1200 {
                num_cpus::get() / 2 + num_cpus::get() / 8
            } else {
                num_cpus::get()
            }
            .to_string(),
        )
        .arg("-r")
        .arg("--verbose")
        .arg("-o")
        .arg(input.as_ref().with_extension("out.mkv"));
    let status = command
        .status()
        .map_err(|e| format!("Failed to execute av1an: {}", e))?;

    if status.success() {
        if !keep_lossless {
            let _ = fs::remove_file(input.as_ref().with_extension("lossless.mkv"));
        }
        Ok(())
    } else {
        Err(format!(
            "Failed to execute av1an: Exited with code {:x}",
            status.code().unwrap()
        ))
    }
}

#[allow(clippy::too_many_arguments)]
#[allow(dead_code)]
pub fn convert_video_rav1e<P: AsRef<Path>>(
    input: P,
    crf: u8,
    profile: Profile,
    dimensions: VideoDimensions,
    is_hdr: bool,
    slots: Option<u8>,
) -> Result<(), String> {
    let fps = (dimensions.fps.0 as f32 / dimensions.fps.1 as f32).round() as u32;
    let filename = input.as_ref().file_name().unwrap().to_str().unwrap();
    let pipe = if filename.ends_with(".vpy") {
        Command::new("vspipe")
            .arg("--y4m")
            .arg(input.as_ref())
            .arg("-")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap()
    } else {
        panic!("Unrecognized input type");
    };
    let slots = if let Some(slots) = slots {
        slots
    } else if dimensions.width >= 1440 {
        3
    } else if dimensions.width >= 1200 {
        5
    } else {
        8
    };
    let tile_rows = if dimensions.width >= 1440 { 2 } else { 1 };
    let tile_cols = if dimensions.width >= 1200 { 2 } else { 1 };

    let mut command = Command::new("nice");
    command
        .arg("rav1e-ch")
        .arg("--speed")
        .arg("5")
        .arg("--min-quantizer")
        .arg(crf.to_string())
        .arg("--bitrate")
        .arg(if dimensions.width >= 1440 {
            "50000"
        } else if dimensions.width >= 1200 {
            "30000"
        } else {
            "16000"
        })
        .arg("--tile-cols")
        .arg(tile_cols.to_string())
        .arg("--tile-rows")
        .arg(tile_rows.to_string())
        .arg("-I")
        .arg(
            match profile {
                Profile::Film | Profile::Fast => fps * 10,
                Profile::Anime => fps * 15,
            }
            .to_string(),
        )
        .arg("-i")
        .arg(
            match profile {
                Profile::Film | Profile::Fast => fps,
                Profile::Anime => fps / 2,
            }
            .to_string(),
        )
        .arg("--primaries")
        .arg(if is_hdr {
            "BT2020"
        } else if dimensions.height >= 576 {
            "BT709"
        } else {
            "BT601"
        })
        .arg("--transfer")
        .arg(if is_hdr {
            "BT2020_10Bit"
        } else if dimensions.height >= 576 {
            "BT709"
        } else {
            "BT601"
        })
        .arg("--matrix")
        .arg(if is_hdr {
            "BT2020NCL"
        } else if dimensions.height >= 576 {
            "BT709"
        } else {
            "BT601"
        })
        .arg("--slots")
        .arg(slots.to_string())
        .arg("--threads")
        .arg((4 + slots * tile_cols * tile_rows).to_string())
        .arg("--limit")
        .arg(dimensions.frames.to_string())
        .arg("-")
        .arg("-o")
        .arg(input.as_ref().with_extension("out.ivf"))
        .stdin(pipe.stdout.unwrap());
    let status = command
        .status()
        .map_err(|e| format!("Failed to execute rav1e: {}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Failed to execute av1an: Exited with code {:x}",
            status.code().unwrap()
        ))
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Encoder {
    #[allow(dead_code)]
    Aom,
    #[allow(dead_code)]
    Rav1e,
    X264,
    X265,
}
