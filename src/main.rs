#[macro_use]
extern crate dotenv_codegen;

use std::env;
use std::fs::File;
use std::io::prelude::*;
use std::io::BufReader;
#[cfg(unix)]
use std::os::unix::io::{AsRawFd, FromRawFd};
#[cfg(windows)]
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::str::FromStr;

use clap::{App, Arg};
use lazy_static::lazy_static;
use regex::Regex;

#[derive(Debug, Clone, Copy)]
enum Profile {
    Film,
    Anime,
    OneTwenty,
}

impl FromStr for Profile {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_ref() {
            "film" => Profile::Film,
            "anime" => Profile::Anime,
            "120" => Profile::OneTwenty,
            _ => {
                return Err("Invalid profile given".to_owned());
            }
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct VideoDimensions {
    pub width: u32,
    pub height: u32,
    pub frames: u32,
}

fn main() {
    env::set_var("RUST_BACKTRACE", "1");
    let args = App::new("mp4batch")
        .arg(
            Arg::with_name("profile")
                .short("p")
                .long("profile")
                .value_name("VALUE")
                .help("Sets a custom profile (default: film, available: film, anime, 120)")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("crf")
                .short("c")
                .long("crf")
                .value_name("VALUE")
                .help("Sets a CRF value to use (default: 18)")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("direct")
                .short("d")
                .long("direct")
                .value_name("A_TRACK")
                .help("remux mkv to mp4; will convert audio streams to aac without touching video")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("keep-audio")
                .short("a")
                .long("keep-audio")
                .help("copy the audio without reencoding"),
        )
        .arg(
            Arg::with_name("skip-video")
                .long("skip-video")
                .help("assume the video has already been encoded (will use .264 files)"),
        )
        .arg(
            Arg::with_name("input")
                .help("Sets the input directory or file")
                .required(true)
                .index(1),
        )
        .get_matches();

    const INPUT_PATH_ERROR: &str = "No input path provided";
    const CRF_PARSE_ERROR: &str = "CRF must be a number between 0-51";

    let input = args.value_of("input").expect(INPUT_PATH_ERROR);
    let profile = Profile::from_str(args.value_of("profile").unwrap_or("film"))
        .expect("Invalid profile given");
    let crf = args
        .value_of("crf")
        .unwrap_or("18")
        .parse::<u8>()
        .expect(CRF_PARSE_ERROR);
    assert!(crf <= 51, CRF_PARSE_ERROR);

    let input = Path::new(input);
    assert!(input.exists(), "Input path does not exist");

    if args.is_present("direct") {
        let track: u32 = args
            .value_of("direct")
            .map(|t| t.parse().unwrap())
            .unwrap_or(0);
        if input.is_dir() {
            let dir_entries = input.read_dir().unwrap();
            for entry in dir_entries.map(|e| e.unwrap()).filter(|e| {
                e.path()
                    .extension()
                    .unwrap_or_default()
                    .to_str()
                    .unwrap_or_default()
                    == "mkv"
            }) {
                let result = process_direct(&entry.path(), track);
                if let Err(err) = result {
                    println!("{}", err);
                }
            }
        } else {
            assert_eq!(
                input
                    .extension()
                    .unwrap_or_default()
                    .to_str()
                    .unwrap_or_default(),
                "mkv",
                "Input file must be a matroska file"
            );
            process_direct(input, track).unwrap();
        }
        return;
    }

    if input.is_dir() {
        let dir_entries = input.read_dir().unwrap();
        for entry in dir_entries.map(|e| e.unwrap()).filter(|e| {
            e.path()
                .extension()
                .unwrap_or_default()
                .to_str()
                .unwrap_or_default()
                == "avs"
        }) {
            let result = process_file(
                &entry.path(),
                profile,
                crf,
                args.is_present("keep-audio"),
                args.is_present("skip-video"),
            );
            if let Err(err) = result {
                println!("{}", err);
            }
        }
    } else {
        process_file(
            input,
            profile,
            crf,
            args.is_present("keep-audio"),
            args.is_present("skip-video"),
        )
        .unwrap();
    }
}

fn process_file(
    input: &Path,
    profile: Profile,
    crf: u8,
    keep_audio: bool,
    skip_video: bool,
) -> Result<(), String> {
    let dims = get_video_dimensions(input)?;
    if !skip_video {
        convert_video(input, profile, crf, dims.height >= 576, dims.frames)?;
    }
    convert_audio(input, !keep_audio)?;
    mux_mp4(input)?;
    println!("Finished converting {}", input.to_string_lossy());
    Ok(())
}

fn process_direct(input: &Path, audio_track: u32) -> Result<(), String> {
    mux_mp4_direct(input, audio_track)?;
    println!("Finished converting {}", input.to_string_lossy());
    Ok(())
}

fn get_video_dimensions(input: &Path) -> Result<VideoDimensions, String> {
    let filename = input.file_name().unwrap().to_str().unwrap();
    if filename.ends_with(".avs") {
        get_video_dimensions_avs(input)
    } else if filename.ends_with(".vpy") {
        get_video_dimensions_vps(input)
    } else {
        panic!("Unrecognized input type");
    }
}

fn get_video_dimensions_avs(input: &Path) -> Result<VideoDimensions, String> {
    let command = cross_platform_command(dotenv!("AVS2YUV_PATH"))
        .arg(input)
        .arg("-o")
        .arg(if Path::new("/dev/null").exists() {
            "/dev/null"
        } else {
            "nul"
        })
        .arg("-frames")
        .arg("1")
        .output()
        .map_err(|e| format!("{}", e))?;
    let output = String::from_utf8_lossy(&command.stderr);

    lazy_static! {
        // Output e.g.: Tangled.Ever.After.2012.720p.BluRay.DD5.1.x264-EbP.avs: 1280x720, 24000/1001 fps, 9327 frames
        static ref DIMENSIONS_REGEX: Regex = Regex::new(r": (\d+)x(\d+), .*fps, (\d+) frames").unwrap();
    }

    const REGEX_ERROR: &str = "Could not detect video dimensions";
    let captures = DIMENSIONS_REGEX.captures(&output);
    if let Some(captures) = captures {
        if captures.len() >= 4 {
            Ok(VideoDimensions {
                width: captures[1].parse().map_err(|e| format!("{}", e))?,
                height: captures[2].parse().map_err(|e| format!("{}", e))?,
                frames: captures[3].parse().map_err(|e| format!("{}", e))?,
            })
        } else {
            Err(REGEX_ERROR.to_owned())
        }
    } else {
        Err(REGEX_ERROR.to_owned())
    }
}

fn get_video_dimensions_vps(input: &Path) -> Result<VideoDimensions, String> {
    let command = cross_platform_command(dotenv!("VSPIPE_PATH"))
        .arg("-i")
        .arg(input)
        .arg("-")
        .output()
        .map_err(|e| format!("{}", e))?;
    let output = String::from_utf8_lossy(&command.stdout);

    const PARSE_ERROR: &str = "Could not detect video dimensions";
    let lines = output.lines().take(3).collect::<Vec<_>>();
    if lines.len() == 3 {
        Ok(VideoDimensions {
            width: lines[0]
                .replace("Width: ", "")
                .trim()
                .parse()
                .map_err(|e| format!("{}", e))?,
            height: lines[1]
                .replace("Height: ", "")
                .trim()
                .parse()
                .map_err(|e| format!("{}", e))?,
            frames: lines[2]
                .replace("Frames: ", "")
                .trim()
                .parse()
                .map_err(|e| format!("{}", e))?,
        })
    } else {
        Err(PARSE_ERROR.to_owned())
    }
}

#[derive(Debug, Clone, Copy)]
struct X264Settings {
    pub crf: u8,
    pub ref_frames: u8,
    pub psy_rd: (f32, f32),
    pub deblock: (i8, i8),
    pub aq_strength: f32,
    pub min_keyint: usize,
    pub max_keyint: usize,
    pub colorspace: &'static str,
}

impl X264Settings {
    pub fn new(crf: u8, profile: Profile, hd: bool) -> Self {
        X264Settings {
            crf,
            ref_frames: match profile {
                Profile::Anime => 8,
                Profile::Film => 5,
                Profile::OneTwenty => 16,
            },
            psy_rd: match profile {
                Profile::Anime => (0.7, 0.0),
                _ => (1.0, 0.2),
            },
            deblock: match profile {
                Profile::Anime => (-1, -1),
                _ => (-2, -2),
            },
            aq_strength: match profile {
                Profile::Anime => 0.7,
                _ => 1.0,
            },
            min_keyint: match profile {
                Profile::Anime => 12,
                Profile::Film => 24,
                Profile::OneTwenty => 120,
            },
            max_keyint: match profile {
                Profile::Anime => 400,
                Profile::Film => 240,
                Profile::OneTwenty => 1200,
            },
            colorspace: if hd { "bt709" } else { "smpte170m" },
        }
    }

