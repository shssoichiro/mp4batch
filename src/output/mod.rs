mod audio;
mod video;

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

pub use self::{audio::*, video::*};

pub fn mux_video(input: &Path, encoder: Encoder) -> Result<(), String> {
    let mut output_path = PathBuf::from(dotenv!("OUTPUT_PATH"));
    output_path.push(input.with_extension("mkv").file_name().unwrap());

    let status = Command::new("ffmpeg")
        .arg("-loglevel")
        .arg("level+error")
        .arg("-stats")
        .arg("-i")
        .arg(match encoder {
            Encoder::Rav1e => input.with_extension("out.ivf"),
            Encoder::X264 | Encoder::Aom | Encoder::X265 => input.with_extension("out.mkv"),
        })
        .arg("-i")
        .arg(input.with_extension("out.mka"))
        .arg("-vcodec")
        .arg("copy")
        .arg("-acodec")
        .arg("copy")
        .arg("-strict")
        .arg("-2")
        .arg("-map_chapters")
        .arg("-1")
        .arg(output_path)
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("{}", e))?;
    if status.success() {
        Ok(())
    } else {
        Err("Failed to execute ffmpeg".to_owned())
    }
}

pub fn mux_video_direct(
    input: &Path,
    audio_track: AudioTrack,
    audio_codec: &str,
    audio_bitrate: u32,
) -> Result<(), String> {
    let mut output_path = PathBuf::from(dotenv!("OUTPUT_PATH"));
    output_path.push(input.with_extension("mkv").file_name().unwrap());

    let mut command = Command::new("ffmpeg");
    command
        .arg("-loglevel")
        .arg("level+error")
        .arg("-stats")
        .arg("-i")
        .arg(input);
    if let AudioTrack::External(ref path) = audio_track {
        command.arg("-i").arg(path);
    }
    command.arg("-vcodec").arg("copy").arg("-acodec");
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
        "flac" => {
            command.arg("flac");
        }
        _ => unreachable!(),
    };
    command
        .arg("-map")
        .arg("0:v:0")
        .arg("-map")
        .arg(match audio_track {
            AudioTrack::FromVideo(ref track) => format!("0:a:{}", track),
            AudioTrack::External(_) => "1:a:0".to_string(),
        })
        .arg("-map_chapters")
        .arg("-1")
        .arg(output_path);
    let status = command
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("{}", e))?;
    if status.success() {
        Ok(())
    } else {
        Err("Failed to execute ffmpeg".to_owned())
    }
}
