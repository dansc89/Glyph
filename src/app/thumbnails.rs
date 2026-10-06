use super::*;

fn thumbnail_caption(page: usize, label: Option<&str>) -> String {
    let number = (page + 1).to_string();
    match label {
        Some(label) if !label.is_empty() && label != number => format!("{number}  {label}"),
        _ => format!("Page {number}"),
    }
}
#[cfg(test)]
mod label_caption_tests {
    use super::*;
    #[test]
    fn numeric_defaults_have_clear_page_names_and_custom_labels_keep_physical_index() {
        assert_eq!(thumbnail_caption(0, None), "Page 1");
        assert_eq!(thumbnail_caption(1, Some("2")), "Page 2");
        assert_eq!(thumbnail_caption(1, Some("A-201")), "2  A-201");
    }
}

#[derive(Default)]
pub(super) struct ThumbnailState {
    cache: HashMap<usize, Option<egui::TextureHandle>>,
    order: VecDeque<usize>,
    pending: HashMap<usize, u64>,
    visible: Vec<usize>,
}
impl GlyphApp {
    pub(super) fn apply_thumbnail_result(
        &mut self,
        ctx: &egui::Context,
        id: u64,
        generation: u64,
        path: PathBuf,
        page: usize,
        result: Result<RenderedPage, PdfError>,
    ) {
        if self.loading_document.is_some()
            || generation != self.document_generation
            || !self
                .project
                .document
                .as_ref()
                .is_some_and(|d| d.path == path)
            || self.thumbnails.pending.get(&page) != Some(&id)
        {
            return;
        }
        self.thumbnails.pending.remove(&page);
        self.cache_thumbnail(ctx, page, result);
    }
    pub(super) fn thumbnail_sidebar(&mut self, ui: &mut egui::Ui) {
        let count = self.page_count().unwrap_or(0);
        // Include overscan within the 64-image cache even on exceptionally tall windows.
        let row_height = (ui.available_height() / 60.).max(164.);
        egui::ScrollArea::vertical()
            .id_salt("page-thumbnails")
            .show_rows(ui, row_height, count, |ui, rows| {
                self.request_thumbnail_range(rows.clone());
                for page in rows {
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), row_height),
                        egui::Sense::click(),
                    );
                    let selected = self.project.selected_page == page;
                    let painter = ui.painter_at(rect);
                    painter.rect_filled(
                        rect.shrink(2.),
                        4.,
                        theme::color(if selected {
                            theme::ACCENT_SOFT
                        } else {
                            theme::CARD
                        }),
                    );
                    if selected {
                        painter.rect_stroke(
                            rect.shrink(2.),
                            4.,
                            egui::Stroke::new(1., theme::color(theme::ACCENT)),
                            egui::StrokeKind::Inside,
                        );
                    }
                    let image_area = egui::Rect::from_min_max(
                        rect.min + egui::vec2(12., 8.),
                        rect.max - egui::vec2(12., 28.),
                    );
                    if let Some(Some(texture)) = self.thumbnails.cache.get(&page) {
                        let size = texture.size_vec2();
                        let scale = (image_area.width() / size.x).min(image_area.height() / size.y);
                        painter.image(
                            texture.id(),
                            egui::Rect::from_center_size(image_area.center(), size * scale),
                            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                            egui::Color32::WHITE,
                        );
                    } else {
                        painter.text(
                            image_area.center(),
                            egui::Align2::CENTER_CENTER,
                            if self.thumbnails.cache.contains_key(&page) {
                                "Preview unavailable"
                            } else {
                                "Loading preview..."
                            },
                            egui::FontId::monospace(11.),
                            theme::color(theme::TEXT_MUTED),
                        );
                    }
                    let label = self
                        .project
                        .document
                        .as_ref()
                        .and_then(|d| d.summary.pages.get(page))
                        .and_then(|p| p.label.as_deref());
                    let caption = thumbnail_caption(page, label);
                    let mut job = egui::text::LayoutJob::simple(
                        caption.clone(),
                        egui::FontId::monospace(11.),
                        theme::color(theme::TEXT),
                        (rect.width() - 24.).max(1.),
                    );
                    job.wrap.max_rows = 1;
                    let galley = painter.layout_job(job);
                    let position = egui::pos2(
                        rect.center().x - galley.size().x / 2.,
                        rect.bottom() - 15. - galley.size().y / 2.,
                    );
                    painter.galley(position, galley, theme::color(theme::TEXT));
                    response.clone().on_hover_text(caption);
                    self.page_edit_menu(&response, page);
                    if response.clicked() {
                        self.select_page(page, ui.ctx());
                    }
                }
            });
    }

    fn cache_thumbnail(
        &mut self,
        ctx: &egui::Context,
        page: usize,
        result: Result<RenderedPage, PdfError>,
    ) {
        let texture = result
            .ok()
            .filter(|image| {
                image.is_valid_rgba_buffer() && image.width <= 128 && image.height <= 160
            })
            .map(|image| {
                ctx.load_texture(
                    format!("thumbnail-{}-{page}", self.document_generation),
                    egui::ColorImage::from_rgba_unmultiplied(
                        [image.width, image.height],
                        &image.rgba,
                    ),
                    egui::TextureOptions::LINEAR,
                )
            });
        self.thumbnails.order.retain(|&p| p != page);
        self.thumbnails.order.push_back(page);
        self.thumbnails.cache.insert(page, texture);
        while self.thumbnails.order.len() > 64 {
            if let Some(old) = self.thumbnails.order.pop_front() {
                self.thumbnails.cache.remove(&old);
            }
        }
    }
    fn request_thumbnail_range(&mut self, rows: std::ops::Range<usize>) {
        if self.loading_document.is_some() {
            return;
        }
        let count = self.page_count().unwrap_or(0);
        let visible: Vec<_> = rows.take(64).filter(|&p| p < count).collect();
        if self.thumbnails.visible == visible
            && (!self.thumbnails.pending.is_empty()
                || visible
                    .iter()
                    .all(|p| self.thumbnails.cache.contains_key(p)))
        {
            return;
        }
        self.thumbnails.visible = visible.clone();
        self.thumbnails.pending.clear();
        let mut jobs = Vec::new();
        if let Some(document) = &self.project.document {
            for page_index in visible {
                if self.thumbnails.cache.contains_key(&page_index) {
                    continue;
                }
                if jobs.len() == 12 {
                    break;
                }
                let id = self.next_render_job_id;
                self.next_render_job_id = id.wrapping_add(1).max(1);
                self.thumbnails.pending.insert(page_index, id);
                jobs.push(RenderJob {
                    id,
                    generation: self.document_generation,
                    path: document.path.clone(),
                    kind: JobKind::Thumbnail { page_index },
                });
            }
        }
        self.render_worker.set_thumbnails(jobs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tall_sidebar_finishes_all_visible_thumbnails_in_bounded_batches() {
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        app.project.open_document(
            "tall.pdf".into(),
            crate::pdf::PdfDocumentSummary {
                page_count: 20,
                pages: Vec::new(),
                bookmarks: Vec::new(),
                title: None,
            },
        );
        app.request_thumbnail_range(0..20);
        for _ in 0..3 {
            for (page, id) in app.thumbnails.pending.clone() {
                app.apply_thumbnail_result(
                    &ctx,
                    id,
                    app.document_generation,
                    "tall.pdf".into(),
                    page,
                    Ok(RenderedPage {
                        page_index: page,
                        width: 2,
                        height: 4,
                        rgba: vec![255; 32],
                    }),
                );
            }
            app.request_thumbnail_range(0..20);
            assert!(app.thumbnails.pending.len() <= 12);
        }
        assert_eq!(
            app.thumbnails.cache.len(),
            20,
            "all visible cards must settle without scrolling"
        );
        assert!(app.thumbnails.pending.is_empty());
    }
    #[test]
    fn thumbnail_cache_is_bounded_and_keeps_recent_images() {
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        for page_index in 0..80 {
            app.cache_thumbnail(
                &ctx,
                page_index,
                Ok(RenderedPage {
                    page_index,
                    width: 2,
                    height: 4,
                    rgba: vec![255; 32],
                }),
            );
        }
        assert_eq!(app.thumbnails.cache.len(), 64);
        assert!(!app.thumbnails.cache.contains_key(&0));
        assert!(app.thumbnails.cache.get(&79).unwrap().is_some());
    }
    #[test]
    fn visible_thumbnail_requests_are_bounded_and_not_reissued_on_idle_frames() {
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        app.project.open_document(
            "thumbnails.pdf".into(),
            crate::pdf::PdfDocumentSummary {
                page_count: 10000,
                pages: Vec::new(),
                bookmarks: Vec::new(),
                title: None,
            },
        );
        app.request_thumbnail_range(0..10000);
        assert_eq!(app.thumbnails.pending.len(), 12);
        let id = app.next_render_job_id;
        app.request_thumbnail_range(0..10000);
        assert_eq!(
            app.next_render_job_id, id,
            "idle frames must not submit thumbnail work"
        );
        app.request_thumbnail_range(9000..9004);
        assert_eq!(app.thumbnails.pending.len(), 4);
        assert!(
            app.thumbnails
                .pending
                .keys()
                .all(|p| (9000..9004).contains(p))
        );
    }
}
