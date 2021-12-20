#![warn(clippy::all)]

#[macro_use]
extern crate dotenv_codegen;

mod input;
mod output;
mod parse;

use std::{
    env,
    fs,
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use ansi_term::Colour::{Blue, Green, Red};
use anyhow::Result;
use clap::{App, Arg};
use itertools::Itertools;
use lexical_sort::natural_lexical_cmp;
use walkdir::WalkDir;

use self::{input::*, output::*};
use crate::parse::{parse_filters, ParsedFilter, Track, TrackSource};

fn main() {
    env::set_var("RUST_BACKTRACE", "1");
    let args = App::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .arg(
            Arg::with_name("verbose")
                .long("verbose")
                .short("v")
                .help("print more stats"),
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
- ab=#: Audio bitrate per channel in Kb/sec [default: 96 for aac, 64 for opus]
- at=#[e][f]: Audio tracks, comma separated [default: 1, e=enabled, f=forced]

Subtitle options:
- st=#[e][f]: Subtitle tracks, comma separated [default: None, e=enabled, f=forced]"#,
                ),
        )
        .arg(
            Arg::with_name("input")
                .help("Sets the input directory or file")
                .required(true)
                .index(1),
        )
        .arg(
            Arg::with_name("output-dir")
                .long("output")
                .short("o")
                .value_name("DIR")
                .help(&format!(
                    "Override the default output directory [default on this machine: {}]",
                    dotenv!("OUTPUT_PATH")
                )),
        )
        .get_matches();

    let input = Path::new(args.value_of("input").expect("No input path provided"));
    assert!(input.exists(), "Input path does not exist");
    let outputs = args
        .value_of("formats")
        .map(|formats| {
            let formats = formats.trim();
            if formats.is_empty() {
                return vec![Output::default()];
            }
            formats
                .split(';')
                .map(|format| {
                    let mut output = Output::default();
                    let filters = parse_filters(format, input);
                    if let Some(encoder) = filters.iter().find_map(|filter| {
                        if let ParsedFilter::VideoEncoder(encoder) = filter {
                            Some(encoder)
                        } else {
                            None
                        }
                    }) {
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
                    for filter in &filters {
                        apply_filter(filter, &mut output);
                    }
                    output
                })
                .collect()
        })
        .unwrap_or_else(|| vec![Output::default()]);

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
            .sorted_unstable_by(|a, b| {
                natural_lexical_cmp(&a.to_string_lossy(), &b.to_string_lossy())
            })
            .collect()
    } else {
        panic!("Input is neither a file nor a directory");
    };

    for input in inputs {
        let result = process_file(
            &input,
            &outputs,
            args.value_of("output-dir"),
            args.is_present("keep-lossless"),
            args.is_present("lossless-only"),
            args.is_present("verbose"),
        );
        if let Err(err) = result {
            eprintln!(
                "{} {}: {}",
                Red.bold().paint("[Error]"),
                Red.paint(input.file_name().unwrap().to_string_lossy()),
                Red.paint(err.to_string())
            );
        }
        eprintln!();
    }
}

