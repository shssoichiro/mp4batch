#![warn(clippy::all)]

#[macro_use]
extern crate dotenv_codegen;

mod input;
mod output;

use std::{
    env,
    fs,
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use ansi_term::Colour::{Blue, Green, Red};
use clap::{App, Arg};
use itertools::Itertools;
use walkdir::WalkDir;

use self::{input::*, output::*};

fn main() {
    env::set_var("RUST_BACKTRACE", "1");
    let args = App::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
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
            Arg::with_name("formats")
                .long("formats")
                .short("f")
                .value_name("FILTERS")
                .help(
                    r#"Takes a list of desired formats to output.
Each filter is comma separated, each output is semicolon separated.

Video encoder options:
- enc=str: Encoder to use [default: x264] [options: copy, x264, x265, aom, rav1e]
- q=#: QP or CRF [default: varies by encoder]
- s=#: Speed/cpu-used [aom/rav1e only] [default: varies by encoder]
- p=str: Encoder settings to use [default: film] [options: film, anime, fast]
- grain=#: Grain synth level [aom only] [0-50, 0 = disabled]
- compat=0/1: Enable extra playback compatibility/DXVA options
- hdr=0/1: Enable HDR encoding features
- ext=mkv/mp4: Output file format [default: mkv]

Video filters (any unset will leave the input unchanged):
- bd=#: Output bit depth
- res=#x#: Output resolution

Audio encoder options:
- aenc=str: Audio encoder to use [default: copy] [options: copy, aac, flac, opus]
- ab=#: Audio bitrate per channel in Kb/sec [default: 96 for aac, 64 for opus]"#,
                ),
        )
        .arg(
            Arg::with_name("input")
                .help("Sets the input directory or file")
                .required(true)
                .index(1),
        )
        .get_matches();

    let input = args.value_of("input").expect("No input path provided");
    let outputs = args
        .value_of("formats")
        .map(|formats| {
            let formats = formats.trim();
            if formats.is_empty() {
                return vec![Output::default()];
            }
            formats
                .split(';')
                .enumerate()
                .map(|(i, format)| {
                    let mut output = Output::default();
                    let filters = format
                        .split(',')
                        .map(|filter| {
                            filter.trim().split_once('=').unwrap_or_else(|| {
                                panic!("Invalid filter in output {}: {}", i, filter)
                            })
                        })
                        .collect::<Vec<_>>();
                    for (filter, group) in filters.iter().group_by(|(filter, _)| filter).into_iter()
                    {
                        if group.count() > 1 {
                            panic!("Duplicate filter in output {}: {}", i, filter);
                        }
                    }
                    if let Some((_, encoder)) = filters.iter().find(|(filter, _)| filter == &"enc")
                    {
                        match encoder.to_lowercase().as_str() {
                            "x264" => {
                                // This is the default, do nothing
                            }
                            "x265" => {
                                output.video.encoder = VideoEncoder::X265 {
                                    crf: 18,
                                    profile: Profile::Film,
                                    compat: false,
                                    is_hdr: true,
                                }
                            }
                            "aom" => {
                                output.video.encoder = VideoEncoder::Aom {
                                    crf: 16,
                                    speed: 4,
                                    profile: Profile::Film,
                                    is_hdr: false,
                                    grain: 0,
                                    compat: false,
                                }
                            }
                            "rav1e" => {
                                output.video.encoder = VideoEncoder::Rav1e {
                                    crf: 40,
                                    speed: 5,
                                    profile: Profile::Film,
                                    is_hdr: false,
                                }
                            }
                            enc => panic!("Unrecognized encoder: {}", enc),
                        }
                    }
                    for (filter, arg) in &filters {
                        parse_filter(filter, arg, &mut output);
                    }
                    output
                })
                .collect()
        })
        .unwrap_or_else(|| vec![Output::default()]);

    let input = Path::new(input);
    assert!(input.exists(), "Input path does not exist");

    let inputs = if input.is_file() {
        vec![input.to_path_buf()]
    } else if input.is_dir() {
        WalkDir::new(input)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext.to_string_lossy().to_string())
                    == Some("vpy".to_string())
            })
            .filter(|e| {
                let filestem = e.path().file_stem().unwrap().to_string_lossy();
                !(filestem.contains(".aom-q")
                    || filestem.contains(".rav1e-q")
                    || filestem.contains(".x264-q")
                    || filestem.contains(".x265-q"))
            })
            .map(|e| e.path().to_path_buf())
            .collect()
    } else {
        panic!("Input is neither a file nor a directory");
    };

    for input in inputs {
        let result = process_file(
            &input,
            &outputs,
            args.is_present("keep-lossless"),
            args.is_present("lossless-only"),
        );
        if let Err(err) = result {
            eprintln!(
                "{} {}: {}",
                Red.bold().paint("[Error]"),
                Red.paint(input.as_os_str().to_string_lossy()),
                Red.paint(err)
            );
        }
        eprintln!();
    }
}

