#[macro_use]
extern crate dotenv_codegen;

mod input;
mod output;

use std::{env, path::Path, str::FromStr};

use clap::{App, Arg};
use itertools::Itertools;

use self::{input::*, output::*};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Target {
    Local,
    Dist,
}

impl Default for Target {
    fn default() -> Self {
        Target::Local
    }
}

impl FromStr for Target {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_ref() {
            "local" => Target::Local,
            "dist" => Target::Dist,
            _ => {
                return Err("Invalid target given".to_owned());
            }
        })
    }
}

fn main() {
    env::set_var("RUST_BACKTRACE", "1");
    let args = App::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .arg(
            Arg::with_name("target")
                .short("t")
                .long("target")
                .value_name("VALUE")
                .help(
                    "The target audience for the encode (default: local, available: local, dist)",
                ),
        )
        .arg(
            Arg::with_name("profile")
                .short("p")
                .long("profile")
                .value_name("VALUE")
                .help("Sets a custom profile (default: film, available: film, anime)")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("crf")
                .short("c")
                .long("crf")
                .value_name("VALUE")
                .help("Sets a CRF value to use (default: 18)")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("av1")
                .long("av1")
                .help("Encode to AV1 using rav1e (default QP: 30)"),
        )
        .arg(
            Arg::with_name("x265")
                .long("x265")
                .help("Encode to x265 (default QP: 30)"),
        )
        .arg(
            Arg::with_name("direct")
                .short("d")
                .long("direct")
                .value_name("A_TRACK")
                .help("remux mkv to mp4; will convert audio streams to aac without touching video")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("audio_track")
                .short("A")
                .long("audio-track")
                .value_name("A_TRACK")
                .help("define which audio track to use when doing a full conversion")
                .default_value("0")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("audio_bitrate")
                .long("ab")
                .value_name("VALUE")
                .help("Audio bitrate per channel in kbps (Default: 96 kbps/channel)")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("hdr")
                .long("hdr")
                .help("this video should be treated as HDR and encoded as BT.2020"),
        )
        .arg(
            Arg::with_name("acodec")
                .short("a")
                .long("acodec")
                .help("codec to use for audio")
                .takes_value(true)
                .default_value("copy")
                .possible_values(&["copy", "aac", "flac", "opus"]),
        )
        .arg(
            Arg::with_name("skip-video")
                .long("skip-video")
                .help("assume the video has already been encoded (will use .out.mkv files)"),
        )
        .arg(
            Arg::with_name("mp4")
                .long("mp4")
                .help("output to mp4 instead of mkv"),
        )
        .arg(
            Arg::with_name("slots")
                .long("slots")
                .help("the number of local workers to use for rav1e encoding (default: auto)")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("input")
                .help("Sets the input directory or file")
                .required(true)
                .index(1),
        )
        .get_matches();

    const INPUT_PATH_ERROR: &str = "No input path provided";
    const CRF_PARSE_ERROR: &str = "CRF must be a number between 0-51";

    let input = args.value_of("input").expect(INPUT_PATH_ERROR);
    let profile = Profile::from_str(args.value_of("profile").unwrap_or("film"))
        .expect("Invalid profile given");
    let target =
        Target::from_str(args.value_of("target").unwrap_or("local")).expect("Invalid target given");
    let encoder = if args.is_present("av1") {
        Encoder::Rav1e
    } else if args.is_present("x265") {
        Encoder::X265
    } else {
        Encoder::X264
    };
    let crf = match encoder {
        Encoder::Aom => args
            .value_of("crf")
            .unwrap_or("30")
            .parse::<u8>()
            .expect(CRF_PARSE_ERROR),
        Encoder::Rav1e => args
            .value_of("crf")
            .unwrap_or("60")
            .parse::<u8>()
            .expect(CRF_PARSE_ERROR),
        Encoder::X264 | Encoder::X265 => {
            let crf = args
                .value_of("crf")
                .unwrap_or("18")
                .parse::<u8>()
                .expect(CRF_PARSE_ERROR);
            assert!(crf <= 51, "{}", CRF_PARSE_ERROR);
            crf
        }
    };
    let audio_track = args.value_of("audio_track").unwrap().parse().unwrap();
    let audio_bitrate = args
        .value_of("audio_bitrate")
        .unwrap_or("96")
        .parse::<u32>()
        .unwrap();
    let extension = if args.is_present("mp4") { "mp4" } else { "mkv" };

    let input = Path::new(input);
    assert!(input.exists(), "Input path does not exist");

    if args.is_present("direct") {
        let track: u8 = args
            .value_of("direct")
            .map(|t| t.parse().unwrap())
            .unwrap_or(0);
        if input.is_dir() {
            let dir_entries = input.read_dir().unwrap();
            for entry in dir_entries
                .map(|e| e.unwrap())
                .filter(|e| {
                    e.path()
                        .extension()
                        .unwrap_or_default()
                        .to_str()
                        .unwrap_or_default()
                        == "mkv"
                })
                .sorted_by_key(|e| e.path())
            {
                let audio_track = find_external_audio(&entry.path(), track);
                let result = process_direct(
                    &entry.path(),
                    audio_track,
                    args.value_of("acodec").unwrap_or("copy"),
                    audio_bitrate,
                    extension,
                );
                if let Err(err) = result {
                    eprintln!(
                        "An error occurred for {}: {}",
                        entry.path().as_os_str().to_string_lossy(),
                        err
                    );
                }
            }
        } else {
            assert_eq!(
                input
                    .extension()
                    .unwrap_or_default()
                    .to_str()
                    .unwrap_or_default(),
                "mkv",
                "Input file must be a matroska file"
            );
            let audio_track = find_external_audio(input, audio_track);
            process_direct(
                input,
                audio_track,
                args.value_of("acodec").unwrap_or("copy"),
                audio_bitrate,
                extension,
            )
            .unwrap();
        }
        return;
    }

    if input.is_dir() {
        let dir_entries = input.read_dir().unwrap();
        for entry in dir_entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                let ext = e
                    .path()
                    .extension()
                    .unwrap_or_default()
                    .to_str()
                    .unwrap_or_default()
                    .to_string();
                ext == "vpy"
            })
            .sorted_by_key(|e| e.path())
        {
            let audio_track = find_external_audio(&entry.path(), audio_track);
            let result = process_file(
                &entry.path(),
                encoder,
                profile,
                target,
                crf,
                args.value_of("acodec").unwrap_or("copy"),
                args.is_present("skip-video"),
                audio_track,
                audio_bitrate,
                args.is_present("hdr"),
                extension,
                args.value_of("slots").map(|val| val.parse().unwrap()),
            );
            if let Err(err) = result {
                eprintln!(
                    "An error occurred for {}: {}",
                    entry.path().as_os_str().to_string_lossy(),
                    err
                );
            }
        }
    } else {
        let audio_track = find_external_audio(input, audio_track);
        process_file(
            input,
            encoder,
            profile,
            target,
            crf,
            args.value_of("acodec").unwrap_or("copy"),
            args.is_present("skip-video"),
            audio_track,
            audio_bitrate,
            args.is_present("hdr"),
            extension,
            args.value_of("slots").map(|val| val.parse().unwrap()),
        )
        .unwrap();
    }
}

