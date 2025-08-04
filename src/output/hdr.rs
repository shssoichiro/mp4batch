use anyhow::{Result, anyhow};
use nom::{
    IResult, Parser,
    bytes::complete::tag,
    character::complete::{char, digit1},
    sequence::{delimited, preceded, separated_pair},
};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub fn copy_extra_data(input: &Path, target: &Path) -> Result<()> {
    let metadata = HdrMetadata::parse(input)?;
    let chapters = extract_chapters(input);
    apply_data(target, metadata.as_ref(), chapters.as_deref())?;

    Ok(())
}

#[derive(Debug, Clone, Copy, Default)]
pub struct HdrMetadata {
    pub color_coords: Option<ColorCoordinates>,
    pub max_luma: u32,
    pub min_luma: f64,
    pub max_content_light: u32,
    pub max_frame_light: u32,
}

impl HdrMetadata {
    // Why do we have to go through all three of these?
    //
    // WELL, I'm glad you asked.
    // Sometimes, exactly one of these three tools will be able
    // to extract the HDR metadata. So we have to test all three.
    // Just to be sure we didn't miss it.
    //
    // Encoding is dumb.
    pub fn parse(input: &Path) -> Result<Option<Self>> {
        let mut data = None;
        match parse_mkvinfo(input) {
            Ok(info) => {
                data = info;
            }
            Err(e) => {
                eprintln!("Warning: {}", e);
            }
        }
        if let Some(data) = data
            && data.color_coords.is_some()
        {
            return Ok(Some(data));
        }

        match parse_mediainfo(input) {
            Ok(info) => {
                if info.is_some() {
                    data = info;
                }
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                anyhow::bail!("Unable to parse metadata");
            }
        }
        if let Some(data) = data
            && data.color_coords.is_some()
        {
            return Ok(Some(data));
        }

        match parse_ffprobe(input) {
            Ok(Some(info)) => {
                data = Some(info);
            }
            Ok(None) => (),
            Err(e) => {
                eprintln!("Warning: {}", e);
            }
        }

        Ok(data)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ColorCoordinates {
    pub red: (f64, f64),
    pub green: (f64, f64),
    pub blue: (f64, f64),
    pub white: (f64, f64),
}

fn extract_chapters(input: &Path) -> Option<PathBuf> {
    let output = input.with_extension("hdrcp_chapters.xml");
    let result = Command::new("mkvextract")
        .arg(input)
        .arg("chapters")
        .arg(&output)
        .status();
    (result.is_ok() && output.exists() && output.metadata().expect("File exists").len() > 0)
        .then_some(output)
}

pub fn apply_data(target: &Path, hdr: Option<&HdrMetadata>, chapters: Option<&Path>) -> Result<()> {
    if hdr.is_none() && chapters.is_none() {
        return Ok(());
    }

    let mut command = build_mkvmerge_command(target, hdr, chapters);
    eprintln!("Running: {:?}", command);
    let status = command.status()?;
    if !status.success() {
        anyhow::bail!("Failed to mux metadata");
    }
    Ok(())
}

fn build_mkvmerge_command(
    target: &Path,
    hdr: Option<&HdrMetadata>,
    chapters: Option<&Path>,
) -> Command {
    let mut command = Command::new("mkvpropedit");
    command.arg("-e").arg("track:v1");
    if let Some(hdr_data) = hdr {
        if hdr_data.max_content_light > 0 {
            command
                .arg("-s")
                .arg(format!("max-content-light={}", hdr_data.max_content_light));
        }
        if hdr_data.max_frame_light > 0 {
            command
                .arg("-s")
                .arg(format!("max-frame-light={}", hdr_data.max_frame_light));
        }
        command
            .arg("-s")
            .arg(format!("max-luminance={}", hdr_data.max_luma))
            .arg("-s")
            .arg(format!("min-luminance={:.4}", hdr_data.min_luma));
        if let Some(ref color_coords) = hdr_data.color_coords {
            command
                .arg("-s")
                .arg(format!(
                    "chromaticity-coordinates-red-x={:.5}",
                    color_coords.red.0
                ))
                .arg("-s")
                .arg(format!(
                    "chromaticity-coordinates-red-y={:.5}",
                    color_coords.red.1
                ))
                .arg("-s")
                .arg(format!(
                    "chromaticity-coordinates-green-x={:.5}",
                    color_coords.green.0
                ))
                .arg("-s")
                .arg(format!(
                    "chromaticity-coordinates-green-y={:.5}",
                    color_coords.green.1
                ))
                .arg("-s")
                .arg(format!(
                    "chromaticity-coordinates-blue-x={:.5}",
                    color_coords.blue.0
                ))
                .arg("-s")
                .arg(format!(
                    "chromaticity-coordinates-blue-y={:.5}",
                    color_coords.blue.1
                ))
                .arg("-s")
                .arg(format!("white-coordinates-x={:.5}", color_coords.white.0))
                .arg("-s")
                .arg(format!("white-coordinates-y={:.5}", color_coords.white.1));
        }
    }
    if let Some(chapters) = chapters {
        command.arg("-c").arg(chapters);
    }
    command.arg(target);
    command
}

// MKVInfo may include data that looks like this:
//
// |    + Colour matrix coefficients: 9
// |    + Colour range: 1
// |    + Horizontal chroma siting: 2
// |    + Vertical chroma siting: 2
// |    + Colour transfer: 16
// |    + Colour primaries: 9
// |    + Maximum content light: 944
// |    + Maximum frame light: 143
// |    + Video colour mastering metadata
// |     + Red colour coordinate x: 0.6800000071525574
// |     + Red colour coordinate y: 0.3199799954891205
// |     + Green colour coordinate x: 0.26499998569488525
// |     + Green colour coordinate y: 0.6899799704551697
// |     + Blue colour coordinate x: 0.15000000596046448
// |     + Blue colour coordinate y: 0.05998000130057335
// |     + White colour coordinate x: 0.3126800060272217
// |     + White colour coordinate y: 0.32899999618530273
// |     + Maximum luminance: 1000
// |     + Minimum luminance: 0.004999999888241291
//
// This is the case if the metadata was muxed into the MKV headers.
fn parse_mkvinfo(input: &Path) -> anyhow::Result<Option<HdrMetadata>> {
    let result = Command::new("mkvinfo").arg(input).output()?;
    let output = String::from_utf8_lossy(&result.stdout);

    let mut hdr = HdrMetadata::default();
    let mut has_hdr = false;
    for line in output.lines() {
        // HDR details
        if line.contains("Video colour mastering metadata") {
            has_hdr = true;
            continue;
        }
        if line.contains("Maximum content light:") {
            hdr.max_content_light = line
                .split_once(": ")
                .expect("line contains delimiter")
                .1
                .parse()?;
            continue;
        }
        if line.contains("Maximum frame light:") {
            hdr.max_frame_light = line
                .split_once(": ")
                .expect("line contains delimiter")
                .1
                .parse()?;
            continue;
        }

        if line.contains("Red colour coordinate x:") {
            // This should always be the first piece of color data, so we initialize here
            hdr.color_coords = Some(ColorCoordinates::default());

            hdr.color_coords
                .as_mut()
                .expect("color coords is set")
                .red
                .0 = line
                .split_once(": ")
                .expect("line contains delimiter")
                .1
                .parse()?;
            continue;
        }
        if line.contains("Red colour coordinate y:") {
            hdr.color_coords
                .as_mut()
                .expect("color coords is set")
                .red
                .1 = line
                .split_once(": ")
                .expect("line contains delimiter")
                .1
                .parse()?;
            continue;
        }
        if line.contains("Green colour coordinate x:") {
            hdr.color_coords
                .as_mut()
                .expect("color coords is set")
                .green
                .0 = line
                .split_once(": ")
                .expect("line contains delimiter")
                .1
                .parse()?;
            continue;
        }
        if line.contains("Green colour coordinate y:") {
            hdr.color_coords
                .as_mut()
                .expect("color coords is set")
                .green
                .1 = line
                .split_once(": ")
                .expect("line contains delimiter")
                .1
                .parse()?;
            continue;
        }
        if line.contains("Blue colour coordinate x:") {
            hdr.color_coords
                .as_mut()
                .expect("color coords is set")
                .blue
                .0 = line
                .split_once(": ")
                .expect("line contains delimiter")
                .1
                .parse()?;
            continue;
        }
        if line.contains("Blue colour coordinate y:") {
            hdr.color_coords
                .as_mut()
                .expect("color coords is set")
                .blue
                .1 = line
                .split_once(": ")
                .expect("line contains delimiter")
                .1
                .parse()?;
            continue;
        }
        if line.contains("White colour coordinate x:") {
            hdr.color_coords
                .as_mut()
                .expect("color coords is set")
                .white
                .0 = line
                .split_once(": ")
                .expect("line contains delimiter")
                .1
                .parse()?;
            continue;
        }
        if line.contains("White colour coordinate y:") {
            hdr.color_coords
                .as_mut()
                .expect("color coords is set")
                .white
                .1 = line
                .split_once(": ")
                .expect("line contains delimiter")
                .1
                .parse()?;
            continue;
        }

        if line.contains("Maximum luminance:") {
            hdr.max_luma = line
                .split_once(": ")
                .expect("Line should contain colon separator")
                .1
                .parse()?;
            continue;
        }
        if line.contains("Minimum luminance:") {
            hdr.min_luma = line
                .split_once(": ")
                .expect("Line should contain colon separator")
                .1
                .parse()?;
        }
    }

    Ok(has_hdr.then_some(hdr))
}

// MediaInfo may include the following pieces of data:
//
// In the x265 headers: master-display=G(13250,34499)B(7499,2999)R(34000,15999)WP(15634,16450)L(10000000,50)cll=944,143
//
// In the video info:
//
// Color range                              : Limited
// Color primaries                          : BT.2020
// Transfer characteristics                 : PQ
// Matrix coefficients                      : BT.2020 non-constant
// Mastering display color primaries        : Display P3
// Mastering display luminance              : min: 0.0050 cd/m2, max: 1000 cd/m2
// Maximum Content Light Level              : 944 cd/m2
// Maximum Frame-Average Light Level        : 143 cd/m2
//
// We need this if the metadata was encoded into the video stream by x265.
// Note that MediaInfo does not print the chroma location, so we should
// always prefer mkvinfo's basic output if we have it.
fn parse_mediainfo(input: &Path) -> Result<Option<HdrMetadata>> {
    let result = Command::new("mediainfo").arg(input).output()?;
    let output = String::from_utf8_lossy(&result.stdout);

    let mut hdr = HdrMetadata::default();
    let mut has_hdr = false;
    for line in output.lines() {
        // HDR details
        if line.contains("Mastering display color primaries") {
            has_hdr = true;
            continue;
        }
        if line.contains("Maximum Content Light Level") {
            hdr.max_content_light = line
                .split_once(": ")
                .expect("line contains delimiter")
                .1
                .trim_end_matches(" cd/m2")
                .parse()?;
            continue;
        }
        if line.contains("Maximum Frame-Average Light Level") {
            hdr.max_frame_light = line
                .split_once(": ")
                .expect("line contains delimiter")
                .1
                .trim_end_matches(" cd/m2")
                .parse()?;
            continue;
        }
        if line.contains("Mastering display luminance") {
            let output = line.split_once(": ").expect("line contains delimiter").1;
            let (min, max) = output.split_once(", ").expect("line contains delimiter");
            hdr.min_luma = min
                .trim_start_matches("min: ")
                .trim_end_matches(" cd/m2")
                .parse()?;
            hdr.max_luma = max
                .trim_start_matches("max: ")
                .trim_end_matches(" cd/m2")
                .parse()?;
            continue;
        }

        if line.contains("Encoding settings") && line.contains("master-display") {
            let settings = line.split_once(": ").expect("line contains delimiter").1;
            hdr.color_coords = Some(parse_x265_settings(settings)?);
        }
    }

    Ok(has_hdr.then_some(hdr))
}

// Takes in a string that contains a substring in the format:
// master-display=G(13250,34499)B(7499,2999)R(34000,15999)WP(15634,16450)L(10000000,50)cll=944,143
//
// Also using unwrap here because I don't want to fight the borrow checker anymore.
#[expect(
    clippy::string_slice,
    reason = "we know the split index is on a char boundary"
)]
fn parse_x265_settings(input: &str) -> Result<ColorCoordinates> {
    const MASTER_DISPLAY_HEADER: &str = "master-display=";
    let header_pos = input
        .find(MASTER_DISPLAY_HEADER)
        .ok_or_else(|| anyhow::anyhow!("Failed to find master display header"))?;
    let input = &input[(header_pos + MASTER_DISPLAY_HEADER.len())..];
    let (input, (gx, gy)) = preceded(char('G'), get_coordinate_pair)
        .parse_complete(input)
        .map_err(|e| anyhow!("{}", e))?;
    let (input, (bx, by)) = preceded(char('B'), get_coordinate_pair)
        .parse_complete(input)
        .map_err(|e| anyhow!("{}", e))?;
    let (input, (rx, ry)) = preceded(char('R'), get_coordinate_pair)
        .parse_complete(input)
        .map_err(|e| anyhow!("{}", e))?;
    let (_, (wx, wy)) = preceded(tag("WP"), get_coordinate_pair)
        .parse_complete(input)
        .map_err(|e| anyhow!("{}", e))?;

    // Why 50000? Why indeed.
    Ok(ColorCoordinates {
        red: (rx as f64 / 50000., ry as f64 / 50000.),
        green: (gx as f64 / 50000., gy as f64 / 50000.),
        blue: (bx as f64 / 50000., by as f64 / 50000.),
        white: (wx as f64 / 50000., wy as f64 / 50000.),
    })
}

