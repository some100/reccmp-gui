use core::ops::Range;
use std::collections::HashMap;

use eframe::egui::{self, Color32, RichText, ScrollArea, TextStyle};
use iced_x86::FlowControl;

use crate::{
    app::{
        tab::TabView,
        ui::UiAction,
        widgets::{
            BRANCH_COLORS, Branch, InstructionRow, MATCH_TEXT, RowBranches, get_matching_color,
        },
    },
    disassemble::{
        Instruction,
        diff::{DiffKind, DiffRow},
        table::TablesDiff,
    },
    reccmp::Address,
};

#[derive(Clone, Debug)]
pub struct DiffTab {
    pub func_name: String,
    matching: f64,
    removed: bool,
    current_diff_row: Option<usize>,
    rows: Vec<DiffRow>,
    tables: TablesDiff,
    hunks: Vec<usize>,
    error: bool,
    show_tables_window: bool,
    scroll_to_address: Option<Address>,
    orig_branches: Vec<RowBranches>,
    recomp_branches: Vec<RowBranches>,
    pending_scroll_row: Option<usize>,
}

impl DiffTab {
    pub fn new(func_name: String, rows: Vec<DiffRow>, matching: f64, tables: TablesDiff) -> Self {
        let mut tab = Self {
            func_name,
            matching,
            removed: false,
            current_diff_row: None,
            rows,
            tables,
            hunks: Vec::new(),
            error: false,
            show_tables_window: false,
            scroll_to_address: None,
            orig_branches: Vec::new(),
            recomp_branches: Vec::new(),
            pending_scroll_row: None,
        };
        tab.hunks = tab.get_hunk_starts();
        tab.recompute_branches();
        tab
    }

    pub fn get_name(&self) -> &str {
        &self.func_name
    }

    pub fn set_rows(&mut self, rows: Vec<DiffRow>) {
        self.rows = rows;
        self.error = false;
        self.hunks = self.get_hunk_starts();
        self.recompute_branches();
    }

    pub fn set_tables(&mut self, tables: TablesDiff) {
        self.tables = tables;
    }

    pub fn set_matching(&mut self, matching: f64) {
        self.matching = matching;
    }

    pub fn set_removed(&mut self, removed: bool) {
        self.removed = removed;
        if removed {
            self.rows.clear();
            self.orig_branches.clear();
            self.recomp_branches.clear();
        }
    }

    pub fn set_error(&mut self, error: bool) {
        self.error = error;
    }

    fn get_hunk_starts(&self) -> Vec<usize> {
        let mut hunks = Vec::new();
        for i in 0..self.rows.len() {
            let is_diff = !matches!(self.rows[i].kind, DiffKind::Matched | DiffKind::Advisory);
            let is_start = is_diff
                && (i == 0
                    || matches!(
                        self.rows[i - 1].kind,
                        DiffKind::Matched | DiffKind::Advisory
                    ));
            if is_start {
                hunks.push(i);
            }
        }
        hunks
    }

    fn scroll_to_next_diff_on_press(&mut self, ui: &egui::Ui) -> Option<usize> {
        if ui.input(|i| i.key_pressed(egui::Key::N)) {
            let shift = ui.input(|i| i.modifiers.shift);

            if shift {
                let curr = self.current_diff_row.unwrap_or(self.rows.len());
                if let Some(&prev) = self.hunks.iter().rev().find(|&&i| i < curr) {
                    self.current_diff_row = Some(prev);
                    return Some(prev);
                }
            } else {
                let next = match self.current_diff_row {
                    Some(c) => self.hunks.iter().copied().find(|&i| i > c),
                    None => self.hunks.first().copied(),
                };
                if let Some(next) = next {
                    self.current_diff_row = Some(next);
                    return Some(next);
                }
            }
        }

        None
    }

    fn recompute_branches(&mut self) {
        self.orig_branches = Self::compute_side_branches(&self.rows, |r| r.orig.as_ref());
        self.recomp_branches = Self::compute_side_branches(&self.rows, |r| r.recomp.as_ref());
    }

