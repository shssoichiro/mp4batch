use std::{
    path::Path,
    process::{Command, Stdio},
};

use ansi_term::Color::Yellow;
use av_data::pixel::{
    ChromaLocation, ColorPrimaries, MatrixCoefficients, TransferCharacteristic, YUVRange,
};

use crate::{
    absolute_path,
    input::{get_video_frame_count, Colorimetry, PixelFormat, VideoDimensions},
    output::Profile,
};

pub fn convert_video_x264(
    vpy_input: &Path,
    output: &Path,
    crf: i16,
    profile: Profile,
    compat: bool,
    dimensions: VideoDimensions,
    colorimetry: &Colorimetry,
) -> anyhow::Result<()> {
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

    if output.exists() && get_video_frame_count(output).unwrap_or(0) == dimensions.frames {
        eprintln!("Video output already exists, reusing");
        return Ok(());
    }

    let pipe = Command::new("vspipe")
        .arg("-c")
        .arg("y4m")
        .arg(absolute_path(vpy_input).expect("Unable to get absolute path"))
        .arg("-")
        .stdout(Stdio::piped())
        .spawn()
        .expect("Unable to run vspipe, is it installed and in PATH?");

    let mut command = Command::new("nice");
    command
        .arg("x264")
        .arg("--demuxer")
        .arg("y4m")
        .arg("--frames")
        .arg(dimensions.frames.to_string());
    let args = build_x264_args_string(crf, dimensions, profile, compat, colorimetry);
    eprintln!("x264 args: {args}");
    for arg in args.split_ascii_whitespace() {
        command.arg(arg);
    }
    command
        .arg("-o")
        .arg(absolute_path(output).expect("Unable to get absolute path"))
        .arg("-");
    command
        .stdin(pipe.stdout.expect("stdout should be writeable"))
        .stderr(Stdio::inherit());
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

pub fn build_x264_args_string(
    crf: i16,
    dimensions: VideoDimensions,
    profile: Profile,
    compat: bool,
    colorimetry: &Colorimetry,
) -> String {
    let fps = (dimensions.fps.0 as f32 / dimensions.fps.1 as f32).round() as u32;
    let min_keyint = if profile == Profile::Anime {
        fps / 2
    } else {
        fps
    };
    let max_keyint = if profile == Profile::Anime {
        fps * 15
    } else {
        fps * 10
    };
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
         --merange {merange} --rc-lookahead 96 --aq-mode 3 --aq-strength {aq_str} {mbtree} -i \
         {min_keyint} -I {max_keyint} --qcomp {qcomp} --ipratio 1.30 --pbratio 1.20 \
         --no-fast-pskip --no-dct-decimate --colorprim {prim} --colormatrix {matrix} --transfer \
         {transfer} --input-range {range} --range {range} {csp} --output-depth {depth} {vbv} \
         {level} "
    )
}
