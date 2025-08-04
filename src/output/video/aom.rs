use crate::{
    input::{Colorimetry, VideoDimensions},
    output::{Profile, VideoEncoderIdent},
};
use std::num::NonZeroUsize;

pub fn build_aom_args_string(
    crf: i16,
    speed: u8,
    dimensions: VideoDimensions,
    profile: Profile,
    colorimetry: Colorimetry,
    threads: NonZeroUsize,
) -> anyhow::Result<String> {
    // Note: aom doesn't have a parameter to control full vs limited range
    let bd = dimensions.bit_depth;
    let tile_cols = i32::from(dimensions.width >= 2000);
    let tile_rows = i32::from(
        dimensions.height >= 2000 || (dimensions.height >= 1550 && dimensions.width >= 3600),
    );
    let arnr_str = if profile == Profile::Anime || profile == Profile::AnimeDetailed {
        1
    } else {
        3
    };
    let deltaq_mode = if colorimetry.is_hdr() { 5 } else { 1 };
    let prim = colorimetry.get_primaries_encoder_string(VideoEncoderIdent::Aom)?;
    let matrix = colorimetry.get_matrix_encoder_string(VideoEncoderIdent::Aom)?;
    let transfer = colorimetry.get_transfer_encoder_string(VideoEncoderIdent::Aom)?;
    let csp = colorimetry.get_chromaloc_encoder_string(VideoEncoderIdent::Aom)?;
    Ok(format!(
        " -b {bd} --end-usage=q --min-q=1 --lag-in-frames=64 --cpu-used={speed} --cq-level={crf} \
         --disable-kf --kf-max-dist=9999 --enable-fwd-kf=0 --sharpness=3 --row-mt=0 \
         --tile-columns={tile_cols} --tile-rows={tile_rows} --arnr-maxframes=15 \
         --arnr-strength={arnr_str} --tune=ssim  --enable-chroma-deltaq=1 \
         --disable-trellis-quant=0 --enable-qm=1 --qm-min=0 --qm-max=8 --quant-b-adapt=1 \
         --aq-mode=0 --deltaq-mode={deltaq_mode} --tune-content=psy --sb-size=dynamic \
         --enable-dnl-denoising=0 --color-primaries={prim} --transfer-characteristics={transfer} \
         --matrix-coefficients={matrix} --chroma-sample-position={csp}  --threads={threads} "
    ))
}
