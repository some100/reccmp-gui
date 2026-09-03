use core::sync::atomic::Ordering;
use std::{
    fs,
    path::{Path, PathBuf},
};

use eframe::egui::{self, ComboBox, RichText, TextStyle, menu::SubMenuButton};
use egui_dock::DockState;

use crate::{
    app::{
        App, AppConfig, AppError, NewProjectWindow, Project, ToolState,
        new_project::NewProjectAction, tab::TabViewer,
    },
    reccmp::{Address, ReccmpReportData},
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
    ToggleUseRoadmap,
    ToggleSidebar,
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
}

impl App {
    fn render_toolbar(&self, ui: &mut egui::Ui, actions: &mut Vec<UiAction>) {
        egui::MenuBar::new().ui(ui, |ui| {
            if ui
                .button(if self.show_sidebar { "⏴" } else { "⏵" })
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
                        .button("reccmp-datacmp (WARNING)")
                        .on_hover_text(
                            "WARNING: This tool will take away at your lifespan (and lock up the worker thread) if you don't have large variables properly ignored in your reccmp-project.yml.",
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
        ui.heading("Project");
        ui.horizontal(|ui| {
            ui.label("Status:");
            if self.project.is_some() {
                ui.label("Loaded");
            } else {
                ui.label("Not loaded");
            }
        });
        ui.horizontal(|ui| {
            ui.label("Tool status:");
            if let Some(project) = &self.project {
                match project.tool_state {
                    ToolState::Cancelling => {
                        ui.label("Cancelling...");
                        ui.spinner();
                    }
                    ToolState::Building => {
                        ui.label("Building...");
                        ui.spinner();
                    }
                    ToolState::Disassembling => {
                        ui.label("Disassembling...");
                        ui.spinner();
                    }
                    ToolState::Stackcmp => {
                        ui.label("Running stackcmp...");
                        ui.spinner();
                    }
                    ToolState::Datacmp => {
                        ui.label("Running datacmp...");
                        ui.spinner();
                    }
                    ToolState::Roadmap => {
                        ui.label("Running roadmap...");
                        ui.spinner();
                    }
                    ToolState::Idle => {
                        ui.label("Idle");
                    }
                }
            } else {
                ui.label("Project not loaded");
            }
        });
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

        if let Some(project) = &self.project {
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
            let mut use_roadmap = self.settings.use_roadmap;
            ui.checkbox(
                &mut use_roadmap,
                "Use roadmap to improve disassembly analysis",
            )
            .on_hover_text(
                "Uses data from reccmp-roadmap to replace addresses with symbols in disassembly",
            );
            if use_roadmap != self.settings.use_roadmap {
                actions.push(UiAction::ToggleUseRoadmap);
            }

            ui.add_enabled_ui(
                (self.settings.reccmp.is_some() && !project.target.is_empty())
                    || project.tool_state != ToolState::Idle,
                |ui| {
                    let can_cancel = project.tool_state != ToolState::Idle
                        && project.tool_state != ToolState::Disassembling
                        && project.tool_state != ToolState::Cancelling;

                    if project.tool_state == ToolState::Idle {
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

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.style_mut().override_text_style = Some(TextStyle::Monospace);
                for log in &self.logs {
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
            .resizable(false)
            .open(&mut open)
            .show(ui, |ui| {
                ui.colored_label(egui::Color32::RED, "Error:");
                for error in &self.errors {
                    ui.label(error);
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
        ui.horizontal(|ui| {
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
            UiAction::Cancel => {
                if let Some(cancelled) = self.tool_cancel.take() {
                    cancelled.store(true, Ordering::Relaxed);
                }
                if let Some(project) = &mut self.project {
                    project.tool_state = ToolState::Cancelling;
                }
            }
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
            UiAction::ToggleUseRoadmap => {
                self.settings.use_roadmap = !self.settings.use_roadmap;
                if !self.settings.use_roadmap {
                    self.tx_disasm
                        .send(crate::worker::DisassembleCommand::UpdateRoadmap(None))?;
                    self.refresh_diff_tabs()?;
                } else if let Some(rows) = &self.roadmap_rows {
                    self.tx_disasm
                        .send(crate::worker::DisassembleCommand::UpdateRoadmap(Some(
                            rows.clone(),
                        )))?;
                    self.refresh_diff_tabs()?;
                } else if self.settings.roadmap.is_some() {
                    self.trigger_roadmap(false)?;
                }
            }
            UiAction::ToggleSidebar => {
                self.show_sidebar = !self.show_sidebar;
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
                let str = yaml_serde::to_string(&config)?;
                fs::write(&config_path, str)?;
                self.project = Project::new(config, project_dir).ok();
                self.dock = DockState::new(Vec::new());
                self.roadmap_rows = None;
                self.last_report = None;
                self.tx_disasm
                    .send(crate::worker::DisassembleCommand::UpdateRoadmap(None))?;
                self.settings.recent_projects.retain(|p| p != &config_path);
                self.settings.recent_projects.insert(0, config_path);
            }
            UiAction::Disassemble(data) => {
                self.trigger_disassemble(data, true)?;
            }
            UiAction::Stackcmp { address, func_name } => {
                self.trigger_stackcmp(address, func_name)?;
            }
            UiAction::Datacmp => {
                self.trigger_datacmp()?;
            }
            UiAction::Roadmap => {
                self.trigger_roadmap(true)?;
            }
            UiAction::OpenVtable(data) => {
                self.open_vtable(data)?;
            }
            UiAction::OpenFunctionByName(name) => {
                if let Some(report) = &self.last_report
                    && let Some(data) = report.data.iter().find(|d| d.name == name)
                {
                    self.trigger_disassemble(data.clone(), true)?;
                } else {
                    self.logs.push(format!("function {name} not found"));
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

        let mut show_sidebar = self.show_sidebar;
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
        eframe::set_value(storage, eframe::APP_KEY, &self.settings);
    }
}
