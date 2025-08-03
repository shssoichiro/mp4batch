use crate::{
    input::{Colorimetry, VideoDimensions},
    output::VideoEncoderIdent,
};

use super::Profile;

pub fn build_svtav1_args_string(
    crf: i16,
    speed: u8,
    threads: usize,
    dimensions: VideoDimensions,
    profile: Profile,
    colorimetry: &Colorimetry,
) -> anyhow::Result<String> {
    let depth = dimensions.bit_depth;
    let tile_cols = i32::from(dimensions.width >= 2000);
    let tile_rows = i32::from(
        dimensions.height >= 2000 || (dimensions.height >= 1550 && dimensions.width >= 3600),
    );
    let prim = colorimetry.get_primaries_encoder_string(VideoEncoderIdent::SvtAv1)?;
    let matrix = colorimetry.get_matrix_encoder_string(VideoEncoderIdent::SvtAv1)?;
    let transfer = colorimetry.get_transfer_encoder_string(VideoEncoderIdent::SvtAv1)?;
    let range = colorimetry.get_range_encoder_string(VideoEncoderIdent::SvtAv1)?;
    let csp = colorimetry.get_chromaloc_encoder_string(VideoEncoderIdent::SvtAv1)?;
    let qm_min = match profile {
        Profile::Grain => 8,
        Profile::AnimeGrain => 5,
        Profile::Film if crf <= 13 => 5,
        Profile::Film if crf <= 15 => 3,
        Profile::AnimeDetailed if crf <= 13 => 3,
        _ => 2,
    };
    let psy_rd = match profile {
        Profile::Film => "0.5",
        Profile::Grain => "1.0",
        Profile::Anime => "0.2",
        Profile::AnimeDetailed => "0.3",
        Profile::AnimeGrain => "0.5",
        Profile::Fast => "0.0",
    };
    let hbd_mds = if psy_rd.parse::<f32>().unwrap() > 0.0 {
        "1"
    } else {
        "0"
    };
    // In theory 2 might be slightly better than 1 for live action,
    // but the difference is small, and 2 seems to currently be bugged at
    // very high bitrates to give worse quality than 1
    let tune = match profile {
        Profile::Fast => '0',
        _ => '1',
    };
    let complex_hvs = if psy_rd.parse::<f32>().unwrap() >= 1.0 {
        "1"
    } else {
        "0"
    };
    Ok(format!(
        " --input-depth {depth} --rc 0 --enable-qm 1 \
        --scd 0 --keyint -1 --scm 0 --film-grain-denoise 0 --enable-dlf 2 \
        --preset {speed} --crf {crf} --tune {tune} --complex-hvs {complex_hvs} \
        --qm-min {qm_min} --ac-bias {psy_rd} --hbd-mds {hbd_mds} \
        --tile-columns {tile_cols} --tile-rows {tile_rows} --lp {threads} --pin 0 \
        --color-primaries {prim} --matrix-coefficients {matrix} \
        --transfer-characteristics {transfer} --color-range {range} \
        --chroma-sample-position {csp} "
    ))
}
