use crate::core::project::ProjectState;
use crate::core::sheet::{SheetCandidate, SheetId, generate_bookmark_titles, normalize_sheet_id};
use crate::pdf::{
    LopdfEditEngine, LopdfInspectionEngine, PdfEngine, PdfError, PdfRenderEngine,
    PdfiumRenderEngine, RenderedPage, RenderedTile, TileRequest,
};
use crate::theme;
use eframe::egui;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const BASE_RENDER_WIDTH: u16 = 1800;
const MAX_RENDER_WIDTH: u16 = 8192;
const RERENDER_UPSCALE_THRESHOLD: f32 = 1.15;
const VIEW_RERENDER_IDLE: Duration = Duration::from_millis(320);
const TILE_RENDER_TRIGGER_ZOOM: f32 = 2.0;
const TILE_RENDER_MAX_EDGE: usize = 2048;
const TILE_RENDER_MARGIN: f32 = 0.06;
const MAX_TILE_FULL_WIDTH: usize = 32_768;
const PAGE_CACHE_LIMIT: usize = 7;
const PAGE_PREFETCH_RADIUS: usize = 2;
const MIN_ZOOM: f32 = 0.1;
const MAX_ZOOM: f32 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavigationTab {
    Pages,
    Bookmarks,
}

pub struct GlyphApp {
    project: ProjectState,
    status: String,
    zoom: f32,
    pan: egui::Vec2,
    inspector: LopdfInspectionEngine,
    renderer: PdfiumRenderEngine,
    editor: LopdfEditEngine,
    rendered_page: Option<RenderedPage>,
    page_texture: Option<egui::TextureHandle>,
    page_cache: HashMap<usize, RenderedPage>,
    page_cache_order: VecDeque<usize>,
    rendered_tile: Option<RenderedTile>,
    tile_texture: Option<egui::TextureHandle>,
    page_aspect_ratio: Option<f32>,
    fit_to_page_requested: bool,
    last_canvas_pointer: Option<egui::Pos2>,
    last_view_change: Option<Instant>,
    navigation_tab: NavigationTab,
    sidebar_collapsed: bool,
    render_result_tx: mpsc::Sender<RenderJobResult>,
    render_result_rx: mpsc::Receiver<RenderJobResult>,
    next_render_job_id: u64,
    pending_page_render: Option<PendingPageRender>,
    pending_prefetch_pages: HashSet<usize>,
    pending_tile_render: Option<PendingTileRender>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingPageRender {
    id: u64,
    path: PathBuf,
    page_index: usize,
    target_width: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingTileRender {
    id: u64,
    path: PathBuf,
    request: TileRequest,
}

enum RenderJobResult {
    Page {
        id: u64,
        path: PathBuf,
        page_index: usize,
        target_width: u16,
        result: Result<RenderedPage, PdfError>,
    },
    Tile {
        id: u64,
        path: PathBuf,
        request: TileRequest,
        result: Result<RenderedTile, PdfError>,
    },
    PrefetchPage {
        path: PathBuf,
        page_index: usize,
        target_width: u16,
        result: Result<RenderedPage, PdfError>,
    },
}

impl GlyphApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_pdf: Option<PathBuf>) -> Self {
        theme::install(&cc.egui_ctx);
        let (render_result_tx, render_result_rx) = mpsc::channel();
        let mut app = Self {
            project: ProjectState::new("Untitled Glyph Set"),
            status: "Ready — drop a PDF or press Ctrl+O.".to_owned(),
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
            inspector: LopdfInspectionEngine,
            renderer: PdfiumRenderEngine,
            editor: LopdfEditEngine,
            rendered_page: None,
            page_texture: None,
            page_cache: HashMap::new(),
            page_cache_order: VecDeque::new(),
            rendered_tile: None,
            tile_texture: None,
            page_aspect_ratio: None,
            fit_to_page_requested: false,
            last_canvas_pointer: None,
            last_view_change: None,
            navigation_tab: NavigationTab::Pages,
            sidebar_collapsed: false,
            render_result_tx,
            render_result_rx,
            next_render_job_id: 1,
            pending_page_render: None,
            pending_prefetch_pages: HashSet::new(),
            pending_tile_render: None,
        };
        if let Some(path) = initial_pdf {
            app.open_pdf(path, &cc.egui_ctx);
        }
        app
    }

    fn choose_pdf(&mut self, ctx: &egui::Context) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Choose PDF in Glyph")
            .add_filter("PDF documents", &["pdf"])
            .pick_file()
        {
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
                self.rendered_tile = None;
                self.tile_texture = None;
                self.page_cache.clear();
                self.page_cache_order.clear();
                self.page_aspect_ratio = None;
                self.last_view_change = None;
                self.pending_page_render = None;
                self.pending_prefetch_pages.clear();
                self.pending_tile_render = None;
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
        if let Some(cached) = self.cached_page(page_index, target_width) {
            self.install_texture(ctx, cached);
            self.status = format!("Page {} ready — cached", page_index + 1);
            self.queue_adjacent_page_prefetch(ctx);
            return;
        }
        self.status = format!("Rendering page {}…", page_index + 1);
        self.queue_page_render(ctx, path, page_index, target_width);
    }

