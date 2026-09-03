use eframe::egui::{self, Id, WidgetText};

use crate::{
    app::{
        tab::{
            diff::DiffTab, listing::ListingTab, roadmap::RoadmapTab, stackcmp::StackcmpTab,
            vtable::VtableTab,
        },
        ui::UiAction,
    },
    disassemble::diff::DiffRow,
    reccmp::{ReccmpReportData, ReccmpReportJson},
    roadmap::RoadmapRow,
    stackcmp::StackcmpReport,
};

mod diff;
mod listing;
mod roadmap;
mod stackcmp;
mod vtable;

pub trait TabView {
    fn render(&mut self, ui: &mut egui::Ui) -> Option<UiAction>;
    fn id(&self) -> Id;
    fn title(&self) -> WidgetText;
}

pub enum Tab {
    Listing(ListingTab),
    Diff(DiffTab),
    Stackcmp(StackcmpTab),
    Roadmap(RoadmapTab),
    Vtable(VtableTab),
}

impl Tab {
    pub fn new_listing(report: ReccmpReportJson) -> Self {
        Self::Listing(ListingTab::new(report))
    }

    pub fn new_diff(func_name: String, rows: Vec<DiffRow>) -> Self {
        Self::Diff(DiffTab::new(func_name, rows))
    }

    pub fn new_stackcmp(report: StackcmpReport) -> Self {
        Self::Stackcmp(StackcmpTab::new(report))
    }

    pub fn new_roadmap(rows: Vec<RoadmapRow>) -> Self {
        Self::Roadmap(RoadmapTab::new(rows))
    }

    pub fn new_vtable(data: &ReccmpReportData) -> Self {
        Self::Vtable(VtableTab::new(data))
    }
}

impl TabView for Tab {
    fn render(&mut self, ui: &mut egui::Ui) -> Option<UiAction> {
        match self {
            Tab::Listing(tab) => tab.render(ui),
            Tab::Diff(tab) => tab.render(ui),
            Tab::Stackcmp(tab) => tab.render(ui),
            Tab::Roadmap(tab) => tab.render(ui),
            Tab::Vtable(tab) => tab.render(ui),
        }
    }

    fn id(&self) -> Id {
        match self {
            Tab::Listing(tab) => tab.id(),
            Tab::Diff(tab) => tab.id(),
            Tab::Stackcmp(tab) => tab.id(),
            Tab::Roadmap(tab) => tab.id(),
            Tab::Vtable(tab) => tab.id(),
        }
    }

    fn title(&self) -> WidgetText {
        match self {
            Tab::Listing(tab) => tab.title(),
            Tab::Diff(tab) => tab.title(),
            Tab::Stackcmp(tab) => tab.title(),
            Tab::Roadmap(tab) => tab.title(),
            Tab::Vtable(tab) => tab.title(),
        }
    }
}

pub struct TabViewer<'a> {
    actions: &'a mut Vec<UiAction>,
}

impl<'a> TabViewer<'a> {
    pub fn new(actions: &'a mut Vec<UiAction>) -> Self {
        Self { actions }
    }
}

impl egui_dock::TabViewer for TabViewer<'_> {
    type Tab = Tab;

    fn id(&mut self, tab: &mut Self::Tab) -> Id {
        tab.id()
    }

    fn title(&mut self, tab: &mut Self::Tab) -> WidgetText {
        tab.title()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        let action = tab.render(ui);
        if let Some(action) = action {
            self.actions.push(action);
        }
    }
}
