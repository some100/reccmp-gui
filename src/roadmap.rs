use std::path::Path;

use serde::{Deserialize, Deserializer};

use crate::{disassemble::diff::DiffKind, reccmp::Address};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub enum RoadmapRowType {
    #[serde(rename = "fun")]
    Function,
    #[serde(rename = "dat")]
    Data,
    #[serde(rename = "str")]
    String,
    #[serde(rename = "vta")]
    Vtable,
    #[serde(rename = "imp")]
    Import,
    #[serde(rename = "lab")]
    Label,
    #[serde(rename = "flo")]
    Float,
    #[serde(rename = "wid")]
    Widechar,
    #[serde(other)]
    Unknown,
}

impl RoadmapRowType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Function => "FUN",
            Self::Data => "DAT",
            Self::String => "STR",
            Self::Vtable => "VTA",
            Self::Import => "IMP",
            Self::Label => "LAB",
            Self::Float => "FLO",
            Self::Widechar => "WID",
            Self::Unknown => "???",
        }
    }

    pub fn as_disasm_str(self) -> &'static str {
        match self {
            Self::Function => "FUNCTION",
            Self::Data => "DATA",
            Self::String => "STRING",
            Self::Vtable => "VTABLE",
            Self::Import => "IMPORT",
            Self::Label => "LABEL",
            Self::Float => "FLOAT",
            Self::Widechar => "WIDECHAR",
            Self::Unknown => "UNKNOWN",
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
pub struct RoadmapRow {
    pub orig_sects_of: Option<String>,
    pub recomp_sects_of: Option<String>,
    pub orig_addr: Option<Address>,
    pub recomp_addr: Option<Address>,
    #[serde(deserialize_with = "deserialize_displacement")]
    pub displacement: Option<i64>,
    pub row_type: RoadmapRowType,
    pub size: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub module: String,
}

impl RoadmapRow {
    pub fn from_path(csv_path: &Path) -> csv::Result<Vec<Self>> {
        let reader = csv::ReaderBuilder::new()
            .flexible(true)
            .from_path(csv_path)?;
        let mut rows = Vec::new();
        for result in reader.into_deserialize() {
            let record: Self = result?;
            rows.push(record);
        }

        Ok(rows)
    }

    pub fn diff_kind(&self) -> DiffKind {
        match (self.orig_addr.is_some(), self.recomp_addr.is_some()) {
            (true, false) => DiffKind::Removed,
            (false, true) => DiffKind::Added,
            (true, true) | (false, false) => {
                if self.displacement == Some(0) {
                    DiffKind::Matched
                } else {
                    DiffKind::Diff
                }
            }
        }
    }

    pub fn row_type(&self) -> RoadmapRowType {
        self.row_type
    }
}

fn deserialize_displacement<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt = Option::<String>::deserialize(deserializer)?;
    let Some(s) = opt else { return Ok(None) };
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let (is_negative, rest) = if let Some(stripped) = trimmed.strip_prefix('-') {
        (true, stripped)
    } else if let Some(stripped) = trimmed.strip_prefix('+') {
        (false, stripped)
    } else {
        (false, trimmed)
    };

    let hex_str = rest.strip_prefix("0x").unwrap_or(rest);
    match i64::from_str_radix(hex_str, 16) {
        Ok(val) => Ok(Some(if is_negative { -val } else { val })),
        Err(_) => Ok(None),
    }
}
