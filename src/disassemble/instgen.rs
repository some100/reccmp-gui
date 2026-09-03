//! See <https://github.com/isledecomp/reccmp/blob/master/reccmp/compare/asm/instgen.py> since this is based off that.

use iced_x86::{
    Code, Decoder, DecoderOptions, FlowControl, Formatter, IntelFormatter, Mnemonic, OpKind,
    Register,
};
use std::collections::BTreeMap;

use crate::{
    disassemble::{
        Function, Instruction,
        table::{DataTable, DataTableEntry, FunctionTables, JumpTable, JumpTableEntry},
    },
    reccmp::Address,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SectionType {
    Code,
    JumpTab,
    DataTab,
}

pub struct InstructGen<'a> {
    blob: &'a [u8],
    start: u64,
    end: u64,
    cur_addr: u64,
    cur_section_type: SectionType,
    section_end: u64,
    confirmed_addrs: BTreeMap<u64, SectionType>,
    stop_on_ret: bool,
    max_known_address: Option<Address>,
}

impl<'a> InstructGen<'a> {
    pub fn new(
        blob: &'a [u8],
        start: u64,
        stop_on_ret: bool,
        max_known_address: Option<Address>,
    ) -> Self {
        let end = start + blob.len() as u64;
        Self {
            blob,
            start,
            end,
            cur_addr: start,
            cur_section_type: SectionType::Code,
            section_end: end,
            confirmed_addrs: BTreeMap::new(),
            stop_on_ret,
            max_known_address,
        }
    }

    fn insert_confirmed_addr(&mut self, addr: u64, sec_type: SectionType) {
        if addr < self.start || addr >= self.end {
            return;
        }

        self.confirmed_addrs.insert(addr, sec_type);

        if sec_type != self.cur_section_type && addr > self.cur_addr {
            self.section_end = self.section_end.min(addr);
        }
    }

    fn next_section(&mut self, addr: u64) -> Option<SectionType> {
        if addr == self.start {
            self.section_end = self.end;
            self.cur_section_type = SectionType::Code;
            return Some(SectionType::Code);
        }

        let (&actual_addr, &new_type) = self.confirmed_addrs.range(addr..).next()?;
        self.cur_addr = actual_addr;
        self.cur_section_type = new_type;

        let next_bound = self
            .confirmed_addrs
            .range((actual_addr + 1)..)
            .find(|&(_, &conf_type)| {
                self.cur_section_type != SectionType::Code || conf_type != SectionType::Code
            })
            .map(|(&conf_addr, _)| conf_addr);

        self.section_end = next_bound.unwrap_or(self.end);
        Some(new_type)
    }

    pub fn analyze(&mut self, formatter: &mut IntelFormatter) -> Function {
        let mut instructions = Vec::new();
        let mut tables = FunctionTables::default();

        self.cur_addr = self.start;

        while let Some(sect_type) = self.next_section(self.cur_addr) {
            match sect_type {
                SectionType::Code => self.analyze_code_section(formatter, &mut instructions),
                SectionType::JumpTab => self.analyze_jump_table(&mut tables),
                SectionType::DataTab => self.analyze_data_table(&mut tables),
            }
        }

        Function {
            instructions,
            tables,
        }
    }

    fn analyze_code_section(
        &mut self,
        formatter: &mut IntelFormatter,
        instructions: &mut Vec<Instruction>,
    ) {
        let offset = (self.cur_addr - self.start) as usize;
        let mut decoder = Decoder::with_ip(
            32,
            &self.blob[offset..],
            self.cur_addr,
            DecoderOptions::NONE,
        );
        let mut instr = iced_x86::Instruction::default();
        let mut decoded_any = false;

        while decoder.can_decode() {
            decoder.decode_out(&mut instr);
            if instr.ip() >= self.section_end {
                break;
            }

            if instr.code() == Code::Int3
                && self
                    .max_known_address
                    .is_none_or(|addr| instr.ip() >= addr.0)
            {
                break;
            }

            decoded_any = true;
            self.cur_addr = instr.ip();

            match instr.flow_control() {
                FlowControl::UnconditionalBranch
                | FlowControl::ConditionalBranch
                | FlowControl::IndirectBranch => {
                    let target = instr.near_branch_target();
                    if target != 0 {
                        self.insert_confirmed_addr(target, SectionType::Code);
                    } else if let Some(disp) = memory_displacement(&instr) {
                        self.insert_confirmed_addr(disp, SectionType::JumpTab);
                    }
                }
                _ => {
                    if matches!(instr.mnemonic(), Mnemonic::Mov | Mnemonic::Movzx)
                        && instr.op_count() > 1
                        && instr.op_kind(1) == OpKind::Memory
                        && let Some(disp) = memory_displacement_op(&instr, 1)
                    {
                        self.insert_confirmed_addr(disp, SectionType::DataTab);
                    }
                }
            }

            instructions.push(format_code_instruction(&instr, formatter));
            self.cur_addr = instr.ip() + instr.len() as u64;

            if self.stop_on_ret
                && matches!(instr.flow_control(), FlowControl::Return)
                && self
                    .max_known_address
                    .is_none_or(|addr| instr.ip() >= addr.0)
            {
                break;
            }
        }

        if !decoded_any {
            self.cur_addr = self.section_end.max(self.cur_addr + 1);
        }
    }

