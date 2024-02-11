mod audio;
mod video;

use std::{
    borrow::Cow,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use ansi_term::Colour::Yellow;
use anyhow::Result;

pub use self::{audio::*, video::*};
use crate::cli::Track;

#[derive(Debug, Clone, Default)]
pub struct Output {
    pub video: VideoOutput,
    pub audio: AudioOutput,
    pub audio_tracks: Vec<Track>,
    pub sub_tracks: Vec<Track>,
}

pub fn mux_video(
    input: &Path,
    video: &Path,
    audios: &[(PathBuf, bool, bool)],
    subtitles: &[(PathBuf, bool, bool)],
    copy_fonts: bool,
    output: &Path,
) -> Result<()> {
    let mut extension = output
        .extension()
        .expect("Video should have extension")
        .to_string_lossy();

    if extension != "mkv" && !subtitles.is_empty() {
        eprintln!(
            "{} {}",
            Yellow.bold().paint("[Warning]"),
            Yellow.paint("Subtitles present, forcing mkv"),
        );
        extension = Cow::Borrowed("mkv");
    }
    let mut command = Command::new("ffmpeg");
    command
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("level+error")
        .arg("-stats")
        .arg("-i")
        .arg(video);
    for audio in audios {
        command.arg("-i").arg(&audio.0);
    }
    for subtitle in subtitles {
        command.arg("-i").arg(&subtitle.0);
    }
    if copy_fonts {
        command.arg("-i").arg(input);
    }
    command
        .arg("-vcodec")
        .arg("copy")
        .arg("-acodec")
        .arg("copy");
    if !subtitles.is_empty() {
        command.arg("-c:s").arg("copy");
    }
    command.arg("-map").arg("0:v:0");
    let mut i = 1;
    for (j, audio) in audios.iter().enumerate() {
        command.arg("-map").arg(&format!("{}:a:0", i));
        if audio.2 {
            command.arg(&format!("-disposition:a:{}", j)).arg("forced");
        } else if audio.1 {
            command.arg(&format!("-disposition:a:{}", j)).arg("default");
        }
        i += 1;
    }
    for (j, subtitle) in subtitles.iter().enumerate() {
        command.arg("-map").arg(&format!("{}:s:0", i));
        if subtitle.2 {
            command.arg(&format!("-disposition:s:{}", j)).arg("forced");
        } else if subtitle.1 {
            command.arg(&format!("-disposition:s:{}", j)).arg("default");
        }
        i += 1;
    }
    if copy_fonts {
        command
            .arg("-map")
            .arg(&format!("{}:t?", 1 + audios.len() + subtitles.len()));
    } else {
        let fonts_dir = input
            .parent()
            .expect("File should have parent dir")
            .join("fonts");
        if fonts_dir.is_dir() {
            let fonts = fonts_dir
                .read_dir()
                .expect("Unable to read directory contents");
            let mut i = 0;
            for font in fonts {
                let font = font.expect("Invalid directory entry");
                let mimetype = match font
                    .path()
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map_or_else(String::new, |ext| ext.to_lowercase())
                    .as_str()
                {
                    "ttf" => "font/ttf",
                    "otf" => "font/otf",
                    "eot" => "application/vnd.ms-fontobject",
                    "woff" => "font/woff",
                    "woff2" => "font/woff2",
                    _ => {
                        eprintln!(
                            "{} {}",
                            Yellow.bold().paint("[Warning]"),
                            Yellow.paint(&format!(
                                "Attachment with unrecognized extension skipped: {}",
                                font.path().to_string_lossy()
                            )),
                        );
                        continue;
                    }
                };
                command
                    .arg("-attach")
                    .arg(font.path())
                    .arg(&format!("-metadata:s:t:{}", i))
                    .arg(&format!("mimetype={}", mimetype));
                i += 1;
            }
        }
    }
    if extension == "mp4" {
        command.arg("-movflags").arg("+faststart");
    }

    let status = command.arg(output).status()?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("Failed to mux video");
    }
}

pub fn extract_subtitles(input: &Path, track: u8, output: &Path) -> Result<()> {
    let mut command = Command::new("ffmpeg");
    command
        .stderr(Stdio::null())
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("level+error")
        .arg("-stats")
        .arg("-y")
        .arg("-i")
        .arg(input)
        .arg("-c:s")
        .arg("copy")
        .arg("-map")
        .arg(&format!("0:s:{}", track))
        .arg(output);
    let status = command.arg(output).status()?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("Failed to extract subtitles");
    }
}
