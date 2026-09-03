use core::sync::atomic::AtomicBool;
use crossbeam_channel::{Receiver, Sender, unbounded};
use eframe::egui;
use egui_dock::DockState;
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
};
use thiserror::Error;

use crate::{
    app::{message::Message, new_project::NewProjectWindow, tab::Tab},
    reccmp::{
        Address, ReccmpBuildTarget, ReccmpBuildYaml, ReccmpProjectYaml, ReccmpReportData,
        ReccmpReportJson, ReccmpUserTarget, ReccmpUserYaml,
    },
    roadmap::RoadmapRow,
    worker::{
        BuildRequest, Command, DatacmpRequest, DisassembleCommand, DisassembleRequest,
        DisassemblyWorker, RoadmapRequest, StackcmpRequest, ToolRequestInfo, WatchRequest, Worker,
    },
};

pub mod message;
mod new_project;
mod tab;
mod ui;
mod widgets;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("yaml serde error: {0}")]
    Serde(#[from] yaml_serde::Error),

    #[error("egui dock access error: {0}")]
    EguiDock(#[from] egui_dock::Error),

    #[error("target not found in build yml: {0}")]
    TargetMissingInBuildYml(String),

    #[error("target not found in user yml: {0}")]
    TargetMissingInUserYml(String),

    #[error("cmd send error: channel closed")]
    CmdSend,

    #[error("disasm send error: channel closed")]
    DisasmSend,

    #[error("project directory not set")]
    ProjectNotSet,

    #[error("no reccmp path set")]
    ReccmpNotSet,

    #[error("no stackcmp path set")]
    StackcmpNotSet,

    #[error("no datacmp path set")]
    DatacmpNotSet,

    #[error("no roadmap path set")]
    RoadmapNotSet,

    #[error("reccmp-build.yml path has no parent")]
    ReccmpBuildYmlNoParent,
}

impl From<crossbeam_channel::SendError<Command>> for AppError {
    fn from(_: crossbeam_channel::SendError<Command>) -> Self {
        AppError::CmdSend
    }
}

impl From<crossbeam_channel::SendError<DisassembleCommand>> for AppError {
    fn from(_: crossbeam_channel::SendError<DisassembleCommand>) -> Self {
        AppError::DisasmSend
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolState {
    Idle,
    Cancelling,
    Building,
    Disassembling,
    Stackcmp,
    Datacmp,
    Roadmap,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ConfigFiles {
    pub build: PathBuf,
    pub project: PathBuf,
    pub user: PathBuf,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct BuildConfig {
    pub cwd: PathBuf,
    pub cmd: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AppConfig {
    pub files: ConfigFiles,
    pub build: BuildConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AppSettings {
    recent_projects: Vec<PathBuf>,
    reccmp: Option<PathBuf>,
    stackcmp: Option<PathBuf>,
    datacmp: Option<PathBuf>,
    roadmap: Option<PathBuf>,
    no_library: bool,
    use_roadmap: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            recent_projects: Vec::new(),
            reccmp: which::which("reccmp-reccmp").ok(),
            stackcmp: which::which("reccmp-stackcmp").ok(),
            datacmp: which::which("reccmp-datacmp").ok(),
            roadmap: which::which("reccmp-roadmap").ok(),
            no_library: false,
            use_roadmap: true,
        }
    }
}

#[derive(Clone, Debug)]
struct Project {
    config: AppConfig,
    dir: PathBuf,
    build_yml: ReccmpBuildYaml,
    project_yml: ReccmpProjectYaml,
    user_yml: ReccmpUserYaml,
    tool_state: ToolState,
    target: String,
    is_watching: bool,
}

impl Project {
    fn new(config: AppConfig, dir: PathBuf) -> Result<Self, AppError> {
        let build_yml_path = dir.join(&config.files.build);
        let build_yml_str = fs::read_to_string(build_yml_path)?;
        let build_yml = yaml_serde::from_str(&build_yml_str)?;

        let project_yml_path = dir.join(&config.files.project);
        let project_yml_str = fs::read_to_string(project_yml_path)?;
        let project_yml: ReccmpProjectYaml = yaml_serde::from_str(&project_yml_str)?;

        let user_yml_path = dir.join(&config.files.user);
        let user_yml_str = fs::read_to_string(user_yml_path)?;
        let user_yml = yaml_serde::from_str(&user_yml_str)?;

        Ok(Self {
            config,
            dir,
            build_yml,
            project_yml,
            user_yml,
            tool_state: ToolState::Idle,
            target: String::new(),
            is_watching: false,
        })
    }

    fn get_user_target(&self, target: &str) -> Result<&ReccmpUserTarget, AppError> {
        let target = self
            .user_yml
            .targets
            .get(target)
            .ok_or_else(|| AppError::TargetMissingInUserYml(target.to_owned()))?;
        Ok(target)
    }

    fn get_build_target(&self, target: &str) -> Result<&ReccmpBuildTarget, AppError> {
        let target = self
            .build_yml
            .targets
            .get(target)
            .ok_or_else(|| AppError::TargetMissingInBuildYml(target.to_owned()))?;
        Ok(target)
    }
}

pub struct App {
    project: Option<Project>,

    tx_cmd: Sender<Command>,
    tx_disasm: Sender<DisassembleCommand>,
    rx_msg: Receiver<Message>,

    show_sidebar: bool,

    settings: AppSettings,

    dock: DockState<Tab>,
    new_project_window: Option<NewProjectWindow>,

    logs: Vec<String>,

    errors: VecDeque<String>,
    tool_cancel: Option<Arc<AtomicBool>>,

    roadmap_rows: Option<Vec<RoadmapRow>>,
    last_report: Option<ReccmpReportJson>,

    _worker_thread: thread::JoinHandle<()>,
    _disasm_worker_thread: thread::JoinHandle<()>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());

        let settings = cc.storage.map_or_default(|storage| {
            eframe::get_value(storage, eframe::APP_KEY).unwrap_or_default()
        });

        let (tx_cmd, rx_cmd) = unbounded();
        let (tx_disasm, rx_disasm) = unbounded();
        let (tx_msg, rx_msg) = unbounded();

        let mut worker = Worker::new(
            rx_cmd,
            tx_msg.clone(),
            tx_disasm.clone(),
            cc.egui_ctx.clone(),
        );
        let worker_thread = thread::spawn(move || {
            worker.run();
        });

        let mut worker = DisassemblyWorker::new(rx_disasm, tx_msg, cc.egui_ctx.clone());
        let disasm_worker_thread = thread::spawn(move || {
            worker.run();
        });

        Self {
            project: None,
            tx_cmd,
            tx_disasm,
            rx_msg,
            show_sidebar: true,
            settings,
            dock: DockState::new(Vec::new()),
            new_project_window: None,
            logs: Vec::new(),
            errors: VecDeque::new(),
            tool_cancel: None,
            roadmap_rows: None,
            last_report: None,
            _worker_thread: worker_thread,
            _disasm_worker_thread: disasm_worker_thread,
        }
    }

    fn open_vtable(&mut self, data: ReccmpReportData) -> Result<(), AppError> {
        let mut vtable_tab_path = None;
        for (tab_path, tab) in self.dock.iter_all_tabs_mut() {
            if let Tab::Vtable(state) = tab
                && state.get_name() == data.name
            {
                state.update_from_data(&data);
                vtable_tab_path = Some(tab_path);
                break;
            }
        }

        if let Some(tab_path) = vtable_tab_path {
            self.dock.set_active_tab(tab_path)?;
        } else {
            self.dock.push_to_focused_leaf(Tab::new_vtable(&data));
        }

        Ok(())
    }

    fn trigger_build(&mut self) -> Result<(), AppError> {
        let Some(reccmp_path) = self.settings.reccmp.as_ref() else {
            return Err(AppError::ReccmpNotSet);
        };

        let project = self.project.as_ref().ok_or(AppError::ProjectNotSet)?;

        let reccmp_build_yml_path = project.dir.join(&project.config.files.build);
        let Some(build_yml_dir) = reccmp_build_yml_path.parent() else {
            return Err(AppError::ReccmpBuildYmlNoParent);
        };

        let info = ToolRequestInfo::new(
            &mut self.tool_cancel,
            reccmp_path.clone(),
            build_yml_dir.to_path_buf(),
            project.target.clone(),
        );
        let request = BuildRequest {
            info,
            build_cwd: project.dir.join(&project.config.build.cwd),
            build_cmd: project.config.build.cmd.clone(),
            no_library: self.settings.no_library,
        };

        self.tx_cmd.send(Command::Build(request))?;

        Ok(())
    }

    fn trigger_change_target(&mut self, target: String) -> Result<(), AppError> {
        let project = self.project.as_mut().ok_or(AppError::ProjectNotSet)?;
        project.target = target;
        project.is_watching = false;

        self.roadmap_rows = None;
        self.last_report = None;
        self.tx_disasm
            .send(DisassembleCommand::UpdateRoadmap(None))?;

        if !project.target.is_empty() {
            let request = WatchRequest {
                reccmp_project_yml: project.project_yml.clone(),
                project_dir: project.dir.clone(),
                target: project.target.clone(),
            };
            self.tx_cmd.send(Command::Watch(request))?;
            project.is_watching = true;
        }

        self.dock = DockState::new(Vec::new());

        if self.settings.reccmp.is_some() {
            self.trigger_build()?;
        }

        Ok(())
    }

    fn trigger_datacmp(&mut self) -> Result<(), AppError> {
        let datacmp = self
            .settings
            .datacmp
            .as_ref()
            .ok_or(AppError::DatacmpNotSet)?;
        let project = self.project.as_ref().ok_or(AppError::ProjectNotSet)?;
        let reccmp_build_yml_path = project.dir.join(&project.config.files.build);
        let reccmp_build_yml_dir = reccmp_build_yml_path
            .parent()
            .ok_or(AppError::ReccmpBuildYmlNoParent)?;

        let info = ToolRequestInfo::new(
            &mut self.tool_cancel,
            datacmp.clone(),
            reccmp_build_yml_dir.to_path_buf(),
            project.target.clone(),
        );
        let request = DatacmpRequest { info };

        self.tx_cmd.send(Command::Datacmp(request))?;

        Ok(())
    }

    fn trigger_roadmap(&mut self, focus: bool) -> Result<(), AppError> {
        if let Some(roadmap_rows) = self.roadmap_rows.take() {
            return self.handle_roadmap_finished(roadmap_rows, focus);
        }

        let roadmap = self
            .settings
            .roadmap
            .as_ref()
            .ok_or(AppError::RoadmapNotSet)?;
        let project = self.project.as_ref().ok_or(AppError::ProjectNotSet)?;

        let reccmp_build_yml_path = project.dir.join(&project.config.files.build);
        let reccmp_build_yml_dir = reccmp_build_yml_path
            .parent()
            .ok_or(AppError::ReccmpBuildYmlNoParent)?;

        let info = ToolRequestInfo::new(
            &mut self.tool_cancel,
            roadmap.clone(),
            reccmp_build_yml_dir.to_path_buf(),
            project.target.clone(),
        );
        let request = RoadmapRequest { info, focus };

        self.tx_cmd.send(Command::Roadmap(request))?;

        Ok(())
    }

    fn trigger_disassemble(&self, data: ReccmpReportData, focus: bool) -> Result<(), AppError> {
        let project = self.project.as_ref().ok_or(AppError::ProjectNotSet)?;

        let orig_exe_path = project
            .dir
            .join(&project.get_user_target(&project.target)?.path);
        let recomp_exe_path = project
            .dir
            .join(&project.get_build_target(&project.target)?.path);
        let request = DisassembleRequest {
            data,
            orig_exe_path,
            recomp_exe_path,
            focus,
        };
        self.tx_disasm
            .send(DisassembleCommand::Disassemble(request))?;

        Ok(())
    }

    fn trigger_stackcmp(&mut self, address: Address, func_name: String) -> Result<(), AppError> {
        let stackcmp_path = self
            .settings
            .stackcmp
            .as_ref()
            .ok_or(AppError::StackcmpNotSet)?;

        let project = self.project.as_ref().ok_or(AppError::ProjectNotSet)?;

        let reccmp_build_yml_path = project.dir.join(&project.config.files.build);
        let build_yml_dir = reccmp_build_yml_path
            .parent()
            .ok_or(AppError::ReccmpBuildYmlNoParent)?;

        let info = ToolRequestInfo::new(
            &mut self.tool_cancel,
            stackcmp_path.clone(),
            build_yml_dir.to_path_buf(),
            project.target.clone(),
        );
        let request = StackcmpRequest {
            info,
            address,
            func_name,
        };

        self.tx_cmd.send(Command::Stackcmp(request))?;

        Ok(())
    }

    fn refresh_diff_tabs(&mut self) -> Result<(), AppError> {
        let Some(project) = &self.project else {
            return Ok(());
        };
        let Some(report) = &self.last_report else {
            return Ok(());
        };

        let build_target = project.get_build_target(&project.target)?;
        let user_target = project.get_user_target(&project.target)?;

        for (_, tab) in self.dock.iter_all_tabs_mut() {
            if let Tab::Diff(state) = tab
                && let Some(data) = report.data.iter().find(|x| x.name == state.get_name())
            {
                let request = DisassembleRequest {
                    data: data.clone(),
                    orig_exe_path: project.dir.join(&user_target.path),
                    recomp_exe_path: project.dir.join(&build_target.path),
                    focus: false,
                };
                self.tx_disasm
                    .send(DisassembleCommand::Disassemble(request))?;
            }
        }

        Ok(())
    }

    fn open_config(&mut self, path: &Path) -> Result<(), AppError> {
        let str = fs::read_to_string(path)?;
        self.project = Some(Project::new(
            yaml_serde::from_str(&str)?,
            path.parent().map(Path::to_path_buf).unwrap_or_default(),
        )?);
        self.dock = DockState::new(Vec::new());
        self.roadmap_rows = None;
        self.last_report = None;
        self.tx_disasm
            .send(DisassembleCommand::UpdateRoadmap(None))?;

        Ok(())
    }
}
