use anyhow::Result;
use colored::*;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::{
    fmt::Display,
    num::NonZeroUsize,
    path::Path,
    process::{Command, Stdio},
    str::FromStr,
    sync::{Arc, atomic::AtomicBool},
    thread::available_parallelism,
};

use crate::{
    absolute_path,
    input::{Colorimetry, PixelFormat, VideoDimensions, get_video_frame_count},
    monitor_for_sigterm,
    output::{
        audio::has_audio,
        video::{
            aom::build_aom_args_string, rav1e::build_rav1e_args_string,
            svt_av1::build_svtav1_args_string, x264::build_x264_args_string,
            x265::build_x265_args_string,
        },
    },
};

pub use self::x264::convert_video_x264;

mod aom;
mod rav1e;
mod svt_av1;
mod x264;
mod x265;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoOutput {
    pub encoder: VideoEncoder,
    pub output_ext: String,
    pub bit_depth: Option<u8>,
    pub resolution: Option<(u32, u32)>,
}

impl Default for VideoOutput {
    fn default() -> Self {
        VideoOutput {
            encoder: VideoEncoder::X264 {
                crf: 18,
                profile: Profile::Film,
                compat: false,
            },
            output_ext: "mkv".to_string(),
            bit_depth: None,
            resolution: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Profile {
    #[default]
    Film,
    Grain,
    Anime,
    AnimeDetailed,
    AnimeGrain,
    Fast,
}

impl FromStr for Profile {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_ref() {
            "film" => Profile::Film,
            "anime" => Profile::Anime,
            "fast" => Profile::Fast,
            "grain" => Profile::Grain,
            "animedetailed" => Profile::AnimeDetailed,
            "animegrain" => Profile::AnimeGrain,
            _ => {
                return Err("Unrecognized profile");
            }
        })
    }
}

impl Display for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        write!(
            f,
            "{}",
            match self {
                Profile::Film => "film",
                Profile::Anime => "anime",
                Profile::Fast => "fast",
                Profile::Grain => "grain",
                Profile::AnimeDetailed => "animedetailed",
                Profile::AnimeGrain => "animegrain",
            }
        )
    }
}

impl Profile {
    pub const fn is_anime(self) -> bool {
        matches!(
            self,
            Profile::Anime | Profile::AnimeDetailed | Profile::AnimeGrain
        )
    }
}

pub fn extract_video(input: &Path, output: &Path) -> Result<()> {
    let mut command = Command::new("ffmpeg");
    command
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("level+error")
        .arg("-stats")
        .arg("-y")
        .arg("-i")
        .arg(input)
        .arg("-vcodec")
        .arg("copy")
        .arg("-map")
        .arg("0:v:0")
        .arg(output);

    let status = command
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to execute ffmpeg: {}", e))?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("Failed to execute ffmpeg");
    }
}

