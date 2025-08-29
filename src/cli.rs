use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

use nom::{
    IResult, Parser,
    branch::alt,
    bytes::complete::tag,
    character::complete::{alpha1, alphanumeric1, char, digit1},
    combinator::{opt, recognize},
    multi::separated_list1,
    sequence::preceded,
};

use crate::{AudioEncoder, Profile, VideoEncoder};

#[derive(Debug, Clone)]
pub enum ParsedFilter<'a> {
    VideoEncoder(&'a str),
    Quantizer(i16),
    Speed(u8),
    Profile(Profile),
    Grain(u8),
    Compat(bool),
    Extension(&'a str),
    BitDepth(u8),
    Resolution { width: u32, height: u32 },
    AudioEncoder(&'a str),
    AudioBitrate(u32),
    AudioTracks(Vec<Track>),
    AudioNormalize,
    SubtitleTracks(Vec<Track>),
}

#[derive(Debug, Clone)]
pub struct Track {
    pub source: TrackSource,
    pub enabled: bool,
    pub forced: bool,
}

#[derive(Debug, Clone)]
pub enum TrackSource {
    FromVideo(u8),
    External(PathBuf),
}

pub fn parse_filters<'a>(input: &'a str, in_file: &Path) -> Vec<ParsedFilter<'a>> {
    let mut filters = Vec::new();
    let mut input = input.trim_start();
    while !input.is_empty() {
        let (next_input, result) = parse_video_encoder(input)
            .or_else(|_| parse_quantizer(input))
            .or_else(|_| parse_speed(input))
            .or_else(|_| parse_profile(input))
            .or_else(|_| parse_grain(input))
            .or_else(|_| parse_compat(input))
            .or_else(|_| parse_extension(input))
            .or_else(|_| parse_bit_depth(input))
            .or_else(|_| parse_resolution(input))
            .or_else(|_| parse_audio_encoder(input))
            .or_else(|_| parse_audio_bitrate(input))
            .or_else(|_| parse_audio_tracks(input, in_file))
            .or_else(|_| parse_audio_norm(input))
            .or_else(|_| parse_subtitle_tracks(input, in_file))
            .expect("Unrecognized filter");
        filters.push(result);
        input = next_input.trim_end().trim_start_matches(',').trim_start();
    }
    filters
}

fn parse_video_encoder(input: &str) -> IResult<&str, ParsedFilter<'_>> {
    preceded(tag("enc="), alphanumeric1)
        .parse_complete(input)
        .map(|(input, token)| {
            if VideoEncoder::supported_encoders().contains(&token) {
                (input, ParsedFilter::VideoEncoder(token))
            } else {
                panic!("Unrecognize video encoder: {}", token);
            }
        })
}

fn parse_quantizer(input: &str) -> IResult<&str, ParsedFilter<'_>> {
    preceded(
        alt((tag("q="), tag("qp="), tag("crf="))),
        recognize((opt(char('-')), digit1)),
    )
    .parse_complete(input)
    .map(|(input, token)| {
        (
            input,
            ParsedFilter::Quantizer(token.parse().expect("Invalid quantizer value")),
        )
    })
}

fn parse_speed(input: &str) -> IResult<&str, ParsedFilter<'_>> {
    preceded(alt((tag("s="), tag("speed="))), digit1)
        .parse_complete(input)
        .map(|(input, token)| {
            (
                input,
                ParsedFilter::Speed(token.parse().expect("Invalid speed value")),
            )
        })
}

fn parse_profile(input: &str) -> IResult<&str, ParsedFilter<'_>> {
    preceded(alt((tag("p="), tag("profile="))), alpha1)
        .parse_complete(input)
        .map(|(input, token)| {
            (
                input,
                ParsedFilter::Profile(Profile::from_str(token).expect("Invalid profile")),
            )
        })
}

fn parse_grain(input: &str) -> IResult<&str, ParsedFilter<'_>> {
    preceded(alt((tag("g="), tag("grain="))), digit1)
        .parse_complete(input)
        .map(|(input, token)| {
            (
                input,
                ParsedFilter::Grain(token.parse().expect("Invalid grain value")),
            )
        })
}

fn parse_compat(input: &str) -> IResult<&str, ParsedFilter<'_>> {
    preceded(tag("compat="), digit1)
        .parse_complete(input)
        .map(|(input, token)| {
            (
                input,
                ParsedFilter::Compat(token.parse::<u8>().expect("Invalid compat value") > 0),
            )
        })
}