#[allow(clippy::too_many_arguments)]
fn process_file(
    input: &Path,
    outputs: &[Output],
    keep_lossless: bool,
    lossless_only: bool,
) -> Result<(), String> {
    eprintln!(
        "{} {} {} {}",
        Blue.bold().paint("[Info]"),
        Blue.paint("Encoding"),
        Blue.paint(input.to_string_lossy()),
        Blue.paint("lossless")
    );

    let dimensions = get_video_dimensions(input)?;
    create_lossless(input, dimensions)?;
    if lossless_only {
        return Ok(());
    }
    eprintln!();

    let audio_track = find_external_audio(input, 0);
    for output in outputs {
        let video_suffix = build_video_suffix(output);
        let vpy_file = input.with_extension(&format!("{}.vpy", video_suffix));
        eprintln!(
            "{} {} {}",
            Blue.bold().paint("[Info]"),
            Blue.paint("Encoding"),
            Blue.paint(vpy_file.to_string_lossy())
        );

        let dimensions = get_video_dimensions(input)?;
        let hdr_info = match output.video.encoder {
            VideoEncoder::Aom { is_hdr, .. }
            | VideoEncoder::Rav1e { is_hdr, .. }
            | VideoEncoder::X265 { is_hdr, .. }
                if is_hdr =>
            {
                Some(get_hdr_info(&find_source_file(input))?)
            }
            _ => None,
        };

        build_vpy_script(&vpy_file, input, output);
        let video_out = vpy_file.with_extension("mkv");
        loop {
            let result = convert_video_av1an(
                &vpy_file,
                &video_out,
                output.video.encoder,
                dimensions,
                hdr_info.as_ref(),
            );
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

        let audio_suffix = &format!(
            "{}-{}kbpc",
            output.audio.encoder, output.audio.kbps_per_channel
        );
        let audio_out = input.with_extension(&format!("{}.mka", audio_suffix));
        convert_audio(
            input,
            &audio_out,
            output.audio.encoder,
            audio_track.clone(),
            output.audio.kbps_per_channel,
        )?;
        let mut output_path = PathBuf::from(dotenv!("OUTPUT_PATH"));
        output_path.push(
            input
                .with_extension(&format!(
                    "{}-{}.{}",
                    video_suffix, audio_suffix, output.video.output_ext
                ))
                .file_name()
                .unwrap(),
        );
        mux_video(&video_out, &audio_out, &output_path)?;

        eprintln!(
            "{} {} {}",
            Green.bold().paint("[Success]"),
            Green.paint("Finished encoding"),
            Green.paint(vpy_file.to_string_lossy())
        );
        eprintln!();
    }

    if !keep_lossless {
        let _ = fs::remove_file(input.with_extension("lossless.mkv"));
    }

    Ok(())
}

fn escape_python_string(input: &str) -> String {
    input.replace(r"\", r"\\").replace(r"'", r"\'")
}

fn parse_filter(filter: &str, arg: &str, output: &mut Output) {
    match filter.to_lowercase().as_str() {
        "q" | "qp" | "crf" => {
            let arg = arg
                .parse()
                .unwrap_or_else(|_| panic!("Invalid value provided for 'q': {}", arg));
            let range = match output.video.encoder {
                VideoEncoder::X264 { ref mut crf, .. } => {
                    *crf = arg;
                    (-12, 51)
                }
                VideoEncoder::X265 { ref mut crf, .. } => {
                    *crf = arg;
                    (0, 51)
                }
                VideoEncoder::Aom { ref mut crf, .. } => {
                    *crf = arg;
                    (0, 63)
                }
                VideoEncoder::Rav1e { ref mut crf, .. } => {
                    *crf = arg;
                    (0, 255)
                }
            };
            if arg < range.0 || arg > range.1 {
                panic!(
                    "'q' must be between {} and {}, received {}",
                    range.0, range.1, arg
                );
            }
        }
        "s" | "speed" => match output.video.encoder {
            VideoEncoder::Aom { ref mut speed, .. } | VideoEncoder::Rav1e { ref mut speed, .. } => {
                let arg = arg
                    .parse()
                    .unwrap_or_else(|_| panic!("Invalid value provided for 's': {}", arg));
                if arg > 10 {
                    panic!("'s' must be between 0 and 10, received {}", arg);
                }
                *speed = arg;
            }
            _ => (),
        },
        "p" | "profile" => match output.video.encoder {
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
            } => {
                *profile = match arg {
                    "film" => Profile::Film,
                    "anime" => Profile::Anime,
                    "fast" => Profile::Fast,
                    arg => panic!("Invalid value provided for 'profile': {}", arg),
                };
            }
        },
        "grain" => {
            if let VideoEncoder::Aom { ref mut grain, .. } = output.video.encoder {
                let arg = arg
                    .parse()
                    .unwrap_or_else(|_| panic!("Invalid value provided for 'grain': {}", arg));
                if arg > 50 {
                    panic!("'grain' must be between 0 and 50, received {}", arg);
                }
                *grain = arg;
            }
        }
        "compat" => match arg {
            "1" => match output.video.encoder {
                VideoEncoder::X264 { ref mut compat, .. }
                | VideoEncoder::X265 { ref mut compat, .. }
                | VideoEncoder::Aom { ref mut compat, .. } => {
                    *compat = true;
                }
                _ => (),
            },
            "0" => (),
            arg => panic!("Invalid value provided for 'compat': {}", arg),
        },
        "hdr" => match arg {
            "1" => match output.video.encoder {
                VideoEncoder::X265 { ref mut is_hdr, .. }
                | VideoEncoder::Aom { ref mut is_hdr, .. }
                | VideoEncoder::Rav1e { ref mut is_hdr, .. } => {
                    *is_hdr = true;
                }
                _ => panic!("Attempted to use HDR with an unsupported encoder"),
            },
            "0" => (),
            arg => panic!("Invalid value provided for 'hdr': {}", arg),
        },
        "ext" => {
            let arg = arg.to_lowercase();
            if arg == "mkv" || arg == "mp4" {
                output.video.output_ext = arg;
            } else {
                panic!("Unrecognized output extension requested: {}", arg);
            }
        }
        "bd" => {
            output.video.bit_depth = Some(match arg {
                "8" => 8,
                "10" => 10,
                arg => panic!("Invalid value provided for 'bd': {}", arg),
            });
        }
        "res" => {
            let (width, height) = arg
                .split_once('x')
                .unwrap_or_else(|| panic!("Invalid value provided for 'res': {}", arg));
            let width = width
                .parse()
                .unwrap_or_else(|_| panic!("Invalid value provided for 'res': {}", arg));
            let height = height
                .parse()
                .unwrap_or_else(|_| panic!("Invalid value provided for 'res': {}", arg));
            if width < 64 || height < 64 {
                panic!("Resolution must be at least 64x64 pixels, got {}", arg);
            }
            if width % 2 != 0 || height % 2 != 0 {
                panic!("Resolution must be mod 2, got {}", arg);
            }
            output.video.resolution = Some((width, height));
        }
        "aenc" => {
            output.audio.encoder = match arg.to_lowercase().as_str() {
                "copy" => AudioEncoder::Copy,
                "flac" => AudioEncoder::Flac,
                "aac" => AudioEncoder::Aac,
                "opus" => AudioEncoder::Opus,
                arg => panic!("Invalid value provided for 'aenc': {}", arg),
            }
        }
        "ab" => {
            let ab = arg
                .parse()
                .unwrap_or_else(|_| panic!("Invalid value provided for 'ab': {}", arg));
            if ab == 0 {
                panic!("'ab' must be greater than 0, got {}", arg);
            }
            output.audio.kbps_per_channel = ab;
        }
        "enc" => (),
        filter => panic!("Unrecognized filter: {}", filter),
    }
}

fn build_video_suffix(output: &Output) -> String {
    match output.video.encoder {
        VideoEncoder::Aom {
            crf,
            speed,
            profile,
            is_hdr,
            grain,
            compat,
        } => format!(
            "aom-q{}-s{}-{}{}-g{}{}",
            crf,
            speed,
            profile,
            if is_hdr { "-hdr" } else { "" },
            grain,
            if compat { "-compat" } else { "" }
        ),
        VideoEncoder::Rav1e {
            crf,
            speed,
            profile,
            is_hdr,
        } => format!(
            "rav1e-q{}-s{}-{}{}",
            crf,
            speed,
            profile,
            if is_hdr { "-hdr" } else { "" }
        ),
        VideoEncoder::X264 {
            crf,
            profile,
            compat,
        } => format!(
            "x264-q{}-{}{}",
            crf,
            profile,
            if compat { "-compat" } else { "" }
        ),
        VideoEncoder::X265 {
            crf,
            profile,
            compat,
            is_hdr,
        } => format!(
            "x265-q{}-{}{}{}",
            crf,
            profile,
            if is_hdr { "-hdr" } else { "" },
            if compat { "-compat" } else { "" }
        ),
    }
}

fn build_vpy_script(filename: &Path, input: &Path, output: &Output) {
    let mut script = BufWriter::new(File::create(&filename).unwrap());
    writeln!(&mut script, "import vapoursynth as vs").unwrap();
    writeln!(&mut script, "core = vs.core").unwrap();
    writeln!(
        &mut script,
        "clip = core.lsmas.LWLibavSource(source=\"{}\")",
        escape_python_string(
            input
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
    if let Some((w, h)) = output.video.resolution {
        writeln!(
            &mut script,
            "clip = core.resize.Spline36(clip, {}, {}, dither_type='error_diffusion')",
            w, h
        )
        .unwrap();
    }
    if let Some(bd) = output.video.bit_depth {
        writeln!(&mut script, "import vsutil").unwrap();
        writeln!(&mut script, "clip = vsutil.depth(clip, {})", bd).unwrap();
    }
    writeln!(&mut script, "clip.set_output()").unwrap();
    script.flush().unwrap();
}
