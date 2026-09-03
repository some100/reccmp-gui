use eframe::egui::{self, Button, Color32, RichText, TextStyle};

use crate::{
    app::{
        tab::TabView,
        ui::UiAction,
        widgets::{TableRow, VirtualTable, get_matching_color},
    },
    reccmp::{ReccmpReportData, ReccmpReportJson, ReccmpReportType},
};

const TABLE_COLUMNS: [(&str, f32); 2] = [("Address", 120.0), ("Match", 120.0)];
const TABLE_COLUMNS_WITH_RECOMP: [(&str, f32); 3] =
    [("Address", 120.0), ("Recomp", 120.0), ("Match", 120.0)];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortColumn {
    Address,
    Recomp,
    Matching,
    Name,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

impl SortDirection {
    pub fn toggle(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }

    pub fn arrow(self) -> &'static str {
        match self {
            Self::Ascending => "▲",
            Self::Descending => "▼",
        }
    }
}

#[derive(Clone, Debug, Default)]
struct ListingFilter {
    search: String,
    hide_matched: bool,
    hide_stub: bool,
}

impl ListingFilter {
    fn matches(&self, data: &ReccmpReportData) -> bool {
        if self.hide_matched && (data.matching >= 1.0 || data.effective) {
            return false;
        }
        if self.hide_stub && data.stub {
            return false;
        }
        if !self.search.is_empty() && !self.matches_query(data) {
            return false;
        }
        true
    }

    fn matches_query(&self, data: &ReccmpReportData) -> bool {
        let q = self.search.trim().to_lowercase();
        data.name.to_lowercase().contains(&q)
            || data.address.to_string().to_lowercase().contains(&q)
            || data.recomp.to_string().to_lowercase().contains(&q)
    }
}

#[derive(Clone, Debug)]
pub struct ListingTab {
    report: ReccmpReportJson,
    filtered_indices: Vec<usize>,
    selected: Option<usize>,
    show_recomp_addr: bool,
    filter: ListingFilter,
    sort_column: SortColumn,
    sort_direction: SortDirection,
}

impl ListingTab {
    pub fn new(report: ReccmpReportJson) -> Self {
        let mut tab = Self {
            report,
            filtered_indices: Vec::new(),
            selected: None,
            show_recomp_addr: false,
            filter: ListingFilter::default(),
            sort_column: SortColumn::Address,
            sort_direction: SortDirection::Ascending,
        };
        tab.update_filter();
        tab
    }

    fn apply_sort(&mut self) {
        let data = &self.report.data;
        let sort_column = self.sort_column;
        let sort_direction = self.sort_direction;

        self.filtered_indices.sort_by(|&a_idx, &b_idx| {
            let a = &data[a_idx];
            let b = &data[b_idx];

            match sort_column {
                SortColumn::Address => {
                    let ord = a.address.cmp(&b.address);
                    if sort_direction == SortDirection::Descending {
                        ord.reverse()
                    } else {
                        ord
                    }
                }
                SortColumn::Recomp => {
                    let ord = a.recomp.cmp(&b.recomp);
                    let ord = if sort_direction == SortDirection::Descending {
                        ord.reverse()
                    } else {
                        ord
                    };
                    ord.then_with(|| a.address.cmp(&b.address))
                }
                SortColumn::Matching => {
                    let ord = a.matching.total_cmp(&b.matching);
                    let ord = if sort_direction == SortDirection::Descending {
                        ord.reverse()
                    } else {
                        ord
                    };
                    ord.then_with(|| a.address.cmp(&b.address))
                }
                SortColumn::Name => {
                    let ord = a.name.to_lowercase().cmp(&b.name.to_lowercase());
                    let ord = if sort_direction == SortDirection::Descending {
                        ord.reverse()
                    } else {
                        ord
                    };
                    ord.then_with(|| a.address.cmp(&b.address))
                }
            }
        });
    }

    fn update_filter(&mut self) {
        self.filtered_indices = self
            .report
            .data
            .iter()
            .enumerate()
            .filter(|(_, d)| self.filter.matches(d))
            .map(|(i, _)| i)
            .collect();

        self.apply_sort();
    }

    pub fn set_report(&mut self, report: ReccmpReportJson) {
        self.report = report;
        self.update_filter();
    }

