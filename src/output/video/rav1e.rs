use crate::{
    input::{Colorimetry, VideoDimensions},
    output::VideoEncoderIdent,
};

pub fn build_rav1e_args_string(
    crf: i16,
    speed: u8,
    dimensions: VideoDimensions,
    colorimetry: Colorimetry,
) -> anyhow::Result<String> {
    // TODO: Add proper HDR metadata
    // TODO: Remove rdo-lookahead-frames limitation if we can reduce rav1e memory
    // usage
    let tile_cols = i32::from(dimensions.width >= 2000);
    let tile_rows = i32::from(
        dimensions.height >= 2000 || (dimensions.height >= 1550 && dimensions.width >= 3600),
    );
    let prim = colorimetry.get_primaries_encoder_string(VideoEncoderIdent::Rav1e)?;
    let matrix = colorimetry.get_matrix_encoder_string(VideoEncoderIdent::Rav1e)?;
    let transfer = colorimetry.get_transfer_encoder_string(VideoEncoderIdent::Rav1e)?;
    let range = colorimetry.get_range_encoder_string(VideoEncoderIdent::Rav1e)?;
    Ok(format!(
        " --speed {speed} --quantizer {crf} --tile-cols {tile_cols} --tile-rows {tile_rows} \
         --primaries {prim} --matrix {matrix}  --transfer {transfer} --range {range} \
         --rdo-lookahead-frames 25 --no-scene-detection --keyint 0 "
    ))
}