    fn compute_side_branches(
        rows: &[DiffRow],
        get_instr: impl Fn(&DiffRow) -> Option<&Instruction>,
    ) -> Vec<RowBranches> {
        let mut row_branches = vec![RowBranches::default(); rows.len()];
        let mut addr_to_row = HashMap::new();

        for (i, row) in rows.iter().enumerate() {
            if let Some(instr) = get_instr(row)
                && let Some(addr) = instr.address
            {
                addr_to_row.insert(addr, i);
            }
        }

        let mut branch_count = 0;
        for i in 0..rows.len() {
            let Some(instr) = get_instr(&rows[i]) else {
                continue;
            };
            let Some(raw) = &instr.raw else { continue };

            if matches!(
                raw.flow_control(),
                FlowControl::ConditionalBranch | FlowControl::UnconditionalBranch
            ) {
                let target = raw.near_branch_target();
                if target != 0
                    && let Some(&target_row) = addr_to_row.get(&Address(target))
                {
                    let color = BRANCH_COLORS[branch_count % BRANCH_COLORS.len()];
                    branch_count += 1;

                    let branch = Branch {
                        from_row: i,
                        to_row: target_row,
                        color,
                    };
                    row_branches[i].outgoing = Some(branch);
                    row_branches[target_row].incoming.push(branch);
                }
            }
        }

        row_branches
    }

    fn render_diff_grid(&mut self, ui: &mut egui::Ui, row_range: Range<usize>) {
        const DIFF_BG: Color32 = Color32::from_rgb(34, 34, 34);
        const ADVISORY_BG: Color32 = Color32::from_rgb(38, 36, 25);
        const ORIG_DIFF_TEXT: Color32 = Color32::from_rgb(240, 110, 110);
        const RECOMP_DIFF_TEXT: Color32 = Color32::from_rgb(110, 225, 110);
        const ADVISORY_TEXT: Color32 = Color32::from_rgb(220, 200, 100);

        let mut clicked_row = None;

        egui::Grid::new("diff_grid")
            .num_columns(2)
            .min_col_width((ui.available_width() - ui.spacing().item_spacing.x) * 0.5)
            .max_col_width((ui.available_width() - ui.spacing().item_spacing.x) * 0.5)
            .show(ui, |ui| {
                for (i, row) in self.rows[row_range.clone()].iter().enumerate() {
                    let row_idx = row_range.start + i;
                    let orig_b = self.orig_branches.get(row_idx);
                    let recomp_b = self.recomp_branches.get(row_idx);

                    let (orig_bg, recomp_bg, orig_color, recomp_color) = match row.kind {
                        DiffKind::Matched => (
                            Color32::TRANSPARENT,
                            Color32::TRANSPARENT,
                            MATCH_TEXT,
                            MATCH_TEXT,
                        ),
                        DiffKind::Advisory => {
                            (ADVISORY_BG, ADVISORY_BG, ADVISORY_TEXT, ADVISORY_TEXT)
                        }
                        DiffKind::ArgDiff | DiffKind::Diff => {
                            (DIFF_BG, DIFF_BG, ORIG_DIFF_TEXT, RECOMP_DIFF_TEXT)
                        }
                        DiffKind::Added => (
                            Color32::TRANSPARENT,
                            DIFF_BG,
                            ORIG_DIFF_TEXT,
                            RECOMP_DIFF_TEXT,
                        ),
                        DiffKind::Removed => (
                            DIFF_BG,
                            Color32::TRANSPARENT,
                            ORIG_DIFF_TEXT,
                            RECOMP_DIFF_TEXT,
                        ),
                    };

                    ui.add(InstructionRow::new(
                        row.orig.as_ref(),
                        &row.op_diffs,
                        row.kind,
                        orig_color,
                        orig_bg,
                        orig_b,
                        &mut clicked_row,
                    ));
                    ui.add(InstructionRow::new(
                        row.recomp.as_ref(),
                        &row.op_diffs,
                        row.kind,
                        recomp_color,
                        recomp_bg,
                        recomp_b,
                        &mut clicked_row,
                    ));
                    ui.end_row();
                }
            });

        if let Some(target) = clicked_row {
            self.pending_scroll_row = Some(target);
            ui.request_repaint();
        }
    }

