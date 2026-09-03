use eframe::egui;

use crate::{
    app::{App, AppError, ToolState, tab::Tab},
    disassemble::Disassembly,
    reccmp::ReccmpReportJson,
    roadmap::RoadmapRow,
    stackcmp::StackcmpReport,
    worker::DisassembleCommand,
};

pub enum Message {
    Log(String),
    BuildFinished(Result<ReccmpReportJson, String>),
    StackcmpFinished(StackcmpReport),
    SourceFileChanged,
    DisassembleFinished(Disassembly),
    DatacmpFinished,
    RoadmapFinished { rows: Vec<RoadmapRow>, focus: bool },
    SetToolState(ToolState),
    Error(String),
}

impl App {
    pub fn handle_messages(&mut self, ui: &mut egui::Ui) {
        while let Ok(msg) = self.rx_msg.try_recv() {
            if let Err(e) = self.handle_message(msg) {
                self.errors.push_back(e.to_string());
            }
            ui.request_repaint();
        }
    }

    pub fn handle_message(&mut self, msg: Message) -> Result<(), AppError> {
        match msg {
            Message::Log(log) => self.logs.push(log),
            Message::BuildFinished(result) => match result {
                Ok(report) => self.handle_build_finished(report)?,
                Err(e) => self.logs.push(e),
            },
            Message::StackcmpFinished(report) => self.handle_stackcmp_finished(report)?,
            Message::SourceFileChanged => {
                if let Some(project) = &self.project
                    && project.tool_state == ToolState::Idle
                {
                    self.trigger_build()?;
                }
            }
            Message::DisassembleFinished(disasm) => self.handle_disassemble_finished(disasm)?,
            Message::DatacmpFinished => self.handle_datacmp_finished()?,
            Message::RoadmapFinished { rows, focus } => {
                self.handle_roadmap_finished(rows, focus)?;
            }
            Message::SetToolState(state) => {
                if let Some(project) = &mut self.project {
                    project.tool_state = state;
                }
            }
            Message::Error(e) => {
                if let Some(project) = &mut self.project {
                    project.tool_state = ToolState::Idle;
                }
                self.errors.push_back(e);
            }
        }

        Ok(())
    }

    pub fn handle_build_finished(&mut self, report: ReccmpReportJson) -> Result<(), AppError> {
        let mut listing_exists = false;
        let mut diff_tabs = Vec::new();
        let mut stackcmp_tabs = Vec::new();
        let mut has_roadmap = false;

        let project = self.project.as_mut().ok_or(AppError::ProjectNotSet)?;
        if project.tool_state == ToolState::Building || project.tool_state == ToolState::Cancelling
        {
            project.tool_state = ToolState::Idle;
        }

        self.tx_disasm
            .send(DisassembleCommand::UpdateResolvers(report.data.clone()))?;

        for (_, tab) in self.dock.iter_all_tabs_mut() {
            match tab {
                Tab::Listing(state) => {
                    listing_exists = true;
                    state.set_report(report.clone());
                }
                Tab::Diff(state) => {
                    let data = report.data.iter().find(|x| x.name == state.get_name());
                    if let Some(data) = data {
                        if data.diff.is_none() {
                            state.mark_removed();
                            continue;
                        }

                        diff_tabs.push(data.clone());
                    } else {
                        state.clear_rows();
                    }
                }
                Tab::Stackcmp(state) => {
                    stackcmp_tabs.push((state.report.address, state.report.func_name.clone()));
                }
                Tab::Roadmap(_) => {
                    has_roadmap = true;
                }
                Tab::Vtable(state) => {
                    if let Some(data) = report.data.iter().find(|x| x.name == state.get_name()) {
                        state.update_from_data(data);
                    } else {
                        state.mark_removed();
                    }
                }
            }
        }
        if !listing_exists {
            self.dock
                .push_to_first_leaf(Tab::new_listing(report.clone()));
        }

        self.last_report = Some(report);

        for data in diff_tabs {
            self.trigger_disassemble(data, false)?;
        }

        // I LOVE YOU BORROW CHECKER!!!!
        if (self.settings.use_roadmap || has_roadmap) && self.settings.roadmap.is_some() {
            self.roadmap_rows = None;
            self.trigger_roadmap(false)?;
        }

        for (address, func_name) in stackcmp_tabs {
            self.trigger_stackcmp(address, func_name)?;
        }

        Ok(())
    }

