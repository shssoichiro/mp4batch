use colored::*;
use std::sync::atomic::Ordering;
use std::{
    env::temp_dir,
    fs::File,
    io::Write,
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, atomic::AtomicBool},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::output::VideoEncoderIdent;
use crate::{
    absolute_path,
    input::{Colorimetry, PixelFormat, VideoDimensions, get_video_frame_count},
    monitor_for_sigterm,
    output::Profile,
};

#[allow(clippy::too_many_arguments)]
pub fn convert_video_x264(
    vpy_input: &Path,
    output: &Path,
    crf: i16,
    profile: Profile,
    compat: bool,
    dimensions: VideoDimensions,
    force_keyframes: Option<&str>,
    colorimetry: Colorimetry,
    sigterm: &Arc<AtomicBool>,
) -> anyhow::Result<()> {
    if !dimensions.width.is_multiple_of(8) {
        eprintln!(
            "{} {} {} {}",
            "[Warning]".yellow().bold(),
            "Width".yellow(),
            dimensions.width.to_string().yellow(),
            "is not divisble by 8".yellow()
        );
    }
    if !dimensions.height.is_multiple_of(8) {
        eprintln!(
            "{} {} {} {}",
            "[Warning]".yellow().bold(),
            "Height".yellow(),
            dimensions.height.to_string().yellow(),
            "is not divisble by 8".yellow()
        );
    }

    if output.exists() && get_video_frame_count(output).unwrap_or(0) == dimensions.frames {
        eprintln!("Video output already exists, reusing");
        return Ok(());
    }

    let mut pipe = Command::new("vspipe")
        .arg("-c")
        .arg("y4m")
        .arg("-o")
        .arg("0")
        .arg(absolute_path(vpy_input).expect("Unable to get absolute path"))
        .arg("-")
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("Failed to execute vspipe for x264 encoding: {}", e))?;

    let mut command = Command::new("x264");
    command
        .arg("--demuxer")
        .arg("y4m")
        .arg("--frames")
        .arg(dimensions.frames.to_string());
    let args = build_x264_args_string(
        crf,
        dimensions,
        profile,
        compat,
        force_keyframes,
        colorimetry,
    )?;
    eprintln!("x264 args: {args}");
    for arg in args.split_ascii_whitespace() {
        command.arg(arg);
    }
    command
        .arg("-o")
        .arg(absolute_path(output).expect("Unable to get absolute path"))
        .arg("-");
    command
        .stdin(pipe.stdout.take().expect("stdout should be writeable"))
        .stderr(Stdio::inherit());
    let status = command
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to execute av1an: {}", e))?;
    let is_done = Arc::new(AtomicBool::new(false));
    monitor_for_sigterm(&pipe, Arc::clone(sigterm), Arc::clone(&is_done));
    pipe.wait()?;
    is_done.store(true, Ordering::Relaxed);

    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "Failed to execute x264: Exited with code {:x}",
            status.code().unwrap_or(-1)
        ))
    }
}

pub fn build_x264_args_string(
    crf: i16,
    dimensions: VideoDimensions,
    profile: Profile,
    compat: bool,
    force_keyframes: Option<&str>,
    colorimetry: Colorimetry,
) -> anyhow::Result<String> {
    let fps = (dimensions.fps.0 as f32 / dimensions.fps.1 as f32).round() as u32;
    let min_keyint = if profile.is_anime() { fps / 2 } else { fps };
    let max_keyint = if profile.is_anime() {
        fps * 15
    } else {
        fps * 10
    };
    let preset = if profile == Profile::Fast {
        "faster"
    } else {
        "veryslow"
    };
    let bframes = match profile {
        Profile::Film | Profile::Grain => 5,
        Profile::Anime | Profile::AnimeDetailed | Profile::AnimeGrain => 8,
        Profile::Fast => 3,
    };
    let psy_rd = if profile.is_anime() {
        format!("{:.1}:{:.1}", 0.7, 0.0)
    } else {
        format!("{:.1}:{:.1}", 1.0, 0.0)
    };
    let deblock = if profile.is_anime() {
        format!("{}:{}", -2, -1)
    } else {
        format!("{}:{}", -3, -3)
    };
    let merange = if dimensions.width > 1440 {
        48
    } else if dimensions.width > 1024 {
        32
    } else {
        24
    };
    let aq_str = match profile {
        Profile::Grain => "0.9",
        Profile::Film | Profile::AnimeGrain => "0.8",
        Profile::Anime | Profile::AnimeDetailed | Profile::Fast => "0.7",
    };
    let qcomp = match profile {
        Profile::Film | Profile::Grain | Profile::Fast => 0.75,
        Profile::AnimeGrain => 0.7,
        Profile::Anime | Profile::AnimeDetailed => 0.65,
    };
    let prim = colorimetry.get_primaries_encoder_string(VideoEncoderIdent::X264)?;
    let matrix = colorimetry.get_matrix_encoder_string(VideoEncoderIdent::X264)?;
    let transfer = colorimetry.get_transfer_encoder_string(VideoEncoderIdent::X264)?;
    let range = colorimetry.get_range_encoder_string(VideoEncoderIdent::X264)?;
    let csp = colorimetry.get_chromaloc_encoder_string(VideoEncoderIdent::X264)?;
    let depth = dimensions.bit_depth;
    let vbv = if compat {
        "--level 4.1 --vbv-maxrate 50000 --vbv-bufsize 78125"
    } else {
        ""
    };
    let level = match dimensions.pixel_format {
        PixelFormat::Yuv422 => "--profile high422 --output-csp i422",
        PixelFormat::Yuv444 => "--profile high444 --output-csp i444",
        _ => "",
    };
    let qpfile = if let Some(list) = force_keyframes {
        let path = temp_dir().join(format!(
            "x264-qp-{}.txt",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("System time is broken")
                .as_millis()
        ));
        let mut file = File::create(&path)?;
        for kf in list.split(',') {
            file.write_all(format!("{} I -1\n", kf).as_bytes())?;
        }
        file.flush()?;
        format!("--qpfile {}", path.to_string_lossy())
    } else {
        String::new()
    };
    Ok(format!(
        " --crf {crf} --preset {preset} --bframes {bframes} --psy-rd {psy_rd} --deblock {deblock} \
         --merange {merange} --rc-lookahead 96 --aq-mode 3 --aq-strength {aq_str} --no-mbtree -i \
         {min_keyint} -I {max_keyint} --qcomp {qcomp} --ipratio 1.30 --pbratio 1.20 \
         --no-fast-pskip --no-dct-decimate --colorprim {prim} --colormatrix {matrix} --transfer \
         {transfer} --input-range {range} --range {range} {csp} --output-depth {depth} {vbv} \
         {level} {qpfile} "
    ))
}
