#![warn(clippy::all)]

#[macro_use]
extern crate dotenv_codegen;

mod input;
mod output;

use std::{env, path::Path, str::FromStr};

use clap::{App, Arg};
use glob::glob;
use itertools::Itertools;

use self::{input::*, output::*};

fn main() {
    env::set_var("RUST_BACKTRACE", "1");
    let args = App::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .arg(
            Arg::with_name("profile")
                .short("p")
                .long("profile")
                .value_name("VALUE")
                .help("Sets a custom profile (default: film, available: film, anime, fast)")
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
                .help("Encode to AV1 using aomenc (default QP: 30)"),
        )
        .arg(
            Arg::with_name("rav1e")
                .long("rav1e")
                .help("Encode to AV1 using rav1e (default QP: 60)"),
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
                .help("this video should be encoded as HDR"),
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
            Arg::with_name("keep-lossless")
                .long("keep-lossless")
                .help("don't delete the lossless intermediate encode"),
        )
        .arg(
            Arg::with_name("lossless-only")
                .long("lossless-only")
                .help("quit after making the lossless video"),
        )
        .arg(
            Arg::with_name("mp4")
                .long("mp4")
                .help("output to mp4 instead of mkv"),
        )
        .arg(
            Arg::with_name("speed")
                .long("speed")
                .short("s")
                .help("the speed level to use for aomenc (default: 4) or rav1e (default: 5)")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("compat")
                .long("compat")
                .help("defines the compatibility level to use for encoding")
                .takes_value(true)
                .possible_values(&["dxva", "normal", "none"])
                .default_value("normal"),
        )
        .arg(
            Arg::with_name("grain")
                .long("grain")
                .value_name("VALUE")
                .help("Grain synthesis noise level (0-50, aomenc only)")
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
    let compat = Compat::from_str(args.value_of("compat").unwrap_or("normal")).unwrap();
    let encoder = if args.is_present("av1") {
        Encoder::Aom {
            crf: args
                .value_of("crf")
                .unwrap_or("30")
                .parse::<u8>()
                .expect(CRF_PARSE_ERROR),
            speed: args
                .value_of("speed")
                .map(|val| val.parse::<u8>().unwrap_or(4)),
            profile,
            is_hdr: args.is_present("hdr"),
            grain: args
                .value_of("grain")
                .map(|val| val.parse::<u8>().unwrap())
                .unwrap_or(0),
            compat,
        }
    } else if args.is_present("rav1e") {
        Encoder::Rav1e {
            crf: args
                .value_of("crf")
                .unwrap_or("60")
                .parse::<u8>()
                .expect(CRF_PARSE_ERROR),
            speed: args
                .value_of("speed")
                .map(|val| val.parse::<u8>().unwrap_or(5)),
            profile,
            is_hdr: args.is_present("hdr"),
        }
    } else if args.is_present("x265") {
        Encoder::X265 {
            crf: args
                .value_of("crf")
                .unwrap_or("18")
                .parse::<u8>()
                .expect(CRF_PARSE_ERROR),
            profile,
            compat,
        }
    } else {
        Encoder::X264 {
            crf: args
                .value_of("crf")
                .unwrap_or("18")
                .parse::<u8>()
                .expect(CRF_PARSE_ERROR),
            profile,
            compat,
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
            for entry in glob(&format!("{}/**/*.mkv", input.to_string_lossy()))
                .unwrap()
                .filter_map(Result::ok)
                .sorted()
            {
                let audio_track = find_external_audio(&entry, track);
                let result = process_direct(
                    &entry,
                    audio_track,
                    args.value_of("acodec").unwrap_or("copy"),
                    audio_bitrate,
                    extension,
                );
                if let Err(err) = result {
                    eprintln!(
                        "An error occurred for {}: {}",
                        entry.as_os_str().to_string_lossy(),
                        err
                    );
                }
                eprintln!();
                eprintln!();
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
        for entry in glob(&format!("{}/**/*.vpy", input.to_string_lossy()))
            .unwrap()
            .filter_map(|e| e.ok())
            .sorted()
        {
            let audio_track = find_external_audio(&entry, audio_track);
            let result = process_file(
                &entry,
                encoder,
                args.value_of("acodec").unwrap_or("copy"),
                args.is_present("skip-video"),
                audio_track,
                audio_bitrate,
                extension,
                args.is_present("keep-lossless"),
                args.is_present("lossless-only"),
            );
            if let Err(err) = result {
                eprintln!(
                    "An error occurred for {}: {}",
                    entry.as_os_str().to_string_lossy(),
                    err
                );
            }
            eprintln!();
            eprintln!();
        }
    } else {
        let audio_track = find_external_audio(input, audio_track);
        process_file(
            input,
            encoder,
            args.value_of("acodec").unwrap_or("copy"),
            args.is_present("skip-video"),
            audio_track,
            audio_bitrate,
            extension,
            args.is_present("keep-lossless"),
            args.is_present("lossless-only"),
        )
        .unwrap();
    }
}

#[allow(clippy::too_many_arguments)]
fn process_file(
    input: &Path,
    encoder: Encoder,
    audio_codec: &str,
    skip_video: bool,
    audio_track: AudioTrack,
    audio_bitrate: u32,
    extension: &str,
    keep_lossless: bool,
    lossless_only: bool,
) -> Result<(), String> {
    eprintln!("Converting {}", input.to_string_lossy());
    let dims = get_video_dimensions(input)?;
    if !skip_video {
        loop {
            let result = convert_video_av1an(input, encoder, dims, keep_lossless, lossless_only);
            // I hate this lazy workaround,
            // but this is due to a heisenbug in DFTTest
            // due to some sort of race condition,
            // which causes crashes often enough to be annoying.
            //
            // Essentially, we retry the encode until it works.
            if result.is_ok() {
                break;
            }
        }
    }

    convert_audio(input, audio_codec, audio_track, audio_bitrate)?;
    mux_video(input, extension)?;

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
