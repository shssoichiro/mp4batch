use std::fmt;
use std::fmt::Display;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy)]
pub struct VideoDimensions {
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    // fps in num/den format
    pub fps: (u32, u32),
    pub pixel_format: PixelFormat,
    pub colorspace: ColorSpace,
}

#[derive(Debug, Clone, Copy)]
pub enum PixelFormat {
    Yuv420,
    Yuv422,
    Yuv444,
}

impl Default for PixelFormat {
    fn default() -> Self {
        PixelFormat::Yuv420
    }
}

impl PixelFormat {
    fn from_vapoursynth_format(format: &str) -> Self {
        if format.starts_with("YUV420") {
            return PixelFormat::Yuv420;
        }
        if format.starts_with("YUV422") {
            return PixelFormat::Yuv422;
        }
        if format.starts_with("YUV444") {
            return PixelFormat::Yuv444;
        }
        unimplemented!()
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ColorSpace {
    Bt709,
    Smpte170m,
}

impl Display for ColorSpace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        match *self {
            ColorSpace::Bt709 => write!(f, "bt709"),
            ColorSpace::Smpte170m => write!(f, "smpte170m"),
        }
    }
}

impl ColorSpace {
    fn from_dimensions(_width: u32, height: u32) -> Self {
        if height >= 576 {
            ColorSpace::Bt709
        } else {
            ColorSpace::Smpte170m
        }
    }
}

pub fn get_video_dimensions(input: &Path) -> Result<VideoDimensions, String> {
    let filename = input.file_name().unwrap().to_str().unwrap();
    if filename.ends_with(".vpy") {
        get_video_dimensions_vps(input)
    } else {
        panic!("Unrecognized input type");
    }
}

fn get_video_dimensions_vps(input: &Path) -> Result<VideoDimensions, String> {
    let command = Command::new("vspipe")
        .arg("-i")
        .arg(input)
        .arg("-")
        .output()
        .map_err(|e| format!("{}", e))?;
    // Width: 1280
    // Height: 720
    // Frames: 17982
    // FPS: 24000/1001 (23.976 fps)
    // Format Name: YUV420P8
    // Color Family: YUV
    // Alpha: No
    // Sample Type: Integer
    // Bits: 8
    // SubSampling W: 1
    // SubSampling H: 1
    let output = String::from_utf8_lossy(&command.stdout);

    const PARSE_ERROR: &str = "Could not detect video dimensions";
    let lines = output.lines().take(5).collect::<Vec<_>>();
    if lines.len() == 5 {
        let width = lines[0]
            .replace("Width: ", "")
            .trim()
            .parse()
            .map_err(|e| format!("{}", e))?;
        let height = lines[1]
            .replace("Height: ", "")
            .trim()
            .parse()
            .map_err(|e| format!("{}", e))?;
        let fps: Vec<_> = lines[3]
            .replace("FPS: ", "")
            .split_whitespace()
            .next()
            .unwrap()
            .split('/')
            .map(|num| num.parse())
            .collect();
        Ok(VideoDimensions {
            width,
            height,
            frames: lines[2]
                .replace("Frames: ", "")
                .trim()
                .parse()
                .map_err(|e| format!("{}", e))?,
            fps: (
                *fps[0].as_ref().map_err(|e| format!("{}", e))?,
                *fps[1].as_ref().map_err(|e| format!("{}", e))?,
            ),
            pixel_format: PixelFormat::from_vapoursynth_format(
                &lines[4].replace("Format Name: ", ""),
            ),
            colorspace: ColorSpace::from_dimensions(width, height),
        })
    } else {
        Err(PARSE_ERROR.to_owned())
    }
}
