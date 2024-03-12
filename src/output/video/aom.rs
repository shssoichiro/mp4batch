use std::num::NonZeroUsize;

use av_data::pixel::{ChromaLocation, ColorPrimaries, MatrixCoefficients, TransferCharacteristic};

use crate::{
    input::{Colorimetry, VideoDimensions},
    output::Profile,
};

pub fn build_aom_args_string(
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