    pub fn apply_to_command<'a>(&self, command: &'a mut Command) -> &'a mut Command {
        command
            .arg("--crf")
            .arg(self.crf.to_string())
            .arg("--ref")
            .arg(self.ref_frames.to_string())
            .arg("--mixed-refs")
            .arg("--no-fast-pskip")
            .arg("--b-adapt")
            .arg("2")
            .arg("--bframes")
            .arg(self.ref_frames.to_string())
            .arg("--b-pyramid")
            .arg("strict")
            .arg("--weightb")
            .arg("--direct")
            .arg("spatial")
            .arg("--subme")
            .arg("10")
            .arg("--trellis")
            .arg("2")
            .arg("--partitions")
            .arg("all")
            .arg("--psy-rd")
            .arg(format!("{:.1}:{:.1}", self.psy_rd.0, self.psy_rd.1))
            .arg("--deblock")
            .arg(format!("{}:{}", self.deblock.0, self.deblock.1))
            .arg("--me")
            .arg("umh")
            .arg("--merange")
            .arg("32")
            .arg("--fade-compensate")
            .arg("0.5")
            .arg("--rc-lookahead")
            .arg("60")
            .arg("--aq-mode")
            .arg("3")
            .arg("--aq-strength")
            .arg(format!("{:.1}", self.aq_strength))
            .arg("-i")
            .arg(self.min_keyint.to_string())
            .arg("-I")
            .arg(self.max_keyint.to_string())
            .arg("--vbv-maxrate")
            .arg("40000")
            .arg("--vbv-bufsize")
            .arg("30000")
            .arg("--colormatrix")
            .arg(self.colorspace)
            .arg("--colorprim")
            .arg(self.colorspace)
            .arg("--transfer")
            .arg(self.colorspace)
    }
}