    fn queue_page_render(
        &mut self,
        ctx: &egui::Context,
        path: PathBuf,
        page_index: usize,
        target_width: u16,
    ) {
        if self.pending_page_render.as_ref().is_some_and(|pending| {
            pending.path == path
                && pending.page_index == page_index
                && pending.target_width == target_width
        }) {
            return;
        }

        let id = self.next_render_job_id;
        self.next_render_job_id = self.next_render_job_id.wrapping_add(1).max(1);
        self.pending_page_render = Some(PendingPageRender {
            id,
            path: path.clone(),
            page_index,
            target_width,
        });
        let tx = self.render_result_tx.clone();
        let renderer = self.renderer;
        let egui_ctx = ctx.clone();
        thread::spawn(move || {
            let result = renderer.render_page(&path, page_index, target_width);
            let _ = tx.send(RenderJobResult::Page {
                id,
                path,
                page_index,
                target_width,
                result,
            });
            egui_ctx.request_repaint();
        });
    }

    fn cached_page(&self, page_index: usize, target_width: u16) -> Option<RenderedPage> {
        self.page_cache.get(&page_index).and_then(|rendered| {
            (rendered.width as f32 >= target_width as f32 * 0.95).then(|| rendered.clone())
        })
    }

    fn cache_rendered_page(&mut self, rendered: RenderedPage) {
        let page_index = rendered.page_index;
        let should_replace = self
            .page_cache
            .get(&page_index)
            .map(|cached| rendered.width >= cached.width)
            .unwrap_or(true);
        if !should_replace {
            return;
        }
        if !self.page_cache.contains_key(&page_index) {
            self.page_cache_order.push_back(page_index);
        }
        self.page_cache.insert(page_index, rendered);
        while self.page_cache_order.len() > PAGE_CACHE_LIMIT {
            if let Some(evicted) = self.page_cache_order.pop_front() {
                if evicted != self.project.selected_page {
                    self.page_cache.remove(&evicted);
                }
            }
        }
    }

