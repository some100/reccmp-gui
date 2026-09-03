use std::path::PathBuf;

use eframe::egui::{self, Button};
use walkdir::WalkDir;

use crate::app::{AppConfig, widgets::PathPicker};

pub enum NewProjectAction {
    SaveAndOpen {
        config: AppConfig,
        project_dir: PathBuf,
    },
    Close,
}

#[derive(Clone, Debug, Default)]
pub struct NewProjectWindow {
    project_dir: PathBuf,
    config: AppConfig,
}

impl NewProjectWindow {
    pub fn render(&mut self, ui: &mut egui::Ui) -> Option<NewProjectAction> {
        let mut action = None;
        let mut is_open = true;

        egui::Window::new("New").open(&mut is_open).show(ui, |ui| {
            egui::Grid::new("new_grid").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Project directory:");
                    ui.label(self.project_dir.to_string_lossy());
                    if ui.button("Browse...").clicked()
                        && let Some(path) = rfd::FileDialog::new().pick_folder()
                    {
                        self.project_dir = path;
                        // try auto detecting reccmp config files if we have a project
                        for entry in WalkDir::new(&self.project_dir)
                            .into_iter()
                            .filter_map(Result::ok)
                        {
                            if entry.file_name().eq_ignore_ascii_case("reccmp-build.yml") {
                                self.config.files.build =
                                    PathPicker::as_relative(&self.project_dir, entry.path());
                            }
                            if entry.file_name().eq_ignore_ascii_case("reccmp-project.yml") {
                                self.config.files.project =
                                    PathPicker::as_relative(&self.project_dir, entry.path());
                            }
                            if entry.file_name().eq_ignore_ascii_case("reccmp-user.yml") {
                                self.config.files.user =
                                    PathPicker::as_relative(&self.project_dir, entry.path());
                            }
                        }
                        self.config.build.cwd = PathBuf::from(".");
                    }
                });
                ui.end_row();

                let project_dir_exists = !self.project_dir.is_empty();
                ui.add_enabled(
                    project_dir_exists,
                    PathPicker::file("reccmp-build.yml", &mut self.config.files.build)
                        .relative_to(&self.project_dir),
                );
                ui.end_row();
                ui.add_enabled(
                    project_dir_exists,
                    PathPicker::file("reccmp-project.yml", &mut self.config.files.project)
                        .relative_to(&self.project_dir),
                );
                ui.end_row();
                ui.add_enabled(
                    project_dir_exists,
                    PathPicker::file("reccmp-user.yml", &mut self.config.files.user)
                        .relative_to(&self.project_dir),
                );
                ui.end_row();
                ui.add_enabled_ui(!self.project_dir.is_empty(), |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Build command:");
                        ui.text_edit_singleline(&mut self.config.build.cmd);
                    });
                });
                ui.end_row();
                ui.add_enabled(
                    project_dir_exists,
                    PathPicker::folder("Build working directory", &mut self.config.build.cwd)
                        .relative_to(&self.project_dir),
                );
                ui.end_row();
                if ui
                    .add_enabled(
                        !self.project_dir.is_empty() && !self.config.build.cmd.is_empty(),
                        Button::new("Finish"),
                    )
                    .clicked()
                {
                    action = Some(NewProjectAction::SaveAndOpen {
                        config: self.config.clone(),
                        project_dir: self.project_dir.clone(),
                    });
                }
            });
        });

        if !is_open {
            action = Some(NewProjectAction::Close);
        }

        action
    }
}
