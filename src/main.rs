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
                .help("Encode to AV1 using rav1e-by-gop (default QP: 60)"),
        )
        .arg(
            Arg::with_name("direct")
                .short("d")
                .long("direct")
                .value_name("A_TRACK")
                .help("remux mkv to mp4; will convert audio streams to opus without touching video")
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
                .help("Audio bitrate per channel in kbps (Default: 80 kbps/channel)")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("high-bd")
                .long("high-bd")
                .long("highbd")
                .help("output as 10-bit video"),
        )
        .arg(
            Arg::with_name("keep-audio")
                .short("a")
                .long("keep-audio")
                .help("copy the audio without reencoding"),
        )
        .arg(
            Arg::with_name("skip-video")
                .long("skip-video")
                .help("assume the video has already been encoded (will use .out.mkv files)"),
        )
        .arg(
            Arg::with_name("tiles")
                .long("tiles")
                .help("the number of tiles to use for av1 encoding (default: 1)")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("local-workers")
                .long("local-workers")
                .help("the number of local workers to use for av1 encoding (default: auto)")
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
    } else {
        Encoder::X264
    };
    let crf = match encoder {
        Encoder::Rav1e => args
            .value_of("crf")
            .unwrap_or("60")
            .parse::<u8>()
            .expect(CRF_PARSE_ERROR),
        Encoder::X264 => {
            let crf = args
                .value_of("crf")
                .unwrap_or("18")
                .parse::<u8>()
                .expect(CRF_PARSE_ERROR);
            assert!(crf <= 51, CRF_PARSE_ERROR);
            crf
        }
    };
    let highbd = args.is_present("high-bd");
    let audio_track = args.value_of("audio_track").unwrap().parse().unwrap();
    let audio_bitrate = args
        .value_of("audio_bitrate")
        .unwrap_or("80")
        .parse::<u32>()
        .unwrap();
    let tiles = args.value_of("tiles").map(|val| val.parse().unwrap());
    let workers = args
        .value_of("local-workers")
        .map(|val| val.parse().unwrap());

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
                    !args.is_present("keep-audio"),
                    audio_bitrate,
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
                !args.is_present("keep-audio"),
                audio_bitrate,
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
                highbd,
                args.is_present("keep-audio"),
                args.is_present("skip-video"),
                audio_track,
                audio_bitrate,
                tiles,
                workers,
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
            highbd,
            args.is_present("keep-audio"),
            args.is_present("skip-video"),
            audio_track,
            audio_bitrate,
            tiles,
            workers,
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
    highbd: bool,
    keep_audio: bool,
    skip_video: bool,
    audio_track: AudioTrack,
    audio_bitrate: u32,
    tiles: Option<u8>,
    workers: Option<u8>,
) -> Result<(), String> {
    eprintln!("Converting {}", input.to_string_lossy());
    let dims = get_video_dimensions(input)?;
    if !skip_video {
        match encoder {
            Encoder::X264 => convert_video_x264(input, profile, crf, highbd, dims),
            Encoder::Rav1e => convert_video_rav1e(input, crf, dims, tiles, workers),
        }?;
    }
    if target == Target::Local {
        // TODO: Handle audio and muxing for dist encodes
        convert_audio(input, !keep_audio, audio_track, audio_bitrate)?;
        mux_mp4(input, encoder)?;
    }
    eprintln!("Finished converting {}", input.to_string_lossy());
    Ok(())
}

fn process_direct(
    input: &Path,
    audio_track: AudioTrack,
    convert_audio: bool,
    audio_bitrate: u32,
) -> Result<(), String> {
    eprintln!("Converting {}", input.to_string_lossy());
    mux_mp4_direct(input, audio_track, convert_audio, audio_bitrate)?;
    eprintln!("Finished converting {}", input.to_string_lossy());
    Ok(())
}
