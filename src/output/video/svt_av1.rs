use av_data::pixel::{ChromaLocation, ToPrimitive, YUVRange};

use crate::input::{Colorimetry, VideoDimensions};

pub fn build_svtav1_args_string(
    crf: i16,
    speed: u8,
    threads: usize,
    dimensions: VideoDimensions,
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
    format!(
        " --input-depth {depth} --scm 0 --preset {speed} --crf {crf} --film-grain-denoise 0 \
         --tile-columns {tile_cols} --tile-rows {tile_rows} --rc 0 --enable-qm 1 --qm-min 0 \
         --qm-max 8 --tune 2 --enable-tf 0 --scd 0 --keyint -1 --lp {threads} \
         --pin 0 --color-primaries {prim} --matrix-coefficients {matrix} \
         --transfer-characteristics {transfer} --color-range {range} --chroma-sample-position \
         {csp} "
    )
}
