use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

use nom::{
    branch::alt,
    bytes::complete::tag,
    character::complete::{alpha1, alphanumeric1, char, digit1},
    combinator::{opt, recognize},
    multi::separated_list1,
    sequence::{preceded, tuple},
    IResult,
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
    Hdr(bool),
    Extension(&'a str),
    BitDepth(u8),
    Resolution { width: u32, height: u32 },
    AudioEncoder(&'a str),
    AudioBitrate(u32),
    AudioTracks(Vec<Track>),
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

pub fn parse_filters<'a, 'b>(input: &'a str, in_file: &'b Path) -> Vec<ParsedFilter<'a>> {
    let mut filters = Vec::new();
    let mut input = input.trim_start();
    while !input.is_empty() {
        let (next_input, result) = parse_video_encoder(input)
            .or_else(|_| parse_quantizer(input))
            .or_else(|_| parse_speed(input))
            .or_else(|_| parse_profile(input))
            .or_else(|_| parse_grain(input))
            .or_else(|_| parse_compat(input))
            .or_else(|_| parse_hdr(input))
            .or_else(|_| parse_extension(input))
            .or_else(|_| parse_bit_depth(input))
            .or_else(|_| parse_resolution(input))
            .or_else(|_| parse_audio_encoder(input))
            .or_else(|_| parse_audio_bitrate(input))
            .or_else(|_| parse_audio_tracks(input, in_file))
            .or_else(|_| parse_subtitle_tracks(input, in_file))
            .unwrap();
        filters.push(result);
        input = next_input.trim_end().trim_start_matches(',').trim_start();
    }
    filters
}

fn parse_video_encoder(input: &str) -> IResult<&str, ParsedFilter> {
    preceded(tag("enc="), alphanumeric1)(input).map(|(input, token)| {
        if VideoEncoder::supported_encoders().contains(&token) {
            (input, ParsedFilter::VideoEncoder(token))
        } else {
            panic!("Unrecognize video encoder: {}", token);
        }
    })
}

fn parse_quantizer(input: &str) -> IResult<&str, ParsedFilter> {
    preceded(
        alt((tag("q="), tag("qp="), tag("crf="))),
        recognize(tuple((opt(char('-')), digit1))),
    )(input)
    .map(|(input, token)| (input, ParsedFilter::Quantizer(token.parse().unwrap())))
}

fn parse_speed(input: &str) -> IResult<&str, ParsedFilter> {
    preceded(alt((tag("s="), tag("speed="))), digit1)(input)
        .map(|(input, token)| (input, ParsedFilter::Speed(token.parse().unwrap())))
}

fn parse_profile(input: &str) -> IResult<&str, ParsedFilter> {
    preceded(alt((tag("p="), tag("profile="))), alpha1)(input).map(|(input, token)| {
        (
            input,
            ParsedFilter::Profile(Profile::from_str(token).unwrap()),
        )
    })
}

fn parse_grain(input: &str) -> IResult<&str, ParsedFilter> {
    preceded(tag("grain="), digit1)(input)
        .map(|(input, token)| (input, ParsedFilter::Grain(token.parse().unwrap())))
}

fn parse_compat(input: &str) -> IResult<&str, ParsedFilter> {
    preceded(tag("compat="), digit1)(input).map(|(input, token)| {
        (
            input,
            ParsedFilter::Compat(token.parse::<u8>().unwrap() > 0),
        )
    })
}

fn parse_hdr(input: &str) -> IResult<&str, ParsedFilter> {
    preceded(tag("hdr="), digit1)(input)
        .map(|(input, token)| (input, ParsedFilter::Hdr(token.parse::<u8>().unwrap() > 0)))
}

fn parse_extension(input: &str) -> IResult<&str, ParsedFilter> {
    preceded(tag("ext="), alphanumeric1)(input).map(|(input, token)| {
        if token == "mp4" || token == "mkv" {
            (input, ParsedFilter::Extension(token))
        } else {
            panic!("Unsupported extension: {}", token);
        }
    })
}

fn parse_bit_depth(input: &str) -> IResult<&str, ParsedFilter> {
    preceded(tag("bd="), alpha1)(input).map(|(input, token)| {
        if token == "8" || token == "10" {
            (input, ParsedFilter::BitDepth(token.parse().unwrap()))
        } else {
            panic!("Unsupported bit depth: {}", token);
        }
    })
}

fn parse_resolution(input: &str) -> IResult<&str, ParsedFilter> {
    preceded(tag("res="), tuple((digit1, char('x'), digit1)))(input).map(|(input, (w, _, h))| {
        let width = w.parse::<u32>().unwrap();
        let height = h.parse::<u32>().unwrap();
        if width % 2 != 0 || height % 2 != 0 {
            panic!("Resolution must be mod 2, got {}x{}", w, h);
        }
        if width < 64 || height < 64 {
            panic!("Resolution must be at least 64x64, got {}x{}", w, h);
        }

        (input, ParsedFilter::Resolution { width, height })
    })
}

fn parse_audio_encoder(input: &str) -> IResult<&str, ParsedFilter> {
    preceded(tag("aenc="), alphanumeric1)(input).map(|(input, token)| {
        if AudioEncoder::supported_encoders().contains(&token) {
            (input, ParsedFilter::AudioEncoder(token))
        } else {
            panic!("Unrecognize audio encoder: {}", token);
        }
    })
}

fn parse_audio_bitrate(input: &str) -> IResult<&str, ParsedFilter> {
    preceded(tag("ab="), digit1)(input)
        .map(|(input, token)| (input, ParsedFilter::AudioBitrate(token.parse().unwrap())))
}

fn parse_audio_tracks<'a, 'b>(
    input: &'a str,
    in_file: &'b Path,
) -> IResult<&'a str, ParsedFilter<'a>> {
    preceded(
        tag("at="),
        separated_list1(
            char('|'),
            tuple((alphanumeric1, opt(char('e')), opt(char('f')))),
        ),
    )(input)
    .map(|(input, tokens)| {
        (
            input,
            ParsedFilter::AudioTracks(
                tokens
                    .into_iter()
                    .map(|(id, e, f)| Track {
                        source: match id.parse() {
                            Ok(id) => TrackSource::FromVideo(id),
                            Err(_) => {
                                let source = in_file.with_extension(id);
                                assert!(source.exists());
                                TrackSource::External(source)
                            }
                        },
                        enabled: e.is_some(),
                        forced: f.is_some(),
                    })
                    .collect(),
            ),
        )
    })
}

fn parse_subtitle_tracks<'a, 'b>(
    input: &'a str,
    in_file: &'b Path,
) -> IResult<&'a str, ParsedFilter<'a>> {
    preceded(
        tag("st="),
        separated_list1(
            char('|'),
            tuple((alphanumeric1, opt(char('e')), opt(char('f')))),
        ),
    )(input)
    .map(|(input, tokens)| {
        (
            input,
            ParsedFilter::SubtitleTracks(
                tokens
                    .into_iter()
                    .map(|(id, e, f)| Track {
                        source: match id.parse() {
                            Ok(id) => TrackSource::FromVideo(id),
                            Err(_) => {
                                let source = in_file.with_extension(id);
                                assert!(source.exists());
                                TrackSource::External(source)
                            }
                        },
                        enabled: e.is_some(),
                        forced: f.is_some(),
                    })
                    .collect(),
            ),
        )
    })
}
