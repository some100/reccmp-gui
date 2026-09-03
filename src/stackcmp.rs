use crate::reccmp::Address;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackcmpStatus {
    Matched,
    // This stack variable matches 1:1, but the order of variables is not correct.
    Mismatch,
    // This stack variable matches multiple variables in the other binary.
    Conflict,
    // This stack variable did not appear in the diff.
    // It either matches or only appears in structural mismatches.
    Unknown,
}

impl StackcmpStatus {
    pub fn from_char(c: char) -> Self {
        match c {
            '✓' => Self::Matched,
            '⇄' => Self::Mismatch,
            '✗' => Self::Conflict,
            _ => Self::Unknown,
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum Section {
    None,
    Orig,
    Recomp,
    Legend,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StackVariable {
    pub raw: String,
    pub base_reg: String,
    pub offset: i64,
    pub name: Option<String>,
}

impl StackVariable {
    pub fn new(s: &str) -> Self {
        let raw = s.trim().to_string();
        let tokens: Vec<&str> = raw.split_whitespace().collect();
        let mut variable = Self::default();

        if tokens.is_empty() {
            return variable;
        }

        // for example, ebp - 0x08
        if tokens.len() >= 3 && (tokens[1] == "+" || tokens[1] == "-") {
            variable.base_reg = tokens[0].to_string();

            let sign = if tokens[1] == "-" { -1i64 } else { 1i64 };
            let hex = tokens[2].strip_prefix("0x").unwrap_or(tokens[2]);

            if let Ok(val) = i64::from_str_radix(hex, 16) {
                variable.offset = sign * val;
            }

            if tokens.len() > 3 {
                variable.name = Some(tokens[3..].join(" "));
            }
        }

        variable
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StackcmpRow {
    pub status: StackcmpStatus,
    pub orig: StackVariable,
    pub recomp: StackVariable,
}

impl StackcmpRow {
    fn new(row: &str) -> Option<Self> {
        let mut chars = row.chars();
        let first_char = chars.next()?;
        let remainder = chars.as_str().trim_start();

        let (orig_str, recomp_str) = remainder.split_once(':')?;

        let status = StackcmpStatus::from_char(first_char);
        let orig = StackVariable::new(orig_str);
        let recomp = StackVariable::new(recomp_str);

        Some(Self {
            status,
            orig,
            recomp,
        })
    }

    pub fn name(&self) -> &str {
        self.orig
            .name
            .as_deref()
            .or(self.recomp.name.as_deref())
            .unwrap_or("-")
    }
}

#[derive(Clone, Debug)]
pub struct StackcmpReport {
    pub ordered_by_orig: Vec<StackcmpRow>,
    pub ordered_by_recomp: Vec<StackcmpRow>,
    pub address: Address,
    pub func_name: String,
}

impl StackcmpReport {
    pub fn new(raw_output: &str, address: Address, func_name: String) -> Self {
        let clean = strip_ansi_escapes::strip_str(raw_output);
        let mut report = Self {
            ordered_by_orig: Vec::new(),
            ordered_by_recomp: Vec::new(),
            address,
            func_name,
        };
        let mut current_section = Section::None;

        for line in clean.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            if trimmed.starts_with("Ordered by original stack") {
                current_section = Section::Orig;
                continue;
            } else if trimmed.starts_with("Ordered by recomp stack") {
                current_section = Section::Recomp;
                continue;
            } else if trimmed.starts_with("Legend:") {
                current_section = Section::Legend;
                continue;
            }

            if current_section == Section::Legend || current_section == Section::None {
                continue;
            }

            if let Some(row) = StackcmpRow::new(trimmed) {
                match current_section {
                    Section::Orig => report.ordered_by_orig.push(row),
                    Section::Recomp => report.ordered_by_recomp.push(row),
                    _ => {}
                }
            }
        }

        report
    }
}
