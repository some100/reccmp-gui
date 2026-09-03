use core::time::Duration;
use crossbeam_channel::{Receiver, Sender, unbounded};
use eframe::egui;
use egui_dock::DockState;
use std::{
    collections::{HashMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    thread,
};
use thiserror::Error;

use crate::{
    app::{
        config::{AppSettings, DisassemblySettings, Project, WatchSettings},
        message::Message,
        new_project::NewProjectWindow,
        tab::Tab,
    },
    reccmp::{Address, ReccmpReportData, ReccmpReportJson},
    roadmap::RoadmapRow,
    worker::{
        Command, Worker,
        disassemble::{DisassembleCommand, DisassembleRequest, DisassemblyWorker},
        request::{
            CancellationToken, CompileRequest, DatacmpRequest, ReccmpRequest, RoadmapRequest,
            StackcmpRequest, ToolRequestInfo, WatchRequest,
        },
    },
};

pub mod config;
pub mod message;
mod new_project;
mod tab;
mod ui;
mod widgets;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("failed to read file '{path}': {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to write file '{path}': {source}")]
    WriteFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse YAML '{path}': {source}")]
    YamlRead {
        path: PathBuf,
        #[source]
        source: yaml_serde::Error,
    },

    #[error("failed to write YAML to '{path}': {source}")]
    YamlWrite {
        path: PathBuf,
        #[source]
        source: yaml_serde::Error,
    },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("yaml serde error: {0}")]
    Serde(#[from] yaml_serde::Error),

    #[error("egui dock access error: {0}")]
    EguiDock(#[from] egui_dock::Error),

    #[error("worker task error: {0}")]
    WorkerTaskError(#[from] crate::worker::WorkerTaskError),

    #[error("target {target} not found in yml {path}")]
    TargetMissing { target: String, path: PathBuf },

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

#[derive(Default)]
pub struct ToolCancellationTokens {
    pub compile: Option<CancellationToken>,
    pub reccmp: Option<CancellationToken>,
    pub roadmap: Option<CancellationToken>,
    pub datacmp: Option<CancellationToken>,
    pub stackcmp: HashMap<Address, CancellationToken>,
}

pub struct App {
    project: Option<Project>,

    tx_cmd: Sender<Command>,
    tx_disasm: Sender<DisassembleCommand>,
    rx_msg: Receiver<Message>,

    settings: AppSettings,

    dock: DockState<Tab>,
    new_project_window: Option<NewProjectWindow>,

    logs: Vec<String>,

    errors: VecDeque<String>,
    tool_cancels: ToolCancellationTokens,

    roadmap_rows: Option<Vec<RoadmapRow>>,
    last_report: Option<ReccmpReportJson>,

    generation: u64,

    _worker_thread: thread::JoinHandle<()>,
    _disasm_worker_thread: thread::JoinHandle<()>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());

        let mut settings: AppSettings = cc.storage.map_or_default(|storage| {
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
        let _ = tx_disasm.send(DisassembleCommand::UpdateSettings(settings.disassembly));

        let dock = if settings.restore_on_startup {
            settings.dock_state.take().map_or_else(
                || DockState::new(Vec::new()),
                |dock| dock.map_tabs(Tab::from_kind),
            )
        } else {
            DockState::new(Vec::new())
        };

        let mut app = Self {
            project: None,
            tx_cmd,
            tx_disasm,
            rx_msg,
            settings,
            dock,
            new_project_window: None,
            logs: Vec::new(),
            errors: VecDeque::new(),
            tool_cancels: ToolCancellationTokens::default(),
            roadmap_rows: None,
            last_report: None,
            generation: 0,
            _worker_thread: worker_thread,
            _disasm_worker_thread: disasm_worker_thread,
        };

        if !app.settings.restore_on_startup {
            return app;
        }

        if let Some(project_path) = app.settings.current_project.clone()
            && project_path.exists()
        {
            if let Err(e) = app.open_config_preserving_dock(&project_path) {
                app.errors
                    .push_back(format!("failed to restore project: {e}"));
            } else if let Some(target) = app.settings.current_target.clone() {
                let _ = app.trigger_change_target_preserving_dock(target);
            }
        }

        app
    }

    pub fn cancel_tools(&mut self) {
        if let Some(c) = self.tool_cancels.compile.take() {
            c.cancel();
        }
        if let Some(c) = self.tool_cancels.reccmp.take() {
            c.cancel();
        }
        if let Some(c) = self.tool_cancels.roadmap.take() {
            c.cancel();
        }
        if let Some(c) = self.tool_cancels.datacmp.take() {
            c.cancel();
        }
        for (_, c) in self.tool_cancels.stackcmp.drain() {
            c.cancel();
        }
        if let Some(project) = &mut self.project {
            project.tool_state.start_cancelling();
        }
        self.generation += 1;
    }

    pub fn check_cancellation_timeout(&mut self, ctx: &egui::Context) {
        if let Some(project) = &mut self.project
            && project.tool_state.cancelling
            && let Some(since) = project.tool_state.cancelling_since
        {
            if since.elapsed() >= Duration::from_secs(5) {
                project.tool_state.force_reset();
                self.tool_cancels = ToolCancellationTokens::default();
                self.logs.push("cancel timed out".to_string());
            } else {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
    }

    fn open_config_preserving_dock(&mut self, path: &Path) -> Result<(), AppError> {
        let str = fs::read_to_string(path).map_err(|e| AppError::ReadFile {
            path: path.to_path_buf(),
            source: e,
        })?;
        let yml = yaml_serde::from_str(&str).map_err(|e| AppError::YamlRead {
            path: path.to_path_buf(),
            source: e,
        })?;
        self.project = Some(Project::new(
            yml,
            path.parent().map(Path::to_path_buf).unwrap_or_default(),
        )?);
        self.settings.current_project = Some(path.to_path_buf());
        Ok(())
    }

    fn trigger_change_target_preserving_dock(&mut self, target: String) -> Result<(), AppError> {
        let project = self.project.as_mut().ok_or(AppError::ProjectNotSet)?;
        project.target.clone_from(&target);
        self.settings.current_target = Some(target);
        project.is_watching = false;

        self.tx_disasm.send(DisassembleCommand::InvalidateOrig)?;
        self.tx_disasm.send(DisassembleCommand::InvalidateRecomp)?;

        if !project.target.is_empty() && self.settings.watch.active {
            let request = WatchRequest::from_project(project, self.settings.watch.debounce_ms)?;
            self.tx_cmd.send(Command::Watch(request))?;
            project.is_watching = true;
        }

        if self.settings.reccmp.is_some() {
            self.trigger_build()?;
        }

        Ok(())
    }

    fn update_disasm_settings(&mut self, settings: DisassemblySettings) -> Result<(), AppError> {
        let was_using_roadmap =
            self.settings.disassembly.use_roadmap && self.settings.disassembly.resolve_symbols;
        let now_using_roadmap = settings.use_roadmap && settings.resolve_symbols;
        let turned_on_roadmap = now_using_roadmap && !was_using_roadmap;

        self.settings.disassembly = settings;

        self.tx_disasm.send(DisassembleCommand::UpdateSettings(
            self.settings.disassembly,
        ))?;

        if turned_on_roadmap {
            if let Some(rows) = &self.roadmap_rows {
                self.tx_disasm
                    .send(DisassembleCommand::UpdateRoadmap(Some(rows.clone())))?;
                self.refresh_diff_tabs()?;
            } else if self.settings.roadmap.is_some() {
                self.trigger_roadmap(false)?;
            } else {
                self.refresh_diff_tabs()?;
            }
        } else {
            self.refresh_diff_tabs()?;
        }

        Ok(())
    }

    fn update_watch_settings(&mut self, settings: WatchSettings) -> Result<(), AppError> {
        self.settings.watch = settings;

        if settings.active
            && let Some(project) = &self.project
            && !project.target.is_empty()
        {
            let request = WatchRequest::from_project(project, settings.debounce_ms)?;
            self.tx_cmd.send(Command::Watch(request))?;
        } else {
            self.tx_cmd.send(Command::StopWatch)?;
        }

        Ok(())
    }

    fn open_vtable(&mut self, data: &ReccmpReportData) -> Result<(), AppError> {
        let mut vtable_tab_path = None;
        for (tab_path, tab) in self.dock.iter_all_tabs_mut() {
            if let Tab::Vtable(state) = tab
                && state.get_name() == data.name
            {
                state.update_from_data(data);
                vtable_tab_path = Some(tab_path);
                break;
            }
        }

        if let Some(tab_path) = vtable_tab_path {
            self.dock.set_active_tab(tab_path)?;
        } else {
            self.dock.push_to_focused_leaf(Tab::new_vtable(data));
        }

        Ok(())
    }

    fn trigger_build(&mut self) -> Result<(), AppError> {
        let (build_cwd, build_cmd) = {
            let project = self.project.as_ref().ok_or(AppError::ProjectNotSet)?;
            (
                project.dir.join(&project.config.build.cwd),
                project.config.build.cmd.clone(),
            )
        };

        self.roadmap_rows = None;

        self.cancel_tools();

        let cancel = CancellationToken::new();
        self.tool_cancels.compile = Some(cancel.clone());
        let request = CompileRequest {
            cancel,
            build_cwd,
            build_cmd,
            generation: self.generation,
        };

        self.tx_cmd.send(Command::Compile(request))?;
        if let Some(project) = &mut self.project {
            project.tool_state.compiling = Some(self.generation);
            project.tool_state.cancelling = false;
            project.tool_state.cancelling_since = None;
        }

        Ok(())
    }

    fn trigger_tools(&mut self) {
        if let Err(e) = self.trigger_reccmp() {
            self.errors.push_back(format!("reccmp failed: {e}"));
        }

        let has_roadmap_tab = self
            .dock
            .iter_all_tabs()
            .any(|(_, tab)| matches!(tab, Tab::Roadmap(_)));
        let needs_roadmap = has_roadmap_tab
            || (self.settings.disassembly.use_roadmap && self.settings.disassembly.resolve_symbols);

        if needs_roadmap
            && self.settings.roadmap.is_some()
            && let Err(e) = self.trigger_roadmap(false)
        {
            self.errors.push_back(format!("roadmap fail: {e}"));
        }

        let mut stackcmp_tabs: Vec<_> = self
            .dock
            .iter_all_tabs()
            .filter_map(|(_, tab)| match tab {
                Tab::Stackcmp(state) => {
                    Some((state.report.address, state.report.func_name.clone()))
                }
                _ => None,
            })
            .collect();

        stackcmp_tabs.sort_by_key(|(addr, _)| *addr);
        stackcmp_tabs.dedup_by_key(|(addr, _)| *addr);

        for (address, func_name) in stackcmp_tabs {
            if let Err(e) = self.trigger_stackcmp(address, func_name, false) {
                self.errors
                    .push_back(format!("stackcmp {address} fail: {e}"));
            }
        }
    }

    fn trigger_reccmp(&mut self) -> Result<(), AppError> {
        let reccmp_path = self
            .settings
            .reccmp
            .as_ref()
            .ok_or(AppError::ReccmpNotSet)?;
        let project = self.project.as_ref().ok_or(AppError::ProjectNotSet)?;
        let reccmp_build_yml_path = project.dir.join(&project.config.files.build);
        let build_yml_dir = reccmp_build_yml_path
            .parent()
            .ok_or(AppError::ReccmpBuildYmlNoParent)?;

        let info = ToolRequestInfo::new(
            &mut self.tool_cancels.reccmp,
            reccmp_path.clone(),
            build_yml_dir.to_path_buf(),
            project.target.clone(),
            self.generation,
        );

        let request = ReccmpRequest {
            info,
            no_library: self.settings.no_library,
        };

        self.tx_cmd.send(Command::Reccmp(request))?;
        if let Some(project) = &mut self.project {
            project.tool_state.reccmp = Some(self.generation);
        }

        Ok(())
    }

    fn trigger_change_target(&mut self, target: String) -> Result<(), AppError> {
        self.cancel_tools();

        let project = self.project.as_mut().ok_or(AppError::ProjectNotSet)?;

        project.target.clone_from(&target);
        self.settings.current_target = Some(target);
        project.is_watching = false;

        self.tx_disasm.send(DisassembleCommand::InvalidateOrig)?;
        self.tx_disasm.send(DisassembleCommand::InvalidateRecomp)?;

        self.roadmap_rows = None;
        self.last_report = None;
        self.tx_disasm
            .send(DisassembleCommand::UpdateRoadmap(None))?;

        if !project.target.is_empty() && self.settings.watch.active {
            let request = WatchRequest::from_project(project, self.settings.watch.debounce_ms)?;
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
            &mut self.tool_cancels.datacmp,
            datacmp.clone(),
            reccmp_build_yml_dir.to_path_buf(),
            project.target.clone(),
            self.generation,
        );
        let request = DatacmpRequest { info };

        self.tx_cmd.send(Command::Datacmp(request))?;
        if let Some(project) = &mut self.project {
            project.tool_state.datacmp = Some(self.generation);
        }

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
            &mut self.tool_cancels.roadmap,
            roadmap.clone(),
            reccmp_build_yml_dir.to_path_buf(),
            project.target.clone(),
            self.generation,
        );
        let request = RoadmapRequest { info, focus };

        self.tx_cmd.send(Command::Roadmap(request))?;
        if let Some(project) = &mut self.project {
            project.tool_state.roadmap = Some(self.generation);
        }

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

    fn trigger_stackcmp(
        &mut self,
        address: Address,
        func_name: String,
        focus: bool,
    ) -> Result<(), AppError> {
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

        if let Some(prev) = self.tool_cancels.stackcmp.remove(&address) {
            prev.cancel();
        }

        let cancel = CancellationToken::new();
        self.tool_cancels.stackcmp.insert(address, cancel.clone());

        let info = ToolRequestInfo {
            path: stackcmp_path.clone(),
            cwd: build_yml_dir.to_path_buf(),
            target: project.target.clone(),
            cancel,
            generation: self.generation,
        };
        let request = StackcmpRequest {
            info,
            address,
            func_name,
            focus,
        };

        self.tx_cmd.send(Command::Stackcmp(request))?;
        if let Some(project) = &mut self.project {
            project.tool_state.stackcmp.insert(address, self.generation);
        }

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
        self.cancel_tools();

        let str = fs::read_to_string(path).map_err(|e| AppError::ReadFile {
            path: path.to_path_buf(),
            source: e,
        })?;
        let yml = yaml_serde::from_str(&str).map_err(|e| AppError::YamlRead {
            path: path.to_path_buf(),
            source: e,
        })?;
        self.project = Some(Project::new(
            yml,
            path.parent().map(Path::to_path_buf).unwrap_or_default(),
        )?);
        self.settings.current_project = Some(path.to_path_buf());
        self.settings.current_target = None;
        self.dock = DockState::new(Vec::new());
        self.roadmap_rows = None;
        self.last_report = None;
        self.tx_disasm
            .send(DisassembleCommand::UpdateRoadmap(None))?;

        Ok(())
    }
}
