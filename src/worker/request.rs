use core::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use std::{
    fs,
    io::{BufRead, BufReader, Read},
    path::PathBuf,
    process::ExitStatus,
    sync::Arc,
};

use command_group::CommandGroup;
use crossbeam_channel::Sender;
use eframe::egui;
use notify_debouncer_full::{
    DebounceEventResult, Debouncer, NoCache, new_debouncer,
    notify::{EventKind, RecommendedWatcher, RecursiveMode, event::ModifyKind},
};
use tempfile::tempdir;

use crate::{
    app::{config::Project, message::Message},
    reccmp::Address,
    roadmap::RoadmapRow,
    stackcmp::StackcmpReport,
    worker::{WorkerTaskError, disassemble::DisassembleCommand},
};

pub type WatchDebouncer = Debouncer<RecommendedWatcher, NoCache>;

#[derive(Clone)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
    sender: crossbeam_channel::Sender<()>,
    receiver: crossbeam_channel::Receiver<()>,
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl CancellationToken {
    pub fn new() -> Self {
        let (sender, receiver) = crossbeam_channel::bounded(1);
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            sender,
            receiver,
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        let _ = self.sender.try_send(());
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    pub fn check_cancelled(&self) -> Result<(), WorkerTaskError> {
        if self.is_cancelled() {
            return Err(WorkerTaskError::ToolCancelled);
        }

        Ok(())
    }

    pub fn receiver(&self) -> &crossbeam_channel::Receiver<()> {
        &self.receiver
    }
}

pub struct ToolRequestInfo {
    pub path: PathBuf,
    pub cwd: PathBuf,
    pub target: String,
    pub cancel: CancellationToken,
    pub generation: u64,
}

impl ToolRequestInfo {
    pub fn new(
        tool_cancel: &mut Option<CancellationToken>,
        path: PathBuf,
        cwd: PathBuf,
        target: String,
        generation: u64,
    ) -> Self {
        if let Some(cancel) = tool_cancel.take() {
            cancel.cancel();
        }

        let cancel = CancellationToken::new();
        *tool_cancel = Some(cancel.clone());

        Self {
            path,
            cwd,
            target,
            cancel,
            generation,
        }
    }
}

pub struct CompileRequest {
    pub cancel: CancellationToken,
    pub build_cwd: PathBuf,
    pub build_cmd: String,
    pub generation: u64,
}

impl CompileRequest {
    pub fn run(
        self,
        tx: &Sender<Message>,
        tx_disasm: &Sender<DisassembleCommand>,
        ctx: &egui::Context,
    ) -> Result<(), WorkerTaskError> {
        let mut cmd: std::process::Command;
        #[cfg(target_os = "windows")]
        {
            cmd = std::process::Command::new("cmd");
            cmd.args(["/C", &self.build_cmd]);
        }

        #[cfg(not(target_os = "windows"))]
        {
            cmd = std::process::Command::new("sh");
            cmd.args(["-c", &self.build_cmd]);
        }

        cmd.current_dir(self.build_cwd);

        let (_, status) = run_command(
            ctx,
            cmd,
            tx,
            Some(&self.cancel),
            "compile",
            CommandOutput::Log,
        )?;
        if !status.success() {
            return Err(WorkerTaskError::CommandFailed("compile", status));
        }

        tx_disasm.send(DisassembleCommand::InvalidateRecomp)?;

        tx.send(Message::CompileFinished(self.generation))?;

        Ok(())
    }
}

pub struct ReccmpRequest {
    pub info: ToolRequestInfo,
    pub no_library: bool,
}

impl ReccmpRequest {
    pub fn run(self, tx: &Sender<Message>, ctx: &egui::Context) -> Result<(), WorkerTaskError> {
        let tmpdir = tempdir()?;

        let report_path = tmpdir.path().join("report.json");

        let mut cmd = std::process::Command::new(self.info.path);
        cmd.current_dir(self.info.cwd)
            .arg("--target")
            .arg(self.info.target)
            .arg("--json")
            .arg(&report_path)
            .arg("-n");

        if self.no_library {
            cmd.arg("--nolib");
        }

        let (_, status) = run_command(
            ctx,
            cmd,
            tx,
            Some(&self.info.cancel),
            "reccmp",
            CommandOutput::Log,
        )?;
        if !status.success() {
            return Err(WorkerTaskError::CommandFailed("reccmp", status));
        }

        let report = fs::read_to_string(&report_path).map_err(|e| {
            WorkerTaskError::ReccmpReportNotFound {
                path: report_path,
                source: e,
            }
        })?;
        tx.send(Message::ReccmpFinished {
            report: serde_json::from_str(&report)?,
            generation: self.info.generation,
        })?;

        Ok(())
    }
}

pub struct StackcmpRequest {
    pub info: ToolRequestInfo,
    pub address: Address,
    pub func_name: String,
    pub focus: bool,
}

impl StackcmpRequest {
    pub fn run(self, tx: &Sender<Message>, ctx: &egui::Context) -> Result<(), WorkerTaskError> {
        let mut cmd = std::process::Command::new(self.info.path);
        cmd.current_dir(self.info.cwd)
            .arg("--target")
            .arg(self.info.target)
            .arg(format!("{}", self.address));
        let (output, status) = run_command(
            ctx,
            cmd,
            tx,
            Some(&self.info.cancel),
            "stackcmp",
            CommandOutput::Capture,
        )?;
        // this won't panic because we specify CommandOutput::Capture
        let output = output.expect("Meiling Socks");
        if !status.success() {
            for line in output.lines() {
                let _ = tx.send(Message::Log(format!("[stackcmp] {line}")));
            }
            return Err(WorkerTaskError::CommandFailed("stackcmp", status));
        }
        tx.send(Message::StackcmpFinished {
            report: StackcmpReport::new(&output, self.address, self.func_name),
            focus: self.focus,
            generation: self.info.generation,
        })?;

        Ok(())
    }
}

pub struct DatacmpRequest {
    pub info: ToolRequestInfo,
}

impl DatacmpRequest {
    pub fn run(self, tx: &Sender<Message>, ctx: &egui::Context) -> Result<(), WorkerTaskError> {
        let mut cmd = std::process::Command::new(self.info.path);
        cmd.current_dir(self.info.cwd)
            .arg("--target")
            .arg(self.info.target)
            .arg("-n")
            .arg("-a");

        let (_, status) = run_command(
            ctx,
            cmd,
            tx,
            Some(&self.info.cancel),
            "datacmp",
            CommandOutput::Log,
        )?;
        if !status.success() {
            tx.send(Message::Log(
                "datacmp found at least one problem".to_owned(),
            ))?;
        }

        tx.send(Message::DatacmpFinished(self.info.generation))?;
        Ok(())
    }
}

pub struct RoadmapRequest {
    pub info: ToolRequestInfo,
    pub focus: bool,
}

impl RoadmapRequest {
    pub fn run(self, tx: &Sender<Message>, ctx: &egui::Context) -> Result<(), WorkerTaskError> {
        let tmpdir = tempdir()?;
        let csv_path = tmpdir.path().join("roadmap.csv");
        let mut cmd = std::process::Command::new(self.info.path);
        cmd.current_dir(self.info.cwd)
            .arg("--target")
            .arg(self.info.target)
            .arg("--csv")
            .arg(&csv_path);

        let (_, status) = run_command(
            ctx,
            cmd,
            tx,
            Some(&self.info.cancel),
            "roadmap",
            CommandOutput::Log,
        )?;
        if !status.success() {
            return Err(WorkerTaskError::CommandFailed("roadmap", status));
        }

        tx.send(Message::RoadmapFinished {
            rows: RoadmapRow::from_path(&csv_path)?,
            focus: self.focus,
            generation: self.info.generation,
        })?;
        Ok(())
    }
}

pub struct WatchRequest {
    pub watch_dir: PathBuf,
    pub debounce_ms: u64,
}

impl WatchRequest {
    pub fn run(
        self,
        tx: Sender<Message>,
        ctx: egui::Context,
    ) -> Result<WatchDebouncer, WorkerTaskError> {
        if !self.watch_dir.is_dir() {
            return Err(WorkerTaskError::WatchDirNotFound {
                path: self.watch_dir,
            });
        }

        let mut debouncer = new_debouncer(
            Duration::from_millis(self.debounce_ms),
            None,
            move |res: DebounceEventResult| {
                if let Ok(events) = res {
                    let has_change = events.iter().any(|event| {
                        matches!(
                            event.kind,
                            EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Name(_))
                                | EventKind::Create(_)
                                | EventKind::Remove(_)
                        )
                    });

                    if has_change {
                        _ = tx.send(Message::SourceFileChanged);
                        ctx.request_repaint();
                    }
                }
            },
        )?;

        debouncer.watch(self.watch_dir, RecursiveMode::Recursive)?;

        Ok(debouncer)
    }

    pub fn from_project(project: &Project, debounce_ms: u64) -> Result<Self, WorkerTaskError> {
        let target = project
            .project_yml
            .targets
            .get(&project.target)
            .ok_or_else(|| WorkerTaskError::TargetNotFound(project.target.clone()))?;
        Ok(Self {
            watch_dir: project.dir.join(&target.source_root),
            debounce_ms,
        })
    }
}

