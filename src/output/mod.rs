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
use crate::{
    cli::{Track, TrackSource},
    find_source_file,
    get_audio_delay_ms,
};

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
    audios: &[(PathBuf, Track)],
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
    if extension == "mkv" {
        let mut track_order = vec!["0:0".to_string()];
        let mut inputs_read = 1;
        let mut command = Command::new("mkvmerge");
        command
            .arg("--ui-language")
            .arg("en")
            .arg("--output")
            .arg(output)
            .arg("--no-audio")
            .arg("--no-subtitles")
            .arg("--no-attachments")
            .arg("--no-chapters")
            .arg("--language")
            .arg("0:en")
            .arg("(")
            .arg(video)
            .arg(")");
        if !audios.is_empty() {
            for audio in audios {
                let audio_delay = get_audio_delay_ms(
                    &match audio.1.source {
                        TrackSource::FromVideo(_) => find_source_file(input),
                        TrackSource::External(ref path) => path.clone(),
                    },
                    match audio.1.source {
                        TrackSource::FromVideo(id) => id as usize,
                        TrackSource::External(_) => 0,
                    },
                )?;

                command
                    .arg("--audio-tracks")
                    .arg("1")
                    .arg("--no-audio")
                    .arg("--no-subtitles")
                    .arg("--no-attachments")
                    .arg("--no-chapters");
                if audio_delay != 0 {
                    command.arg("--sync").arg(&format!("{}:{}", 0, audio_delay));
                }
                command
                    .arg("--language")
                    .arg("0:und")
                    .arg("--track-enabled-flag")
                    .arg(format!("0:{}", if audio.1.enabled { "yes" } else { "no" }))
                    .arg("--forced-display-flag")
                    .arg(format!("0:{}", if audio.1.forced { "yes" } else { "no" }))
                    .arg("(")
                    .arg(&audio.0)
                    .arg(")");
                track_order.push(format!("{}:0", inputs_read));
                inputs_read += 1;
            }
        }
        if !subtitles.is_empty() {
            for subtitle in subtitles {
                command
                    .arg("--language")
                    .arg("0:en")
                    .arg("--sub-charset")
                    .arg("0:UTF-8")
                    .arg("--track-enabled-flag")
                    .arg(format!("0:{}", if subtitle.1 { "yes" } else { "no" }))
                    .arg("--forced-display-flag")
                    .arg(format!("0:{}", if subtitle.2 { "yes" } else { "no" }))
                    .arg("(")
                    .arg(&subtitle.0)
                    .arg(")");
                track_order.push(format!("{}:0", inputs_read));
                inputs_read += 1;
            }
        }
        if copy_fonts {
            todo!("copy fonts not currently implemented for mkv");
        }
        command.arg("--track-order").arg(track_order.join(","));

        let status = command.arg(output).status()?;
        if status.success() {
            Ok(())
        } else {
            anyhow::bail!("Failed to mux video");
        }
    } else {
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
            if audio.1.forced {
                command.arg(&format!("-disposition:a:{}", j)).arg("forced");
            } else if audio.1.enabled {
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
