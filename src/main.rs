use anyhow::{Result, anyhow, bail};
use clap::Parser;
use colored::*;
use dotenvy_macro::dotenv;
use itertools::Itertools;
use lexical_sort::natural_lexical_cmp;
use path_clean::PathClean;
use signal_hook::{
    consts::{SIGINT, SIGTERM},
    flag::register,
};
use size::Size;
use std::process::Child;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::{
    env,
    fmt::Write as FmtWrite,
    fs::{self, File, read_to_string},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    thread,
};
use walkdir::WalkDir;
use which::which;

use crate::cli::{ParsedFilter, Track, TrackSource, parse_filters};

use self::{input::*, output::*};

mod cli;
mod input;
mod output;

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
    /// - enc=str: Encoder to use [default: x264] [options: copy, x264, x265,
    ///   aom, rav1e]
    /// - q=#: QP or CRF [default: varies by encoder]
    /// - s=#: Speed/cpu-used [aom/rav1e only] [default: varies by encoder]
    /// - p=str: Encoder settings to use [default: film] [options: film, grain,
    ///   anime, animedetailed, animegrain, fast]
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
    /// - aenc=str: Audio encoder to use [default: copy] [options: copy, aac,
    ///   flac, opus]
    /// - ab=#: Audio bitrate per channel in Kb/sec [default: 96 for aac, 64 for
    ///   opus]
    /// - at=#-[e][f]: Audio tracks, pipe separated [default: 0, e=enabled,
    ///   f=forced]
    /// - an=1: Enable audio normalization. Be SURE you want this. [default: 0]
    ///
    /// Subtitle options:
    ///
    /// - st=#-[e][f]: Subtitle tracks, pipe separated [default: None,
    ///   e=enabled, f=forced]
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

    /// Comma-separated list of forced keyframes.
    #[clap(long)]
    pub force_keyframes: Option<String>,

    /// Do not verify the length of the video after encoding
    #[clap(long)]
    pub no_verify: bool,

    /// Do not copy audio delay to the output
    #[clap(long)]
    pub no_delay: bool,

    /// Instead of retrying failed encodes, exit immediately
    #[clap(long)]
    pub no_retry: bool,
}

fn main() {
    check_for_required_apps().unwrap();

    let sigterm = Arc::new(AtomicBool::new(false));
    register(SIGTERM, Arc::clone(&sigterm)).unwrap();
    register(SIGINT, Arc::clone(&sigterm)).unwrap();

    let args = InputArgs::parse();

    let input = Path::new(&args.input);
    assert!(input.exists(), "Input path does not exist");

    eprintln!("DID YOU INSTALL FONTS???");
    eprintln!();

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
                let filestem = e
                    .path()
                    .file_stem()
                    .expect("File should have a name")
                    .to_string_lossy();
                !(filestem.contains(".aom-q")
                    || filestem.contains(".rav1e-q")
                    || filestem.contains(".svt-q")
                    || filestem.contains(".x264-q")
                    || filestem.contains(".x265-q")
                    || filestem.ends_with(".copy"))
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
        let outputs = args.formats.as_ref().map_or_else(
            || vec![Output::default()],
            |formats| {
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
                                    which("x264")
                                        .map_err(|_| anyhow!("x264 not installed or not in PATH!"))
                                        .unwrap();
                                    // This is the default, do nothing
                                }
                                "x265" => {
                                    which("x265")
                                        .map_err(|_| anyhow!("x265 not installed or not in PATH!"))
                                        .unwrap();
                                    output.video.encoder = VideoEncoder::X265 {
                                        crf: 18,
                                        profile: Profile::Film,
                                        compat: false,
                                    }
                                }
                                "aom" => {
                                    which("aomenc")
                                        .map_err(|_| {
                                            anyhow!("aomenc not installed or not in PATH!")
                                        })
                                        .unwrap();
                                    output.video.encoder = VideoEncoder::Aom {
                                        crf: 16,
                                        speed: 4,
                                        profile: Profile::Film,
                                        grain: 0,
                                        compat: false,
                                    }
                                }
                                "rav1e" => {
                                    which("rav1e")
                                        .map_err(|_| anyhow!("rav1e not installed or not in PATH!"))
                                        .unwrap();
                                    output.video.encoder = VideoEncoder::Rav1e {
                                        crf: 40,
                                        speed: 5,
                                        profile: Profile::Film,
                                        grain: 0,
                                    }
                                }
                                "svt" => {
                                    which("SvtAv1EncApp")
                                        .map_err(|_| {
                                            anyhow!("SvtAv1EncApp not installed or not in PATH!")
                                        })
                                        .unwrap();
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
                                enc => panic!("Unrecognized encoder: {}", enc),
                            }
                        }
                        for filter in &filters {
                            apply_filter(filter, &mut output);
                        }
                        output
                    })
                    .collect()
            },
        );

        let result = process_file(
            &input,
            &outputs,
            args.output.as_deref(),
            args.keep_lossless,
            args.lossless_only,
            args.skip_lossless,
            &args.force_keyframes,
            !args.no_verify,
            args.no_delay,
            args.no_retry,
            Arc::clone(&sigterm),
        );
        if let Err(err) = result {
            eprintln!(
                "{} Failed processing file {}: {}",
                "[Error]".red().bold(),
                input
                    .file_name()
                    .expect("File should have a name")
                    .to_string_lossy()
                    .red(),
                err.to_string().red()
            );
        }
        eprintln!();
        if sigterm.load(Ordering::Relaxed) {
            return;
        }
    }
}

