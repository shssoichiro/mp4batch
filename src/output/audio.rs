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
    pub normalize: bool,
}

impl Default for AudioOutput {
    fn default() -> Self {
        AudioOutput {
            encoder: AudioEncoder::Copy,
            kbps_per_channel: 0,
            normalize: false,
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
        write!(f, "{}", match self {
            AudioEncoder::Copy => "copy",
            AudioEncoder::Aac => "aac",
            AudioEncoder::Flac => "flac",
            AudioEncoder::Opus => "opus",
        })
    }
}

impl AudioEncoder {
    pub const fn supported_encoders() -> &'static [&'static str] {
        &["copy", "aac", "flac", "opus"]
    }
}

#[derive(Clone, Copy)]
struct FirstPassData {
    pub integrated: f32,
    pub true_peak: f32,
    pub lra: f32,
    pub threshold: f32,
    pub offset: f32,
}

pub fn convert_audio(
    input: &Path,
    output: &Path,
    audio_codec: AudioEncoder,
    audio_track: &Track,
    mut audio_bitrate: u32,
    normalize: bool,
) -> Result<()> {
    if output.exists() {
        // TODO: Verify the audio output is complete
        return Ok(());
    }

    let mut fp_data = None;
    if normalize {
        eprintln!("Normalizing audio");
        let result = Command::new("ffmpeg")
            .arg("-hide_banner")
            .arg("-y")
            .arg("-i")
            .arg(match audio_track.source {
                TrackSource::FromVideo(_) => find_source_file(input),
                TrackSource::External(ref path) => path.clone(),
            })
            .arg("-map")
            .arg(format!("0:a:{}", match audio_track.source {
                TrackSource::FromVideo(id) => id,
                TrackSource::External(_) => 0,
            }))
            .arg("-map_chapters")
            .arg("-1")
            .arg("-af")
            .arg("loudnorm=I=-16:dual_mono=true:TP=-1.5:LRA=11:print_format=summary")
            .arg("-f")
            .arg("null")
            .arg("-")
            .output()?;

        let stderr = String::from_utf8_lossy(&result.stderr);
        let norm_data = stderr
            .lines()
            .skip_while(|line| !line.starts_with("[Parsed_loudnorm_"))
            .skip(1)
            .collect::<Vec<_>>();
        fp_data = Some(FirstPassData {
            integrated: norm_data
                .iter()
                .find(|line| line.starts_with("Input Integrated:"))
                .unwrap()
                .split_whitespace()
                .nth(2)
                .unwrap()
                .parse()
                .unwrap(),
            true_peak: norm_data
                .iter()
                .find(|line| line.starts_with("Input True Peak:"))
                .unwrap()
                .split_whitespace()
                .nth(3)
                .unwrap()
                .parse()
                .unwrap(),
            lra: norm_data
                .iter()
                .find(|line| line.starts_with("Input LRA:"))
                .unwrap()
                .split_whitespace()
                .nth(2)
                .unwrap()
                .parse()
                .unwrap(),
            threshold: norm_data
                .iter()
                .find(|line| line.starts_with("Input Threshold:"))
                .unwrap()
                .split_whitespace()
                .nth(2)
                .unwrap()
                .parse()
                .unwrap(),
            offset: norm_data
                .iter()
                .find(|line| line.starts_with("Target Offset:"))
                .unwrap()
                .split_whitespace()
                .nth(2)
                .unwrap()
                .parse()
                .unwrap(),
        });
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
        .arg("-map")
        .arg(format!("0:a:{}", match audio_track.source {
            TrackSource::FromVideo(id) => id,
            TrackSource::External(_) => 0,
        }))
        .arg("-map_chapters")
        .arg("-1");
    if normalize {
        let params = fp_data.unwrap();
        command.arg("-af").arg(&format!(
            "loudnorm=I=-16:dual_mono=true:TP=-1.5:LRA=11:measured_I={:.1}:measured_TP={:.1}:\
             measured_LRA={:.1}:measured_thresh={:.1}:offset={:.1}:linear=true:\
             print_format=summary",
            params.integrated, params.true_peak, params.lra, params.threshold, params.offset
        ));
    }
    match audio_codec {
        AudioEncoder::Copy => {
            command.arg("-acodec").arg("copy");
        }
        AudioEncoder::Aac => {
            if audio_bitrate == 0 {
                audio_bitrate = 96;
            }
            command
                .arg("-acodec")
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
            let channels = get_channel_count(
                &match audio_track.source {
                    TrackSource::FromVideo(_) => find_source_file(input),
                    TrackSource::External(ref path) => path.clone(),
                },
                audio_track,
            )?;
            command
                .arg("-acodec")
                .arg("libopus")
                .arg("-b:a")
                .arg(&format!("{}k", audio_bitrate * channels))
                .arg("-af")
                .arg("aformat=channel_layouts=7.1|5.1|stereo")
                .arg("-mapping_family")
                .arg(if channels > 2 { "1" } else { "0" });
        }
        AudioEncoder::Flac => {
            command.arg("-acodec").arg("flac");
        }
    };
    command.arg(output);

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
        .arg(format!("a:{}", match audio_track.source {
            TrackSource::FromVideo(id) => id,
            TrackSource::External(_) => 0,
        }))
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