pub fn create_lossless(
    input: &Path,
    dimensions: VideoDimensions,
    colorimetry: Colorimetry,
    verify_frame_count: bool,
    sigterm: &Arc<AtomicBool>,
    slow: bool,
    copy_audio_from: Option<&Path>,
) -> Result<PathBuf> {
    let lossless_filename = input.with_extension("lossless.mkv");
    if lossless_filename.exists()
        && let Ok(lossless_frames) = get_video_frame_count(&lossless_filename)
    {
        // We use a fuzzy frame count check because *some cursed sources*
        // report a different frame count from the number of actual decodeable frames.
        let diff = (lossless_frames as i64 - dimensions.frames as i64).unsigned_abs() as u32;
        let allowance = dimensions.frames / 200;
        if !verify_frame_count || diff <= allowance {
            eprintln!(
                "{} {}",
                "[Success]".green().bold(),
                "Lossless already exists".green(),
            );
            return Ok(lossless_filename);
        }
    }

    let prim = colorimetry.get_primaries_encoder_string(VideoEncoderIdent::X264)?;
    let matrix = colorimetry.get_matrix_encoder_string(VideoEncoderIdent::X264)?;
    let transfer = colorimetry.get_transfer_encoder_string(VideoEncoderIdent::X264)?;
    let range = colorimetry.get_range_encoder_string(VideoEncoderIdent::X264)?;

    let filename = input
        .file_name()
        .expect("File should have a name")
        .to_string_lossy();
    let mut pipe = if filename.ends_with(".vpy") {
        Command::new("vspipe")
            .arg("-c")
            .arg("y4m")
            .arg(input)
            .arg("-")
            .arg("-o")
            .arg("0")
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to execute vspipe for lossless encoding: {}", e))?
    } else {
        panic!("Unrecognized input type");
    };
    let mut command = Command::new("ffmpeg");
    command
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("level+error")
        .arg("-stats")
        .arg("-y")
        .arg("-i")
        .arg("-");
    if let Some(source2) = copy_audio_from {
        // Only map audio if the source actually has audio tracks
        if has_audio(source2)? {
            command
                .arg("-i")
                .arg(source2)
                .arg("-map")
                .arg("0:v:0")
                .arg("-map")
                .arg("1:a:0")
                .arg("-acodec")
                .arg("copy");
        }
    }
    command
        .arg("-vcodec")
        .arg("libx264")
        .arg("-preset")
        // ultrafast -> superfast = 50% speed reduction, 15% size reduction
        // superfast -> veryfast = basically identical
        // veryfast -> fast = another 50% speed reduction, 2% size reduction
        // so we are just going to use "superfast" for the "slow" mode
        // as a note, ffv1 compresses with similar speed and efficiency to x264 fast,
        // but it decodes 70% slower.
        .arg(if slow { "superfast" } else { "ultrafast" })
        .arg("-qp")
        .arg("0")
        .arg("-x264-params")
        .arg(format!(
            "colorprim={prim}:colormatrix={matrix}:transfer={transfer}:input-range={range}:range={range}"
        ));

    command
        .arg(&lossless_filename)
        .stdin(pipe.stdout.take().expect("stdout should be writeable"))
        .stderr(Stdio::inherit());
    let status = command
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to execute ffmpeg: {}", e))?;
    let is_done = Arc::new(AtomicBool::new(false));
    monitor_for_sigterm(&pipe, Arc::clone(sigterm), Arc::clone(&is_done));
    pipe.wait()?;
    is_done.store(true, Ordering::Relaxed);
    if !status.success() {
        anyhow::bail!(
            "Failed to execute ffmpeg: Exited with code {:x}",
            status.code().unwrap_or(-1)
        );
    }

    if let Ok(lossless_frames) = get_video_frame_count(&lossless_filename)
        && verify_frame_count
    {
        // We use a fuzzy frame count check because *some cursed sources*
        // report a different frame count from the number of actual decodeable frames.
        let diff = (lossless_frames as i64 - dimensions.frames as i64).unsigned_abs() as u32;
        let allowance = dimensions.frames / 200;
        if diff > allowance {
            anyhow::bail!("Incomplete lossless encode");
        }
    }

    eprintln!(
        "{} {}",
        "[Success]".green().bold(),
        "Finished encoding lossless".green(),
    );

    Ok(lossless_filename)
}