fn get_coordinate_pair(input: &str) -> IResult<&str, (u32, u32)> {
    delimited(
        char('('),
        separated_pair(digit1, char(','), digit1),
        char(')'),
    )
    .parse_complete(input)
    .map(|(input, (x, y))| {
        (
            input,
            (
                x.parse::<u32>().expect("value is numeric"),
                y.parse::<u32>().expect("value is numeric"),
            ),
        )
    })
}

// And then there are some videos where the data only shows in ffprobe.
//
// Like so:
//
// [SIDE_DATA]
// side_data_type=Mastering display metadata
// red_x=34000/50000
// red_y=15999/50000
// green_x=13250/50000
// green_y=34499/50000
// blue_x=7499/50000
// blue_y=2999/50000
// white_point_x=15634/50000
// white_point_y=16450/50000
// min_luminance=50/10000
// max_luminance=10000000/10000
// [/SIDE_DATA]
// [SIDE_DATA]
// side_data_type=Content light level metadata
// max_content=944
// max_average=143
// [/SIDE_DATA]
//
// This only looks at HDR data, because at least one of mediainfo
// or mkvinfo should have found the color primary data.
// Or your source is badly broken.
fn parse_ffprobe(input: &Path) -> anyhow::Result<Option<HdrMetadata>> {
    let result = Command::new("ffprobe")
        .arg("-v")
        .arg("quiet")
        .arg("-select_streams")
        .arg("v:0")
        .arg("-show_frames")
        .arg("-read_intervals")
        .arg("%+#1")
        .arg(input)
        .output()?;
    let output = String::from_utf8_lossy(&result.stdout);

    if !(output.contains("side_data_type=Mastering display metadata")
        && output.contains("side_data_type=Content light level metadata"))
    {
        return Ok(None);
    }

    let mut hdr = HdrMetadata::default();
    for line in output.lines() {
        if line.starts_with("red_x=") {
            // This should always be the first piece of color data, so we initialize here
            hdr.color_coords = Some(ColorCoordinates::default());

            let (num, denom) = line
                .split_once('=')
                .expect("delimiter is present")
                .1
                .split_once('/')
                .expect("delimiter is present");
            hdr.color_coords
                .as_mut()
                .expect("color coords is set")
                .red
                .0 = num.parse::<f64>()? / denom.parse::<f64>()?;
            continue;
        }
        if line.starts_with("red_y=") {
            let (num, denom) = line
                .split_once('=')
                .expect("delimiter is present")
                .1
                .split_once('/')
                .expect("delimiter is present");
            hdr.color_coords
                .as_mut()
                .expect("color coords is set")
                .red
                .1 = num.parse::<f64>()? / denom.parse::<f64>()?;
            continue;
        }
        if line.starts_with("green_x=") {
            let (num, denom) = line
                .split_once('=')
                .expect("delimiter is present")
                .1
                .split_once('/')
                .expect("delimiter is present");
            hdr.color_coords
                .as_mut()
                .expect("color coords is set")
                .green
                .0 = num.parse::<f64>()? / denom.parse::<f64>()?;
            continue;
        }
        if line.starts_with("green_y=") {
            let (num, denom) = line
                .split_once('=')
                .expect("delimiter is present")
                .1
                .split_once('/')
                .expect("delimiter is present");
            hdr.color_coords
                .as_mut()
                .expect("color coords is set")
                .green
                .1 = num.parse::<f64>()? / denom.parse::<f64>()?;
            continue;
        }
        if line.starts_with("blue_x=") {
            let (num, denom) = line
                .split_once('=')
                .expect("delimiter is present")
                .1
                .split_once('/')
                .expect("delimiter is present");
            hdr.color_coords
                .as_mut()
                .expect("color coords is set")
                .blue
                .0 = num.parse::<f64>()? / denom.parse::<f64>()?;
            continue;
        }
        if line.starts_with("blue_y=") {
            let (num, denom) = line
                .split_once('=')
                .expect("delimiter is present")
                .1
                .split_once('/')
                .expect("delimiter is present");
            hdr.color_coords
                .as_mut()
                .expect("color coords is set")
                .blue
                .1 = num.parse::<f64>()? / denom.parse::<f64>()?;
            continue;
        }
        if line.starts_with("white_point_x=") {
            let (num, denom) = line
                .split_once('=')
                .expect("delimiter is present")
                .1
                .split_once('/')
                .expect("delimiter is present");
            hdr.color_coords
                .as_mut()
                .expect("color coords is set")
                .white
                .0 = num.parse::<f64>()? / denom.parse::<f64>()?;
            continue;
        }
        if line.starts_with("white_point_y=") {
            let (num, denom) = line
                .split_once('=')
                .expect("delimiter is present")
                .1
                .split_once('/')
                .expect("delimiter is present");
            hdr.color_coords
                .as_mut()
                .expect("color coords is set")
                .white
                .1 = num.parse::<f64>()? / denom.parse::<f64>()?;
            continue;
        }
        if line.starts_with("min_luminance=") {
            let (num, denom) = line
                .split_once('=')
                .expect("delimiter is present")
                .1
                .split_once('/')
                .expect("delimiter is present");
            hdr.min_luma = num.parse::<f64>()? / denom.parse::<f64>()?;
            continue;
        }
        if line.starts_with("max_luminance=") {
            let (num, denom) = line
                .split_once('=')
                .expect("delimiter is present")
                .1
                .split_once('/')
                .expect("delimiter is present");
            hdr.max_luma = num.parse::<u32>()? / denom.parse::<u32>()?;
            continue;
        }

        if line.starts_with("max_content=") {
            hdr.max_content_light = line
                .split_once('=')
                .expect("Line should contain equals separator")
                .1
                .parse()?;
            continue;
        }
        if line.starts_with("max_average=") {
            hdr.max_frame_light = line
                .split_once('=')
                .expect("Line should contain equals separator")
                .1
                .parse()?;
        }
    }
    Ok(Some(hdr))
}
