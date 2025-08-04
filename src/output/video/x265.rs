use crate::{
    input::{Colorimetry, VideoDimensions},
    output::{Profile, VideoEncoderIdent},
};
use std::num::NonZeroUsize;

pub fn build_x265_args_string(
    crf: i16,
    dimensions: VideoDimensions,
    profile: Profile,
    compat: bool,
    colorimetry: Colorimetry,
    threads: NonZeroUsize,
) -> anyhow::Result<String> {
    // TODO: Add full HDR metadata

    let deblock = if profile.is_anime() { -1 } else { -2 };
    let chroma_offset = if profile.is_anime() { -2 } else { 0 };
    let bframes = match profile {
        Profile::Film | Profile::Grain => 5,
        Profile::Anime | Profile::AnimeDetailed | Profile::AnimeGrain => 8,
        Profile::Fast => 3,
    };
    let refframes = match profile {
        Profile::Film | Profile::Grain | Profile::AnimeGrain => 4,
        Profile::Anime | Profile::AnimeDetailed => 6,
        Profile::Fast => 3,
    };
    let sao = if crf >= 22 {
        "--sao"
    } else if crf >= 17 {
        "--limit-sao"
    } else {
        "--no-sao --no-strong-intra-smoothing"
    };
    let psy_rd = match profile {
        Profile::Anime | Profile::Fast => "1.0",
        Profile::Film | Profile::AnimeDetailed => "1.5",
        Profile::Grain | Profile::AnimeGrain => "2.0",
    };
    let psy_rdo = match profile {
        Profile::Anime | Profile::Fast => "1.0",
        Profile::AnimeDetailed => "1.5",
        Profile::Film | Profile::AnimeGrain => "2.0",
        Profile::Grain => "4.0",
    };
    let aq_str = match profile {
        Profile::Grain => "0.9",
        Profile::Film | Profile::AnimeGrain => "0.8",
        Profile::Anime | Profile::AnimeDetailed | Profile::Fast => "0.7",
    };
    let prim = colorimetry.get_primaries_encoder_string(VideoEncoderIdent::X265)?;
    let matrix = colorimetry.get_matrix_encoder_string(VideoEncoderIdent::X265)?;
    let transfer = colorimetry.get_transfer_encoder_string(VideoEncoderIdent::X265)?;
    let range = colorimetry.get_range_encoder_string(VideoEncoderIdent::X265)?;
    let csp = colorimetry.get_chromaloc_encoder_string(VideoEncoderIdent::X265)?;
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
    Ok(format!(
        " --crf {crf} --preset slow --bframes {bframes} --ref {refframes} --keyint -1 --min-keyint 1 \
          --no-scenecut {sao} --deblock {deblock}:{deblock} --psy-rd {psy_rd} --psy-rdoq {psy_rdo} --qcomp 0.65 \
         --aq-mode 3 --aq-strength {aq_str} --cbqpoffs {chroma_offset} --crqpoffs {chroma_offset} \
         --no-open-gop --no-cutree --fades --colorprim {prim} --colormatrix {matrix} --transfer {transfer} \
         --range {range} {csp} --output-depth {depth} --frame-threads {threads} --lookahead-threads {threads} \
         --y4m {level} {hdr} "
    ))
}
