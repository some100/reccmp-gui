use eframe::egui::{self, Color32, RichText};

use crate::{
    app::{
        tab::TabView,
        ui::UiAction,
        widgets::{TableRow, VirtualTable},
    },
    disassemble::diff::DiffKind,
    reccmp::{Address, ReccmpReportData, ReccmpReportDiff},
};

const TABLE_COLUMNS: [(&str, f32); 5] = [
    ("Status", 65.0),
    ("Slot", 65.0),
    ("Index", 45.0),
    ("Orig Addr", 85.0),
    ("Recomp Addr", 85.0),
];

#[derive(Clone, Debug)]
pub struct VtableTarget {
    pub address: Option<Address>,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct VtableRow {
    pub slot_str: String,
    pub index: Option<u32>,
    pub orig: Option<VtableTarget>,
    pub recomp: Option<VtableTarget>,
    pub kind: DiffKind,
}

pub struct VtableTab {
    pub name: String,
    pub matching: f64,
    pub rows: Vec<VtableRow>,
    pub removed: bool,
}

impl VtableTab {
    pub fn new(data: &ReccmpReportData) -> Self {
        let mut tab = Self {
            name: data.name.clone(),
            matching: data.matching,
            rows: Vec::new(),
            removed: false,
        };
        tab.update_from_data(data);
        tab
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn mark_removed(&mut self) {
        self.rows.clear();
        self.removed = true;
    }

    pub fn update_from_data(&mut self, data: &ReccmpReportData) {
        self.matching = data.matching;
        self.rows.clear();
        self.removed = false;

        let Some(diff_sections) = &data.diff else {
            return;
        };

        for (_, hunks) in diff_sections {
            for hunk in hunks {
                match hunk {
                    ReccmpReportDiff::Both { both } => {
                        for entry in both {
                            let (offset, slot_str) = Self::parse_slot(&entry.tag);
                            let (orig_addr, recomp_addr, name) =
                                Self::parse_vtable_line(&entry.asm);

                            self.rows.push(VtableRow {
                                slot_str,
                                index: offset.map(|o| o / 4),
                                orig: Some(VtableTarget {
                                    address: orig_addr,
                                    name: name.clone(),
                                }),
                                recomp: Some(VtableTarget {
                                    address: recomp_addr,
                                    name,
                                }),
                                kind: DiffKind::Matched,
                            });
                        }
                    }
                    ReccmpReportDiff::Changed { orig, recomp } => {
                        let len = orig.len().max(recomp.len());
                        for i in 0..len {
                            let o = orig.get(i);
                            let r = recomp.get(i);

                            match (o, r) {
                                (Some(o_diff), Some(r_diff)) => {
                                    let (offset, slot_str) = Self::parse_slot(&o_diff.tag);
                                    let (orig_addr, _, orig_name) =
                                        Self::parse_vtable_line(&o_diff.asm);
                                    let (recomp_addr, _, recomp_name) =
                                        Self::parse_vtable_line(&r_diff.asm);

                                    self.rows.push(VtableRow {
                                        slot_str,
                                        index: offset.map(|o| o / 4),
                                        orig: Some(VtableTarget {
                                            address: orig_addr,
                                            name: orig_name,
                                        }),
                                        recomp: Some(VtableTarget {
                                            address: recomp_addr,
                                            name: recomp_name,
                                        }),
                                        kind: DiffKind::Diff,
                                    });
                                }
                                (Some(o_diff), None) => {
                                    let (offset, slot_str) = Self::parse_slot(&o_diff.tag);
                                    let (orig_addr, _, orig_name) =
                                        Self::parse_vtable_line(&o_diff.asm);

                                    self.rows.push(VtableRow {
                                        slot_str,
                                        index: offset.map(|o| o / 4),
                                        orig: Some(VtableTarget {
                                            address: orig_addr,
                                            name: orig_name,
                                        }),
                                        recomp: None,
                                        kind: DiffKind::Removed,
                                    });
                                }
                                (None, Some(r_diff)) => {
                                    let (offset, slot_str) = Self::parse_slot(&r_diff.tag);
                                    let (recomp_addr, _, recomp_name) =
                                        Self::parse_vtable_line(&r_diff.asm);

                                    self.rows.push(VtableRow {
                                        slot_str,
                                        index: offset.map(|o| o / 4),
                                        orig: None,
                                        recomp: Some(VtableTarget {
                                            address: recomp_addr,
                                            name: recomp_name,
                                        }),
                                        kind: DiffKind::Added,
                                    });
                                }
                                (None, None) => unreachable!(),
                            }
                        }
                    }
                }
            }
        }
    }

    fn parse_slot(s: &str) -> (Option<u32>, String) {
        let trimmed = s.trim();
        let hex_part = trimmed.strip_prefix("vtable0x").unwrap_or(trimmed);

        if let Ok(offset) = u32::from_str_radix(hex_part, 16) {
            (Some(offset), format!("+0x{offset:02x}"))
        } else {
            (None, trimmed.to_string())
        }
    }

    fn parse_vtable_line(asm: &str) -> (Option<Address>, Option<Address>, String) {
        let trimmed = asm.trim();
        if let Some(rest) = trimmed.strip_prefix('(')
            && let Some((addrs_part, name_part)) = rest.split_once(')')
        {
            let name = name_part
                .trim()
                .strip_prefix(':')
                .unwrap_or(name_part)
                .trim()
                .to_string();

            let addrs_part = addrs_part.trim();
            if let Some((orig_str, recomp_str)) = addrs_part.split_once('/') {
                return (
                    Address::from_str_opt(orig_str.trim()),
                    Address::from_str_opt(recomp_str.trim()),
                    name,
                );
            }

            return (Address::from_str_opt(addrs_part), None, name);
        }

        (None, None, trimmed.to_string())
    }
}

impl TabView for VtableTab {
    fn render(&mut self, ui: &mut egui::Ui) -> Option<UiAction> {
        let mut action = None;

        ui.vertical(|ui| {
            if self.removed {
                ui.label("Vtable was removed or renamed.");
                return;
            }

            ui.horizontal(|ui| {
                ui.label(RichText::new(&self.name).strong());
                ui.separator();
                let pct = self.matching * 100.0;
                let color = if pct >= 100.0 {
                    Color32::LIGHT_GREEN
                } else {
                    Color32::YELLOW
                };
                ui.colored_label(color, format!("Match: {pct:.2}%"));
                ui.separator();
                ui.label(
                    RichText::new(format!("{} entries", self.rows.len()))
                        .small()
                        .weak(),
                );
            });

            ui.separator();

            let mut target_to_open = None;

            ui.add(
                VirtualTable::new(
                    &TABLE_COLUMNS,
                    self.rows.len(),
                    |ui, row_idx, row_height| {
                        let row = &self.rows[row_idx];

                        let status_badge = match row.kind {
                            DiffKind::Matched => RichText::new("MATCH").color(Color32::GRAY),
                            DiffKind::Diff => RichText::new("DIFF").color(Color32::DARK_RED),
                            DiffKind::Removed => RichText::new("DEL").color(Color32::LIGHT_RED),
                            DiffKind::Added => RichText::new("ADD").color(Color32::DARK_GREEN),
                            _ => RichText::new("").color(Color32::TRANSPARENT),
                        };

                        let index_str = row.index.map_or("-".to_string(), |i| format!("[{i}]"));

                        let orig_addr_str = row
                            .orig
                            .as_ref()
                            .and_then(|o| o.address)
                            .map_or("-".to_string(), |a| a.to_string());

                        let recomp_addr_str = row
                            .recomp
                            .as_ref()
                            .and_then(|r| r.address)
                            .map_or("-".to_string(), |a| a.to_string());

                        ui.add(
                            TableRow::new(&TABLE_COLUMNS, row_height)
                                .cell(status_badge)
                                .cell(RichText::new(&row.slot_str).color(Color32::LIGHT_GRAY))
                                .cell(RichText::new(index_str).color(Color32::GRAY))
                                .cell(RichText::new(orig_addr_str).color(Color32::GRAY))
                                .cell(RichText::new(recomp_addr_str).color(Color32::GRAY))
                                .trailing_ui(&mut |ui| {
                                    ui.horizontal(|ui| match (&row.orig, &row.recomp) {
                                        (Some(orig), Some(recomp)) => {
                                            if ui.link(&orig.name).clicked() {
                                                target_to_open = Some(orig.name.clone());
                                            }
                                            if orig.name != recomp.name {
                                                ui.label("->");
                                                if ui.link(&recomp.name).clicked() {
                                                    target_to_open = Some(recomp.name.clone());
                                                }
                                            }
                                        }
                                        (Some(orig), None) => {
                                            if ui.link(&orig.name).clicked() {
                                                target_to_open = Some(orig.name.clone());
                                            }
                                        }
                                        (None, Some(recomp)) => {
                                            if ui.link(&recomp.name).clicked() {
                                                target_to_open = Some(recomp.name.clone());
                                            }
                                        }
                                        (None, None) => {}
                                    })
                                    .response
                                }),
                        );
                    },
                )
                .with_trailing("Target Function"),
            );

            if let Some(name) = target_to_open {
                action = Some(UiAction::OpenFunctionByName(name));
            }
        });

        action
    }

    fn id(&self) -> egui::Id {
        egui::Id::new(("vtable", &self.name))
    }

    fn title(&self) -> egui::WidgetText {
        self.name.clone().into()
    }
}
