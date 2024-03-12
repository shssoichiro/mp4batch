use std::num::NonZeroUsize;

use av_data::pixel::{
    ChromaLocation, ColorPrimaries, MatrixCoefficients, TransferCharacteristic, YUVRange,
};

use crate::{
    input::{Colorimetry, VideoDimensions},
    output::Profile,
};

pub fn build_x265_args_string(
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