fn check_for_required_apps() -> Result<()> {
    which("mediainfo").map_err(|_| anyhow!("mediainfo not installed or not in PATH!"))?;
    which("mkvmerge").map_err(|_| anyhow!("mkvmerge not installed or not in PATH!"))?;
    which("vspipe").map_err(|_| anyhow!("vspipe not installed or not in PATH!"))?;
    which("ffmpeg").map_err(|_| anyhow!("ffmpeg not installed or not in PATH!"))?;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::fn_params_excessive_bools)]
fn process_file(
    input_vpy: &Path,
    outputs: &[Output],
    output_dir: Option<&str>,
    keep_lossless: bool,
    lossless_only: bool,
    mut skip_lossless: bool,
    force_keyframes: &Option<String>,
    verify_frame_count: bool,
    ignore_delay: bool,
    no_retry: bool,
    sigterm: Arc<AtomicBool>,
) -> Result<()> {
    let source_video = find_source_file(input_vpy);
    let mediainfo = get_video_mediainfo(&source_video)?;
    let colorimetry = get_video_colorimetry(input_vpy)?;
    eprintln!(
        "{} {} {}{}{}{}",
        "[Info]".blue().bold(),
        source_video
            .file_name()
            .expect("File should have a name")
            .to_string_lossy()
            .blue()
            .bold(),
        "(".blue(),
        Size::from_bytes(
            source_video
                .metadata()
                .expect("Unable to get source file metadata")
                .len()
        )
        .format()
        .to_string()
        .blue()
        .bold(),
        mediainfo
            .get("Stream size")
            .map_or_else(String::new, |stream_size| format!(
                "{}{}",
                " - Video stream: ".blue(),
                stream_size.blue().bold()
            )),
        ")".blue(),
    );
    if outputs
        .iter()
        .all(|output| matches!(output.video.encoder, VideoEncoder::Copy))
    {
        skip_lossless = true;
    }
    let mut lossless_filename = None;
    if !skip_lossless {
        eprintln!(
            "{} {} {} {}",
            "[Info]".blue().bold(),
            "Encoding".blue(),
            input_vpy
                .file_name()
                .expect("File should have a name")
                .to_string_lossy()
                .blue(),
            "lossless".blue()
        );
        let mut retry_count = 0;

        loop {
            if sigterm.load(Ordering::Relaxed) {
                bail!("Exited via Ctrl+C");
            }
            // I hate this lazy workaround,
            // but this is due to a heisenbug in Vapoursynth
            // due to some sort of race condition,
            // which causes crashes often enough to be annoying.
            //
            // Essentially, we retry the encode until it works.
            let dimensions = get_video_dimensions(input_vpy)?;
            let result = create_lossless(
                input_vpy,
                dimensions,
                verify_frame_count,
                Arc::clone(&sigterm),
                keep_lossless,
            );
            match result {
                Ok(lf) => {
                    lossless_filename = Some(lf);
                    break;
                }
                Err(e) => {
                    if no_retry || retry_count >= 3 {
                        bail!(
                            "{} {}: {}",
                            "[Error]".red().bold(),
                            "While encoding lossless".red(),
                            e
                        );
                    } else {
                        retry_count += 1;
                        eprintln!(
                            "{} {}: {}",
                            "[Error]".red().bold(),
                            "While encoding lossless".red(),
                            e
                        );
                    }
                }
            }
        }
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
        let video_suffix = build_video_suffix(output)?;
        let output_vpy = input_vpy.with_extension(format!("{}.vpy", video_suffix));
        eprintln!(
            "{} {} {}",
            "[Info]".blue().bold(),
            "Encoding".blue(),
            output_vpy
                .file_name()
                .expect("File should have a name")
                .to_string_lossy()
                .blue()
        );

        let video_out = output_vpy.with_extension("mkv");
        match output.video.encoder {
            VideoEncoder::Copy => {
                extract_video(&source_video, &video_out)?;
            }
            VideoEncoder::X264 {
                crf,
                profile,
                compat,
            } => {
                build_vpy_script(&output_vpy, input_vpy, output, skip_lossless);
                let dimensions = get_video_dimensions(&output_vpy)?;
                convert_video_x264(
                    &output_vpy,
                    &video_out,
                    crf,
                    profile,
                    compat,
                    dimensions,
                    force_keyframes,
                    &colorimetry,
                    Arc::clone(&sigterm),
                )?;
            }
            encoder => {
                let should_use_vpy = skip_lossless
                    || output.video.bit_depth.is_some()
                    || output.video.resolution.is_some();
                build_vpy_script(&output_vpy, input_vpy, output, skip_lossless);
                let dimensions = get_video_dimensions(&output_vpy)?;
                convert_video_av1an(
                    if should_use_vpy {
                        &output_vpy
                    } else {
                        lossless_filename.as_ref().unwrap()
                    },
                    &video_out,
                    encoder,
                    dimensions,
                    force_keyframes,
                    &colorimetry,
                )?;
            }
        };

        let mut audio_tracks = if output.audio_tracks.is_empty() {
            vec![Track {
                source: TrackSource::FromVideo(0),
                enabled: true,
                forced: false,
            }]
        } else {
            output.audio_tracks.clone()
        };
        let has_vpy_audio = vspipe_has_audio(input_vpy)?;
        if let Some(track) = has_vpy_audio {
            let audio_path = input_vpy.with_extension("vpy.flac");
            save_vpy_audio(input_vpy, track, &audio_path, Arc::clone(&sigterm))?;
            audio_tracks = vec![Track {
                source: TrackSource::External(audio_path),
                enabled: true,
                forced: false,
            }];
        }
        let mut audio_outputs = Vec::new();
        let mut audio_suffixes = Vec::new();
        for (i, audio_track) in audio_tracks.iter().enumerate() {
            let audio_suffix = format!(
                "{}-{}kbpc-at{}",
                output.audio.encoder, output.audio.kbps_per_channel, i
            );
            let audio_out = input_vpy.with_extension(format!("{}.mka", audio_suffix));
            convert_audio(
                input_vpy,
                &audio_out,
                output.audio.encoder,
                audio_track,
                output.audio.kbps_per_channel,
                output.audio.normalize,
            )?;
            audio_outputs.push((audio_out, audio_track.clone(), output.audio.encoder));
            audio_suffixes.push(audio_suffix);
        }
        let audio_suffix = audio_suffixes.join("-");
        let mut output_path = PathBuf::from(output_dir.unwrap_or(dotenv!("OUTPUT_PATH")));
        output_path.push(
            input_vpy
                .with_extension(format!(
                    "{}-{}.{}",
                    video_suffix, audio_suffix, output.video.output_ext
                ))
                .file_name()
                .expect("File should have a name"),
        );

        let mut subtitle_outputs = Vec::new();
        if !output.sub_tracks.is_empty() {
            for (i, subtitle) in output.sub_tracks.iter().enumerate() {
                let mut subtitle_out;
                match &subtitle.source {
                    TrackSource::External(path) => {
                        let ext = path
                            .extension()
                            .expect("Output file should have an extension")
                            .to_string_lossy();
                        subtitle_out = input_vpy.with_extension(format!("{}.{}", i, ext));
                        fs::copy(path, &subtitle_out)
                            .map_err(|e| anyhow!("Failed to copy subtitle track: {e}"))?;
                    }
                    TrackSource::FromVideo(j) => {
                        subtitle_out = input_vpy.with_extension(format!("{}.ass", i));
                        if extract_subtitles(&source_video, *j, &subtitle_out).is_err() {
                            subtitle_out = input_vpy.with_extension(format!("{}.srt", i));
                            extract_subtitles(&source_video, *j, &subtitle_out)
                                .map_err(|e| anyhow!("Failed to extract subtitle track: {e}"))?;
                        }
                    }
                }
                subtitle_outputs.push((subtitle_out, subtitle.enabled, subtitle.forced));
            }
        }

        let timestamps = fs::read_dir(
            absolute_path(input_vpy)?
                .parent()
                .expect("file has a parent dir"),
        )
        .map_err(|e| anyhow!("Failed to read script's parent directory: {e}"))?
        .filter_map(Result::ok)
        .find_map(|item| {
            let path = item.path();
            let file_name = path.file_name().unwrap().to_string_lossy();
            if file_name.starts_with(input_vpy.file_stem().unwrap().to_str().unwrap())
                && file_name.contains(".vfr.")
            {
                Some(path)
            } else {
                None
            }
        });

        mux_video(
            &source_video,
            &video_out,
            &audio_outputs,
            &subtitle_outputs,
            timestamps.as_deref(),
            output
                .sub_tracks
                .iter()
                .any(|track| matches!(track.source, TrackSource::FromVideo(_))),
            ignore_delay || has_vpy_audio.is_some(),
            &output_path,
        )
        .map_err(|e| anyhow!("Failed to mux video: {e}"))?;

        let _ = copy_extra_data(&source_video, &output_path)
            .map_err(|e| anyhow!("Failed to copy extra data: {e}"));

        eprintln!(
            "{} {} {}",
            "[Success]".green().bold(),
            "Finished encoding".green(),
            output_vpy
                .file_name()
                .expect("File should have a name")
                .to_string_lossy()
                .green()
        );
        eprintln!();
    }

    if !keep_lossless {
        let _ = fs::remove_file(input_vpy.with_extension("lossless.mkv"));
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
    input.replace('\\', r"\\").replace('\'', r"\'")
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
                VideoEncoder::Aom { ref mut crf, .. }
                | VideoEncoder::SvtAv1 { ref mut crf, .. } => {
                    *crf = arg;
                    (0, 63)
                }
                VideoEncoder::Rav1e { ref mut crf, .. } => {
                    *crf = arg;
                    (0, 255)
                }
                VideoEncoder::Copy => {
                    return;
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
            VideoEncoder::Aom { ref mut speed, .. }
            | VideoEncoder::Rav1e { ref mut speed, .. }
            | VideoEncoder::SvtAv1 { ref mut speed, .. } => {
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
            }
            | VideoEncoder::SvtAv1 {
                ref mut profile, ..
            } => {
                *profile = *arg;
            }
            VideoEncoder::Copy => (),
        },
        ParsedFilter::Grain(arg) => match output.video.encoder {
            VideoEncoder::Aom { ref mut grain, .. }
            | VideoEncoder::Rav1e { ref mut grain, .. }
            | VideoEncoder::SvtAv1 { ref mut grain, .. } => {
                let arg = *arg;
                if arg > 64 {
                    panic!("'grain' must be between 0 and 64, received {}", arg);
                }
                *grain = arg;
            }
            _ => (),
        },
        ParsedFilter::Compat(arg) => match output.video.encoder {
            VideoEncoder::X264 { ref mut compat, .. }
            | VideoEncoder::X265 { ref mut compat, .. }
            | VideoEncoder::Aom { ref mut compat, .. } => {
                *compat = *arg;
            }
            _ => (),
        },
        ParsedFilter::Extension(arg) => {
            output.video.output_ext = (*arg).to_string();
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

fn build_video_suffix(output: &Output) -> Result<String> {
    let mut codec_str = match output.video.encoder {
        VideoEncoder::Aom {
            crf,
            speed,
            profile,
            grain,
            compat,
        } => format!(
            "aom-q{}-s{}-{}-g{}{}",
            crf,
            speed,
            profile,
            grain,
            if compat { "-compat" } else { "" }
        ),
        VideoEncoder::Rav1e {
            crf,
            speed,
            profile,
            grain,
        } => format!("rav1e-q{}-s{}-{}-g{}", crf, speed, profile, grain),
        VideoEncoder::SvtAv1 {
            crf,
            speed,
            profile,
            grain,
        } => format!("svt-q{}-s{}-{}-g{}", crf, speed, profile, grain),
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
        } => format!(
            "x265-q{}-{}{}",
            crf,
            profile,
            if compat { "-compat" } else { "" }
        ),
        VideoEncoder::Copy => "copy".to_string(),
    };
    if let Some(res) = output.video.resolution {
        write!(codec_str, "-{}x{}", res.0, res.1)?;
    }
    if let Some(bd) = output.video.bit_depth {
        write!(codec_str, "-{}b", bd)?;
    }
    Ok(codec_str)
}

fn build_vpy_script(filename: &Path, input: &Path, output: &Output, skip_lossless: bool) {
    let mut script = BufWriter::new(File::create(filename).expect("Unable to write script file"));
    if skip_lossless {
        copy_and_modify_vpy_script(input, output, &mut script);
    } else {
        build_new_vpy_script(input, output, &mut script);
    }
}

fn build_new_vpy_script(input: &Path, output: &Output, script: &mut BufWriter<File>) {
    writeln!(script, "import vapoursynth as vs").unwrap();
    writeln!(script, "core = vs.core").unwrap();
    writeln!(script, "core.max_cache_size=1024").unwrap();
    writeln!(script, "core.num_threads=1").unwrap();
    writeln!(
        script,
        "clip = core.lsmas.LWLibavSource(source=\"{}\")",
        escape_python_string(
            &absolute_path(input.with_extension("lossless.mkv"))
                .expect("Should be able to get absolute filepath")
                .to_string_lossy()
        )
    )
    .unwrap();

    write_filters(output, script, None);

    writeln!(script, "clip.set_output()").unwrap();
    script.flush().expect("Unable to flush script data");
}

fn copy_and_modify_vpy_script(input: &Path, output: &Output, script: &mut BufWriter<File>) {
    let contents = read_to_string(input).expect("Unable to read input script");
    let mut output_pos = None;
    let mut output_var = None;
    for line in contents.lines() {
        if let Some(pos) = line
            .find(".set_output()")
            .or_else(|| line.find(".set_output(0)"))
        {
            assert!(pos > 0);
            output_pos = Some(
                contents
                    .find(line)
                    .expect("Input script does not have an output clip"),
            );
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
            script.flush().expect("Unable to flush contents of script");
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

fn monitor_for_sigterm(child: &Child, sigterm: Arc<AtomicBool>, child_is_done: Arc<AtomicBool>) {
    let child_pid = child.id();
    thread::spawn(move || {
        while !sigterm.load(Ordering::Relaxed) {
            if child_is_done.load(Ordering::Relaxed) {
                return;
            }
            thread::sleep(Duration::from_millis(250));
        }

        eprintln!("Received sigterm or sigint, killing child processes");
        // We have to do this manually, we can't use `child.kill()` because
        // we can't borrow `child` mutably here.
        #[cfg(unix)]
        {
            use nix::libc::SIGKILL;
            use nix::libc::pid_t;

            unsafe {
                let _ = nix::libc::kill(child_pid as pid_t, SIGKILL);
            }
        }

        #[cfg(windows)]
        {
            use winapi::um::handleapi::CloseHandle;
            use winapi::um::processthreadsapi::OpenProcess;
            use winapi::um::processthreadsapi::TerminateProcess;
            use winapi::um::winnt::PROCESS_TERMINATE;

            unsafe {
                let handle = OpenProcess(PROCESS_TERMINATE, 0, child_pid);
                if !handle.is_null() {
                    TerminateProcess(handle, 1);
                    CloseHandle(handle);
                }
            }
        }
    });
}
