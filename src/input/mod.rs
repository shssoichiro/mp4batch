use std::{
    borrow::Cow,
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, anyhow, bail};
use av_data::pixel::{
    ChromaLocation, ColorPrimaries, FromPrimitive, MatrixCoefficients, ToPrimitive,
    TransferCharacteristic, YUVRange,
};
use itertools::Itertools;
use once_cell::sync::OnceCell;
use regex::Regex;
use vapoursynth::vsscript::{Environment, EvalFlags};

use crate::output::VideoEncoderIdent;

#[derive(Debug, Clone, Copy)]
pub struct VideoDimensions {
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    // fps in num/den format
    pub fps: (u32, u32),
    pub pixel_format: PixelFormat,
    pub bit_depth: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PixelFormat {
    #[default]
    Yuv420,
    Yuv422,
    Yuv444,
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

pub fn get_video_dimensions(input: &Path) -> Result<VideoDimensions> {
    let filename = input
        .file_name()
        .expect("File should have a name")
        .to_string_lossy();
    if filename.ends_with(".vpy") {
        get_video_dimensions_vps(input)
    } else {
        get_video_dimensions_mediainfo(input)
    }
}

fn get_video_dimensions_mediainfo(input: &Path) -> Result<VideoDimensions> {
    let mediainfo = get_video_mediainfo(input)?;

    let width = mediainfo
        .get("Width")
        .expect("Width should be specified in mediainfo output")
        .replace(" pixels", "")
        .replace(' ', "")
        .trim()
        .parse()
        .context("failed to parse width")?;
    let height = mediainfo
        .get("Height")
        .expect("Height should be specified in mediainfo output")
        .replace(" pixels", "")
        .replace(' ', "")
        .trim()
        .parse()
        .context("failed to parse height")?;
    let frac_regex = Regex::new(r"\((\d+)\/(\d+)\) FPS").expect("Valid FPS regex");
    let fps_str = mediainfo
        .get("Frame rate")
        .expect("Frame rate should be specified in mediainfo output");
    let fps = if let Some(captures) = frac_regex.captures(fps_str) {
        (
            captures
                .get(1)
                .expect("FPS numerator capture group")
                .as_str()
                .parse::<u32>()
                .context("failed to parse fps numer")?,
            captures
                .get(2)
                .expect("FPS denominator capture group")
                .as_str()
                .parse::<u32>()
                .context("failed to parse fps denom")?,
        )
    } else {
        (
            (fps_str
                .replace(" FPS", "")
                .trim()
                .parse::<f32>()
                .context("failed to parse fps")?
                * 1000.0) as u32,
            1000,
        )
    };
    let bit_depth = mediainfo
        .get("Bit depth")
        .expect("Bit depth should be specified in mediainfo output")
        .replace(" bits", "")
        .parse()
        .context("failed to parse bit depth")?;
    let pixel_format = match mediainfo
        .get("Chroma subsampling")
        .expect("Chroma subsampling should be specified in mediainfo output")
        .as_str()
    {
        "4:2:0" => PixelFormat::Yuv420,
        "4:2:2" => PixelFormat::Yuv422,
        "4:4:4" => PixelFormat::Yuv444,
        _ => bail!("Unimplemented pixel format"),
    };

    Ok(VideoDimensions {
        width,
        height,
        fps,
        frames: get_video_frame_count(input)?,
        pixel_format,
        bit_depth,
    })
}

pub fn get_video_frame_count(input: &Path) -> Result<u32> {
    let command = Command::new("mediainfo")
        .arg("--Output=Video;%FrameCount%")
        .arg(input)
        .output()?;
    let output = String::from_utf8_lossy(&command.stdout);
    output.trim().parse().context("failed to parse frame count")
}

fn get_video_dimensions_vps(input: &Path) -> Result<VideoDimensions> {
    let command = Command::new("vspipe")
        .arg("-i")
        .arg(input)
        .arg("-o")
        .arg("0")
        .arg("-")
        .output()
        .map_err(|e| anyhow!("Failed to execute vspipe -i to get video dimensions: {}", e))?;
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
    let width = lines
        .iter()
        .find(|l| l.starts_with("Width: "))
        .expect("Width line in vspipe output")
        .replace("Width: ", "")
        .trim()
        .parse()?;
    let height = lines
        .iter()
        .find(|l| l.starts_with("Height: "))
        .expect("Height line in vspipe output")
        .replace("Height: ", "")
        .trim()
        .parse()?;
    let fps: Vec<_> = lines
        .iter()
        .find(|l| l.starts_with("FPS: "))
        .expect("FPS line in vspipe output")
        .replace("FPS: ", "")
        .split_whitespace()
        .next()
        .expect("FPS should have a value")
        .split('/')
        .map(|num| num.parse())
        .collect();
    let bit_depth = lines
        .iter()
        .find(|l| l.starts_with("Bits: "))
        .expect("Bits line in vspipe output")
        .replace("Bits: ", "")
        .trim()
        .parse()
        .expect("Bits should be a number");
    Ok(VideoDimensions {
        width,
        height,
        frames: lines
            .iter()
            .find(|l| l.starts_with("Frames: "))
            .expect("Frames line in vspipe output")
            .replace("Frames: ", "")
            .trim()
            .parse()?,
        fps: (fps[0].clone()?, fps[1].clone()?),
        pixel_format: PixelFormat::from_vapoursynth_format(
            &lines
                .iter()
                .find(|l| l.starts_with("Format Name: "))
                .expect("Format Name line in vspipe output")
                .replace("Format Name: ", ""),
        ),
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
            let (key, value) = line
                .split_once(':')
                .expect("String should be a 'key: value' pair");
            (key.trim().to_string(), value.trim().to_string())
        })
        .collect::<HashMap<String, String>>();
    data.insert(
        "File size".to_owned(),
        output
            .lines()
            .find(|l| l.starts_with("File size"))
            .unwrap_or("File size: 0.00 MiB")
            .split_once(':')
            .expect("String should be a 'key: value' pair")
            .1
            .trim()
            .to_string(),
    );

    Ok(data)
}

pub fn find_source_file(input: &Path) -> PathBuf {
    if input
        .extension()
        .map(|ext| ext.to_string_lossy())
        .as_deref()
        != Some("vpy")
    {
        return input.to_path_buf();
    }

    let script = fs::read_to_string(input).expect("Failed to read source script");
    let sources = parse_sources(&script);
    // If there's a source that matches this script's name then use that,
    // otherwise assume the first source is correct.
    // This is mostly for OC merging.
    let source = sources
        .iter()
        .find(|source| source.file_stem() == input.file_stem())
        .unwrap_or_else(|| &sources[0]);
    // Handle relative or absolute paths
    input
        .parent()
        .expect("File should have a parent dir")
        .to_path_buf()
        .join(source)
}

fn parse_sources(script: &str) -> Vec<PathBuf> {
    // If you have a quotation mark in your filename then go to hell
    static PATTERN: OnceCell<Regex> = OnceCell::new();
    let pattern = PATTERN
        .get_or_init(|| Regex::new("source=['\"](.+\\.\\w{2,4})['\"]").expect("Valid regex"));
    pattern
        .captures_iter(script)
        .map(|cap| PathBuf::from(&cap[1]))
        .unique()
        .collect()
}

#[derive(Debug, Clone, Copy)]
pub struct Colorimetry {
    pub range: YUVRange,
    pub primaries: ColorPrimaries,
    pub matrix: MatrixCoefficients,
    pub transfer: TransferCharacteristic,
    pub chroma_location: ChromaLocation,
}

impl Colorimetry {
    pub fn from_path<P: AsRef<Path>>(input: P) -> Result<Self> {
        let env = Environment::from_file(input, EvalFlags::SetWorkingDir).map_err(|e| match e {
            vapoursynth::vsscript::Error::VSScript(e) => {
                anyhow!("An error occurred in VSScript: {}", e)
            }
            _ => anyhow!("{}", e),
        })?;
        let (node, _) = env.get_output(0)?;
        let frame = node.get_frame(0)?;
        let props = frame.props();
        Ok(Colorimetry {
            range: match props.get_int("_ColorRange") {
                Ok(0) => YUVRange::Full,
                _ => YUVRange::Limited,
            },
            primaries: props
                .get_int("_Primaries")
                .map_or(ColorPrimaries::Unspecified, |val| {
                    ColorPrimaries::from_i64(val).unwrap_or(ColorPrimaries::Unspecified)
                }),
            matrix: props
                .get_int("_Matrix")
                .map_or(MatrixCoefficients::Unspecified, |val| {
                    MatrixCoefficients::from_i64(val).unwrap_or(MatrixCoefficients::Unspecified)
                }),
            transfer: props.get_int("_Transfer").map_or(
                TransferCharacteristic::Unspecified,
                |val| {
                    TransferCharacteristic::from_i64(val)
                        .unwrap_or(TransferCharacteristic::Unspecified)
                },
            ),
            chroma_location: match props.get_int("_ChromaLocation") {
                Ok(0) => ChromaLocation::Left,
                Ok(1) => ChromaLocation::Center,
                Ok(2) => ChromaLocation::TopLeft,
                Ok(3) => ChromaLocation::Top,
                Ok(4) => ChromaLocation::BottomLeft,
                Ok(5) => ChromaLocation::Bottom,
                _ => ChromaLocation::Unspecified,
            },
        })
    }

    pub fn is_hdr(self) -> bool {
        self.transfer == TransferCharacteristic::HybridLogGamma
            || self.transfer == TransferCharacteristic::PerceptualQuantizer
    }

    pub fn get_primaries_encoder_string(
        self,
        encoder: VideoEncoderIdent,
    ) -> Result<Cow<'static, str>> {
        match encoder {
            VideoEncoderIdent::Copy => bail!("copy does not support colorimetry args"),
            VideoEncoderIdent::Aom => Ok(Cow::Borrowed(match self.primaries {
                ColorPrimaries::BT709 => "bt709",
                ColorPrimaries::BT470M => "bt470m",
                ColorPrimaries::BT470BG => "bt470bg",
                ColorPrimaries::ST170M | ColorPrimaries::ST240M => "smpte240",
                ColorPrimaries::Film => "film",
                ColorPrimaries::BT2020 => "bt2020",
                ColorPrimaries::ST428 => "xyz",
                ColorPrimaries::P3DCI => "smpte431",
                ColorPrimaries::P3Display => "smpte432",
                ColorPrimaries::Tech3213 => "ebu3213",
                ColorPrimaries::Unspecified => bail!("Color primaries unspecified"),
                x => bail!("Color primaries {x} not implemented for aom"),
            })),
            VideoEncoderIdent::Rav1e => Ok(Cow::Borrowed(match self.primaries {
                ColorPrimaries::BT709 => "BT709",
                ColorPrimaries::BT470M => "BT470M",
                ColorPrimaries::BT470BG => "BT470BG",
                ColorPrimaries::ST170M => "BT601",
                ColorPrimaries::ST240M => "SMPTE240",
                ColorPrimaries::Film => "GenericFilm",
                ColorPrimaries::BT2020 => "BT2020",
                ColorPrimaries::ST428 => "XYZ",
                ColorPrimaries::P3DCI => "SMPTE431",
                ColorPrimaries::P3Display => "SMPTE432",
                ColorPrimaries::Tech3213 => "EBU3213",
                ColorPrimaries::Unspecified => bail!("Color primaries unspecified"),
                x => bail!("Color primaries {x} not implemented for rav1e"),
            })),
            VideoEncoderIdent::SvtAv1 => Ok(self
                .primaries
                .to_u8()
                .expect("Valid primaries enum")
                .to_string()
                .into()),
            VideoEncoderIdent::X264 => Ok(Cow::Borrowed(match self.primaries {
                ColorPrimaries::BT709 => "bt709",
                ColorPrimaries::BT470M => "bt470m",
                ColorPrimaries::BT470BG => "bt470bg",
                ColorPrimaries::ST170M => "smpte170m",
                ColorPrimaries::ST240M => "smpte240m",
                ColorPrimaries::Film => "film",
                ColorPrimaries::BT2020 => "bt2020",
                ColorPrimaries::ST428 => "smpte428",
                ColorPrimaries::P3DCI => "smpte431",
                ColorPrimaries::P3Display => "smpte432",
                ColorPrimaries::Unspecified => bail!("Color primaries unspecified"),
                x => bail!("Color primaries {x} not implemented for x264"),
            })),
            VideoEncoderIdent::X265 => Ok(Cow::Borrowed(match self.primaries {
                ColorPrimaries::BT709 => "bt709",
                ColorPrimaries::BT470M => "bt470m",
                ColorPrimaries::BT470BG => "bt470bg",
                ColorPrimaries::ST170M => "smpte170m",
                ColorPrimaries::ST240M => "smpte240m",
                ColorPrimaries::Film => "film",
                ColorPrimaries::BT2020 => "bt2020",
                ColorPrimaries::ST428 => "smpte428",
                ColorPrimaries::P3DCI => "smpte431",
                ColorPrimaries::P3Display => "smpte432",
                ColorPrimaries::Unspecified => bail!("Color primaries unspecified"),
                x => bail!("Color primaries {x} not implemented for x265"),
            })),
        }
    }

    pub fn get_matrix_encoder_string(
        self,
        encoder: VideoEncoderIdent,
    ) -> Result<Cow<'static, str>> {
        match encoder {
            VideoEncoderIdent::Copy => bail!("copy does not support colorimetry args"),
            VideoEncoderIdent::Aom => Ok(Cow::Borrowed(match self.matrix {
                MatrixCoefficients::Identity => "identity",
                MatrixCoefficients::BT709 => "bt709",
                MatrixCoefficients::BT470M => "fcc73",
                MatrixCoefficients::BT470BG => "bt470bg",
                MatrixCoefficients::ST170M => "bt601",
                MatrixCoefficients::ST240M => "smpte240",
                MatrixCoefficients::YCgCo => "ycgco",
                MatrixCoefficients::BT2020NonConstantLuminance => "bt2020ncl",
                MatrixCoefficients::BT2020ConstantLuminance => "bt2020cl",
                MatrixCoefficients::ST2085 => "smpte2085",
                MatrixCoefficients::ChromaticityDerivedNonConstantLuminance => "chromncl",
                MatrixCoefficients::ChromaticityDerivedConstantLuminance => "chromcl",
                MatrixCoefficients::ICtCp => "ictcp",
                MatrixCoefficients::Unspecified => bail!("Matrix coefficients unspecified"),
                x => bail!("Matrix coefficients {x} not implemented for aom"),
            })),
            VideoEncoderIdent::Rav1e => Ok(Cow::Borrowed(match self.matrix {
                MatrixCoefficients::Identity => "Identity",
                MatrixCoefficients::BT709 => "BT709",
                MatrixCoefficients::BT470M => "FCC",
                MatrixCoefficients::BT470BG => "BT470BG",
                MatrixCoefficients::ST170M => "BT601",
                MatrixCoefficients::ST240M => "SMPTE240",
                MatrixCoefficients::YCgCo => "YCgCo",
                MatrixCoefficients::BT2020NonConstantLuminance => "BT2020NCL",
                MatrixCoefficients::BT2020ConstantLuminance => "BT2020CL",
                MatrixCoefficients::ST2085 => "SMPTE2085",
                MatrixCoefficients::ChromaticityDerivedNonConstantLuminance => "ChromatNCL",
                MatrixCoefficients::ChromaticityDerivedConstantLuminance => "ChromatCL",
                MatrixCoefficients::ICtCp => "ICtCp",
                MatrixCoefficients::Unspecified => bail!("Matrix coefficients unspecified"),
                x => bail!("Matrix coefficients {x} not implemented for rav1e"),
            })),
            VideoEncoderIdent::SvtAv1 => Ok(self
                .matrix
                .to_u8()
                .expect("Valid matrix enum")
                .to_string()
                .into()),
            VideoEncoderIdent::X264 => Ok(Cow::Borrowed(match self.matrix {
                MatrixCoefficients::Identity => "GBR",
                MatrixCoefficients::BT709 => "bt709",
                MatrixCoefficients::BT470M => "fcc",
                MatrixCoefficients::BT470BG => "bt470bg",
                MatrixCoefficients::ST170M => "smpte170m",
                MatrixCoefficients::ST240M => "smpte240m",
                MatrixCoefficients::YCgCo => "YCgCo",
                MatrixCoefficients::BT2020NonConstantLuminance => "bt2020nc",
                MatrixCoefficients::BT2020ConstantLuminance => "bt2020c",
                MatrixCoefficients::ST2085 => "smpte2085",
                MatrixCoefficients::ChromaticityDerivedNonConstantLuminance => "chroma-derived-nc",
                MatrixCoefficients::ChromaticityDerivedConstantLuminance => "chroma-derived-c",
                MatrixCoefficients::ICtCp => "ICtCp",
                MatrixCoefficients::Unspecified => bail!("Matrix coefficients unspecified"),
                x => bail!("Matrix coefficients {x} not implemented for x264"),
            })),
            VideoEncoderIdent::X265 => Ok(Cow::Borrowed(match self.matrix {
                MatrixCoefficients::Identity => "gbr",
                MatrixCoefficients::BT709 => "bt709",
                MatrixCoefficients::BT470M => "fcc",
                MatrixCoefficients::BT470BG => "bt470bg",
                MatrixCoefficients::ST170M => "smpte170m",
                MatrixCoefficients::ST240M => "smpte240m",
                MatrixCoefficients::YCgCo => "ycgco",
                MatrixCoefficients::BT2020NonConstantLuminance => "bt2020nc",
                MatrixCoefficients::BT2020ConstantLuminance => "bt2020c",
                MatrixCoefficients::ST2085 => "smpte2085",
                MatrixCoefficients::ChromaticityDerivedNonConstantLuminance => "chroma-derived-nc",
                MatrixCoefficients::ChromaticityDerivedConstantLuminance => "chroma-derived-c",
                MatrixCoefficients::ICtCp => "ictcp",
                MatrixCoefficients::Unspecified => bail!("Matrix coefficients unspecified"),
                x => bail!("Matrix coefficients {x} not implemented for x265"),
            })),
        }
    }

