use crate::core::project::ProjectState;
use crate::pdf::{
    LopdfInspectionEngine, PdfEngine, PdfRenderEngine, PdfiumRenderEngine, RenderedPage,
};
use crate::theme;
use eframe::egui;
use std::path::{Path, PathBuf};

const INITIAL_RENDER_WIDTH: u16 = 1800;
const MIN_ZOOM: f32 = 0.1;
const MAX_ZOOM: f32 = 8.0;

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
    fit_to_page_requested: bool,
}

impl GlyphApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_pdf: Option<PathBuf>) -> Self {
        theme::install(&cc.egui_ctx);
        let mut app = Self {
            project: ProjectState::new("Untitled Glyph Set"),
            pdf_path_input: String::new(),
            status: "Open a PDF to start.".to_owned(),
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
            inspector: LopdfInspectionEngine,
            renderer: PdfiumRenderEngine,
            rendered_page: None,
            page_texture: None,
            fit_to_page_requested: false,
        };
        if let Some(path) = initial_pdf {
            app.pdf_path_input = path.display().to_string();
            app.open_pdf(path, &cc.egui_ctx);
        }
        app
    }

    fn open_pdf_from_input(&mut self, ctx: &egui::Context) {
        let path_text = self.pdf_path_input.trim();
        if path_text.is_empty() {
            self.status = "Pick a PDF or paste a path first.".to_owned();
            return;
        }
        self.open_pdf(PathBuf::from(path_text), ctx);
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
                    "Rendered page {} of {} — {}",
                    page_index + 1,
                    self.page_count().unwrap_or(0),
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

    fn page_count(&self) -> Option<usize> {
        self.project
            .document
            .as_ref()
            .map(|document| document.summary.page_count)
    }

    fn can_go_previous(&self) -> bool {
        self.project.document.is_some() && self.project.selected_page > 0
    }

    fn can_go_next(&self) -> bool {
        self.page_count()
            .map(|count| self.project.selected_page + 1 < count)
            .unwrap_or(false)
    }

    fn select_page(&mut self, page_index: usize, ctx: &egui::Context) {
        let Some(page_count) = self.page_count() else {
            return;
        };
        let page_index = page_index.min(page_count.saturating_sub(1));
        if self.project.selected_page != page_index {
            self.project.selected_page = page_index;
            self.zoom = 1.0;
            self.pan = egui::Vec2::ZERO;
            self.render_selected_page(ctx);
        }
    }

    fn next_page(&mut self, ctx: &egui::Context) {
        if self.can_go_next() {
            self.select_page(self.project.selected_page + 1, ctx);
        }
    }

    fn previous_page(&mut self, ctx: &egui::Context) {
        if self.can_go_previous() {
            self.select_page(self.project.selected_page - 1, ctx);
        }
    }

    fn fit_page_to_rect(&mut self, rect: egui::Rect) {
        let Some(rendered) = &self.rendered_page else {
            return;
        };
        let safe_width = (rect.width() - 64.0).max(100.0);
        let safe_height = (rect.height() - 64.0).max(100.0);
        let width_zoom = safe_width / rendered.width as f32;
        let height_zoom = safe_height / rendered.height as f32;
        self.zoom = width_zoom.min(height_zoom).clamp(MIN_ZOOM, MAX_ZOOM);
        self.pan = egui::Vec2::ZERO;
    }

    fn reset_view(&mut self) {
        self.zoom = 1.0;
        self.pan = egui::Vec2::ZERO;
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped_path = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .find(|path| {
                    path.extension()
                        .and_then(|extension| extension.to_str())
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
                })
        });

        if let Some(path) = dropped_path {
            self.pdf_path_input = path.display().to_string();
            self.open_pdf(path, ctx);
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::O)) {
            self.choose_pdf(ctx);
        }
        if ctx.input(|input| {
            input.key_pressed(egui::Key::ArrowRight) || input.key_pressed(egui::Key::PageDown)
        }) {
            self.next_page(ctx);
        }
        if ctx.input(|input| {
            input.key_pressed(egui::Key::ArrowLeft) || input.key_pressed(egui::Key::PageUp)
        }) {
            self.previous_page(ctx);
        }
        if ctx.input(|input| input.key_pressed(egui::Key::Home)) {
            self.select_page(0, ctx);
        }
        if ctx.input(|input| input.key_pressed(egui::Key::End)) {
            if let Some(page_count) = self.page_count() {
                self.select_page(page_count.saturating_sub(1), ctx);
            }
        }
        if ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::Num0)) {
            self.reset_view();
        }
    }
}

