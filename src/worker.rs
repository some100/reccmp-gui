use std::{
    path::PathBuf,
    process::ExitStatus,
    sync::{Arc, Condvar, Mutex},
};

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use notify_debouncer_full::notify;
use thiserror::Error;
use threadpool::ThreadPool;

use crate::{
    app::{config::Tool, message::Message},
    disassemble::DisassembleError,
    worker::{
        disassemble::DisassembleCommand,
        request::{
            CompileRequest, DatacmpRequest, ReccmpRequest, RoadmapRequest, StackcmpRequest,
            WatchDebouncer, WatchRequest,
        },
    },
};

pub mod disassemble;
pub mod request;

#[derive(Error, Debug)]
pub enum WorkerTaskError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("failed to read binary '{path}': {source}")]
    BinaryRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to exec '{cmd}' in {cwd}: {source}")]
    SpawnCommand {
        cmd: String,
        cwd: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("watch dir {path} does not exist")]
    WatchDirNotFound { path: PathBuf },

    #[error("reccmp report expected to be {path} not found: {source}")]
    ReccmpReportNotFound {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("msg send error: channel closed")]
    MsgSend,

    #[error("disasm send error: channel closed")]
    DisasmSend,

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

impl From<crossbeam_channel::SendError<Message>> for WorkerTaskError {
    fn from(_: crossbeam_channel::SendError<Message>) -> Self {
        WorkerTaskError::MsgSend
    }
}

impl From<crossbeam_channel::SendError<DisassembleCommand>> for WorkerTaskError {
    fn from(_: crossbeam_channel::SendError<DisassembleCommand>) -> Self {
        WorkerTaskError::DisasmSend
    }
}

pub enum Command {
    Compile(CompileRequest),
    Reccmp(ReccmpRequest),
    Stackcmp(StackcmpRequest),
    Datacmp(DatacmpRequest),
    Roadmap(RoadmapRequest),
    Watch(WatchRequest),
    StopWatch,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum CompileProgress {
    #[default]
    Idle,
    Compiling,
}

#[derive(Default)]
struct CompileState {
    generation: u64,
    progress: CompileProgress,
    last_compile_succeeded: Option<bool>,
}

#[derive(Clone, Default)]
struct CompileStatus {
    inner: Arc<(Mutex<CompileState>, Condvar)>,
}

impl CompileStatus {
    fn new() -> Self {
        Self {
            inner: Arc::new((Mutex::new(CompileState::default()), Condvar::new())),
        }
    }

    fn init(&self, generation: u64) {
        let (lock, cvar) = &*self.inner;
        let mut state = lock.lock().unwrap();

        state.generation = generation;
        state.progress = CompileProgress::Compiling;
        state.last_compile_succeeded = None;

        cvar.notify_all();
    }

    fn wait(&self, generation: u64) -> bool {
        let (lock, cvar) = &*self.inner;
        let mut state = lock.lock().unwrap();

        while state.generation == generation && state.progress == CompileProgress::Compiling {
            state = cvar.wait(state).unwrap();
        }

        if state.generation > generation {
            return false;
        }

        if state.generation == generation {
            state.last_compile_succeeded.unwrap_or(true)
        } else {
            true
        }
    }

    fn finish(&self, succeeded: bool, generation: u64) {
        let (lock, cvar) = &*self.inner;
        let mut state = lock.lock().unwrap();

        if state.generation == generation {
            state.progress = CompileProgress::Idle;
            state.last_compile_succeeded = Some(succeeded);
        }

        cvar.notify_all();
    }
}

pub struct Worker {
    rx: Receiver<Command>,
    tx: Sender<Message>,
    tx_disasm: Sender<DisassembleCommand>,
    ctx: egui::Context,
    debouncer: Option<WatchDebouncer>,
    compile_status: CompileStatus,
    pool: ThreadPool,
}

impl Worker {
    pub fn new(
        rx: Receiver<Command>,
        tx: Sender<Message>,
        tx_disasm: Sender<DisassembleCommand>,
        ctx: egui::Context,
    ) -> Self {
        let num_threads = std::thread::available_parallelism()
            .map_or(4, core::num::NonZero::get)
            .clamp(2, 8);

        Self {
            rx,
            tx,
            tx_disasm,
            ctx,
            debouncer: None,
            compile_status: CompileStatus::new(),
            pool: ThreadPool::new(num_threads),
        }
    }