    fn render_toolbar(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut self.filter.search)
                        .hint_text("Search name or address...")
                        .desired_width(ui.available_width()),
                )
                .changed();
            changed |= ui
                .checkbox(&mut self.filter.hide_matched, "Hide 100% match functions")
                .on_hover_text("This also includes effective matched functions")
                .changed();
            ui.separator();
            changed |= ui
                .checkbox(&mut self.filter.hide_stub, "Hide stub functions")
                .changed();
            ui.separator();
            changed |= ui
                .checkbox(&mut self.show_recomp_addr, "Show recomp address")
                .changed();
            ui.separator();
        });
        changed
    }

    fn render_header(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;

        ui.horizontal(|ui| {
            ui.style_mut().override_text_style = Some(TextStyle::Monospace);
            changed |= self.render_sort_button(ui, "Address", Some(120.0), SortColumn::Address);
            if self.show_recomp_addr {
                changed |= self.render_sort_button(ui, "Recomp", Some(120.0), SortColumn::Recomp);
            }
            changed |= self.render_sort_button(ui, "Matching", Some(120.0), SortColumn::Matching);
            changed |= self.render_sort_button(ui, "Name", None, SortColumn::Name);
        });

        if changed {
            self.apply_sort();
        }
    }

    fn render_sort_button(
        &mut self,
        ui: &mut egui::Ui,
        title: &str,
        width: Option<f32>,
        col: SortColumn,
    ) -> bool {
        let is_current = self.sort_column == col;
        let label = if is_current {
            format!("{title} {}", self.sort_direction.arrow())
        } else {
            title.to_string()
        };

        let btn = Button::new(RichText::new(label).strong())
            .frame_when_inactive(false)
            .selected(is_current);
        let resp = if let Some(w) = width {
            ui.add_sized([w, 18.0], btn)
        } else {
            ui.add(btn)
        };

        if resp.clicked() {
            if self.sort_column == col {
                self.sort_direction = self.sort_direction.toggle();
            } else {
                self.sort_column = col;
                self.sort_direction = SortDirection::Ascending;
            }
            return true;
        }

        false
    }
}

impl TabView for ListingTab {
    fn render(&mut self, ui: &mut egui::Ui) -> Option<UiAction> {
        let mut action = None;

        ui.vertical(|ui| {
            if self.render_toolbar(ui) {
                self.update_filter();
            }
            ui.separator();

            ui.label(
                RichText::new(format!(
                    "Functions ({} / {})",
                    self.filtered_indices.len(),
                    self.report.data.len()
                ))
                .small()
                .weak(),
            );

            self.render_header(ui);

            let mut clicked_idx = None;
            let selected = self.selected;

            let columns = if self.show_recomp_addr {
                &TABLE_COLUMNS_WITH_RECOMP[..]
            } else {
                &TABLE_COLUMNS[..]
            };

            ui.add(
                VirtualTable::new(
                    columns,
                    self.filtered_indices.len(),
                    |ui, row_idx, row_height| {
                        let data_idx = self.filtered_indices[row_idx];
                        let data = &self.report.data[data_idx];
                        let is_selected = selected == Some(data_idx);

                        let pct = data.matching * 100.0;
                        let (pct_color, pct_str) = if data.stub {
                            (Color32::GRAY, "stub".to_owned())
                        } else if data.effective && pct < 100.0 {
                            (Color32::DARK_GREEN, format!("100%* ({pct:.2}%)"))
                        } else {
                            (
                                get_matching_color(data.matching as f32),
                                format!("{pct:.2}%"),
                            )
                        };

                        let mut row = TableRow::new(columns, row_height)
                            .cell(RichText::new(data.address.to_string()).color(Color32::GRAY));
                        if self.show_recomp_addr {
                            row = row
                                .cell(RichText::new(data.recomp.to_string()).color(Color32::GRAY));
                        }
                        ui.add(
                            row.cell(RichText::new(pct_str).color(pct_color).strong())
                                .trailing_ui(&mut |ui| {
                                    let resp = ui.selectable_label(is_selected, &data.name);
                                    if resp.clicked() {
                                        clicked_idx = Some(data_idx);
                                    }
                                    resp
                                }),
                        );
                    },
                )
                .without_header(),
            );

            if let Some(idx) = clicked_idx {
                self.selected = Some(idx);
                let data = self.report.data[idx].clone();
                if data.type_ == ReccmpReportType::Vtable {
                    action = Some(UiAction::OpenVtable(data));
                } else {
                    action = Some(UiAction::Disassemble(data));
                }
            }
        });

        action
    }

    fn id(&self) -> egui::Id {
        egui::Id::new("listing")
    }

    fn title(&self) -> egui::WidgetText {
        "Listing".into()
    }
}
