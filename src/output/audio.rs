use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{anyhow, Result};

#[derive(Debug, Clone)]
pub enum AudioTrack {
    FromVideo(u8),
    External(PathBuf, u8),
}

pub fn find_external_audio(input: &Path, from_video: u8) -> AudioTrack {
    const TRY_EXTENSIONS: &[&str] = &[
        "flac", "wav", "aac", "ac3", "dts", "mka", "mkv", "avi", "mp4", "flv", "m2ts", "ts", "wmv",
    ];
    let mut i = 0;
    let mut input_audio = input.with_extension(TRY_EXTENSIONS[i]);
    while !input_audio.exists() {
        i += 1;
        if i >= TRY_EXTENSIONS.len() {
            return AudioTrack::FromVideo(from_video);
        }
        input_audio = input.with_extension(TRY_EXTENSIONS[i]);
    }
    AudioTrack::External(input_audio, from_video)
}

pub fn convert_audio(
    input: &Path,
    output: &Path,
    audio_codec: &str,
    audio_track: AudioTrack,
    audio_bitrate: u32,
) -> Result<(), String> {
    let mut command = Command::new("ffmpeg");
    command
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("level+error")
        .arg("-stats")
        .arg("-y")
        .arg("-i")
        .arg(match audio_track {
            AudioTrack::FromVideo(_) => input,
            AudioTrack::External(ref path, _) => path,
        })
        .arg("-acodec");
    match audio_codec {
        "copy" => {
            command.arg("copy");
        }
        "aac" => {
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
        "opus" => {
            command
                .arg("libopus")
                .arg("-b:a")
                .arg(&format!(
                    "{}k",
                    audio_bitrate
                        * get_channel_count(
                            match audio_track {
                                AudioTrack::FromVideo(_) => input,
                                AudioTrack::External(ref path, _) => path,
                            },
                            &audio_track
                        )
                        .map_err(|e| e.to_string())?
                ))
                .arg("-af")
                .arg("aformat=channel_layouts=7.1|5.1|stereo");
        }
        "flac" => {
            command.arg("flac");
        }
        _ => unreachable!(),
    };
    command
        .arg("-map")
        .arg(format!(
            "0:a:{}",
            match audio_track {
                AudioTrack::FromVideo(ref track) => *track,
                AudioTrack::External(_, ref track) => *track,
            }
        ))
        .arg("-map_chapters")
        .arg("-1")
        .arg(output);

    let status = command
        .status()
        .map_err(|e| format!("Failed to execute ffmpeg: {}", e))?;
    if status.success() {
        Ok(())
    } else {
        Err("Failed to execute ffmpeg".to_owned())
    }
}

fn get_channel_count(path: &Path, audio_track: &AudioTrack) -> Result<u32> {
    let output = Command::new("ffprobe")
        .arg("-v")
        .arg("error")
        .arg("-select_streams")
        .arg(format!(
            "a:{}",
            match audio_track {
                AudioTrack::FromVideo(ref track) => *track,
                AudioTrack::External(_, ref track) => *track,
            }
        ))
        .arg("-show_entries")
        .arg("stream=channels")
        .arg("-of")
        .arg("compact=p=0:nk=1")
        .arg(path.as_os_str())
        .output()
        .map_err(|e| anyhow!("Failed to run ffprobe on {}: {}", path.to_string_lossy(), e))?;
    let output = String::from_utf8_lossy(&output.stdout)
        .lines()
        .find(|line| !line.is_empty())
        .ok_or_else(|| anyhow!("No output from ffprobe"))?
        .to_string();
    Ok(output.parse()?)
}
