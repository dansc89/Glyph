mod automation;
#[cfg(test)]
mod compact_tests;
mod editing;
mod icons;
#[cfg(test)]
mod loading_tests;
mod markup;
#[cfg(test)]
mod render_failure_tests;
mod render_worker;
#[cfg(test)]
mod shortcut_tests;
mod text_selection;
mod text_view;
mod thumbnails;
#[cfg(test)]
mod tile_reuse_tests;
use crate::core::navigation::NavigationHistory;
use crate::core::project::ProjectState;
use crate::pdf::PdfInternalLink;
use crate::pdf::overlay::normalize_rectangles;
use crate::pdf::search::{MAX_SEARCH_HITS, PdfSearchHit, search_pdf};
use crate::pdf::{PdfError, RenderedPage, RenderedTile, TileRequest};
use crate::theme;
use automation::{AutomationFeedback, AutomationKind, AutomationOutcome};
use eframe::egui;
use render_worker::{JobKind, RenderJob, RenderWorker};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

const BASE_RENDER_WIDTH: u16 = 1800;
const RENDER_WORKER_FAILURE: &str = "Renderer stopped — PDF and document changes retained. Save or Save As if needed, then restart Glyph to restore previews. Opening PDFs and preview retry are unavailable until restart.";
const MAX_RENDER_WIDTH: u16 = 8192;
const RERENDER_UPSCALE_THRESHOLD: f32 = 1.15;
const VIEW_RERENDER_IDLE: Duration = Duration::from_millis(320);
const TILE_RENDER_TRIGGER_ZOOM: f32 = 2.0;
const TILE_RENDER_MAX_EDGE: usize = 2048;
const TILE_RENDER_MARGIN: f32 = 0.06;
const MAX_TILE_FULL_WIDTH: usize = 32_768;
const PAGE_CACHE_LIMIT: usize = 7;
const PAGE_CACHE_BYTE_BUDGET: usize = 128 * 1024 * 1024;
const PAGE_PREFETCH_RADIUS: usize = 2;
const MIN_ZOOM: f32 = 0.1;
const MAX_ZOOM: f32 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavigationTab {
    Pages,
    Bookmarks,
    Search,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum FitMode {
    #[default]
    Manual,
    Page,
    Width,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ViewLocation {
    page: usize,
    zoom: f32,
    pan: egui::Vec2,
    fit_mode: FitMode,
}

type AutomationReceiver = (
    u64,
    PathBuf,
    mpsc::Receiver<Result<AutomationOutcome, PdfError>>,
);

pub struct GlyphApp {
    project: ProjectState,
    status: String,
    zoom: f32,
    pan: egui::Vec2,
    loading_document: Option<u64>,
    editing: editing::EditingState,
    markup: markup::MarkupState,
    automation_rx: Option<AutomationReceiver>,
    automation_feedback: Option<AutomationFeedback>,
    rendered_page: Option<Arc<RenderedPage>>,
    page_texture: Option<egui::TextureHandle>,
    page_cache: HashMap<usize, Arc<RenderedPage>>,
    page_texture_cache: HashMap<usize, egui::TextureHandle>,
    page_cache_order: VecDeque<usize>,
    rendered_tile: Option<RenderedTile>,
    tile_texture: Option<egui::TextureHandle>,
    page_aspect_ratio: Option<f32>,
    fit_to_page_requested: bool,
    fit_to_width_requested: bool,
    fit_mode: FitMode,
    fitted_viewport: Option<(egui::Vec2, usize)>,
    last_canvas_pointer: Option<egui::Pos2>,
    last_view_change: Option<Instant>,
    navigation_tab: NavigationTab,
    sidebar_collapsed: bool,
    navigation_history: NavigationHistory<ViewLocation>,
    page_links: Vec<PdfInternalLink>,
    link_cache: HashMap<(u64, usize), Vec<PdfInternalLink>>,
    link_cache_order: VecDeque<(u64, usize)>,
    pending_links: Option<(u64, usize)>,
    show_link_highlights: bool,
    search_query: String,
    search_hits: Vec<PdfSearchHit>,
    selected_search_hit: Option<usize>,
    search_focus_requested: bool,
    page_entry: String,
    page_entry_focus_requested: bool,
    search_cancel: Arc<AtomicBool>,
    search_job_id: u64,
    search_running: bool,
    search_progress: (usize, usize),
    search_result_tx: mpsc::Sender<SearchMessage>,
    search_result_rx: mpsc::Receiver<SearchMessage>,
    thumbnails: thumbnails::ThumbnailState,
    page_text: Option<Arc<crate::pdf::PageText>>,
    pending_text: Option<(u64, usize)>,
    text_error: Option<String>,
    selection: text_selection::TextSelection,
    selecting_text: bool,
    render_worker: RenderWorker,
    render_worker_dead: bool,
    document_generation: u64,
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

enum SearchMessage {
    Progress {
        id: u64,
        done: usize,
        total: usize,
    },
    Finished {
        id: u64,
        cancelled: bool,
        result: Result<Vec<PdfSearchHit>, PdfError>,
    },
}

enum RenderJobResult {
    Text {
        id: u64,
        generation: u64,
        path: PathBuf,
        page_index: usize,
        result: Result<crate::pdf::PageText, PdfError>,
    },
    Thumbnail {
        id: u64,
        generation: u64,
        path: PathBuf,
        page_index: usize,
        result: Result<RenderedPage, PdfError>,
    },
    Links {
        id: u64,
        generation: u64,
        path: PathBuf,
        page_index: usize,
        result: Result<Vec<PdfInternalLink>, PdfError>,
    },
    Inspection {
        id: u64,
        path: PathBuf,
        result: Result<crate::pdf::PdfDocumentSummary, PdfError>,
    },
    Page {
        generation: u64,
        id: u64,
        path: PathBuf,
        page_index: usize,
        target_width: u16,
        result: Result<RenderedPage, PdfError>,
    },
    Tile {
        generation: u64,
        id: u64,
        path: PathBuf,
        request: TileRequest,
        result: Result<RenderedTile, PdfError>,
    },
    PrefetchPage {
        generation: u64,
        path: PathBuf,
        page_index: usize,
        target_width: u16,
        result: Result<RenderedPage, PdfError>,
    },
}

impl GlyphApp {
    pub(super) fn render_worker_failed(&self) -> bool {
        self.render_worker_dead
    }

    pub fn new(cc: &eframe::CreationContext<'_>, initial_pdf: Option<PathBuf>) -> Self {
        Self::with_context(&cc.egui_ctx, initial_pdf)
    }

    fn with_context(ctx: &egui::Context, initial_pdf: Option<PathBuf>) -> Self {
        theme::install(ctx);
        let (render_worker, render_result_rx) = RenderWorker::new(ctx);
        let (search_result_tx, search_result_rx) = mpsc::channel();
        let mut app = Self {
            project: ProjectState::new("Untitled Glyph Set"),
            status: "Ready — drop a PDF or press Ctrl+O.".to_owned(),
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
            loading_document: None,
            editing: editing::EditingState::default(),
            markup: markup::MarkupState::default(),
            automation_rx: None,
            automation_feedback: None,
            rendered_page: None,
            page_texture: None,
            page_cache: HashMap::new(),
            page_texture_cache: HashMap::new(),
            page_cache_order: VecDeque::new(),
            rendered_tile: None,
            tile_texture: None,
            page_aspect_ratio: None,
            fit_to_page_requested: false,
            fit_to_width_requested: false,
            fit_mode: FitMode::Manual,
            fitted_viewport: None,
            last_canvas_pointer: None,
            last_view_change: None,
            navigation_tab: NavigationTab::Pages,
            sidebar_collapsed: false,
            navigation_history: NavigationHistory::default(),
            page_links: Vec::new(),
            link_cache: HashMap::new(),
            link_cache_order: VecDeque::new(),
            pending_links: None,
            show_link_highlights: true,
            search_query: String::new(),
            search_hits: Vec::new(),
            selected_search_hit: None,
            search_focus_requested: false,
            page_entry: String::new(),
            page_entry_focus_requested: false,
            search_cancel: Arc::new(AtomicBool::new(false)),
            search_job_id: 0,
            search_running: false,
            search_progress: (0, 0),
            search_result_tx,
            search_result_rx,
            thumbnails: thumbnails::ThumbnailState::default(),
            page_text: None,
            pending_text: None,
            text_error: None,
            selection: text_selection::TextSelection::default(),
            selecting_text: false,
            render_worker,
            render_worker_dead: false,
            document_generation: 0,
            render_result_rx,
            next_render_job_id: 1,
            pending_page_render: None,
            pending_prefetch_pages: HashSet::new(),
            pending_tile_render: None,
        };
        if let Some(path) = initial_pdf {
            app.open_pdf(path, ctx);
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

    fn open_pdf(&mut self, path: PathBuf, _ctx: &egui::Context) {
        if self.render_worker_failed() {
            self.status = RENDER_WORKER_FAILURE.to_owned();
            return;
        }
        if self.defer_document_open(path.clone()) {
            _ctx.request_repaint();
            return;
        }
        let id = self.next_render_job_id;
        self.next_render_job_id = self.next_render_job_id.wrapping_add(1).max(1);
        self.clear_page_text();
        self.loading_document = Some(id);
        self.thumbnails = thumbnails::ThumbnailState::default();
        self.render_worker.set_thumbnails(Vec::new());
        self.fit_to_page_requested = false;
        self.fit_to_width_requested = false;
        self.pending_links = None;
        self.pending_page_render = None;
        self.pending_tile_render = None;
        self.pending_prefetch_pages.clear();
        self.render_worker.submit(RenderJob {
            id,
            generation: self.document_generation,
            path: path.clone(),
            kind: JobKind::Inspect,
        });
        self.status = format!("Loading {}…", path.display());
    }

    fn apply_inspection(
        &mut self,
        id: u64,
        path: PathBuf,
        result: Result<crate::pdf::PdfDocumentSummary, PdfError>,
        ctx: &egui::Context,
    ) {
        if self.loading_document != Some(id) {
            return;
        }
        self.loading_document = None;
        match result {
            Ok(summary) => {
                self.editing = editing::EditingState::default();
                self.markup = markup::MarkupState::default();
                self.cancel_search();
                self.search_hits.clear();
                self.selected_search_hit = None;
                self.navigation_history = NavigationHistory::default();
                self.page_links.clear();
                self.link_cache.clear();
                self.link_cache_order.clear();
                self.pending_links = None;
                self.document_generation = self.document_generation.wrapping_add(1).max(1);
                self.render_worker.reset(self.document_generation);
                self.project.open_document(path.clone(), summary);
                self.zoom = 1.0;
                self.pan = egui::Vec2::ZERO;
                self.fit_to_page_requested = true;
                self.fit_to_width_requested = false;
                self.rendered_page = None;
                self.page_texture = None;
                self.rendered_tile = None;
                self.tile_texture = None;
                self.page_cache.clear();
                self.page_texture_cache.clear();
                self.page_cache_order.clear();
                self.page_aspect_ratio = None;
                self.last_view_change = None;
                self.pending_page_render = None;
                self.pending_prefetch_pages.clear();
                self.pending_tile_render = None;
                self.status = format!("Loaded {}", path.display());
                self.render_selected_page(ctx, BASE_RENDER_WIDTH);
                self.queue_page_links(ctx);
                self.queue_page_text();
            }
            Err(err) => {
                if self.project.document.is_some() {
                    self.render_selected_page(ctx, BASE_RENDER_WIDTH);
                    self.queue_page_links(ctx);
                    self.queue_page_text();
                }
                self.status = format!("Load failed: {err}");
            }
        }
    }

    fn render_selected_page(&mut self, ctx: &egui::Context, target_width: u16) {
        if self.render_worker_failed() {
            return;
        }
        let Some(document) = &self.project.document else {
            return;
        };
        let path = document.path.clone();
        let page_index = self.project.selected_page;
        if let Some(cached) = self.cached_page(page_index, target_width) {
            self.render_worker.set_view(page_index);
            self.pending_links = None;
            self.pending_page_render = None;
            self.pending_tile_render = None;
            self.pending_prefetch_pages.clear();
            self.install_texture(ctx, cached);
            self.status = format!("Page {} ready — cached", page_index + 1);
            self.queue_page_links(ctx);
            self.queue_adjacent_page_prefetch();
            return;
        }
        self.status = format!("Rendering page {}…", page_index + 1);
        self.queue_page_render(path, page_index, target_width);
    }

    fn queue_page_render(&mut self, path: PathBuf, page_index: usize, target_width: u16) {
        if self.render_worker_failed() {
            return;
        }
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
        self.pending_prefetch_pages.clear();
        self.pending_tile_render = None;
        self.render_worker.submit(RenderJob {
            id,
            generation: self.document_generation,
            path,
            kind: JobKind::Page {
                page_index,
                target_width,
            },
        });
    }

    fn cached_page(&self, page_index: usize, target_width: u16) -> Option<Arc<RenderedPage>> {
        self.page_cache.get(&page_index).and_then(|rendered| {
            (rendered.width as f32 >= target_width as f32 * 0.95).then(|| rendered.clone())
        })
    }

    fn cache_rendered_page(&mut self, rendered: Arc<RenderedPage>) {
        let page_index = rendered.page_index;
        let should_replace = self
            .page_cache
            .get(&page_index)
            .map(|cached| rendered.width >= cached.width)
            .unwrap_or(true);
        if !should_replace {
            return;
        }
        self.page_cache_order.retain(|index| *index != page_index);
        self.page_texture_cache.remove(&page_index);
        // Reserve the raster plus an eventual GPU copy. Oversized pages stay only in the active view.
        self.page_cache.remove(&page_index);
        if rendered.rgba.len().saturating_mul(2) > PAGE_CACHE_BYTE_BUDGET {
            return;
        }
        self.page_cache_order.push_back(page_index);
        self.page_cache.insert(page_index, rendered);
        while self.page_cache_order.len() > PAGE_CACHE_LIMIT
            || self
                .page_cache
                .values()
                .map(|page| page.rgba.len().saturating_mul(2))
                .sum::<usize>()
                > PAGE_CACHE_BYTE_BUDGET
        {
            if let Some(evicted) = self.page_cache_order.pop_front() {
                self.page_cache.remove(&evicted);
                self.page_texture_cache.remove(&evicted);
            }
        }
    }

    fn queue_adjacent_page_prefetch(&mut self) {
        if self.render_worker_failed() {
            return;
        }
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
            self.render_worker.submit(RenderJob {
                id: 0,
                generation: self.document_generation,
                path: path.clone(),
                kind: JobKind::Prefetch {
                    page_index,
                    target_width: BASE_RENDER_WIDTH,
                },
            });
        }
    }

    fn install_texture(&mut self, ctx: &egui::Context, rendered: Arc<RenderedPage>) {
        if rendered.width > 0 {
            self.page_aspect_ratio = Some(rendered.height as f32 / rendered.width as f32);
        }
        let texture = if let Some(texture) = self.page_texture_cache.get(&rendered.page_index) {
            texture.clone()
        } else {
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
            if self.page_cache.contains_key(&rendered.page_index) {
                self.page_texture_cache
                    .insert(rendered.page_index, texture.clone());
            }
            texture
        };
        if self.page_cache.contains_key(&rendered.page_index) {
            self.page_cache_order
                .retain(|index| *index != rendered.page_index);
            self.page_cache_order.push_back(rendered.page_index);
        }
        self.rendered_page = Some(rendered);
        self.page_texture = Some(texture);
        self.preview_ready();
        self.rendered_tile = None;
        self.tile_texture = None;
    }

    fn apply_render_results(&mut self, ctx: &egui::Context) {
        loop {
            let result = match self.render_result_rx.try_recv() {
                Ok(result) => result,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.render_worker_dead = true;
                    self.loading_document = None;
                    self.pending_page_render = None;
                    self.pending_tile_render = None;
                    self.pending_prefetch_pages.clear();
                    self.pending_links = None;
                    self.pending_text = None;
                    self.thumbnails = thumbnails::ThumbnailState::default();
                    self.markup.preview_pending = false;
                    self.status = RENDER_WORKER_FAILURE.to_owned();
                    break;
                }
            };
            match result {
                RenderJobResult::Text {
                    id,
                    generation,
                    path,
                    page_index,
                    result,
                } => self.apply_page_text_result(id, generation, path, page_index, result),
                RenderJobResult::Thumbnail {
                    id,
                    generation,
                    path,
                    page_index,
                    result,
                } => self.apply_thumbnail_result(ctx, id, generation, path, page_index, result),
                RenderJobResult::Inspection { id, path, result } => {
                    self.apply_inspection(id, path, result, ctx)
                }
                RenderJobResult::Links {
                    id,
                    generation,
                    path,
                    page_index,
                    result,
                } => {
                    if generation != self.document_generation
                        || self.loading_document.is_some()
                        || !self
                            .project
                            .document
                            .as_ref()
                            .is_some_and(|doc| doc.path == path)
                        || self.project.selected_page != page_index
                        || self.pending_links != Some((id, page_index))
                    {
                        continue;
                    }
                    self.pending_links = None;
                    match result {
                        Ok(links) => {
                            self.cache_links(generation, page_index, links.clone());
                            self.page_links = links;
                        }
                        Err(err) => self.status = format!("Link inspection failed: {err}"),
                    }
                }
                RenderJobResult::Page {
                    generation,
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
                    if generation != self.document_generation || !is_current || !is_latest {
                        continue;
                    }
                    match result {
                        Ok(rendered) => {
                            let rendered = Arc::new(rendered);
                            self.cache_rendered_page(rendered.clone());
                            self.install_texture(ctx, rendered);
                            self.queue_adjacent_page_prefetch();
                            self.status = format!(
                                "Rendered page {} of {} — {}",
                                page_index + 1,
                                self.page_count().unwrap_or(0),
                                display_name(&path)
                            );
                        }
                        Err(err) => {
                            self.preview_failed(err);
                        }
                    }
                }
                RenderJobResult::Tile {
                    generation,
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
                    if generation != self.document_generation || !is_current || !is_latest {
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
                    generation,
                    path,
                    page_index,
                    target_width,
                    result,
                } => {
                    if generation != self.document_generation {
                        continue;
                    }
                    self.pending_prefetch_pages.remove(&page_index);
                    let is_same_document = self
                        .project
                        .document
                        .as_ref()
                        .is_some_and(|document| document.path == path);
                    if generation != self.document_generation || !is_same_document {
                        continue;
                    }
                    if target_width != BASE_RENDER_WIDTH {
                        continue;
                    }
                    if let Ok(rendered) = result {
                        let rendered = Arc::new(rendered);
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
                    "{}{} · {} sheets",
                    document.display_name(),
                    if self.editing.dirty { " *" } else { "" },
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

    fn manual_view_changed(&mut self) {
        self.fit_mode = FitMode::Manual;
        self.fitted_viewport = None;
        self.fit_to_page_requested = false;
        self.fit_to_width_requested = false;
        self.mark_view_changed();
    }

    fn mark_view_changed(&mut self) {
        self.last_view_change = Some(Instant::now());
        self.rendered_tile = None;
        self.tile_texture = None;
    }

    fn select_page(&mut self, page_index: usize, ctx: &egui::Context) {
        if self.render_worker_failed() || self.loading_document.is_some() {
            return;
        }
        if let Some(count) = self.page_count() {
            let current = self.view_location();
            let next = ViewLocation {
                page: page_index.min(count.saturating_sub(1)),
                ..current
            };
            self.navigation_history.visit(current, next);
        }
        self.select_page_without_history(page_index, ctx);
    }

    fn select_page_without_history(&mut self, page_index: usize, ctx: &egui::Context) {
        if self.render_worker_failed() || self.loading_document.is_some() {
            return;
        }
        let Some(page_count) = self.page_count() else {
            return;
        };
        let page_index = page_index.min(page_count.saturating_sub(1));
        if self.project.selected_page != page_index {
            self.cancel_markup_selection();
            self.clear_page_text();
            self.project.selected_page = page_index;
            self.render_worker.set_view(page_index);
            self.pending_links = None;
            self.page_links.clear();
            self.rendered_page = None;
            self.page_texture = None;
            self.page_aspect_ratio = None;
            self.rendered_tile = None;
            self.tile_texture = None;
            self.pending_page_render = None;
            self.pending_tile_render = None;
            self.render_selected_page(ctx, BASE_RENDER_WIDTH);
            self.queue_page_links(ctx);
            self.queue_page_text();
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

    fn fit_width_to_rect(&mut self, rect: egui::Rect) {
        let Some(rendered) = &self.rendered_page else {
            return;
        };
        let logical_size = self.logical_page_size(rendered);
        self.zoom = ((rect.width() - 64.0).max(100.0) / logical_size.x).clamp(MIN_ZOOM, MAX_ZOOM);
        // Top-align tall drawings; center shorter sheets vertically.
        self.pan = egui::vec2(
            0.0,
            ((logical_size.y * self.zoom - rect.height()) / 2.0 + 32.0).max(0.0),
        );
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
        if self.render_worker_failed() || self.loading_document.is_some() {
            return;
        }
        // At high zoom only the visible tile needs high resolution, not the full drawing.
        if self.zoom >= TILE_RENDER_TRIGGER_ZOOM {
            return;
        }
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
        if self.render_worker_failed() || self.loading_document.is_some() {
            return;
        }
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
        if self.rendered_tile.as_ref().is_some_and(|tile| {
            // Padding is for future pans, not a requirement for current coverage.
            // Keep the raster identity guard so zoom/HiDPI changes still rerender.
            tile.contains(&request)
                || (tile.page_index == request.page_index
                    && tile.full_width == request.full_width
                    && tile.full_height == request.full_height
                    && tile_screen_rect(tile, page_rect)
                        .contains_rect(viewport.intersect(page_rect)))
        }) {
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
        self.render_worker.submit(RenderJob {
            id,
            generation: self.document_generation,
            path: document.path.clone(),
            kind: JobKind::Tile(request),
        });
    }

    fn generate_bookmarks_for_current_pdf(&mut self, ctx: &egui::Context) {
        self.start_automation(AutomationKind::Bookmarks, ctx);
    }
    fn generate_hyperlinks_for_current_pdf(&mut self, ctx: &egui::Context) {
        self.start_automation(AutomationKind::Hyperlinks, ctx);
    }
    fn start_automation(&mut self, kind: AutomationKind, ctx: &egui::Context) {
        if self.editing.dirty || self.edit_pending() || self.editing_modal_open() {
            self.status =
                "Save or discard your edits before running drawing-set automation.".into();
            return;
        }
        if self.automation_rx.is_some() || self.loading_document.is_some() {
            return;
        }
        let Some(document) = &self.project.document else {
            return;
        };
        let path = document.path.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        self.automation_feedback = Some(AutomationFeedback {
            kind,
            message: "Reading sheet numbers and drawing references…".into(),
            output: None,
            busy: true,
        });
        self.automation_rx = Some((self.document_generation, path.clone(), rx));
        let repaint = ctx.clone();
        thread::spawn(move || {
            let result = automation::execute(&path, kind);
            let _ = tx.send(result);
            repaint.request_repaint();
        });
    }
    fn apply_automation_results(&mut self, ctx: &egui::Context) {
        let Some((generation, path, rx)) = self.automation_rx.as_ref() else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err(PdfError::Edit(
                "PDF processing worker stopped unexpectedly.".into(),
            )),
        };
        let current = *generation == self.document_generation
            && self.loading_document.is_none()
            && self
                .project
                .document
                .as_ref()
                .is_some_and(|d| d.path == *path);
        let kind = self
            .automation_feedback
            .as_ref()
            .map(|f| f.kind)
            .unwrap_or(AutomationKind::Bookmarks);
        let source = path.display().to_string();
        self.automation_rx = None;
        match result {
            Ok(outcome) => {
                if current && let Some(output) = &outcome.output {
                    self.open_pdf(output.clone(), ctx);
                    if outcome.kind == AutomationKind::Bookmarks {
                        self.navigation_tab = NavigationTab::Bookmarks;
                    }
                }
                let message = if current {
                    self.status = outcome.message.clone();
                    outcome.message
                } else {
                    format!("Result for previous source {source}: {}", outcome.message)
                };
                self.automation_feedback = Some(AutomationFeedback {
                    kind: outcome.kind,
                    message,
                    output: outcome.output,
                    busy: false,
                });
            }
            Err(err) => {
                let message = if current {
                    let message = format!("{} failed: {err}", kind.label());
                    self.status = message.clone();
                    message
                } else {
                    format!(
                        "{} failed for previous source {source}: {err}",
                        kind.label()
                    )
                };
                self.automation_feedback = Some(AutomationFeedback {
                    kind,
                    message,
                    output: None,
                    busy: false,
                });
            }
        }
    }
    fn automation_dialog(&mut self, ctx: &egui::Context) {
        let Some(feedback) = &self.automation_feedback else {
            return;
        };
        let mut dismiss = false;
        egui::Window::new(format!("{} · PDF processing", feedback.kind.label()))
            .id(egui::Id::new("automation_result"))
            .collapsible(false)
            .resizable(false)
            .default_width(440.0)
            .show(ctx, |ui| {
                if feedback.busy {
                    ui.spinner();
                    ctx.request_repaint_after(Duration::from_millis(100));
                }
                ui.label(&feedback.message);
                if let Some(path) = &feedback.output {
                    ui.separator();
                    ui.label("Saved copy:");
                    ui.label(path.display().to_string());
                }
                if !feedback.busy && ui.button("Close").clicked() {
                    dismiss = true;
                }
            });
        if dismiss {
            self.automation_feedback = None;
        }
    }

    fn apply_view_fit(&mut self, rect: egui::Rect) {
        if self.loading_document.is_some() {
            return;
        }
        let Some(page) = self.rendered_page.as_ref() else {
            return;
        };
        let key = (rect.size(), page.page_index);
        if self.fit_to_page_requested {
            self.fit_mode = FitMode::Page;
            self.fitted_viewport = None;
        }
        if self.fit_to_width_requested {
            self.fit_mode = FitMode::Width;
            self.fitted_viewport = None;
        }
        self.fit_to_page_requested = false;
        self.fit_to_width_requested = false;
        if self.fitted_viewport == Some(key) {
            return;
        }
        match self.fit_mode {
            FitMode::Manual => return,
            FitMode::Page => self.fit_page_to_rect(rect),
            FitMode::Width => self.fit_width_to_rect(rect),
        }
        self.fitted_viewport = Some(key);
    }

    fn reset_view(&mut self) {
        self.fit_mode = FitMode::Manual;
        self.fitted_viewport = None;
        self.fit_to_width_requested = false;
        self.fit_to_page_requested = false;
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

    fn draw_page_entry(&mut self, ui: &mut egui::Ui) {
        let Some(count) = self.page_count().filter(|count| *count > 0) else {
            return;
        };
        let id = egui::Id::new("direct-page-entry");
        if !ui.memory(|memory| memory.has_focus(id)) {
            self.page_entry = (self.project.selected_page + 1).to_string();
        }
        ui.label("Page");
        let response = ui
            .add_enabled(
                self.loading_document.is_none(),
                egui::TextEdit::singleline(&mut self.page_entry)
                    .id(id)
                    .desired_width(42.)
                    .char_limit(12),
            )
            .on_hover_text("Enter a page number · Ctrl+G");
        if self.page_entry_focus_requested {
            response.request_focus();
            let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(self.page_entry.chars().count()),
                )));
            state.store(ui.ctx(), id);
            self.page_entry_focus_requested = false;
        }
        if response.lost_focus()
            && ui.input(|input| input.key_pressed(egui::Key::Enter))
            && self.loading_document.is_none()
        {
            match self.page_entry.trim().parse::<usize>() {
                Ok(number) if number > 0 && number <= count => {
                    self.select_page(number - 1, ui.ctx())
                }
                _ => self.status = format!("Enter a page number from 1 to {count}."),
            }
        }
        ui.label(format!("/ {count}"));
    }

    fn search_sidebar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let response = ui.add(
            egui::TextEdit::singleline(&mut self.search_query)
                .hint_text("Find text in PDF · Ctrl+F")
                .desired_width(f32::INFINITY),
        );
        if self.search_focus_requested {
            response.request_focus();
            self.search_focus_requested = false;
        }
        let enter = response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        let mut search = enter;
        ui.horizontal(|ui| {
            if ui.button("Find").clicked() {
                search = true;
            }
            if self.search_running && ui.button("Cancel").clicked() {
                self.cancel_search();
                self.status = "Search cancelled.".to_owned();
            }
        });
        if search {
            self.start_search(ctx);
        }
        if self.search_running {
            let (done, total) = self.search_progress;
            ui.spinner();
            ui.label(format!("Searching page {done} / {total}"));
        } else {
            ui.label(format!("{} matches", self.search_hits.len()));
            ui.small("Selectable text only; scanned pages need OCR.");
        }
        let mut selected = None;
        if !self.search_hits.is_empty() {
            ui.horizontal(|ui| {
                let current = self.selected_search_hit.unwrap_or(0);
                if ui.button("Previous hit").clicked() {
                    selected =
                        Some((current + self.search_hits.len() - 1) % self.search_hits.len());
                }
                if ui.button("Next hit").clicked() {
                    selected = Some(
                        self.selected_search_hit
                            .map_or(0, |index| (index + 1) % self.search_hits.len()),
                    );
                }
            });
        }
        egui::ScrollArea::vertical()
            .id_salt("search_results")
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("search-hits")
                    .show_rows(ui, 28.0, self.search_hits.len(), |ui, rows| {
                        for index in rows {
                            let hit = &self.search_hits[index];
                            if page_row(
                                ui,
                                &format!(
                                    "{} · Page {}  {}",
                                    index + 1,
                                    hit.page_index + 1,
                                    hit.text
                                ),
                                self.selected_search_hit == Some(index),
                            )
                            .clicked()
                            {
                                selected = Some(index);
                            }
                        }
                    });
            });
        if let Some(index) = selected {
            self.selected_search_hit = Some(index);
            self.select_page(self.search_hits[index].page_index, ctx);
            self.fit_to_page_requested = true;
            self.fit_to_width_requested = false;
        }
    }

    fn cancel_search(&mut self) {
        self.search_cancel.store(true, Ordering::Relaxed);
        self.search_job_id = self.search_job_id.wrapping_add(1);
        self.search_running = false;
    }

    fn start_search(&mut self, ctx: &egui::Context) {
        self.cancel_search();
        self.search_hits.clear();
        self.selected_search_hit = None;
        let Some(document) = &self.project.document else {
            return;
        };
        let query = self.search_query.trim().to_owned();
        if query.is_empty() {
            self.status = "Enter text to search.".to_owned();
            return;
        }
        let path = document.path.clone();
        let id = self.search_job_id;
        let cancel = Arc::new(AtomicBool::new(false));
        self.search_cancel = cancel.clone();
        self.search_running = true;
        self.search_progress = (0, document.summary.page_count);
        let tx = self.search_result_tx.clone();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let result = search_pdf(&path, &query, &cancel, |done, total| {
                let _ = tx.send(SearchMessage::Progress { id, done, total });
                ctx.request_repaint();
            })
            .and_then(|mut hits| {
                let mut rects: Vec<_> = hits
                    .iter()
                    .flat_map(|hit| hit.rects.iter().map(move |rect| (hit.page_index, *rect)))
                    .collect();
                if !cancel.load(Ordering::Relaxed) {
                    normalize_rectangles(&path, &mut rects)?;
                }
                let mut mapped = rects.into_iter();
                for hit in &mut hits {
                    for rect in &mut hit.rects {
                        *rect = mapped.next().unwrap().1;
                    }
                }
                Ok(hits)
            });
            let _ = tx.send(SearchMessage::Finished {
                id,
                cancelled: cancel.load(Ordering::Relaxed),
                result,
            });
            ctx.request_repaint();
        });
    }

    fn cache_links(&mut self, generation: u64, page_index: usize, links: Vec<PdfInternalLink>) {
        let key = (generation, page_index);
        self.link_cache_order.retain(|entry| *entry != key);
        self.link_cache_order.push_back(key);
        self.link_cache.insert(key, links);
        while self.link_cache_order.len() > PAGE_CACHE_LIMIT {
            if let Some(key) = self.link_cache_order.pop_front() {
                self.link_cache.remove(&key);
            }
        }
    }

    fn queue_page_links(&mut self, _ctx: &egui::Context) {
        if self.render_worker_failed() {
            return;
        }
        let Some(document) = &self.project.document else {
            return;
        };
        let path = document.path.clone();
        let page_index = self.project.selected_page;
        if let Some(links) = self.link_cache.get(&(self.document_generation, page_index)) {
            self.page_links = links.clone();
            return;
        }
        if self
            .pending_links
            .is_some_and(|(_, page)| page == page_index)
        {
            return;
        }
        let id = self.next_render_job_id;
        self.next_render_job_id = self.next_render_job_id.wrapping_add(1).max(1);
        self.pending_links = Some((id, page_index));
        self.render_worker.submit(RenderJob {
            id,
            generation: self.document_generation,
            path,
            kind: JobKind::Links { page_index },
        });
    }

    fn apply_search_results(&mut self) {
        while let Ok(message) = self.search_result_rx.try_recv() {
            match message {
                SearchMessage::Progress { id, done, total } if id == self.search_job_id => {
                    self.search_progress = (done, total)
                }
                SearchMessage::Finished {
                    id,
                    cancelled,
                    result,
                } if id == self.search_job_id => {
                    self.search_running = false;
                    if cancelled {
                        self.status = "Search cancelled.".to_owned();
                        continue;
                    }
                    match result {
                        Ok(hits) => {
                            self.status = if hits.is_empty() {
                                "No matches found in selectable PDF text.".to_owned()
                            } else if hits.len() == MAX_SEARCH_HITS {
                                format!(
                                    "Showing first {MAX_SEARCH_HITS} matches — refine your search."
                                )
                            } else {
                                format!("{} search matches", hits.len())
                            };
                            self.search_hits = hits;
                        }
                        Err(err) => self.status = format!("Search failed: {err}"),
                    }
                }
                _ => {}
            }
        }
    }

    fn view_location(&self) -> ViewLocation {
        ViewLocation {
            page: self.project.selected_page,
            zoom: self.zoom,
            pan: self.pan,
            fit_mode: self.fit_mode,
        }
    }

    fn restore_view(&mut self, view: ViewLocation, ctx: &egui::Context) {
        self.select_page_without_history(view.page, ctx);
        self.zoom = view.zoom;
        self.pan = view.pan;
        self.fit_mode = view.fit_mode;
        self.fitted_viewport = None;
        self.fit_to_page_requested = false;
        self.fit_to_width_requested = false;
        self.mark_view_changed();
        ctx.request_repaint();
    }

    fn go_back(&mut self, ctx: &egui::Context) {
        if self.render_worker_failed() || self.loading_document.is_some() {
            return;
        }
        if let Some(view) = self.navigation_history.back(self.view_location()) {
            self.restore_view(view, ctx);
        }
    }
    fn go_forward(&mut self, ctx: &egui::Context) {
        if self.render_worker_failed() || self.loading_document.is_some() {
            return;
        }
        if let Some(view) = self.navigation_history.forward(self.view_location()) {
            self.restore_view(view, ctx);
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if self.editing_modal_open() {
            return;
        }
        if self.handle_edit_shortcuts(ctx) {
            return;
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
            self.choose_pdf(ctx);
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
            self.navigation_tab = NavigationTab::Search;
            self.sidebar_collapsed = false;
            self.search_focus_requested = true;
        }
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) && self.search_running {
            self.cancel_search();
            self.status = "Search cancelled.".to_owned();
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Num1))
            && self.loading_document.is_none()
        {
            self.fit_to_page_requested = true;
            self.fit_to_width_requested = false;
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Num2))
            && self.loading_document.is_none()
        {
            self.fit_to_width_requested = true;
            self.fit_to_page_requested = false;
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Num0)) {
            self.fit_to_width_requested = false;
            self.fit_to_page_requested = false;
            self.reset_view();
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::G))
            && self.page_count().is_some_and(|count| count > 0)
            && self.loading_document.is_none()
        {
            self.page_entry_focus_requested = true;
        }
        if self.search_focus_requested
            || self.page_entry_focus_requested
            || ctx.egui_wants_keyboard_input()
        {
            return;
        }
        self.markup_shortcuts(ctx);
        let copy = ctx.input_mut(|input| {
            let native_copy = input.events.iter().any(|e| matches!(e, egui::Event::Copy));
            if native_copy {
                input.events.retain(|e| !matches!(e, egui::Event::Copy));
            }
            native_copy || input.consume_key(egui::Modifiers::COMMAND, egui::Key::C)
        });
        if copy {
            self.copy_pdf_selection(ctx);
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::ALT, egui::Key::ArrowLeft)) {
            self.go_back(ctx);
            return;
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::ALT, egui::Key::ArrowRight)) {
            self.go_forward(ctx);
            return;
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
        if ctx.input(|input| input.key_pressed(egui::Key::End))
            && let Some(page_count) = self.page_count()
        {
            self.select_page(page_count.saturating_sub(1), ctx);
        }
    }
}

