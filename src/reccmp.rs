use core::fmt;
use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_repr::Deserialize_repr;

use crate::disassemble::Instruction;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Address(pub u64);

impl core::str::FromStr for Address {
    type Err = core::num::ParseIntError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        let stripped = trimmed.strip_prefix("0x").unwrap_or(trimmed);
        u64::from_str_radix(stripped, 16).map(Address)
    }
}

impl Serialize for Address {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Address {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = <&str>::deserialize(deserializer)?;
        let trimmed = s.trim();
        let stripped = trimmed.strip_prefix("0x").unwrap_or(trimmed);

        u64::from_str_radix(stripped, 16)
            .map(Address)
            .map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:08x}", self.0)
    }
}

impl From<u64> for Address {
    fn from(val: u64) -> Self {
        Address(val)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ReccmpProjectYaml {
    pub targets: BTreeMap<String, ReccmpProjectTarget>,
}

// Thats a lot of dead code
#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
pub struct ReccmpProjectTarget {
    pub filename: String,
    pub source_root: PathBuf,
    pub data_sources: Option<Vec<String>>,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
pub struct ReccmpBuildYaml {
    pub project: PathBuf,
    pub targets: BTreeMap<String, ReccmpBuildTarget>,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
pub struct ReccmpBuildTarget {
    pub path: PathBuf,
    pub pdb: PathBuf,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ReccmpReportJson {
    pub file: String,
    pub format: u32,
    pub timestamp: f64,
    pub data: Vec<ReccmpReportData>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ReccmpReportData {
    pub address: Address,

    pub name: String,
    pub matching: f64,

    pub recomp: Address,

    #[serde(default)]
    pub effective: bool,

    #[serde(default)]
    pub stub: bool,

    pub diff: Option<Vec<(String, Vec<ReccmpReportDiff>)>>,

    #[serde(rename = "type")]
    pub type_: ReccmpReportType,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum ReccmpReportDiff {
    Both {
        both: Vec<ReccmpReportBothDiff>,
    },
    Changed {
        orig: Vec<ReccmpReportChangedDiff>,
        recomp: Vec<ReccmpReportChangedDiff>,
    },
}

impl ReccmpReportDiff {
    pub fn last_orig_code_address(&self) -> Option<Address> {
        match self {
            Self::Both { both } => both.iter().rev().find_map(|e| e.orig),
            Self::Changed { orig, .. } => orig.iter().rev().find_map(|e| e.address),
        }
    }

    pub fn last_recomp_code_address(&self) -> Option<Address> {
        match self {
            Self::Both { both } => both.iter().rev().find_map(|e| e.recomp),
            Self::Changed { recomp, .. } => recomp.iter().rev().find_map(|e| e.address),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(from = "(String, String, String)")]
pub struct ReccmpReportBothDiff {
    pub orig: Option<Address>,
    pub tag: String,
    pub asm: String,
    pub recomp: Option<Address>,
}

impl From<(String, String, String)> for ReccmpReportBothDiff {
    fn from((orig, asm, recomp): (String, String, String)) -> Self {
        Self {
            orig: orig.parse().ok(),
            tag: orig,
            asm,
            recomp: recomp.parse().ok(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(from = "(String, String)")]
pub struct ReccmpReportChangedDiff {
    pub address: Option<Address>,
    pub tag: String,
    pub asm: String,
}

impl From<(String, String)> for ReccmpReportChangedDiff {
    fn from((address, asm): (String, String)) -> Self {
        Self {
            address: address.parse().ok(),
            tag: address,
            asm,
        }
    }
}

pub fn get_comment_from_diff(
    asm: &str,
    address: Address,
    disasm: &[Instruction],
) -> Option<(usize, String)> {
    if let Some((_, comment)) = asm.split_once('\t') {
        let comment = comment.trim().to_owned();
        if let Some(idx) = disasm.iter().position(|i| i.address == Some(address)) {
            return Some((idx, comment));
        }
    }

    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize_repr)]
#[repr(u8)]
pub enum ReccmpReportType {
    // There are more EntityTypes than these, but these are the only ones actually output
    // by reccmp-reccmp. Probably.
    Function = 1,
    Vtable = 5,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ReccmpUserYaml {
    pub targets: BTreeMap<String, ReccmpUserTarget>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ReccmpUserTarget {
    // Sadly, these are not guaranteed to be absolute.
    pub path: PathBuf,
}