pub fn convert_video_xav(
    input: &Path,
    output: &Path,
    encoder: VideoEncoder,
    dimensions: VideoDimensions,
    force_keyframes: Option<&str>,
    colorimetry: Colorimetry,
) -> Result<()> {
    if !dimensions.width.is_multiple_of(8) {
        eprintln!(
            "{} {} {} {}",
            "[Warning]".yellow().bold(),
            "Width".yellow(),
            dimensions.width.to_string().yellow(),
            "is not divisble by 8".yellow()
        );
    }
    if !dimensions.height.is_multiple_of(8) {
        eprintln!(
            "{} {} {} {}",
            "[Warning]".yellow().bold(),
            "Height".yellow(),
            dimensions.height.to_string().yellow(),
            "is not divisble by 8".yellow()
        );
    }

    if output.exists() && get_video_frame_count(output).unwrap_or(0) == dimensions.frames {
        eprintln!("Video output already exists, reusing");
        return Ok(());
    }

    // We may not actually split tiles at this point,
    // but we want to make sure we don't run out of memory
    let tiles = NonZeroUsize::new(
        if dimensions.height >= 2000 || (dimensions.height >= 1550 && dimensions.width >= 3600) {
            2
        } else {
            1
        } * if dimensions.width >= 2000 { 2 } else { 1 },
    )
    .expect("not 0");
    let thread_info = calculate_workers_and_threads(encoder, tiles);

    let mut command = Command::new("xav");
    command
        .arg("-p")
        .arg(&encoder.get_args_string(
            dimensions,
            colorimetry,
            thread_info.threads_per_worker,
            thread_info.cores,
            thread_info.workers,
            force_keyframes,
        )?)
        .arg("-w")
        .arg(thread_info.workers.to_string())
        .arg("-r");
    if let VideoEncoder::Aom { grain, .. }
    | VideoEncoder::Rav1e { grain, .. }
    | VideoEncoder::SvtAv1 { grain, .. } = encoder
        && grain > 0
    {
        command.arg("-n").arg(grain.to_string());
    }
    command
        .arg(absolute_path(input).expect("Unable to get absolute path"))
        .arg(absolute_path(output).expect("Unable to get absolute path"));
    let status = command
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to execute av1an: {}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "Failed to execute av1an: Exited with code {:x}",
            status.code().unwrap_or(-1)
        ))
    }
}