    fn queue_adjacent_page_prefetch(&mut self, ctx: &egui::Context) {
        let Some(document) = &self.project.document else {
            return;
        };
        let page_count = document.summary.page_count;
        let selected_page = self.project.selected_page;
        let start = selected_page.saturating_sub(PAGE_PREFETCH_RADIUS);
        let end = (selected_page + PAGE_PREFETCH_RADIUS).min(page_count.saturating_sub(1));
        let mut targets = Vec::new();
        for page_index in start..=end {
            if page_index == selected_page {
                continue;
            }
            if self.cached_page(page_index, BASE_RENDER_WIDTH).is_some()
                || self.pending_prefetch_pages.contains(&page_index)
                || self
                    .pending_page_render
                    .as_ref()
                    .is_some_and(|pending| pending.page_index == page_index)
            {
                continue;
            }
            targets.push(page_index);
        }
        let path = document.path.clone();
        for page_index in targets {
            self.pending_prefetch_pages.insert(page_index);
            let tx = self.render_result_tx.clone();
            let path = path.clone();
            let renderer = self.renderer;
            let egui_ctx = ctx.clone();
            thread::spawn(move || {
                let result = renderer.render_page(&path, page_index, BASE_RENDER_WIDTH);
                let _ = tx.send(RenderJobResult::PrefetchPage {
                    path,
                    page_index,
                    target_width: BASE_RENDER_WIDTH,
                    result,
                });
                egui_ctx.request_repaint();
            });
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
        self.rendered_tile = None;
        self.tile_texture = None;
    }

    fn apply_render_results(&mut self, ctx: &egui::Context) {
        while let Ok(result) = self.render_result_rx.try_recv() {
            match result {
                RenderJobResult::Page {
                    id,
                    path,
                    page_index,
                    target_width,
                    result,
                } => {
                    let is_current = self.project.document.as_ref().is_some_and(|document| {
                        document.path == path && self.project.selected_page == page_index
                    });
                    let is_latest = self.pending_page_render.as_ref().is_some_and(|pending| {
                        pending.id == id
                            && pending.path == path
                            && pending.page_index == page_index
                            && pending.target_width == target_width
                    });
                    if is_latest {
                        self.pending_page_render = None;
                    }
                    if !is_current || !is_latest {
                        continue;
                    }
                    match result {
                        Ok(rendered) => {
                            self.cache_rendered_page(rendered.clone());
                            self.install_texture(ctx, rendered);
                            self.queue_adjacent_page_prefetch(ctx);
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
                RenderJobResult::Tile {
                    id,
                    path,
                    request,
                    result,
                } => {
                    let is_current = self.project.document.as_ref().is_some_and(|document| {
                        document.path == path && self.project.selected_page == request.page_index
                    });
                    let is_latest = self.pending_tile_render.as_ref().is_some_and(|pending| {
                        pending.id == id && pending.path == path && pending.request == request
                    });
                    if is_latest {
                        self.pending_tile_render = None;
                    }
                    if !is_current || !is_latest {
                        continue;
                    }
                    match result {
                        Ok(tile) => {
                            self.install_tile_texture(ctx, tile);
                            self.status = format!(
                                "Rendered high-res viewport tile — page {} — {}%",
                                self.project.selected_page + 1,
                                (self.zoom * 100.0).round() as i32
                            );
                        }
                        Err(err) => {
                            self.status = format!("High-res tile render failed: {err}");
                        }
                    }
                }
                RenderJobResult::PrefetchPage {
                    path,
                    page_index,
                    target_width,
                    result,
                } => {
                    self.pending_prefetch_pages.remove(&page_index);
                    let is_same_document = self
                        .project
                        .document
                        .as_ref()
                        .is_some_and(|document| document.path == path);
                    if !is_same_document {
                        continue;
                    }
                    if target_width != BASE_RENDER_WIDTH {
                        continue;
                    }
                    if let Ok(rendered) = result {
                        self.cache_rendered_page(rendered.clone());
                        if self.project.selected_page == page_index && self.page_texture.is_none() {
                            self.install_texture(ctx, rendered);
                        }
                    }
                }
            }
        }
    }

    fn install_tile_texture(&mut self, ctx: &egui::Context, tile: RenderedTile) {
        let image = egui::ColorImage::from_rgba_unmultiplied([tile.width, tile.height], &tile.rgba);
        let texture = ctx.load_texture(
            format!(
                "glyph-page-{}-tile-{}x{}-{}-{}-{}x{}",
                tile.page_index,
                tile.full_width,
                tile.full_height,
                tile.x,
                tile.y,
                tile.width,
                tile.height
            ),
            image,
            egui::TextureOptions::LINEAR,
        );
        self.rendered_tile = Some(tile);
        self.tile_texture = Some(texture);
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

    fn mark_view_changed(&mut self) {
        self.last_view_change = Some(Instant::now());
        self.rendered_tile = None;
        self.tile_texture = None;
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
            self.rendered_page = None;
            self.page_texture = None;
            self.page_aspect_ratio = None;
            self.rendered_tile = None;
            self.tile_texture = None;
            self.pending_page_render = None;
            self.pending_tile_render = None;
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
        self.mark_view_changed();
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
        if self.zoom >= TILE_RENDER_TRIGGER_ZOOM {
            return;
        }

        let Some(rendered) = &self.rendered_page else {
            return;
        };
        let desired_width = desired_render_width(self.zoom, ctx.pixels_per_point());
        if desired_width as f32 <= rendered.width as f32 * RERENDER_UPSCALE_THRESHOLD {
            return;
        }

        if let Some(last_view_change) = self.last_view_change {
            let elapsed = last_view_change.elapsed();
            if elapsed < VIEW_RERENDER_IDLE {
                ctx.request_repaint_after(VIEW_RERENDER_IDLE - elapsed);
                return;
            }
        }

        let Some(document) = &self.project.document else {
            return;
        };
        self.queue_page_render(
            ctx,
            document.path.clone(),
            self.project.selected_page,
            desired_width,
        );
        self.last_view_change = None;
    }

    fn ensure_visible_tile(
        &mut self,
        ctx: &egui::Context,
        viewport: egui::Rect,
        page_rect: egui::Rect,
    ) {
        if self.zoom < TILE_RENDER_TRIGGER_ZOOM {
            self.rendered_tile = None;
            self.tile_texture = None;
            return;
        }
        if let Some(last_view_change) = self.last_view_change {
            let elapsed = last_view_change.elapsed();
            if elapsed < VIEW_RERENDER_IDLE {
                ctx.request_repaint_after(VIEW_RERENDER_IDLE - elapsed);
                return;
            }
        }
        let Some(document) = &self.project.document else {
            return;
        };
        let Some(request) = visible_tile_request(
            self.project.selected_page,
            self.page_aspect_ratio,
            self.zoom,
            ctx.pixels_per_point(),
            viewport,
            page_rect,
        ) else {
            return;
        };
        if self
            .rendered_tile
            .as_ref()
            .is_some_and(|tile| tile.contains(&request))
        {
            return;
        }

        if self
            .pending_tile_render
            .as_ref()
            .is_some_and(|pending| pending.path == document.path && pending.request == request)
        {
            return;
        }
        let id = self.next_render_job_id;
        self.next_render_job_id = self.next_render_job_id.wrapping_add(1).max(1);
        self.pending_tile_render = Some(PendingTileRender {
            id,
            path: document.path.clone(),
            request,
        });
        let tx = self.render_result_tx.clone();
        let path = document.path.clone();
        let renderer = self.renderer;
        let egui_ctx = ctx.clone();
        thread::spawn(move || {
            let result = renderer.render_tile(&path, request);
            let _ = tx.send(RenderJobResult::Tile {
                id,
                path,
                request,
                result,
            });
            egui_ctx.request_repaint();
        });
    }

    fn generate_bookmarks_for_current_pdf(&mut self, ctx: &egui::Context) {
        let Some(document) = &self.project.document else {
            self.status = "Load a PDF before generating bookmarks.".to_owned();
            return;
        };
        let candidates = bookmark_candidates_from_pages(&document.summary.pages);
        let bookmarks = generate_bookmark_titles(&candidates, document.summary.page_count);
        let output = sibling_pdf_path(&document.path, "bookmarked");
        match self
            .editor
            .write_bookmarks(&document.path, &output, &bookmarks)
        {
            Ok(report) => {
                let message = format!(
                    "Generated {} bookmarks -> {}",
                    report.bookmarks_written,
                    report.output_path.display()
                );
                self.open_pdf(report.output_path, ctx);
                self.status = message;
                self.navigation_tab = NavigationTab::Bookmarks;
            }
            Err(err) => {
                self.status = format!("Bookmark generator failed: {err}");
            }
        }
    }

    fn generate_hyperlinks_for_current_pdf(&mut self, ctx: &egui::Context) {
        let Some(document) = &self.project.document else {
            self.status = "Load a PDF before generating hyperlinks.".to_owned();
            return;
        };
        let candidates = bookmark_candidates_from_pages(&document.summary.pages);
        let sheets = generate_bookmark_titles(&candidates, document.summary.page_count);
        match self
            .renderer
            .generate_sheet_label_links(&document.path, &sheets)
        {
            Ok(links) if links.is_empty() => {
                self.status = "No sheet-label hyperlinks found in visible PDF text.".to_owned();
            }
            Ok(links) => {
                let output = sibling_pdf_path(&document.path, "hyperlinked");
                match self.editor.write_links(&document.path, &output, &links) {
                    Ok(report) => {
                        let message = format!(
                            "Generated {} hyperlinks -> {}",
                            report.links_written,
                            report.output_path.display()
                        );
                        self.open_pdf(report.output_path, ctx);
                        self.status = message;
                    }
                    Err(err) => {
                        self.status = format!("Hyperlink generator failed: {err}");
                    }
                }
            }
            Err(err) => {
                self.status = format!("Hyperlink generator failed: {err}");
            }
        }
    }

    fn flatten_current_pdf(&mut self, ctx: &egui::Context) {
        let Some(document) = &self.project.document else {
            self.status = "Load a PDF before flattening.".to_owned();
            return;
        };
        let output = sibling_pdf_path(&document.path, "flattened");
        match self
            .editor
            .flatten_interactive_annotations(&document.path, &output)
        {
            Ok(report) => {
                let message = format!(
                    "Flattened {} annotation sets -> {}",
                    report.annotations_removed,
                    report.output_path.display()
                );
                self.open_pdf(report.output_path, ctx);
                self.status = message;
            }
            Err(err) => {
                self.status = format!("Flatten failed: {err}");
            }
        }
    }

    fn reset_view(&mut self) {
        self.zoom = 1.0;
        self.pan = egui::Vec2::ZERO;
        self.last_view_change = Some(Instant::now());
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
        self.apply_render_results(&ctx);
        self.handle_dropped_files(&ctx);
        self.handle_shortcuts(&ctx);

        egui::Panel::top("title_bar")
            .frame(
                egui::Frame::new()
                    .fill(theme::SURFACE)
                    .stroke(egui::Stroke::new(1.0, theme::STROKE_STRONG))
                    .inner_margin(egui::Margin::symmetric(12, 8)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(
                        egui::RichText::new("GLYPH")
                            .monospace()
                            .strong()
                            .size(13.0)
                            .color(theme::ACCENT_STRONG),
                    );
                    ui.separator();
                    ui.label(
                        egui::RichText::new(self.window_title())
                            .monospace()
                            .size(13.0)
                            .color(theme::TEXT),
                    );
                    ui.add_space(12.0);
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

        if self.sidebar_collapsed {
            egui::Panel::left("sheet_sidebar_collapsed")
                .resizable(false)
                .default_size(46.0)
                .size_range(46.0..=46.0)
                .frame(
                    egui::Frame::new()
                        .fill(theme::PANEL)
                        .stroke(egui::Stroke::new(1.0, theme::STROKE_STRONG))
                        .inner_margin(egui::Margin::symmetric(6, 12)),
                )
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        if sidebar_toggle_button(ui, ">", "Show sidebar").clicked() {
                            self.sidebar_collapsed = false;
                        }
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new("NAV")
                                .monospace()
                                .size(10.0)
                                .color(theme::TEXT_FAINT),
                        );
                    });
                });
        } else {
            egui::Panel::left("sheet_sidebar")
                .resizable(true)
                .default_size(328.0)
                .size_range(260.0..=420.0)
                .frame(
                    egui::Frame::new()
                        .fill(theme::PANEL)
                        .stroke(egui::Stroke::new(1.0, theme::STROKE_STRONG))
                        .inner_margin(egui::Margin::symmetric(10, 12)),
                )
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        if sidebar_wide_toggle_button(ui, "<  Hide sidebar", "Hide sidebar")
                            .clicked()
                        {
                            self.sidebar_collapsed = true;
                        }
                        ui.add_space(8.0);

                        if let Some(document) = &self.project.document {
                            let display_name = document.display_name();
                            let page_count = document.summary.page_count;
                            let pages = document.summary.pages.clone();
                            let bookmarks = document.summary.bookmarks.clone();
                            egui::Frame::new()
                                .fill(theme::CARD)
                                .stroke(egui::Stroke::new(1.0, theme::STROKE))
                                .corner_radius(egui::CornerRadius::same(4))
                                .inner_margin(egui::Margin::symmetric(10, 8))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new("PDF")
                                                .monospace()
                                                .strong()
                                                .size(11.0)
                                                .color(theme::ACCENT),
                                        );
                                        ui.label(
                                            egui::RichText::new(display_name)
                                                .monospace()
                                                .size(13.0)
                                                .strong()
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
                                });
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                if ui
                                    .add_enabled(
                                        self.can_go_previous(),
                                        egui::Button::new("← Prev")
                                            .fill(theme::CONTROL)
                                            .corner_radius(4),
                                    )
                                    .clicked()
                                {
                                    self.previous_page(&ctx);
                                }
                                if ui
                                    .add_enabled(
                                        self.can_go_next(),
                                        egui::Button::new("Next →")
                                            .fill(theme::CONTROL)
                                            .corner_radius(4),
                                    )
                                    .clicked()
                                {
                                    self.next_page(&ctx);
                                }
                            });
                            ui.add_space(12.0);

                            ui.horizontal(|ui| {
                                if nav_tab_button(
                                    ui,
                                    "Pages",
                                    self.navigation_tab == NavigationTab::Pages,
                                )
                                .clicked()
                                {
                                    self.navigation_tab = NavigationTab::Pages;
                                }
                                if nav_tab_button(
                                    ui,
                                    "Bookmarks",
                                    self.navigation_tab == NavigationTab::Bookmarks,
                                )
                                .clicked()
                                {
                                    self.navigation_tab = NavigationTab::Bookmarks;
                                }
                            });
                            ui.add_space(8.0);

                            match self.navigation_tab {
                                NavigationTab::Pages => {
                                    egui::ScrollArea::vertical().show(ui, |ui| {
                                        for page in pages {
                                            let is_selected =
                                                self.project.selected_page == page.index;
                                            let title = page.label.as_deref().unwrap_or("Page");
                                            let label = format!("{:>3}   {title}", page.index + 1);
                                            if page_row(ui, &label, is_selected).clicked() {
                                                self.select_page(page.index, &ctx);
                                            }
                                        }
                                    });
                                }
                                NavigationTab::Bookmarks => {
                                    if bookmarks.is_empty() {
                                        empty_sidebar_note(ui, "No bookmarks in this PDF.");
                                    } else {
                                        egui::ScrollArea::vertical().show(ui, |ui| {
                                            for bookmark in bookmarks {
                                                let is_selected =
                                                    bookmark.page_index.is_some_and(|page| {
                                                        page == self.project.selected_page
                                                    });
                                                let response = bookmark_row(
                                                    ui,
                                                    &bookmark.title,
                                                    bookmark.depth,
                                                    bookmark.page_index,
                                                    is_selected,
                                                );
                                                if response.clicked() {
                                                    if let Some(page_index) = bookmark.page_index {
                                                        self.select_page(page_index, &ctx);
                                                    }
                                                }
                                            }
                                        });
                                    }
                                }
                            }
                        } else {
                            egui::Frame::new()
                                .fill(theme::CARD)
                                .stroke(egui::Stroke::new(1.0, theme::STROKE))
                                .corner_radius(egui::CornerRadius::same(4))
                                .inner_margin(egui::Margin::same(14))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new("No PDF loaded")
                                                .size(16.0)
                                                .strong()
                                                .color(theme::TEXT),
                                        );
                                    });
                                    ui.add_space(4.0);
                                    ui.label(
                                        egui::RichText::new("Drop a PDF here or press Ctrl+O.")
                                            .color(theme::TEXT_MUTED),
                                    );
                                });
                        }
                    });
                });
        }

        egui::Panel::bottom("status_bar")
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .stroke(egui::Stroke::new(1.0, theme::STROKE_STRONG))
                    .inner_margin(egui::Margin::symmetric(12, 6)),
            )
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new("▣").color(theme::ACCENT));
                    ui.label(egui::RichText::new(&self.status).color(theme::TEXT_MUTED));
                    ui.separator();
                    ui.label(
                        egui::RichText::new(
                            "Ctrl+O file picker · ←/→ sheets · middle-button drag pans · scroll zoom",
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
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .stroke(egui::Stroke::new(1.0, theme::STROKE))
                    .corner_radius(egui::CornerRadius::same(4))
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if tool_chip(ui, "−").clicked() {
                                self.zoom = (self.zoom * 0.9).max(MIN_ZOOM);
                                self.mark_view_changed();
                                ctx.request_repaint();
                            }
                            metric_pill(ui, &format_zoom_label(self.zoom));
                            if tool_chip(ui, "+").clicked() {
                                self.zoom = (self.zoom * 1.1).min(MAX_ZOOM);
                                self.mark_view_changed();
                                ctx.request_repaint();
                            }
                            ui.add_space(8.0);
                            if tool_chip(ui, "Fit page").clicked() {
                                self.fit_to_page_requested = true;
                            }
                            if tool_chip(ui, "Reset").clicked() {
                                self.reset_view();
                            }
                            ui.add_space(12.0);
                            if tool_chip(ui, "Auto bookmarks").clicked() {
                                self.generate_bookmarks_for_current_pdf(&ctx);
                            }
                            if tool_chip(ui, "Hyperlinks").clicked() {
                                self.generate_hyperlinks_for_current_pdf(&ctx);
                            }
                            if tool_chip(ui, "Flatten").clicked() {
                                self.flatten_current_pdf(&ctx);
                            }
                        });
                    });
                ui.add_space(12.0);

                let available = ui.available_size();
                let (rect, response) = ui.allocate_exact_size(available, egui::Sense::drag());
                if self.fit_to_page_requested {
                    self.fit_page_to_rect(rect);
                    self.fit_to_page_requested = false;
                }
                let pointer_delta = ui.input(|i| i.pointer.delta());
                let middle_pan = response.hovered()
                    && ui.input(|i| i.pointer.button_down(egui::PointerButton::Middle))
                    && pointer_delta != egui::Vec2::ZERO;
                let primary_pan = response.dragged_by(egui::PointerButton::Primary);
                if middle_pan || primary_pan {
                    self.pan += pointer_delta;
                    self.mark_view_changed();
                    ctx.request_repaint();
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
                        self.mark_view_changed();
                        ctx.request_repaint();
                    } else {
                        let scroll_y = ui.input(|i| i.smooth_scroll_delta.y);
                        if scroll_y.abs() > 0.0 {
                            let scale = if scroll_y > 0.0 { 1.08 } else { 0.92 };
                            (self.zoom, self.pan) =
                                zoom_around_pointer(self.zoom, self.pan, scale, pointer, rect);
                            self.mark_view_changed();
                            ctx.request_repaint();
                        }
                    }
                }

                self.ensure_render_quality(ui.ctx());

                let painter = ui.painter_at(rect);
                draw_canvas_backdrop(&painter, rect);

                if self.rendered_page.is_some() && self.page_texture.is_some() {
                    let rendered = self.rendered_page.as_ref().unwrap();
                    let logical_size = self.logical_page_size(rendered);
                    let page_index = rendered.page_index;
                    let page_texture_id = self.page_texture.as_ref().unwrap().id();
                    let page_w = logical_size.x * self.zoom;
                    let page_h = logical_size.y * self.zoom;
                    let page_rect = egui::Rect::from_center_size(
                        rect.center() + self.pan,
                        egui::vec2(page_w, page_h),
                    );
                    painter.rect_filled(
                        page_rect.expand(10.0).translate(egui::vec2(0.0, 5.0)),
                        3.0,
                        egui::Color32::from_black_alpha(78),
                    );
                    self.ensure_visible_tile(ui.ctx(), rect, page_rect);
                    painter.image(
                        page_texture_id,
                        page_rect,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                    if let (Some(tile), Some(tile_texture)) =
                        (&self.rendered_tile, &self.tile_texture)
                    {
                        if tile.page_index == page_index {
                            let tile_rect = tile_screen_rect(tile, page_rect);
                            painter.image(
                                tile_texture.id(),
                                tile_rect,
                                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                                egui::Color32::WHITE,
                            );
                        }
                    }
                    painter.rect_stroke(
                        page_rect,
                        1.0,
                        egui::Stroke::new(1.0, egui::Color32::from_black_alpha(80)),
                        egui::StrokeKind::Inside,
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

fn high_res_full_page_width(zoom: f32, pixels_per_point: f32) -> usize {
    let width = BASE_RENDER_WIDTH as f32 * zoom.max(1.0) * pixels_per_point.max(1.0);
    width
        .round()
        .clamp(MAX_RENDER_WIDTH as f32, MAX_TILE_FULL_WIDTH as f32) as usize
}

fn visible_tile_request(
    page_index: usize,
    page_aspect_ratio: Option<f32>,
    zoom: f32,
    pixels_per_point: f32,
    viewport: egui::Rect,
    page_rect: egui::Rect,
) -> Option<TileRequest> {
    let visible = page_rect.intersect(viewport);
    if visible.width() <= 1.0
        || visible.height() <= 1.0
        || page_rect.width() <= 1.0
        || page_rect.height() <= 1.0
    {
        return None;
    }

    let full_width = high_res_full_page_width(zoom, pixels_per_point);
    let aspect_ratio =
        page_aspect_ratio.unwrap_or_else(|| page_rect.height() / page_rect.width().max(1.0));
    let full_height = ((full_width as f32 * aspect_ratio).round() as usize).max(1);

    let x0 = ((visible.min.x - page_rect.min.x) / page_rect.width()).clamp(0.0, 1.0);
    let y0 = ((visible.min.y - page_rect.min.y) / page_rect.height()).clamp(0.0, 1.0);
    let x1 = ((visible.max.x - page_rect.min.x) / page_rect.width()).clamp(0.0, 1.0);
    let y1 = ((visible.max.y - page_rect.min.y) / page_rect.height()).clamp(0.0, 1.0);

    let mut x = (x0 * full_width as f32).floor() as isize;
    let mut y = (y0 * full_height as f32).floor() as isize;
    let mut right = (x1 * full_width as f32).ceil() as isize;
    let mut bottom = (y1 * full_height as f32).ceil() as isize;
    let margin = (((right - x).max(bottom - y) as f32) * TILE_RENDER_MARGIN).round() as isize;
    let margin = margin.max(96);
    x = (x - margin).max(0);
    y = (y - margin).max(0);
    right = (right + margin).min(full_width as isize);
    bottom = (bottom + margin).min(full_height as isize);

    let width = (right - x).max(1) as usize;
    let height = (bottom - y).max(1) as usize;
    let width = width
        .min(TILE_RENDER_MAX_EDGE)
        .min(full_width.saturating_sub(x as usize).max(1));
    let height = height
        .min(TILE_RENDER_MAX_EDGE)
        .min(full_height.saturating_sub(y as usize).max(1));

    Some(TileRequest {
        page_index,
        full_width,
        full_height,
        x: x as usize,
        y: y as usize,
        width,
        height,
    })
}

fn tile_screen_rect(tile: &RenderedTile, page_rect: egui::Rect) -> egui::Rect {
    let x0 = tile.x as f32 / tile.full_width.max(1) as f32;
    let y0 = tile.y as f32 / tile.full_height.max(1) as f32;
    let x1 = (tile.x + tile.width) as f32 / tile.full_width.max(1) as f32;
    let y1 = (tile.y + tile.height) as f32 / tile.full_height.max(1) as f32;
    egui::Rect::from_min_max(
        egui::pos2(
            page_rect.min.x + x0 * page_rect.width(),
            page_rect.min.y + y0 * page_rect.height(),
        ),
        egui::pos2(
            page_rect.min.x + x1 * page_rect.width(),
            page_rect.min.y + y1 * page_rect.height(),
        ),
    )
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

fn sibling_pdf_path(path: &Path, suffix: &str) -> PathBuf {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("glyph-document");
    path.with_file_name(format!("{stem}.glyph-{suffix}.pdf"))
}

fn bookmark_candidates_from_pages(pages: &[crate::pdf::PdfPageInfo]) -> Vec<SheetCandidate> {
    pages
        .iter()
        .filter_map(|page| {
            let label = page.label.as_deref()?;
            let id = normalize_sheet_id(label)
                .unwrap_or_else(|| SheetId(format!("P{}", page.index + 1)));
            Some(SheetCandidate {
                id,
                title: Some(label.to_owned()),
                page_index: page.index,
                confidence: 50,
            })
        })
        .collect()
}

fn format_page_counter(selected_page: usize, page_count: usize) -> String {
    format!("Page {} / {}", selected_page + 1, page_count)
}

fn draw_canvas_backdrop(painter: &egui::Painter, rect: egui::Rect) {
    painter.rect_filled(rect, 6.0, theme::CANVAS);
    let grid_color = theme::translucent(theme::STROKE_STRONG, 42);
    let step = 36.0;
    let mut x = rect.left() + step;
    while x < rect.right() {
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(1.0, grid_color),
        );
        x += step;
    }
    let mut y = rect.top() + step;
    while y < rect.bottom() {
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            egui::Stroke::new(1.0, grid_color),
        );
        y += step;
    }
    painter.rect_stroke(
        rect,
        6.0,
        egui::Stroke::new(1.0, theme::STROKE_STRONG),
        egui::StrokeKind::Inside,
    );
}

fn tool_chip(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(theme::TEXT).size(12.0))
            .fill(theme::PANEL_RAISED)
            .stroke(egui::Stroke::new(1.0, theme::STROKE))
            .corner_radius(egui::CornerRadius::same(4))
            .min_size(egui::vec2(36.0, 26.0)),
    )
}