    pub fn get_transfer_encoder_string(
        self,
        encoder: VideoEncoderIdent,
    ) -> Result<Cow<'static, str>> {
        match encoder {
            VideoEncoderIdent::Copy => bail!("copy does not support colorimetry args"),
            VideoEncoderIdent::Aom => Ok(Cow::Borrowed(match self.transfer {
                TransferCharacteristic::BT1886 => "bt709",
                TransferCharacteristic::BT470M => "bt470m",
                TransferCharacteristic::BT470BG => "bt470bg",
                TransferCharacteristic::ST170M => "bt601",
                TransferCharacteristic::ST240M => "smpte240",
                TransferCharacteristic::Linear => "lin",
                TransferCharacteristic::Logarithmic100 => "log100",
                TransferCharacteristic::Logarithmic316 => "log100sq10",
                TransferCharacteristic::XVYCC => "iec61966",
                TransferCharacteristic::BT1361E => "bt1361",
                TransferCharacteristic::SRGB => "srgb",
                TransferCharacteristic::BT2020Ten => "bt2020-10bit",
                TransferCharacteristic::BT2020Twelve => "bt2020-12bit",
                TransferCharacteristic::PerceptualQuantizer => "smpte2084",
                TransferCharacteristic::ST428 => "smpte428",
                TransferCharacteristic::HybridLogGamma => "hlg",
                TransferCharacteristic::Unspecified => {
                    bail!("Transfer characteristics unspecified")
                }
                x => bail!("Transfer characteristics {x} not implemented for aom"),
            })),
            VideoEncoderIdent::Rav1e => Ok(Cow::Borrowed(match self.transfer {
                TransferCharacteristic::BT1886 => "BT709",
                TransferCharacteristic::BT470M => "BT470M",
                TransferCharacteristic::BT470BG => "BT470BG",
                TransferCharacteristic::ST170M => "BT601",
                TransferCharacteristic::ST240M => "SMPTE240",
                TransferCharacteristic::Linear => "Linear",
                TransferCharacteristic::Logarithmic100 => "Log100",
                TransferCharacteristic::Logarithmic316 => "Log100Sqrt10",
                TransferCharacteristic::XVYCC => "IEC61966",
                TransferCharacteristic::BT1361E => "BT1361",
                TransferCharacteristic::SRGB => "SRGB",
                TransferCharacteristic::BT2020Ten => "BT2020_10Bit",
                TransferCharacteristic::BT2020Twelve => "BT2020_12Bit",
                TransferCharacteristic::PerceptualQuantizer => "SMPTE2084",
                TransferCharacteristic::ST428 => "SMPTE428",
                TransferCharacteristic::HybridLogGamma => "HLG",
                TransferCharacteristic::Unspecified => {
                    bail!("Transfer characteristics unspecified")
                }
                x => bail!("Transfer characteristics {x} not implemented for rav1e"),
            })),
            VideoEncoderIdent::SvtAv1 => Ok(self
                .transfer
                .to_u8()
                .expect("Valid transfer enum")
                .to_string()
                .into()),
            VideoEncoderIdent::X264 => Ok(Cow::Borrowed(match self.transfer {
                TransferCharacteristic::BT1886 => "bt709",
                TransferCharacteristic::BT470M => "bt470m",
                TransferCharacteristic::BT470BG => "bt470bg",
                TransferCharacteristic::ST170M => "smpte170m",
                TransferCharacteristic::ST240M => "smpte240m",
                TransferCharacteristic::Linear => "linear",
                TransferCharacteristic::Logarithmic100 => "log100",
                TransferCharacteristic::Logarithmic316 => "log316",
                TransferCharacteristic::XVYCC => "iec61966-2-4",
                TransferCharacteristic::BT1361E => "bt1361e",
                TransferCharacteristic::SRGB => "iec61966-2-1",
                TransferCharacteristic::BT2020Ten => "bt2020-10",
                TransferCharacteristic::BT2020Twelve => "bt2020-12",
                TransferCharacteristic::PerceptualQuantizer => "smpte2084",
                TransferCharacteristic::ST428 => "smpte428",
                TransferCharacteristic::HybridLogGamma => "arib-std-b67",
                TransferCharacteristic::Unspecified => {
                    bail!("Transfer characteristics unspecified")
                }
                x => unimplemented!("Transfer characteristics {x} not implemented for x264"),
            })),
            VideoEncoderIdent::X265 => Ok(Cow::Borrowed(match self.transfer {
                TransferCharacteristic::BT1886 => "bt709",
                TransferCharacteristic::BT470M => "bt470m",
                TransferCharacteristic::BT470BG => "bt470bg",
                TransferCharacteristic::ST170M => "smpte170m",
                TransferCharacteristic::ST240M => "smpte240m",
                TransferCharacteristic::Linear => "linear",
                TransferCharacteristic::Logarithmic100 => "log100",
                TransferCharacteristic::Logarithmic316 => "log316",
                TransferCharacteristic::XVYCC => "iec61966-2-4",
                TransferCharacteristic::BT1361E => "bt1361e",
                TransferCharacteristic::SRGB => "iec61966-2-1",
                TransferCharacteristic::BT2020Ten => "bt2020-10",
                TransferCharacteristic::BT2020Twelve => "bt2020-12",
                TransferCharacteristic::PerceptualQuantizer => "smpte2084",
                TransferCharacteristic::ST428 => "smpte428",
                TransferCharacteristic::HybridLogGamma => "arib-std-b67",
                TransferCharacteristic::Unspecified => {
                    bail!("Transfer characteristics unspecified")
                }
                x => bail!("Transfer characteristics {x} not implemented for x265"),
            })),
        }
    }

    pub fn get_range_encoder_string(self, encoder: VideoEncoderIdent) -> Result<Cow<'static, str>> {
        match encoder {
            VideoEncoderIdent::Copy => bail!("copy does not support colorimetry args"),
            VideoEncoderIdent::Aom => bail!("aom does not support a range argument"),
            VideoEncoderIdent::Rav1e => Ok(Cow::Borrowed(match self.range {
                YUVRange::Limited => "Limited",
                YUVRange::Full => "Full",
            })),
            VideoEncoderIdent::SvtAv1 => Ok(Cow::Borrowed(match self.range {
                YUVRange::Limited => "0",
                YUVRange::Full => "1",
            })),
            VideoEncoderIdent::X264 => Ok(Cow::Borrowed(match self.range {
                YUVRange::Limited => "tv",
                YUVRange::Full => "pc",
            })),
            VideoEncoderIdent::X265 => Ok(Cow::Borrowed(match self.range {
                YUVRange::Limited => "limited",
                YUVRange::Full => "full",
            })),
        }
    }

    pub fn get_chromaloc_encoder_string(
        self,
        encoder: VideoEncoderIdent,
    ) -> Result<Cow<'static, str>> {
        match encoder {
            VideoEncoderIdent::Copy => bail!("copy does not support colorimetry args"),
            VideoEncoderIdent::Aom => Ok(Cow::Borrowed(match self.chroma_location {
                ChromaLocation::Left => "left",
                ChromaLocation::TopLeft => "colocated",
                _ => "unknown",
            })),
            VideoEncoderIdent::Rav1e => bail!("rav1e does not support a chroma loc argument"),
            VideoEncoderIdent::SvtAv1 => Ok(Cow::Borrowed(match self.chroma_location {
                ChromaLocation::TopLeft => "topleft",
                ChromaLocation::Left => "left",
                _ => "unknown",
            })),
            VideoEncoderIdent::X264 => Ok(Cow::Borrowed(match self.chroma_location {
                ChromaLocation::Left => " --chromaloc 0",
                ChromaLocation::Center => " --chromaloc 1",
                ChromaLocation::TopLeft => " --chromaloc 2",
                ChromaLocation::Top => " --chromaloc 3",
                ChromaLocation::BottomLeft => " --chromaloc 4",
                ChromaLocation::Bottom => " --chromaloc 5",
                _ => "",
            })),
            VideoEncoderIdent::X265 => Ok(Cow::Borrowed(match self.chroma_location {
                ChromaLocation::Left => " --chromaloc 0",
                ChromaLocation::Center => " --chromaloc 1",
                ChromaLocation::TopLeft => " --chromaloc 2",
                ChromaLocation::Top => " --chromaloc 3",
                ChromaLocation::BottomLeft => " --chromaloc 4",
                ChromaLocation::Bottom => " --chromaloc 5",
                _ => "",
            })),
        }
    }
}

