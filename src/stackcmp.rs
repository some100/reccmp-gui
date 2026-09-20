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
    pub is_continuation: bool,
    pub orig_repeated: bool,
    pub recomp_repeated: bool,
}

impl StackcmpRow {
    fn new(row: &str) -> Vec<Self> {
        let mut chars = row.chars();
        let Some(first_char) = chars.next() else {
            return Vec::new();
        };
        let remainder = chars.as_str().trim_start();

        let Some((orig_str, recomp_str)) = remainder.split_once(':') else {
            return Vec::new();
        };

        let status = StackcmpStatus::from_char(first_char);
        let orig_vars = Self::parse_stack_side(orig_str);
        let recomp_vars = Self::parse_stack_side(recomp_str);

        let max_len = orig_vars.len().max(recomp_vars.len());
        if max_len == 0 {
            return Vec::new();
        }

        let mut rows = Vec::with_capacity(max_len);
        for i in 0..max_len {
            let (orig, orig_repeated) = if i < orig_vars.len() {
                (orig_vars[i].clone(), false)
            } else {
                (orig_vars.last().cloned().unwrap_or_default(), true)
            };

            let (recomp, recomp_repeated) = if i < recomp_vars.len() {
                (recomp_vars[i].clone(), false)
            } else {
                (recomp_vars.last().cloned().unwrap_or_default(), true)
            };

            rows.push(Self {
                status,
                orig,
                recomp,
                is_continuation: i > 0,
                orig_repeated,
                recomp_repeated,
            });
        }

        rows
    }

    pub fn name(&self) -> &str {
        self.orig
            .name
            .as_deref()
            .or(self.recomp.name.as_deref())
            .unwrap_or("-")
    }

    fn parse_stack_side(s: &str) -> Vec<StackVariable> {
        let trimmed = s.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let inner = &trimmed[1..trimmed.len() - 1];
            let mut items = Vec::new();
            let mut in_quote = false;
            let mut quote_char = ' ';
            let mut escaped = false;
            let mut current = String::new();

            for c in inner.chars() {
                if escaped {
                    current.push(c);
                    escaped = false;
                    continue;
                }
                if c == '\\' {
                    escaped = true;
                    continue;
                }

                if in_quote {
                    if c == quote_char {
                        in_quote = false;
                        let item = current.trim();
                        if !item.is_empty() {
                            items.push(item.to_string());
                        }
                        current.clear();
                    } else {
                        current.push(c);
                    }
                } else if c == '\'' || c == '"' {
                    in_quote = true;
                    quote_char = c;
                } else if c == ',' {
                    let item = current.trim();
                    if !item.is_empty() {
                        items.push(item.to_string());
                        current.clear();
                    }
                } else {
                    current.push(c);
                }
            }

            let item = current.trim();
            if !item.is_empty() {
                items.push(item.to_string());
            }

            items
                .into_iter()
                .map(|item| StackVariable::new(&item))
                .collect()
        } else {
            vec![StackVariable::new(trimmed)]
        }
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

            let rows = StackcmpRow::new(trimmed);
            match current_section {
                Section::Orig => report.ordered_by_orig.extend(rows),
                Section::Recomp => report.ordered_by_recomp.extend(rows),
                _ => {}
            }
        }

        report
    }
}
