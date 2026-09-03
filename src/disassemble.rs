use std::{
    collections::{HashMap, btree_map::BTreeMap},
    sync::Arc,
};

use iced_x86::{
    FlowControl, Formatter, IntelFormatter, MemorySizeOptions, SymbolResolver, SymbolResult,
};
use pelite::pe32::{Pe, PeFile};
use thiserror::Error;

use crate::{
    app::config::DisassemblySettings,
    disassemble::{
        diff::{DiffContext, DiffRow},
        instgen::InstructGen,
        table::{FunctionTables, TablesDiff},
    },
    reccmp::{Address, ReccmpReportData, ReccmpReportDiff, get_comment_from_diff},
    roadmap::RoadmapRow,
};

pub mod diff;
mod instgen;
pub mod table;

#[derive(Clone, Debug, Default)]
pub struct Function {
    pub instructions: Vec<Instruction>,
    pub tables: FunctionTables,
}

#[derive(Clone, Debug)]
pub struct RangeSymbol {
    pub name: String,
    pub size: u64,
}

#[derive(Clone, Debug, Default)]
pub struct SymbolMap {
    pub exact: HashMap<Address, String>,
    pub ranges: BTreeMap<Address, RangeSymbol>,
}

impl SymbolMap {
    pub fn get(&self, address: Address, settings: DisassemblySettings) -> Option<String> {
        if let Some(name) = self.exact.get(&address) {
            return Some(name.clone());
        }

        if settings.resolve_global_offsets
            && let Some((&start_addr, sym)) = self.ranges.range(..=address).next_back()
        {
            let offset = address.0 - start_addr.0;
            if offset > 0 && offset < sym.size {
                if settings.display_global_offsets_hex {
                    return Some(format!("{}+{:#x} (OFFSET)", sym.name, offset));
                }

                return Some(format!("{}+{offset} (OFFSET)", sym.name));
            }
        }

        None
    }
}

#[derive(Error, Debug)]
pub enum DisassembleError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("pelite error from {bin_type:?}: {source}")]
    Pelite {
        bin_type: BinaryType,
        #[source]
        source: pelite::Error,
    },
}

#[derive(Clone, Copy, Debug)]
pub enum BinaryType {
    Orig,
    Recomp,
}

#[derive(Clone, Debug)]
struct DisassemblySymbolResolver {
    map: Arc<SymbolMap>,
    settings: DisassemblySettings,
}

impl DisassemblySymbolResolver {
    fn new(map: Arc<SymbolMap>, settings: DisassemblySettings) -> Self {
        Self { map, settings }
    }
}

// poor man's reccmp
impl SymbolResolver for DisassemblySymbolResolver {
    fn symbol(
        &mut self,
        instruction: &iced_x86::Instruction,
        _operand: u32,
        _instruction_operand: Option<u32>,
        address: u64,
        _address_size: u32,
    ) -> Option<SymbolResult<'_>> {
        // me thinks this should be returning Option, but whatever
        let target = instruction.near_branch_target();
        if target != 0 {
            if let Some(func_name) = self.map.get(Address(target), self.settings)
                && self.settings.resolve_symbols
            {
                // this is a call instr
                Some(SymbolResult::with_string(target, func_name))
            } else if matches!(
                instruction.flow_control(),
                FlowControl::UnconditionalBranch
                    | FlowControl::IndirectBranch
                    | FlowControl::ConditionalBranch
            ) && self.settings.relative_jump
            {
                // this is a jump instr, so convert it to relative
                let ip = instruction.ip();

                let rel_str = if target >= ip {
                    format!("{:#x}", target - ip)
                } else {
                    format!("-{:#x}", ip - target)
                };

                Some(SymbolResult::with_string(target, rel_str))
            } else {
                None
            }
        } else if address != 0 && self.settings.resolve_symbols {
            self.map
                .get(Address(address), self.settings)
                .map(|name| SymbolResult::with_string(address, name))
        } else {
            None
        }
    }
}

pub struct Disassembly {
    pub func_name: String,
    pub rows: Vec<DiffRow>,
    pub tables: TablesDiff,
    pub matching: f64,
    pub focus: bool,
}

pub struct Disassembler {
    orig_formatter: IntelFormatter,
    recomp_formatter: IntelFormatter,
    pub orig_map: Arc<SymbolMap>,
    pub recomp_map: Arc<SymbolMap>,
    report_data: Vec<ReccmpReportData>,
    roadmap_rows: Option<Vec<RoadmapRow>>,
    settings: DisassemblySettings,
}