enum CommandOutput {
    Log,
    Capture,
}

fn run_command(
    ctx: &egui::Context,
    mut command: std::process::Command,
    tx: &Sender<Message>,
    cancel: Option<&CancellationToken>,
    tag: &'static str,
    output: CommandOutput,
) -> Result<(Option<String>, ExitStatus), WorkerTaskError> {
    if let Some(cancel) = cancel {
        cancel.check_cancelled()?;
    }

    let (reader, writer) = std::io::pipe()?;
    let writer_err = writer.try_clone()?;

    let mut child = command
        .stdout(writer)
        .stderr(writer_err)
        .group_spawn()
        .map_err(|e| WorkerTaskError::SpawnCommand {
            cmd: command.get_program().to_string_lossy().into_owned(),
            cwd: command
                .get_current_dir()
                .map(PathBuf::from)
                .unwrap_or_default(),
            source: e,
        })?;

    drop(command);

    let io_thread = std::thread::spawn({
        let tx = tx.clone();
        let ctx = ctx.clone();
        move || {
            let mut captured = String::new();
            let mut reader = BufReader::new(reader);
            match output {
                CommandOutput::Log => {
                    for line in reader.lines().map_while(Result::ok) {
                        let _ = tx.send(Message::Log(format!("[{tag}] {line}")));
                        ctx.request_repaint();
                    }
                    None
                }
                CommandOutput::Capture => {
                    let _ = reader.read_to_string(&mut captured);
                    Some(captured)
                }
            }
        }
    });

    let status = match cancel {
        Some(cancel) => loop {
            crossbeam_channel::select! {
                recv(cancel.receiver()) -> _ => {
                    let _ = tx.send(Message::Log(format!("{tag} cancelled")));
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = io_thread.join();
                    return Err(WorkerTaskError::ToolCancelled);
                }
                default(Duration::from_millis(50)) => {
                    if let Some(status) = child.try_wait()? {
                        break status;
                    }
                }
            }
        },
        None => child.wait()?,
    };
    drop(child);

    let captured_stdout = io_thread.join().unwrap_or_default();

    Ok((captured_stdout, status))
}
