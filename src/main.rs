#![feature(plugin)]
#![cfg_attr(feature = "clippy", plugin(clippy))]
#![plugin(dotenv_macros)]

extern crate clap;
#[macro_use]
extern crate lazy_static;
extern crate regex;

use std::fs::File;
use std::io::BufReader;
use std::io::prelude::*;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::os::unix::io::{AsRawFd, FromRawFd};
#[cfg(windows)]
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::str::FromStr;

use clap::{App, Arg};
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

fn main() {
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
                .help("remux mkv to mp4; will convert audio streams to aac without touching video"),
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
    let crf = args.value_of("crf")
        .unwrap_or("18")
        .parse::<u8>()
        .expect(CRF_PARSE_ERROR);
    assert!(crf <= 51, CRF_PARSE_ERROR);

    let input = Path::new(input);
    assert!(input.exists(), "Input path does not exist");

    if args.is_present("direct") {
        if input.is_dir() {
            let dir_entries = input.read_dir().unwrap();
            for entry in dir_entries.map(|e| e.unwrap()).filter(|e| {
                e.path()
                    .extension()
                    .unwrap_or_default()
                    .to_str()
                    .unwrap_or_default() == "mkv"
            }) {
                let result = process_direct(&entry.path());
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
            process_direct(input).unwrap();
        }
    }

    if input.is_dir() {
        let dir_entries = input.read_dir().unwrap();
        for entry in dir_entries.map(|e| e.unwrap()).filter(|e| {
            e.path()
                .extension()
                .unwrap_or_default()
                .to_str()
                .unwrap_or_default() == "avs"
        }) {
            let result = process_file(&entry.path(), profile, crf);
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
            "avs",
            "Input file must be an avisynth script"
        );
        process_file(input, profile, crf).unwrap();
    }
}

fn process_file(input: &Path, profile: Profile, crf: u8) -> Result<(), String> {
    let (_, height, frames) = get_video_dimensions(input)?;
    convert_video(input, profile, crf, height >= 576, frames)?;
    convert_audio(input)?;
    mux_mp4(input)?;
    println!("Finished converting {}", input.to_string_lossy());
    Ok(())
}

fn process_direct(input: &Path) -> Result<(), String> {
    mux_mp4_direct(input)?;
    println!("Finished converting {}", input.to_string_lossy());
    Ok(())
}

fn get_video_dimensions(input: &Path) -> Result<(u32, u32, u32), String> {
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
            Ok((
                captures[1].parse().map_err(|e| format!("{}", e))?,
                captures[2].parse().map_err(|e| format!("{}", e))?,
                captures[3].parse().map_err(|e| format!("{}", e))?,
            ))
        } else {
            Err(REGEX_ERROR.to_owned())
        }
    } else {
        Err(REGEX_ERROR.to_owned())
    }
}

fn convert_video(
    input: &Path,
    profile: Profile,
    crf: u8,
    hd: bool,
    frames: u32,
) -> Result<(), String> {
    let ref_frames = match profile {
        Profile::Anime => "8",
        Profile::Film => "5",
        Profile::OneTwenty => "16",
    };
    let psy_rd = match profile {
        Profile::Anime => "0.7:0.0",
        _ => "1.0:0.2",
    };
    let deblock = match profile {
        Profile::Anime => "-1:-1",
        _ => "-2:-2",
    };
    let aq_strength = match profile {
        Profile::Anime => "0.7",
        _ => "1.0",
    };
    let min_keyint = match profile {
        Profile::Anime => "12",
        Profile::Film => "25",
        Profile::OneTwenty => "120",
    };
    let max_keyint = match profile {
        Profile::Anime => "400",
        Profile::Film => "250",
        Profile::OneTwenty => "1200",
    };
    let colorspace = if hd { "bt709" } else { "smpte170m" };

    let avs2yuv = cross_platform_command(dotenv!("AVS2YUV_PATH"))
        .arg(input)
        .arg("-")
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();

    let status = cross_platform_command(dotenv!("X264_PATH"))
        .arg("--frames")
        .arg(format!("{}", frames))
        .arg("--crf")
        .arg(format!("{}", crf))
        .arg("--ref")
        .arg(ref_frames)
        .arg("--mixed-refs")
        .arg("--no-fast-pskip")
        .arg("--b-adapt")
        .arg("2")
        .arg("--bframes")
        .arg(ref_frames)
        .arg("--b-pyramid")
        .arg("normal")
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
        .arg(psy_rd)
        .arg("--deblock")
        .arg(deblock)
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
        .arg(aq_strength)
        .arg("-i")
        .arg(min_keyint)
        .arg("-I")
        .arg(max_keyint)
        .arg("--vbv-maxrate")
        .arg("40000")
        .arg("--vbv-bufsize")
        .arg("30000")
        .arg("--colormatrix")
        .arg(colorspace)
        .arg("--colorprim")
        .arg(colorspace)
        .arg("--transfer")
        .arg(colorspace)
        .arg("--stdin")
        .arg("y4m")
        .arg("--output")
        .arg(input.with_extension("264"))
        .arg("-")
        .stdin(unsafe {
            #[cfg(unix)]
            {
                Stdio::from_raw_fd(avs2yuv.stdout.unwrap().as_raw_fd())
            }
            #[cfg(windows)]
            {
                Stdio::from_raw_handle(avs2yuv.stdout.unwrap().as_raw_handle())
            }
        })
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("{}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err("Failed to execute x264".to_owned())
    }
}

fn convert_audio(input: &Path) -> Result<(), String> {
    let avs_contents = read_file(input)?;
    if !avs_contents.to_lowercase().contains("audiodub") {
        const TRY_EXTENSIONS: [&str; 8] = ["mkv", "avi", "mp4", "flv", "wav", "aac", "ac3", "dts"];
        let mut i = 0;
        let mut input_video = input.with_extension(TRY_EXTENSIONS[i]);
        while !input_video.exists() {
            i += 1;
            if i >= TRY_EXTENSIONS.len() {
                return Err("No file found to read audio from".to_owned());
            }
            input_video = input.with_extension(TRY_EXTENSIONS[i]);
        }
        let status = cross_platform_command(dotenv!("FFMPEG_PATH"))
            .arg("-y")
            .arg("-i")
            .arg(input_video)
            .arg("-acodec")
            .arg("aac")
            .arg("-q:a")
            .arg("1")
            .arg("-map")
            .arg("0:a:0")
            .arg("-map_chapters")
            .arg("-1")
            .arg(input.with_extension("m4a"))
            .status()
            .map_err(|e| format!("{}", e))?;
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
        .map_err(|e| format!("{}", e))?;

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
        .map_err(|e| format!("{}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err("Failed to execute ffmpeg".to_owned())
    }
}

fn mux_mp4(input: &Path) -> Result<(), String> {
    let mut output_path = PathBuf::from(dotenv!("OUTPUT_PATH"));
    output_path.push(input.with_extension("mp4").file_name().unwrap());

    let status = cross_platform_command(dotenv!("MP4BOX_PATH"))
        .arg("-add")
        .arg(input.with_extension("264#trackID=1"))
        .arg("-add")
        .arg(input.with_extension("m4a#trackID=1"))
        .arg("-tmp")
        .arg(dotenv!("TMP_PATH"))
        .arg("-new")
        .arg(output_path)
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("{}", e))?;
    if status.success() {
        Ok(())
    } else {
        Err("Failed to execute mp4box".to_owned())
    }
}

fn mux_mp4_direct(input: &Path) -> Result<(), String> {
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
        .arg("0:a:0")
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
