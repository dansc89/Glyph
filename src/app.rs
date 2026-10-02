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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SidebarTab {
    Pages,
    Bookmarks,
    Links,
}

impl SidebarTab {
    fn label(self) -> &'static str {
        match self {
            Self::Pages => "Pages",
            Self::Bookmarks => "Bookmarks",
            Self::Links => "Links",
        }
    }
}

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
    last_canvas_pointer: Option<egui::Pos2>,
    sidebar_tab: SidebarTab,
}

impl GlyphApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_pdf: Option<PathBuf>) -> Self {
        theme::install(&cc.egui_ctx);
        let mut app = Self {
            project: ProjectState::new("Untitled Glyph Set"),
            pdf_path_input: String::new(),
            status: "Ready — open a PDF.".to_owned(),
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
            inspector: LopdfInspectionEngine,
            renderer: PdfiumRenderEngine,
            rendered_page: None,
            page_texture: None,
            fit_to_page_requested: false,
            last_canvas_pointer: None,
            sidebar_tab: SidebarTab::Pages,
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

        egui::Panel::top("title_bar")
            .frame(
                egui::Frame::new()
                    .fill(theme::SURFACE)
                    .stroke(egui::Stroke::new(1.0, theme::STROKE))
                    .inner_margin(egui::Margin::symmetric(14, 9)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    brand_mark(ui);
                    ui.add_space(8.0);
                    status_dot(ui, self.project.document.is_some());
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new("Glyph")
                            .size(16.0)
                            .strong()
                            .color(theme::TEXT),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(self.window_title())
                            .size(12.0)
                            .color(theme::TEXT_MUTED),
                    );
                    ui.add_space(16.0);
                    if toolbar_button(ui, "Open").clicked() {
                        self.choose_pdf(&ctx);
                    }
                    if toolbar_button(ui, "Fit").clicked() {
                        self.fit_to_page_requested = true;
                    }
                    if toolbar_button(ui, "Actual").clicked() {
                        self.reset_view();
                    }
                    if self.project.document.is_some() && toolbar_button(ui, "Reload").clicked() {
                        self.render_selected_page(&ctx);
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
                    sidebar_title(ui);
                    ui.add_space(10.0);

                    egui::Frame::new()
                        .fill(theme::CARD)
                        .stroke(egui::Stroke::new(1.0, theme::STROKE_STRONG))
                        .corner_radius(egui::CornerRadius::same(10))
                        .inner_margin(egui::Margin::same(10))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                if primary_button(ui, "Choose PDF…").clicked() {
                                    self.choose_pdf(&ctx);
                                }
                                if soft_button(ui, "Open path").clicked() {
                                    self.open_pdf_from_input(&ctx);
                                }
                            });
                            ui.add_space(7.0);
                            let response = ui.add(
                                egui::TextEdit::singleline(&mut self.pdf_path_input)
                                    .hint_text("/path/to/drawing-set.pdf")
                                    .desired_width(f32::INFINITY),
                            );
                            if response.lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                            {
                                self.open_pdf_from_input(&ctx);
                            }
                            ui.add_space(5.0);
                            ui.label(
                                egui::RichText::new("Drag PDF here · Ctrl+O · glyph file.pdf")
                                    .size(11.0)
                                    .color(theme::TEXT_MUTED),
                            );
                        });

                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        for tab in [SidebarTab::Pages, SidebarTab::Bookmarks, SidebarTab::Links] {
                            if tab_button(ui, tab, self.sidebar_tab == tab).clicked() {
                                self.sidebar_tab = tab;
                            }
                        }
                    });
                    ui.add_space(10.0);

                    if let Some(document) = &self.project.document {
                        let display_name = document.display_name();
                        let page_count = document.summary.page_count;
                        let pages = document.summary.pages.clone();
                        let bookmarks = document.summary.bookmarks.clone();
                        egui::Frame::new()
                            .fill(theme::CARD)
                            .stroke(egui::Stroke::new(1.0, theme::STROKE))
                            .corner_radius(egui::CornerRadius::same(10))
                            .inner_margin(egui::Margin::same(10))
                            .show(ui, |ui| {
                                accent_bar(ui, theme::ACCENT_STRONG);
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(display_name)
                                            .strong()
                                            .color(theme::TEXT),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                egui::RichText::new(format!("{} pages", page_count))
                                                    .size(11.0)
                                                    .color(theme::TEXT_MUTED),
                                            );
                                        },
                                    );
                                });
                                ui.label(
                                    egui::RichText::new(format_page_counter(
                                        self.project.selected_page,
                                        page_count,
                                    ))
                                    .size(12.0)
                                    .color(theme::TEXT_MUTED),
                                );
                                ui.add_space(7.0);
                                ui.horizontal(|ui| {
                                    if ui
                                        .add_enabled(
                                            self.can_go_previous(),
                                            egui::Button::new("Previous")
                                                .fill(theme::CONTROL)
                                                .corner_radius(6),
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
                                                .corner_radius(6),
                                        )
                                        .clicked()
                                    {
                                        self.next_page(&ctx);
                                    }
                                });
                            });
                        ui.add_space(10.0);

                        match self.sidebar_tab {
                            SidebarTab::Pages => {
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
                            }
                            SidebarTab::Bookmarks => {
                                section_header(ui, "Bookmarks");
                                if bookmarks.is_empty() {
                                    empty_sidebar_note(ui, "This PDF has no outline bookmarks yet.");
                                } else {
                                    egui::ScrollArea::vertical()
                                        .id_salt("bookmark_list")
                                        .show(ui, |ui| {
                                            for bookmark in bookmarks {
                                                let indent = 12.0 * bookmark.depth as f32;
                                                ui.horizontal(|ui| {
                                                    ui.add_space(indent);
                                                    let target = bookmark
                                                        .page_index
                                                        .map(|page_index| format!("p{}", page_index + 1))
                                                        .unwrap_or_else(|| "—".to_owned());
                                                    let label = format!("{}  ·  {target}", bookmark.title);
                                                    if ui
                                                        .add_enabled(
                                                            bookmark.page_index.is_some(),
                                                            egui::Button::new(label)
                                                                .selected(bookmark.page_index == Some(self.project.selected_page))
                                                                .fill(theme::CONTROL)
                                                                .corner_radius(8),
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
                            }
                            SidebarTab::Links => {
                                section_header(ui, "Links");
                                empty_sidebar_note(
                                    ui,
                                    "Link review and creation tools land here next: source rectangle, target page, verify, flatten.",
                                );
                            }
                        }
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
                                    egui::RichText::new("Open a drawing set to populate pages and bookmarks.")
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
                            "Ctrl+O open · ←/→ sheets · Home/End jump · drag pan · scroll zoom",
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
                    }
                    metric_pill(ui, &format_zoom_label(self.zoom));
                    if tool_chip(ui, "+").clicked() {
                        self.zoom = (self.zoom * 1.1).min(MAX_ZOOM);
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
                    } else {
                        let scroll_y = ui.input(|i| i.smooth_scroll_delta.y);
                        if scroll_y.abs() > 0.0 {
                            let scale = if scroll_y > 0.0 { 1.08 } else { 0.92 };
                            (self.zoom, self.pan) =
                                zoom_around_pointer(self.zoom, self.pan, scale, pointer, rect);
                        }
                    }
                }

                let painter = ui.painter_at(rect);
                draw_canvas_backdrop(&painter, rect);

                if let (Some(rendered), Some(texture)) = (&self.rendered_page, &self.page_texture) {
                    let page_w = rendered.width as f32 * self.zoom;
                    let page_h = rendered.height as f32 * self.zoom;
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

fn brand_mark(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
    let painter = ui.painter_at(rect.expand(4.0));
    let center = rect.center();
    painter.circle_filled(
        center,
        12.0,
        egui::Color32::from_rgba_premultiplied(139, 92, 246, 34),
    );
    painter.add(egui::Shape::convex_polygon(
        vec![
            center + egui::vec2(0.0, -10.0),
            center + egui::vec2(9.0, 0.0),
            center + egui::vec2(0.0, 10.0),
            center + egui::vec2(-9.0, 0.0),
        ],
        theme::ACCENT,
        egui::Stroke::new(1.0, theme::ACCENT_STRONG),
    ));
    painter.line_segment(
        [
            center + egui::vec2(-4.0, -2.0),
            center + egui::vec2(4.0, 2.0),
        ],
        egui::Stroke::new(1.2, theme::ACCENT_STRONG),
    );
}

fn sidebar_title(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Workspace")
                .size(11.0)
                .strong()
                .color(theme::ACCENT_STRONG),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new("PDF GRAPH")
                    .size(10.0)
                    .monospace()
                    .color(theme::TEXT_FAINT),
            );
        });
    });
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new("Sheets")
            .size(20.0)
            .strong()
            .color(theme::TEXT),
    );
    ui.label(
        egui::RichText::new("Pages, bookmarks, links")
            .size(12.0)
            .color(theme::TEXT_MUTED),
    );
}

