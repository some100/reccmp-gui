use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};

use crate::{
    app::{App, AppError, config::Tool, tab::Tab},
    disassemble::Disassembly,
    reccmp::ReccmpReportJson,
    roadmap::RoadmapRow,
    stackcmp::StackcmpReport,
    worker::disassemble::DisassembleCommand,
};

pub enum Message {
    Log(String),
    CompileFinished(u64),
    ReccmpFinished {
        report: ReccmpReportJson,
        generation: u64,
    },
    StackcmpFinished {
        report: StackcmpReport,
        focus: bool,
        generation: u64,
    },
    SourceFileChanged,
    DisassembleFinished(Disassembly),
    DatacmpFinished(u64),
    RoadmapFinished {
        rows: Vec<RoadmapRow>,
        focus: bool,
        generation: u64,
    },
    Error(String),
    ToolFailed {
        tool: Tool,
        generation: u64,
    },
    ToolCancelled {
        tool: Tool,
        generation: u64,
    },
}

impl App {
    pub fn handle_messages(&mut self, ui: &mut egui::Ui) {
        while let Ok(msg) = self.rx_msg.try_recv() {
            if let Err(e) = self.handle_message(msg) {
                self.errors.push_back(e.to_string());
            }
            ui.request_repaint();
        }

        ui.input_mut(|i| self.handle_input(i));
    }

    pub fn handle_message(&mut self, msg: Message) -> Result<(), AppError> {
        match msg {
            Message::Log(log) => self.logs.push(log),
            Message::CompileFinished(generation) => {
                let was_cancelling = self
                    .project
                    .as_ref()
                    .is_some_and(|project| project.tool_state.cancelling);

                if let Some(project) = &mut self.project {
                    project.tool_state.compiling = None;
                    project.tool_state.update_cancelling();
                }

                if self.generation != generation {
                    return Ok(());
                }

                self.tool_cancels.compile.take();

                if !was_cancelling {
                    self.trigger_tools();
                }
            }
            Message::ReccmpFinished { report, generation } => {
                if let Some(project) = &mut self.project {
                    project.tool_state.reccmp = None;
                    project.tool_state.update_cancelling();
                }

                if self.generation != generation {
                    return Ok(());
                }

                self.tool_cancels.reccmp.take();

                self.handle_reccmp_finished(report)?;
            }
            Message::StackcmpFinished {
                report,
                focus,
                generation,
            } => {
                if let Some(project) = &mut self.project
                    && project.tool_state.stackcmp.get(&report.address) == Some(&generation)
                {
                    project.tool_state.stackcmp.remove(&report.address);
                    project.tool_state.update_cancelling();
                }

                if self.generation != generation {
                    return Ok(());
                }

                self.tool_cancels.stackcmp.remove(&report.address);

                self.handle_stackcmp_finished(report, focus)?;
            }
            Message::SourceFileChanged => {
                if self.settings.watch.active {
                    self.trigger_build()?;
                }
            }
            Message::DisassembleFinished(disasm) => self.handle_disassemble_finished(disasm)?,
            Message::DatacmpFinished(generation) => {
                if let Some(project) = &mut self.project {
                    project.tool_state.datacmp = None;
                    project.tool_state.update_cancelling();
                }

                if self.generation != generation {
                    return Ok(());
                }

                self.tool_cancels.datacmp.take();

                self.handle_datacmp_finished()?;
            }
            Message::RoadmapFinished {
                rows,
                focus,
                generation,
            } => {
                if self.generation != generation {
                    return Ok(());
                }

                self.tool_cancels.roadmap.take();
                if let Some(project) = &mut self.project {
                    project.tool_state.roadmap = None;
                    project.tool_state.update_cancelling();
                }
                self.handle_roadmap_finished(rows, focus)?;
            }
            Message::Error(e) => {
                self.errors.push_back(e);
            }
            Message::ToolFailed { tool, generation } => {
                self.handle_tool_failed(tool, generation);
            }
            Message::ToolCancelled { tool, generation } => {
                self.handle_tool_cancelled(tool, generation);
            }
        }

        Ok(())
    }