impl GlyphApp {
    fn draw_missing_page(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        let opening = self.loading_document.is_some();
        let failed = self.render_worker_failed();
        if !failed && !opening && self.project.document.is_none() {
            draw_empty_state(ui, rect);
            return;
        }
        let busy =
            !failed && (opening || self.pending_page_render.is_some() || self.edit_pending());
        let title = if failed {
            "Renderer stopped".to_owned()
        } else if opening {
            "Opening PDF…".to_owned()
        } else if busy {
            format!(
                "Loading page {} of {}…",
                self.project.selected_page + 1,
                self.page_count().unwrap_or(0)
            )
        } else {
            "Page preview unavailable".to_owned()
        };
        let panel = egui::Rect::from_center_size(
            rect.center(),
            egui::vec2(rect.width().min(420.0), rect.height().min(150.0)),
        );
        let mut content = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("missing_page")
                .max_rect(panel)
                .layout(egui::Layout::top_down(egui::Align::Center)),
        );
        content.set_clip_rect(rect.intersect(ui.clip_rect()));
        let painter = content.painter();
        painter.rect_filled(panel, 6.0, theme::color(theme::PANEL));
        content.add_space(18.0);
        if busy {
            let (spinner, _) =
                content.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
            // A bounded animation cadence; no context access inside another context lock.
            let phase = content.input(|input| input.time) as f32 * 4.0;
            let points = (0..=24)
                .map(|step| {
                    let angle = phase + step as f32 / 24.0 * std::f32::consts::PI * 1.5;
                    spinner.center() + egui::vec2(angle.cos(), angle.sin()) * 10.0
                })
                .collect();
            content.painter().add(egui::Shape::line(
                points,
                egui::Stroke::new(2.5, theme::color(theme::ACCENT)),
            ));
            content
                .ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        }
        content.add_space(8.0);
        content.label(
            egui::RichText::new(title)
                .size(20.0)
                .color(theme::color(theme::TEXT)),
        );
        let detail = if failed {
            "Restart Glyph to restore previews. Save retained changes first if needed."
        } else if opening {
            "Reading your PDF. Preparing the first page."
        } else if busy {
            "Your PDF is still open. Preparing this page."
        } else if self.preview_unavailable() {
            "Your PDF is still open. See the error below for recovery options."
        } else {
            "Your PDF is still open. Retry the page preview."
        };
        content.label(egui::RichText::new(detail).color(theme::color(theme::TEXT_MUTED)));
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        theme::refresh(&ctx);
        self.apply_edit_results(&ctx);
        self.guard_window_close(&ctx);
        self.apply_render_results(&ctx);
        self.apply_search_results();
        self.apply_automation_results(&ctx);
        self.automation_dialog(&ctx);
        self.handle_dropped_files(&ctx);
        self.handle_shortcuts(&ctx);
        self.editing_dialogs(&ctx);

