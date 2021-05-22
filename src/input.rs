use std::{fmt, fmt::Display, path::Path, process::Command};

use regex::Regex;

#[derive(Debug, Clone, Copy)]
pub struct VideoDimensions {
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    // fps in num/den format
    pub fps: (u32, u32),
    pub pixel_format: PixelFormat,
    pub colorspace: ColorSpace,
    pub bit_depth: u8,
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
        get_video_dimensions_ffprobe(input)
    }
}

fn get_video_dimensions_ffprobe(input: &Path) -> Result<VideoDimensions, String> {
    let command = Command::new("mediainfo")
        .arg(input)
        .output()
        .map_err(|e| format!("{}", e))?;
    let output = String::from_utf8_lossy(&command.stdout);

    // Raw videos only care about width, height, and FPS
    let width_regex = Regex::new(r"Width\s+: ([\d ]+) pixels").unwrap();
    let height_regex = Regex::new(r"Height\s+: ([\d ]+) pixels").unwrap();
    let fps_regex = Regex::new(r"Frame rate\s+: (\d+\.\d+) FPS").unwrap();
    let bit_depth_regex = Regex::new(r"Bit depth\s+: (\d+) bits").unwrap();

    let width = width_regex.captures(&output).unwrap()[1]
        .replace(' ', "")
        .parse()
        .unwrap();
    let height = height_regex.captures(&output).unwrap()[1]
        .replace(' ', "")
        .parse()
        .unwrap();
    let fps = (
        fps_regex.captures(&output).unwrap()[1]
            .parse::<f32>()
            .unwrap()
            .round() as u32,
        1,
    );
    let bit_depth = bit_depth_regex.captures(&output).unwrap()[1]
        .parse()
        .unwrap();

    Ok(VideoDimensions {
        width,
        height,
        fps,
        frames: 0,
        pixel_format: PixelFormat::Yuv420,
        colorspace: ColorSpace::Bt709,
        bit_depth,
    })
}

pub fn get_video_frame_count(input: &Path) -> Result<u32, String> {
    let command = Command::new("mediainfo")
        .arg("--Output=Video;%FrameCount%")
        .arg(input)
        .output()
        .map_err(|e| format!("{}", e))?;
    let output = String::from_utf8_lossy(&command.stdout);
    Ok(output.trim().parse().unwrap())
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

    let lines = output.lines().collect::<Vec<_>>();
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
    let bit_depth = lines[8].replace("Bits: ", "").trim().parse().unwrap();
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
        pixel_format: PixelFormat::from_vapoursynth_format(&lines[4].replace("Format Name: ", "")),
        colorspace: ColorSpace::from_dimensions(width, height),
        bit_depth,
    })
}
