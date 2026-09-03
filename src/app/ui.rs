use std::{
    fs,
    path::{Path, PathBuf},
};

use eframe::egui::{self, ComboBox, RichText, TextStyle, menu::SubMenuButton};
use egui_dock::DockState;

use crate::{
    app::{
        App, AppError, NewProjectWindow, Project,
        config::{AppConfig, DisassemblySettings, WatchSettings},
        new_project::NewProjectAction,
        tab::{Tab, TabViewer},
    },
    reccmp::{Address, ReccmpReportData},
    worker::disassemble::DisassembleCommand,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecKind {
    Reccmp,
    Stackcmp,
    Datacmp,
    Roadmap,
}

impl ExecKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ExecKind::Reccmp => "reccmp-reccmp",
            ExecKind::Stackcmp => "reccmp-stackcmp",
            ExecKind::Datacmp => "reccmp-datacmp",
            ExecKind::Roadmap => "reccmp-roadmap",
        }
    }
}

pub enum UiAction {
    Build,
    Cancel,
    SetExecPath {
        kind: ExecKind,
        path: PathBuf,
    },
    ChangeTarget(String),
    ToggleNoLibrary,
    UpdateDisasmSettings(DisassemblySettings),
    UpdateWatchSettings(WatchSettings),
    ToggleSidebar,
    ToggleRestoreOnStartup,
    OpenNewProjectWindow,
    OpenConfig(PathBuf),
    SaveAndOpenProject {
        config: AppConfig,
        project_dir: PathBuf,
    },
    Datacmp,
    Roadmap,
    Disassemble(ReccmpReportData),
    Stackcmp {
        address: Address,
        func_name: String,
    },
    OpenVtable(ReccmpReportData),
    OpenFunctionByName(String),
    OpenVtableByName(String),
}

impl App {
    fn render_toolbar(&self, ui: &mut egui::Ui, actions: &mut Vec<UiAction>) {
        egui::MenuBar::new().ui(ui, |ui| {
            if ui
                .button(if self.settings.show_sidebar { "⏴" } else { "⏵" })
                .clicked()
            {
                actions.push(UiAction::ToggleSidebar);
            }
            ui.menu_button("File", |ui| {
                if ui.button("New").clicked() {
                    actions.push(UiAction::OpenNewProjectWindow);
                }

                SubMenuButton::new("Open").ui(ui, |ui| {
                    if ui.button("Browse...").clicked()
                        && let Some(path) = rfd::FileDialog::new().pick_file()
                    {
                        actions.push(UiAction::OpenConfig(path));
                    }

                    for path in &self.settings.recent_projects {
                        if ui.button(path.to_string_lossy()).clicked() {
                            actions.push(UiAction::OpenConfig(path.clone()));
                        }
                    }
                });
                if ui.button("Quit").clicked() {
                    ui.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Tool", |ui| {
                if self.settings.datacmp.is_some()
                    && ui
                        .button("reccmp-datacmp")
                        .on_hover_text(
                            "This tool will take away at your lifespan if you don't have large variables properly ignored in your reccmp-project.yml.",
                        )
                        .clicked()
                {
                    actions.push(UiAction::Datacmp);
                }
                if self.settings.roadmap.is_some() && ui.button("reccmp-roadmap").clicked() {
                    actions.push(UiAction::Roadmap);
                }
            });
        });
    }

    fn render_sidebar(&self, ui: &mut egui::Ui, actions: &mut Vec<UiAction>) {
        ui.style_mut().spacing.item_spacing.x = 4.0;
        self.render_project_status(ui, actions);
        ui.separator();
        self.render_tool_location_settings(ui, actions);
        ui.separator();
        self.render_watch_settings(ui, actions);
        ui.separator();
        self.render_project_settings(ui, actions);
    }

    fn render_project_status(&self, ui: &mut egui::Ui, actions: &mut Vec<UiAction>) {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);

        ui.heading("Project");
        ui.horizontal(|ui| {
            ui.label("Status:");
            if let Some(project) = &self.project {
                ui.label(format!("Loaded ({})", project.dir.display()));
            } else {
                ui.label("Not loaded");
            }
        });
        ui.horizontal(|ui| {
            ui.label("Tool status:");
            if let Some(project) = &self.project {
                let state = &project.tool_state;
                if state.cancelling {
                    ui.label("Cancelling...");
                    ui.spinner();
                } else if state.compiling.is_some() {
                    ui.label("Compiling...");
                    ui.spinner();
                } else if state.is_idle() {
                    ui.label("Idle");
                } else {
                    let mut running = Vec::new();
                    if state.reccmp.is_some() {
                        running.push("reccmp");
                    }
                    if state.roadmap.is_some() {
                        running.push("roadmap");
                    }
                    if state.datacmp.is_some() {
                        running.push("datacmp");
                    }
                    if !state.stackcmp.is_empty() {
                        running.push("stackcmp");
                    }

                    ui.label(format!("Running: {}", running.join(", ")));
                    ui.spinner();
                }
            } else {
                ui.label("Project not loaded");
            }
        });
        let mut restore_on_setup = self.settings.restore_on_startup;
        ui.checkbox(&mut restore_on_setup, "Restore on startup");
        if restore_on_setup != self.settings.restore_on_startup {
            actions.push(UiAction::ToggleRestoreOnStartup);
        }
    }