    pub fn handle_input(&mut self, input: &mut egui::InputState) {
        if input.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::W))
            && let Some(leaf) = self.dock.focused_leaf()
            && let Some(node) = self.dock[leaf].get_leaf_mut()
        {
            node.remove_tab(node.active);
        }

        if input.consume_shortcut(&KeyboardShortcut::new(
            Modifiers::COMMAND | Modifiers::SHIFT,
            Key::Tab,
        )) && let Some(leaf) = self.dock.focused_leaf()
            && let Some(node) = self.dock[leaf].get_leaf_mut()
        {
            if node.set_active_tab(node.active.0 - 1).is_err() {
                let _ = node.set_active_tab(node.tabs().len() - 1);
            }
        }

        if input.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Tab))
            && let Some(leaf) = self.dock.focused_leaf()
            && let Some(node) = self.dock[leaf].get_leaf_mut()
        {
            if node.set_active_tab(node.active.0 + 1).is_err() {
                let _ = node.set_active_tab(0);
            }
        }
    }

    pub fn handle_reccmp_finished(&mut self, report: ReccmpReportJson) -> Result<(), AppError> {
        let mut listing_exists = false;
        let mut diff_tabs = Vec::new();

        self.tx_disasm
            .send(DisassembleCommand::UpdateResolvers(report.data.clone()))?;

        for (_, tab) in self.dock.iter_all_tabs_mut() {
            match tab {
                Tab::Listing(state) => {
                    listing_exists = true;
                    state.set_report(report.clone());
                }
                Tab::Diff(state) => {
                    state.set_error(false);
                    let data = report.data.iter().find(|x| x.name == state.get_name());
                    if let Some(data) = data {
                        state.set_removed(false);
                        state.set_matching(data.matching);
                        diff_tabs.push(data.clone());
                    } else {
                        state.set_removed(true);
                    }
                }
                Tab::Vtable(state) => {
                    if let Some(data) = report.data.iter().find(|x| x.name == state.get_name()) {
                        state.set_removed(false);
                        state.update_from_data(data);
                    } else {
                        state.set_removed(true);
                    }
                }
                _ => (),
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

        Ok(())
    }

    pub fn handle_stackcmp_finished(
        &mut self,
        report: StackcmpReport,
        focus: bool,
    ) -> Result<(), AppError> {
        let mut stackcmp_tab_path = None;
        for (tab_path, tab) in self.dock.iter_all_tabs_mut() {
            if let Tab::Stackcmp(state) = tab
                && state.report.address == report.address
            {
                state.update_report(report.clone());
                stackcmp_tab_path = Some(tab_path);
                break;
            }
        }

        if let Some(tab_path) = stackcmp_tab_path
            && focus
        {
            self.dock.set_active_tab(tab_path)?;
        } else if focus {
            self.dock.push_to_focused_leaf(Tab::new_stackcmp(report));
        }

        Ok(())
    }

    pub fn handle_disassemble_finished(&mut self, disasm: Disassembly) -> Result<(), AppError> {
        let mut func_tab_path = None;
        for (tab_path, tab) in self.dock.iter_all_tabs_mut() {
            if let Tab::Diff(state) = tab
                && state.get_name() == disasm.func_name
            {
                state.set_removed(false);
                state.set_matching(disasm.matching);
                state.set_rows(disasm.rows.clone());
                state.set_tables(disasm.tables.clone());
                func_tab_path = Some(tab_path);
            }
        }

        if let Some(tab_path) = func_tab_path
            && disasm.focus
        {
            self.dock.set_active_tab(tab_path)?;
        } else if disasm.focus {
            self.dock.push_to_focused_leaf(Tab::new_diff(
                disasm.func_name,
                disasm.rows,
                disasm.matching,
                disasm.tables,
            ));
        }

        Ok(())
    }

    pub fn handle_datacmp_finished(&mut self) -> Result<(), AppError> {
        // TODO: maybe when datacmp becomes more substantial we can have a special tab for it
        Ok(())
    }

    pub fn handle_roadmap_finished(
        &mut self,
        rows: Vec<RoadmapRow>,
        focus: bool,
    ) -> Result<(), AppError> {
        self.roadmap_rows = Some(rows.clone());
        if self.settings.disassembly.use_roadmap {
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

    fn handle_tool_failed(&mut self, tool: Tool, generation: u64) {
        match tool {
            Tool::Compile => {
                self.tool_cancels.compile.take();
                for (_, tab) in self.dock.iter_all_tabs_mut() {
                    match tab {
                        Tab::Diff(state) => state.set_error(true),
                        Tab::Stackcmp(state) => state.set_error(true),
                        _ => (),
                    }
                }
            }
            Tool::Reccmp => {
                for (_, tab) in self.dock.iter_all_tabs_mut() {
                    if let Tab::Diff(state) = tab {
                        state.set_error(true);
                    }
                }
                self.tool_cancels.reccmp.take();
            }
            Tool::Stackcmp(address) => {
                self.tool_cancels.stackcmp.remove(&address);
                for (_, tab) in self.dock.iter_all_tabs_mut() {
                    if let Tab::Stackcmp(state) = tab
                        && state.report.address == address
                    {
                        state.set_error(true);
                    }
                }
            }
            Tool::Datacmp => {
                self.tool_cancels.datacmp.take();
            }
            Tool::Roadmap => {
                self.tool_cancels.roadmap.take();
            }
        }

        if let Some(project) = &mut self.project {
            match tool {
                Tool::Compile => {
                    if project.tool_state.compiling == Some(generation) {
                        project.tool_state.compiling = None;
                    }
                }
                Tool::Reccmp => {
                    if project.tool_state.reccmp == Some(generation) {
                        project.tool_state.reccmp = None;
                    }
                }
                Tool::Stackcmp(address) => {
                    if project.tool_state.stackcmp.get(&address) == Some(&generation) {
                        project.tool_state.stackcmp.remove(&address);
                    }
                }
                Tool::Datacmp => {
                    if project.tool_state.datacmp == Some(generation) {
                        project.tool_state.datacmp = None;
                    }
                }
                Tool::Roadmap => {
                    if project.tool_state.roadmap == Some(generation) {
                        project.tool_state.roadmap = None;
                    }
                }
            }
            project.tool_state.update_cancelling();
        }
    }

    fn handle_tool_cancelled(&mut self, tool: Tool, generation: u64) {
        if let Some(project) = &mut self.project {
            match tool {
                Tool::Compile => {
                    if project.tool_state.compiling == Some(generation) {
                        project.tool_state.compiling = None;
                    }
                }
                Tool::Reccmp => {
                    if project.tool_state.reccmp == Some(generation) {
                        project.tool_state.reccmp = None;
                    }
                }
                Tool::Stackcmp(address) => {
                    if project.tool_state.stackcmp.get(&address) == Some(&generation) {
                        project.tool_state.stackcmp.remove(&address);
                    }
                }
                Tool::Datacmp => {
                    if project.tool_state.datacmp == Some(generation) {
                        project.tool_state.datacmp = None;
                    }
                }
                Tool::Roadmap => {
                    if project.tool_state.roadmap == Some(generation) {
                        project.tool_state.roadmap = None;
                    }
                }
            }
            project.tool_state.update_cancelling();
        }
    }
}