pub fn get_audio_delay_ms(input: &Path, track: usize) -> Result<i32> {
    let command = Command::new("mediainfo")
        .arg("--Output=Audio;%Delay%,")
        .arg(input)
        .output()?;
    let output = String::from_utf8_lossy(&command.stdout);
    Ok(output
        .split(',')
        .filter(|p| !p.trim().is_empty())
        .nth(track)
        .unwrap_or_else(|| panic!("Expected {} tracks, did not find enough", track + 1))
        .parse::<i32>()?)
}

// Returns `Some(track_num)` if audio found
pub fn vspipe_has_audio(input: &Path) -> Result<Option<usize>> {
    let command = Command::new("vspipe")
        .arg("-i")
        .arg(input)
        .arg("-")
        .output()
        .map_err(|e| anyhow!("Failed to execute vspipe -i to get video dimensions: {}", e))?;
    let output = String::from_utf8_lossy(&command.stdout);
    let mut iter = output.lines().peekable();
    while let Some(line) = iter.next() {
        let line = line.trim();
        if line.starts_with("Output Index:") {
            if let Some(next_line) = iter.peek() {
                if next_line.trim() == "Type: Audio" {
                    let (_, index) = line.split_once(": ").expect("Valid index line format");
                    return Ok(Some(index.parse()?));
                }
            }
        }
    }
    Ok(None)
}