    fn render_tool_location_settings(&self, ui: &mut egui::Ui, actions: &mut Vec<UiAction>) {
        ui.heading("reccmp");
        Self::browse_executable(
            ui,
            ExecKind::Reccmp,
            self.settings.reccmp.as_deref(),
            actions,
        );
        Self::browse_executable(
            ui,
            ExecKind::Stackcmp,
            self.settings.stackcmp.as_deref(),
            actions,
        );
        Self::browse_executable(
            ui,
            ExecKind::Datacmp,
            self.settings.datacmp.as_deref(),
            actions,
        );
        Self::browse_executable(
            ui,
            ExecKind::Roadmap,
            self.settings.roadmap.as_deref(),
            actions,
        );
    }

    fn render_watch_settings(&self, ui: &mut egui::Ui, actions: &mut Vec<UiAction>) {
        ui.heading("Watch");

        let mut watch_settings = self.settings.watch;
        let mut changed = false;
        changed |= ui.checkbox(&mut watch_settings.active, "Watch source root directory for changes")
            .on_hover_text("Recompiles and reruns tools when a change is detected in the source root directory").changed();
        changed |= ui
            .add_enabled_ui(watch_settings.active, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Debounce timeout:");
                    ui.add(
                        egui::DragValue::new(&mut watch_settings.debounce_ms)
                            .range(10..=5000)
                            .speed(5)
                            .suffix(" ms"),
                    )
                    .changed()
                })
                .inner
            })
            .inner;
        if changed {
            actions.push(UiAction::UpdateWatchSettings(watch_settings));
        }
    }

    fn render_project_settings(&self, ui: &mut egui::Ui, actions: &mut Vec<UiAction>) {
        let Some(project) = &self.project else {
            return;
        };

        ui.heading("Build");
        let mut cur_target = project.target.clone();
        ui.horizontal(|ui| {
            ui.label("Target:");
            ComboBox::from_id_salt("target")
                .selected_text(&project.target)
                .show_ui(ui, |ui| {
                    for target in project.project_yml.targets.keys() {
                        ui.selectable_value(&mut cur_target, target.clone(), target);
                    }
                });
        });

        if project.target != cur_target {
            actions.push(UiAction::ChangeTarget(cur_target));
        }

        let mut no_library = self.settings.no_library;
        ui.checkbox(&mut no_library, "No library in reccmp summary")
            .on_hover_text("Excludes LIBRARY annotated functions from reccmp analysis");
        if no_library != self.settings.no_library {
            actions.push(UiAction::ToggleNoLibrary);
        }
        let mut disasm_settings = self.settings.disassembly;
        let mut changed = false;
        changed |= ui.checkbox(
                &mut disasm_settings.relative_jump,
                "Use relative jump offsets",
            )
            .on_hover_text(
                "Replaces absolute jump offsets (e.g. 0x00416600: jmp 0x00416700) with relative offsets (e.g. jmp 0x100)",
            ).changed();
        changed |= ui
            .checkbox(&mut disasm_settings.resolve_symbols, "Resolve symbols")
            .on_hover_text("Resolve some addresses to functions in disassembly")
            .changed();
        changed |= ui
            .add_enabled(
                disasm_settings.resolve_symbols,
                egui::Checkbox::new(
                    &mut disasm_settings.use_roadmap,
                    "Use roadmap to improve disassembly analysis",
                ),
            )
            .on_hover_text(
                "Uses data from reccmp-roadmap to replace addresses with symbols in disassembly",
            )
            .changed();
        changed |= ui.add_enabled(disasm_settings.use_roadmap, egui::Checkbox::new(
                &mut disasm_settings.use_roadmap_func_sizes,
                "Use roadmap for determining function sizes",
            ))
            .on_hover_text(
                "Uses roadmap's csvs to determine when to stop disassembling a function. Note that this may be inaccurate, since if there are no function sizes defined in the csvs, reccmp falls back to a quite conservative estimate for determining the end of a function (it stops only on INT3), while we use a more eager estimate (we stop at RET or INT3). As a result you might find your functions to be too big if it is compiled with Os, where there isn't any padding.",
            ).changed();
        changed |= ui.add_enabled(disasm_settings.use_roadmap, egui::Checkbox::new(
                &mut disasm_settings.resolve_global_offsets,
                "Resolve global data offsets",
            ))
            .on_hover_text(
                "Resolves addresses that fall within a global's range (e.g. 0x575adc) into an offset (e.g. g_Supervisor+8). May have false positives for big numbers.",
            ).changed();
        changed |= ui.add_enabled(disasm_settings.use_roadmap && disasm_settings.resolve_global_offsets, egui::Checkbox::new(
                &mut disasm_settings.display_global_offsets_hex,
                "Display global data offsets as hex",
            ))
            .on_hover_text(
                "Displays global data offsets (e.g. g_Supervisor+12) into as hex (e.g. g_Supervisor+0xc).",
            ).changed();
        if changed {
            actions.push(UiAction::UpdateDisasmSettings(disasm_settings));
        }

        ui.separator();

        ui.add_enabled_ui(
            (self.settings.reccmp.is_some() && !project.target.is_empty())
                || !project.tool_state.is_idle(),
            |ui| {
                let can_cancel = !project.tool_state.is_idle() && !project.tool_state.cancelling;

                if project.tool_state.is_idle() {
                    if ui.button("Rebuild").clicked() {
                        actions.push(UiAction::Build);
                    }
                } else if ui
                    .add_enabled(can_cancel, egui::Button::new("Cancel"))
                    .clicked()
                {
                    actions.push(UiAction::Cancel);
                }
            },
        );
    }

    fn render_log(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Console").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Clear").clicked() {
                    self.logs.clear();
                }
            });
        });
        ui.separator();

        let text_style = TextStyle::Monospace;
        let row_height = ui.text_style_height(&text_style);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show_rows(ui, row_height, self.logs.len(), |ui, row_range| {
                ui.style_mut().override_text_style = Some(TextStyle::Monospace);
                for log in &self.logs[row_range] {
                    ui.label(RichText::new(log).size(11.0));
                }
            });
    }

    fn render_error(&mut self, ui: &mut egui::Ui) {
        if self.errors.is_empty() {
            return;
        }

        let mut open = true;

        egui::Window::new("Error")
            .collapsible(false)
            .resizable(true)
            .open(&mut open)
            .show(ui, |ui| {
                ui.colored_label(egui::Color32::RED, "Error:");
                for error in &self.errors {
                    ui.add(egui::Label::new(error).wrap());
                }

                ui.separator();

                if ui.button("OK cool").clicked() {
                    self.errors.clear();
                }
            });

        if !open {
            self.errors.clear();
        }
    }

    fn browse_executable(
        ui: &mut egui::Ui,
        kind: ExecKind,
        current_path: Option<&Path>,
        actions: &mut Vec<UiAction>,
    ) {
        ui.vertical(|ui| {
            ui.label(format!(
                "{}: {}",
                kind.as_str(),
                if current_path.is_some() {
                    "Found!"
                } else {
                    "Missing"
                }
            ));
            if ui.button("Browse...").clicked()
                && let Some(path) = rfd::FileDialog::new().pick_file()
            {
                actions.push(UiAction::SetExecPath { kind, path });
            }
        });
    }

    fn handle_actions(&mut self, actions: Vec<UiAction>) {
        for action in actions {
            if let Err(e) = self.handle_action(action) {
                self.errors.push_back(e.to_string());
            }
        }
    }

    fn handle_action(&mut self, action: UiAction) -> Result<(), AppError> {
        match action {
            UiAction::Build => self.trigger_build()?,
            UiAction::Cancel => self.cancel_tools(),
            UiAction::SetExecPath { kind, path } => match kind {
                ExecKind::Reccmp => self.settings.reccmp = Some(path),
                ExecKind::Stackcmp => self.settings.stackcmp = Some(path),
                ExecKind::Datacmp => self.settings.datacmp = Some(path),
                ExecKind::Roadmap => self.settings.roadmap = Some(path),
            },
            UiAction::ChangeTarget(target) => {
                self.trigger_change_target(target)?;
            }
            UiAction::ToggleNoLibrary => {
                self.settings.no_library = !self.settings.no_library;
            }
            UiAction::UpdateDisasmSettings(settings) => {
                self.update_disasm_settings(settings)?;
            }
            UiAction::UpdateWatchSettings(settings) => {
                self.update_watch_settings(settings)?;
            }
            UiAction::ToggleSidebar => {
                self.settings.show_sidebar = !self.settings.show_sidebar;
            }
            UiAction::ToggleRestoreOnStartup => {
                self.settings.restore_on_startup = !self.settings.restore_on_startup;
            }
            UiAction::OpenNewProjectWindow => {
                self.new_project_window = Some(NewProjectWindow::default());
            }
            UiAction::OpenConfig(path) => {
                self.open_config(&path)?;
                self.settings.recent_projects.retain(|p| p != &path);
                self.settings.recent_projects.insert(0, path);
            }
            UiAction::SaveAndOpenProject {
                config,
                project_dir,
            } => {
                let config_path = project_dir.join("reccmp-gui.yml");
                let str = yaml_serde::to_string(&config).map_err(|e| AppError::YamlWrite {
                    path: config_path.clone(),
                    source: e,
                })?;
                fs::write(&config_path, str).map_err(|e| AppError::WriteFile {
                    path: config_path.clone(),
                    source: e,
                })?;
                self.cancel_tools();
                self.project = Some(Project::new(config, project_dir)?);
                self.settings.current_project = Some(config_path.clone());
                self.settings.current_target = None;
                self.dock = DockState::new(Vec::new());
                self.roadmap_rows = None;
                self.last_report = None;
                self.tx_disasm
                    .send(DisassembleCommand::UpdateRoadmap(None))?;
                self.settings.recent_projects.retain(|p| p != &config_path);
                self.settings.recent_projects.insert(0, config_path);
            }
            UiAction::Disassemble(data) => {
                self.trigger_disassemble(data, true)?;
            }
            UiAction::Stackcmp { address, func_name } => {
                self.trigger_stackcmp(address, func_name, true)?;
            }
            UiAction::Datacmp => {
                self.trigger_datacmp()?;
            }
            UiAction::Roadmap => {
                self.trigger_roadmap(true)?;
            }
            UiAction::OpenVtable(data) => {
                self.open_vtable(&data)?;
            }
            UiAction::OpenFunctionByName(name) => {
                if let Some(report) = &self.last_report
                    && let Some(data) = report.data.iter().find(|d| d.name == name)
                {
                    self.trigger_disassemble(data.clone(), true)?;
                } else {
                    self.logs.push(format!(
                        "function {name} not found (perhaps its not in the reccmp listing)"
                    ));
                }
            }
            UiAction::OpenVtableByName(name) => {
                if let Some(report) = &self.last_report
                    && let Some(data) = report.data.iter().find(|d| d.name == name)
                {
                    self.open_vtable(&data.clone())?;
                } else {
                    self.logs.push(format!(
                        "vtable {name} not found (perhaps its not in the reccmp listing)"
                    ));
                }
            }
        }

        Ok(())
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.handle_messages(ui);

        let mut actions = Vec::new();

        egui::Panel::top("toolbar").show(ui, |ui| {
            self.render_toolbar(ui, &mut actions);
        });

        if let Some(mut window) = self.new_project_window.take() {
            match window.render(ui) {
                Some(NewProjectAction::SaveAndOpen {
                    config,
                    project_dir,
                }) => {
                    actions.push(UiAction::SaveAndOpenProject {
                        config,
                        project_dir,
                    });
                }
                Some(NewProjectAction::Close) => self.new_project_window = None,
                None => self.new_project_window = Some(window),
            }
        }

        let mut show_sidebar = self.settings.show_sidebar;
        egui::Panel::left("sidebar").show_collapsible(ui, &mut show_sidebar, |ui| {
            self.render_sidebar(ui, &mut actions);
        });
        egui::Panel::bottom("logs")
            .resizable(true)
            .min_size(60.0)
            .show(ui, |ui| {
                self.render_log(ui);
            });
        egui::CentralPanel::default().show(ui, |ui| {
            egui_dock::DockArea::new(&mut self.dock)
                .style(egui_dock::Style::from_egui(ui.style().as_ref()))
                .show_inside(ui, &mut TabViewer::new(&mut actions));
            self.render_error(ui);
        });

        self.handle_actions(actions);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.settings.dock_state = Some(self.dock.map_tabs(Tab::to_kind));
        eframe::set_value(storage, eframe::APP_KEY, &self.settings);
    }
}