impl Disassembler {
    pub fn new() -> Self {
        Self {
            orig_formatter: IntelFormatter::new(),
            recomp_formatter: IntelFormatter::new(),
            orig_map: Arc::new(SymbolMap::default()),
            recomp_map: Arc::new(SymbolMap::default()),
            report_data: Vec::new(),
            roadmap_rows: None,
            settings: DisassemblySettings::default(),
        }
    }

    pub fn disasm(
        &mut self,
        bytes: &[u8],
        address: Address,
        max_known_address: Option<Address>,
        bin_type: BinaryType,
    ) -> Result<Function, DisassembleError> {
        let pe = PeFile::from_bytes(bytes).map_err(|e| DisassembleError::Pelite {
            bin_type,
            source: e,
        })?;

        let start = address.0;

        let roadmap_size = if self.settings.use_roadmap && self.settings.use_roadmap_func_sizes {
            self.roadmap_rows.as_ref().and_then(|rows| {
                rows.iter()
                    .find(|r| {
                        let r_addr = match bin_type {
                            BinaryType::Orig => r.orig_addr,
                            BinaryType::Recomp => r.recomp_addr,
                        };
                        r_addr == Some(address)
                    })
                    .and_then(RoadmapRow::size)
            })
        } else {
            None
        };

        let next_func_addr = self
            .report_data
            .iter()
            .filter_map(|d| {
                let a = match bin_type {
                    BinaryType::Orig => d.address.0,
                    BinaryType::Recomp => d.recomp.0,
                };
                if a > start { Some(a) } else { None }
            })
            .min();

        let image_base = u64::from(pe.optional_header().ImageBase);
        let section_end = pe.section_headers().iter().find_map(|s| {
            let s_start = image_base + u64::from(s.VirtualAddress);
            let s_end = s_start + u64::from(s.VirtualSize);
            if start >= s_start && start < s_end {
                Some(s_end)
            } else {
                None
            }
        });

        let mut end = if let Some(size) = roadmap_size {
            start + size
        } else if let Some(next) = next_func_addr {
            next
        } else if let Some(sec_end) = section_end {
            sec_end
        } else {
            start + 0x2000
        };

        if let Some(max_known) = max_known_address {
            end = end.max(max_known.0 + 16);
        }

        let rva = pe
            .va_to_rva(start as u32)
            .map_err(|e| DisassembleError::Pelite {
                bin_type,
                source: e,
            })?;
        let offset = pe
            .rva_to_file_offset(rva)
            .map_err(|e| DisassembleError::Pelite {
                bin_type,
                source: e,
            })?;
        let max_len = ((end - start) as usize).min(bytes.len().saturating_sub(offset));
        let blob = &bytes[offset..offset + max_len];

        let formatter = match bin_type {
            BinaryType::Orig => &mut self.orig_formatter,
            BinaryType::Recomp => &mut self.recomp_formatter,
        };

        Ok(
            InstructGen::new(blob, start, roadmap_size.is_none(), max_known_address)
                .analyze(formatter),
        )
    }

    pub fn diff(
        &self,
        orig_fn: &Function,
        mut recomp_fn: Function,
        hunks: &[ReccmpReportDiff],
    ) -> (Vec<DiffRow>, TablesDiff) {
        for hunk in hunks {
            match hunk {
                ReccmpReportDiff::Both { both } => {
                    for entry in both {
                        if let Some(addr) = entry.recomp
                            && let Some((idx, comment)) =
                                get_comment_from_diff(&entry.asm, addr, &recomp_fn.instructions)
                        {
                            recomp_fn.instructions[idx].comment = Some(comment);
                        }
                    }
                }
                ReccmpReportDiff::Changed { recomp, .. } => {
                    for entry in recomp {
                        if let Some(addr) = entry.address
                            && let Some((idx, comment)) =
                                get_comment_from_diff(&entry.asm, addr, &recomp_fn.instructions)
                        {
                            recomp_fn.instructions[idx].comment = Some(comment);
                        }
                    }
                }
            }
        }

        let tables_diff = TablesDiff::from_tables(&orig_fn.tables, &recomp_fn.tables);
        let ctx = DiffContext {
            orig_symbols: &self.orig_map,
            recomp_symbols: &self.recomp_map,
            settings: self.settings,
            tables: &tables_diff,
        };
        let rows = DiffRow::from_hunks(&orig_fn.instructions, &recomp_fn.instructions, hunks, &ctx);

        (rows, tables_diff)
    }

