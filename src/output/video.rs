use std::{
    path::Path,
    process::{Command, Stdio},
    str::FromStr,
};

use crate::input::{ColorSpace, PixelFormat, VideoDimensions};

#[derive(Debug, Clone, Copy)]
pub enum Profile {
    Film,
    Anime,
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
            _ => {
                return Err("Invalid profile given".to_owned());
            }
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct X264Settings {
    pub crf: u8,
    pub ref_frames: u8,
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
            crf,
            ref_frames: match profile {
                Profile::Film => 5,
                Profile::Anime => 8,
            },
            psy_rd: match profile {
                Profile::Film => (1.0, 0.0),
                Profile::Anime => (0.7, 0.0),
            },
            deblock: match profile {
                Profile::Film => (-3, -3),
                Profile::Anime => (-2, -1),
            },
            aq_strength: match profile {
                Profile::Film => 0.8,
                Profile::Anime => 0.7,
            },
            min_keyint: match profile {
                Profile::Film => fps.round() as usize,
                Profile::Anime => fps.round() as usize / 2,
            },
            max_keyint: match profile {
                Profile::Film => fps.round() as usize * 10,
                Profile::Anime => fps.round() as usize * 30 / 2,
            },
            qcomp: match profile {
                Profile::Film => 0.7,
                Profile::Anime => 0.6,
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
            .arg("veryslow")
            .arg("--ref")
            .arg(self.ref_frames.to_string())
            .arg("--bframes")
            .arg(self.ref_frames.to_string())
            .arg("--no-fast-pskip")
            .arg("--no-dct-decimate")
            .arg("--subme")
            .arg("11")
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
            .arg("--vbv-maxrate")
            .arg("30000")
            .arg("--vbv-bufsize")
            .arg("60000")
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
        Profile::Anime => -1,
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
        })
        .arg("--min-keyint")
        .arg(
            match profile {
                Profile::Film => fps.round() as usize,
                Profile::Anime => fps.round() as usize / 2,
            }
            .to_string(),
        )
        .arg("--keyint")
        .arg(
            match profile {
                Profile::Film => fps.round() as usize * 10,
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
            Profile::Anime => "1.0",
        })
        .arg("--psy-rdoq")
        .arg(match profile {
            Profile::Film => "4.0",
            Profile::Anime => "1.5",
        })
        .arg("--aq-mode")
        .arg("3")
        .arg("--aq-strength")
        .arg(match profile {
            Profile::Film => "0.8",
            Profile::Anime => "0.7",
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
        .arg(input.with_extension("out.mkv"))
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

pub fn convert_video_av1<P: AsRef<Path>>(
    input: P,
    crf: u8,
    dimensions: VideoDimensions,
    threads_per_worker: Option<u8>,
    workers: Option<u8>,
    profile: Profile,
    is_hdr: bool,
) -> Result<(), String> {
    let fps = (dimensions.fps.0 as f32 / dimensions.fps.1 as f32).round() as u32;

    let mut command = Command::new("nice");
    command
        .arg("av1an")
        .arg("-i")
        .arg(input.as_ref())
        .arg("-enc")
        .arg("aom")
        .arg("-v")
        .arg(&format!(
            "--cpu-used=5 --threads={} -b 10 --input-bit-depth=10 --end-usage=q --cq-level={} \
             --good --lag-in-frames=35 --enable-fwd-kf=1 --tile-columns={} --tile-rows={} \
             --auto-alt-ref=1 --color-primaries={} --transfer-characteristics={} \
             --matrix-coefficients={}",
            threads_per_worker.unwrap_or(8).to_string(),
            crf,
            if dimensions.width >= 1440 { 1 } else { 0 },
            if dimensions.height >= 1200 { 1 } else { 0 },
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
        .arg("-w")
        .arg(
            workers
                .unwrap_or(if dimensions.height >= 1200 {
                    4
                } else if dimensions.height >= 1024 {
                    6
                } else {
                    8
                })
                .to_string(),
        )
        .arg("--split_method")
        .arg("aom_keyframes")
        .arg("-xs")
        .arg(
            match profile {
                Profile::Film => fps * 10,
                Profile::Anime => fps * 15,
            }
            .to_string(),
        )
        .arg("-r")
        .arg("-o")
        .arg(input.as_ref().with_extension("out.mkv"));
    let status = command
        .status()
        .map_err(|e| format!("Failed to execute av1an: {}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Failed to execute av1an: Exited with code {:x}",
            status.code().unwrap()
        ))
    }
}

#[allow(clippy::clippy::too_many_arguments)]
pub fn convert_video_rav1e(
    input: &Path,
    crf: u8,
    dimensions: VideoDimensions,
    tiles: Option<u8>,
    workers: Option<u8>,
    is_hdr: bool,
    matrix: Option<&str>,
    primaries: Option<&str>,
    transfer: Option<&str>,
) -> Result<(), String> {
    let mut command = Command::new("rav1e-by-gop");
    let fps = (dimensions.fps.0 as f64 / dimensions.fps.1 as f64).round() as u32;
    command
        .arg("-")
        .arg("-q")
        .arg(crf.to_string())
        .arg("-s")
        .arg("5")
        .arg("-i")
        .arg(fps.to_string())
        .arg("-I")
        .arg((fps * 10).to_string())
        .arg("-o")
        .arg(input.with_extension("out.ivf"))
        .arg("--resume")
        .arg("--frames")
        .arg(dimensions.frames.to_string())
        .arg("--tmp-input")
        .arg("--max-bitrate")
        .arg(if dimensions.height >= 1440 {
            "100000"
        } else if dimensions.height >= 768 && fps >= 45 {
            "50000"
        } else {
            "30000"
        })
        .arg("--matrix")
        .arg(if let Some(matrix) = matrix {
            matrix
        } else if is_hdr {
            "BT2020NCL"
        } else if dimensions.height >= 576 {
            "BT709"
        } else {
            "BT601"
        })
        .arg("--primaries")
        .arg(if let Some(primaries) = primaries {
            primaries
        } else if is_hdr {
            "BT2020"
        } else if dimensions.height >= 576 {
            "BT709"
        } else {
            "BT601"
        })
        .arg("--transfer")
        .arg(if let Some(transfer) = transfer {
            transfer
        } else if is_hdr {
            "BT2020_10Bit"
        } else if dimensions.height >= 576 {
            "BT709"
        } else {
            "BT601"
        });
    if let Some(tiles) = tiles {
        command.arg("--tiles").arg(tiles.to_string());
    } else {
        command.arg("--tiles").arg(if dimensions.height >= 1200 {
            "4"
        } else if dimensions.width >= 1440 {
            "2"
        } else {
            "1"
        });
    }
    if let Some(workers) = workers {
        command.arg("--local-workers").arg(workers.to_string());
    }
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
    command.stdin(pipe.stdout.unwrap()).stderr(Stdio::inherit());
    let status = command
        .status()
        .map_err(|e| format!("Failed to execute rav1e: {}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Failed to execute rav1e: Exited with code {:x}",
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
