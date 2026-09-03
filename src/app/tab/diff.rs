use core::ops::Range;

use eframe::egui::{self, Color32, RichText, ScrollArea, TextStyle};

use crate::{
    app::{
        tab::TabView,
        ui::UiAction,
        widgets::{InstructionRow, MATCH_TEXT},
    },
    disassemble::diff::{DiffKind, DiffRow},
};

pub struct DiffTab {
    pub func_name: String,
    rows: Vec<DiffRow>,
    removed: bool,
    current_diff_row: Option<usize>,
    hunks: Vec<usize>,
}

impl DiffTab {
    pub fn new(func_name: String, rows: Vec<DiffRow>) -> Self {
        let mut tab = Self {
            func_name,
            rows,
            removed: false,
            current_diff_row: None,
            hunks: Vec::new(),
        };
        tab.hunks = tab.get_hunk_starts();
        tab
    }

    pub fn get_name(&self) -> &str {
        &self.func_name
    }

    pub fn set_rows(&mut self, rows: Vec<DiffRow>) {
        self.rows = rows;
        self.hunks = self.get_hunk_starts();
    }

    pub fn clear_rows(&mut self) {
        self.rows.clear();
    }

    pub fn mark_removed(&mut self) {
        self.rows.clear();
        self.removed = true;
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

    fn render_diff_grid(&mut self, ui: &mut egui::Ui, row_range: Range<usize>) {
        const DIFF_BG: Color32 = Color32::from_rgb(34, 34, 34);
        const ADVISORY_BG: Color32 = Color32::from_rgb(38, 36, 25);
        const ORIG_DIFF_TEXT: Color32 = Color32::from_rgb(240, 110, 110);
        const RECOMP_DIFF_TEXT: Color32 = Color32::from_rgb(110, 225, 110);
        const ADVISORY_TEXT: Color32 = Color32::from_rgb(220, 200, 100);

        egui::Grid::new("diff_grid")
            .num_columns(2)
            .min_col_width((ui.available_width() - ui.spacing().item_spacing.x) * 0.5)
            .max_col_width((ui.available_width() - ui.spacing().item_spacing.x) * 0.5)
            .show(ui, |ui| {
                for row in &self.rows[row_range] {
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
                    ));
                    ui.add(InstructionRow::new(
                        row.recomp.as_ref(),
                        &row.op_diffs,
                        row.kind,
                        recomp_color,
                        recomp_bg,
                    ));
                    ui.end_row();
                }
            });
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
            if self.rows.is_empty() {
                ui.label("Function has no rows.");
                return;
            }

            let scroll_to_row = self.scroll_to_next_diff_on_press(ui);

            let total_hunks = self.hunks.len();
            let current_hunk = self
                .current_diff_row
                .and_then(|row| self.hunks.iter().position(|&r| r == row))
                .map_or(0, |idx| idx + 1);

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

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("Hunk {current_hunk}/{total_hunks}"))
                            .monospace()
                            .color(Color32::LIGHT_GRAY),
                    );
                });
            });
            egui::Frame::central_panel(ui.style()).show(ui, |ui| {
                let text_style = TextStyle::Monospace;
                let row_height = ui.text_style_height(&text_style);

                let mut scroll_area = ScrollArea::vertical().auto_shrink(false);
                if let Some(target_row) = scroll_to_row {
                    let row_height_with_spacing = row_height + ui.spacing().item_spacing.y;
                    let scroll_offset = (target_row as f32) * row_height_with_spacing;
                    scroll_area = scroll_area
                        .vertical_scroll_offset(scroll_offset - ui.content_rect().height() * 0.2);
                }

                scroll_area.show_rows(ui, row_height, self.rows.len(), |ui, row_range| {
                    ui.style_mut().override_text_style = Some(TextStyle::Monospace);

                    self.render_diff_grid(ui, row_range);
                });
            });
        });
        action
    }

    fn id(&self) -> egui::Id {
        egui::Id::new(("diff", &self.func_name))
    }

    fn title(&self) -> egui::WidgetText {
        self.func_name.clone().into()
    }
}
