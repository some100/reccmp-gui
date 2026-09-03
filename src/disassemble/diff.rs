use crate::{
    app::config::DisassemblySettings,
    disassemble::{Instruction, SymbolMap, table::TablesDiff},
    reccmp::{Address, ReccmpReportDiff},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffKind {
    Matched,
    Advisory,
    ArgDiff,
    Diff,
    Added,
    Removed,
}

pub struct DiffContext<'a> {
    pub orig_symbols: &'a SymbolMap,
    pub recomp_symbols: &'a SymbolMap,
    pub settings: DisassemblySettings,
    pub tables: &'a TablesDiff,
}

impl DiffContext<'_> {
    pub fn tables_match(&self, orig_addr: u64, recomp_addr: u64) -> bool {
        let orig = Address(orig_addr);
        let recomp = Address(recomp_addr);

        self.tables.jump_tables.iter().any(|jt| {
            jt.orig_address == Some(orig) && jt.recomp_address == Some(recomp) && jt.all_matched
        }) || self.tables.data_tables.iter().any(|dt| {
            dt.orig_address == Some(orig) && dt.recomp_address == Some(recomp) && dt.all_matched
        })
    }

    pub fn symbols_match(&self, orig_addr: u64, recomp_addr: u64) -> bool {
        self.tables_match(orig_addr, recomp_addr)
            || match (
                self.orig_symbols.get(Address(orig_addr), self.settings),
                self.recomp_symbols.get(Address(recomp_addr), self.settings),
            ) {
                (Some(na), Some(nb)) => na == nb,
                _ => orig_addr == recomp_addr,
            }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum DiffKey<'a> {
    Code(iced_x86::Code),
    Mnemonic(&'a str),
}

#[derive(Clone, Debug)]
pub struct DiffRow {
    pub orig: Option<Instruction>,
    pub recomp: Option<Instruction>,
    pub kind: DiffKind,
    pub op_diffs: Vec<u32>,
}

impl DiffRow {
    pub fn align_instructions(
        orig: &[Instruction],
        recomp: &[Instruction],
        ctx: &DiffContext,
        is_advisory: bool,
    ) -> Vec<Self> {
        let orig_keys: Vec<_> = orig
            .iter()
            .map(|i| {
                if let Some(raw) = i.raw {
                    DiffKey::Code(raw.code())
                } else {
                    DiffKey::Mnemonic(&i.mnemonic)
                }
            })
            .collect();
        let recomp_keys: Vec<_> = recomp
            .iter()
            .map(|i| {
                if let Some(raw) = i.raw {
                    DiffKey::Code(raw.code())
                } else {
                    DiffKey::Mnemonic(&i.mnemonic)
                }
            })
            .collect();

        let ops =
            similar::capture_diff_slices(similar::Algorithm::Patience, &orig_keys, &recomp_keys);

        let mut rows = Vec::new();

        for op in ops {
            let (tag, old_range, new_range) = op.as_tag_tuple();
            let len = old_range.len().max(new_range.len());

            for i in 0..len {
                let o = (i < old_range.len()).then(|| orig[old_range.start + i].clone());
                let r = (i < new_range.len()).then(|| recomp[new_range.start + i].clone());

                let (kind, op_diffs) = match (tag, &o, &r) {
                    (similar::DiffTag::Equal, Some(o), Some(r)) => match (&o.raw, &r.raw) {
                        (Some(o_raw), Some(r_raw)) if o_raw.op_count() == r_raw.op_count() => {
                            let diffs: Vec<u32> = (0..o.operands.len() as u32)
                                .filter(|&i| {
                                    let text_matches =
                                        o.operands.get(i as usize) == r.operands.get(i as usize);
                                    !text_matches && !operand_eq(o_raw, r_raw, i, ctx)
                                })
                                .collect();

                            if diffs.is_empty() {
                                (DiffKind::Matched, Vec::new())
                            } else if is_advisory {
                                (DiffKind::Advisory, diffs)
                            } else {
                                (DiffKind::ArgDiff, diffs)
                            }
                        }
                        _ => {
                            if o.operands == r.operands {
                                (DiffKind::Matched, Vec::new())
                            } else if o.operands.len() == r.operands.len() {
                                let diffs: Vec<u32> = (0..o.operands.len() as u32)
                                    .filter(|&i| {
                                        o.operands.get(i as usize) != r.operands.get(i as usize)
                                    })
                                    .collect();
                                if is_advisory {
                                    (DiffKind::Advisory, diffs)
                                } else {
                                    (DiffKind::ArgDiff, diffs)
                                }
                            } else {
                                (
                                    if is_advisory {
                                        DiffKind::Advisory
                                    } else {
                                        DiffKind::Diff
                                    },
                                    Vec::new(),
                                )
                            }
                        }
                    },
                    (_, Some(_), Some(_)) => (DiffKind::Diff, Vec::new()),
                    (_, Some(_), None) => (DiffKind::Removed, Vec::new()),
                    (_, None, Some(_)) => (DiffKind::Added, Vec::new()),
                    (_, None, None) => unreachable!(),
                };

                rows.push(Self {
                    orig: o,
                    recomp: r,
                    kind,
                    op_diffs,
                });
            }
        }

        rows
    }

    pub fn from_hunks(
        orig_disasm: &[Instruction],
        recomp_disasm: &[Instruction],
        hunks: &[ReccmpReportDiff],
        ctx: &DiffContext,
    ) -> Vec<Self> {
        if hunks.is_empty() {
            return Self::align_instructions(orig_disasm, recomp_disasm, ctx, true);
        }

        let mut anchors: Vec<(usize, usize)> = Vec::new();
        let mut search_orig = 0;
        let mut search_recomp = 0;

        for hunk in hunks {
            // We use only the both hunks as anchors since the changed hunks
            // may be unreliable if e.g. they get cut off
            if let ReccmpReportDiff::Both { both } = hunk {
                for diff in both {
                    if let (Some(orig_addr), Some(recomp_addr)) = (diff.orig, diff.recomp) {
                        let orig_pos = orig_disasm[search_orig..]
                            .iter()
                            .position(|i| i.address == Some(orig_addr));
                        let recomp_pos = recomp_disasm[search_recomp..]
                            .iter()
                            .position(|i| i.address == Some(recomp_addr));

                        if let (Some(o_pos), Some(r_pos)) = (orig_pos, recomp_pos) {
                            let orig_idx = search_orig + o_pos;
                            let recomp_idx = search_recomp + r_pos;
                            anchors.push((orig_idx, recomp_idx));
                            search_orig = orig_idx + 1;
                            search_recomp = recomp_idx + 1;
                        }
                    }
                }
            }
        }

        let mut rows = Vec::with_capacity(orig_disasm.len().max(recomp_disasm.len()));
        let mut orig_idx = 0;
        let mut recomp_idx = 0;

        // So basically we just ignore the changed hunks and make our own
        for (anchor_orig, anchor_recomp) in anchors {
            if orig_idx < anchor_orig || recomp_idx < anchor_recomp {
                let orig_slice = &orig_disasm[orig_idx..anchor_orig];
                let recomp_slice = &recomp_disasm[recomp_idx..anchor_recomp];
                rows.extend(Self::align_instructions(
                    orig_slice,
                    recomp_slice,
                    ctx,
                    false,
                ));
            }

            let orig_instr = &orig_disasm[anchor_orig];
            let recomp_instr = &recomp_disasm[anchor_recomp];

            let (kind, op_diffs) = match (&orig_instr.raw, &recomp_instr.raw) {
                (Some(o_raw), Some(r_raw)) if o_raw.op_count() == r_raw.op_count() => {
                    let diffs: Vec<u32> = (0..orig_instr.operands.len() as u32)
                        .filter(|&i| {
                            let text_matches = orig_instr.operands.get(i as usize)
                                == recomp_instr.operands.get(i as usize);
                            !text_matches && !operand_eq(o_raw, r_raw, i, ctx)
                        })
                        .collect();
                    if diffs.is_empty() {
                        (DiffKind::Matched, Vec::new())
                    } else {
                        (DiffKind::Advisory, diffs)
                    }
                }
                _ => {
                    if orig_instr.operands == recomp_instr.operands {
                        (DiffKind::Matched, Vec::new())
                    } else {
                        let diffs: Vec<u32> = (0..orig_instr.operands.len() as u32)
                            .filter(|&i| {
                                orig_instr.operands.get(i as usize)
                                    != recomp_instr.operands.get(i as usize)
                            })
                            .collect();
                        (DiffKind::Advisory, diffs)
                    }
                }
            };

            rows.push(DiffRow {
                orig: Some(orig_instr.clone()),
                recomp: Some(recomp_instr.clone()),
                kind,
                op_diffs,
            });

            orig_idx = anchor_orig + 1;
            recomp_idx = anchor_recomp + 1;
        }

        if orig_idx < orig_disasm.len() || recomp_idx < recomp_disasm.len() {
            let orig_tail = &orig_disasm[orig_idx..];
            let recomp_tail = &recomp_disasm[recomp_idx..];
            rows.extend(Self::align_instructions(orig_tail, recomp_tail, ctx, false));
        }

        rows
    }
}

fn operand_eq(
    a: &iced_x86::Instruction,
    b: &iced_x86::Instruction,
    op_idx: u32,
    ctx: &DiffContext,
) -> bool {
    use iced_x86::OpKind;
    if a.op_kind(op_idx) != b.op_kind(op_idx) {
        return false;
    }
    match a.op_kind(op_idx) {
        OpKind::Register => a.op_register(op_idx) == b.op_register(op_idx),
        OpKind::Immediate8
        | OpKind::Immediate16
        | OpKind::Immediate32
        | OpKind::Immediate64
        | OpKind::Immediate8to32
        | OpKind::Immediate8to64
        | OpKind::Immediate32to64 => {
            let imm_a = a.immediate(op_idx);
            let imm_b = b.immediate(op_idx);
            imm_a == imm_b || ctx.symbols_match(imm_a, imm_b)
        }
        OpKind::Immediate8_2nd => a.immediate8_2nd() == b.immediate8_2nd(),
        OpKind::NearBranch16 | OpKind::NearBranch32 | OpKind::NearBranch64 => {
            ctx.symbols_match(a.near_branch_target(), b.near_branch_target())
                || a.near_branch_target().wrapping_sub(a.ip())
                    == b.near_branch_target().wrapping_sub(b.ip())
        }
        OpKind::Memory => {
            if a.memory_size() != b.memory_size() || a.memory_segment() != b.memory_segment() {
                return false;
            }
            if a.memory_base() != b.memory_base()
                || a.memory_index() != b.memory_index()
                || a.memory_index_scale() != b.memory_index_scale()
            {
                return false;
            }
            let disp_a = a.memory_displacement64();
            let disp_b = b.memory_displacement64();

            if a.memory_base() == iced_x86::Register::None {
                disp_a == disp_b || ctx.symbols_match(disp_a, disp_b)
            } else {
                disp_a == disp_b
            }
        }
        _ => false,
    }
}
