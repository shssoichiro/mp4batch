use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub enum AudioTrack {
    FromVideo(u8),
    External(PathBuf),
}

pub fn find_external_audio(input: &Path, from_video: u8) -> AudioTrack {
    const TRY_EXTENSIONS: &[&str] = &[
        "flac", "wav", "aac", "ac3", "dts", "mka", "mkv", "avi", "mp4", "flv", "m2ts", "ts",
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
    AudioTrack::External(input_audio)
}

pub fn convert_audio(
    input: &Path,
    convert: bool,
    audio_track: AudioTrack,
    audio_bitrate: u32,
) -> Result<(), String> {
    let channels = get_audio_channel_count(input, audio_track.clone())?;
    let mut command = Command::new("ffmpeg");
    command
        .arg("-loglevel")
        .arg("level+error")
        .arg("-stats")
        .arg("-y")
        .arg("-i")
        .arg(match audio_track {
            AudioTrack::FromVideo(_) => input,
            AudioTrack::External(ref path) => &path,
        })
        .arg("-acodec")
        .arg(if convert { "libopus" } else { "copy" });
    if convert {
        command
            .arg("-b:a")
            .arg(&format!("{}k", audio_bitrate * channels))
            .arg("-af")
            .arg("aformat=channel_layouts=7.1|5.1|stereo");
    }
    command
        .arg("-map")
        .arg(format!(
            "0:a:{}",
            match audio_track {
                AudioTrack::FromVideo(ref track) => *track,
                AudioTrack::External(_) => 0,
            }
        ))
        .arg("-map_chapters")
        .arg("-1")
        .arg(input.with_extension("out.mka"));

    let status = command
        .status()
        .map_err(|e| format!("Failed to execute ffmpeg: {}", e))?;
    if status.success() {
        Ok(())
    } else {
        Err("Failed to execute ffmpeg".to_owned())
    }
}

pub fn get_audio_channel_count(input: &Path, audio_track: AudioTrack) -> Result<u32, String> {
    let output = Command::new("ffprobe")
        .arg("-i")
        .arg(match audio_track {
            AudioTrack::FromVideo(_) => input,
            AudioTrack::External(ref path) => &path,
        })
        .arg("-show_entries")
        .arg("stream=channels")
        .arg("-select_streams")
        .arg(format!(
            "a:{}",
            match audio_track {
                AudioTrack::FromVideo(track) => track,
                AudioTrack::External(_) => 0,
            }
        ))
        .arg("-of")
        .arg("compact=p=0:nk=1")
        .arg("-v")
        .arg("0")
        .output()
        .map_err(|e| format!("Failed to execute ffprobe: {}", e))?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap()
        .trim()
        .parse()
        .map_err(|e| format!("Failed to parse channel count: {}", e))
}
