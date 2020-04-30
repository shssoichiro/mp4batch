use std::path::Path;
use std::process::Command;

pub fn convert_audio(input: &Path, convert: bool, audio_track: u8) -> Result<(), String> {
    const TRY_EXTENSIONS: &[&str] = &[
        "flac", "wav", "aac", "ac3", "dts", "mkv", "avi", "mp4", "flv", "m2ts",
    ];
    let mut i = 0;
    let mut input_audio = input.with_extension(TRY_EXTENSIONS[i]);
    while !input_audio.exists() {
        i += 1;
        if i >= TRY_EXTENSIONS.len() {
            return Err("No file found to read audio from".to_owned());
        }
        input_audio = input.with_extension(TRY_EXTENSIONS[i]);
    }

    let channels = get_audio_channel_count(&input_audio, audio_track)?;
    let mut command = Command::new("ffmpeg");
    command
        .arg("-y")
        .arg("-i")
        .arg(input_audio)
        .arg("-acodec")
        .arg(if convert { "libopus" } else { "copy" });
    if convert {
        command.arg("-b:a").arg(&format!("{}k", 80 * channels));
    }
    command
        .arg("-af")
        .arg("aformat=channel_layouts=7.1|5.1|stereo")
        .arg("-map")
        .arg(format!("0:a:{}", audio_track))
        .arg("-map_chapters")
        .arg("-1")
        .arg(input.with_extension("opus"));

    let status = command
        .status()
        .map_err(|e| format!("Failed to execute ffmpeg: {}", e))?;
    if status.success() {
        Ok(())
    } else {
        Err("Failed to execute ffmpeg".to_owned())
    }
}

pub fn get_audio_channel_count(input: &Path, audio_track: u8) -> Result<u32, String> {
    let output = Command::new("ffprobe")
        .arg("-i")
        .arg(input)
        .arg("-show_entries")
        .arg("stream=channels")
        .arg("-select_streams")
        .arg(format!("a:{}", audio_track))
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
