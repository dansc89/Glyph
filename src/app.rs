use crate::core::project::ProjectState;
use crate::pdf::{
    LopdfInspectionEngine, PdfEngine, PdfRenderEngine, PdfiumRenderEngine, RenderedPage,
};
use crate::theme;
use eframe::egui;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const BASE_RENDER_WIDTH: u16 = 1800;
const MAX_RENDER_WIDTH: u16 = 8192;
const RERENDER_UPSCALE_THRESHOLD: f32 = 1.15;
const ZOOM_RERENDER_IDLE: Duration = Duration::from_millis(180);
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
    page_aspect_ratio: Option<f32>,
    fit_to_page_requested: bool,
    last_canvas_pointer: Option<egui::Pos2>,
    last_zoom_change: Option<Instant>,
}

impl GlyphApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_pdf: Option<PathBuf>) -> Self {
        theme::install(&cc.egui_ctx);
        let mut app = Self {
            project: ProjectState::new("Untitled Glyph Set"),
            pdf_path_input: String::new(),
            status: "Ready — drop a PDF, paste a path, or press Ctrl+O.".to_owned(),
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
            inspector: LopdfInspectionEngine,
            renderer: PdfiumRenderEngine,
            rendered_page: None,
            page_texture: None,
            page_aspect_ratio: None,
            fit_to_page_requested: false,
            last_canvas_pointer: None,
            last_zoom_change: None,
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
            self.status = "Paste a PDF path first.".to_owned();
            return;
        }
        self.open_pdf(PathBuf::from(path_text), ctx);
    }

    fn choose_pdf(&mut self, ctx: &egui::Context) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Choose PDF in Glyph")
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
                self.page_aspect_ratio = None;
                self.last_zoom_change = None;
                self.status = format!("Loaded {}", path.display());
                self.render_selected_page(ctx, BASE_RENDER_WIDTH);
            }
            Err(err) => {
                self.status = format!("Load failed: {err}");
            }
        }
    }

    fn render_selected_page(&mut self, ctx: &egui::Context, target_width: u16) {
        let Some(document) = &self.project.document else {
            return;
        };
        let path = document.path.clone();
        let page_index = self.project.selected_page;
        self.status = format!("Rendering page {}…", page_index + 1);
        match self.renderer.render_page(&path, page_index, target_width) {
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
                self.status = format!("Render failed: {err}");
            }
        }
    }

    fn install_texture(&mut self, ctx: &egui::Context, rendered: RenderedPage) {
        if rendered.width > 0 {
            self.page_aspect_ratio = Some(rendered.height as f32 / rendered.width as f32);
        }
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

    fn window_title(&self) -> String {
        self.project
            .document
            .as_ref()
            .map(|document| {
                format!(
                    "{} · {} sheets",
                    document.display_name(),
                    document.summary.page_count
                )
            })
            .unwrap_or_else(|| "Professional PDF review".to_owned())
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
            self.page_aspect_ratio = None;
            self.render_selected_page(ctx, BASE_RENDER_WIDTH);
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
        let logical_size = self.logical_page_size(rendered);
        let width_zoom = safe_width / logical_size.x;
        let height_zoom = safe_height / logical_size.y;
        self.zoom = width_zoom.min(height_zoom).clamp(MIN_ZOOM, MAX_ZOOM);
        self.pan = egui::Vec2::ZERO;
        self.last_zoom_change = Some(Instant::now());
    }

    fn logical_page_size(&self, rendered: &RenderedPage) -> egui::Vec2 {
        let aspect_ratio = self
            .page_aspect_ratio
            .unwrap_or_else(|| rendered.height as f32 / rendered.width.max(1) as f32);
        egui::vec2(
            BASE_RENDER_WIDTH as f32,
            BASE_RENDER_WIDTH as f32 * aspect_ratio,
        )
    }

    fn ensure_render_quality(&mut self, ctx: &egui::Context) {
        let Some(rendered) = &self.rendered_page else {
            return;
        };
        let desired_width = desired_render_width(self.zoom, ctx.pixels_per_point());
        if desired_width as f32 <= rendered.width as f32 * RERENDER_UPSCALE_THRESHOLD {
            return;
        }

        if let Some(last_zoom_change) = self.last_zoom_change {
            let elapsed = last_zoom_change.elapsed();
            if elapsed < ZOOM_RERENDER_IDLE {
                ctx.request_repaint_after(ZOOM_RERENDER_IDLE - elapsed);
                return;
            }
        }

        self.render_selected_page(ctx, desired_width);
        self.last_zoom_change = None;
    }

    fn reset_view(&mut self) {
        self.zoom = 1.0;
        self.pan = egui::Vec2::ZERO;
        self.last_zoom_change = Some(Instant::now());
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

        egui::Panel::top("title_bar")
            .frame(
                egui::Frame::new()
                    .fill(theme::SURFACE)
                    .stroke(egui::Stroke::new(1.0, theme::STROKE))
                    .inner_margin(egui::Margin::symmetric(12, 7)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(
                        egui::RichText::new(self.window_title())
                            .size(13.0)
                            .color(theme::TEXT),
                    );
                    ui.add_space(12.0);
                    if toolbar_button(ui, "Fit").clicked() {
                        self.fit_to_page_requested = true;
                    }
                    if toolbar_button(ui, "Actual").clicked() {
                        self.reset_view();
                    }
                    if self.project.document.is_some() && toolbar_button(ui, "Reload").clicked() {
                        self.render_selected_page(
                            &ctx,
                            desired_render_width(self.zoom, ctx.pixels_per_point()),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        metric_pill(ui, &format_zoom_label(self.zoom));
                        if let Some(count) = self.page_count() {
                            metric_pill(
                                ui,
                                &format_page_counter(self.project.selected_page, count),
                            );
                        }
                    });
                });
            });

        egui::Panel::left("sheet_sidebar")
            .resizable(true)
            .default_size(328.0)
            .size_range(260.0..=420.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .stroke(egui::Stroke::new(1.0, theme::STROKE))
                    .inner_margin(egui::Margin::symmetric(10, 12)),
            )
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new("PDF path")
                            .size(11.0)
                            .strong()
                            .color(theme::TEXT_MUTED),
                    );
                    ui.add_space(4.0);
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.pdf_path_input)
                            .hint_text("/path/to/drawing-set.pdf  ↵")
                            .desired_width(f32::INFINITY),
                    );
                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        self.open_pdf_from_input(&ctx);
                    }
                    ui.add_space(12.0);

                    if let Some(document) = &self.project.document {
                        let display_name = document.display_name();
                        let page_count = document.summary.page_count;
                        let pages = document.summary.pages.clone();
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(display_name)
                                    .size(13.0)
                                    .color(theme::TEXT),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new(format_page_counter(
                                            self.project.selected_page,
                                            page_count,
                                        ))
                                        .size(12.0)
                                        .color(theme::TEXT_MUTED),
                                    );
                                },
                            );
                        });
                        ui.add_space(7.0);
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(
                                    self.can_go_previous(),
                                    egui::Button::new("Previous")
                                        .fill(theme::CONTROL)
                                        .corner_radius(5),
                                )
                                .clicked()
                            {
                                self.previous_page(&ctx);
                            }
                            if ui
                                .add_enabled(
                                    self.can_go_next(),
                                    egui::Button::new("Next")
                                        .fill(theme::CONTROL)
                                        .corner_radius(5),
                                )
                                .clicked()
                            {
                                self.next_page(&ctx);
                            }
                        });
                        ui.add_space(14.0);

                        section_header(ui, "Pages");
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            for page in pages {
                                let is_selected = self.project.selected_page == page.index;
                                let title = page.label.as_deref().unwrap_or("Page");
                                let label = format!("{:>3}   {title}", page.index + 1);
                                if page_row(ui, &label, is_selected).clicked() {
                                    self.select_page(page.index, &ctx);
                                }
                            }
                        });
                    } else {
                        egui::Frame::new()
                            .fill(theme::CARD)
                            .stroke(egui::Stroke::new(1.0, theme::STROKE))
                            .corner_radius(egui::CornerRadius::same(10))
                            .inner_margin(egui::Margin::same(14))
                            .show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new("No PDF loaded")
                                        .size(16.0)
                                        .strong()
                                        .color(theme::TEXT),
                                );
                                ui.add_space(4.0);
                                ui.label(
                                    egui::RichText::new(
                                        "Drop a PDF here, press Ctrl+O, or paste a path and press Enter.",
                                    )
                                    .color(theme::TEXT_MUTED),
                                );
                            });
                    }
                });
            });

        egui::Panel::bottom("status_bar")
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .stroke(egui::Stroke::new(1.0, theme::STROKE))
                    .inner_margin(egui::Margin::symmetric(12, 6)),
            )
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new(&self.status).color(theme::TEXT_MUTED));
                    ui.separator();
                    ui.label(
                        egui::RichText::new(
                            "Ctrl+O file picker · Enter loads path · ←/→ sheets · drag pan · scroll zoom",
                        )
                        .color(theme::TEXT_MUTED),
                    );
                });
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::CANVAS)
                    .inner_margin(egui::Margin::same(14)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if tool_chip(ui, "−").clicked() {
                        self.zoom = (self.zoom * 0.9).max(MIN_ZOOM);
                        self.last_zoom_change = Some(Instant::now());
                    }
                    metric_pill(ui, &format_zoom_label(self.zoom));
                    if tool_chip(ui, "+").clicked() {
                        self.zoom = (self.zoom * 1.1).min(MAX_ZOOM);
                        self.last_zoom_change = Some(Instant::now());
                    }
                    ui.add_space(8.0);
                    if tool_chip(ui, "Fit page").clicked() {
                        self.fit_to_page_requested = true;
                    }
                    if tool_chip(ui, "Reset").clicked() {
                        self.reset_view();
                    }
                });
                ui.add_space(12.0);

                let available = ui.available_size();
                let (rect, response) = ui.allocate_exact_size(available, egui::Sense::drag());
                if self.fit_to_page_requested {
                    self.fit_page_to_rect(rect);
                    self.fit_to_page_requested = false;
                }
                if response.dragged() {
                    self.pan += ui.input(|i| i.pointer.delta());
                }
                if let Some(pointer) = response.hover_pos() {
                    self.last_canvas_pointer = Some(pointer);
                }

                if response.hovered() {
                    let pointer = response
                        .hover_pos()
                        .or(self.last_canvas_pointer)
                        .filter(|pos| rect.contains(*pos))
                        .unwrap_or_else(|| rect.center());
                    let pinch_scale = ui.input(|i| i.zoom_delta());
                    if (pinch_scale - 1.0).abs() > f32::EPSILON {
                        (self.zoom, self.pan) =
                            zoom_around_pointer(self.zoom, self.pan, pinch_scale, pointer, rect);
                        self.last_zoom_change = Some(Instant::now());
                    } else {
                        let scroll_y = ui.input(|i| i.smooth_scroll_delta.y);
                        if scroll_y.abs() > 0.0 {
                            let scale = if scroll_y > 0.0 { 1.08 } else { 0.92 };
                            (self.zoom, self.pan) =
                                zoom_around_pointer(self.zoom, self.pan, scale, pointer, rect);
                            self.last_zoom_change = Some(Instant::now());
                        }
                    }
                }

                self.ensure_render_quality(ui.ctx());

                let painter = ui.painter_at(rect);
                draw_canvas_backdrop(&painter, rect);

                if let (Some(rendered), Some(texture)) = (&self.rendered_page, &self.page_texture) {
                    let logical_size = self.logical_page_size(rendered);
                    let page_w = logical_size.x * self.zoom;
                    let page_h = logical_size.y * self.zoom;
                    let page_rect = egui::Rect::from_center_size(
                        rect.center() + self.pan,
                        egui::vec2(page_w, page_h),
                    );
                    painter.rect_filled(
                        page_rect.expand(18.0).translate(egui::vec2(0.0, 8.0)),
                        12.0,
                        egui::Color32::from_black_alpha(110),
                    );
                    painter.rect_filled(
                        page_rect.expand(5.0),
                        8.0,
                        egui::Color32::from_rgb(226, 228, 232),
                    );
                    painter.image(
                        texture.id(),
                        page_rect,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                    painter.rect_stroke(
                        page_rect,
                        6.0,
                        egui::Stroke::new(1.0, egui::Color32::from_gray(92)),
                        egui::StrokeKind::Outside,
                    );
                } else {
                    draw_empty_state(ui, rect);
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

fn format_zoom_label(zoom: f32) -> String {
    format!("{:.0}%", zoom * 100.0)
}

fn desired_render_width(zoom: f32, pixels_per_point: f32) -> u16 {
    let width = BASE_RENDER_WIDTH as f32 * zoom.max(1.0) * pixels_per_point.max(1.0);
    width
        .round()
        .clamp(BASE_RENDER_WIDTH as f32, MAX_RENDER_WIDTH as f32) as u16
}

fn zoom_around_pointer(
    old_zoom: f32,
    old_pan: egui::Vec2,
    zoom_factor: f32,
    pointer: egui::Pos2,
    viewport: egui::Rect,
) -> (f32, egui::Vec2) {
    let new_zoom = (old_zoom * zoom_factor).clamp(MIN_ZOOM, MAX_ZOOM);
    if (new_zoom - old_zoom).abs() <= f32::EPSILON {
        return (new_zoom, old_pan);
    }

    let old_page_center = viewport.center() + old_pan;
    let document_point_under_pointer = (pointer - old_page_center) / old_zoom;
    let new_page_center = pointer - document_point_under_pointer * new_zoom;
    (new_zoom, new_page_center - viewport.center())
}

fn format_page_counter(selected_page: usize, page_count: usize) -> String {
    format!("Page {} / {}", selected_page + 1, page_count)
}

fn toolbar_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(theme::TEXT).size(12.0))
            .fill(theme::CONTROL)
            .stroke(egui::Stroke::new(1.0, theme::STROKE))
            .corner_radius(egui::CornerRadius::same(6))
            .min_size(egui::vec2(68.0, 26.0)),
    )
}