    pub fn rebuild_resolvers(&mut self) {
        let roadmap = if self.settings.use_roadmap && self.settings.resolve_symbols {
            self.roadmap_rows.take() // I love the borrow checkr
        } else {
            None
        };

        self.orig_map = Arc::new(Self::build_symbol_map(
            &self.report_data,
            roadmap.as_deref(),
            BinaryType::Orig,
        ));
        self.orig_formatter = IntelFormatter::with_options(
            Some(Box::new(DisassemblySymbolResolver::new(
                self.orig_map.clone(),
                self.settings,
            ))),
            None,
        );
        self.set_formatter_settings(BinaryType::Orig);

        self.recomp_map = Arc::new(Self::build_symbol_map(
            &self.report_data,
            roadmap.as_deref(),
            BinaryType::Recomp,
        ));
        self.recomp_formatter = IntelFormatter::with_options(
            Some(Box::new(DisassemblySymbolResolver::new(
                self.recomp_map.clone(),
                self.settings,
            ))),
            None,
        );
        self.set_formatter_settings(BinaryType::Recomp);

        if let Some(rows) = roadmap {
            self.roadmap_rows = Some(rows);
        }
    }

    pub fn update_resolvers(&mut self, data: Vec<ReccmpReportData>) {
        self.report_data = data;
        self.rebuild_resolvers();
    }

    pub fn update_roadmap(&mut self, rows: Option<Vec<RoadmapRow>>) {
        self.roadmap_rows = rows;
        self.rebuild_resolvers();
    }

    pub fn update_settings(&mut self, settings: DisassemblySettings) {
        self.settings = settings;
        self.rebuild_resolvers();
    }

    fn build_symbol_map(
        data: &[ReccmpReportData],
        roadmap_rows: Option<&[RoadmapRow]>,
        bin_type: BinaryType,
    ) -> SymbolMap {
        let mut map = SymbolMap::default();

        if let Some(rows) = roadmap_rows {
            for row in rows {
                let addr = match bin_type {
                    BinaryType::Orig => row.orig_addr,
                    BinaryType::Recomp => row.recomp_addr,
                };
                if let Some(addr) = addr
                    && !row.name.is_empty()
                {
                    map.exact.insert(
                        addr,
                        format!("{} ({})", row.name, row.row_type.as_disasm_str()),
                    );

                    if row.row_type() == crate::roadmap::RoadmapRowType::Data
                        && let Some(size) = row.size()
                        && size > 0
                    {
                        map.ranges.insert(
                            addr,
                            RangeSymbol {
                                name: row.name.clone(),
                                size,
                            },
                        );
                    }
                }
            }
        }

        for d in data {
            let addr = match bin_type {
                BinaryType::Orig => d.address,
                BinaryType::Recomp => d.recomp,
            };
            map.exact.insert(addr, format!("{} (FUNCTION)", d.name));
        }

        map
    }

    fn set_formatter_settings(&mut self, bin_type: BinaryType) {
        // Also see https://github.com/isledecomp/reccmp/issues/175
        let formatter = match bin_type {
            BinaryType::Orig => &mut self.orig_formatter,
            BinaryType::Recomp => &mut self.recomp_formatter,
        };
        formatter.options_mut().set_hex_prefix("0x");
        formatter.options_mut().set_hex_suffix("");
        formatter.options_mut().set_uppercase_hex(false);
        formatter.options_mut().set_show_branch_size(false);
        formatter.options_mut().set_prefer_st0(true);
        formatter
            .options_mut()
            .set_memory_size_options(MemorySizeOptions::Always);
        formatter
            .options_mut()
            .set_space_after_operand_separator(true);
        formatter
            .options_mut()
            .set_space_between_memory_add_operators(true);
        formatter
            .options_mut()
            .set_space_between_memory_mul_operators(false);
    }
}

#[derive(Clone, Debug)]
pub struct Instruction {
    pub address: Option<Address>,
    pub mnemonic: String,
    pub operands: Vec<String>,
    pub comment: Option<String>,
    pub address_str: String,
    pub raw: Option<iced_x86::Instruction>,
}
