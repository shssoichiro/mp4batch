use crate::input::{ColorSpace, PixelFormat, VideoDimensions};
use std::cmp;
use std::path::Path;
use std::process::{Command, Stdio};
use std::str::FromStr;

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
    pub highbd: bool,
    pub pixel_format: PixelFormat,
    pub colorspace: ColorSpace,
}

impl X264Settings {
    pub fn new(crf: u8, profile: Profile, highbd: bool, dimensions: VideoDimensions) -> Self {
        let fps = dimensions.fps.0 as f32 / dimensions.fps.1 as f32;
        X264Settings {
            crf,
            ref_frames: cmp::min(
                match profile {
                    Profile::Anime => 6,
                    Profile::Film => 3,
                } + (fps / 10.).round() as u8,
                16,
            ),
            psy_rd: match profile {
                Profile::Anime => (0.7, 0.0),
                _ => (1.0, 0.2),
            },
            deblock: (-3, -3),
            aq_strength: match profile {
                Profile::Anime => 0.7,
                _ => 1.0,
            },
            min_keyint: match profile {
                Profile::Anime => fps.round() as usize / 2,
                Profile::Film => fps.round() as usize,
            },
            max_keyint: match profile {
                Profile::Anime => fps.round() as usize * 30 / 2,
                Profile::Film => fps.round() as usize * 10,
            },
            qcomp: match profile {
                Profile::Anime => 0.6,
                Profile::Film => 0.7,
            },
            highbd,
            pixel_format: dimensions.pixel_format,
            colorspace: dimensions.colorspace,
        }
    }

    pub fn apply_to_command<'a>(&self, command: &'a mut Command) -> &'a mut Command {
        command
            .arg("--crf")
            .arg(self.crf.to_string())
            .arg("--ref")
            .arg(self.ref_frames.to_string())
            .arg("--mixed-refs")
            .arg("--no-fast-pskip")
            .arg("--b-adapt")
            .arg("2")
            .arg("--bframes")
            .arg(self.ref_frames.to_string())
            .arg("--b-pyramid")
            .arg("normal")
            .arg("--weightb")
            .arg("--direct")
            .arg("spatial")
            .arg("--subme")
            .arg("10")
            .arg("--trellis")
            .arg("2")
            .arg("--partitions")
            .arg("all")
            .arg("--psy-rd")
            .arg(format!("{:.2}:{:.2}", self.psy_rd.0, self.psy_rd.1))
            .arg("--deblock")
            .arg(format!("{}:{}", self.deblock.0, self.deblock.1))
            .arg("--me")
            .arg("umh")
            .arg("--merange")
            .arg("32")
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
            .arg("--vbv-maxrate")
            .arg("30000")
            .arg("--vbv-bufsize")
            .arg("60000")
            .arg("--colormatrix")
            .arg(self.colorspace.to_string())
            .arg("--colorprim")
            .arg(self.colorspace.to_string())
            .arg("--transfer")
            .arg(self.colorspace.to_string());
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
        if self.highbd {
            command.arg("--output-depth").arg("10");
        }
        command
    }
}

pub fn convert_video_x264(
    input: &Path,
    profile: Profile,
    crf: u8,
    highbd: bool,
    dimensions: VideoDimensions,
) -> Result<(), String> {
    let settings = X264Settings::new(crf, profile, highbd, dimensions);
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

pub fn convert_video_rav1e(
    input: &Path,
    crf: u8,
    dimensions: VideoDimensions,
    tiles: Option<u8>,
    workers: Option<u8>,
) -> Result<(), String> {
    let mut command = Command::new("rav1e-by-gop");
    let fps = (dimensions.fps.0 as f64 / dimensions.fps.1 as f64).round() as u32;
    command
        .arg("-")
        .arg("-q")
        .arg(crf.to_string())
        .arg("-s")
        .arg("6")
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
        .arg(30000.to_string());
    if let Some(tiles) = tiles {
        command.arg("--tiles").arg(tiles.to_string());
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
    Rav1e,
    X264,
}
