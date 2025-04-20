use av_data::pixel::{ChromaLocation, ToPrimitive, YUVRange};

use crate::input::{Colorimetry, VideoDimensions};

use super::Profile;

pub fn build_svtav1_args_string(
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
    format!(
        " --input-depth {depth} --rc 0 --enable-qm 1 \
        --scd 0 --keyint -1  --scm 0 --film-grain-denoise 0 \
        --preset {speed} --crf {crf} --tune 3 \
        --qm-min {qm_min} --psy-rd {psy_rd} --hbd-mds {hbd_mds} \
        --tile-columns {tile_cols} --tile-rows {tile_rows} --lp {threads} --pin 0 \
        --color-primaries {prim} --matrix-coefficients {matrix} \
        --transfer-characteristics {transfer} --color-range {range} \
        --chroma-sample-position {csp} "
    )
}
