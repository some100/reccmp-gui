use eframe::egui::{
    self, Color32, Label, Response, RichText, ScrollArea, TextFormat, TextStyle, Widget,
    WidgetText, text::LayoutJob,
};
use std::path::{Path, PathBuf};

use crate::disassemble::{Instruction, diff::DiffKind};

pub const MATCH_TEXT: Color32 = Color32::from_rgb(190, 190, 180);
pub const SEPARATOR_TEXT: Color32 = Color32::from_rgb(110, 110, 110);

pub enum PickTarget {
    File,
    Folder,
}

pub struct PathPicker<'a> {
    label: &'a str,
    path: &'a mut PathBuf,
    root: Option<&'a Path>,
    target: PickTarget,
}

impl<'a> PathPicker<'a> {
    pub fn file(label: &'a str, path: &'a mut PathBuf) -> Self {
        Self {
            label,
            path,
            root: None,
            target: PickTarget::File,
        }
    }

    pub fn folder(label: &'a str, path: &'a mut PathBuf) -> Self {
        Self {
            label,
            path,
            root: None,
            target: PickTarget::Folder,
        }
    }

    pub fn relative_to(mut self, root: &'a Path) -> Self {
        self.root = Some(root);
        self
    }

    pub fn as_relative(root: &Path, target: &Path) -> PathBuf {
        match target.strip_prefix(root) {
            Ok(rel) if rel.is_empty() => PathBuf::from("."),
            Ok(rel) => rel.to_path_buf(),
            Err(_) => target.to_path_buf(),
        }
    }
}

impl Widget for PathPicker<'_> {
    fn ui(self, ui: &mut egui::Ui) -> Response {
        ui.horizontal(|ui| {
            ui.label(self.label);
            ui.label(self.path.to_string_lossy());
            if ui.button("Browse...").clicked() {
                let mut dialog = rfd::FileDialog::new();
                if let Some(root) = self.root
                    && !root.is_empty()
                {
                    dialog = dialog.set_directory(root);
                }

                let chosen = match self.target {
                    PickTarget::File => dialog.pick_file(),
                    PickTarget::Folder => dialog.pick_folder(),
                };

                if let Some(target) = chosen {
                    *self.path = match self.root {
                        Some(root) => Self::as_relative(root, &target),
                        None => target,
                    };
                }
            }
        })
        .response
    }
}

pub struct TableHeader<'a> {
    columns: &'a [(&'a str, f32)],
    trailing_column: Option<&'a str>,
}

impl<'a> TableHeader<'a> {
    pub fn new(columns: &'a [(&'a str, f32)]) -> Self {
        Self {
            columns,
            trailing_column: None,
        }
    }

    pub fn with_trailing(mut self, title: &'a str) -> Self {
        self.trailing_column = Some(title);
        self
    }
}

impl Widget for TableHeader<'_> {
    fn ui(self, ui: &mut egui::Ui) -> Response {
        ui.horizontal(|ui| {
            ui.style_mut().override_text_style = Some(TextStyle::Monospace);
            for &(title, width) in self.columns {
                ui.add_sized([width, 18.0], Label::new(RichText::new(title).strong()));
            }
            if let Some(trailing) = self.trailing_column {
                ui.label(RichText::new(trailing).strong());
            }
        })
        .response
    }
}

pub enum TableTrailing<'a> {
    Text(WidgetText),
    Custom(&'a mut dyn FnMut(&mut egui::Ui) -> Response),
}

pub struct TableRow<'a> {
    columns: &'a [(&'a str, f32)],
    row_height: f32,
    cells: Vec<WidgetText>,
    trailing: Option<TableTrailing<'a>>,
}

impl<'a> TableRow<'a> {
    pub fn new(columns: &'a [(&'a str, f32)], row_height: f32) -> Self {
        Self {
            columns,
            row_height,
            cells: Vec::with_capacity(columns.len()),
            trailing: None,
        }
    }

    pub fn cell(mut self, text: impl Into<WidgetText>) -> Self {
        self.cells.push(text.into());
        self
    }

    pub fn trailing(mut self, text: impl Into<WidgetText>) -> Self {
        self.trailing = Some(TableTrailing::Text(text.into()));
        self
    }

    pub fn trailing_ui(mut self, render: &'a mut dyn FnMut(&mut egui::Ui) -> Response) -> Self {
        self.trailing = Some(TableTrailing::Custom(render));
        self
    }
}

impl Widget for TableRow<'_> {
    fn ui(self, ui: &mut egui::Ui) -> Response {
        ui.horizontal(|ui| {
            for (i, cell) in self.cells.into_iter().enumerate() {
                let width = self.columns.get(i).map_or(60.0, |col| col.1);
                let size = [width, self.row_height];

                ui.add_sized(size, Label::new(cell).truncate());
            }

            match self.trailing {
                Some(TableTrailing::Text(text)) => {
                    ui.label(text);
                }
                Some(TableTrailing::Custom(render)) => {
                    render(ui);
                }
                None => {}
            }
        })
        .response
    }
}

