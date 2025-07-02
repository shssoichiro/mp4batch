use anyhow::{Result, anyhow};
use std::path::Path;
use which::which;

use crate::cli::{ParsedFilter, parse_filters};
use crate::output::{AudioEncoder, Output, Profile, VideoEncoder};

/// Parses the formats string and creates output configurations
pub fn parse_output_configurations(formats: Option<&String>, input_file: &Path) -> Vec<Output> {
    formats.map_or_else(
        || vec![Output::default()],
        |formats| {
            let formats = formats.trim();
            if formats.is_empty() {
                return vec![Output::default()];
            }
            formats
                .split(';')
                .map(|format| create_output_from_format(format, input_file))
                .collect()
        },
    )
}

/// Creates an Output configuration from a format string
fn create_output_from_format(format: &str, input_file: &Path) -> Output {
    let mut output = Output::default();
    let filters = parse_filters(format, input_file);

    // First, handle video encoder selection
    if let Some(encoder) = find_video_encoder(&filters) {
        configure_video_encoder(&mut output, encoder).expect("Failed to configure video encoder");
    }

    // Then apply all other filters
    for filter in &filters {
        apply_filter(filter, &mut output);
    }

    output
}

/// Finds the video encoder from the parsed filters
fn find_video_encoder<'a>(filters: &'a [ParsedFilter<'a>]) -> Option<&'a str> {
    filters.iter().find_map(|filter| {
        if let ParsedFilter::VideoEncoder(encoder) = filter {
            Some(*encoder)
        } else {
            None
        }
    })
}

/// Configures the video encoder and validates it's installed
fn configure_video_encoder(output: &mut Output, encoder: &str) -> Result<()> {
    match encoder.to_lowercase().as_str() {
        "x264" => {
            validate_encoder_installed("x264")?;
            // x264 is already the default, no change needed
        }
        "x265" => {
            validate_encoder_installed("x265")?;
            output.video.encoder = VideoEncoder::X265 {
                crf: 18,
                profile: Profile::Film,
                compat: false,
            }
        }
        "aom" => {
            validate_encoder_installed("aomenc")?;
            output.video.encoder = VideoEncoder::Aom {
                crf: 16,
                speed: 4,
                profile: Profile::Film,
                grain: 0,
                compat: false,
            }
        }
        "rav1e" => {
            validate_encoder_installed("rav1e")?;
            output.video.encoder = VideoEncoder::Rav1e {
                crf: 40,
                speed: 5,
                profile: Profile::Film,
                grain: 0,
            }
        }
        "svt" => {
            validate_encoder_installed("SvtAv1EncApp")?;
            output.video.encoder = VideoEncoder::SvtAv1 {
                crf: 16,
                speed: 4,
                profile: Profile::Film,
                grain: 0,
            }
        }
        "copy" => {
            output.video.encoder = VideoEncoder::Copy;
        }
        enc => anyhow::bail!("Unrecognized encoder: {}", enc),
    }
    Ok(())
}

/// Validates that an encoder is installed and available in PATH
fn validate_encoder_installed(encoder_name: &str) -> Result<()> {
    which(encoder_name).map_err(|_| anyhow!("{} not installed or not in PATH!", encoder_name))?;
    Ok(())
}

/// Applies a parsed filter to the output configuration
fn apply_filter(filter: &ParsedFilter, output: &mut Output) {
    match filter {
        ParsedFilter::VideoEncoder(_) => (), // Already handled in configure_video_encoder
        ParsedFilter::Quantizer(arg) => apply_quantizer_filter(*arg, output),
        ParsedFilter::Speed(arg) => apply_speed_filter(*arg, output),
        ParsedFilter::Profile(arg) => apply_profile_filter(*arg, output),
        ParsedFilter::Grain(arg) => apply_grain_filter(*arg, output),
        ParsedFilter::Compat(arg) => apply_compat_filter(*arg, output),
        ParsedFilter::Extension(arg) => {
            output.video.output_ext = (*arg).to_string();
        }
        ParsedFilter::BitDepth(arg) => {
            output.video.bit_depth = Some(*arg);
        }
        ParsedFilter::Resolution { width, height } => {
            output.video.resolution = Some((*width, *height));
        }
        ParsedFilter::AudioEncoder(arg) => apply_audio_encoder_filter(arg, output),
        ParsedFilter::AudioBitrate(arg) => apply_audio_bitrate_filter(*arg, output),
        ParsedFilter::AudioTracks(args) => {
            output.audio_tracks.clone_from(args);
        }
        ParsedFilter::AudioNormalize => {
            output.audio.normalize = true;
        }
        ParsedFilter::SubtitleTracks(args) => {
            output.sub_tracks.clone_from(args);
        }
    }
}

fn apply_quantizer_filter(arg: i16, output: &mut Output) {
    let range = match output.video.encoder {
        VideoEncoder::X264 { ref mut crf, .. } => {
            *crf = arg;
            (-12, 51)
        }
        VideoEncoder::X265 { ref mut crf, .. } => {
            *crf = arg;
            (0, 51)
        }
        VideoEncoder::Aom { ref mut crf, .. } | VideoEncoder::SvtAv1 { ref mut crf, .. } => {
            *crf = arg;
            (0, 63)
        }
        VideoEncoder::Rav1e { ref mut crf, .. } => {
            *crf = arg;
            (0, 255)
        }
        VideoEncoder::Copy => return,
    };

    if arg < range.0 || arg > range.1 {
        panic!(
            "'q' must be between {} and {}, received {}",
            range.0, range.1, arg
        );
    }
}

fn apply_speed_filter(arg: u8, output: &mut Output) {
    match output.video.encoder {
        VideoEncoder::Aom { ref mut speed, .. }
        | VideoEncoder::Rav1e { ref mut speed, .. }
        | VideoEncoder::SvtAv1 { ref mut speed, .. } => {
            if arg > 10 {
                panic!("'s' must be between 0 and 10, received {}", arg);
            }
            *speed = arg;
        }
        _ => (),
    }
}

fn apply_profile_filter(arg: Profile, output: &mut Output) {
    match output.video.encoder {
        VideoEncoder::X264 {
            ref mut profile, ..
        }
        | VideoEncoder::X265 {
            ref mut profile, ..
        }
        | VideoEncoder::Aom {
            ref mut profile, ..
        }
        | VideoEncoder::Rav1e {
            ref mut profile, ..
        }
        | VideoEncoder::SvtAv1 {
            ref mut profile, ..
        } => {
            *profile = arg;
        }
        VideoEncoder::Copy => (),
    }
}

fn apply_grain_filter(arg: u8, output: &mut Output) {
    match output.video.encoder {
        VideoEncoder::Aom { ref mut grain, .. }
        | VideoEncoder::Rav1e { ref mut grain, .. }
        | VideoEncoder::SvtAv1 { ref mut grain, .. } => {
            if arg > 64 {
                panic!("'grain' must be between 0 and 64, received {}", arg);
            }
            *grain = arg;
        }
        _ => (),
    }
}

fn apply_compat_filter(arg: bool, output: &mut Output) {
    match output.video.encoder {
        VideoEncoder::X264 { ref mut compat, .. }
        | VideoEncoder::X265 { ref mut compat, .. }
        | VideoEncoder::Aom { ref mut compat, .. } => {
            *compat = arg;
        }
        _ => (),
    }
}

fn apply_audio_encoder_filter(arg: &str, output: &mut Output) {
    output.audio.encoder = match arg.to_lowercase().as_str() {
        "copy" => AudioEncoder::Copy,
        "flac" => AudioEncoder::Flac,
        "aac" => AudioEncoder::Aac,
        "opus" => AudioEncoder::Opus,
        arg => panic!("Invalid value provided for 'aenc': {}", arg),
    }
}

fn apply_audio_bitrate_filter(arg: u32, output: &mut Output) {
    if arg == 0 {
        panic!("'ab' must be greater than 0, got {}", arg);
    }
    output.audio.kbps_per_channel = arg;
}