fn sidebar_toggle_button(ui: &mut egui::Ui, label: &str, tooltip: &str) -> egui::Response {
    ui.add(
        egui::Button::new(
            egui::RichText::new(label)
                .strong()
                .color(theme::ACCENT)
                .size(14.0),
        )
        .fill(theme::CONTROL)
        .stroke(egui::Stroke::new(1.0, theme::STROKE))
        .corner_radius(egui::CornerRadius::same(4))
        .min_size(egui::vec2(28.0, 26.0)),
    )
    .on_hover_text(tooltip)
}

fn sidebar_wide_toggle_button(ui: &mut egui::Ui, label: &str, tooltip: &str) -> egui::Response {
    ui.add(
        egui::Button::new(
            egui::RichText::new(label)
                .monospace()
                .strong()
                .color(theme::ACCENT),
        )
        .fill(theme::CONTROL)
        .stroke(egui::Stroke::new(1.0, theme::STROKE))
        .corner_radius(egui::CornerRadius::same(4))
        .min_size(egui::vec2(ui.available_width(), 28.0)),
    )
    .on_hover_text(tooltip)
}

fn metric_pill(ui: &mut egui::Ui, label: &str) {
    egui::Frame::new()
        .fill(theme::PANEL_RAISED)
        .stroke(egui::Stroke::new(1.0, theme::STROKE))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(label)
                    .size(12.0)
                    .color(theme::TEXT_MUTED),
            );
        });
}

