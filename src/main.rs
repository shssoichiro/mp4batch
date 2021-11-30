#![warn(clippy::all)]

#[macro_use]
extern crate dotenv_codegen;

mod input;
mod output;

use std::{
    cmp::Ordering,
    env,
    fs,
    fs::File,
    io::{BufWriter, Write},
    path::Path,
    process::exit,
    str::FromStr,
};

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
            Arg::with_name("scale")
                .long("scale")
                .value_name("FILTERS")
                .help(
                    "Takes a list of desired formats to output. Each filter is comma separated, \
                     each output format is semicolon separated\n- orig: Output without scaling\n- \
                     b#: Output bit depth\n- r#x#: Output resolution",
                ),
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
    let scale_filters = args
        .value_of("scale")
        .map(|scale| {
            scale
                .split(';')
                .map(|chain| {
                    if chain == "orig" {
                        return Filter::Original;
                    }

                    let mut parsed = Filter::Scaled {
                        resolution: None,
                        bit_depth: None,
                    };
                    for filter in chain.split(',') {
                        if let Some(filter) = filter.strip_prefix('r') {
                            if let Filter::Scaled {
                                resolution: Some(_),
                                ..
                            } = parsed
                            {
                                panic!(
                                    "Each filter chain may only specify one resolution: {}",
                                    chain
                                );
                            }
                            let resolution = filter
                                .split_once('x')
                                .unwrap_or_else(|| panic!("Invalid resolution filter: {}", filter));
                            let width = resolution.0.parse().unwrap_or_else(|_| {
                                panic!("Invalid resolution filter: {}", filter)
                            });
                            let height = resolution.1.parse().unwrap_or_else(|_| {
                                panic!("Invalid resolution filter: {}", filter)
                            });
                            if let Filter::Scaled {
                                ref mut resolution, ..
                            } = parsed
                            {
                                *resolution = Some((width, height));
                            }
                        } else if let Some(filter) = filter.strip_prefix('b') {
                            if let Filter::Scaled {
                                bit_depth: Some(_), ..
                            } = parsed
                            {
                                panic!(
                                    "Each filter chain may only specify one bit depth: {}",
                                    chain
                                );
                            }
                            let depth = filter
                                .parse()
                                .unwrap_or_else(|_| panic!("Invalid bit depth: {}", filter));
                            if ![8, 10].contains(&depth) {
                                panic!("Invalid bit depth: {}", filter);
                            }
                            if let Filter::Scaled {
                                ref mut bit_depth, ..
                            } = parsed
                            {
                                *bit_depth = Some(depth);
                            }
                        } else if filter == "orig" {
                            panic!(
                                "\"orig\" filter may not be specified with other filters: {}",
                                chain
                            );
                        } else {
                            panic!("Unrecognized filter argument: {}", filter);
                        }
                    }
                    parsed
                })
                .unique()
                .sorted_by(|a, _b| {
                    if let Filter::Original = a {
                        Ordering::Less
                    } else {
                        Ordering::Equal
                    }
                })
                .collect()
        })
        .unwrap_or_else(|| vec![Filter::Original]);
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
                &scale_filters,
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
            &scale_filters,
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
    scale_filters: &[Filter],
) -> Result<(), String> {
    for (i, filter_chain) in scale_filters.iter().enumerate() {
        let orig_input = input;
        eprintln!("Converting {}", input.to_string_lossy());
        let dimensions = get_video_dimensions(input)?;

        let hdr_info = match encoder {
            Encoder::Aom { is_hdr, .. } | Encoder::Rav1e { is_hdr, .. } if is_hdr => {
                Some(get_hdr_info(&find_source_file(orig_input))?)
            }
            _ => None,
        };

        if !skip_video {
            loop {
                if filter_chain == &Filter::Original || i == 0 {
                    create_lossless(orig_input, dimensions)?;
                    if lossless_only {
                        exit(0);
                    }
                }

                let input = match filter_chain {
                    Filter::Original => orig_input.to_path_buf(),
                    Filter::Scaled {
                        resolution,
                        bit_depth,
                    } => {
                        let input = orig_input.with_extension(&format!(
                            "{}{}vpy",
                            resolution
                                .map(|(w, h)| format!("{}x{}.", w, h))
                                .unwrap_or_else(String::new),
                            bit_depth
                                .map(|bd| format!("{}.", bd))
                                .unwrap_or_else(String::new),
                        ));
                        let mut script = BufWriter::new(File::create(&input).unwrap());
                        writeln!(&mut script, "import vapoursynth as vs").unwrap();
                        writeln!(&mut script, "core = vs.get_core()").unwrap();
                        writeln!(
                            &mut script,
                            "clip = core.lsmas.LWLibavSource(source=\"{}\")",
                            escape_python_string(
                                orig_input
                                    .with_extension("lossless.mkv")
                                    .canonicalize()
                                    .unwrap()
                                    .to_str()
                                    .unwrap()
                            )
                        )
                        .unwrap();
                        // We downscale resolution first because it's more likely that
                        // we would be going from 10 bit to 8 bit, rather than the other way.
                        // So this gives the best quality.
                        if let Some((w, h)) = resolution {
                            writeln!(
                                &mut script,
                                "clip = core.resize.Spline36(clip, {}, {}, \
                                 dither_type='error_diffusion')",
                                w, h
                            )
                            .unwrap();
                        }
                        if let Some(bd) = bit_depth {
                            writeln!(&mut script, "import vsutil").unwrap();
                            writeln!(&mut script, "clip = vsutil.depth(clip, {})", bd).unwrap();
                        }
                        script.flush().unwrap();
                        input
                    }
                };

                let result = convert_video_av1an(&input, encoder, dimensions, hdr_info.as_ref());
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

        convert_audio(
            orig_input,
            &input.with_extension("out.mka"),
            audio_codec,
            audio_track.clone(),
            audio_bitrate,
        )?;
        mux_video(input, extension)?;

        eprintln!("Finished converting {}", input.to_string_lossy());
    }

    if !keep_lossless {
        let _ = fs::remove_file(input.with_extension("lossless.mkv"));
    }

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

fn escape_python_string(input: &str) -> String {
    input.replace(r"\", r"\\").replace(r"'", r"\'")
}
