use core::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use std::{
    fs,
    io::{BufRead, BufReader, Read},
    path::PathBuf,
    process::{ExitStatus, Stdio},
    sync::Arc,
};

use command_group::CommandGroup;
use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use notify_debouncer_full::{
    DebounceEventResult, Debouncer, NoCache, new_debouncer,
    notify::{self, EventKind, RecommendedWatcher, RecursiveMode, event::ModifyKind},
};
use tempfile::tempdir;
use thiserror::Error;

use crate::{
    app::{ToolState, message::Message},
    disassemble::{BinaryType, DisassembleError, Disassembler, Disassembly},
    reccmp::{Address, ReccmpProjectYaml, ReccmpReportData},
    roadmap::RoadmapRow,
    stackcmp::StackcmpReport,
};

#[derive(Error, Debug)]
pub enum WorkerTaskError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("msg send error: {0}")]
    MsgSend(#[from] crossbeam_channel::SendError<Message>),

    #[error("disassemble error: {0}")]
    Disassemble(#[from] DisassembleError),

    #[error("notify error: {0}")]
    Notify(#[from] notify::Error),

    #[error("csv error: {0}")]
    Csv(#[from] csv::Error),

    #[error("target {0} not found")]
    TargetNotFound(String),

    #[error("{0} failed with status {1}")]
    CommandFailed(&'static str, ExitStatus),

    #[error("tool was cancelled")]
    ToolCancelled,
}

pub enum Command {
    Build(BuildRequest),
    Stackcmp(StackcmpRequest),
    Datacmp(DatacmpRequest),
    Roadmap(RoadmapRequest),
    Watch(WatchRequest),
}

pub struct Worker {
    rx: Receiver<Command>,
    tx: Sender<Message>,
    tx_disasm: Sender<DisassembleCommand>,
    ctx: egui::Context,
    debouncer: Option<Debouncer<RecommendedWatcher, NoCache>>,
    target: String,
}

impl Worker {
    pub fn new(
        rx: Receiver<Command>,
        tx: Sender<Message>,
        tx_disasm: Sender<DisassembleCommand>,
        ctx: egui::Context,
    ) -> Self {
        Self {
            rx,
            tx,
            tx_disasm,
            ctx,
            debouncer: None,
            target: String::new(),
        }
    }

    pub fn run(&mut self) {
        while let Ok(cmd) = self.rx.recv() {
            match cmd {
                Command::Build(request) => {
                    let result = self.build(request);
                    self.handle_tool_result(result);
                }
                Command::Stackcmp(request) => {
                    let result = self.stackcmp(request);
                    self.handle_tool_result(result);
                }
                Command::Datacmp(request) => {
                    let result = self.datacmp(request);
                    self.handle_tool_result(result);
                }
                Command::Roadmap(request) => {
                    let result = self.roadmap(request);
                    self.handle_tool_result(result);
                }
                Command::Watch(request) => {
                    if let Err(e) = self.watch(request) {
                        let _ = self.tx.send(Message::Error(e.to_string()));
                    }
                }
            }
            self.ctx.request_repaint();
        }
    }

    fn handle_tool_result(&self, res: Result<(), WorkerTaskError>) {
        match res {
            Ok(()) => {}
            Err(WorkerTaskError::ToolCancelled) => {
                let _ = self.tx.send(Message::SetToolState(ToolState::Idle));
            }
            Err(e) => {
                let _ = self.tx.send(Message::Error(e.to_string()));
                let _ = self.tx.send(Message::SetToolState(ToolState::Idle));
            }
        }
    }

    fn build(&mut self, request: BuildRequest) -> Result<(), WorkerTaskError> {
        self.tx.send(Message::SetToolState(ToolState::Building))?;

        Self::check_tool_cancelled(&request.info.cancelled)?;

        let mut build_proc: std::process::Command;
        #[cfg(target_os = "windows")]
        {
            build_proc = std::process::Command::new("cmd");
            build_proc.args(["/C", &request.build_cmd]);
        }

        #[cfg(not(target_os = "windows"))]
        {
            build_proc = std::process::Command::new("sh");
            build_proc.args(["-c", &request.build_cmd]);
        }

        build_proc.current_dir(&request.build_cwd);

        let status = self.run_command(&mut build_proc, &self.tx, Some(&request.info.cancelled))?;
        if !status.success() {
            return Err(WorkerTaskError::CommandFailed("build", status));
        }

        Self::check_tool_cancelled(&request.info.cancelled)?;

        let tmpdir = tempdir()?;

        let report_path = tmpdir.path().join("report.json");

        let mut build_proc = std::process::Command::new(&request.info.path);
        build_proc
            .current_dir(&request.info.cwd)
            .arg("--target")
            .arg(&request.info.target)
            .arg("--json")
            .arg(&report_path)
            .arg("-n");

        if request.no_library {
            build_proc.arg("--nolib");
        }

        let status = self.run_command(&mut build_proc, &self.tx, Some(&request.info.cancelled))?;
        if !status.success() {
            return Err(WorkerTaskError::CommandFailed("reccmp", status));
        }

        Self::check_tool_cancelled(&request.info.cancelled)?;

        let report = fs::read_to_string(&report_path)?;
        self.tx.send(Message::BuildFinished(
            serde_json::from_str(&report).map_err(|e| e.to_string()),
        ))?;

        if self.target != request.info.target {
            self.tx_disasm.send(DisassembleCommand::InvalidateOrig).ok();
            self.target = request.info.target;
        }

        self.tx_disasm
            .send(DisassembleCommand::InvalidateRecomp)
            .ok();

        Ok(())
    }

    fn stackcmp(&mut self, request: StackcmpRequest) -> Result<(), WorkerTaskError> {
        self.tx.send(Message::SetToolState(ToolState::Stackcmp))?;

        let mut cmd = std::process::Command::new(&request.info.path);
        cmd.current_dir(&request.info.cwd)
            .arg("--target")
            .arg(&request.info.target)
            .arg(format!("{}", request.address));
        let (output, status) =
            self.run_command_get_stdout(&mut cmd, &self.tx, Some(&request.info.cancelled))?;
        if !status.success() {
            return Err(WorkerTaskError::CommandFailed("stackcmp", status));
        }
        self.tx.send(Message::StackcmpFinished(StackcmpReport::new(
            &output,
            request.address,
            request.func_name,
        )))?;
        Ok(())
    }

    fn datacmp(&mut self, request: DatacmpRequest) -> Result<(), WorkerTaskError> {
        self.tx.send(Message::SetToolState(ToolState::Datacmp))?;

        let mut cmd = std::process::Command::new(&request.info.path);
        cmd.current_dir(&request.info.cwd)
            .arg("--target")
            .arg(request.info.target)
            .arg("-n")
            .arg("-a");

        let status = self.run_command(&mut cmd, &self.tx, Some(&request.info.cancelled))?;
        if !status.success() {
            self.tx.send(Message::Log(
                "datacmp found at least one problem".to_owned(),
            ))?;
        }

        self.tx.send(Message::DatacmpFinished)?;
        Ok(())
    }

    fn roadmap(&mut self, request: RoadmapRequest) -> Result<(), WorkerTaskError> {
        self.tx.send(Message::SetToolState(ToolState::Roadmap))?;

        let tmpdir = tempdir()?;
        let csv_path = tmpdir.path().join("roadmap.csv");
        let mut cmd = std::process::Command::new(request.info.path);
        cmd.current_dir(request.info.cwd)
            .arg("--target")
            .arg(request.info.target)
            .arg("--csv")
            .arg(&csv_path);

        let status = self.run_command(&mut cmd, &self.tx, Some(&request.info.cancelled))?;
        if !status.success() {
            return Err(WorkerTaskError::CommandFailed("roadmap", status));
        }

        self.tx.send(Message::RoadmapFinished {
            rows: RoadmapRow::from_path(&csv_path)?,
            focus: request.focus,
        })?;
        Ok(())
    }

    fn watch(&mut self, request: WatchRequest) -> Result<(), WorkerTaskError> {
        let tx = self.tx.clone();
        let mut debouncer = new_debouncer(
            Duration::from_millis(150),
            None,
            move |res: DebounceEventResult| {
                if let Ok(events) = res {
                    for event in events {
                        if matches!(
                            event.kind,
                            EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Name(_))
                                | EventKind::Create(_)
                                | EventKind::Remove(_)
                        ) {
                            _ = tx.send(Message::SourceFileChanged);
                        }
                    }
                }
            },
        )?;

        let reccmp_project_target = request
            .reccmp_project_yml
            .targets
            .get(&request.target)
            .ok_or(WorkerTaskError::TargetNotFound(request.target))?;
        let source_root = request.project_dir.join(&reccmp_project_target.source_root);
        debouncer.watch(&source_root, RecursiveMode::Recursive)?;

        self.debouncer = Some(debouncer);

        Ok(())
    }

    fn run_command(
        &self,
        command: &mut std::process::Command,
        tx: &Sender<Message>,
        cancel: Option<&AtomicBool>,
    ) -> Result<std::process::ExitStatus, WorkerTaskError> {
        if let Some(cancel) = cancel {
            Self::check_tool_cancelled(cancel)?;
        }

        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .group_spawn()?;

        let stdout = child.inner().stdout.take().expect("stdout present");
        let stderr = child.inner().stderr.take().expect("stderr present");

        let tx_out = tx.clone();
        let tx_err = tx.clone();
        let ctx = self.ctx.clone();

        let out_thread = std::thread::spawn(move || {
            let reader = BufReader::new(stdout);

            for line in reader.lines().map_while(Result::ok) {
                let _ = tx_out.send(Message::Log(line));
                ctx.request_repaint();
            }
        });

        let ctx = self.ctx.clone();
        let err_thread = std::thread::spawn(move || {
            let reader = BufReader::new(stderr);

            for line in reader.lines().map_while(Result::ok) {
                let _ = tx_err.send(Message::Log(format!("[stderr] {line}")));
                ctx.request_repaint();
            }
        });

        let status = match cancel {
            Some(cancel) => loop {
                if cancel.load(Ordering::Relaxed) {
                    let _ = tx.send(Message::Log("current tool cancelled".into()));

                    let _ = child.kill();
                    let _ = child.wait();

                    let _ = out_thread.join();
                    let _ = err_thread.join();
                    return Err(WorkerTaskError::ToolCancelled);
                }

                if let Some(exit_status) = child.try_wait()? {
                    break exit_status;
                }

                std::thread::sleep(Duration::from_millis(100));
            },
            None => child.wait()?,
        };

        let _ = out_thread.join();
        let _ = err_thread.join();

        Ok(status)
    }

    fn run_command_get_stdout(
        &self,
        command: &mut std::process::Command,
        tx: &Sender<Message>,
        cancel: Option<&AtomicBool>,
    ) -> Result<(String, ExitStatus), WorkerTaskError> {
        if let Some(cancel) = cancel {
            Self::check_tool_cancelled(cancel)?;
        }

        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .group_spawn()?;

        let stdout = child.inner().stdout.take().expect("stdout present");
        let stderr = child.inner().stderr.take().expect("stderr present");

        let out_thread = std::thread::spawn(move || {
            let mut stdout_str = String::new();
            let _ = BufReader::new(stdout).read_to_string(&mut stdout_str);
            stdout_str
        });

        let tx_err = tx.clone();
        let ctx = self.ctx.clone();
        let err_thread = std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                let _ = tx_err.send(Message::Log(format!("[stderr] {line}")));
                ctx.request_repaint();
            }
        });

        let status = match cancel {
            Some(cancel) => loop {
                if cancel.load(Ordering::Relaxed) {
                    let _ = tx.send(Message::Log("current tool cancelled".into()));

                    let _ = child.kill();
                    let _ = child.wait();

                    let _ = out_thread.join();
                    let _ = err_thread.join();
                    return Err(WorkerTaskError::ToolCancelled);
                }
                if let Some(exit_status) = child.try_wait()? {
                    break exit_status;
                }

                std::thread::sleep(Duration::from_millis(100));
            },
            None => child.wait()?,
        };
        let _ = err_thread.join();
        let stdout_str = out_thread.join().unwrap_or_default();

        Ok((stdout_str, status))
    }

    fn check_tool_cancelled(cancelled: &AtomicBool) -> Result<(), WorkerTaskError> {
        if cancelled.load(Ordering::Relaxed) {
            return Err(WorkerTaskError::ToolCancelled);
        }
        Ok(())
    }
}