    fn analyze_jump_table(&mut self, tables: &mut FunctionTables) {
        let mut offset_addr = self.cur_addr;
        let table_start = offset_addr;
        let mut entries = Vec::new();

        while offset_addr + 4 <= self.section_end {
            let offset = (offset_addr - self.start) as usize;
            let Some(bytes) = self.blob.get(offset..offset + 4) else {
                break;
            };
            let target = u64::from(u32::from_le_bytes(bytes.try_into().unwrap()));

            if target >= self.start && target < self.end {
                self.insert_confirmed_addr(target, SectionType::Code);
                entries.push(JumpTableEntry {
                    address: Address(offset_addr),
                    target: Address(target),
                    target_offset: (target as i64) - (self.start as i64),
                });
            } else {
                break;
            }

            offset_addr += 4;
        }

        if !entries.is_empty() {
            tables.jump_tables.push(JumpTable {
                address: Address(table_start),
                entries,
            });
        }

        self.cur_addr = if offset_addr > self.cur_addr {
            offset_addr
        } else if self.section_end > self.cur_addr {
            self.section_end
        } else {
            self.cur_addr + 1
        };
    }

    fn analyze_data_table(&mut self, tables: &mut FunctionTables) {
        let read_size = self.section_end.saturating_sub(self.cur_addr);
        if read_size > 0 {
            let table_start = self.cur_addr;
            let mut entries = Vec::with_capacity(read_size as usize);
            for offset_addr in self.cur_addr..self.cur_addr + read_size {
                let offset = (offset_addr - self.start) as usize;
                let val = self.blob[offset];
                entries.push(DataTableEntry {
                    address: Address(offset_addr),
                    value: val,
                });
            }

            tables.data_tables.push(DataTable {
                address: Address(table_start),
                entries,
            });
        }
        self.cur_addr = if self.section_end > self.cur_addr {
            self.section_end
        } else {
            self.cur_addr + 1
        };
    }
}

fn memory_displacement(instr: &iced_x86::Instruction) -> Option<u64> {
    for op in 0..instr.op_count() {
        if let Some(disp) = memory_displacement_op(instr, op) {
            return Some(disp);
        }
    }
    None
}

fn memory_displacement_op(instr: &iced_x86::Instruction, op: u32) -> Option<u64> {
    if instr.op_kind(op) == OpKind::Memory {
        let has_reg =
            instr.memory_base() != Register::None || instr.memory_index() != Register::None;
        let disp = instr.memory_displacement64();
        if has_reg && disp != 0 {
            return Some(disp);
        }
    }
    None
}

fn format_code_instruction(
    instr: &iced_x86::Instruction,
    formatter: &mut IntelFormatter,
) -> Instruction {
    let instr_addr = instr.ip();
    let mut mnemonic = String::new();
    formatter.format_mnemonic(instr, &mut mnemonic);

    let op_count = formatter.operand_count(instr);
    let mut operands = Vec::with_capacity(op_count as usize);
    for i in 0..op_count {
        let mut op = String::new();
        if formatter.format_operand(instr, &mut op, i).is_ok() {
            // Useless operand
            if op == "st(0)" {
                continue;
            }
            operands.push(op);
        }
    }

    Instruction {
        address: Some(Address(instr_addr)),
        mnemonic,
        operands,
        comment: None,
        address_str: format!("0x{instr_addr:08x}"),
        raw: Some(*instr),
    }
}
