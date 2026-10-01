use crate::core::project::ProjectState;
use crate::pdf::{LopdfInspectionEngine, PdfEngine};
use crate::theme;
use eframe::egui;
use std::path::PathBuf;

pub struct GlyphApp {
    project: ProjectState,
    pdf_path_input: String,
    status: String,
    zoom: f32,
    pan: egui::Vec2,
    engine: LopdfInspectionEngine,
}

impl GlyphApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install(&cc.egui_ctx);
        Self {
            project: ProjectState::new("Untitled Glyph Set"),
            pdf_path_input: String::new(),
            status: "Open a PDF to start.".to_owned(),
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
            engine: LopdfInspectionEngine,
        }
    }

    fn open_pdf_from_input(&mut self) {
        let path = PathBuf::from(self.pdf_path_input.trim());
        match self.engine.inspect(&path) {
            Ok(summary) => {
                self.project.open_document(path.clone(), summary);
                self.status = format!("Opened {}", path.display());
                self.zoom = 1.0;
                self.pan = egui::Vec2::ZERO;
            }
            Err(err) => {
                self.status = format!("Open failed: {err}");
            }
        }
    }
}

impl eframe::App for GlyphApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("top_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Glyph");
                ui.separator();
                ui.label("Native Linux drawing-set PDF editor");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("Zoom {:.0}%", self.zoom * 100.0));
                });
            });
        });

        egui::Panel::left("sheet_sidebar")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| {
                ui.heading("Drawing Set");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label("PDF");
                    let response = ui.text_edit_singleline(&mut self.pdf_path_input);
                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        self.open_pdf_from_input();
                    }
                });
                if ui.button("Open PDF").clicked() {
                    self.open_pdf_from_input();
                }
                ui.label(&self.status);
                ui.separator();

                if let Some(document) = &self.project.document {
                    ui.label(format!("File: {}", document.display_name()));
                    ui.label(format!("Pages: {}", document.summary.page_count));
                    ui.separator();
                    ui.heading("Pages");
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for page in &document.summary.pages {
                            let label = format!(
                                "{:>3}  {}",
                                page.index + 1,
                                page.label.as_deref().unwrap_or("Page")
                            );
                            if ui
                                .selectable_label(self.project.selected_page == page.index, label)
                                .clicked()
                            {
                                self.project.selected_page = page.index;
                            }
                        }
                    });
                } else {
                    ui.monospace("No PDF loaded yet.");
                }
            });

        egui::Panel::bottom("status_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Milestone 1: native shell + PDF inspection. Rendering engine next.");
                ui.separator();
                ui.label(&self.status);
            });
        });

        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("−").clicked() { self.zoom = (self.zoom * 0.9).max(0.1); }
                if ui.button("+").clicked() { self.zoom = (self.zoom * 1.1).min(8.0); }
                if ui.button("Reset view").clicked() { self.zoom = 1.0; self.pan = egui::Vec2::ZERO; }
            });
            ui.separator();

            let available = ui.available_size();
            let (rect, response) = ui.allocate_exact_size(available, egui::Sense::drag());
            if response.dragged() {
                self.pan += response.drag_delta();
            }

            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 12.0, theme::SURFACE);
            let page_w = 620.0 * self.zoom;
            let page_h = 860.0 * self.zoom;
            let page_rect = egui::Rect::from_center_size(rect.center() + self.pan, egui::vec2(page_w, page_h));
            painter.rect_filled(page_rect, 4.0, egui::Color32::from_rgb(238, 238, 232));
            painter.rect_stroke(page_rect, 4.0, egui::Stroke::new(1.0, egui::Color32::from_gray(80)), egui::StrokeKind::Outside);

            let page_text = if let Some(document) = &self.project.document {
                format!("{}\nPage {} of {}\n\nPDF rendering engine hooks are in place.\nNext: PDFium/MuPDF raster tiles here.",
                    document.display_name(),
                    self.project.selected_page + 1,
                    document.summary.page_count)
            } else {
                "Open a PDF path from the sidebar.\n\nGlyph is starting as a native Rust workspace, not a web wrapper.".to_owned()
            };
            painter.text(
                page_rect.center(),
                egui::Align2::CENTER_CENTER,
                page_text,
                egui::FontId::proportional(22.0),
                egui::Color32::from_rgb(30, 32, 36),
            );
        });
    }
}
