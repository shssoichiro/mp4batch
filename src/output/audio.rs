use crate::cross_platform_command;
use crate::input::read_file;
use std::path::Path;
use std::process::Command;
use std::process::Stdio;

pub fn convert_audio(input: &Path, convert: bool) -> Result<(), String> {
    let avs_contents = read_file(input)?;
    if !avs_contents.to_lowercase().contains("audiodub") {
        const TRY_EXTENSIONS: [&str; 9] = [
            "flac", "wav", "aac", "ac3", "dts", "mkv", "avi", "mp4", "flv",
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

        let channels = get_audio_channel_count(&input_audio)?;
        let mut command = cross_platform_command(dotenv!("FFMPEG_PATH"));
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
            .arg("-map")
            .arg("0:a:0")
            .arg("-map_chapters")
            .arg("-1")
            .arg(input.with_extension("opus"));

        let status = command
            .status()
            .map_err(|e| format!("Failed to execute ffmpeg: {}", e))?;
        return if status.success() {
            Ok(())
        } else {
            Err("Failed to execute ffmpeg".to_owned())
        };
    }

    // TODO: Use libopus here.
    // But this codepath might go away since Avisynth support
    // is dying.
    let wavi = cross_platform_command(dotenv!("WAVI_PATH"))
        .arg(if dotenv!("WAVI_PATH").starts_with("wine") {
            format!("Z:{}", input.canonicalize().unwrap().to_string_lossy())
        } else {
            input.to_string_lossy().to_string()
        })
        .arg("-")
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to execute wavi: {}", e))?;

    let status = cross_platform_command(dotenv!("FFMPEG_PATH"))
        .arg("-y")
        .arg("-i")
        .arg("-")
        .arg("-acodec")
        .arg("aac")
        .arg("-q:a")
        .arg("1")
        .arg("-map")
        .arg("0:a:0")
        .arg("-map_chapters")
        .arg("-1")
        .arg(input.with_extension("m4a"))
        .stdin(wavi.stdout.unwrap())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("Failed to execute ffmpeg: {}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err("Failed to execute ffmpeg".to_owned())
    }
}

pub fn get_audio_channel_count(input: &Path) -> Result<u32, String> {
    let output = Command::new("ffprobe")
        .arg("-i")
        .arg(input)
        .arg("-show_entries")
        .arg("stream=channels")
        .arg("-select_streams")
        .arg("a:0")
        .arg("-of")
        .arg("compact=p=0:nk=1")
        .arg("-v")
        .arg("0")
        .output()
        .map_err(|e| format!("Failed to execute ffprobe: {}", e))?;
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .map_err(|e| format!("Failed to parse channel count: {}", e))
}
