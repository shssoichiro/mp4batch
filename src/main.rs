#![warn(clippy::all)]

#[macro_use]
extern crate dotenv_codegen;

mod input;
mod output;
mod parse;

use std::{
    borrow::Cow,
    env,
    fs,
    fs::{read_to_string, File},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
};

use ansi_term::Colour::{Blue, Green, Red};
use anyhow::Result;
use clap::Parser;
use itertools::Itertools;
use lexical_sort::natural_lexical_cmp;
use path_clean::PathClean;
use walkdir::WalkDir;

use self::{input::*, output::*};
use crate::parse::{parse_filters, ParsedFilter, Track, TrackSource};

#[derive(Parser, Debug)]
struct InputArgs {
    /// Sets the input directory or file
    pub input: String,

    /// Override the default output directory
    #[clap(short, long, value_name = "DIR")]
    pub output: Option<String>,

    /// Takes a list of desired formats to output.
    /// Each filter is comma separated, each output is semicolon separated.
    ///
    ///
    /// Video encoder options:
    ///
    /// - enc=str: Encoder to use [default: x264] [options: copy, x264, x265, aom, rav1e]
    /// - q=#: QP or CRF [default: varies by encoder]
    /// - s=#: Speed/cpu-used [aom/rav1e only] [default: varies by encoder]
    /// - p=str: Encoder settings to use [default: film] [options: film, anime, fast]
    /// - grain=#: Grain synth level [aom only] [0-50, 0 = disabled]
    /// - compat=0/1: Enable extra playback compatibility/DXVA options
    /// - hdr=0/1: Enable HDR encoding features
    /// - ext=mkv/mp4: Output file format [default: mkv]
    ///
    /// Video filters (any unset will leave the input unchanged):
    ///
    /// - bd=#: Output bit depth
    /// - res=#x#: Output resolution
    ///
    /// Audio encoder options:
    ///
    /// - aenc=str: Audio encoder to use [default: copy] [options: copy, aac, flac, opus]
    /// - ab=#: Audio bitrate per channel in Kb/sec [default: 96 for aac, 64 for opus]
    /// - at=#-[e][f]: Audio tracks, pipe separated [default: 0, e=enabled, f=forced]
    ///
    /// Subtitle options:
    ///
    /// - st=#-[e][f]: Subtitle tracks, pipe separated [default: None, e=enabled, f=forced]
    #[clap(short, long, value_name = "FILTERS", verbatim_doc_comment)]
    pub formats: Option<String>,

    /// Don't delete the lossless intermediate encode
    #[clap(long)]
    pub keep_lossless: bool,

    /// Quit after making the lossless video
    #[clap(long)]
    pub lossless_only: bool,

    /// Do not create a lossless before running av1an.
    ///
    /// Useful for encodes with very little or no filtering.
    #[clap(long)]
    pub skip_lossless: bool,
}

