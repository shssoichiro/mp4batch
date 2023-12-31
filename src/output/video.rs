use std::{
    fmt::Display,
    num::NonZeroUsize,
    path::Path,
    process::{Command, Stdio},
    str::FromStr,
    thread::available_parallelism,
};

use ansi_term::Colour::{Green, Yellow};
use anyhow::Result;
use av_data::pixel::{
    ChromaLocation,
    ColorPrimaries,
    MatrixCoefficients,
    ToPrimitive,
    TransferCharacteristic,
    YUVRange,
};

use crate::{
    absolute_path,
    input::{get_video_frame_count, Colorimetry, PixelFormat, VideoDimensions},
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
        write!(f, "{}", match self {
            Profile::Film => "film",
            Profile::Anime => "anime",
            Profile::Fast => "fast",
        })
    }
}

pub fn extract_video(input: &Path, output: &Path) -> Result<()> {
    let mut command = Command::new("ffmpeg");
    command
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("level+error")
        .arg("-stats")
        .arg("-y")
        .arg("-i")
        .arg(input)
        .arg("-vcodec")
        .arg("copy")
        .arg("-map")
        .arg("0:v:0")
        .arg(output);

    let status = command
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to execute ffmpeg: {}", e))?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("Failed to execute ffmpeg");
    }
}