pub struct ToolRequestInfo {
    pub path: PathBuf,
    pub cwd: PathBuf,
    pub target: String,
    pub cancelled: Arc<AtomicBool>,
}

impl ToolRequestInfo {
    pub fn new(
        tool_cancel: &mut Option<Arc<AtomicBool>>,
        path: PathBuf,
        cwd: PathBuf,
        target: String,
    ) -> Self {
        if let Some(cancelled) = tool_cancel.take() {
            cancelled.store(true, Ordering::Relaxed);
        }

        let cancelled = Arc::new(AtomicBool::new(false));
        *tool_cancel = Some(cancelled.clone());

        Self {
            path,
            cwd,
            target,
            cancelled,
        }
    }
}

pub struct BuildRequest {
    pub info: ToolRequestInfo,
    pub build_cwd: PathBuf,
    pub build_cmd: String,
    pub no_library: bool,
}

pub struct StackcmpRequest {
    pub info: ToolRequestInfo,
    pub address: Address,
    pub func_name: String,
}

pub struct DatacmpRequest {
    pub info: ToolRequestInfo,
}

pub struct RoadmapRequest {
    pub info: ToolRequestInfo,
    pub focus: bool,
}

pub struct WatchRequest {
    pub reccmp_project_yml: ReccmpProjectYaml,
    pub project_dir: PathBuf,
    pub target: String,
}

