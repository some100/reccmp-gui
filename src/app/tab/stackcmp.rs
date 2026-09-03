use core::cmp::Ordering;

use eframe::egui::{self, Color32, RichText, ScrollArea, TextStyle};

use crate::{
    app::{
        tab::TabView,
        ui::UiAction,
        widgets::{TableHeader, TableRow, VirtualTable},
    },
    stackcmp::{StackVariable, StackcmpReport, StackcmpRow, StackcmpStatus},
};

const TABLE_COLUMNS: [(&str, f32); 5] = [
    ("Status", 65.0),
    ("Orig", 90.0),
    ("Recomp", 90.0),
    ("Delta", 40.0),
    ("Size", 40.0),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackOrderView {
    Orig,
    Recomp,
    Both,
}

#[derive(Default)]
pub struct StackcmpStats {
    pub matched: usize,
    pub mismatch: usize,
    pub conflict: usize,
    pub unknown: usize,
}

impl StackcmpStats {
    pub fn new(rows: &[StackcmpRow]) -> Self {
        let mut stats = Self::default();
        for row in rows {
            match row.status {
                StackcmpStatus::Matched => stats.matched += 1,
                StackcmpStatus::Mismatch => stats.mismatch += 1,
                StackcmpStatus::Conflict => stats.conflict += 1,
                StackcmpStatus::Unknown => stats.unknown += 1,
            }
        }
        stats
    }
}

// The real reason why you use reccmp
pub struct StackcmpTab {
    pub report: StackcmpReport,
    stats: StackcmpStats,
    view_mode: StackOrderView,
    hide_matched: bool,
    filtered_orig: Vec<usize>,
    filtered_recomp: Vec<usize>,
}

impl StackcmpTab {
    pub fn new(report: StackcmpReport) -> Self {
        let stats = StackcmpStats::new(&report.ordered_by_orig);
        let mut tab = Self {
            report,
            stats,
            view_mode: StackOrderView::Orig,
            hide_matched: false,
            filtered_orig: Vec::new(),
            filtered_recomp: Vec::new(),
        };
        tab.update_filter();
        tab
    }

    fn current_rows(&self) -> &[StackcmpRow] {
        match self.view_mode {
            StackOrderView::Orig | StackOrderView::Both => &self.report.ordered_by_orig,
            StackOrderView::Recomp => &self.report.ordered_by_recomp,
        }
    }

    fn current_filtered(&self) -> &[usize] {
        match self.view_mode {
            StackOrderView::Orig | StackOrderView::Both => &self.filtered_orig,
            StackOrderView::Recomp => &self.filtered_recomp,
        }
    }

    fn current_filter(&self, rows: &[StackcmpRow]) -> Vec<usize> {
        if self.hide_matched {
            rows.iter()
                .enumerate()
                .filter(|(_, row)| !matches!(row.status, StackcmpStatus::Matched))
                .map(|(idx, _)| idx)
                .collect()
        } else {
            (0..rows.len()).collect()
        }
    }

    fn update_filter(&mut self) {
        self.filtered_orig = self.current_filter(&self.report.ordered_by_orig);
        self.filtered_recomp = self.current_filter(&self.report.ordered_by_recomp);
    }

    fn render_stats_banner(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Function: ");
            ui.label(&self.report.func_name);
            ui.separator();

            let badges = [
                (Color32::LIGHT_GREEN, "Matched", self.stats.matched),
                (Color32::YELLOW, "Reordered", self.stats.mismatch),
                (Color32::LIGHT_RED, "Conflict", self.stats.conflict),
                (Color32::LIGHT_BLUE, "Unknown", self.stats.unknown),
            ];

            for (color, label, count) in badges {
                ui.colored_label(color, format!("{label}: {count}"));
            }
        });
    }

    fn render_toolbar(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;

        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Order by:").strong());
            changed |= ui
                .selectable_value(&mut self.view_mode, StackOrderView::Both, "Both")
                .changed();
            changed |= ui
                .selectable_value(&mut self.view_mode, StackOrderView::Orig, "Original")
                .changed();
            changed |= ui
                .selectable_value(&mut self.view_mode, StackOrderView::Recomp, "Recompiled")
                .changed();
            ui.separator();

            changed |= ui
                .checkbox(&mut self.hide_matched, "Hide Matched")
                .changed();
            ui.separator();
        });

        changed
    }

    fn render_side_by_side(&mut self, ui: &mut egui::Ui) {
        let text_style = TextStyle::Monospace;
        let row_height = ui.text_style_height(&text_style) + 6.0;
        let half_width = (ui.available_width() - 20.0) * 0.5;

        let max_rows = self.filtered_orig.len().max(self.filtered_recomp.len());

        ui.horizontal(|ui| {
            ui.add_sized(
                [half_width, 20.0],
                egui::Label::new(RichText::new("Original Stack").strong()),
            );
            ui.separator();
            ui.add_sized(
                [half_width, 20.0],
                egui::Label::new(RichText::new("Recompiled Stack").strong()),
            );
        });
        ui.horizontal(|ui| {
            // uhhhh huh
            ui.horizontal(|ui| {
                ui.set_width(half_width);
                ui.add(TableHeader::new(&TABLE_COLUMNS).with_trailing("Name"));
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.set_width(half_width);
                ui.add(TableHeader::new(&TABLE_COLUMNS).with_trailing("Name"));
            });
        });
        ui.separator();

        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, row_height, max_rows, |ui, row_range| {
                ui.style_mut().override_text_style = Some(TextStyle::Monospace);

                for idx in row_range {
                    ui.horizontal(|ui| {
                        ui.horizontal(|ui| {
                            ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                            ui.set_width(half_width);

                            if let Some(&orig_idx) = self.filtered_orig.get(idx) {
                                let row = &self.report.ordered_by_orig[orig_idx];
                                let next = self.report.ordered_by_orig.get(orig_idx + 1);
                                self.render_row(ui, row, next, row_height);
                            }
                        });

                        ui.separator();

                        ui.horizontal(|ui| {
                            ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                            ui.set_width(half_width);

                            if let Some(&recomp_idx) = self.filtered_recomp.get(idx) {
                                let row = &self.report.ordered_by_recomp[recomp_idx];
                                let next = self.report.ordered_by_recomp.get(recomp_idx + 1);
                                self.render_row(ui, row, next, row_height);
                            }
                        });
                    });
                }
            });
    }

    fn render_table(&mut self, ui: &mut egui::Ui) {
        let rows = self.current_rows();
        let filtered = self.current_filtered();

        ui.add(
            VirtualTable::new(
                &TABLE_COLUMNS,
                self.current_filtered().len(),
                |ui, idx, row_height| {
                    let actual_idx = filtered[idx];
                    let row = &rows[actual_idx];
                    let next = rows.get(actual_idx + 1);
                    self.render_row(ui, row, next, row_height);
                },
            )
            .with_trailing("Name"),
        );
    }

    fn render_row(
        &self,
        ui: &mut egui::Ui,
        row: &StackcmpRow,
        next_row: Option<&StackcmpRow>,
        row_height: f32,
    ) {
        if self.hide_matched && matches!(row.status, StackcmpStatus::Matched) {
            return;
        }

        let status_badge = match row.status {
            StackcmpStatus::Matched => RichText::new("MATCH").color(Color32::GREEN),
            StackcmpStatus::Mismatch => RichText::new("DIFF").color(Color32::YELLOW),
            StackcmpStatus::Conflict => RichText::new("CONFLICT").color(Color32::LIGHT_RED),
            StackcmpStatus::Unknown => RichText::new("UNKNOWN").color(Color32::LIGHT_BLUE),
        };

        let delta = row.orig.offset - row.recomp.offset;
        let delta_str = match delta {
            _ if matches!(row.status, StackcmpStatus::Unknown) => "-".to_owned(),
            0 => "0x0".to_owned(),
            d if d >= 0 => format!("+{d:#x}"),
            d => format!("-{:#x}", -d),
        };

        let delta_color = if delta_str == "0x0" {
            Color32::GREEN
        } else if delta_str.starts_with('+') {
            Color32::LIGHT_YELLOW
        } else {
            Color32::LIGHT_RED
        };

        let orig_str = Self::format_offset(&row.orig, row.status);

        let recomp_str = Self::format_offset(&row.recomp, row.status);

        let size_str = if let Some(next) = next_row {
            if matches!(row.status, StackcmpStatus::Unknown) {
                format!("{}", (next.recomp.offset - row.recomp.offset).abs())
            } else {
                // Even if the row is ordered by recomp, we still use the orig
                // offset here since that's more useful for matching the original
                format!("{}", (next.orig.offset - row.orig.offset).abs())
            }
        } else {
            "-".to_string()
        };

        ui.add(
            TableRow::new(&TABLE_COLUMNS, row_height)
                .cell(status_badge)
                .cell(RichText::new(orig_str).color(Color32::LIGHT_GRAY))
                .cell(RichText::new(recomp_str).color(Color32::LIGHT_GRAY))
                .cell(RichText::new(delta_str).color(delta_color))
                .cell(RichText::new(size_str).color(Color32::LIGHT_GRAY))
                .trailing(row.name()),
        );
    }

    fn format_offset(row: &StackVariable, status: StackcmpStatus) -> String {
        match row.offset.cmp(&0) {
            Ordering::Equal if matches!(status, StackcmpStatus::Unknown) => "-".to_string(),
            Ordering::Equal => format!("{} + 0x0", row.base_reg),
            Ordering::Greater => format!("{} + {:#x}", row.base_reg, row.offset),
            Ordering::Less => format!("{} - {:#x}", row.base_reg, -row.offset),
        }
    }
}

impl TabView for StackcmpTab {
    fn render(&mut self, ui: &mut egui::Ui) -> Option<UiAction> {
        ui.vertical(|ui| {
            self.render_stats_banner(ui);
            ui.separator();

            if self.render_toolbar(ui) {
                self.update_filter();
            }

            ui.separator();

            match self.view_mode {
                StackOrderView::Both => self.render_side_by_side(ui),
                StackOrderView::Orig | StackOrderView::Recomp => self.render_table(ui),
            }
        });
        None
    }

    fn id(&self) -> egui::Id {
        egui::Id::new(("stackcmp", &self.report.func_name, self.report.address))
    }

    fn title(&self) -> egui::WidgetText {
        format!(
            "stackcmp: {} ({})",
            self.report.func_name, self.report.address
        )
        .into()
    }
}
