use av_data::pixel::{ColorPrimaries, MatrixCoefficients, TransferCharacteristic, YUVRange};

use crate::input::{Colorimetry, VideoDimensions};

pub fn build_rav1e_args_string(
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
