use anyhow::{Result, anyhow, bail};
use clap::Parser;
use colored::*;
use dotenvy_macro::dotenv;
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
use which::which;

use crate::cli::{Track, TrackSource};

use self::{input::*, output::*};

mod cli;
mod file_discovery;
mod input;
mod output;
mod output_configuration;
mod workflow;

#[derive(Parser, Debug)]
struct InputArgs {
    /// Sets the input directory or file
    pub input: PathBuf,

    /// Override the default output directory
    #[clap(short, long, value_name = "DIR")]
    pub output: Option<PathBuf>,

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

    #[command(subcommand)]
    pub subcommand: Option<Subcommand>,
}

#[derive(Debug, Clone, clap::Subcommand)]
enum Subcommand {
    /// Just make a lossless, then exit. Copies the audio to the output.
    Lossless,
}

fn main() -> Result<()> {
    check_for_required_apps()?;

    let sigterm = Arc::new(AtomicBool::new(false));
    register(SIGTERM, Arc::clone(&sigterm))?;
    register(SIGINT, Arc::clone(&sigterm))?;

    let args = InputArgs::parse();

    let input = &args.input;
    assert!(input.exists(), "Input path does not exist");

    match args.subcommand {
        Some(Subcommand::Lossless) => {
            if let Err(err) = workflow::run_processing_workflow(
                input,
                None,
                absolute_path(input)?.parent(),
                true,
                true,
                false,
                true,
                None,
                !args.no_verify,
                args.no_delay,
                args.no_retry,
                &sigterm,
            ) {
                eprintln!("{} Workflow failed: {}", "[Error]".red().bold(), err);
            }
        }
        None => {
            eprintln!("DID YOU INSTALL FONTS???");
            eprintln!();

            if let Err(err) = workflow::run_processing_workflow(
                input,
                args.formats.as_deref(),
                args.output.as_deref(),
                args.keep_lossless,
                args.lossless_only,
                args.skip_lossless,
                false,
                args.force_keyframes.as_deref(),
                !args.no_verify,
                args.no_delay,
                args.no_retry,
                &sigterm,
            ) {
                eprintln!("{} Workflow failed: {}", "[Error]".red().bold(), err);
            }
        }
    }

    Ok(())
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
    output_dir: Option<&Path>,
    keep_lossless: bool,
    lossless_only: bool,
    mut skip_lossless: bool,
    copy_audio_to_lossless: bool,
    force_keyframes: Option<&str>,
    verify_frame_count: bool,
    ignore_delay: bool,
    no_retry: bool,
    sigterm: &Arc<AtomicBool>,
) -> Result<()> {
    let source_video = find_source_file(input_vpy);
    let mediainfo = get_video_mediainfo(&source_video)?;
    let colorimetry = Colorimetry::from_path(input_vpy)?;
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
                colorimetry,
                verify_frame_count,
                sigterm,
                keep_lossless,
                copy_audio_to_lossless.then_some(source_video.as_path()),
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
                    colorimetry,
                    sigterm,
                )?;
            }
            encoder => {
                let should_use_vpy = skip_lossless
                    || output.video.bit_depth.is_some()
                    || output.video.resolution.is_some();

                let dimensions = if should_use_vpy {
                    build_vpy_script(&output_vpy, input_vpy, output, skip_lossless);
                    get_video_dimensions(&output_vpy)?
                } else {
                    get_video_dimensions(
                        lossless_filename
                            .as_ref()
                            .expect("lossless filename is set"),
                    )?
                };
                convert_video_av1an(
                    if should_use_vpy {
                        &output_vpy
                    } else {
                        lossless_filename
                            .as_ref()
                            .expect("lossless filename is set")
                    },
                    &video_out,
                    encoder,
                    dimensions,
                    force_keyframes,
                    colorimetry,
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
            save_vpy_audio(input_vpy, track, &audio_path, sigterm)?;
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
        let output_path = output_dir
            .map_or_else(|| PathBuf::from(dotenv!("OUTPUT_PATH")), |d| d.to_owned())
            .join(
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
            let file_name = path.file_name().expect("has file name").to_string_lossy();
            (file_name.starts_with(
                input_vpy
                    .file_stem()
                    .expect("has file stem")
                    .to_string_lossy()
                    .as_ref(),
            ) && (
                // Wobbly naming
                file_name.contains(".vfr.") ||
                    // gmkvextractgui naming
                    file_name.ends_with(".tc.txt")
            ))
            .then_some(path)
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

#[expect(
    clippy::string_slice,
    reason = "`find` returns a byte index that we know is on a character boundary"
)]
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

            output_var = Some(&line[..pos]);
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

            // SAFETY: Only works on unix, validated by cfg
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