fn accent_bar(ui: &mut egui::Ui, color: egui::Color32) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 3.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 2.0, color);
    ui.add_space(8.0);
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

fn status_dot(ui: &mut egui::Ui, loaded: bool) {
    let color = if loaded {
        theme::GREEN
    } else {
        theme::TEXT_FAINT
    };
    let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

fn primary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(
            egui::RichText::new(label)
                .color(egui::Color32::from_rgb(15, 13, 24))
                .strong(),
        )
        .fill(theme::ACCENT_STRONG)
        .stroke(egui::Stroke::new(1.0, theme::ACCENT))
        .corner_radius(egui::CornerRadius::same(7))
        .min_size(egui::vec2(110.0, 32.0)),
    )
}

fn soft_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(theme::TEXT).size(13.0))
            .fill(theme::CONTROL)
            .stroke(egui::Stroke::new(1.0, theme::STROKE))
            .corner_radius(egui::CornerRadius::same(7))
            .min_size(egui::vec2(88.0, 32.0)),
    )
}

fn tab_button(ui: &mut egui::Ui, tab: SidebarTab, selected: bool) -> egui::Response {
    let (fill, text, stroke) = if selected {
        (
            theme::ACCENT,
            egui::Color32::WHITE,
            egui::Stroke::new(1.0, theme::ACCENT_STRONG),
        )
    } else {
        (
            theme::CONTROL,
            theme::TEXT_MUTED,
            egui::Stroke::new(1.0, theme::STROKE),
        )
    };
    ui.add(
        egui::Button::new(
            egui::RichText::new(tab.label())
                .color(text)
                .size(12.0)
                .strong(),
        )
        .fill(fill)
        .stroke(stroke)
        .corner_radius(egui::CornerRadius::same(7))
        .min_size(egui::vec2(82.0, 28.0)),
    )
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
    let card = egui::Rect::from_center_size(rect.center(), egui::vec2(500.0, 260.0));
    painter.rect_filled(
        card.translate(egui::vec2(0.0, 14.0)).expand(8.0),
        16.0,
        egui::Color32::from_black_alpha(125),
    );
    painter.rect_filled(card, 14.0, theme::CARD);
    painter.rect_stroke(
        card,
        14.0,
        egui::Stroke::new(1.0, theme::STROKE_STRONG),
        egui::StrokeKind::Inside,
    );

    let accent = egui::Rect::from_min_max(
        card.left_top() + egui::vec2(0.0, 18.0),
        card.left_top() + egui::vec2(4.0, card.height() - 18.0),
    );
    painter.rect_filled(accent, 3.0, theme::ACCENT_STRONG);

    let page_icon = egui::Rect::from_min_size(
        card.left_top() + egui::vec2(62.0, 56.0),
        egui::vec2(118.0, 150.0),
    );
    painter.rect_filled(page_icon, 7.0, theme::SURFACE);
    painter.rect_stroke(
        page_icon,
        7.0,
        egui::Stroke::new(1.0, theme::STROKE),
        egui::StrokeKind::Inside,
    );
    painter.rect_filled(
        egui::Rect::from_min_size(
            page_icon.left_top() + egui::vec2(16.0, 20.0),
            egui::vec2(68.0, 8.0),
        ),
        2.0,
        theme::STROKE_STRONG,
    );
    for row in 0..4 {
        let y = page_icon.top() + 50.0 + row as f32 * 20.0;
        painter.line_segment(
            [
                egui::pos2(page_icon.left() + 16.0, y),
                egui::pos2(page_icon.right() - 16.0, y),
            ],
            egui::Stroke::new(1.0, theme::STROKE),
        );
    }

    let badge = egui::Rect::from_center_size(
        card.left_top() + egui::vec2(336.0, 58.0),
        egui::vec2(112.0, 30.0),
    );
    painter.rect_filled(badge, 7.0, theme::PANEL_RAISED);
    painter.rect_stroke(
        badge,
        7.0,
        egui::Stroke::new(1.0, theme::STROKE),
        egui::StrokeKind::Inside,
    );
    painter.text(
        badge.center(),
        egui::Align2::CENTER_CENTER,
        "GLYPH VAULT",
        egui::FontId::monospace(12.0),
        theme::ACCENT_STRONG,
    );
    painter.text(
        card.left_top() + egui::vec2(252.0, 102.0),
        egui::Align2::CENTER_CENTER,
        "Open a PDF drawing set",
        egui::FontId::proportional(24.0),
        theme::TEXT,
    );
    painter.text(
        card.left_top() + egui::vec2(318.0, 135.0),
        egui::Align2::CENTER_CENTER,
        "Drag a PDF here, press Ctrl+O, or paste a path.",
        egui::FontId::proportional(13.0),
        theme::TEXT_MUTED,
    );
    let command = egui::Rect::from_center_size(
        card.left_top() + egui::vec2(318.0, 178.0),
        egui::vec2(270.0, 34.0),
    );
    painter.rect_filled(command, 8.0, theme::SURFACE);
    painter.rect_stroke(
        command,
        8.0,
        egui::Stroke::new(1.0, theme::STROKE),
        egui::StrokeKind::Inside,
    );
    painter.text(
        command.center(),
        egui::Align2::CENTER_CENTER,
        "⌘O  open     /  command palette",
        egui::FontId::monospace(12.0),
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
    fn format_page_counter_uses_one_based_pages() {
        assert_eq!(format_page_counter(0, 12), "Page 1 / 12");
        assert_eq!(format_page_counter(11, 12), "Page 12 / 12");
    }

    #[test]
    fn polish_tab_labels_are_stable_for_sidebar_controls() {
        assert_eq!(SidebarTab::Pages.label(), "Pages");
        assert_eq!(SidebarTab::Bookmarks.label(), "Bookmarks");
        assert_eq!(SidebarTab::Links.label(), "Links");
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
