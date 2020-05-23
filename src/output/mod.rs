mod audio;
mod video;

pub use self::audio::*;
pub use self::video::*;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::process::Stdio;

pub fn mux_mp4(input: &Path) -> Result<(), String> {
    let mut output_path = PathBuf::from(dotenv!("OUTPUT_PATH"));
    output_path.push(input.with_extension("mp4").file_name().unwrap());

    let audio_path = if input.with_extension("opus").is_file() {
        input.with_extension("opus")
    } else {
        input.with_extension("m4a")
    };
    let status = Command::new("ffmpeg")
        .arg("-i")
        .arg(input.with_extension("out.mkv"))
        .arg("-i")
        .arg(audio_path)
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

pub fn mux_mp4_direct(input: &Path, audio_track: u8, convert_audio: bool) -> Result<(), String> {
    let mut output_path = PathBuf::from(dotenv!("OUTPUT_PATH"));
    output_path.push(input.with_extension("mp4").file_name().unwrap());

    let channels = get_audio_channel_count(input, audio_track)?;
    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input).arg("-vcodec").arg("copy");
    if convert_audio {
        command
            .arg("-acodec")
            .arg("libopus")
            .arg("-strict")
            .arg("-2")
            .arg("-b:a")
            .arg(&format!("{}k", 80 * channels))
            .arg("-af")
            .arg("aformat=channel_layouts=7.1|5.1|stereo")
    } else {
        command.arg("-acodec").arg("copy")
    }
    .arg("-map")
    .arg("0:v:0")
    .arg("-map")
    .arg(format!("0:a:{}", audio_track))
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
