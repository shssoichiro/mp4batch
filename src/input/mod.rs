use std::{
    collections::HashMap,
    fmt,
    fmt::Display,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::Result;

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

#[derive(Debug, Clone, Copy, PartialEq)]
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorSpace {
    Bt709,
    Smpte170m,
}

impl Display for ColorSpace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> ::std::result::Result<(), fmt::Error> {
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

pub fn get_video_dimensions(input: &Path) -> Result<VideoDimensions> {
    let filename = input.file_name().unwrap().to_str().unwrap();
    if filename.ends_with(".vpy") {
        get_video_dimensions_vps(input)
    } else {
        get_video_dimensions_ffprobe(input)
    }
}

fn get_video_dimensions_ffprobe(input: &Path) -> Result<VideoDimensions> {
    let mediainfo = get_video_mediainfo(input)?;

    let width = mediainfo
        .get(&"Width".to_string())
        .unwrap()
        .replace(' ', "")
        .parse()?;
    let height = mediainfo
        .get(&"Height".to_string())
        .unwrap()
        .replace(' ', "")
        .parse()?;
    let fps = (
        mediainfo
            .get(&"Frame rate".to_string())
            .unwrap()
            .parse::<f32>()?
            .round() as u32,
        1,
    );
    let bit_depth = mediainfo.get(&"Bit depth".to_string()).unwrap().parse()?;

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

pub fn get_video_frame_count(input: &Path) -> Result<u32> {
    let command = Command::new("mediainfo")
        .arg("--Output=Video;%FrameCount%")
        .arg(input)
        .output()?;
    let output = String::from_utf8_lossy(&command.stdout);
    Ok(output.trim().parse()?)
}

fn get_video_dimensions_vps(input: &Path) -> Result<VideoDimensions> {
    let command = Command::new("vspipe")
        .arg("-i")
        .arg(input)
        .arg("-")
        .output()?;
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
    let width = lines[0].replace("Width: ", "").trim().parse()?;
    let height = lines[1].replace("Height: ", "").trim().parse()?;
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
        frames: lines[2].replace("Frames: ", "").trim().parse()?,
        fps: (fps[0].clone()?, fps[1].clone()?),
        pixel_format: PixelFormat::from_vapoursynth_format(&lines[4].replace("Format Name: ", "")),
        colorspace: ColorSpace::from_dimensions(width, height),
        bit_depth,
    })
}

pub fn get_video_mediainfo(input: &Path) -> Result<HashMap<String, String>> {
    let command = Command::new("mediainfo").arg(input).output()?;
    let output = String::from_utf8_lossy(&command.stdout);

    let mut data = output
        .lines()
        .skip_while(|line| line.trim() != "Video")
        .skip(1)
        .take_while(|line| !line.is_empty())
        .map(|line| {
            let (key, value) = line.split_once(':').unwrap();
            (key.trim().to_string(), value.trim().to_string())
        })
        .collect::<HashMap<String, String>>();
    data.insert(
        "File size".to_owned(),
        output
            .lines()
            .find(|l| l.starts_with("File size"))
            .unwrap()
            .split_once(':')
            .unwrap()
            .1
            .trim()
            .to_string(),
    );

    Ok(data)
}

pub fn find_source_file(input: &Path) -> PathBuf {
    if input.extension().unwrap().to_string_lossy() != "vpy" {
        return input.to_path_buf();
    }

    let script = fs::read_to_string(input).unwrap();
    let (_, source) = script.split_once("source=").unwrap();
    // If you have a quotation mark in your filename then go to hell
    let source = source
        .chars()
        .skip(1)
        .take_while(|&c| c != '"')
        .collect::<String>();
    // Handle relative or absolute paths
    let mut output = if input.is_dir() {
        input.to_path_buf()
    } else {
        input.parent().unwrap().to_path_buf()
    };
    output.push(&PathBuf::from(&source));
    output
}
