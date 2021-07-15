mod audio;
mod video;

use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub use self::{audio::*, video::*};

pub fn mux_video(input: &Path, encoder: Encoder, extension: &str) -> Result<(), String> {
    let mut output_path = PathBuf::from(dotenv!("OUTPUT_PATH"));
    output_path.push(input.with_extension(extension).file_name().unwrap());

    let status = if extension == "mkv" {
        Command::new("mkvmerge")
            .arg("-q")
            .arg("--ui-language")
            .arg("en_US")
            .arg("--output")
            .arg(&output_path)
            .arg("--language")
            .arg("0:und")
            .arg("(")
            .arg(match encoder {
                Encoder::X264 | Encoder::Aom | Encoder::Rav1e => input.with_extension("out.mkv"),
                Encoder::X265 => input.with_extension("out.265"),
            })
            .arg(")")
            .arg("--language")
            .arg("0:en")
            .arg("--track-name")
            .arg("0:Audio")
            .arg("--default-track")
            .arg("0:yes")
            .arg("(")
            .arg(input.with_extension("out.mka"))
            .arg(")")
            .arg("--title")
            .arg(output_path.file_stem().unwrap().to_string_lossy().as_ref())
            .arg("--track-order")
            .arg("0:0,1:0")
            .status()
            .map_err(|e| format!("{}", e))?
    } else {
        Command::new("ffmpeg")
            .arg("-hide_banner")
            .arg("-loglevel")
            .arg("level+error")
            .arg("-stats")
            .arg("-i")
            .arg(match encoder {
                Encoder::Rav1e => input.with_extension("out.ivf"),
                Encoder::X264 | Encoder::Aom => input.with_extension("out.mkv"),
                Encoder::X265 => input.with_extension("out.265"),
            })
            .arg("-i")
            .arg(input.with_extension("out.mka"))
            .arg("-vcodec")
            .arg("copy")
            .arg("-acodec")
            .arg("copy")
            .arg("-map")
            .arg("0:v:0")
            .arg("-map")
            .arg("1:a:0")
            .arg("-map_chapters")
            .arg("-1")
            .arg("-movflags")
            .arg("+faststart")
            .arg(output_path)
            .status()
            .map_err(|e| format!("{}", e))?
    };
    if status.success() {
        Ok(())
    } else {
        Err("Failed to execute mkvmerge".to_owned())
    }
}

pub fn mux_video_direct(
    input: &Path,
    audio_track: AudioTrack,
    audio_codec: &str,
    audio_bitrate: u32,
    extension: &str,
) -> Result<(), String> {
    let mut output_path = PathBuf::from(dotenv!("OUTPUT_PATH"));
    output_path.push(input.with_extension(extension).file_name().unwrap());

    let mut command = Command::new("ffmpeg");
    command
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("level+error")
        .arg("-stats")
        .arg("-i")
        .arg(input);
    if let AudioTrack::External(ref path, _) = audio_track {
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
            AudioTrack::External(_, ref track) => format!("1:a:{}", track),
        })
        .arg("-map_chapters")
        .arg("-1")
        .arg(output_path);
    let status = command.status().map_err(|e| format!("{}", e))?;
    if status.success() {
        Ok(())
    } else {
        Err("Failed to execute ffmpeg".to_owned())
    }
}
