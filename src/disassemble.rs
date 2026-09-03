use std::{collections::HashMap, sync::Arc};

use iced_x86::{
    Code, Decoder, DecoderOptions, FlowControl, Formatter, IntelFormatter, MemorySizeOptions,
    SymbolResolver, SymbolResult,
};
use pelite::pe32::{Pe, PeFile};
use thiserror::Error;

use crate::{
    disassemble::diff::{DiffContext, DiffRow},
    reccmp::{Address, ReccmpReportData, ReccmpReportDiff},
    roadmap::RoadmapRow,
};

pub mod diff;

pub type SymbolMap = HashMap<Address, String>;

#[derive(Error, Debug)]
pub enum DisassembleError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("pelite error: {0}")]
    Pelite(#[from] pelite::Error),
}

#[derive(Clone, Copy, Debug)]
pub enum BinaryType {
    Orig,
    Recomp,
}

#[derive(Clone, Debug)]
struct DisassemblySymbolResolver {
    map: Arc<SymbolMap>,
}

impl DisassemblySymbolResolver {
    fn new(map: Arc<SymbolMap>) -> Self {
        Self { map }
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
            if let Some(func_name) = self.map.get(&Address(target)) {
                // this is a call instr
                Some(SymbolResult::with_string(target, func_name.clone()))
            } else if matches!(
                instruction.flow_control(),
                FlowControl::UnconditionalBranch
                    | FlowControl::IndirectBranch
                    | FlowControl::ConditionalBranch
            ) {
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
        } else if address != 0 {
            self.map
                .get(&Address(address))
                .map(|name| SymbolResult::with_string(address, name.clone()))
        } else {
            None
        }
    }
}

pub struct Disassembly {
    pub func_name: String,
    pub rows: Vec<DiffRow>,
    pub focus: bool,
}

pub struct Disassembler {
    orig_formatter: IntelFormatter,
    recomp_formatter: IntelFormatter,
    pub orig_map: Arc<SymbolMap>,
    pub recomp_map: Arc<SymbolMap>,
    report_data: Vec<ReccmpReportData>,
    roadmap_rows: Option<Vec<RoadmapRow>>,
}

impl Disassembler {
    pub fn new() -> Self {
        Self {
            orig_formatter: IntelFormatter::new(),
            recomp_formatter: IntelFormatter::new(),
            orig_map: Arc::new(HashMap::new()),
            recomp_map: Arc::new(HashMap::new()),
            report_data: Vec::new(),
            roadmap_rows: None,
        }
    }

    pub fn disasm(
        &mut self,
        bytes: &[u8],
        address: Address,
        max_known_address: Option<Address>,
        bin_type: BinaryType,
    ) -> Result<Vec<Instruction>, DisassembleError> {
        let pe = PeFile::from_bytes(bytes)?;
        let offset = pe.rva_to_file_offset(pe.va_to_rva(address.0 as u32)?)?;

        // reccmp only supports 32-bit x86 anyways
        let mut decoder = Decoder::with_ip(32, &bytes[offset..], address.0, DecoderOptions::NONE);

        let mut instructions = Vec::new();
        let mut instr = iced_x86::Instruction::default();

        while decoder.can_decode() {
            decoder.decode_out(&mut instr);

            let instr_addr = instr.ip();

            // If we find padding then we are probably already at the end of a function
            //
            // Incidentally, I tried looking to see if there was a better way to solve this problem
            // in reccmp's source code. However they kind of just do the exact same thing here.
            // So this is how it's gonna be unless they (reccmp) find a better way and expose it
            // in their json
            if instr.code() == Code::Int3
                && max_known_address.is_none_or(|addr| instr_addr >= addr.0)
            {
                break;
            }

            let formatter = match bin_type {
                BinaryType::Orig => &mut self.orig_formatter,
                BinaryType::Recomp => &mut self.recomp_formatter,
            };

            let mut mnemonic = String::new();
            formatter.format_mnemonic(&instr, &mut mnemonic);

            let op_count = formatter.operand_count(&instr);
            let mut operands = Vec::with_capacity(op_count as usize);
            for i in 0..op_count {
                let mut op = String::new();
                if formatter.format_operand(&instr, &mut op, i).is_err() {
                    break;
                }
                operands.push(op);
            }

            instructions.push(Instruction {
                address: Some(instr_addr.into()),
                mnemonic,
                operands,
                comment: None,
                address_str: format!("0x{instr_addr:08x}"),
                raw: Some(instr),
            });

            // It's at least far enough that it doesn't matter for diffing purposes anyways
            //
            // MSVC doesn't even do early returns by ret, but rather merges every return path at
            // the end (even on debug). So on the compiler you're gonna be using reccmp with
            // anyways, this does not matter.
            if matches!(instr.flow_control(), FlowControl::Return)
                && max_known_address.is_none_or(|addr| instr_addr >= addr.0)
            {
                break;
            }
        }
        Ok(instructions)
    }

