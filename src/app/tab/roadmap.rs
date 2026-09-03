use std::collections::btree_set::BTreeSet;

use eframe::egui::{self, Color32, RichText};

use crate::{
    app::{
        tab::TabView,
        ui::UiAction,
        widgets::{TableRow, VirtualTable},
    },
    disassemble::diff::DiffKind,
    roadmap::{RoadmapRow, RoadmapRowType},
};

const TABLE_COLUMNS: [(&str, f32); 7] = [
    ("Status", 65.0),
    ("Orig", 90.0),
    ("Recomp", 90.0),
    ("Disp", 80.0),
    ("Type", 45.0),
    ("Size", 50.0),
    ("Module", 260.0),
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StatusFilter {
    #[default]
    All,
    Matched,
    Diff,
    Removed,
    Added,
}

impl StatusFilter {
    pub fn matches(self, kind: DiffKind) -> bool {
        match self {
            Self::All => true,
            Self::Matched => kind == DiffKind::Matched,
            Self::Diff => kind == DiffKind::Diff,
            Self::Removed => kind == DiffKind::Removed,
            Self::Added => kind == DiffKind::Added,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RoadmapFilter {
    pub search: String,
    pub module: String,
    pub status: StatusFilter,
    pub row_type: Option<RoadmapRowType>,
}

impl RoadmapFilter {
    pub fn matches(&self, row: &RoadmapRow) -> bool {
        if !self.status.matches(row.diff_kind()) {
            return false;
        }
        if let Some(row_type) = self.row_type
            && row.row_type() != row_type
        {
            return false;
        }
        if self.module != "All Modules" && row.module != self.module {
            return false;
        }
        if !self.search.is_empty() && !self.matches_query(row) {
            return false;
        }
        true
    }

    fn matches_query(&self, row: &RoadmapRow) -> bool {
        let q = self.search.trim().to_lowercase();
        row.name.to_lowercase().contains(&q)
            || row.orig_addr.is_some_and(|a| a.to_string().contains(&q))
            || row.recomp_addr.is_some_and(|a| a.to_string().contains(&q))
            || row.module.to_lowercase().contains(&q)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RoadmapStats {
    pub total: usize,
    pub matched: usize,
    pub diff: usize,
    pub removed: usize,
    pub added: usize,
}

impl RoadmapStats {
    pub fn new(rows: &[RoadmapRow]) -> Self {
        let mut stats = Self {
            total: rows.len(),
            ..Default::default()
        };
        for row in rows {
            match row.diff_kind() {
                DiffKind::Matched => stats.matched += 1,
                DiffKind::Diff => stats.diff += 1,
                DiffKind::Removed => stats.removed += 1,
                DiffKind::Added => stats.added += 1,
                _ => {}
            }
        }
        stats
    }

    pub fn percentage(&self, count: usize) -> f32 {
        if self.total > 0 {
            (count as f32 / self.total as f32) * 100.0
        } else {
            0.0
        }
    }
}

pub struct RoadmapTab {
    rows: Vec<RoadmapRow>,
    stats: RoadmapStats,
    available_modules: Vec<String>,
    filter: RoadmapFilter,
    filtered_indices: Vec<usize>,
}

impl RoadmapTab {
    pub fn new(rows: Vec<RoadmapRow>) -> Self {
        let mut tab = Self {
            rows: Vec::new(),
            stats: RoadmapStats::default(),
            available_modules: vec!["All Modules".to_string()],
            filter: RoadmapFilter {
                module: "All Modules".to_string(),
                ..Default::default()
            },
            filtered_indices: Vec::new(),
        };
        tab.set_rows(rows);
        tab
    }

    pub fn set_rows(&mut self, rows: Vec<RoadmapRow>) {
        let mut modules = BTreeSet::new();
        for r in &rows {
            if !r.module.is_empty() {
                modules.insert(r.module.clone());
            }
        }

        self.stats = RoadmapStats::new(&rows);
        self.available_modules = std::iter::once("All Modules".to_string())
            .chain(modules)
            .collect();
        self.rows = rows;
        self.update_filter();
    }

    fn update_filter(&mut self) {
        self.filtered_indices = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| self.filter.matches(r))
            .map(|(i, _)| i)
            .collect();
    }

    fn render_stats_banner(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Total: {}", self.stats.total)).strong());
            ui.separator();

            let badges = [
                (Color32::LIGHT_GREEN, "Matched", self.stats.matched),
                (Color32::DARK_RED, "Diff", self.stats.diff),
                (Color32::LIGHT_RED, "Removed", self.stats.removed),
                (Color32::DARK_GREEN, "Added", self.stats.added),
            ];

            for (color, label, count) in badges {
                let pct = self.stats.percentage(count);
                ui.colored_label(color, format!("{label}: {count} ({pct:.1}%)"));
            }
        });
    }

    fn render_toolbar(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;

        ui.horizontal_wrapped(|ui| {
            changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut self.filter.search)
                        .hint_text("Search name, module, or address...")
                        .desired_width(220.0),
                )
                .changed();

            egui::ComboBox::from_id_salt("roadmap_module_filter")
                .selected_text(self.filter.module.clone())
                .show_ui(ui, |ui| {
                    for m in &self.available_modules {
                        changed |= ui
                            .selectable_value(&mut self.filter.module, m.clone(), m)
                            .changed();
                    }
                });

            ui.separator();

            let statuses = [
                (StatusFilter::All, "All"),
                (StatusFilter::Matched, "Matched"),
                (StatusFilter::Diff, "Diff"),
                (StatusFilter::Removed, "Removed"),
                (StatusFilter::Added, "Added"),
            ];
            for (status, label) in statuses {
                changed |= ui
                    .selectable_value(&mut self.filter.status, status, label)
                    .clicked();
            }

            ui.separator();

            let type_filters = [
                (None, "All Types"),
                (Some(RoadmapRowType::Function), "FUN"),
                (Some(RoadmapRowType::Data), "DAT"),
                (Some(RoadmapRowType::String), "STR"),
                (Some(RoadmapRowType::Vtable), "VTA"),
                (Some(RoadmapRowType::Import), "IMP"),
                (Some(RoadmapRowType::Label), "LAB"),
                (Some(RoadmapRowType::Float), "FLO"),
                (Some(RoadmapRowType::Widechar), "WID"),
            ];
            for (row_type, label) in type_filters {
                if ui
                    .selectable_label(self.filter.row_type == row_type, label)
                    .clicked()
                {
                    self.filter.row_type = row_type;
                    changed = true;
                }
            }
        });

        changed
    }

    fn render_table(&mut self, ui: &mut egui::Ui) -> Option<UiAction> {
        let mut action = None;

        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("Showing {} entries", self.filtered_indices.len()))
                    .small()
                    .weak(),
            );
        });

        ui.add(
            VirtualTable::new(
                &TABLE_COLUMNS,
                self.filtered_indices.len(),
                |ui, idx, height| {
                    if let Some(row_action) =
                        self.render_row(ui, height, self.filtered_indices[idx])
                    {
                        action = Some(row_action);
                    }
                },
            )
            .with_trailing("Name"),
        );

        action
    }

    fn render_row(&self, ui: &mut egui::Ui, row_height: f32, idx: usize) -> Option<UiAction> {
        let row = &self.rows[idx];

        let status_badge = match row.diff_kind() {
            DiffKind::Matched => RichText::new("MATCH").color(Color32::GRAY),
            DiffKind::Diff => RichText::new("DIFF").color(Color32::DARK_RED),
            DiffKind::Removed => RichText::new("DEL").color(Color32::LIGHT_RED),
            DiffKind::Added => RichText::new("ADD").color(Color32::DARK_GREEN),
            _ => RichText::new("").color(Color32::PLACEHOLDER), // This should not be happening
        };

        let (type_bg, type_fg) = match row.row_type() {
            RoadmapRowType::Function => (Color32::DARK_BLUE, Color32::WHITE),
            RoadmapRowType::Data => (Color32::DARK_GREEN, Color32::WHITE),
            RoadmapRowType::String => (Color32::BROWN, Color32::WHITE),
            RoadmapRowType::Vtable => (Color32::PURPLE, Color32::WHITE),
            // Hopefully no one notices that these use the same color
            RoadmapRowType::Import | RoadmapRowType::Float | RoadmapRowType::Widechar => {
                (Color32::DARK_GRAY, Color32::WHITE)
            }
            _ => (Color32::TRANSPARENT, Color32::GRAY),
        };

        let displ_str = match row.displacement {
            Some(0) => "0x0".to_owned(),
            Some(d) if d >= 0 => format!("+{d:#x}"),
            Some(d) => format!("-{:#x}", -d),
            None => "-".to_string(),
        };

        let displ_color = match row.displacement {
            Some(0) => Color32::DARK_GREEN,
            Some(_) => Color32::LIGHT_RED,
            None => Color32::GRAY,
        };

        let orig_str = row.orig_addr.map_or("-".into(), |a| a.to_string());
        let recomp_str = row.recomp_addr.map_or("-".into(), |a| a.to_string());
        let type_badge = RichText::new(row.row_type().as_str())
            .background_color(type_bg)
            .color(type_fg)
            .strong();
        let size_str = row.size.as_deref().unwrap_or("-");

        let mut target_to_open = None;
        ui.add(
            TableRow::new(&TABLE_COLUMNS, row_height)
                .cell(status_badge)
                .cell(RichText::new(orig_str).color(Color32::LIGHT_GRAY))
                .cell(RichText::new(recomp_str).color(Color32::LIGHT_GRAY))
                .cell(RichText::new(displ_str).color(displ_color))
                .cell(type_badge)
                .cell(RichText::new(size_str).color(Color32::GRAY))
                .cell(RichText::new(row.module.clone()).color(Color32::GRAY))
                .trailing_ui(&mut |ui| {
                    ui.horizontal(|ui| {
                        if row.row_type() == RoadmapRowType::Function {
                            let label = ui.link(&row.name);
                            if label.clicked() {
                                target_to_open = Some(row.name.clone());
                            }
                        } else {
                            ui.label(&row.name);
                        }
                    })
                    .response
                }),
        );

        if let Some(name) = target_to_open {
            return Some(UiAction::OpenFunctionByName(name));
        }

        None
    }
}

impl TabView for RoadmapTab {
    fn render(&mut self, ui: &mut egui::Ui) -> Option<UiAction> {
        let mut action = None;

        ui.vertical(|ui| {
            self.render_stats_banner(ui);
            ui.separator();

            if self.render_toolbar(ui) {
                self.update_filter();
            }

            ui.separator();

            action = self.render_table(ui);
        });

        action
    }

    fn id(&self) -> egui::Id {
        egui::Id::new("roadmap")
    }

    fn title(&self) -> egui::WidgetText {
        "Roadmap".into()
    }
}