pub enum DisassembleCommand {
    Disassemble(DisassembleRequest),
    UpdateResolvers(Vec<ReccmpReportData>),
    UpdateRoadmap(Option<Vec<RoadmapRow>>),
    InvalidateOrig,
    InvalidateRecomp,
}

pub struct DisassemblyWorker {
    rx: Receiver<DisassembleCommand>,
    tx: Sender<Message>,
    ctx: egui::Context,
    orig_exe: Option<Vec<u8>>,
    recomp_exe: Option<Vec<u8>>,
}

impl DisassemblyWorker {
    pub fn new(rx: Receiver<DisassembleCommand>, tx: Sender<Message>, ctx: egui::Context) -> Self {
        Self {
            rx,
            tx,
            ctx,
            orig_exe: None,
            recomp_exe: None,
        }
    }

    pub fn run(&mut self) {
        let mut disassembler = Disassembler::new();
        while let Ok(cmd) = self.rx.recv() {
            match cmd {
                DisassembleCommand::Disassemble(request) => {
                    if let Err(e) = self.disassemble(request, &mut disassembler) {
                        let _ = self.tx.send(Message::Error(e.to_string()));
                    }
                }
                DisassembleCommand::UpdateResolvers(data) => {
                    disassembler.update_resolvers(data);
                }
                DisassembleCommand::UpdateRoadmap(rows) => {
                    disassembler.update_roadmap(rows);
                }
                DisassembleCommand::InvalidateOrig => self.orig_exe = None,
                DisassembleCommand::InvalidateRecomp => self.recomp_exe = None,
            }
            self.ctx.request_repaint();
        }
    }