fn parse_extension(input: &str) -> IResult<&str, ParsedFilter<'_>> {
    preceded(tag("ext="), alphanumeric1)
        .parse_complete(input)
        .map(|(input, token)| {
            if token == "mp4" || token == "mkv" {
                (input, ParsedFilter::Extension(token))
            } else {
                panic!("Unsupported extension: {}", token);
            }
        })
}

fn parse_bit_depth(input: &str) -> IResult<&str, ParsedFilter<'_>> {
    preceded(tag("bd="), digit1)
        .parse_complete(input)
        .map(|(input, token)| {
            if token == "8" || token == "10" {
                (
                    input,
                    ParsedFilter::BitDepth(token.parse().expect("Invalid bit depth value")),
                )
            } else {
                panic!("Unsupported bit depth: {}", token);
            }
        })
}

fn parse_resolution(input: &str) -> IResult<&str, ParsedFilter<'_>> {
    preceded(tag("res="), (digit1, char('x'), digit1))
        .parse_complete(input)
        .map(|(input, (w, _, h))| {
            let width = w.parse::<u32>().expect("Invalid width value");
            let height = h.parse::<u32>().expect("Invalid height value");
            assert!(
                !(width % 2 != 0 || height % 2 != 0),
                "Resolution must be mod 2, got {}x{}",
                w,
                h
            );
            assert!(
                !(width < 64 || height < 64),
                "Resolution must be at least 64x64, got {}x{}",
                w,
                h
            );

            (input, ParsedFilter::Resolution { width, height })
        })
}

fn parse_audio_encoder(input: &str) -> IResult<&str, ParsedFilter<'_>> {
    preceded(tag("aenc="), alphanumeric1)
        .parse_complete(input)
        .map(|(input, token)| {
            if AudioEncoder::supported_encoders().contains(&token) {
                (input, ParsedFilter::AudioEncoder(token))
            } else {
                panic!("Unrecognize audio encoder: {}", token);
            }
        })
}

fn parse_audio_bitrate(input: &str) -> IResult<&str, ParsedFilter<'_>> {
    preceded(tag("ab="), digit1)
        .parse_complete(input)
        .map(|(input, token)| {
            (
                input,
                ParsedFilter::AudioBitrate(token.parse().expect("Invalid audio bitrate value")),
            )
        })
}

fn parse_audio_tracks<'a>(input: &'a str, in_file: &Path) -> IResult<&'a str, ParsedFilter<'a>> {
    preceded(
        tag("at="),
        separated_list1(char('|'), (alphanumeric1, opt(preceded(char('-'), alpha1)))),
    )
    .parse_complete(input)
    .map(|(input, tokens)| {
        (
            input,
            ParsedFilter::AudioTracks(
                tokens
                    .into_iter()
                    .map(|(id, tags)| {
                        let tags = tags.unwrap_or("");
                        Track {
                            source: id.parse().map_or_else(
                                |_| {
                                    let source = in_file.with_extension(id);
                                    assert!(source.exists());
                                    TrackSource::External(source)
                                },
                                TrackSource::FromVideo,
                            ),
                            enabled: tags.contains('d') || tags.contains('e'),
                            forced: tags.contains('f'),
                        }
                    })
                    .collect(),
            ),
        )
    })
}

fn parse_audio_norm(input: &str) -> IResult<&str, ParsedFilter<'_>> {
    tag("an=1")(input).map(|(input, _)| (input, ParsedFilter::AudioNormalize))
}

fn parse_subtitle_tracks<'a>(input: &'a str, in_file: &Path) -> IResult<&'a str, ParsedFilter<'a>> {
    preceded(
        tag("st="),
        separated_list1(char('|'), (alphanumeric1, opt(preceded(char('-'), alpha1)))),
    )
    .parse_complete(input)
    .map(|(input, tokens)| {
        (
            input,
            ParsedFilter::SubtitleTracks(
                tokens
                    .into_iter()
                    .map(|(id, tags)| {
                        let tags = tags.unwrap_or("");
                        Track {
                            source: id.parse().map_or_else(
                                |_| {
                                    let source = in_file.with_extension(id);
                                    assert!(source.exists());
                                    TrackSource::External(source)
                                },
                                TrackSource::FromVideo,
                            ),
                            enabled: tags.contains('d') || tags.contains('e'),
                            forced: tags.contains('f'),
                        }
                    })
                    .collect(),
            ),
        )
    })
}
