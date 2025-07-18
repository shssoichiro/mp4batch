use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::Result;

use crate::{
    cli::{Track, TrackSource},
    find_source_file, get_audio_delay_ms,
};

pub use self::{audio::*, hdr::*, video::*};

mod audio;
mod hdr;
mod video;

#[derive(Debug, Clone, Default)]
pub struct Output {
    pub video: VideoOutput,
    pub audio: AudioOutput,
    pub audio_tracks: Vec<Track>,
    pub sub_tracks: Vec<Track>,
}

#[allow(clippy::too_many_arguments)]
pub fn mux_video(
    input: &Path,
    video: &Path,
    audios: &[(PathBuf, Track, AudioEncoder)],
    subtitles: &[(PathBuf, bool, bool)],
    timestamps: Option<&Path>,
    copy_fonts: bool,
    ignore_delay: bool,
    output: &Path,
) -> Result<()> {
    let mut track_order = vec!["0:0".to_string()];
    let mut inputs_read = 1;

    let mut command = Command::new("mkvmerge");
    command
        .arg("--output")
        .arg(output)
        .arg("--no-audio")
        .arg("--no-subtitles")
        .arg("--no-attachments")
        .arg("--no-chapters");
    if let Some(timestamps) = timestamps {
        command
            .arg("--timestamps")
            .arg(format!("0:{}", timestamps.to_string_lossy()));
    }
    command
        .arg("--language")
        .arg("0:en")
        .arg("(")
        .arg(video)
        .arg(")");

    if !audios.is_empty() {
        for audio in audios {
            let audio_delay = if ignore_delay || audio.2 == AudioEncoder::Copy {
                // If we're copying, mkvtoolnix copies the sync automatically.
                0
            } else {
                // If we're reencoding the audio, then we need to manually apply the sync.
                // Note that mediainfo can give unparseable and wrong results for some formats
                // like PCM, so we just assume 0 for those.
                get_audio_delay_ms(
                    &match audio.1.source {
                        TrackSource::FromVideo(_) => find_source_file(input),
                        TrackSource::External(ref path) => path.clone(),
                    },
                    match audio.1.source {
                        TrackSource::FromVideo(id) => id as usize,
                        TrackSource::External(_) => 0,
                    },
                )
                .unwrap_or(0)
            };

            command
                .arg("--audio-tracks")
                .arg("0")
                .arg("--no-video")
                .arg("--no-subtitles")
                .arg("--no-attachments")
                .arg("--no-chapters");

            if audio_delay != 0 {
                command.arg("--sync").arg(format!("{}:{}", 0, audio_delay));
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
                .arg("--no-video")
                .arg("--no-audio")
                .arg("--no-attachments")
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
        eprintln!("WARNING: copy fonts not currently implemented for mkv");
    }
    command.arg("--track-order").arg(track_order.join(","));

    let status = command.status()?;
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
        .arg(format!("0:s:{}", track))
        .arg(output);
    let status = command.arg(output).status()?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("Failed to extract subtitles");
    }
}