#[allow(clippy::too_many_arguments)]
fn process_file(
    input: &Path,
    encoder: Encoder,
    profile: Profile,
    target: Target,
    crf: u8,
    audio_codec: &str,
    skip_video: bool,
    audio_track: AudioTrack,
    audio_bitrate: u32,
    is_hdr: bool,
    extension: &str,
    slots: Option<u8>,
) -> Result<(), String> {
    eprintln!("Converting {}", input.to_string_lossy());
    let dims = get_video_dimensions(input)?;
    if !skip_video {
        match encoder {
            Encoder::Aom => convert_video_av1(input, crf, dims, profile, is_hdr, true),
            Encoder::X264 => convert_video_x264(input, profile, crf, dims),
            Encoder::X265 => convert_video_x265(input, profile, crf, dims),
            Encoder::Rav1e => convert_video_rav1e(input, crf, profile, dims, is_hdr, slots),
        }?;
    }
    if target == Target::Local {
        // TODO: Handle audio and muxing for dist encodes
        convert_audio(input, audio_codec, audio_track, audio_bitrate)?;
        mux_video(input, encoder, extension)?;
    }
    eprintln!("Finished converting {}", input.to_string_lossy());
    Ok(())
}

fn process_direct(
    input: &Path,
    audio_track: AudioTrack,
    audio_codec: &str,
    audio_bitrate: u32,
    extension: &str,
) -> Result<(), String> {
    eprintln!("Converting {}", input.to_string_lossy());
    mux_video_direct(input, audio_track, audio_codec, audio_bitrate, extension)?;
    eprintln!("Finished converting {}", input.to_string_lossy());
    Ok(())
}