        egui::Panel::top("title_bar")
            .frame(
                egui::Frame::new()
                    .fill(theme::color(theme::SURFACE))
                    .stroke(egui::Stroke::new(1.0, theme::color(theme::STROKE_STRONG)))
                    .inner_margin(egui::Margin::symmetric(8, 3)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(
                        egui::RichText::new("GLYPH")
                            .monospace()
                            .strong()
                            .size(13.0)
                            .color(theme::color(theme::ACCENT_STRONG)),
                    );
                    ui.separator();
                    self.document_menu(ui, &ctx);
                    ui.separator();
                    let title = self.window_title();
                    let title_width = (ui.available_width() - 180.).max(32.);
                    ui.add_sized(
                        [title_width, 18.],
                        egui::Label::new(
                            egui::RichText::new(&title)
                                .monospace()
                                .size(12.)
                                .color(theme::color(theme::TEXT)),
                        )
                        .truncate(),
                    )
                    .on_hover_text(title);
                    ui.add_space(4.0);
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
                .default_size(34.0)
                .size_range(34.0..=34.0)
                .frame(
                    egui::Frame::new()
                        .fill(theme::color(theme::PANEL))
                        .stroke(egui::Stroke::new(1.0, theme::color(theme::STROKE_STRONG)))
                        .inner_margin(egui::Margin::symmetric(4, 4)),
                )
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        if icons::button(ui, icons::Icon::Sidebar, true, false)
                            .on_hover_text("Show sidebar")
                            .clicked()
                        {
                            self.sidebar_collapsed = false;
                        }
                    });
                });
        } else {
            egui::Panel::left("sheet_sidebar")
                .resizable(true)
                .default_size(220.0)
                .size_range(180.0..=280.0)
                .frame(
                    egui::Frame::new()
                        .fill(theme::color(theme::PANEL))
                        .stroke(egui::Stroke::new(1.0, theme::color(theme::STROKE_STRONG)))
                        .inner_margin(egui::Margin::symmetric(6, 4)),
                )
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        if icons::button(ui, icons::Icon::Sidebar, true, true)
                            .on_hover_text("Hide sidebar")
                            .clicked()
                        {
                            self.sidebar_collapsed = true;
                        }
                        ui.add_space(2.0);

                        if let Some(document) = &self.project.document {
                            let display_name = document.display_name();
                            let page_count = document.summary.page_count;
                            let bookmark_rows = document.summary.bookmarks.len();
                            egui::Frame::new()
                                .fill(theme::color(theme::CARD))
                                .stroke(egui::Stroke::new(1.0, theme::color(theme::STROKE)))
                                .corner_radius(egui::CornerRadius::same(4))
                                .inner_margin(egui::Margin::symmetric(6, 4))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new("PDF")
                                                .monospace()
                                                .strong()
                                                .size(11.0)
                                                .color(theme::color(theme::ACCENT)),
                                        );
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(&display_name)
                                                    .monospace()
                                                    .size(12.0)
                                                    .strong()
                                                    .color(theme::color(theme::TEXT)),
                                            )
                                            .truncate(),
                                        )
                                        .on_hover_text(document.path.display().to_string());
                                    });
                                    ui.label(
                                        egui::RichText::new(format_page_counter(
                                            self.project.selected_page,
                                            page_count,
                                        ))
                                        .size(12.0)
                                        .color(theme::color(theme::TEXT_MUTED)),
                                    );
                                });
                            ui.add_space(2.0);
                            ui.horizontal(|ui| {
                                if icons::button(
                                    ui,
                                    icons::Icon::ArrowLeft,
                                    self.can_go_previous(),
                                    false,
                                )
                                .on_hover_text("Previous page · Left")
                                .clicked()
                                {
                                    self.previous_page(&ctx);
                                }
                                if icons::button(
                                    ui,
                                    icons::Icon::ArrowRight,
                                    self.can_go_next(),
                                    false,
                                )
                                .on_hover_text("Next page · Right")
                                .clicked()
                                {
                                    self.next_page(&ctx);
                                }
                                ui.separator();
                                for (tab, icon) in [
                                    (NavigationTab::Pages, icons::Icon::Pages),
                                    (NavigationTab::Bookmarks, icons::Icon::Bookmarks),
                                    (NavigationTab::Search, icons::Icon::Search),
                                ] {
                                    if icons::button(ui, icon, true, self.navigation_tab == tab)
                                        .clicked()
                                    {
                                        self.navigation_tab = tab;
                                        if tab == NavigationTab::Search {
                                            self.search_focus_requested = true;
                                        }
                                    }
                                }
                            });
                            ui.add_space(2.0);

                            match self.navigation_tab {
                                NavigationTab::Search => self.search_sidebar(ui, &ctx),
                                NavigationTab::Pages => {
                                    if self.render_worker_failed() {
                                        empty_sidebar_note(ui, "Renderer stopped. Restart Glyph to restore page previews.");
                                    } else {
                                        self.thumbnail_sidebar(ui);
                                    }
                                }
                                NavigationTab::Bookmarks => {
                                    if bookmark_rows == 0 {
                                        empty_sidebar_note(ui, "No bookmarks in this PDF.");
                                    } else {
                                        egui::ScrollArea::vertical()
                                            .id_salt("bookmarks")
                                            .show_rows(ui, 28.0, bookmark_rows, |ui, rows| {
                                                for index in rows {
                                                    let bookmark = self
                                                        .project
                                                        .document
                                                        .as_ref()
                                                        .unwrap()
                                                        .summary
                                                        .bookmarks[index]
                                                        .clone();
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
                                                    self.bookmark_edit_menu(&response, index);
                                                    if response.clicked()
                                                        && let Some(page_index) =
                                                            bookmark.page_index
                                                    {
                                                        self.select_page(page_index, &ctx);
                                                    }
                                                }
                                            });
                                    }
                                }
                            }
                        } else {
                            egui::Frame::new()
                                .fill(theme::color(theme::CARD))
                                .stroke(egui::Stroke::new(1.0, theme::color(theme::STROKE)))
                                .corner_radius(egui::CornerRadius::same(4))
                                .inner_margin(egui::Margin::same(6))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new("No PDF loaded")
                                                .size(12.0)
                                                .strong()
                                                .color(theme::color(theme::TEXT)),
                                        );
                                    });
                                    ui.add_space(4.0);
                                    ui.label(
                                        egui::RichText::new("Drop a PDF here or press Ctrl+O.")
                                            .color(theme::color(theme::TEXT_MUTED)),
                                    );
                                });
                        }
                    });
                });
        }

        egui::Panel::bottom("status_bar")
            .frame(
                egui::Frame::new()
                    .fill(theme::color(theme::PANEL))
                    .stroke(egui::Stroke::new(1.0, theme::color(theme::STROKE_STRONG)))
                    .inner_margin(egui::Margin::symmetric(8, 1)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let error = self
                        .persistent_edit_error()
                        .or_else(|| self.render_worker_failed().then_some(RENDER_WORKER_FAILURE));
                    let progress = self.editing_progress_status(Instant::now());
                    let status = error.or(progress.as_deref()).unwrap_or(&self.status);
                    let width = (ui.available_width() - 28.).max(0.);
                    ui.add_sized(
                        [width, 24.],
                        egui::Label::new(egui::RichText::new(status).color(if error.is_some() {
                            egui::Color32::LIGHT_RED
                        } else {
                            theme::color(theme::TEXT_MUTED)
                        }))
                        .truncate(),
                    )
                    .on_hover_text(status);
                    let help = icons::button(ui, icons::Icon::Help, true, false);
                    egui::Popup::menu(&help).show(|ui| {
                        ui.label("Keyboard & canvas");
                        ui.separator();
                        ui.label("Ctrl+O open · Ctrl+S save · Ctrl+Shift+S save copy");
                        ui.label("Ctrl+F search · Ctrl+G page");
                        ui.label("Ctrl+1 fit page · Ctrl+2 fit width");
                        ui.label("Alt+Left/Right history · middle drag pans · scroll zoom");
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::color(theme::CANVAS))
                    .inner_margin(egui::Margin::same(6)),
            )
            .show(ui, |ui| {
                egui::Frame::new()
                    .fill(theme::color(theme::PANEL))
                    .stroke(egui::Stroke::new(1.0, theme::color(theme::STROKE)))
                    .corner_radius(egui::CornerRadius::same(4))
                    .inner_margin(egui::Margin::symmetric(4, 3))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let ready = self.project.document.is_some() && self.loading_document.is_none();
                            let back_enabled = ready && self.navigation_history.can_back();
                            let back = icons::button_named(ui, icons::Icon::ArrowLeft, back_enabled, false, "Back", "Back · Alt+Left");
                            if back.clicked() { self.go_back(&ctx); }
                            let forward_enabled = ready && self.navigation_history.can_forward();
                            let forward = icons::button_named(ui, icons::Icon::ArrowRight, forward_enabled, false, "Forward", "Forward · Alt+Right");
                            if forward.clicked() { self.go_forward(&ctx); }
                            ui.separator();
                            if icons::button(ui, icons::Icon::ZoomOut, ready, false).clicked() {
                                self.zoom = (self.zoom * 0.9).max(MIN_ZOOM);
                                self.manual_view_changed();
                                ctx.request_repaint();
                            }
                            ui.add_sized([42., 24.], egui::Label::new(format_zoom_label(self.zoom)).truncate());
                            if icons::button(ui, icons::Icon::ZoomIn, ready, false).clicked() {
                                self.zoom = (self.zoom * 1.1).min(MAX_ZOOM);
                                self.manual_view_changed();
                                ctx.request_repaint();
                            }
                            if icons::button(ui, icons::Icon::FitPage, ready, self.fit_mode == FitMode::Page).on_hover_text("Fit page · Ctrl+1").clicked() {
                                self.fit_to_page_requested = true;
                                self.fit_to_width_requested = false;
                            }
                            if icons::button(ui, icons::Icon::FitWidth, ready, self.fit_mode == FitMode::Width).on_hover_text("Fit width · Ctrl+2").clicked() {
                                self.fit_to_width_requested = true;
                                self.fit_to_page_requested = false;
                            }
                            if icons::button(ui, icons::Icon::Reset, ready, false).clicked() { self.reset_view(); }
                            ui.separator();
                            self.draw_page_entry(ui);
                            ui.separator();
                            self.draw_markup_tools(ui, &ctx);
                            let more = icons::button(ui, icons::Icon::More, true, false);
                            egui::Popup::menu(&more).show(|ui| {
                                let automation_ready = ready && self.automation_rx.is_none()
                                    && !self.edit_pending() && !self.editing_modal_open() && !self.editing.dirty;
                                if ui.add_enabled(automation_ready, egui::Button::new("Auto bookmarks"))
                                    .on_hover_text("Saves a new copy and replaces its existing bookmark hierarchy with detected sheet numbers. The original PDF is unchanged.")
                                    .on_disabled_hover_text("Load a PDF and save or discard edits before running automation.").clicked() {
                                    self.generate_bookmarks_for_current_pdf(&ctx);
                                    ui.close();
                                }
                                if ui.add_enabled(automation_ready, egui::Button::new("Hyperlinks")).on_hover_text("Creates a new linked copy; the original PDF is unchanged.").clicked() {
                                    self.generate_hyperlinks_for_current_pdf(&ctx);
                                    ui.close();
                                }
                                ui.add_enabled(false, egui::Button::new("Flatten"))
                                    .on_disabled_hover_text("Temporarily disabled: appearance-preserving, reversible flattening is not implemented yet.");
                                ui.add_enabled(ready, egui::Checkbox::new(&mut self.show_link_highlights, "Show links"));
                            });
                        });
                    });
                ui.add_space(4.0);
                if let Some(error)=&self.text_error {ui.label(egui::RichText::new("Text selection unavailable").color(theme::color(theme::TEXT_MUTED))).on_hover_text(error);}

                let available = ui.available_size();
                let (rect, response) = ui.allocate_exact_size(available, egui::Sense::click_and_drag());
                self.apply_view_fit(rect);
                if self.markup.mode==markup::Mode::View{self.interact_with_page_text(ui,&response,rect);}
                let pointer_delta = ui.input(|i| i.pointer.delta());
                let middle_pan = response.hovered()
                    && ui.input(|i| i.pointer.button_down(egui::PointerButton::Middle))
                    && pointer_delta != egui::Vec2::ZERO;
                let primary_pan = response.dragged_by(egui::PointerButton::Primary) && !self.selecting_text && self.markup.mode==markup::Mode::View;
                if middle_pan || primary_pan {
                    self.pan += pointer_delta;
                    self.manual_view_changed();
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
                        self.manual_view_changed();
                        ctx.request_repaint();
                    } else {
                        let scroll_y = ui.input(|i| i.smooth_scroll_delta.y);
                        if scroll_y.abs() > 0.0 {
                            // Integrate scroll distance, not the number of smoothing frames.
                            let scale = (scroll_y * 0.002).exp();
                            (self.zoom, self.pan) =
                                zoom_around_pointer(self.zoom, self.pan, scale, pointer, rect);
                            self.manual_view_changed();
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
                    if let (Some(tile), Some(tile_texture)) = (&self.rendered_tile, &self.tile_texture) && tile.page_index == page_index {
                        let tile_rect = tile_screen_rect(tile, page_rect);
                        painter.image(tile_texture.id(),tile_rect,egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),egui::Color32::WHITE);
                    }
                    self.interact_with_markup(ui,&response,page_rect,rect,page_index);
                    self.paint_markup(&painter,page_rect,page_index);
                    self.paint_text_selection(&painter,page_rect,rect);
                    let mut link_target = None;
                    for link in &self.page_links {
                        let link_rect = overlay_screen_rect(link.rect, page_rect);
                        let hovered = self.markup.mode==markup::Mode::View && response.hover_pos().is_some_and(|pos| link_rect.contains(pos));
                        if self.show_link_highlights || hovered {
                            painter.rect_filled(link_rect, 0.0, theme::translucent(theme::color(theme::ACCENT), if hovered { 70 } else { 30 }));
                            painter.rect_stroke(link_rect, 0.0, egui::Stroke::new(1.0, theme::color(theme::ACCENT)), egui::StrokeKind::Inside);
                        }
                        if hovered {
                            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                            if response.clicked() { link_target = Some(link.target_page); }
                        }
                    }
                    for (index, hit) in self.search_hits.iter().enumerate().filter(|(_, hit)| hit.page_index == page_index) {
                        for bounds in &hit.rects {
                            let hit_rect = overlay_screen_rect(*bounds, page_rect);
                            painter.rect_filled(hit_rect, 0.0, egui::Color32::from_rgba_unmultiplied(255, 205, 40, if self.selected_search_hit == Some(index) { 130 } else { 65 }));
                        }
                    }
                    if let Some(target) = link_target { self.select_page(target, &ctx); }
                    painter.rect_stroke(
                        page_rect,
                        1.0,
                        egui::Stroke::new(1.0, egui::Color32::from_black_alpha(80)),
                        egui::StrokeKind::Inside,
                    );
                } else {
                    self.draw_missing_page(ui, rect);
                }
            });
        // Canvas release must commit before a same-frame Save starts a worker.
        self.finish_save_intent(&ctx);
    }
}

