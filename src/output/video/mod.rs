use std::{
    fmt::Display,
    num::NonZeroUsize,
    path::Path,
    process::{Command, Stdio},
    str::FromStr,
    thread::available_parallelism,
};

use ansi_term::Colour::{Green, Yellow};
use anyhow::Result;

use crate::{
    absolute_path,
    input::{get_video_frame_count, Colorimetry, PixelFormat, VideoDimensions},
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Film,
    Grain,
    Anime,
    AnimeDetailed,
    AnimeGrain,
    Fast,
}

impl Default for Profile {
    fn default() -> Self {
        Profile::Film
    }
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
    verify_frame_count: bool,
) -> Result<()> {
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
                    Green.bold().paint("[Success]"),
                    Green.paint("Lossless already exists"),
                );
                return Ok(());
            }
        }
    }

    // Print the info once
    Command::new("vspipe")
        .arg("-i")
        .arg(input)
        .arg("-")
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to execute vspipe -i prior to lossless: {}", e))?;

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
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to execute vspipe for lossless encoding: {}", e))?
    } else {
        panic!("Unrecognized input type");
    };
    let mut command = Command::new("ffmpeg");
    let status = command
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("level+error")
        .arg("-stats")
        .arg("-y")
        .arg("-i")
        .arg("-")
        .arg("-vcodec")
        .arg("libx264")
        .arg("-preset")
        .arg("ultrafast")
        .arg("-qp")
        .arg("0")
        .arg(&lossless_filename)
        .stdin(pipe.stdout.take().expect("stdout should be writeable"))
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to execute ffmpeg: {}", e))?;
    pipe.wait()?;
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
        Green.bold().paint("[Success]"),
        Green.paint("Finished encoding lossless"),
    );

    Ok(())
}

pub fn convert_video_av1an(
    vpy_input: &Path,
    output: &Path,
    encoder: VideoEncoder,
    dimensions: VideoDimensions,
    force_keyframes: &Option<String>,
    colorimetry: &Colorimetry,
) -> Result<()> {
    if dimensions.width % 8 != 0 {
        eprintln!(
            "{} {} {} {}",
            Yellow.bold().paint("[Warning]"),
            Yellow.paint("Width"),
            Yellow.paint(dimensions.width.to_string()),
            Yellow.paint("is not divisble by 8")
        );
    }
    if dimensions.height % 8 != 0 {
        eprintln!(
            "{} {} {} {}",
            Yellow.bold().paint("[Warning]"),
            Yellow.paint("Height"),
            Yellow.paint(dimensions.height.to_string()),
            Yellow.paint("is not divisble by 8")
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
        .arg(absolute_path(vpy_input).expect("Unable to get absolute path"))
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
        force_keyframes: &Option<String>,
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
            ),
            VideoEncoder::Rav1e { crf, speed, .. } => {
                build_rav1e_args_string(crf, speed, dimensions, colorimetry)
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
            ),
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
            ),
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

pub fn copy_hdr_data(input: &Path, target: &Path) -> Result<()> {
    let status = Command::new("hdrcopier")
        .arg("copy")
        .arg("--chapters")
        .arg(input)
        .arg(target)
        .status()?;
    if !status.success() {
        anyhow::bail!("Error copying hdr data");
    }
    Ok(())
}
