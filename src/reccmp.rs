use core::fmt;
use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Deserializer};
use serde_repr::Deserialize_repr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Address(pub u64);

impl Address {
    pub fn from_str_opt(s: &str) -> Option<Self> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return None;
        }
        let stripped = trimmed.strip_prefix("0x").unwrap_or(trimmed);
        u64::from_str_radix(stripped, 16).ok().map(Address)
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
#[derive(Clone, Debug, Deserialize)]
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
    pub fn is_table(&self) -> bool {
        match self {
            Self::Both { both } => both.iter().any(|e| e.asm.contains("table:")),
            Self::Changed { orig, recomp } => {
                orig.iter().any(|e| e.asm.contains("table:"))
                    || recomp.iter().any(|e| e.asm.contains("table:"))
            }
        }
    }

    pub fn last_orig_address(&self) -> Option<Address> {
        match self {
            Self::Both { both } => both.iter().rev().find_map(|x| x.orig),
            Self::Changed { orig, .. } => orig.iter().rev().find_map(|x| x.address),
        }
    }

    pub fn last_recomp_address(&self) -> Option<Address> {
        match self {
            Self::Both { both } => both.iter().rev().find_map(|x| x.recomp),
            Self::Changed { recomp, .. } => recomp.iter().rev().find_map(|x| x.address),
        }
    }

    pub fn last_orig_code_address(&self) -> Option<Address> {
        match self {
            Self::Both { both } => {
                let mut last = None;
                for entry in both {
                    if entry.asm.contains("table:") {
                        break;
                    }
                    if let Some(addr) = entry.orig {
                        last = Some(addr);
                    }
                }
                last
            }
            Self::Changed { orig, .. } => {
                let mut last = None;
                for entry in orig {
                    if entry.asm.contains("table:") {
                        break;
                    }
                    if let Some(addr) = entry.address {
                        last = Some(addr);
                    }
                }
                last
            }
        }
    }

    pub fn last_recomp_code_address(&self) -> Option<Address> {
        match self {
            Self::Both { both } => {
                let mut last = None;
                for entry in both {
                    if entry.asm.contains("table:") {
                        break;
                    }
                    if let Some(addr) = entry.recomp {
                        last = Some(addr);
                    }
                }
                last
            }
            Self::Changed { recomp, .. } => {
                let mut last = None;
                for entry in recomp {
                    if entry.asm.contains("table:") {
                        break;
                    }
                    if let Some(addr) = entry.address {
                        last = Some(addr);
                    }
                }
                last
            }
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
            orig: Address::from_str_opt(&orig),
            tag: orig,
            asm,
            recomp: Address::from_str_opt(&recomp),
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
            address: Address::from_str_opt(&address),
            tag: address,
            asm,
        }
    }
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