fn nav_tab_button(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let fill = if selected {
        theme::ACCENT_SOFT
    } else {
        theme::PANEL_RAISED
    };
    let text = if selected {
        theme::ACCENT_STRONG
    } else {
        theme::TEXT_MUTED
    };
    ui.add(
        egui::Button::new(egui::RichText::new(label).strong().color(text).size(12.0))
            .selected(selected)
            .fill(fill)
            .stroke(egui::Stroke::new(
                1.0,
                if selected {
                    theme::ACCENT
                } else {
                    theme::STROKE
                },
            ))
            .corner_radius(egui::CornerRadius::same(4))
            .min_size(egui::vec2((ui.available_width() - 6.0) / 2.0, 30.0)),
    )
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
        theme::ACCENT_SOFT
    } else {
        theme::PANEL_RAISED
    };
    let text = if selected {
        theme::ACCENT_STRONG
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
        .stroke(egui::Stroke::new(
            1.0,
            if selected {
                theme::ACCENT
            } else {
                theme::STROKE
            },
        ))
        .corner_radius(egui::CornerRadius::same(4))
        .min_size(egui::vec2(ui.available_width(), 28.0)),
    )
}

fn bookmark_row(
    ui: &mut egui::Ui,
    title: &str,
    depth: usize,
    page_index: Option<usize>,
    selected: bool,
) -> egui::Response {
    let fill = if selected {
        theme::ACCENT_SOFT
    } else {
        theme::PANEL_RAISED
    };
    let text = if selected {
        theme::ACCENT_STRONG
    } else if page_index.is_some() {
        theme::TEXT
    } else {
        theme::TEXT_MUTED
    };
    let label = format_bookmark_label(title, depth, page_index);
    ui.add_enabled(
        page_index.is_some(),
        egui::Button::new(egui::RichText::new(label).color(text).size(13.0))
            .selected(selected)
            .fill(fill)
            .stroke(egui::Stroke::new(
                1.0,
                if selected {
                    theme::ACCENT
                } else {
                    theme::STROKE
                },
            ))
            .corner_radius(egui::CornerRadius::same(4))
            .min_size(egui::vec2(ui.available_width(), 28.0)),
    )
}