    pub fn diff(
        &self,
        orig_disasm: &[Instruction],
        recomp_disasm: &[Instruction],
        hunks: &[ReccmpReportDiff],
    ) -> Vec<DiffRow> {
        let ctx = DiffContext {
            orig_symbols: &self.orig_map,
            recomp_symbols: &self.recomp_map,
        };
        DiffRow::from_hunks(orig_disasm, recomp_disasm, hunks, &ctx)
    }

    pub fn rebuild_resolvers(&mut self) {
        self.orig_map = Arc::new(Self::build_symbol_map(
            &self.report_data,
            self.roadmap_rows.as_deref(),
            BinaryType::Orig,
        ));
        self.orig_formatter = IntelFormatter::with_options(
            Some(Box::new(DisassemblySymbolResolver::new(
                self.orig_map.clone(),
            ))),
            None,
        );
        self.set_formatter_settings(BinaryType::Orig);

        self.recomp_map = Arc::new(Self::build_symbol_map(
            &self.report_data,
            self.roadmap_rows.as_deref(),
            BinaryType::Recomp,
        ));
        self.recomp_formatter = IntelFormatter::with_options(
            Some(Box::new(DisassemblySymbolResolver::new(
                self.recomp_map.clone(),
            ))),
            None,
        );
        self.set_formatter_settings(BinaryType::Recomp);
    }

    pub fn update_resolvers(&mut self, data: Vec<ReccmpReportData>) {
        self.report_data = data;
        self.rebuild_resolvers();
    }

    pub fn update_roadmap(&mut self, rows: Option<Vec<RoadmapRow>>) {
        self.roadmap_rows = rows;
        self.rebuild_resolvers();
    }

    fn build_symbol_map(
        data: &[ReccmpReportData],
        roadmap_rows: Option<&[RoadmapRow]>,
        bin_type: BinaryType,
    ) -> SymbolMap {
        let mut map = HashMap::new();

        if let Some(rows) = roadmap_rows {
            for row in rows {
                let addr = match bin_type {
                    BinaryType::Orig => row.orig_addr,
                    BinaryType::Recomp => row.recomp_addr,
                };
                if let Some(addr) = addr
                    && !row.name.is_empty()
                {
                    map.insert(
                        addr,
                        format!("{} ({})", row.name, row.row_type.as_disasm_str()),
                    );
                }
            }
        }

        for d in data {
            let addr = match bin_type {
                BinaryType::Orig => d.address,
                BinaryType::Recomp => d.recomp,
            };
            map.insert(addr, format!("{} (FUNCTION)", d.name));
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

    #[allow(dead_code)]
    pub raw: Option<iced_x86::Instruction>, // Could be useful
}

impl Instruction {
    fn from_reccmp(address: Option<Address>, asm: &str) -> Self {
        let (asm, comment) = if let Some((asm, comment)) = asm.split_once('\t') {
            (asm.trim(), Some(comment.trim().to_owned()))
        } else {
            (asm.trim(), None)
        };

        let address_str = address.map(|a| a.to_string()).unwrap_or_default();

        if asm == "Jump table:" || asm == "Data table:" {
            return Self {
                address,
                mnemonic: asm.to_owned(),
                operands: Vec::new(),
                comment,
                address_str,
                raw: None,
            };
        }

        // then this is probably jump table (if it begins with start) or data table entry (if its just a hex)
        if asm.starts_with("start") || asm.starts_with("0x") {
            return Self {
                address,
                mnemonic: asm.to_owned(),
                operands: Vec::new(),
                comment,
                address_str,
                raw: None,
            };
        }

        let trimmed = asm.trim();
        let (mnemonic, op_str) = if let Some(rest) = trimmed.strip_prefix("rep ") {
            if let Some((sub_mnemonic, ops)) = rest.split_once(' ') {
                (format!("rep {sub_mnemonic}"), ops.trim())
            } else {
                (format!("rep {rest}"), "")
            }
        } else if let Some((m, ops)) = trimmed.split_once(' ') {
            (m.trim().to_string(), ops.trim())
        } else {
            (trimmed.to_string(), "")
        };

        Self {
            address,
            mnemonic: mnemonic.to_owned(),
            operands: split_operands(op_str),
            comment,
            address_str: address.map(|addr| addr.to_string()).unwrap_or_default(),
            raw: None,
        }
    }
}

fn split_operands(op_str: &str) -> Vec<String> {
    if op_str.is_empty() {
        Vec::new()
    } else {
        op_str.split(",").map(str::trim).map(String::from).collect()
    }
}