fn draw_canvas_backdrop(painter: &egui::Painter, rect: egui::Rect) {
    painter.rect_filled(rect, 10.0, theme::CANVAS);
    painter.rect_stroke(
        rect,
        10.0,
        egui::Stroke::new(1.0, theme::STROKE),
        egui::StrokeKind::Inside,
    );
}

fn tool_chip(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(theme::TEXT).size(12.0))
            .fill(theme::PANEL_RAISED)
            .stroke(egui::Stroke::new(1.0, theme::STROKE))
            .corner_radius(egui::CornerRadius::same(6))
            .min_size(egui::vec2(36.0, 26.0)),
    )
}

fn metric_pill(ui: &mut egui::Ui, label: &str) {
    egui::Frame::new()
        .fill(theme::PANEL_RAISED)
        .stroke(egui::Stroke::new(1.0, theme::STROKE))
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(label)
                    .size(12.0)
                    .color(theme::TEXT_MUTED),
            );
        });
}

fn section_header(ui: &mut egui::Ui, label: &str) {
    ui.label(
        egui::RichText::new(label.to_uppercase())
            .size(11.0)
            .strong()
            .color(theme::TEXT_MUTED),
    );
    ui.add_space(4.0);
}

fn page_row(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let fill = if selected {
        theme::ACCENT
    } else {
        theme::PANEL_RAISED
    };
    let text = if selected {
        egui::Color32::WHITE
    } else {
        theme::TEXT
    };
    ui.add(
        egui::Button::new(
            egui::RichText::new(label)
                .monospace()
                .color(text)
                .size(13.0),
        )
        .selected(selected)
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, theme::STROKE))
        .corner_radius(egui::CornerRadius::same(6))
        .min_size(egui::vec2(ui.available_width(), 28.0)),
    )
}