    pub fn run(&mut self) {
        while let Ok(cmd) = self.rx.recv() {
            match cmd {
                Command::Compile(request) => {
                    self.compile_status.init(request.generation);

                    std::thread::spawn({
                        let tx = self.tx.clone();
                        let tx_disasm = self.tx_disasm.clone();
                        let ctx = self.ctx.clone();
                        let compile_status = self.compile_status.clone();
                        let generation = request.generation;

                        move || {
                            let result = request.run(&tx, &tx_disasm, &ctx);
                            compile_status.finish(result.is_ok(), generation);
                            handle_tool_result(&tx, result, Tool::Compile, generation);
                            ctx.request_repaint();
                        }
                    });
                }
                Command::Reccmp(request) => {
                    self.pool.execute({
                        let tx = self.tx.clone();
                        let ctx = self.ctx.clone();
                        let compile_status = self.compile_status.clone();
                        let generation = request.info.generation;

                        move || {
                            if !compile_status.wait(generation) {
                                let _ = tx.send(Message::ToolFailed {
                                    tool: Tool::Reccmp,
                                    generation,
                                });
                                ctx.request_repaint();
                                return;
                            }

                            let result = request.run(&tx, &ctx);
                            handle_tool_result(&tx, result, Tool::Reccmp, generation);
                            ctx.request_repaint();
                        }
                    });
                }
                Command::Stackcmp(request) => {
                    self.pool.execute({
                        let tx = self.tx.clone();
                        let ctx = self.ctx.clone();
                        let compile_status = self.compile_status.clone();
                        let address = request.address;
                        let generation = request.info.generation;

                        move || {
                            if !compile_status.wait(generation) {
                                let _ = tx.send(Message::ToolFailed {
                                    tool: Tool::Stackcmp(address),
                                    generation,
                                });
                                ctx.request_repaint();
                                return;
                            }

                            let result = request.run(&tx, &ctx);
                            handle_tool_result(&tx, result, Tool::Stackcmp(address), generation);
                            ctx.request_repaint();
                        }
                    });
                }
                Command::Datacmp(request) => {
                    self.pool.execute({
                        let tx = self.tx.clone();
                        let ctx = self.ctx.clone();
                        let compile_status = self.compile_status.clone();
                        let generation = request.info.generation;

                        move || {
                            if !compile_status.wait(generation) {
                                let _ = tx.send(Message::ToolFailed {
                                    tool: Tool::Datacmp,
                                    generation,
                                });
                                ctx.request_repaint();
                                return;
                            }

                            let result = request.run(&tx, &ctx);
                            handle_tool_result(&tx, result, Tool::Datacmp, generation);
                            ctx.request_repaint();
                        }
                    });
                }
                Command::Roadmap(request) => {
                    self.pool.execute({
                        let tx = self.tx.clone();
                        let ctx = self.ctx.clone();
                        let compile_status = self.compile_status.clone();
                        let generation = request.info.generation;

                        move || {
                            if !compile_status.wait(generation) {
                                let _ = tx.send(Message::ToolFailed {
                                    tool: Tool::Roadmap,
                                    generation,
                                });
                                ctx.request_repaint();
                                return;
                            }

                            let result = request.run(&tx, &ctx);
                            handle_tool_result(&tx, result, Tool::Roadmap, generation);
                            ctx.request_repaint();
                        }
                    });
                }
                Command::Watch(request) => {
                    let result = request.run(self.tx.clone(), self.ctx.clone());
                    match result {
                        Ok(debouncer) => self.debouncer = Some(debouncer),
                        Err(e) => {
                            let _ = self.tx.send(Message::Error(e.to_string()));
                        }
                    }
                }
                Command::StopWatch => {
                    self.debouncer.take();
                }
            }
            self.ctx.request_repaint();
        }
    }
}

fn handle_tool_result(
    tx: &Sender<Message>,
    res: Result<(), WorkerTaskError>,
    tool: Tool,
    generation: u64,
) {
    match res {
        Ok(()) => {}
        Err(e) => {
            match e {
                WorkerTaskError::CommandFailed(_, _) => {
                    let _ = tx.send(Message::Log(e.to_string()));
                }
                WorkerTaskError::ToolCancelled => {
                    let _ = tx.send(Message::ToolCancelled { tool, generation });
                    return;
                }
                _ => {
                    let _ = tx.send(Message::Error(e.to_string()));
                }
            }
            let _ = tx.send(Message::ToolFailed { tool, generation });
        }
    }
}
