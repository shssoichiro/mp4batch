use std::{path::Path, str::FromStr};

use crate::input::get_video_mediainfo;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HdrPrimaries {
    Bt2020,
}

impl FromStr for HdrPrimaries {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, <Self as std::str::FromStr>::Err> {
        match s {
            "BT.2020" => Ok(HdrPrimaries::Bt2020),
            _ => Err("Unrecognized HDR color primaries".to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HdrTransfer {
    Pq,
}

impl FromStr for HdrTransfer {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, <Self as std::str::FromStr>::Err> {
        match s {
            "PQ" => Ok(HdrTransfer::Pq),
            _ => Err("Unrecognized HDR transfer characteristics".to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HdrMatrix {
    Bt2020NonConstant,
}

impl FromStr for HdrMatrix {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, <Self as std::str::FromStr>::Err> {
        match s {
            "BT.2020 non-constant" => Ok(HdrMatrix::Bt2020NonConstant),
            _ => Err("Unrecognized HDR matrix coefficients".to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HdrMasteringPrimaries {
    DisplayP3,
}

impl FromStr for HdrMasteringPrimaries {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, <Self as std::str::FromStr>::Err> {
        match s {
            "Display P3" => Ok(HdrMasteringPrimaries::DisplayP3),
            _ => Err("Unrecognized HDR mastering color primaries".to_string()),
        }
    }
}

// These are stored as integers representing 1e4 * the float value
#[derive(Debug, Clone, Copy)]
pub struct HdrMasteringLuminance {
    pub min: u32,
    pub max: u32,
}

impl FromStr for HdrMasteringLuminance {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, <Self as std::str::FromStr>::Err> {
        let parse_val = |val: &str| -> u32 {
            if val.contains('.') {
                assert!(val.split_once('.').unwrap().1.len() == 4);
                val.replace('.', "").parse().unwrap()
            } else {
                val.parse::<u32>().unwrap() * 10000
            }
        };

        let (min, max) = s.split_once(", ").unwrap();
        let min = min.replace("min: ", "").replace(" cd/m2", "");
        let max = max.replace("max: ", "").replace(" cd/m2", "");
        Ok(HdrMasteringLuminance {
            min: parse_val(&min),
            max: parse_val(&max),
        })
    }
}

pub struct HdrInfo {
    pub primaries: HdrPrimaries,
    pub transfer: HdrTransfer,
    pub matrix: HdrMatrix,
    pub mastering_display_primaries: HdrMasteringPrimaries,
    pub mastering_display_luminance: HdrMasteringLuminance,
    pub maximum_content_light_level: u32,
    pub maximum_frame_average_light_level: u32,
    pub master_display: String,
}

pub fn get_hdr_info(input: &Path) -> Result<HdrInfo, String> {
    let mediainfo = get_video_mediainfo(input)?;
    Ok(HdrInfo {
        primaries: HdrPrimaries::from_str(mediainfo.get(&"Color primaries".to_string()).unwrap())?,
        transfer: HdrTransfer::from_str(
            mediainfo
                .get(&"Transfer characteristics".to_string())
                .unwrap(),
        )?,
        matrix: HdrMatrix::from_str(mediainfo.get(&"Matrix coefficients".to_string()).unwrap())?,
        mastering_display_primaries: HdrMasteringPrimaries::from_str(
            mediainfo
                .get(&"Mastering display color primaries".to_string())
                .unwrap(),
        )?,
        mastering_display_luminance: HdrMasteringLuminance::from_str(
            mediainfo
                .get(&"Mastering display luminance".to_string())
                .unwrap(),
        )?,
        maximum_content_light_level: mediainfo
            .get(&"Maximum Content Light Level".to_string())
            .unwrap()
            .replace(" cd/m2", "")
            .parse()
            .unwrap(),
        maximum_frame_average_light_level: mediainfo
            .get(&"Maximum Frame-Average Light Level".to_string())
            .unwrap()
            .replace(" cd/m2", "")
            .parse()
            .unwrap(),
        master_display: mediainfo
            .get(&"Encoding settings".to_string())
            .unwrap()
            .split(" / ")
            .find(|item| item.starts_with("master-display="))
            .unwrap()
            .replace("master-display=", ""),
    })
}