pub fn convert_video_av1an(
    input: &Path,
    output: &Path,
    encoder: VideoEncoder,
    dimensions: VideoDimensions,
    force_keyframes: Option<&str>,
    colorimetry: Colorimetry,
) -> Result<()> {
    if !dimensions.width.is_multiple_of(8) {
        eprintln!(
            "{} {} {} {}",
            "[Warning]".yellow().bold(),
            "Width".yellow(),
            dimensions.width.to_string().yellow(),
            "is not divisble by 8".yellow()
        );
    }
    if !dimensions.height.is_multiple_of(8) {
        eprintln!(
            "{} {} {} {}",
            "[Warning]".yellow().bold(),
            "Height".yellow(),
            dimensions.height.to_string().yellow(),
            "is not divisble by 8".yellow()
        );
    }

    if output.exists() && get_video_frame_count(output).unwrap_or(0) == dimensions.frames {
        eprintln!("Video output already exists, reusing");
        return Ok(());
    }

    let fps = (dimensions.fps.0 as f32 / dimensions.fps.1 as f32).round() as u32;
    // We may not actually split tiles at this point,
    // but we want to make sure we don't run out of memory
    let tiles = NonZeroUsize::new(
        if dimensions.height >= 2000 || (dimensions.height >= 1550 && dimensions.width >= 3600) {
            2
        } else {
            1
        } * if dimensions.width >= 2000 { 2 } else { 1 },
    )
    .expect("not 0");
    let thread_info = calculate_workers_and_threads(encoder, tiles);
    let mut command = Command::new("av1an");
    command
        .arg("-i")
        .arg(absolute_path(input).expect("Unable to get absolute path"))
        .arg("-e")
        .arg(encoder.get_av1an_name())
        .arg("-v")
        .arg(&encoder.get_args_string(
            dimensions,
            colorimetry,
            thread_info.threads_per_worker,
            thread_info.cores,
            thread_info.workers,
            force_keyframes,
        )?)
        .arg("--sc-method")
        .arg("standard")
        // Should be safe since our inputs are always lossless x264 with no open-gop
        .arg("--chunk-method")
        .arg("ffms2")
        .arg("-x")
        .arg(
            match encoder {
                VideoEncoder::Aom { profile, .. }
                | VideoEncoder::Rav1e { profile, .. }
                | VideoEncoder::SvtAv1 { profile, .. }
                | VideoEncoder::X264 { profile, .. }
                | VideoEncoder::X265 { profile, .. } => {
                    if profile.is_anime() {
                        fps * 15
                    } else {
                        fps * 10
                    }
                }
                VideoEncoder::Copy => unreachable!(),
            }
            .to_string(),
        )
        .arg("--min-scene-len")
        .arg(
            match encoder {
                VideoEncoder::Aom { profile, .. }
                | VideoEncoder::Rav1e { profile, .. }
                | VideoEncoder::SvtAv1 { profile, .. }
                | VideoEncoder::X264 { profile, .. }
                | VideoEncoder::X265 { profile, .. } => {
                    if profile.is_anime() {
                        fps / 2
                    } else {
                        fps
                    }
                }
                VideoEncoder::Copy => unreachable!(),
            }
            .to_string(),
        )
        .arg("-w")
        .arg(thread_info.workers.to_string())
        .arg("--pix-format")
        .arg(match (dimensions.bit_depth, dimensions.pixel_format) {
            (8, PixelFormat::Yuv420) => "yuv420p".to_string(),
            (8, PixelFormat::Yuv422) => "yuv422p".to_string(),
            (8, PixelFormat::Yuv444) => "yuv444p".to_string(),
            (bd, PixelFormat::Yuv420) => format!("yuv420p{}le", bd),
            (bd, PixelFormat::Yuv422) => format!("yuv422p{}le", bd),
            (bd, PixelFormat::Yuv444) => format!("yuv444p{}le", bd),
        })
        .arg("-r")
        .arg("-o")
        .arg(absolute_path(output).expect("Unable to get absolute path"));
    if let Some(force_keyframes) = force_keyframes {
        command.arg("--force-keyframes").arg(force_keyframes);
    }
    if dimensions.height > 1080 {
        command.arg("--sc-downscale-height").arg("1080");
    }
    if encoder.uses_av1an_thread_pinning() {
        command
            .arg("--set-thread-affinity")
            .arg((thread_info.cores.get() / thread_info.workers).to_string());
    }
    if let VideoEncoder::Aom { grain, .. }
    | VideoEncoder::Rav1e { grain, .. }
    | VideoEncoder::SvtAv1 { grain, .. } = encoder
        && grain > 0
    {
        command
            .arg("--photon-noise")
            .arg(grain.to_string())
            .arg("--chroma-noise");
    }
    if let VideoEncoder::X265 { .. } = encoder {
        command.arg("--concat").arg("mkvmerge");
    }
    let status = command
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to execute av1an: {}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "Failed to execute av1an: Exited with code {:x}",
            status.code().unwrap_or(-1)
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoEncoder {
    Copy,
    Aom {
        crf: i16,
        speed: u8,
        profile: Profile,
        grain: u8,
        compat: bool,
    },
    Rav1e {
        crf: i16,
        speed: u8,
        profile: Profile,
        grain: u8,
    },
    SvtAv1 {
        crf: i16,
        speed: u8,
        profile: Profile,
        grain: u8,
    },
    X264 {
        crf: i16,
        profile: Profile,
        compat: bool,
    },
    X265 {
        crf: i16,
        profile: Profile,
        compat: bool,
    },
}

impl VideoEncoder {
    pub const fn supported_encoders() -> &'static [&'static str] {
        &["aom", "rav1e", "svt", "x264", "x265", "copy"]
    }

    pub const fn get_av1an_name(&self) -> &str {
        match self {
            VideoEncoder::Copy => "copy",
            VideoEncoder::Aom { .. } => "aom",
            VideoEncoder::Rav1e { .. } => "rav1e",
            VideoEncoder::SvtAv1 { .. } => "svt-av1",
            VideoEncoder::X264 { .. } => "x264",
            VideoEncoder::X265 { .. } => "x265",
        }
    }

    pub fn get_args_string(
        self,
        dimensions: VideoDimensions,
        colorimetry: Colorimetry,
        computed_threads: NonZeroUsize,
        cores: NonZeroUsize,
        workers: NonZeroUsize,
        force_keyframes: Option<&str>,
    ) -> anyhow::Result<String> {
        Ok(match self {
            VideoEncoder::Aom {
                crf,
                speed,
                profile,
                ..
            } => build_aom_args_string(
                crf,
                speed,
                dimensions,
                profile,
                colorimetry,
                computed_threads,
            )?,
            VideoEncoder::Rav1e { crf, speed, .. } => {
                build_rav1e_args_string(crf, speed, dimensions, colorimetry)?
            }
            VideoEncoder::SvtAv1 {
                crf,
                speed,
                profile,
                ..
            } => build_svtav1_args_string(
                crf,
                speed,
                cores.get() / workers.get(),
                dimensions,
                profile,
                colorimetry,
            )?,
            VideoEncoder::X264 {
                crf,
                profile,
                compat,
            } => build_x264_args_string(
                crf,
                dimensions,
                profile,
                compat,
                force_keyframes,
                colorimetry,
            )?,
            VideoEncoder::X265 {
                crf,
                profile,
                compat,
                ..
            } => build_x265_args_string(
                crf,
                dimensions,
                profile,
                compat,
                colorimetry,
                computed_threads,
            )?,
            VideoEncoder::Copy => unreachable!(),
        })
    }

    pub const fn uses_av1an_thread_pinning(self) -> bool {
        matches!(
            self,
            VideoEncoder::Aom { .. } | VideoEncoder::SvtAv1 { .. } | VideoEncoder::Rav1e { .. }
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoEncoderIdent {
    Copy,
    Aom,
    Rav1e,
    SvtAv1,
    X264,
    X265,
}

impl From<VideoEncoder> for VideoEncoderIdent {
    fn from(value: VideoEncoder) -> Self {
        match value {
            VideoEncoder::Copy => Self::Copy,
            VideoEncoder::Aom { .. } => Self::Aom,
            VideoEncoder::Rav1e { .. } => Self::Rav1e,
            VideoEncoder::SvtAv1 { .. } => Self::SvtAv1,
            VideoEncoder::X264 { .. } => Self::X264,
            VideoEncoder::X265 { .. } => Self::X265,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ThreadingCalcs {
    cores: NonZeroUsize,
    workers: NonZeroUsize,
    threads_per_worker: NonZeroUsize,
}

fn calculate_workers_and_threads(encoder: VideoEncoder, tiles: NonZeroUsize) -> ThreadingCalcs {
    let cores = available_parallelism().expect("Unable to get machine parallelism count");
    let workers = NonZeroUsize::new(match encoder {
        VideoEncoder::Aom { .. } | VideoEncoder::Rav1e { .. } | VideoEncoder::SvtAv1 { .. } => {
            std::cmp::max(cores.get() / tiles.get(), 1)
        }
        _ => (std::cmp::max(cores.get() / tiles.get(), 1) / 4).max(1),
    })
    .expect("value is at least 1")
    // Limit the number of workers, as memory bandwidth eventually becomes a limiter
    // SAFETY: non-zero constant
    .min(unsafe { NonZeroUsize::new_unchecked(16) });
    assert!(
        workers <= cores,
        "Worker count exceeded core count, this is a bug"
    );

    let threads_per_worker = NonZeroUsize::new(std::cmp::min(
        64,
        (cores.get() as f32 / workers.get() as f32 * 1.5).ceil() as usize + 2,
    ))
    .expect("value is at least 1");

    ThreadingCalcs {
        cores,
        workers,
        threads_per_worker,
    }
}