impl eframe::App for GlyphApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }
}

fn overlay_screen_rect(rect: crate::core::links::PdfRect, page: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(
            page.left() + rect.x * page.width(),
            page.top() + rect.y * page.height(),
        ),
        egui::pos2(
            page.left() + (rect.x + rect.width) * page.width(),
            page.top() + (rect.y + rect.height) * page.height(),
        ),
    )
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

fn format_page_counter(selected_page: usize, page_count: usize) -> String {
    format!("Page {} / {}", selected_page + 1, page_count)
}

fn draw_canvas_backdrop(painter: &egui::Painter, rect: egui::Rect) {
    painter.rect_filled(rect, 6.0, theme::color(theme::CANVAS));
    let grid_color = theme::translucent(theme::color(theme::STROKE_STRONG), 42);
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
        egui::Stroke::new(1.0, theme::color(theme::STROKE_STRONG)),
        egui::StrokeKind::Inside,
    );
}

#[cfg(test)]
fn tool_chip_button(label: &str) -> egui::Button<'static> {
    egui::Button::new(
        egui::RichText::new(label)
            .color(theme::color(theme::TEXT))
            .size(12.0),
    )
    .wrap_mode(egui::TextWrapMode::Extend)
    .fill(theme::color(theme::PANEL_RAISED))
    .stroke(egui::Stroke::new(1.0, theme::color(theme::STROKE)))
    .corner_radius(egui::CornerRadius::same(4))
    .min_size(egui::vec2(36.0, 26.0))
}

