use eframe::{
    egui::{self, Color32, RichText},
    epaint::Hsva,
};

use crate::{
    app::{
        tab::TabView,
        ui::UiAction,
        widgets::{TableRow, VirtualTable},
    },
    reccmp::{ReccmpReportJson, ReccmpReportType},
};

const TABLE_COLUMNS: [(&str, f32); 2] = [("Address", 90.0), ("Match", 120.0)];

pub struct ListingTab {
    report: ReccmpReportJson,
    search_query: String,
    filtered_indices: Vec<usize>,
    selected: Option<usize>,
}

impl ListingTab {
    pub fn new(report: ReccmpReportJson) -> Self {
        let mut tab = Self {
            report,
            search_query: String::new(),
            filtered_indices: Vec::new(),
            selected: None,
        };
        tab.update_filter();
        tab
    }

    fn update_filter(&mut self) {
        let query = self.search_query.trim().to_lowercase();
        if query.is_empty() {
            self.filtered_indices = (0..self.report.data.len()).collect();
        } else {
            self.filtered_indices = self
                .report
                .data
                .iter()
                .enumerate()
                .filter(|(_, d)| {
                    d.name.to_lowercase().contains(&query)
                        || d.address.to_string().contains(&query)
                        || d.recomp.to_string().contains(&query)
                })
                .map(|(i, _)| i)
                .collect();
        }
    }

    pub fn set_report(&mut self, report: ReccmpReportJson) {
        self.report = report;
    }
}

impl TabView for ListingTab {
    fn render(&mut self, ui: &mut egui::Ui) -> Option<UiAction> {
        let mut action = None;

        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.search_query)
                            .hint_text("Search name or address...")
                            .desired_width(ui.available_width()),
                    )
                    .changed()
                {
                    self.update_filter();
                }
            });
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

            let mut clicked_idx = None;
            let selected = self.selected;

            ui.add(
                VirtualTable::new(
                    &TABLE_COLUMNS,
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

                        ui.add(
                            TableRow::new(&TABLE_COLUMNS, row_height)
                                .cell(RichText::new(data.address.to_string()).color(Color32::GRAY))
                                .cell(RichText::new(pct_str).color(pct_color).strong())
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
                .with_trailing("Name"),
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

fn get_matching_color(matching: f32) -> Color32 {
    let red = Hsva::from(Color32::DARK_RED);
    let green = Hsva::from(Color32::DARK_GREEN);

    Hsva::new(
        (green.h - red.h) * matching + red.h,
        (green.s - red.s) * matching + red.s,
        (green.v - red.v) * matching + red.v,
        1.0,
    )
    .into()
}
