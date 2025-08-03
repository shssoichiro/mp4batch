use anyhow::Result;
use colored::*;
use std::fs::File;
use std::io::Write;
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
    output::video::{
        aom::build_aom_args_string, rav1e::build_rav1e_args_string,
        svt_av1::build_svtav1_args_string, x264::build_x264_args_string,
        x265::build_x265_args_string,
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
    colorimetry: &Colorimetry,
    verify_frame_count: bool,
    sigterm: Arc<AtomicBool>,
    slow: bool,
    copy_audio_from: Option<&Path>,
) -> Result<PathBuf> {
    let lossless_filename = input.with_extension("lossless.mkv");
    if lossless_filename.exists() {
        if let Ok(lossless_frames) = get_video_frame_count(&lossless_filename) {
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
    }

    // Print the info once
    Command::new("vspipe")
        .arg("-i")
        .arg(input)
        .arg("-o")
        .arg("0")
        .arg("-")
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to execute vspipe -i prior to lossless: {}", e))?;

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

    let is_done = Arc::new(AtomicBool::new(false));
    monitor_for_sigterm(&pipe, Arc::clone(&sigterm), Arc::clone(&is_done));

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

    pipe.wait()?;
    is_done.store(true, Ordering::Relaxed);

    if !status.success() {
        anyhow::bail!(
            "Failed to execute ffmpeg: Exited with code {:x}",
            status.code().unwrap_or(-1)
        );
    }

    if let Ok(lossless_frames) = get_video_frame_count(&lossless_filename) {
        if verify_frame_count {
            // We use a fuzzy frame count check because *some cursed sources*
            // report a different frame count from the number of actual decodeable frames.
            let diff = (lossless_frames as i64 - dimensions.frames as i64).unsigned_abs() as u32;
            let allowance = dimensions.frames / 200;
            if diff > allowance {
                anyhow::bail!("Incomplete lossless encode");
            }
        }
    }

    eprintln!(
        "{} {}",
        "[Success]".green().bold(),
        "Finished encoding lossless".green(),
    );

    Ok(lossless_filename)
}

pub fn create_lossless_with_topaz_enhance(
    input: &Path,
    dimensions: VideoDimensions,
    colorimetry: &Colorimetry,
    verify_frame_count: bool,
    sigterm: Arc<AtomicBool>,
    slow: bool,
    copy_audio_from: Option<&Path>,
) -> Result<PathBuf> {
    let lossless_filename = input.with_extension("lossless.mkv");
    if lossless_filename.exists() {
        if let Ok(lossless_frames) = get_video_frame_count(&lossless_filename) {
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
    }

    // Print the info once
    Command::new("vspipe")
        .arg("-i")
        .arg(input)
        .arg("-o")
        .arg("0")
        .arg("-")
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to execute vspipe -i prior to lossless: {}", e))?;

    let prim = colorimetry.get_primaries_encoder_string(VideoEncoderIdent::X264)?;
    let matrix = colorimetry.get_matrix_encoder_string(VideoEncoderIdent::X264)?;
    let transfer = colorimetry.get_transfer_encoder_string(VideoEncoderIdent::X264)?;
    let range = colorimetry.get_range_encoder_string(VideoEncoderIdent::X264)?;

    let filename = input
        .file_name()
        .expect("File should have a name")
        .to_string_lossy();
    let mut pipe1 = if filename.ends_with(".vpy") {
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

    let (w, h) = compute_upscaled_dimensions(dimensions.width, dimensions.height);
    let mut pipe2 = Command::new("ffmpeg")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-i")
        .arg("-")
        .arg("-sws_flags")
        .arg("spline+accurate_rnd+full_chroma_int")
        .arg("-filter_complex")
        .arg(format!("tvai_up=model=prob-4:scale=0:w={w}:h={h}:preblur=0:noise=0:details=0:halo=0:blur=0:compression=0:estimate=8:blend=0.4:device=-2:vram=1:instances=1"))
        .arg("-level")
        .arg("3")
        .arg("-vcodec")
        .arg("libx264")
        .arg("-preset")
        .arg("ultrafast")
        .arg("-qp")
        .arg("0")
        .arg("-x264-params")
        .arg(format!(
            "colorprim={prim}:colormatrix={matrix}:transfer={transfer}:input-range={range}:range={range}"
        ))
        .arg("-fps_mode:v")
        .arg("passthrough")
        .arg("-movflags")
        .arg("frag_keyframe+empty_moov+delay_moov+use_metadata_tags+write_colr")
        .arg("-bf" )
        .arg("0")
        .arg("-")
        .stdin(pipe1.stdout.take().expect("stdout should be writeable"))
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("Failed to execute vspipe for lossless encoding: {}", e))?;

    let second_input_script = r#"
import vapoursynth as vs
core = vs.code

clip = core.ffms2.Source(source="-")

import soifunc

clip = soifunc.mc_dfttest(clip)
clip = soifunc.retinex_deband(clip, 24)
clip = vstools.finalize_clip(clip, clamp_tv_range=True)

clip.set_output(0)
"#;
    let second_input = input.with_file_name(format!(
        "{}-filter2",
        input.file_name().unwrap().to_string_lossy()
    ));
    {
        let mut file = File::create(&second_input).unwrap();
        file.write_all(second_input_script.as_bytes()).unwrap();
        file.flush().unwrap();
    }

    let mut pipe3 = Command::new("vspipe")
        .arg("-c")
        .arg("y4m")
        .arg(second_input)
        .arg("-")
        .arg("-o")
        .arg("0")
        .stdin(pipe2.stdout.take().expect("stdout should be writeable"))
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("Failed to execute vspipe for lossless encoding: {}", e))?;

    let is_done = Arc::new(AtomicBool::new(false));
    monitor_for_sigterm(&pipe1, Arc::clone(&sigterm), Arc::clone(&is_done));
    monitor_for_sigterm(&pipe2, Arc::clone(&sigterm), Arc::clone(&is_done));
    monitor_for_sigterm(&pipe3, Arc::clone(&sigterm), Arc::clone(&is_done));

    let mut command = Command::new("ffmpeg");
    command
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("level+error")
        .arg("-stats")
        .arg("-y")
        .arg("-i")
        .arg("-");
    if let Some(audio_source) = copy_audio_from {
        command
            .arg("-i")
            .arg(audio_source)
            .arg("-map")
            .arg("0:v:0")
            .arg("-map")
            .arg("1:a:0")
            .arg("-acodec")
            .arg("copy");
    }
    let status = command.arg("-vcodec")
        .arg("libx264")
        .arg("-preset")
        .arg(if slow { "superfast" } else { "ultrafast" })
        .arg("-qp")
        .arg("0")
        .arg("-x264-params")
        .arg(format!(
            "colorprim={prim}:colormatrix={matrix}:transfer={transfer}:input-range={range}:range={range}"
        ))
        .arg(&lossless_filename)
        .stdin(pipe3.stdout.take().expect("stdout should be writeable"))
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to execute ffmpeg: {}", e))?;

    pipe1.wait()?;
    pipe2.wait()?;
    pipe3.wait()?;
    is_done.store(true, Ordering::Relaxed);

    if !status.success() {
        anyhow::bail!(
            "Failed to execute ffmpeg: Exited with code {:x}",
            status.code().unwrap_or(-1)
        );
    }

    if let Ok(lossless_frames) = get_video_frame_count(&lossless_filename) {
        if verify_frame_count {
            // We use a fuzzy frame count check because *some cursed sources*
            // report a different frame count from the number of actual decodeable frames.
            let diff = (lossless_frames as i64 - dimensions.frames as i64).unsigned_abs() as u32;
            let allowance = dimensions.frames / 200;
            if diff > allowance {
                anyhow::bail!("Incomplete lossless encode");
            }
        }
    }

    eprintln!(
        "{} {}",
        "[Success]".green().bold(),
        "Finished encoding lossless".green(),
    );

    Ok(lossless_filename)
}

fn compute_upscaled_dimensions(input_w: u32, input_h: u32) -> (u32, u32) {
    let (target_max_w, target_max_h) = if input_w >= input_h {
        // Landscape: target 1920x1080 max
        (1920u32, 1080u32)
    } else {
        // Portrait: target 1080x1920 max
        (1080u32, 1920u32)
    };

    // Calculate scale factor to fit within target bounds (only scale up, never down)
    let scale_w = target_max_w as f64 / input_w as f64;
    let scale_h = target_max_h as f64 / input_h as f64;
    let scale_factor = scale_w.min(scale_h).max(1.0); // Never scale down

    // Apply scale factor
    let scaled_w = (input_w as f64 * scale_factor) as u32;
    let scaled_h = (input_h as f64 * scale_factor) as u32;

    // Round to nearest multiple of 4
    let round_to_multiple_of_4 = |n: u32| -> u32 { ((n + 2) / 4) * 4 };

    (
        round_to_multiple_of_4(scaled_w),
        round_to_multiple_of_4(scaled_h),
    )
}

pub fn convert_video_av1an(
    input: &Path,
    output: &Path,
    encoder: VideoEncoder,
    dimensions: VideoDimensions,
    force_keyframes: Option<&str>,
    colorimetry: &Colorimetry,
) -> Result<()> {
    if dimensions.width % 8 != 0 {
        eprintln!(
            "{} {} {} {}",
            "[Warning]".yellow().bold(),
            "Width".yellow(),
            dimensions.width.to_string().yellow(),
            "is not divisble by 8".yellow()
        );
    }
    if dimensions.height % 8 != 0 {
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
    let cores = available_parallelism().expect("Unable to get machine parallelism count");
    let workers = NonZeroUsize::new(match encoder {
        VideoEncoder::Aom { .. } | VideoEncoder::Rav1e { .. } | VideoEncoder::SvtAv1 { .. } => {
            std::cmp::max(cores.get() / tiles.get(), 1)
        }
        _ => (std::cmp::max(cores.get() / tiles.get(), 1) / 4).max(1),
    })
    .unwrap();
    assert!(
        workers <= cores,
        "Worker count exceeded core count, this is a bug"
    );

    let threads_per_worker = NonZeroUsize::new(std::cmp::min(
        64,
        (cores.get() as f32 / workers.get() as f32 * 1.5).ceil() as usize + 2,
    ))
    .unwrap();
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
            threads_per_worker,
            cores,
            workers,
            force_keyframes,
        )?)
        .arg("--sc-method")
        .arg("standard")
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
        .arg(workers.to_string())
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
            .arg((cores.get() / workers).to_string());
    }
    if let VideoEncoder::Aom { grain, .. }
    | VideoEncoder::Rav1e { grain, .. }
    | VideoEncoder::SvtAv1 { grain, .. } = encoder
    {
        if grain > 0 {
            command
                .arg("--photon-noise")
                .arg(grain.to_string())
                .arg("--chroma-noise");
        }
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
        colorimetry: &Colorimetry,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_upscaled_dimensions_landscape_unchanged() {
        // Input of 1920x1080 -> output of 1920x1080 (unchanged)
        assert_eq!(compute_upscaled_dimensions(1920, 1080), (1920, 1080));
    }

    #[test]
    fn test_compute_upscaled_dimensions_landscape_already_at_max() {
        // Input of 1920x800 -> output of 1920x800 (unchanged because max dimension is already 1920)
        assert_eq!(compute_upscaled_dimensions(1920, 800), (1920, 800));
    }

    #[test]
    fn test_compute_upscaled_dimensions_landscape_upscale() {
        // Input of 640x480 -> output of 1440x1080
        assert_eq!(compute_upscaled_dimensions(640, 480), (1440, 1080));
    }

    #[test]
    fn test_compute_upscaled_dimensions_portrait_upscale() {
        // Input of 720x1280 -> output of 1080x1920
        assert_eq!(compute_upscaled_dimensions(720, 1280), (1080, 1920));
    }

    #[test]
    fn test_compute_upscaled_dimensions_portrait_unchanged() {
        // Portrait already at max dimensions
        assert_eq!(compute_upscaled_dimensions(1080, 1920), (1080, 1920));
    }

    #[test]
    fn test_compute_upscaled_dimensions_portrait_partial_upscale() {
        // Portrait where one dimension is already at max
        assert_eq!(compute_upscaled_dimensions(1080, 1600), (1080, 1600));
    }

    #[test]
    fn test_compute_upscaled_dimensions_square() {
        // Square input should be treated as landscape (w >= h)
        assert_eq!(compute_upscaled_dimensions(800, 800), (1080, 1080));
    }

    #[test]
    fn test_compute_upscaled_dimensions_rounding_to_4() {
        // Test that dimensions are rounded to nearest multiple of 4
        // 640x360 scaled by 1920/640 = 3.0 gives 1920x1080 (already multiples of 4)
        assert_eq!(compute_upscaled_dimensions(640, 360), (1920, 1080));

        // Test case that would need rounding
        // 641x361 scaled by 1920/641 ≈ 2.996 gives ~1918x~1081, rounded to 1920x1080
        assert_eq!(compute_upscaled_dimensions(641, 361), (1916, 1080));
    }

    #[test]
    fn test_compute_upscaled_dimensions_very_small_input() {
        // Very small input should scale up significantly
        assert_eq!(compute_upscaled_dimensions(160, 120), (1440, 1080));
    }

    #[test]
    fn test_compute_upscaled_dimensions_extreme_aspect_ratios() {
        // Very wide landscape
        assert_eq!(compute_upscaled_dimensions(1920, 400), (1920, 400));
        assert_eq!(compute_upscaled_dimensions(960, 200), (1920, 400));

        // Very tall portrait
        assert_eq!(compute_upscaled_dimensions(540, 1920), (540, 1920));
        assert_eq!(compute_upscaled_dimensions(270, 960), (540, 1920));
    }
}