fn main() {
    env::set_var("RUST_BACKTRACE", "1");
    let args = InputArgs::parse();

    let input = Path::new(&args.input);
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
            .sorted_unstable_by(|a, b| {
                natural_lexical_cmp(&a.to_string_lossy(), &b.to_string_lossy())
            })
            .collect()
    } else {
        panic!("Input is neither a file nor a directory");
    };

    for input in inputs {
        let outputs = args
            .formats
            .as_ref()
            .map(|formats| {
                let formats = formats.trim();
                if formats.is_empty() {
                    return vec![Output::default()];
                }
                formats
                    .split(';')
                    .map(|format| {
                        let mut output = Output::default();
                        let filters = parse_filters(format, &input);
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

        let result = process_file(
            &input,
            &outputs,
            args.output.as_deref(),
            args.keep_lossless,
            args.lossless_only,
            args.skip_lossless,
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
    skip_lossless: bool,
) -> Result<()> {
    if !skip_lossless {
        eprintln!(
            "{} {} {} {}",
            Blue.bold().paint("[Info]"),
            Blue.paint("Encoding"),
            Blue.paint(input.file_name().unwrap().to_string_lossy()),
            Blue.paint("lossless")
        );
        let dimensions = get_video_dimensions(input)?;
        create_lossless(input, dimensions)?;
        eprintln!();
    }

    if lossless_only {
        if skip_lossless {
            eprintln!(
                "Received both --lossless-only and --skip-lossless. Doing nothing. This is \
                 probably a mistake."
            );
        }
        return Ok(());
    }

    for output in outputs {
        let video_suffix = build_video_suffix(output);
        let vpy_file = input.with_extension(&format!("{}.vpy", video_suffix));
        eprintln!(
            "{} {} {}",
            Blue.bold().paint("[Info]"),
            Blue.paint("Encoding"),
            Blue.paint(vpy_file.file_name().unwrap().to_string_lossy())
        );

        let input_to_script = if skip_lossless {
            Cow::Owned(input.with_extension("vpy"))
        } else {
            Cow::Borrowed(input)
        };
        build_vpy_script(&vpy_file, input_to_script.as_ref(), output, skip_lossless);
        let video_out = vpy_file.with_extension("mkv");
        let dimensions = get_video_dimensions(&vpy_file)?;
        loop {
            let result =
                convert_video_av1an(&vpy_file, &video_out, output.video.encoder, dimensions);
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
                let mut subtitle_out;
                match &subtitle.source {
                    TrackSource::External(path) => {
                        let ext = path.extension().unwrap().to_str().unwrap();
                        subtitle_out = input.with_extension(&format!("{}.{}", i, ext));
                        fs::copy(path, &subtitle_out)?;
                    }
                    TrackSource::FromVideo(j) => {
                        subtitle_out = input.with_extension(&format!("{}.ass", i));
                        if extract_subtitles(&find_source_file(input), *j, &subtitle_out).is_err() {
                            subtitle_out = input.with_extension(&format!("{}.srt", i));
                            extract_subtitles(&find_source_file(input), *j, &subtitle_out)?;
                        }
                    }
                }
                subtitle_outputs.push((subtitle_out, subtitle.enabled, subtitle.forced));
            }
        }

        mux_video(
            &find_source_file(input),
            &video_out,
            &audio_outputs,
            &subtitle_outputs,
            output
                .sub_tracks
                .iter()
                .any(|track| matches!(track.source, TrackSource::FromVideo(_))),
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

fn absolute_path(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    let path = path.as_ref();

    let absolute_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()?.join(path)
    }
    .clean();

    Ok(absolute_path)
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
                if arg > 64 {
                    panic!("'grain' must be between 0 and 64, received {}", arg);
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

fn build_vpy_script(filename: &Path, input: &Path, output: &Output, skip_lossless: bool) {
    let mut script = BufWriter::new(File::create(&filename).unwrap());
    if skip_lossless {
        copy_and_modify_vpy_script(input, output, &mut script);
    } else {
        build_new_vpy_script(input, output, &mut script);
    }
}

fn build_new_vpy_script(input: &Path, output: &Output, script: &mut BufWriter<File>) {
    writeln!(script, "import vapoursynth as vs").unwrap();
    writeln!(script, "core = vs.core").unwrap();
    writeln!(
        script,
        "clip = core.lsmas.LWLibavSource(source=\"{}\")",
        escape_python_string(
            absolute_path(input.with_extension("lossless.mkv"))
                .unwrap()
                .to_str()
                .unwrap()
        )
    )
    .unwrap();

    write_filters(output, script, None);

    writeln!(script, "clip.set_output()").unwrap();
    script.flush().unwrap();
}

fn copy_and_modify_vpy_script(input: &Path, output: &Output, script: &mut BufWriter<File>) {
    let contents = read_to_string(input).unwrap();
    let mut output_pos = None;
    let mut output_var = None;
    for line in contents.lines() {
        if let Some(pos) = line.find(".set_output()") {
            assert!(pos > 0);
            output_pos = Some(contents.find(line).unwrap());
            output_var = Some(&line[0..pos]);
            break;
        }
    }
    match (output_pos, output_var) {
        (Some(pos), Some(var)) => {
            write!(script, "{}", &contents[..pos]).unwrap();
            write_filters(output, script, Some(var));
            writeln!(script).unwrap();
            write!(script, "{}", &contents[pos..]).unwrap();
            script.flush().unwrap();
        }
        _ => {
            panic!("Invalid input vapoursynth script, no `set_output()` found");
        }
    }
}

fn write_filters(output: &Output, script: &mut BufWriter<File>, clip: Option<&str>) {
    let clip = clip.unwrap_or("clip");

    // We downscale resolution first because it's more likely that
    // we would be going from 10 bit to 8 bit, rather than the other way.
    // So this gives the best quality.
    if let Some((w, h)) = output.video.resolution {
        writeln!(
            script,
            "{clip} = {clip}.resize.Spline36({w}, {h}, dither_type='error_diffusion')"
        )
        .unwrap();
    }
    if let Some(bd) = output.video.bit_depth {
        writeln!(script, "import vsutil").unwrap();
        writeln!(script, "{clip} = vsutil.depth({clip}, {bd})").unwrap();
    }
}
