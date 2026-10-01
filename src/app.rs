use crate::core::project::ProjectState;
use crate::pdf::{
    LopdfInspectionEngine, PdfEngine, PdfRenderEngine, PdfiumRenderEngine, RenderedPage,
};
use crate::theme;
use eframe::egui;
use std::path::{Path, PathBuf};

const INITIAL_RENDER_WIDTH: u16 = 1600;

pub struct GlyphApp {
    project: ProjectState,
    pdf_path_input: String,
    status: String,
    zoom: f32,
    pan: egui::Vec2,
    inspector: LopdfInspectionEngine,
    renderer: PdfiumRenderEngine,
    rendered_page: Option<RenderedPage>,
    page_texture: Option<egui::TextureHandle>,
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
            inspector: LopdfInspectionEngine,
            renderer: PdfiumRenderEngine,
            rendered_page: None,
            page_texture: None,
        }
    }

    fn open_pdf_from_input(&mut self, ctx: &egui::Context) {
        let path = PathBuf::from(self.pdf_path_input.trim());
        self.open_pdf(path, ctx);
    }

    fn choose_pdf(&mut self, ctx: &egui::Context) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Open PDF in Glyph")
            .add_filter("PDF documents", &["pdf"])
            .pick_file()
        {
            self.pdf_path_input = path.display().to_string();
            self.open_pdf(path, ctx);
        }
    }

    fn open_pdf(&mut self, path: PathBuf, ctx: &egui::Context) {
        match self.inspector.inspect(&path) {
            Ok(summary) => {
                self.project.open_document(path.clone(), summary);
                self.zoom = 1.0;
                self.pan = egui::Vec2::ZERO;
                self.rendered_page = None;
                self.page_texture = None;
                self.status = format!("Opened {}", path.display());
                self.render_selected_page(ctx);
            }
            Err(err) => {
                self.status = format!("Open failed: {err}");
            }
        }
    }

    fn render_selected_page(&mut self, ctx: &egui::Context) {
        let Some(document) = &self.project.document else {
            return;
        };
        let path = document.path.clone();
        let page_index = self.project.selected_page;
        self.status = format!("Rendering page {}…", page_index + 1);
        match self
            .renderer
            .render_page(&path, page_index, INITIAL_RENDER_WIDTH)
        {
            Ok(rendered) => {
                self.install_texture(ctx, rendered);
                self.status = format!(
                    "Rendered page {} from {}",
                    page_index + 1,
                    display_name(&path)
                );
            }
            Err(err) => {
                self.rendered_page = None;
                self.page_texture = None;
                self.status = format!("Render failed: {err}");
            }
        }
    }

    fn install_texture(&mut self, ctx: &egui::Context, rendered: RenderedPage) {
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [rendered.width, rendered.height],
            &rendered.rgba,
        );
        let texture = ctx.load_texture(
            format!(
                "glyph-page-{}-{}x{}",
                rendered.page_index, rendered.width, rendered.height
            ),
            image,
            egui::TextureOptions::LINEAR,
        );
        self.rendered_page = Some(rendered);
        self.page_texture = Some(texture);
    }

    fn select_page(&mut self, page_index: usize, ctx: &egui::Context) {
        if self.project.selected_page != page_index {
            self.project.selected_page = page_index;
            self.zoom = 1.0;
            self.pan = egui::Vec2::ZERO;
            self.render_selected_page(ctx);
        }
    }
}

impl eframe::App for GlyphApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
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
            .default_size(320.0)
            .show(ui, |ui| {
                ui.heading("Drawing Set");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Choose PDF…").clicked() {
                        self.choose_pdf(&ctx);
                    }
                    if ui.button("Open path").clicked() {
                        self.open_pdf_from_input(&ctx);
                    }
                });
                let response = ui.text_edit_singleline(&mut self.pdf_path_input);
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.open_pdf_from_input(&ctx);
                }
                ui.label(&self.status);
                ui.separator();

                if let Some(document) = &self.project.document {
                    ui.label(format!("File: {}", document.display_name()));
                    ui.label(format!("Pages: {}", document.summary.page_count));
                    ui.separator();
                    ui.heading("Pages");
                    let pages = document.summary.pages.clone();
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for page in pages {
                            let label = format!(
                                "{:>3}  {}",
                                page.index + 1,
                                page.label.as_deref().unwrap_or("Page")
                            );
                            if ui
                                .selectable_label(self.project.selected_page == page.index, label)
                                .clicked()
                            {
                                self.select_page(page.index, &ctx);
                            }
                        }
                    });
                } else {
                    ui.monospace("No PDF loaded yet.");
                }
            });

        egui::Panel::bottom("status_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Milestone 2: PDFium-backed rendering, zoom, pan, page sidebar.");
                ui.separator();
                ui.label(&self.status);
            });
        });

        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("−").clicked() {
                    self.zoom = (self.zoom * 0.9).max(0.1);
                }
                if ui.button("+").clicked() {
                    self.zoom = (self.zoom * 1.1).min(8.0);
                }
                if ui.button("Reset view").clicked() {
                    self.zoom = 1.0;
                    self.pan = egui::Vec2::ZERO;
                }
                if self.project.document.is_some() && ui.button("Re-render").clicked() {
                    self.render_selected_page(&ctx);
                }
            });
            ui.separator();

            let available = ui.available_size();
            let (rect, response) = ui.allocate_exact_size(available, egui::Sense::drag());
            if response.dragged() {
                self.pan += response.drag_delta();
            }
            if response.hovered() {
                let scroll_y = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll_y.abs() > 0.0 {
                    let scale = if scroll_y > 0.0 { 1.08 } else { 0.92 };
                    self.zoom = (self.zoom * scale).clamp(0.1, 8.0);
                }
            }

            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 12.0, theme::SURFACE);

            if let (Some(rendered), Some(texture)) = (&self.rendered_page, &self.page_texture) {
                let page_w = rendered.width as f32 * self.zoom;
                let page_h = rendered.height as f32 * self.zoom;
                let page_rect = egui::Rect::from_center_size(
                    rect.center() + self.pan,
                    egui::vec2(page_w, page_h),
                );
                painter.rect_filled(
                    page_rect.expand(16.0),
                    8.0,
                    egui::Color32::from_black_alpha(90),
                );
                painter.image(
                    texture.id(),
                    page_rect,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
                painter.rect_stroke(
                    page_rect,
                    4.0,
                    egui::Stroke::new(1.0, egui::Color32::from_gray(80)),
                    egui::StrokeKind::Outside,
                );
            } else {
                let page_rect =
                    egui::Rect::from_center_size(rect.center(), egui::vec2(620.0, 860.0));
                painter.rect_filled(page_rect, 4.0, egui::Color32::from_rgb(238, 238, 232));
                painter.rect_stroke(
                    page_rect,
                    4.0,
                    egui::Stroke::new(1.0, egui::Color32::from_gray(80)),
                    egui::StrokeKind::Outside,
                );
                painter.text(
                    page_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Open a PDF to render it here.\n\nDrag to pan. Scroll or +/- to zoom.",
                    egui::FontId::proportional(22.0),
                    egui::Color32::from_rgb(30, 32, 36),
                );
            }
        });
    }
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("PDF")
        .to_owned()
}
