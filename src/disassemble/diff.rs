use crate::{
    disassemble::{Instruction, SymbolMap},
    reccmp::{Address, ReccmpReportChangedDiff, ReccmpReportDiff},
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
}

impl DiffContext<'_> {
    pub fn symbols_match(&self, orig_addr: u64, recomp_addr: u64) -> bool {
        match (
            self.orig_symbols.get(&Address(orig_addr)),
            self.recomp_symbols.get(&Address(recomp_addr)),
        ) {
            (Some(na), Some(nb)) => na == nb,
            _ => orig_addr == recomp_addr,
        }
    }
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
        let orig_mnemonics: Vec<_> = orig.iter().map(|i| &i.mnemonic).collect();
        let recomp_mnemonics: Vec<_> = recomp.iter().map(|i| &i.mnemonic).collect();

        let ops = similar::capture_diff_slices(
            similar::Algorithm::Patience,
            &orig_mnemonics,
            &recomp_mnemonics,
        );

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

        let mut rows = Vec::with_capacity(orig_disasm.len().max(recomp_disasm.len()));
        let mut orig_idx = 0;
        let mut recomp_idx = 0;

        let (first_diff_orig, first_diff_recomp) = Self::get_first_diff_addresses(hunks);

        if let (Some(target_orig), Some(target_recomp)) = (first_diff_orig, first_diff_recomp) {
            while orig_idx < orig_disasm.len()
                && recomp_idx < recomp_disasm.len()
                && orig_disasm[orig_idx]
                    .address
                    .is_some_and(|a| a < target_orig)
                && recomp_disasm[recomp_idx]
                    .address
                    .is_some_and(|a| a < target_recomp)
            {
                rows.push(DiffRow {
                    orig: Some(orig_disasm[orig_idx].clone()),
                    recomp: Some(recomp_disasm[recomp_idx].clone()),
                    kind: DiffKind::Matched,
                    op_diffs: Vec::new(),
                });
                orig_idx += 1;
                recomp_idx += 1;
            }
        }

        for diff in hunks {
            match diff {
                ReccmpReportDiff::Both { both } => {
                    for diff in both {
                        let orig = Instruction::from_reccmp(diff.orig, &diff.asm);
                        let recomp = Instruction::from_reccmp(diff.recomp, &diff.asm);

                        while orig_idx < orig_disasm.len()
                            && orig_disasm[orig_idx].address <= orig.address
                        {
                            orig_idx += 1;
                        }
                        while recomp_idx < recomp_disasm.len()
                            && recomp_disasm[recomp_idx].address <= recomp.address
                        {
                            recomp_idx += 1;
                        }

                        rows.push(DiffRow {
                            orig: Some(orig),
                            recomp: Some(recomp),
                            kind: DiffKind::Matched,
                            op_diffs: Vec::new(),
                        });
                    }
                }
                ReccmpReportDiff::Changed { orig, recomp } => {
                    let start_orig = orig_idx;
                    if let Some(last_orig_addr) = diff.last_orig_address() {
                        while orig_idx < orig_disasm.len()
                            && orig_disasm[orig_idx]
                                .address
                                .is_some_and(|a| a <= last_orig_addr)
                        {
                            orig_idx += 1;
                        }
                    }
                    let end_orig = orig_idx;

                    let start_recomp = recomp_idx;
                    if let Some(last_recomp_addr) = diff.last_recomp_address() {
                        while recomp_idx < recomp_disasm.len()
                            && recomp_disasm[recomp_idx]
                                .address
                                .is_some_and(|a| a <= last_recomp_addr)
                        {
                            recomp_idx += 1;
                        }
                    }
                    let end_recomp = recomp_idx;

                    if diff.is_table() {
                        Self::handle_changed_diff(&mut rows, orig, recomp, ctx);
                    } else {
                        let orig_slice = &orig_disasm[start_orig..end_orig];
                        let recomp_slice = &recomp_disasm[start_recomp..end_recomp];
                        rows.extend(Self::align_instructions(
                            orig_slice,
                            recomp_slice,
                            ctx,
                            false,
                        ));
                    }
                }
            }
        }

        if orig_idx < orig_disasm.len() || recomp_idx < recomp_disasm.len() {
            let tail = Self::align_instructions(
                &orig_disasm[orig_idx..],
                &recomp_disasm[recomp_idx..],
                ctx,
                true,
            );
            rows.extend(tail);
        }

        rows
    }

    fn get_first_diff_addresses(diff: &[ReccmpReportDiff]) -> (Option<Address>, Option<Address>) {
        for hunk in diff {
            match hunk {
                ReccmpReportDiff::Both { both } => {
                    if let Some(first) = both.first() {
                        return (first.orig, first.recomp);
                    }
                }
                ReccmpReportDiff::Changed { orig, recomp } => {
                    let orig_addr = orig.first().and_then(|x| x.address);
                    let recomp_addr = recomp.first().and_then(|x| x.address);
                    if orig_addr.is_some() || recomp_addr.is_some() {
                        return (orig_addr, recomp_addr);
                    }
                }
            }
        }
        (None, None)
    }

    fn handle_changed_diff(
        rows: &mut Vec<Self>,
        orig: &[ReccmpReportChangedDiff],
        recomp: &[ReccmpReportChangedDiff],
        ctx: &DiffContext,
    ) {
        let orig_instrs: Vec<Instruction> = orig
            .iter()
            .map(|d| Instruction::from_reccmp(d.address, &d.asm))
            .collect();
        let recomp_instrs: Vec<Instruction> = recomp
            .iter()
            .map(|d| Instruction::from_reccmp(d.address, &d.asm))
            .collect();

        rows.extend(Self::align_instructions(
            &orig_instrs,
            &recomp_instrs,
            ctx,
            false,
        ));
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