fn empty_sidebar_note(ui: &mut egui::Ui, note: &str) {
    egui::Frame::new()
        .fill(theme::SURFACE)
        .stroke(egui::Stroke::new(1.0, theme::STROKE))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(note)
                    .color(theme::TEXT_MUTED)
                    .size(12.0),
            );
        });
}

fn draw_empty_state(ui: &mut egui::Ui, rect: egui::Rect) {
    let painter = ui.painter_at(rect);
    let panel = egui::Rect::from_center_size(rect.center(), egui::vec2(420.0, 150.0));
    painter.rect_filled(panel, 8.0, theme::PANEL);
    painter.rect_stroke(
        panel,
        8.0,
        egui::Stroke::new(1.0, theme::STROKE),
        egui::StrokeKind::Inside,
    );
    painter.text(
        panel.center_top() + egui::vec2(0.0, 42.0),
        egui::Align2::CENTER_CENTER,
        "Drop a PDF",
        egui::FontId::proportional(20.0),
        theme::TEXT,
    );
    painter.text(
        panel.center_top() + egui::vec2(0.0, 72.0),
        egui::Align2::CENTER_CENTER,
        "Drag a file here, press Ctrl+O, or paste a path in the sidebar.",
        egui::FontId::proportional(13.0),
        theme::TEXT_MUTED,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_zoom_label_rounds_to_whole_percent() {
        assert_eq!(format_zoom_label(1.0), "100%");
        assert_eq!(format_zoom_label(0.333), "33%");
        assert_eq!(format_zoom_label(1.666), "167%");
    }

    #[test]
    fn desired_render_width_rerenders_zoomed_pages_at_display_scale() {
        assert_eq!(desired_render_width(0.5, 1.0), BASE_RENDER_WIDTH);
        assert_eq!(desired_render_width(1.0, 1.0), BASE_RENDER_WIDTH);
        assert_eq!(desired_render_width(2.2, 1.0), 3960);
        assert_eq!(desired_render_width(2.2, 2.0), 7920);
        assert_eq!(desired_render_width(8.0, 2.0), MAX_RENDER_WIDTH);
    }

    #[test]
    fn format_page_counter_uses_one_based_pages() {
        assert_eq!(format_page_counter(0, 12), "Page 1 / 12");
        assert_eq!(format_page_counter(11, 12), "Page 12 / 12");
    }

    #[test]
    fn zoom_around_pointer_keeps_document_point_under_cursor() {
        let viewport =
            egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(1200.0, 900.0));
        let pointer = egui::pos2(900.0, 500.0);
        let old_zoom = 1.0;
        let old_pan = egui::vec2(80.0, -40.0);
        let old_page_center = viewport.center() + old_pan;
        let document_point = (pointer - old_page_center) / old_zoom;

        let (new_zoom, new_pan) = zoom_around_pointer(old_zoom, old_pan, 1.25, pointer, viewport);
        let new_page_center = viewport.center() + new_pan;
        let remapped_pointer = new_page_center + document_point * new_zoom;

        assert_eq!(new_zoom, 1.25);
        assert!((remapped_pointer.x - pointer.x).abs() < 0.01);
        assert!((remapped_pointer.y - pointer.y).abs() < 0.01);
    }

    #[test]
    fn zoom_around_pointer_clamps_without_drifting_anchor() {
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 700.0));
        let pointer = egui::pos2(300.0, 250.0);
        let old_zoom = 7.5;
        let old_pan = egui::vec2(-120.0, 60.0);
        let old_page_center = viewport.center() + old_pan;
        let document_point = (pointer - old_page_center) / old_zoom;

        let (new_zoom, new_pan) = zoom_around_pointer(old_zoom, old_pan, 2.0, pointer, viewport);
        let new_page_center = viewport.center() + new_pan;
        let remapped_pointer = new_page_center + document_point * new_zoom;

        assert_eq!(new_zoom, MAX_ZOOM);
        assert!((remapped_pointer.x - pointer.x).abs() < 0.01);
        assert!((remapped_pointer.y - pointer.y).abs() < 0.01);
    }
}