fn convert_video(
    input: &Path,
    profile: Profile,
    crf: u8,
    hd: bool,
    frames: u32,
) -> Result<(), String> {
    let settings = X264Settings::new(crf, profile, hd);
    let mut command = cross_platform_command(dotenv!("X264_PATH"));
    settings
        .apply_to_command(&mut command)
        .arg("--output")
        .arg(input.with_extension("264"))
        .arg(input);
    // TODO: Fix piping on Windows
//    let filename = input.file_name().unwrap().to_str().unwrap();
//    let pipe = if filename.ends_with(".avs") {
//        cross_platform_command(dotenv!("AVS2YUV_PATH"))
//            .arg(input)
//            .arg("-")
//            .stdout(Stdio::piped())
//            .spawn()
//            .unwrap()
//    } else if filename.ends_with(".vpy") {
//        cross_platform_command(dotenv!("VSPIPE_PATH"))
//            .arg("--y4m")
//            .arg(input)
//            .arg("-")
//            .stdout(Stdio::piped())
//            .spawn()
//            .unwrap()
//    } else {
//        panic!("Unrecognized input type");
//    };
//    command
//        .arg("--frames")
//        .arg(format!("{}", frames))
//        .arg("--stdin")
//        .arg("y4m")
//        .arg("-")
//        .stdin(unsafe {
//            #[cfg(unix)]
//            {
//                Stdio::from_raw_fd(pipe.stdout.unwrap().as_raw_fd())
//            }
//            #[cfg(windows)]
//            {
//                Stdio::from_raw_handle(pipe.stdout.unwrap().as_raw_handle())
//            }
//        })
//        .stderr(Stdio::inherit());
    let status = command
        .status()
        .map_err(|e| format!("Failed to execute x264: {}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Failed to execute x264: Exited with code {:x}",
            status.code().unwrap()
        ))
    }
}

fn convert_audio(input: &Path, convert: bool) -> Result<(), String> {
    let avs_contents = read_file(input)?;
    if !avs_contents.to_lowercase().contains("audiodub") {
        const TRY_EXTENSIONS: [&str; 8] = ["wav", "aac", "ac3", "dts", "mkv", "avi", "mp4", "flv"];
        let mut i = 0;
        let mut input_video = input.with_extension(TRY_EXTENSIONS[i]);
        while !input_video.exists() {
            i += 1;
            if i >= TRY_EXTENSIONS.len() {
                return Err("No file found to read audio from".to_owned());
            }
            input_video = input.with_extension(TRY_EXTENSIONS[i]);
        }
        let mut command = cross_platform_command(dotenv!("FFMPEG_PATH"));
        command
            .arg("-y")
            .arg("-i")
            .arg(input_video)
            .arg("-acodec")
            .arg(if convert { "aac" } else { "copy" });
        if convert {
            command.arg("-q:a").arg("1");
        }
        command
            .arg("-map")
            .arg("0:a:0")
            .arg("-map_chapters")
            .arg("-1")
            .arg(input.with_extension("m4a"));
        let status = command
            .status()
            .map_err(|e| format!("Failed to execute ffmpeg: {}", e))?;
        return if status.success() {
            Ok(())
        } else {
            Err("Failed to execute ffmpeg".to_owned())
        };
    }

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
        .stdin(unsafe {
            #[cfg(unix)]
            {
                Stdio::from_raw_fd(wavi.stdout.unwrap().as_raw_fd())
            }
            #[cfg(windows)]
            {
                Stdio::from_raw_handle(wavi.stdout.unwrap().as_raw_handle())
            }
        })
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("Failed to execute ffmpeg: {}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err("Failed to execute ffmpeg".to_owned())
    }
}

fn mux_mp4(input: &Path) -> Result<(), String> {
    let mut output_path = PathBuf::from(dotenv!("OUTPUT_PATH"));
    output_path.push(input.with_extension("mp4").file_name().unwrap());

    let status = cross_platform_command(dotenv!("FFMPEG_PATH"))
        .arg("-i")
        .arg(input.with_extension("264"))
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

fn mux_mp4_direct(input: &Path, audio_track: u32) -> Result<(), String> {
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

fn read_file(input: &Path) -> Result<String, String> {
    let file = File::open(input).unwrap();
    let mut buf_reader = BufReader::new(file);
    let mut contents = String::new();
    buf_reader.read_to_string(&mut contents).unwrap();
    Ok(contents)
}

fn cross_platform_command(program: &str) -> Command {
    if program.starts_with("wine ") {
        let mut command = Command::new("wine");
        command.arg(&program[5..]);
        command
    } else {
        Command::new(program)
    }
}
