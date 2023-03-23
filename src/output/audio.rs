use std::{
    fmt::Display,
    path::Path,
    process::{Command, Stdio},
};

use ansi_term::Colour::Green;
use anyhow::Result;

use crate::{
    find_source_file,
    parse::{Track, TrackSource},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioOutput {
    pub encoder: AudioEncoder,
    pub kbps_per_channel: u32,
}

impl Default for AudioOutput {
    fn default() -> Self {
        AudioOutput {
            encoder: AudioEncoder::Copy,
            kbps_per_channel: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioEncoder {
    Copy,
    Aac,
    Flac,
    Opus,
}

impl Display for AudioEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        write!(
            f,
            "{}",
            match self {
                AudioEncoder::Copy => "copy",
                AudioEncoder::Aac => "aac",
                AudioEncoder::Flac => "flac",
                AudioEncoder::Opus => "opus",
            }
        )
    }
}

impl AudioEncoder {
    pub const fn supported_encoders() -> &'static [&'static str] {
        &["copy", "aac", "flac", "opus"]
    }
}

pub fn convert_audio(
    input: &Path,
    output: &Path,
    audio_codec: AudioEncoder,
    audio_track: &Track,
    mut audio_bitrate: u32,
) -> Result<()> {
    if output.exists() {
        // TODO: Verify the audio output is complete
        return Ok(());
    }

    let mut command = Command::new("ffmpeg");
    command
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("level+error")
        .arg("-stats")
        .arg("-y")
        .arg("-i")
        .arg(match audio_track.source {
            TrackSource::FromVideo(_) => find_source_file(input),
            TrackSource::External(ref path) => path.clone(),
        })
        .arg("-acodec");
    match audio_codec {
        AudioEncoder::Copy => {
            command.arg("copy");
        }
        AudioEncoder::Aac => {
            if audio_bitrate == 0 {
                audio_bitrate = 96;
            }
            command
                .arg("libfdk_aac")
                .arg("-vbr")
                .arg(match audio_bitrate {
                    0..=31 => "1",
                    32..=43 => "2",
                    44..=59 => "3",
                    60..=83 => "4",
                    _ => "5",
                })
                .arg("-af")
                .arg("aformat=channel_layouts=7.1|5.1|stereo");
        }
        AudioEncoder::Opus => {
            if audio_bitrate == 0 {
                audio_bitrate = 64;
            }
            command
                .arg("libopus")
                .arg("-b:a")
                .arg(&format!(
                    "{}k",
                    audio_bitrate
                        * get_channel_count(
                            &match audio_track.source {
                                TrackSource::FromVideo(_) => find_source_file(input),
                                TrackSource::External(ref path) => path.clone(),
                            },
                            audio_track
                        )?
                ))
                .arg("-af")
                .arg("aformat=channel_layouts=7.1|5.1|stereo");
        }
        AudioEncoder::Flac => {
            command.arg("flac");
        }
    };
    command
        .arg("-map")
        .arg(format!(
            "0:a:{}",
            match audio_track.source {
                TrackSource::FromVideo(id) => id,
                TrackSource::External(_) => 0,
            }
        ))
        .arg("-map_chapters")
        .arg("-1")
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

pub fn save_vpy_audio(input: &Path, output: &Path) -> Result<()> {
    let filename = input
        .file_name()
        .expect("File should have a name")
        .to_string_lossy();
    let pipe = if filename.ends_with(".vpy") {
        Command::new("vspipe")
            .arg("-o")
            .arg("1")
            .arg("-c")
            .arg("wav")
            .arg(input)
            .arg("-")
            .stdout(Stdio::piped())
            .spawn()
            .expect("Unable to run vspipe, is it installed and in PATH?")
    } else {
        panic!("Unrecognized input type");
    };

    let mut command = Command::new("nice");
    let status = command
        .arg("ffmpeg")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("level+error")
        .arg("-stats")
        .arg("-y")
        .arg("-i")
        .arg("-")
        .arg("-acodec")
        .arg("flac")
        .arg("-compression_level")
        .arg("9")
        .arg(output)
        .stdin(pipe.stdout.expect("stdout should be writeable"))
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to execute ffmpeg: {}", e))?;
    if !status.success() {
        anyhow::bail!(
            "Failed to execute ffmpeg: Exited with code {:x}",
            status.code().unwrap_or(-1)
        );
    }

    eprintln!(
        "{} {}",
        Green.bold().paint("[Success]"),
        Green.paint("Finished extracting Vapoursynth audio"),
    );

    Ok(())
}

fn get_channel_count(path: &Path, audio_track: &Track) -> Result<u32> {
    let output = Command::new("ffprobe")
        .arg("-v")
        .arg("error")
        .arg("-select_streams")
        .arg(format!(
            "a:{}",
            match audio_track.source {
                TrackSource::FromVideo(id) => id,
                TrackSource::External(_) => 0,
            }
        ))
        .arg("-show_entries")
        .arg("stream=channels")
        .arg("-of")
        .arg("compact=p=0:nk=1")
        .arg(path.as_os_str())
        .output()
        .map_err(|e| {
            anyhow::anyhow!("Failed to run ffprobe on {}: {}", path.to_string_lossy(), e)
        })?;
    let output = String::from_utf8_lossy(&output.stdout)
        .lines()
        .find(|line| !line.is_empty())
        .ok_or_else(|| anyhow::anyhow!("No output from ffprobe"))?
        .to_string();
    Ok(output.parse()?)
}