pub fn create_lossless(
    input: &Path,
    dimensions: VideoDimensions,
    verify_frame_count: bool,
) -> Result<()> {
    let lossless_filename = input.with_extension("lossless.mkv");
    if lossless_filename.exists() {
        if let Ok(lossless_frames) = get_video_frame_count(&lossless_filename) {
            // We use a fuzzy frame count check because *some cursed sources*
            // report a different frame count from the number of actual decodeable frames.
            let diff = (lossless_frames as i64 - dimensions.frames as i64).unsigned_abs() as u32;
            let allowance = dimensions.frames / 200;
            if !verify_frame_count || diff <= allowance {
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

    let filename = input
        .file_name()
        .expect("File should have a name")
        .to_string_lossy();
    let pipe = if filename.ends_with(".vpy") {
        Command::new("vspipe")
            .arg("-c")
            .arg("y4m")
            .arg(input)
            .arg("-")
            .stdout(Stdio::piped())
            .spawn()
            .expect("Unable to run vspipe, is it installed and in PATH?")
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
        .arg("superfast")
        .arg("-qp")
        .arg("0")
        .arg(&lossless_filename)
        .stdin(pipe.stdout.expect("stdout should be writeable"))
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to execute ffmpeg: {}", e))?;
    if !status.success() {
        anyhow::bail!(
            "Failed to execute ffmpeg: Exited with code {:x}",
            status.code().unwrap_or(-1)
        );
    }

    if let Ok(lossless_frames) = get_video_frame_count(&lossless_filename) {
        if verify_frame_count {
            // We use a fuzzy frame count check because *some cursed sources*
            // report a different frame count from the number of actual decodeable frames.
            let diff = (lossless_frames as i64 - dimensions.frames as i64).unsigned_abs() as u32;
            let allowance = dimensions.frames / 200;
            if diff > allowance {
                anyhow::bail!("Incomplete lossless encode");
            }
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
    force_keyframes: &Option<String>,
    colorimetry: &Colorimetry,
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
    let tiles = NonZeroUsize::new(
        if dimensions.height >= 2000 || (dimensions.height >= 1550 && dimensions.width >= 3600) {
            2
        } else {
            1
        } * if dimensions.width >= 2000 { 2 } else { 1 },
    )
    .expect("not 0");
    let cores = available_parallelism().expect("Unable to get machine parallelism count");
    let workers = NonZeroUsize::new(match encoder {
        VideoEncoder::Aom { .. } | VideoEncoder::Rav1e { .. } | VideoEncoder::SvtAv1 { .. } => {
            std::cmp::max(cores.get() / tiles.get(), 1)
        }
        _ => (std::cmp::max(cores.get() / tiles.get(), 1) / 4).max(1),
    })
    .unwrap();
    assert!(
        workers <= cores,
        "Worker count exceeded core count, this is a bug"
    );

    let threads_per_worker = NonZeroUsize::new(std::cmp::min(
        64,
        (cores.get() as f32 / workers.get() as f32 * 1.5).ceil() as usize + 2,
    ))
    .unwrap();
    let mut command = Command::new("nice");
    command
        .arg("av1an")
        .arg("-i")
        .arg(absolute_path(vpy_input).expect("Unable to get absolute path"))
        .arg("-e")
        .arg(encoder.get_av1an_name())
        .arg("-v")
        .arg(&encoder.get_args_string(dimensions, colorimetry, threads_per_worker, cores, workers))
        .arg("--sc-method")
        .arg("standard")
        .arg("-x")
        .arg(
            match encoder {
                VideoEncoder::Aom { profile, .. }
                | VideoEncoder::Rav1e { profile, .. }
                | VideoEncoder::SvtAv1 { profile, .. }
                | VideoEncoder::X264 { profile, .. }
                | VideoEncoder::X265 { profile, .. } => match profile {
                    Profile::Film | Profile::Fast => fps * 10,
                    Profile::Anime => fps * 15,
                },
                VideoEncoder::Copy => unreachable!(),
            }
            .to_string(),
        )
        .arg("--min-scene-len")
        .arg(
            match encoder {
                VideoEncoder::Aom { profile, .. }
                | VideoEncoder::Rav1e { profile, .. }
                | VideoEncoder::SvtAv1 { profile, .. }
                | VideoEncoder::X264 { profile, .. }
                | VideoEncoder::X265 { profile, .. } => match profile {
                    Profile::Film | Profile::Fast => fps,
                    Profile::Anime => fps / 2,
                },
                VideoEncoder::Copy => unreachable!(),
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
        .arg(absolute_path(output).expect("Unable to get absolute path"));
    if let Some(force_keyframes) = force_keyframes {
        command.arg("--force-keyframes").arg(force_keyframes);
    }
    if dimensions.height > 1080 {
        command.arg("--sc-downscale-height").arg("1080");
    }
    if encoder.uses_av1an_thread_pinning() {
        command
            .arg("--set-thread-affinity")
            .arg((cores.get() / workers).to_string());
    }
    if let VideoEncoder::Aom { grain, .. }
    | VideoEncoder::Rav1e { grain, .. }
    | VideoEncoder::SvtAv1 { grain, .. } = encoder
    {
        if grain > 0 {
            command
                .arg("--photon-noise")
                .arg(grain.to_string())
                .arg("--chroma-noise");
        }
    }
    if let VideoEncoder::X265 { .. } = encoder {
        command.arg("--concat").arg("mkvmerge");
    }
    let status = command
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to execute av1an: {}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "Failed to execute av1an: Exited with code {:x}",
            status.code().unwrap_or(-1)
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoEncoder {
    Copy,
    Aom {
        crf: i16,
        speed: u8,
        profile: Profile,
        grain: u8,
        compat: bool,
    },
    Rav1e {
        crf: i16,
        speed: u8,
        profile: Profile,
        grain: u8,
    },
    SvtAv1 {
        crf: i16,
        speed: u8,
        profile: Profile,
        grain: u8,
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
    },
}

impl VideoEncoder {
    pub const fn supported_encoders() -> &'static [&'static str] {
        &["aom", "rav1e", "svt", "x264", "x265", "copy"]
    }

    pub const fn get_av1an_name(&self) -> &str {
        match self {
            VideoEncoder::Copy => "copy",
            VideoEncoder::Aom { .. } => "aom",
            VideoEncoder::Rav1e { .. } => "rav1e",
            VideoEncoder::SvtAv1 { .. } => "svt-av1",
            VideoEncoder::X264 { .. } => "x264",
            VideoEncoder::X265 { .. } => "x265",
        }
    }

    pub fn get_args_string(
        self,
        dimensions: VideoDimensions,
        colorimetry: &Colorimetry,
        computed_threads: NonZeroUsize,
        cores: NonZeroUsize,
        workers: NonZeroUsize,
    ) -> String {
        match self {
            VideoEncoder::Aom {
                crf,
                speed,
                profile,
                ..
            } => build_aom_args_string(
                crf,
                speed,
                dimensions,
                profile,
                colorimetry,
                computed_threads,
            ),
            VideoEncoder::Rav1e { crf, speed, .. } => {
                build_rav1e_args_string(crf, speed, dimensions, colorimetry)
            }
            VideoEncoder::SvtAv1 {
                crf,
                speed,
                profile,
                ..
            } => build_svtav1_args_string(
                crf,
                speed,
                cores.get() / workers.get(),
                dimensions,
                profile,
                colorimetry,
            ),
            VideoEncoder::X264 {
                crf,
                profile,
                compat,
            } => build_x264_args_string(
                crf,
                dimensions,
                profile,
                compat,
                colorimetry,
                computed_threads,
            ),
            VideoEncoder::X265 {
                crf,
                profile,
                compat,
                ..
            } => build_x265_args_string(
                crf,
                dimensions,
                profile,
                compat,
                colorimetry,
                computed_threads,
            ),
            VideoEncoder::Copy => unreachable!(),
        }
    }

    pub const fn uses_av1an_thread_pinning(self) -> bool {
        matches!(
            self,
            VideoEncoder::Aom { .. } | VideoEncoder::SvtAv1 { .. } | VideoEncoder::Rav1e { .. }
        )
    }
}

fn build_aom_args_string(
    crf: i16,
    speed: u8,
    dimensions: VideoDimensions,
    profile: Profile,
    colorimetry: &Colorimetry,
    threads: NonZeroUsize,
) -> String {
    // Note: aom doesn't have a parameter to control full vs limited range
    let bd = dimensions.bit_depth;
    let tile_cols = i32::from(dimensions.width >= 2000);
    let tile_rows = i32::from(
        dimensions.height >= 2000 || (dimensions.height >= 1550 && dimensions.width >= 3600),
    );
    let arnr_str = if profile == Profile::Anime { 1 } else { 3 };
    let deltaq_mode = if colorimetry.is_hdr() { 5 } else { 1 };
    let prim = match colorimetry.primaries {
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
        ColorPrimaries::Unspecified => panic!("Color primaries unspecified"),
        _ => unimplemented!("Color primaries not implemented for aom"),
    };
    let transfer = match colorimetry.transfer {
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
        TransferCharacteristic::Unspecified => panic!("Transfer characteristics unspecified"),
        _ => unimplemented!("Transfer characteristics not implemented for aom"),
    };
    let matrix = match colorimetry.matrix {
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
        MatrixCoefficients::Unspecified => panic!("Matrix coefficients unspecified"),
        _ => unimplemented!("Matrix coefficients not implemented for aom"),
    };
    let csp = match colorimetry.chroma_location {
        ChromaLocation::Top => "vertical",
        ChromaLocation::Center => "colocated",
        _ => "unknown",
    };
    format!(
        " -b {bd} --end-usage=q --min-q=1 --lag-in-frames=64 --cpu-used={speed} --cq-level={crf} \
         --disable-kf --kf-max-dist=9999 --enable-fwd-kf=0 --sharpness=3 --row-mt=0 \
         --tile-columns={tile_cols} --tile-rows={tile_rows} --arnr-maxframes=15 \
         --arnr-strength={arnr_str} --tune=ssim  --enable-chroma-deltaq=1 \
         --disable-trellis-quant=0 --enable-qm=1 --qm-min=0 --qm-max=8 --quant-b-adapt=1 \
         --aq-mode=0 --deltaq-mode={deltaq_mode} --tune-content=psy --sb-size=dynamic \
         --enable-dnl-denoising=0 --color-primaries={prim} --transfer-characteristics={transfer} \
         --matrix-coefficients={matrix} --chroma-sample-position={csp}  --threads={threads} "
    )
}

fn build_rav1e_args_string(
    crf: i16,
    speed: u8,
    dimensions: VideoDimensions,
    colorimetry: &Colorimetry,
) -> String {
    // TODO: Add proper HDR metadata
    // TODO: Remove rdo-lookahead-frames limitation if we can reduce rav1e memory
    // usage
    let tile_cols = i32::from(dimensions.width >= 2000);
    let tile_rows = i32::from(
        dimensions.height >= 2000 || (dimensions.height >= 1550 && dimensions.width >= 3600),
    );
    let prim = match colorimetry.primaries {
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
        ColorPrimaries::Unspecified => panic!("Color primaries unspecified"),
        _ => unimplemented!("Color primaries not implemented for rav1e"),
    };
    let matrix = match colorimetry.matrix {
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
        MatrixCoefficients::Unspecified => panic!("Matrix coefficients unspecified"),
        _ => unimplemented!("Matrix coefficients not implemented for rav1e"),
    };
    let transfer = match colorimetry.transfer {
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
        TransferCharacteristic::Unspecified => panic!("Transfer characteristics unspecified"),
        _ => unimplemented!("Transfer characteristics not implemented for rav1e"),
    };
    let range = match colorimetry.range {
        YUVRange::Limited => "Limited",
        YUVRange::Full => "Full",
    };
    format!(
        " --speed {speed} --quantizer {crf} --tile-cols {tile_cols} --tile-rows {tile_rows} \
         --primaries {prim} --matrix {matrix}  --transfer {transfer} --range {range} \
         --rdo-lookahead-frames 25 --no-scene-detection --keyint 0 "
    )
}

fn build_svtav1_args_string(
    crf: i16,
    speed: u8,
    threads: usize,
    dimensions: VideoDimensions,
    profile: Profile,
    colorimetry: &Colorimetry,
) -> String {
    let depth = dimensions.bit_depth;
    let tile_cols = i32::from(dimensions.width >= 2000);
    let tile_rows = i32::from(
        dimensions.height >= 2000 || (dimensions.height >= 1550 && dimensions.width >= 3600),
    );
    let tune = if profile == Profile::Anime { "2" } else { "0" };
    let prim = colorimetry.primaries.to_u8().unwrap();
    let matrix = colorimetry.matrix.to_u8().unwrap();
    let transfer = colorimetry.transfer.to_u8().unwrap();
    let range = match colorimetry.range {
        YUVRange::Limited => 0,
        YUVRange::Full => 1,
    };
    let csp = match colorimetry.chroma_location {
        ChromaLocation::Top => "vertical",
        ChromaLocation::Center => "colocated",
        ChromaLocation::TopLeft => "topleft",
        ChromaLocation::Left => "left",
        _ => "unknown",
    };
    format!(
        " --input-depth {depth} --scm 0 --preset {speed} --crf {crf} --film-grain-denoise 0 \
         --tile-columns {tile_cols} --tile-rows {tile_rows} --rc 0 --bias-pct 100 \
         --maxsection-pct 10000 --enable-qm 1 --qm-min 0 --qm-max 8 --irefresh-type 1 --tune \
         {tune} --enable-tf 0 --scd 0 --keyint -1 --lp {threads} --pin 0 --color-primaries {prim} \
         --matrix-coefficients {matrix} --transfer-characteristics {transfer} --color-range \
         {range} --chroma-sample-position {csp} "
    )
}

fn build_x265_args_string(
    crf: i16,
    dimensions: VideoDimensions,
    profile: Profile,
    compat: bool,
    colorimetry: &Colorimetry,
    threads: NonZeroUsize,
) -> String {
    // TODO: Add full HDR metadata

    let deblock = match profile {
        Profile::Film => -2,
        Profile::Anime | Profile::Fast => -1,
    };
    let chroma_offset = match profile {
        Profile::Film | Profile::Fast => 0,
        Profile::Anime => -2,
    };
    let bframes = match profile {
        Profile::Film => 5,
        Profile::Anime => 8,
        Profile::Fast => 3,
    };
    let sao = if (profile == Profile::Anime && crf >= 17) || crf >= 19 {
        "--limit-sao"
    } else {
        "--no-sao"
    };
    let psy_rd = match profile {
        Profile::Film => "1.5",
        Profile::Anime | Profile::Fast => "1.0",
    };
    let psy_rdo = match profile {
        Profile::Film => "3.0",
        Profile::Anime | Profile::Fast => "1.5",
    };
    let aq_str = match profile {
        Profile::Film => "0.8",
        Profile::Anime | Profile::Fast => "0.7",
    };
    let prim = match colorimetry.primaries {
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
        ColorPrimaries::Unspecified => panic!("Color primaries unspecified"),
        _ => unimplemented!("Color primaries not implemented for x265"),
    };
    let matrix = match colorimetry.matrix {
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
        MatrixCoefficients::Unspecified => panic!("Matrix coefficients unspecified"),
        _ => unimplemented!("Matrix coefficients not implemented for x265"),
    };
    let transfer = match colorimetry.transfer {
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
        TransferCharacteristic::Unspecified => panic!("Transfer characteristics unspecified"),
        _ => unimplemented!("Transfer characteristics not implemented for x265"),
    };
    let range = match colorimetry.range {
        YUVRange::Limited => "limited",
        YUVRange::Full => "full",
    };
    let csp = match colorimetry.chroma_location {
        ChromaLocation::Left => " --chromaloc 0",
        ChromaLocation::Center => " --chromaloc 1",
        ChromaLocation::TopLeft => " --chromaloc 2",
        ChromaLocation::Top => " --chromaloc 3",
        ChromaLocation::BottomLeft => " --chromaloc 4",
        ChromaLocation::Bottom => " --chromaloc 5",
        _ => "",
    };
    let depth = dimensions.bit_depth;
    let level = if compat {
        if dimensions.bit_depth == 10 {
            "--profile main10 --level-idc 5.1"
        } else {
            "--profile main --level-idc 5.1"
        }
    } else {
        ""
    };
    let hdr: &str = if colorimetry.is_hdr() {
        "--hdr10-opt"
    } else {
        ""
    };
    format!(
        " --crf {crf} --preset slow --bframes {bframes} --keyint -1 --min-keyint 1 --no-scenecut \
         {sao} --deblock {deblock}:{deblock} --psy-rd {psy_rd} --psy-rdoq {psy_rdo} --qcomp 0.65 \
         --aq-mode 3 --aq-strength {aq_str} --cbqpoffs {chroma_offset} --crqpoffs {chroma_offset} \
         --no-open-gop --no-cutree --rc-lookahead 60 --lookahead-slices 1 --lookahead-threads \
         {threads} --weightb --b-intra --tu-intra-depth 2 --tu-inter-depth 2 --limit-tu 1 \
         --no-limit-modes --no-strong-intra-smoothing --limit-refs 1 --colorprim {prim} \
         --colormatrix {matrix} --transfer {transfer} --range {range} {csp} --output-depth \
         {depth} --frame-threads {threads} --y4m {level} {hdr} "
    )
}

fn build_x264_args_string(
    crf: i16,
    dimensions: VideoDimensions,
    profile: Profile,
    compat: bool,
    colorimetry: &Colorimetry,
    threads: NonZeroUsize,
) -> String {
    let preset = if profile == Profile::Fast {
        "faster"
    } else {
        "veryslow"
    };
    let bframes = match profile {
        Profile::Film => 5,
        Profile::Anime => 8,
        Profile::Fast => 3,
    };
    let psy_rd = match profile {
        Profile::Film => format!("{:.1}:{:.1}", 1.0, 0.0),
        Profile::Anime => format!("{:.1}:{:.1}", 0.7, 0.0),
        Profile::Fast => format!("{:.1}:{:.1}", 0.0, 0.0),
    };
    let deblock = match profile {
        Profile::Film => format!("{}:{}", -3, -3),
        Profile::Anime => format!("{}:{}", -2, -1),
        Profile::Fast => format!("{}:{}", 0, 0),
    };
    let merange = if dimensions.width > 1440 {
        48
    } else if dimensions.width > 1024 {
        32
    } else {
        24
    };
    let aq_str = match profile {
        Profile::Film => 0.8,
        Profile::Anime | Profile::Fast => 0.7,
    };
    let mbtree = match profile {
        // mbtree works fine on live action, but on anime it has undesirable effects
        Profile::Anime => "--no-mbtree",
        _ => "",
    };
    let qcomp = match profile {
        Profile::Film | Profile::Fast => 0.75,
        Profile::Anime => 0.65,
    };
    let prim = match colorimetry.primaries {
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
        ColorPrimaries::Unspecified => panic!("Color primaries unspecified"),
        _ => unimplemented!("Color primaries not implemented for x264"),
    };
    let matrix = match colorimetry.matrix {
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
        MatrixCoefficients::Unspecified => panic!("Matrix coefficients unspecified"),
        _ => unimplemented!("Matrix coefficients not implemented for x264"),
    };
    let transfer = match colorimetry.transfer {
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
        TransferCharacteristic::Unspecified => panic!("Transfer characteristics unspecified"),
        _ => unimplemented!("Transfer characteristics not implemented for x264"),
    };
    let range = match colorimetry.range {
        YUVRange::Limited => "tv",
        YUVRange::Full => "pc",
    };
    let csp = match colorimetry.chroma_location {
        ChromaLocation::Left => " --chromaloc 0",
        ChromaLocation::Center => " --chromaloc 1",
        ChromaLocation::TopLeft => " --chromaloc 2",
        ChromaLocation::Top => " --chromaloc 3",
        ChromaLocation::BottomLeft => " --chromaloc 4",
        ChromaLocation::Bottom => " --chromaloc 5",
        _ => "",
    };
    let depth = dimensions.bit_depth;
    let vbv = if compat {
        "--level 4.1 --vbv-maxrate 50000 --vbv-bufsize 78125"
    } else {
        ""
    };
    let level = match dimensions.pixel_format {
        PixelFormat::Yuv422 => "--profile high422 --output-csp i422",
        PixelFormat::Yuv444 => "--profile high444 --output-csp i444",
        _ => "",
    };
    format!(
        " --crf {crf} --preset {preset} --bframes {bframes} --psy-rd {psy_rd} --deblock {deblock} \
         --merange {merange} --rc-lookahead 96 --aq-mode 3 --aq-strength {aq_str} {mbtree} -i 1 \
         -I infinite --no-scenecut --qcomp {qcomp} --ipratio 1.30 --pbratio 1.20 --no-fast-pskip \
         --no-dct-decimate --colorprim {prim} --colormatrix {matrix} --transfer {transfer} \
         --range {range} {csp} --output-depth {depth} {vbv} {level} --threads {threads} "
    )
}

pub fn copy_hdr_data(input: &Path, target: &Path) -> Result<()> {
    let status = Command::new("hdrcopier")
        .arg("copy")
        .arg("--chapters")
        .arg(input)
        .arg(target)
        .status()?;
    if !status.success() {
        anyhow::bail!("Error copying hdr data");
    }
    Ok(())
}