    fn render_tables_window(&mut self, ui: &egui::Ui) {
        if !self.show_tables_window {
            return;
        }

        let mut open = self.show_tables_window;
        let mut clicked_target = None;

        egui::Window::new(format!("Tables - {}", self.func_name))
            .open(&mut open)
            .resizable(true)
            .default_size([520.0, 320.0])
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (i, jt) in self.tables.jump_tables.iter().enumerate() {
                        let orig_addr = jt.orig_address.map_or("-".into(), |a| a.to_string());
                        let recomp_addr = jt.recomp_address.map_or("-".into(), |a| a.to_string());

                        let header_color = if jt.all_matched {
                            Color32::GREEN
                        } else {
                            Color32::YELLOW
                        };

                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("Jump Table {i}"))
                                    .strong()
                                    .color(header_color),
                            );
                            ui.label(
                                RichText::new(format!("({orig_addr} / {recomp_addr})")).weak(),
                            );
                        });

                        egui::Grid::new(format!("jt_grid_{i}"))
                            .striped(true)
                            .min_col_width(60.0)
                            .show(ui, |ui| {
                                ui.label(RichText::new("Index").strong());
                                ui.label(RichText::new("Status").strong());
                                ui.label(RichText::new("Orig Target").strong());
                                ui.label(RichText::new("Recomp Target").strong());
                                ui.label(RichText::new("Offset").strong());
                                ui.end_row();

                                for entry in &jt.entries {
                                    ui.label(format!("[{}]", entry.index));

                                    if entry.matches {
                                        ui.colored_label(Color32::GREEN, "MATCH");
                                    } else {
                                        ui.colored_label(Color32::LIGHT_RED, "DIFF");
                                    }

                                    if let Some(orig) = entry.orig_target {
                                        if ui.link(orig.to_string()).clicked() {
                                            clicked_target = Some(orig);
                                        }
                                    } else {
                                        ui.label("-");
                                    }

                                    if let Some(recomp) = entry.recomp_target {
                                        if ui.link(recomp.to_string()).clicked() {
                                            clicked_target = Some(recomp);
                                        }
                                    } else {
                                        ui.label("-");
                                    }

                                    let offset_str = match (entry.orig_offset, entry.recomp_offset)
                                    {
                                        (Some(o), Some(r)) if o == r => format!("+{o:#x}"),
                                        (Some(o), Some(r)) => format!("+{o:#x} vs +{r:#x}"),
                                        (Some(o), None) => format!("+{o:#x}"),
                                        (None, Some(r)) => format!("+{r:#x}"),
                                        _ => "-".to_string(),
                                    };
                                    ui.label(RichText::new(offset_str).weak());

                                    ui.end_row();
                                }
                            });

                        ui.separator();
                    }

                    for (i, dt) in self.tables.data_tables.iter().enumerate() {
                        let orig_addr = dt.orig_address.map_or("-".into(), |a| a.to_string());
                        let recomp_addr = dt.recomp_address.map_or("-".into(), |a| a.to_string());

                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("Data Table #{i}")).strong());
                            ui.label(
                                RichText::new(format!("({orig_addr} / {recomp_addr})")).weak(),
                            );
                        });

                        egui::Grid::new(format!("dt_grid_{i}"))
                            .striped(true)
                            .show(ui, |ui| {
                                ui.label(RichText::new("Index").strong());
                                ui.label(RichText::new("Orig Val").strong());
                                ui.label(RichText::new("Recomp Val").strong());
                                ui.end_row();

                                for entry in &dt.entries {
                                    ui.label(format!("[{}]", entry.index));

                                    let orig_val = entry
                                        .orig_value
                                        .map_or("-".into(), |v| format!("{v:#04x}"));
                                    let recomp_val = entry
                                        .recomp_value
                                        .map_or("-".into(), |v| format!("{v:#04x}"));

                                    let color = if entry.matches {
                                        Color32::LIGHT_GRAY
                                    } else {
                                        Color32::LIGHT_RED
                                    };
                                    ui.colored_label(color, orig_val);
                                    ui.colored_label(color, recomp_val);

                                    ui.end_row();
                                }
                            });

                        ui.separator();
                    }
                });
            });

        self.show_tables_window = open;

        if let Some(target) = clicked_target {
            self.scroll_to_address = Some(target);
        }
    }
}

