use crate::reccmp::Address;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JumpTableEntry {
    pub address: Address,
    pub target: Address,
    pub target_offset: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JumpTable {
    pub address: Address,
    pub entries: Vec<JumpTableEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataTableEntry {
    pub address: Address,
    pub value: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataTable {
    pub address: Address,
    pub entries: Vec<DataTableEntry>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FunctionTables {
    pub jump_tables: Vec<JumpTable>,
    pub data_tables: Vec<DataTable>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JumpTableEntryDiff {
    pub index: usize,
    pub orig_entry_addr: Option<Address>,
    pub recomp_entry_addr: Option<Address>,
    pub orig_target: Option<Address>,
    pub recomp_target: Option<Address>,
    pub orig_offset: Option<i64>,
    pub recomp_offset: Option<i64>,
    pub matches: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JumpTableDiff {
    pub orig_address: Option<Address>,
    pub recomp_address: Option<Address>,
    pub entries: Vec<JumpTableEntryDiff>,
    pub all_matched: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataTableEntryDiff {
    pub index: usize,
    pub orig_addr: Option<Address>,
    pub recomp_addr: Option<Address>,
    pub orig_value: Option<u8>,
    pub recomp_value: Option<u8>,
    pub matches: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataTableDiff {
    pub orig_address: Option<Address>,
    pub recomp_address: Option<Address>,
    pub entries: Vec<DataTableEntryDiff>,
    pub all_matched: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TablesDiff {
    pub jump_tables: Vec<JumpTableDiff>,
    pub data_tables: Vec<DataTableDiff>,
}

impl TablesDiff {
    pub fn from_tables(orig: &FunctionTables, recomp: &FunctionTables) -> Self {
        let num_jt = orig.jump_tables.len().max(recomp.jump_tables.len());
        let mut jump_tables = Vec::with_capacity(num_jt);

        for i in 0..num_jt {
            let o_jt = orig.jump_tables.get(i);
            let r_jt = recomp.jump_tables.get(i);

            let num_entries = o_jt
                .map_or(0, |j| j.entries.len())
                .max(r_jt.map_or(0, |j| j.entries.len()));

            let mut entries = Vec::with_capacity(num_entries);
            let mut all_matched = true;

            for idx in 0..num_entries {
                let o_entry = o_jt.and_then(|j| j.entries.get(idx));
                let r_entry = r_jt.and_then(|j| j.entries.get(idx));

                let matches = match (o_entry, r_entry) {
                    (Some(o), Some(r)) => o.target_offset == r.target_offset,
                    _ => false,
                };

                if !matches {
                    all_matched = false;
                }

                entries.push(JumpTableEntryDiff {
                    index: idx,
                    orig_entry_addr: o_entry.map(|e| e.address),
                    recomp_entry_addr: r_entry.map(|e| e.address),
                    orig_target: o_entry.map(|e| e.target),
                    recomp_target: r_entry.map(|e| e.target),
                    orig_offset: o_entry.map(|e| e.target_offset),
                    recomp_offset: r_entry.map(|e| e.target_offset),
                    matches,
                });
            }

            jump_tables.push(JumpTableDiff {
                orig_address: o_jt.map(|j| j.address),
                recomp_address: r_jt.map(|j| j.address),
                entries,
                all_matched: all_matched && o_jt.is_some() && r_jt.is_some(),
            });
        }

        let num_dt = orig.data_tables.len().max(recomp.data_tables.len());
        let mut data_tables = Vec::with_capacity(num_dt);

        for i in 0..num_dt {
            let o_dt = orig.data_tables.get(i);
            let r_dt = recomp.data_tables.get(i);

            let num_entries = o_dt
                .map_or(0, |d| d.entries.len())
                .max(r_dt.map_or(0, |d| d.entries.len()));

            let mut entries = Vec::with_capacity(num_entries);
            let mut all_matched = true;

            for idx in 0..num_entries {
                let o_entry = o_dt.and_then(|d| d.entries.get(idx));
                let r_entry = r_dt.and_then(|d| d.entries.get(idx));

                let matches = match (o_entry, r_entry) {
                    (Some(o), Some(r)) => o.value == r.value,
                    _ => false,
                };

                if !matches {
                    all_matched = false;
                }

                entries.push(DataTableEntryDiff {
                    index: idx,
                    orig_addr: o_entry.map(|e| e.address),
                    recomp_addr: r_entry.map(|e| e.address),
                    orig_value: o_entry.map(|e| e.value),
                    recomp_value: r_entry.map(|e| e.value),
                    matches,
                });
            }

            data_tables.push(DataTableDiff {
                orig_address: o_dt.map(|d| d.address),
                recomp_address: r_dt.map(|d| d.address),
                entries,
                all_matched: all_matched && o_dt.is_some() && r_dt.is_some(),
            });
        }

        Self {
            jump_tables,
            data_tables,
        }
    }
}