fn format_bookmark_label(title: &str, depth: usize, page_index: Option<usize>) -> String {
    let page = page_index
        .map(|index| format!("  {}", index + 1))
        .unwrap_or_default();
    format!("{}{}{}", "  ".repeat(depth.min(5)), title, page)
}

fn empty_sidebar_note(ui: &mut egui::Ui, note: &str) {
    egui::Frame::new()
        .fill(theme::SURFACE)
        .stroke(egui::Stroke::new(1.0, theme::STROKE))
        .corner_radius(egui::CornerRadius::same(4))
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
    painter.rect_filled(panel, 6.0, theme::PANEL);
    painter.rect_stroke(
        panel,
        6.0,
        egui::Stroke::new(1.0, theme::STROKE),
        egui::StrokeKind::Inside,
    );
    painter.rect_filled(
        egui::Rect::from_min_size(panel.min, egui::vec2(4.0, panel.height())),
        0.0,
        theme::ACCENT,
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
        "Drag a file here or press Ctrl+O.",
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
    fn adjacent_page_prefetch_keeps_small_native_cache() {
        assert_eq!(PAGE_PREFETCH_RADIUS, 2);
        assert_eq!(PAGE_CACHE_LIMIT, 7);
    }

    #[test]
    fn format_bookmark_label_indents_and_uses_one_based_page() {
        assert_eq!(format_bookmark_label("Plan", 0, Some(0)), "Plan  1");
        assert_eq!(
            format_bookmark_label("Detail", 2, Some(11)),
            "    Detail  12"
        );
        assert_eq!(
            format_bookmark_label("Section", 8, None),
            "          Section"
        );
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