#[cfg(test)]
fn tool_chip(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(tool_chip_button(label))
}

fn metric_pill(ui: &mut egui::Ui, label: &str) {
    egui::Frame::new()
        .fill(theme::color(theme::PANEL_RAISED))
        .stroke(egui::Stroke::new(1.0, theme::color(theme::STROKE)))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(label)
                    .size(12.0)
                    .color(theme::color(theme::TEXT_MUTED)),
            );
        });
}

fn section_header(ui: &mut egui::Ui, label: &str) {
    ui.label(
        egui::RichText::new(label.to_uppercase())
            .size(11.0)
            .strong()
            .color(theme::color(theme::TEXT_MUTED)),
    );
    ui.add_space(4.0);
}

fn page_row(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let fill = if selected {
        theme::color(theme::ACCENT_SOFT)
    } else {
        theme::color(theme::PANEL_RAISED)
    };
    let text = if selected {
        theme::color(theme::ACCENT_STRONG)
    } else {
        theme::color(theme::TEXT)
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
                theme::color(theme::ACCENT)
            } else {
                theme::color(theme::STROKE)
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
        theme::color(theme::ACCENT_SOFT)
    } else {
        theme::color(theme::PANEL_RAISED)
    };
    let text = if selected {
        theme::color(theme::ACCENT_STRONG)
    } else if page_index.is_some() {
        theme::color(theme::TEXT)
    } else {
        theme::color(theme::TEXT_MUTED)
    };
    let label = format_bookmark_label(title, depth, page_index);
    ui.add_enabled(
        true,
        egui::Button::new(egui::RichText::new(label).color(text).size(13.0))
            .selected(selected)
            .fill(fill)
            .stroke(egui::Stroke::new(
                1.0,
                if selected {
                    theme::color(theme::ACCENT)
                } else {
                    theme::color(theme::STROKE)
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
        .fill(theme::color(theme::SURFACE))
        .stroke(egui::Stroke::new(1.0, theme::color(theme::STROKE)))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(note)
                    .color(theme::color(theme::TEXT_MUTED))
                    .size(12.0),
            );
        });
}

fn draw_empty_state(ui: &mut egui::Ui, rect: egui::Rect) {
    let painter = ui.painter_at(rect);
    let panel = egui::Rect::from_center_size(rect.center(), egui::vec2(420.0, 150.0));
    painter.rect_filled(panel, 6.0, theme::color(theme::PANEL));
    painter.rect_stroke(
        panel,
        6.0,
        egui::Stroke::new(1.0, theme::color(theme::STROKE)),
        egui::StrokeKind::Inside,
    );
    painter.rect_filled(
        egui::Rect::from_min_size(panel.min, egui::vec2(4.0, panel.height())),
        0.0,
        theme::color(theme::ACCENT),
    );
    painter.text(
        panel.center_top() + egui::vec2(0.0, 42.0),
        egui::Align2::CENTER_CENTER,
        "Drop a PDF",
        egui::FontId::proportional(20.0),
        theme::color(theme::TEXT),
    );
    painter.text(
        panel.center_top() + egui::vec2(0.0, 72.0),
        egui::Align2::CENTER_CENTER,
        "Drag a file here or press Ctrl+O.",
        egui::FontId::proportional(13.0),
        theme::color(theme::TEXT_MUTED),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "manual full UI performance measurement"]
    fn benchmark_large_document_sidebar() {
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        app.project.open_document(
            "benchmark.pdf".into(),
            crate::pdf::PdfDocumentSummary {
                page_count: 10000,
                pages: (0..10000)
                    .map(|index| crate::pdf::PdfPageInfo {
                        index,
                        label: Some(format!("Sheet {index}")),
                    })
                    .collect(),
                bookmarks: Vec::new(),
                title: None,
            },
        );
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1440., 920.),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input.clone(), |ui| app.draw(ui));
        output.textures_delta.clear();
        let start = Instant::now();
        for _ in 0..20 {
            let mut output = ctx.run_ui(input.clone(), |ui| app.draw(ui));
            output.textures_delta.clear();
        }
        println!("sidebar_10000_pages_20_frames={:?}", start.elapsed());
    }

    #[test]
    fn same_path_stale_generation_links_are_rejected() {
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        app.project.open_document(
            "test.pdf".into(),
            crate::pdf::PdfDocumentSummary {
                page_count: 1,
                pages: vec![],
                bookmarks: vec![],
                title: None,
            },
        );
        app.document_generation = 2;
        app.pending_links = Some((1, 0));
        let (tx, rx) = mpsc::channel();
        app.render_result_rx = rx;
        let link = PdfInternalLink {
            rect: crate::core::links::PdfRect {
                x: 0.1,
                y: 0.2,
                width: 0.3,
                height: 0.4,
            },
            target_page: 0,
        };
        tx.send(RenderJobResult::Links {
            id: 1,
            generation: 1,
            path: "test.pdf".into(),
            page_index: 0,
            result: Ok(vec![link.clone()]),
        })
        .unwrap();
        app.apply_render_results(&ctx);
        assert!(app.page_links.is_empty());
        assert!(app.link_cache.is_empty());
        app.pending_links = Some((2, 0));
        tx.send(RenderJobResult::Links {
            id: 2,
            generation: 2,
            path: "test.pdf".into(),
            page_index: 0,
            result: Ok(vec![link.clone()]),
        })
        .unwrap();
        app.apply_render_results(&ctx);
        assert_eq!(app.page_links, vec![link]);
        let next = app.next_render_job_id;
        app.queue_page_links(&ctx);
        assert_eq!(
            app.next_render_job_id, next,
            "cache hit must not submit work"
        );
    }

    #[test]
    fn link_cache_is_bounded_and_generation_keyed() {
        let mut app = GlyphApp::with_context(&egui::Context::default(), None);
        for page in 0..100 {
            app.cache_links(1, page, vec![]);
            assert!(app.link_cache.len() <= 7);
        }
        assert_eq!(app.link_cache.len(), 7);
        assert!(!app.link_cache.contains_key(&(2, 99)));
        assert!(app.link_cache.contains_key(&(1, 99)));
    }

    #[test]
    fn page_submission_clears_cancelled_pending_tile() {
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        app.project.open_document(
            "test.pdf".into(),
            crate::pdf::PdfDocumentSummary {
                page_count: 1,
                pages: vec![],
                bookmarks: vec![],
                title: None,
            },
        );
        app.zoom = 3.;
        app.page_aspect_ratio = Some(1.);
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500., 500.));
        let page_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1500., 1500.));
        app.ensure_visible_tile(&ctx, viewport, page_rect);
        let cancelled = app.pending_tile_render.clone().unwrap();
        app.queue_page_render("test.pdf".into(), 0, 1800);
        assert!(app.pending_tile_render.is_none());
        app.ensure_visible_tile(&ctx, viewport, page_rect);
        let resubmitted = app.pending_tile_render.as_ref().unwrap();
        assert_eq!(resubmitted.request, cancelled.request);
        assert_ne!(resubmitted.id, cancelled.id);
    }

    #[test]
    fn sheet_navigation_preserves_zoom_and_pan() {
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        app.project.open_document(
            "test.pdf".into(),
            crate::pdf::PdfDocumentSummary {
                page_count: 3,
                pages: Vec::new(),
                bookmarks: Vec::new(),
                title: None,
            },
        );
        app.zoom = 2.5;
        app.pan = egui::vec2(40., -20.);
        app.select_page(1, &ctx);
        assert_eq!(app.zoom, 2.5);
        assert_eq!(app.pan, egui::vec2(40., -20.));
    }
    #[test]
    fn stale_document_inspection_cannot_replace_latest_request() {
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        app.loading_document = Some(2);
        let summary = crate::pdf::PdfDocumentSummary {
            page_count: 0,
            pages: Vec::new(),
            bookmarks: Vec::new(),
            title: None,
        };
        app.apply_inspection(1, "obsolete.pdf".into(), Ok(summary), &ctx);
        assert!(app.project.document.is_none());
        assert_eq!(app.loading_document, Some(2));
    }

    #[test]
    fn page_cache_obeys_raster_and_gpu_memory_budget() {
        let mut app = GlyphApp::with_context(&egui::Context::default(), None);
        for index in 0..7 {
            app.cache_rendered_page(
                RenderedPage {
                    page_index: index,
                    width: 1800,
                    height: 2400,
                    rgba: vec![255; 1800 * 2400 * 4],
                }
                .into(),
            );
        }
        assert!(
            app.page_cache
                .values()
                .map(|p| p.rgba.len() * 2)
                .sum::<usize>()
                <= PAGE_CACHE_BYTE_BUDGET
        );
        assert_eq!(app.page_cache.len(), app.page_cache_order.len());
    }

    #[test]
    fn stale_automation_output_does_not_replace_current_or_loading_document() {
        let ctx = egui::Context::default();
        for (generation, loading) in [(1, None), (0, Some(77))] {
            let mut app = GlyphApp::with_context(&ctx, None);
            app.project.open_document(
                "current.pdf".into(),
                crate::pdf::PdfDocumentSummary {
                    page_count: 1,
                    pages: Vec::new(),
                    bookmarks: Vec::new(),
                    title: None,
                },
            );
            app.document_generation = generation;
            app.loading_document = loading;
            app.status = "Current document status".into();
            let (tx, rx) = mpsc::channel();
            app.automation_rx = Some((0, "current.pdf".into(), rx));
            tx.send(Ok(AutomationOutcome {
                kind: AutomationKind::Bookmarks,
                message: "Saved bookmarks".into(),
                output: Some("export.pdf".into()),
            }))
            .unwrap();
            app.apply_automation_results(&ctx);
            assert_eq!(
                app.project.document.as_ref().unwrap().path,
                PathBuf::from("current.pdf")
            );
            assert_eq!(app.loading_document, loading);
            assert_eq!(app.status, "Current document status");
            let feedback = app.automation_feedback.as_ref().unwrap();
            assert!(feedback.message.contains("current.pdf"));
            assert!(!feedback.busy);
            assert_eq!(feedback.output.as_deref(), Some(Path::new("export.pdf")));
        }
    }

    #[test]
    fn current_bookmark_completion_queues_saved_copy_and_selects_bookmark_tab() {
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        app.project.open_document(
            "current.pdf".into(),
            crate::pdf::PdfDocumentSummary {
                page_count: 1,
                pages: Vec::new(),
                bookmarks: Vec::new(),
                title: None,
            },
        );
        let (tx, rx) = mpsc::channel();
        app.automation_rx = Some((app.document_generation, "current.pdf".into(), rx));
        tx.send(Ok(AutomationOutcome {
            kind: AutomationKind::Bookmarks,
            message: "Saved bookmarks".into(),
            output: Some("export.pdf".into()),
        }))
        .unwrap();
        app.apply_automation_results(&ctx);
        assert!(app.loading_document.is_some());
        assert_eq!(app.navigation_tab, NavigationTab::Bookmarks);
        assert!(!app.automation_feedback.as_ref().unwrap().busy);
    }

    #[test]
    fn automation_completion_remains_visible_after_render_status_changes() {
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        app.project.open_document(
            "fixture.pdf".into(),
            crate::pdf::PdfDocumentSummary {
                page_count: 1,
                pages: Vec::new(),
                bookmarks: Vec::new(),
                title: None,
            },
        );
        let (tx, rx) = mpsc::channel();
        app.automation_rx = Some((app.document_generation, "fixture.pdf".into(), rx));
        tx.send(Ok(automation::AutomationOutcome {
            kind: AutomationKind::Hyperlinks,
            message: "No cross-sheet targets: this PDF contains one sheet.".into(),
            output: None,
        }))
        .unwrap();
        app.apply_automation_results(&ctx);
        app.status = "Rendered high-res viewport tile".into();
        let feedback = app.automation_feedback.as_ref().unwrap();
        assert!(!feedback.busy);
        assert!(feedback.message.contains("one sheet"));
        assert_eq!(
            app.project.document.as_ref().unwrap().path,
            PathBuf::from("fixture.pdf")
        );
    }

    #[test]
    fn cached_page_shares_raster_storage_instead_of_copying_it() {
        let mut app = GlyphApp::with_context(&egui::Context::default(), None);
        app.cache_rendered_page(
            RenderedPage {
                page_index: 0,
                width: 2,
                height: 2,
                rgba: vec![255; 16],
            }
            .into(),
        );
        let first = app.cached_page(0, 2).unwrap();
        let second = app.cached_page(0, 2).unwrap();
        assert_eq!(first.rgba.as_ptr(), second.rgba.as_ptr());
    }

    #[test]
    fn page_cache_never_retains_untracked_pinned_pages() {
        let mut app = GlyphApp::with_context(&egui::Context::default(), None);
        for index in 0usize..32 {
            app.project.selected_page = index.saturating_sub(PAGE_CACHE_LIMIT);
            app.cache_rendered_page(
                RenderedPage {
                    page_index: index,
                    width: 2,
                    height: 2,
                    rgba: vec![255; 16],
                }
                .into(),
            );
        }
        assert!(app.page_cache.len() <= PAGE_CACHE_LIMIT);
        assert_eq!(app.page_cache.len(), app.page_cache_order.len());
    }

    #[test]
    fn navigating_to_a_cached_page_reuses_its_texture() {
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        let page = RenderedPage {
            page_index: 0,
            width: 2,
            height: 2,
            rgba: vec![255; 16],
        };
        app.cache_rendered_page(page.clone().into());
        app.install_texture(&ctx, page.into());
        let texture = app.page_texture.as_ref().unwrap().id();
        app.install_texture(&ctx, app.cached_page(0, 2).unwrap());
        assert_eq!(app.page_texture.as_ref().unwrap().id(), texture);
    }

    #[test]
    #[ignore = "manual cache performance measurement"]
    fn benchmark_cache_lookup() {
        let mut app = GlyphApp::with_context(&egui::Context::default(), None);
        app.cache_rendered_page(
            RenderedPage {
                page_index: 0,
                width: 1800,
                height: 2400,
                rgba: vec![255; 1800 * 2400 * 4],
            }
            .into(),
        );
        let start = Instant::now();
        for _ in 0..200 {
            std::hint::black_box(app.cached_page(0, 1800));
        }
        println!("cache_lookup_200={:?}", start.elapsed());
    }

    #[test]
    fn stale_search_results_cannot_replace_current_document_results() {
        let mut app = GlyphApp::with_context(&egui::Context::default(), None);
        app.search_job_id = 4;
        app.search_running = true;
        app.search_result_tx
            .send(SearchMessage::Finished {
                id: 3,
                cancelled: false,
                result: Ok(vec![]),
            })
            .unwrap();
        app.apply_search_results();
        assert!(app.search_running);
        app.search_result_tx
            .send(SearchMessage::Finished {
                id: 4,
                cancelled: false,
                result: Ok(vec![]),
            })
            .unwrap();
        app.apply_search_results();
        assert!(!app.search_running);
        assert_eq!(app.status, "No matches found in selectable PDF text.");
    }

    #[test]
    fn cancelling_search_invalidates_pending_messages() {
        let mut app = GlyphApp::with_context(&egui::Context::default(), None);
        app.search_running = true;
        app.cancel_search();
        assert!(app.search_cancel.load(Ordering::Relaxed));
        assert!(!app.search_running);
        assert_eq!(app.search_job_id, 1);
    }

    #[test]
    fn normalized_overlays_follow_zoomed_and_panned_page_rect() {
        let page = egui::Rect::from_min_size(egui::pos2(100., -50.), egui::vec2(1000., 2000.));
        let rect = crate::core::links::PdfRect {
            x: 0.1,
            y: 0.2,
            width: 0.3,
            height: 0.1,
        };
        assert_eq!(
            overlay_screen_rect(rect, page),
            egui::Rect::from_min_max(egui::pos2(200., 350.), egui::pos2(500., 550.))
        );
    }

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