impl TabView for DiffTab {
    fn render(&mut self, ui: &mut egui::Ui) -> Option<UiAction> {
        let mut action = None;

        ui.vertical(|ui| {
            if self.removed {
                ui.label("Function was removed or renamed.");
                return;
            }
            if self.error {
                ui.label("reccmp-reccmp failed. See logs for details.");
                return;
            }
            if self.rows.is_empty() {
                ui.horizontal(|ui| {
                    ui.label("Waiting for disassembly/build...");
                    ui.spinner();
                });
                return;
            }

            let mut scroll_to_row = self.scroll_to_next_diff_on_press(ui);

            if let Some(row_idx) = self.pending_scroll_row.take() {
                scroll_to_row = Some(row_idx);
                self.current_diff_row = Some(row_idx);
            }

            if let Some(target_addr) = self.scroll_to_address.take()
                && let Some(row_idx) = self.rows.iter().position(|r| {
                    r.orig.as_ref().and_then(|i| i.address) == Some(target_addr)
                        || r.recomp.as_ref().and_then(|i| i.address) == Some(target_addr)
                })
            {
                scroll_to_row = Some(row_idx);
                self.current_diff_row = Some(row_idx);
            }

            let total_hunks = self.hunks.len();
            let current_hunk = self
                .current_diff_row
                .and_then(|row| self.hunks.iter().position(|&r| r == row))
                .map_or(0, |i| i + 1);

            egui::MenuBar::new().ui(ui, |ui| {
                if ui.button("Run reccmp-stackcmp").clicked()
                    && let Some(first_orig_address) = self
                        .rows
                        .iter()
                        .find_map(|x| x.orig.as_ref().and_then(|x| x.address))
                {
                    action = Some(UiAction::Stackcmp {
                        address: first_orig_address,
                        func_name: self.func_name.clone(),
                    });
                }

                let num_jt = self.tables.jump_tables.len();
                let num_dt = self.tables.data_tables.len();
                let total_tables = num_jt + num_dt;

                if total_tables > 0 {
                    let all_matched = self.tables.jump_tables.iter().all(|j| j.all_matched)
                        && self.tables.data_tables.iter().all(|d| d.all_matched);

                    let (status, color) = if all_matched {
                        ("MATCH", Color32::GREEN)
                    } else {
                        ("DIFF", Color32::YELLOW)
                    };

                    let btn_text =
                        RichText::new(format!("Tables: {status} ({total_tables})")).color(color);

                    if ui.button(btn_text).clicked() {
                        self.show_tables_window = !self.show_tables_window;
                    }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("Hunk {current_hunk}/{total_hunks}"))
                            .monospace()
                            .color(Color32::LIGHT_GRAY),
                    );
                    ui.separator();
                    ui.label(
                        RichText::new(format!("{:.2}% matching", self.matching * 100.0))
                            .monospace()
                            .color(get_matching_color(self.matching as f32)),
                    );
                });
            });
            egui::Frame::central_panel(ui.style()).show(ui, |ui| {
                let text_style = TextStyle::Monospace;
                let row_height = ui.text_style_height(&text_style);

                let scroll_area = ScrollArea::vertical().auto_shrink(false);

                scroll_area.show_rows(ui, row_height, self.rows.len(), |ui, row_range| {
                    ui.style_mut().override_text_style = Some(TextStyle::Monospace);

                    if let Some(target_row) = scroll_to_row {
                        let row_height_with_spacing = row_height + ui.spacing().item_spacing.y;
                        let content_top =
                            ui.cursor().top() - (row_range.start as f32 * row_height_with_spacing);
                        let target_y = content_top + (target_row as f32 * row_height_with_spacing);

                        let target_rect = egui::Rect::from_min_size(
                            egui::pos2(ui.min_rect().left(), target_y),
                            egui::vec2(ui.available_width(), row_height),
                        );

                        ui.scroll_to_rect(target_rect, Some(egui::Align::Center));
                    }

                    self.render_diff_grid(ui, row_range);
                });
            });
        });

        self.render_tables_window(ui);

        action
    }

    fn id(&self) -> egui::Id {
        egui::Id::new(("diff", &self.func_name))
    }

    fn title(&self) -> egui::WidgetText {
        self.func_name.clone().into()
    }
}