    pub fn handle_stackcmp_finished(&mut self, report: StackcmpReport) -> Result<(), AppError> {
        let project = self.project.as_mut().ok_or(AppError::ProjectNotSet)?;
        if project.tool_state == ToolState::Stackcmp || project.tool_state == ToolState::Cancelling
        {
            project.tool_state = ToolState::Idle;
        }

        let mut stackcmp_tab_path = None;
        if let Some(tab_path) = self
            .dock
            .iter_all_tabs_mut()
            .find(|(_, tab)| {
                if let Tab::Stackcmp(state) = tab {
                    state.report.address == report.address
                } else {
                    false
                }
            })
            .map(|(tab_path, _)| tab_path)
        {
            stackcmp_tab_path = Some(tab_path);
        }

        if let Some(tab_path) = stackcmp_tab_path {
            self.dock.set_active_tab(tab_path)?;
        } else {
            self.dock.push_to_focused_leaf(Tab::new_stackcmp(report));
        }

        Ok(())
    }

    pub fn handle_disassemble_finished(&mut self, disasm: Disassembly) -> Result<(), AppError> {
        let project = self.project.as_mut().ok_or(AppError::ProjectNotSet)?;
        if project.tool_state == ToolState::Disassembling
            || project.tool_state == ToolState::Cancelling
        {
            project.tool_state = ToolState::Idle;
        }

        let mut func_tab_path = None;
        for (tab_path, tab) in self.dock.iter_all_tabs_mut() {
            if let Tab::Diff(state) = tab
                && state.get_name() == disasm.func_name
            {
                state.set_rows(disasm.rows.clone());
                func_tab_path = Some(tab_path);
            }
        }

        if let Some(tab_path) = func_tab_path
            && disasm.focus
        {
            self.dock.set_active_tab(tab_path)?;
        } else if disasm.focus {
            self.dock
                .push_to_focused_leaf(Tab::new_diff(disasm.func_name, disasm.rows));
        }

        Ok(())
    }

    pub fn handle_datacmp_finished(&mut self) -> Result<(), AppError> {
        let project = self.project.as_mut().ok_or(AppError::ProjectNotSet)?;
        if project.tool_state == ToolState::Datacmp || project.tool_state == ToolState::Cancelling {
            project.tool_state = ToolState::Idle;
        }
        // TODO: maybe when datacmp becomes more substantial we can have a special tab for it
        Ok(())
    }

    pub fn handle_roadmap_finished(
        &mut self,
        rows: Vec<RoadmapRow>,
        focus: bool,
    ) -> Result<(), AppError> {
        let project = self.project.as_mut().ok_or(AppError::ProjectNotSet)?;
        if project.tool_state == ToolState::Roadmap || project.tool_state == ToolState::Cancelling {
            project.tool_state = ToolState::Idle;
        }

        self.roadmap_rows = Some(rows.clone());
        if self.settings.use_roadmap {
            self.tx_disasm
                .send(DisassembleCommand::UpdateRoadmap(Some(rows.clone())))?;
            self.refresh_diff_tabs()?;
        }

        let mut roadmap_tab_path = None;
        for (tab_path, tab) in self.dock.iter_all_tabs_mut() {
            if let Tab::Roadmap(state) = tab {
                state.set_rows(rows.clone());
                if focus {
                    roadmap_tab_path = Some(tab_path);
                }
                break;
            }
        }

        if let Some(tab_path) = roadmap_tab_path {
            self.dock.set_active_tab(tab_path)?;
        } else if focus {
            self.dock.push_to_focused_leaf(Tab::new_roadmap(rows));
        }

        Ok(())
    }
}
