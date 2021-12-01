mod audio;
mod video;

use std::{path::Path, process::Command};

pub use self::{audio::*, video::*};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output {
    pub video: VideoOutput,
    pub audio: AudioOutput,
}

pub fn mux_video(video: &Path, audio: &Path, output: &Path) -> Result<(), String> {
    let extension = output.extension().unwrap().to_string_lossy();
    let status = if extension == "mkv" {
        Command::new("mkvmerge")
            .arg("-q")
            .arg("--ui-language")
            .arg("en_US")
            .arg("--output")
            .arg(&output)
            .arg("--language")
            .arg("0:und")
            .arg("(")
            .arg(video)
            .arg(")")
            .arg("--language")
            .arg("0:en")
            .arg("--track-name")
            .arg("0:Audio")
            .arg("--default-track")
            .arg("0:yes")
            .arg("(")
            .arg(audio)
            .arg(")")
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
            .arg(video)
            .arg("-i")
            .arg(audio)
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
            .arg(output)
            .status()
            .map_err(|e| format!("{}", e))?
    };
    if status.success() {
        Ok(())
    } else {
        Err("Failed to mux video".to_owned())
    }
}