fn process_file(
    input: &Path,
    outputs: &[Output],
    output_dir: Option<&str>,
    keep_lossless: bool,
    lossless_only: bool,
    verbose: bool,
) -> Result<()> {
    eprintln!(
        "{} {} {} {}",
        Blue.bold().paint("[Info]"),
        Blue.paint("Encoding"),
        Blue.paint(input.file_name().unwrap().to_string_lossy()),
        Blue.paint("lossless")
    );

    let dimensions = get_video_dimensions(input)?;
    create_lossless(input, dimensions)?;
    if lossless_only {
        return Ok(());
    }
    eprintln!();

    for output in outputs {
        let video_suffix = build_video_suffix(output);
        let vpy_file = input.with_extension(&format!("{}.vpy", video_suffix));
        eprintln!(
            "{} {} {}",
            Blue.bold().paint("[Info]"),
            Blue.paint("Encoding"),
            Blue.paint(vpy_file.file_name().unwrap().to_string_lossy())
        );

        build_vpy_script(&vpy_file, input, output);
        let video_out = vpy_file.with_extension("mkv");
        let dimensions = get_video_dimensions(&vpy_file)?;
        loop {
            let result = convert_video_av1an(
                &vpy_file,
                &video_out,
                output.video.encoder,
                dimensions,
                verbose,
            );
            // I hate this lazy workaround,
            // but this is due to a heisenbug in Vapoursynth
            // due to some sort of race condition,
            // which causes crashes often enough to be annoying.
            //
            // Essentially, we retry the encode until it works.
            if result.is_ok() {
                break;
            }
        }

        let audio_tracks = if output.audio_tracks.is_empty() {
            vec![Track {
                source: TrackSource::FromVideo(0),
                enabled: true,
                forced: false,
            }]
        } else {
            output.audio_tracks.clone()
        };
        let mut audio_outputs = Vec::new();
        let mut audio_suffixes = Vec::new();
        for (i, audio_track) in audio_tracks.iter().enumerate() {
            let audio_suffix = format!(
                "{}-{}kbpc-at{}",
                output.audio.encoder, output.audio.kbps_per_channel, i
            );
            let audio_out = input.with_extension(&format!("{}.mka", audio_suffix));
            convert_audio(
                input,
                &audio_out,
                output.audio.encoder,
                audio_track,
                output.audio.kbps_per_channel,
            )?;
            audio_outputs.push((audio_out, audio_track.enabled, audio_track.forced));
            audio_suffixes.push(audio_suffix);
        }
        let audio_suffix = audio_suffixes.join("-");
        let mut output_path = PathBuf::from(output_dir.unwrap_or(dotenv!("OUTPUT_PATH")));
        output_path.push(
            input
                .with_extension(&format!(
                    "{}-{}.{}",
                    video_suffix, audio_suffix, output.video.output_ext
                ))
                .file_name()
                .unwrap(),
        );

        let mut subtitle_outputs = Vec::new();
        if !output.sub_tracks.is_empty() {
            for (i, subtitle) in output.sub_tracks.iter().enumerate() {
                let subtitle_out = input.with_extension(&format!("-{}.ass", i));
                extract_subtitles(&find_source_file(input), i as u8, &subtitle_out)?;
                subtitle_outputs.push((subtitle_out, subtitle.enabled, subtitle.forced));
            }
        }

        mux_video(
            &find_source_file(input),
            &video_out,
            &audio_outputs,
            &subtitle_outputs,
            &output_path,
        )?;

        eprintln!(
            "{} {} {}",
            Green.bold().paint("[Success]"),
            Green.paint("Finished encoding"),
            Green.paint(vpy_file.file_name().unwrap().to_string_lossy())
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

fn apply_filter(filter: &ParsedFilter, output: &mut Output) {
    match filter {
        ParsedFilter::VideoEncoder(_) => (),
        ParsedFilter::Quantizer(arg) => {
            let arg = *arg;
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
        ParsedFilter::Speed(arg) => match output.video.encoder {
            VideoEncoder::Aom { ref mut speed, .. } | VideoEncoder::Rav1e { ref mut speed, .. } => {
                let arg = *arg;
                if arg > 10 {
                    panic!("'s' must be between 0 and 10, received {}", arg);
                }
                *speed = arg;
            }
            _ => (),
        },
        ParsedFilter::Profile(arg) => match output.video.encoder {
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
                *profile = *arg;
            }
        },
        ParsedFilter::Grain(arg) => {
            if let VideoEncoder::Aom { ref mut grain, .. } = output.video.encoder {
                let arg = *arg;
                if arg > 50 {
                    panic!("'grain' must be between 0 and 50, received {}", arg);
                }
                *grain = arg;
            }
        }
        ParsedFilter::Compat(arg) => match output.video.encoder {
            VideoEncoder::X264 { ref mut compat, .. }
            | VideoEncoder::X265 { ref mut compat, .. }
            | VideoEncoder::Aom { ref mut compat, .. } => {
                *compat = *arg;
            }
            _ => (),
        },
        ParsedFilter::Hdr(arg) => match output.video.encoder {
            VideoEncoder::X265 { ref mut is_hdr, .. }
            | VideoEncoder::Aom { ref mut is_hdr, .. }
            | VideoEncoder::Rav1e { ref mut is_hdr, .. } => {
                *is_hdr = *arg;
            }
            _ => panic!("Attempted to use HDR with an unsupported encoder"),
        },
        ParsedFilter::Extension(arg) => {
            output.video.output_ext = arg.to_string();
        }
        ParsedFilter::BitDepth(arg) => {
            output.video.bit_depth = Some(*arg);
        }
        ParsedFilter::Resolution { width, height } => {
            output.video.resolution = Some((*width, *height));
        }
        ParsedFilter::AudioEncoder(arg) => {
            output.audio.encoder = match arg.to_lowercase().as_str() {
                "copy" => AudioEncoder::Copy,
                "flac" => AudioEncoder::Flac,
                "aac" => AudioEncoder::Aac,
                "opus" => AudioEncoder::Opus,
                arg => panic!("Invalid value provided for 'aenc': {}", arg),
            }
        }
        ParsedFilter::AudioBitrate(arg) => {
            let arg = *arg;
            if arg == 0 {
                panic!("'ab' must be greater than 0, got {}", arg);
            }
            output.audio.kbps_per_channel = arg;
        }
        ParsedFilter::AudioTracks(args) => {
            output.audio_tracks = args.clone();
        }
        ParsedFilter::SubtitleTracks(args) => {
            output.sub_tracks = args.clone();
        }
    }
}

fn build_video_suffix(output: &Output) -> String {
    let mut codec_str = match output.video.encoder {
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
    };
    if let Some(res) = output.video.resolution {
        codec_str.push_str(&format!("-{}x{}", res.0, res.1));
    }
    if let Some(bd) = output.video.bit_depth {
        codec_str.push_str(&format!("-{}b", bd));
    }
    codec_str
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