pub struct VirtualTable<'a, F> {
    columns: &'a [(&'a str, f32)],
    trailing_column: Option<&'a str>,
    total_rows: usize,
    row_height_padding: f32,
    render_row: F,
}

impl<'a, F> VirtualTable<'a, F>
where
    F: FnMut(&mut egui::Ui, usize, f32),
{
    pub fn new(columns: &'a [(&'a str, f32)], total_rows: usize, render_row: F) -> Self {
        Self {
            columns,
            trailing_column: None,
            total_rows,
            row_height_padding: 4.0,
            render_row,
        }
    }

    pub fn with_trailing(mut self, title: &'a str) -> Self {
        self.trailing_column = Some(title);
        self
    }
}

impl<F> Widget for VirtualTable<'_, F>
where
    F: FnMut(&mut egui::Ui, usize, f32),
{
    fn ui(mut self, ui: &mut egui::Ui) -> Response {
        ui.scope(|ui| {
            let row_height = ui.text_style_height(&TextStyle::Monospace) + self.row_height_padding;

            let mut header = TableHeader::new(self.columns);
            if let Some(trailing) = self.trailing_column {
                header = header.with_trailing(trailing);
            }
            ui.add(header);

            ScrollArea::both().auto_shrink([false, false]).show_rows(
                ui,
                row_height,
                self.total_rows,
                |ui, row_range| {
                    ui.style_mut().override_text_style = Some(TextStyle::Monospace);
                    for idx in row_range {
                        (self.render_row)(ui, idx, row_height);
                    }
                },
            );
        })
        .response
    }
}

pub struct InstructionRow<'a> {
    instruction: Option<&'a Instruction>,
    op_diffs: &'a [u32],
    kind: DiffKind,
    diff_color: Color32,
    bg_color: Color32,
}

impl<'a> InstructionRow<'a> {
    pub fn new(
        instruction: Option<&'a Instruction>,
        op_diffs: &'a [u32],
        kind: DiffKind,
        diff_color: Color32,
        bg_color: Color32,
    ) -> Self {
        Self {
            instruction,
            op_diffs,
            kind,
            diff_color,
            bg_color,
        }
    }
}

impl Widget for InstructionRow<'_> {
    fn ui(self, ui: &mut egui::Ui) -> Response {
        let Some(instr) = self.instruction else {
            return ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover());
        };

        let mut frame = egui::Frame::new();
        if self.bg_color != Color32::TRANSPARENT {
            frame = frame.fill(self.bg_color);
        }

        frame
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.style_mut().override_text_style = Some(TextStyle::Monospace);

                ui.horizontal(|ui| {
                    ui.colored_label(Color32::GRAY, &instr.address_str);

                    let font_id = TextStyle::Monospace.resolve(ui.style());
                    let mut job = LayoutJob::default();

                    let mnemonic_is_diff = matches!(
                        self.kind,
                        DiffKind::Diff | DiffKind::Added | DiffKind::Removed
                    );

                    let mnemonic_color = if mnemonic_is_diff {
                        self.diff_color
                    } else {
                        MATCH_TEXT
                    };

                    let mnemonic_text = if instr.operands.is_empty() {
                        instr.mnemonic.clone()
                    } else {
                        format!("{:<6}", instr.mnemonic)
                    };

                    job.append(
                        &mnemonic_text,
                        0.0,
                        TextFormat {
                            color: mnemonic_color,
                            font_id: font_id.clone(),
                            ..Default::default()
                        },
                    );

                    for (i, op) in instr.operands.iter().enumerate() {
                        job.append(
                            if i == 0 { " " } else { ", " },
                            0.0,
                            TextFormat {
                                color: SEPARATOR_TEXT,
                                font_id: font_id.clone(),
                                ..Default::default()
                            },
                        );

                        let is_op_diff = match self.kind {
                            DiffKind::ArgDiff | DiffKind::Advisory => {
                                self.op_diffs.contains(&(i as u32))
                            }
                            DiffKind::Diff | DiffKind::Added | DiffKind::Removed => true,
                            DiffKind::Matched => false,
                        };

                        let op_color = if is_op_diff {
                            self.diff_color
                        } else {
                            MATCH_TEXT
                        };

                        job.append(
                            op,
                            0.0,
                            TextFormat {
                                color: op_color,
                                font_id: font_id.clone(),
                                ..Default::default()
                            },
                        );
                    }

                    if let Some(comment) = &instr.comment {
                        job.append(
                            &format!(" {comment}"),
                            0.0,
                            TextFormat {
                                color: Color32::GRAY,
                                font_id: font_id.clone(),
                                ..Default::default()
                            },
                        );
                    }

                    ui.add(Label::new(job).truncate());
                });
            })
            .response
    }
}
