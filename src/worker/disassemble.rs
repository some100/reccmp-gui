use std::{fs, path::PathBuf};

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;

use crate::{
    app::{config::DisassemblySettings, message::Message},
    disassemble::{BinaryType, Disassembler, Disassembly},
    reccmp::ReccmpReportData,
    roadmap::RoadmapRow,
    worker::WorkerTaskError,
};

pub enum DisassembleCommand {
    Disassemble(DisassembleRequest),
    UpdateResolvers(Vec<ReccmpReportData>),
    UpdateRoadmap(Option<Vec<RoadmapRow>>),
    UpdateSettings(DisassemblySettings),
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
                DisassembleCommand::UpdateSettings(settings) => {
                    disassembler.update_settings(settings);
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
        let hunks = if let Some(diff) = &request.data.diff {
            diff.iter().flat_map(|(_, h)| h.clone()).collect()
        } else {
            Vec::new()
        };

        let orig_exe = match &self.orig_exe {
            Some(bytes) => bytes,
            None => self
                .orig_exe
                .insert(fs::read(&request.orig_exe_path).map_err(|e| {
                    WorkerTaskError::BinaryRead {
                        path: request.orig_exe_path.clone(),
                        source: e,
                    }
                })?),
        };

        let mut max_known_address = None;
        for hunk in &hunks {
            if let Some(addr) = hunk.last_orig_code_address() {
                max_known_address = Some(addr);
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
            None => self
                .recomp_exe
                .insert(fs::read(&request.recomp_exe_path).map_err(|e| {
                    WorkerTaskError::BinaryRead {
                        path: request.recomp_exe_path.clone(),
                        source: e,
                    }
                })?),
        };

        let mut max_known_address = None;
        for hunk in &hunks {
            if let Some(addr) = hunk.last_recomp_code_address() {
                max_known_address = Some(addr);
            }
        }
        let recomp_disasm = disassembler.disasm(
            recomp_exe,
            request.data.recomp,
            max_known_address,
            BinaryType::Recomp,
        )?;
        let (rows, tables) = disassembler.diff(&orig_disasm, recomp_disasm, &hunks);

        let disasm = Disassembly {
            func_name: request.data.name,
            rows,
            tables,
            matching: request.data.matching,
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
