mod audio;
mod video;

pub use self::audio::*;
pub use self::video::*;
use crate::cross_platform_command;
use std::path::{Path, PathBuf};
use std::process::Stdio;

pub fn mux_mp4(input: &Path) -> Result<(), String> {
    let mut output_path = PathBuf::from(dotenv!("OUTPUT_PATH"));
    output_path.push(input.with_extension("mp4").file_name().unwrap());

    let status = cross_platform_command(dotenv!("FFMPEG_PATH"))
        .arg("-i")
        .arg(input.with_extension("out.mkv"))
        .arg("-i")
        .arg(input.with_extension("m4a"))
        .arg("-vcodec")
        .arg("copy")
        .arg("-acodec")
        .arg("copy")
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

pub fn mux_mp4_direct(input: &Path, audio_track: u32) -> Result<(), String> {
    let mut output_path = PathBuf::from(dotenv!("OUTPUT_PATH"));
    output_path.push(input.with_extension("mp4").file_name().unwrap());

    let status = cross_platform_command(dotenv!("FFMPEG_PATH"))
        .arg("-i")
        .arg(input)
        .arg("-vcodec")
        .arg("copy")
        .arg("-acodec")
        .arg("aac")
        .arg("-q:a")
        .arg("1")
        .arg("-map")
        .arg("0:v:0")
        .arg("-map")
        .arg(format!("0:a:{}", audio_track))
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