    fn disassemble(
        &mut self,
        request: DisassembleRequest,
        disassembler: &mut Disassembler,
    ) -> Result<(), WorkerTaskError> {
        self.tx
            .send(Message::SetToolState(ToolState::Disassembling))?;

        let hunks = if let Some(diff) = &request.data.diff {
            diff.iter().flat_map(|(_, h)| h.clone()).collect()
        } else {
            Vec::new()
        };

        let orig_exe = match &self.orig_exe {
            Some(bytes) => bytes,
            None => self.orig_exe.insert(fs::read(&request.orig_exe_path)?),
        };

        let mut max_known_address = None;
        for hunk in &hunks {
            if let Some(addr) = hunk.last_orig_code_address() {
                max_known_address = Some(addr);
            }
            if hunk.is_table() {
                break;
            }
        }

        let orig_disasm = disassembler.disasm(
            orig_exe,
            request.data.address,
            max_known_address,
            BinaryType::Orig,
        )?;

        let recomp_exe = match &self.recomp_exe {
            Some(bytes) => bytes,
            None => self.recomp_exe.insert(fs::read(&request.recomp_exe_path)?),
        };

        let mut max_known_address = None;
        for hunk in &hunks {
            if let Some(addr) = hunk.last_recomp_code_address() {
                max_known_address = Some(addr);
            }
            if hunk.is_table() {
                break;
            }
        }
        let recomp_disasm = disassembler.disasm(
            recomp_exe,
            request.data.recomp,
            max_known_address,
            BinaryType::Recomp,
        )?;
        let rows = disassembler.diff(&orig_disasm, &recomp_disasm, &hunks);

        let disasm = Disassembly {
            func_name: request.data.name,
            rows,
            focus: request.focus,
        };
        self.tx.send(Message::DisassembleFinished(disasm))?;
        Ok(())
    }
}

pub struct DisassembleRequest {
    pub data: ReccmpReportData,
    pub orig_exe_path: PathBuf,
    pub recomp_exe_path: PathBuf,
    pub focus: bool,
}