impl eframe::App for GlyphApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_dropped_files(&ctx);
        self.handle_shortcuts(&ctx);

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
                ui.small("Tip: glyph /path/to/file.pdf also opens directly. You can drag a PDF onto the window.");
                ui.label(&self.status);
                ui.separator();

                if let Some(document) = &self.project.document {
                    let display_name = document.display_name();
                    let page_count = document.summary.page_count;
                    let pages = document.summary.pages.clone();
                    let bookmarks = document.summary.bookmarks.clone();
                    ui.label(format!("File: {display_name}"));
                    ui.label(format!(
                        "Page: {} / {}",
                        self.project.selected_page + 1,
                        page_count
                    ));
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(self.can_go_previous(), egui::Button::new("Previous"))
                            .clicked()
                        {
                            self.previous_page(&ctx);
                        }
                        if ui
                            .add_enabled(self.can_go_next(), egui::Button::new("Next"))
                            .clicked()
                        {
                            self.next_page(&ctx);
                        }
                    });
                    ui.separator();
                    ui.heading("Bookmarks");
                    if bookmarks.is_empty() {
                        ui.small("No PDF outline bookmarks found.");
                    } else {
                        egui::ScrollArea::vertical()
                            .id_salt("bookmark_list")
                            .max_height(180.0)
                            .show(ui, |ui| {
                                for bookmark in bookmarks {
                                    let indent = 12.0 * bookmark.depth as f32;
                                    ui.horizontal(|ui| {
                                        ui.add_space(indent);
                                        let target = bookmark
                                            .page_index
                                            .map(|page_index| format!("p{}", page_index + 1))
                                            .unwrap_or_else(|| "—".to_owned());
                                        let label = format!("{}  {target}", bookmark.title);
                                        if ui
                                            .add_enabled(
                                                bookmark.page_index.is_some(),
                                                egui::Button::new(label).selected(
                                                    bookmark.page_index == Some(self.project.selected_page),
                                                ),
                                            )
                                            .clicked()
                                        {
                                            if let Some(page_index) = bookmark.page_index {
                                                self.select_page(page_index, &ctx);
                                            }
                                        }
                                    });
                                }
                            });
                    }

                    ui.separator();
                    ui.heading("Pages");
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
                    ui.add_space(8.0);
                    ui.label("Open a PDF with the button above, paste a path, or launch Glyph with a PDF path.");
                }
            });

        egui::Panel::bottom("status_bar").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Shortcuts: Ctrl+O open · ←/→ pages · Home/End first/last · drag pan · scroll zoom");
                ui.separator();
                ui.label(&self.status);
            });
        });

        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("−").clicked() {
                    self.zoom = (self.zoom * 0.9).max(MIN_ZOOM);
                }
                if ui.button("+").clicked() {
                    self.zoom = (self.zoom * 1.1).min(MAX_ZOOM);
                }
                if ui.button("Reset").clicked() {
                    self.reset_view();
                }
                if ui.button("Fit page").clicked() {
                    self.fit_to_page_requested = true;
                }
                if self.project.document.is_some() && ui.button("Re-render").clicked() {
                    self.render_selected_page(&ctx);
                }
            });
            ui.separator();

            let available = ui.available_size();
            let (rect, response) = ui.allocate_exact_size(available, egui::Sense::drag());
            if self.fit_to_page_requested {
                self.fit_page_to_rect(rect);
                self.fit_to_page_requested = false;
            }
            if response.dragged() {
                self.pan += response.drag_delta();
            }
            if response.hovered() {
                let scroll_y = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll_y.abs() > 0.0 {
                    let scale = if scroll_y > 0.0 { 1.08 } else { 0.92 };
                    self.zoom = (self.zoom * scale).clamp(MIN_ZOOM, MAX_ZOOM);
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
                    "Open a PDF to render it here.\n\nDrag to pan. Scroll or +/- to zoom.\nLaunch with: glyph file.pdf",
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
