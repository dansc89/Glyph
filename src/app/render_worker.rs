//! Single-owner PDF rendering, with latest-request-wins foreground scheduling.
use super::RenderJobResult;
use crate::pdf::{PdfError, PdfiumSession, TileRequest, bind_render_pdfium};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, mpsc},
    thread,
};

#[derive(Debug, Clone)]
pub(super) enum JobKind {
    Inspect,
    Text {
        page_index: usize,
    },
    Page {
        page_index: usize,
        target_width: u16,
    },
    Tile(TileRequest),
    Links {
        page_index: usize,
    },
    Thumbnail {
        page_index: usize,
    },
    Prefetch {
        page_index: usize,
        target_width: u16,
    },
}
#[derive(Debug, Clone)]
pub(super) struct RenderJob {
    pub id: u64,
    pub generation: u64,
    pub path: PathBuf,
    pub kind: JobKind,
}
#[derive(Default)]
struct JobQueue {
    generation: u64,
    snapshot: Option<(u64, PathBuf, Vec<u8>)>,
    inspection: Option<RenderJob>,
    latest_inspection: u64,
    foreground: Option<RenderJob>,
    tile: Option<RenderJob>,
    links: Option<RenderJob>,
    latest_links: u64,
    text: Option<RenderJob>,
    latest_text: u64,
    prefetch: VecDeque<RenderJob>,
    thumbnails: VecDeque<RenderJob>,
    thumbnail_ids: std::collections::HashSet<u64>,
    latest_page: u64,
    latest_tile: u64,
    closed: bool,
}
impl JobQueue {
    fn set_view(&mut self, _page_index: usize) {
        self.links = None;
        self.latest_links = 0;
        self.text = None;
        self.latest_text = 0;
        self.foreground = None;
        self.tile = None;
        self.prefetch.clear();
        self.latest_page = 0;
        self.latest_tile = 0;
    }

    fn reset(&mut self, generation: u64) {
        self.snapshot = None;
        self.links = None;
        self.latest_links = 0;
        self.text = None;
        self.latest_text = 0;
        self.generation = generation;
        self.thumbnails.clear();
        self.thumbnail_ids.clear();
        self.inspection = None;
        self.latest_inspection = 0;
        self.foreground = None;
        self.tile = None;
        self.prefetch.clear();
        self.latest_page = 0;
        self.latest_tile = 0;
    }
    fn submit(&mut self, job: RenderJob) {
        if self.closed || (self.generation != 0 && self.generation != job.generation) {
            return;
        }
        self.generation = job.generation;
        match job.kind {
            JobKind::Text { .. } => {
                self.latest_text = job.id;
                self.text = Some(job);
            }
            JobKind::Inspect => {
                self.latest_inspection = job.id;
                self.inspection = Some(job);
                self.set_view(0);
            }
            JobKind::Page { .. } => {
                self.latest_page = job.id;
                self.foreground = Some(job);
                self.tile = None;
                self.latest_tile = 0;
                self.prefetch.clear();
            }
            JobKind::Tile(_) => {
                self.latest_tile = job.id;
                self.tile = Some(job);
            }
            JobKind::Links { .. } => {
                self.latest_links = job.id;
                self.links = Some(job);
            }
            JobKind::Thumbnail { page_index } => {
                if self.thumbnail_ids.len() < 12 && !self.thumbnails.iter().any(|j| matches!(j.kind, JobKind::Thumbnail { page_index: p } if p == page_index)) {
                    self.thumbnail_ids.insert(job.id);
                    self.thumbnails.push_back(job);
                }
            }
            JobKind::Prefetch { page_index, .. } => {
                if !self
                    .prefetch
                    .iter()
                    .any(|j| matches!(j.kind,JobKind::Prefetch{page_index:p,..} if p==page_index))
                    && self.prefetch.len() < 4
                {
                    self.prefetch.push_back(job);
                }
            }
        }
    }
    fn pop(&mut self) -> Option<RenderJob> {
        self.inspection
            .take()
            .or_else(|| self.foreground.take())
            .or_else(|| self.tile.take())
            .or_else(|| self.text.take())
            .or_else(|| self.links.take())
            .or_else(|| self.thumbnails.pop_front())
            .or_else(|| self.prefetch.pop_front())
    }
    fn len(&self) -> usize {
        usize::from(self.inspection.is_some())
            + usize::from(self.foreground.is_some())
            + usize::from(self.tile.is_some())
            + usize::from(self.text.is_some())
            + usize::from(self.links.is_some())
            + self.thumbnails.len()
            + self.prefetch.len()
    }
    fn relevant(&self, job: &RenderJob) -> bool {
        !self.closed
            && self.generation == job.generation
            && match job.kind {
                JobKind::Text { .. } => job.id == self.latest_text,
                JobKind::Inspect => job.id == self.latest_inspection,
                JobKind::Page { .. } => job.id == self.latest_page,
                JobKind::Tile(_) => job.id == self.latest_tile,
                JobKind::Links { .. } => job.id == self.latest_links,
                JobKind::Thumbnail { .. } => self.thumbnail_ids.contains(&job.id),
                JobKind::Prefetch { .. } => true,
            }
    }
}

pub(super) struct RenderWorker {
    state: Arc<(Mutex<JobQueue>, Condvar)>,
}
impl RenderWorker {
    pub fn new(ctx: &egui::Context) -> (Self, mpsc::Receiver<RenderJobResult>) {
        let state = Arc::new((Mutex::new(JobQueue::default()), Condvar::new()));
        // Never accumulate an unbounded backlog of multi-megabyte rendered bitmaps.
        let (tx, rx) = mpsc::sync_channel(2);
        let worker_state = state.clone();
        let ctx = ctx.clone();
        thread::Builder::new()
            .name("glyph-render".into())
            .spawn(move || {
                let binding = bind_render_pdfium();
                let mut session = binding.as_ref().ok().map(PdfiumSession::new);
                let mut link_index: Option<(
                    u64,
                    PathBuf,
                    Result<crate::pdf::InternalLinkIndex, PdfError>,
                )> = None;
                let mut source_error: Option<(u64, PathBuf, String)> = None;
                loop {
                    let (job, snapshot) = {
                        let (lock, wake) = &*worker_state;
                        let mut queue = lock.lock().unwrap();
                        while queue.len() == 0 && !queue.closed {
                            queue = wake.wait(queue).unwrap();
                        }
                        if queue.closed {
                            break;
                        }
                        (queue.pop().unwrap(), queue.snapshot.take())
                    };
                    if let Some((generation, path, bytes)) = snapshot {
                        source_error = session.as_mut().map_or_else(
                            || Some((generation, path.clone(), "PDFium unavailable".into())),
                            |s| {
                                s.replace_document(&path, generation, bytes)
                                    .err()
                                    .map(|e| (generation, path.clone(), e.to_string()))
                            },
                        );
                    }
                    let failed = source_error
                        .as_ref()
                        .filter(|(g, p, _)| *g == job.generation && *p == job.path);
                    let mut active_session = if failed.is_some() {
                        None
                    } else {
                        session.as_mut()
                    };
                    let error = || {
                        PdfError::Render(
                            failed
                                .map(|(_, _, e)| e.clone())
                                .or_else(|| binding.as_ref().err().map(ToString::to_string))
                                .unwrap_or_else(|| "PDFium unavailable".into()),
                        )
                    };
                    if !worker_state.0.lock().unwrap().relevant(&job) {
                        continue;
                    }
                    if link_index.as_ref().is_some_and(|(generation, path, _)| {
                        *generation != job.generation || *path != job.path
                    }) {
                        link_index = None;
                    }
                    let result = match job.kind {
                        JobKind::Text { page_index } => RenderJobResult::Text {
                            id: job.id,
                            generation: job.generation,
                            path: job.path.clone(),
                            page_index,
                            result: active_session.as_mut().map_or_else(
                                || Err(error()),
                                |s| s.extract_page_text(&job.path, job.generation, page_index),
                            ),
                        },
                        JobKind::Inspect => RenderJobResult::Inspection {
                            id: job.id,
                            path: job.path.clone(),
                            result: crate::pdf::PdfEngine::inspect(
                                &crate::pdf::LopdfInspectionEngine,
                                &job.path,
                            ),
                        },
                        JobKind::Page {
                            page_index,
                            target_width,
                        } => RenderJobResult::Page {
                            id: job.id,
                            generation: job.generation,
                            path: job.path.clone(),
                            page_index,
                            target_width,
                            result: active_session.as_mut().map_or_else(
                                || Err(error()),
                                |s| {
                                    s.render_page(
                                        &job.path,
                                        job.generation,
                                        page_index,
                                        target_width,
                                    )
                                },
                            ),
                        },
                        JobKind::Tile(request) => RenderJobResult::Tile {
                            id: job.id,
                            generation: job.generation,
                            path: job.path.clone(),
                            request,
                            result: active_session.as_mut().map_or_else(
                                || Err(error()),
                                |s| s.render_tile(&job.path, job.generation, request),
                            ),
                        },
                        JobKind::Links { page_index } => {
                            if link_index.is_none() {
                                link_index = Some((
                                    job.generation,
                                    job.path.clone(),
                                    crate::pdf::InternalLinkIndex::load(&job.path),
                                ));
                            }
                            // Parsing cannot be interrupted; skip normalization/publication
                            // if navigation superseded the request during the parse.
                            if !worker_state.0.lock().unwrap().relevant(&job) {
                                continue;
                            }
                            let links = match &link_index.as_ref().unwrap().2 {
                                Ok(index) => index.links(page_index),
                                Err(err) => Err(PdfError::Load(err.to_string())),
                            };
                            if !worker_state.0.lock().unwrap().relevant(&job) {
                                continue;
                            }
                            let result = links.and_then(|mut links| {
                                if failed.is_some() {
                                    return Err(error());
                                }
                                let mut rects: Vec<_> =
                                    links.iter().map(|link| link.rect).collect();
                                if !rects.is_empty() {
                                    active_session
                                        .as_mut()
                                        .ok_or_else(error)?
                                        .normalize_rectangles(
                                            &job.path,
                                            job.generation,
                                            page_index,
                                            &mut rects,
                                        )?;
                                }
                                for (link, rect) in links.iter_mut().zip(rects) {
                                    link.rect = rect;
                                }
                                Ok(links)
                            });
                            RenderJobResult::Links {
                                id: job.id,
                                generation: job.generation,
                                path: job.path.clone(),
                                page_index,
                                result,
                            }
                        }
                        JobKind::Thumbnail { page_index } => RenderJobResult::Thumbnail {
                            id: job.id,
                            generation: job.generation,
                            path: job.path.clone(),
                            page_index,
                            result: active_session.as_mut().map_or_else(
                                || Err(error()),
                                |s| {
                                    s.render_page(&job.path, job.generation, page_index, 128)
                                        .and_then(shrink_thumbnail)
                                },
                            ),
                        },
                        JobKind::Prefetch {
                            page_index,
                            target_width,
                        } => RenderJobResult::PrefetchPage {
                            generation: job.generation,
                            path: job.path.clone(),
                            page_index,
                            target_width,
                            result: active_session.as_mut().map_or_else(
                                || Err(error()),
                                |s| {
                                    s.render_page(
                                        &job.path,
                                        job.generation,
                                        page_index,
                                        target_width,
                                    )
                                },
                            ),
                        },
                    };
                    if !worker_state.0.lock().unwrap().relevant(&job) {
                        continue;
                    }
                    // Wake before send as well: a bounded send may wait for the UI to drain older results.
                    ctx.request_repaint();
                    if tx.send(result).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .expect("start render worker");
        (Self { state }, rx)
    }
    pub fn replace_source(&self, generation: u64, path: PathBuf, bytes: Vec<u8>) {
        let mut queue = self.state.0.lock().unwrap();
        queue.reset(generation);
        queue.snapshot = Some((generation, path, bytes));
        self.state.1.notify_one();
    }
    pub fn set_thumbnails(&self, jobs: Vec<RenderJob>) {
        let mut queue = self.state.0.lock().unwrap();
        queue.thumbnails.clear();
        queue.thumbnail_ids.clear();
        for job in jobs.into_iter().take(12) {
            queue.submit(job);
        }
        self.state.1.notify_one();
    }
    pub fn set_view(&self, page_index: usize) {
        self.state.0.lock().unwrap().set_view(page_index);
    }
    pub fn reset(&self, generation: u64) {
        self.state.0.lock().unwrap().reset(generation);
        self.state.1.notify_one();
    }
    pub fn submit(&self, job: RenderJob) {
        self.state.0.lock().unwrap().submit(job);
        self.state.1.notify_one();
    }
}
impl Drop for RenderWorker {
    fn drop(&mut self) {
        let mut queue = self.state.0.lock().unwrap();
        queue.closed = true;
        queue.inspection = None;
        queue.links = None;
        queue.text = None;
        queue.foreground = None;
        queue.tile = None;
        queue.prefetch.clear();
        queue.thumbnails.clear();
        queue.thumbnail_ids.clear();
        self.state.1.notify_one();
    }
}

fn shrink_thumbnail(image: crate::pdf::RenderedPage) -> Result<crate::pdf::RenderedPage, PdfError> {
    if !image.is_valid_rgba_buffer() {
        return Err(PdfError::Render("Invalid thumbnail pixels".into()));
    }
    let scale = (128. / image.width as f32)
        .min(160. / image.height as f32)
        .min(1.);
    if scale == 1. {
        return Ok(image);
    }
    let width = ((image.width as f32 * scale) as usize).max(1);
    let height = ((image.height as f32 * scale) as usize).max(1);
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let source = ((y * image.height / height) * image.width + x * image.width / width) * 4;
            rgba.extend_from_slice(&image.rgba[source..source + 4]);
        }
    }
    Ok(crate::pdf::RenderedPage {
        page_index: image.page_index,
        width,
        height,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn job(id: u64, kind: JobKind) -> RenderJob {
        RenderJob {
            id,
            generation: 1,
            path: "test.pdf".into(),
            kind,
        }
    }
    fn invalid_snapshot_result(kind: JobKind) -> RenderJobResult {
        use lopdf::{Document, Object, dictionary};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("valid-on-disk.pdf");
        let mut doc = Document::with_version("1.7");
        let pages = doc.new_object_id();
        let page = doc.new_object_id();
        let link = doc.add_object(dictionary! {"Type"=>"Annot", "Subtype"=>"Link", "Rect"=>vec![10.into(),10.into(),50.into(),50.into()], "Dest"=>vec![Object::Reference(page), Object::Name(b"Fit".to_vec())]});
        doc.objects.insert(page, dictionary! {"Type"=>"Page", "Parent"=>pages, "MediaBox"=>vec![0.into(),0.into(),200.into(),200.into()], "Annots"=>vec![Object::Reference(link)]}.into());
        doc.objects.insert(
            pages,
            dictionary! {"Type"=>"Pages", "Kids"=>vec![Object::Reference(page)], "Count"=>1}.into(),
        );
        let root = doc.add_object(dictionary! {"Type"=>"Catalog", "Pages"=>pages});
        doc.trailer.set("Root", root);
        doc.save(&path).unwrap();
        assert_eq!(
            crate::pdf::InternalLinkIndex::load(&path)
                .unwrap()
                .links(0)
                .unwrap()
                .len(),
            1
        );
        let ctx = egui::Context::default();
        let (worker, rx) = RenderWorker::new(&ctx);
        // First populate the real PDFium session from disk, then reject a replacement.
        worker.submit(RenderJob {
            id: 1,
            generation: 1,
            path: path.clone(),
            kind: JobKind::Page {
                page_index: 0,
                target_width: 128,
            },
        });
        assert!(matches!(
            rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap(),
            RenderJobResult::Page { result: Ok(_), .. }
        ));
        worker.replace_source(2, path.clone(), b"invalid PDF snapshot".to_vec());
        worker.submit(RenderJob {
            id: 2,
            generation: 2,
            path,
            kind,
        });
        rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap()
    }
    #[test]
    fn invalid_snapshot_page_does_not_fall_back_to_disk() {
        assert!(matches!(
            invalid_snapshot_result(JobKind::Page {
                page_index: 0,
                target_width: 128
            }),
            RenderJobResult::Page {
                result: Err(PdfError::Render(_)),
                ..
            }
        ));
    }
    #[test]
    fn invalid_snapshot_text_does_not_fall_back_to_disk() {
        assert!(matches!(
            invalid_snapshot_result(JobKind::Text { page_index: 0 }),
            RenderJobResult::Text {
                result: Err(PdfError::Render(_)),
                ..
            }
        ));
    }
    #[test]
    fn invalid_snapshot_link_normalization_does_not_fall_back_to_disk() {
        assert!(
            matches!(
                invalid_snapshot_result(JobKind::Links { page_index: 0 }),
                RenderJobResult::Links {
                    result: Err(PdfError::Render(_)),
                    ..
                }
            ),
            "link normalization must use source-error-gated session, never reopen disk"
        );
    }
    #[test]
    fn worker_renders_snapshot_instead_of_original_source() {
        use lopdf::{Document, Object, Stream, dictionary};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("original.pdf");
        fn bytes(color: &str) -> Vec<u8> {
            let mut d = Document::with_version("1.7");
            let pages = d.new_object_id();
            let c = d.add_object(Stream::new(
                dictionary! {},
                format!("{color} rg 20 20 160 160 re f").into_bytes(),
            ));
            let page=d.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"MediaBox"=>vec![0.into(),0.into(),200.into(),200.into()],"Contents"=>c,"Resources"=>dictionary!{}});
            d.objects.insert(
                pages,
                Object::Dictionary(
                    dictionary! {"Type"=>"Pages","Kids"=>vec![page.into()],"Count"=>1},
                ),
            );
            let root = d.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages});
            d.trailer.set("Root", root);
            let mut b = vec![];
            d.save_to(&mut b).unwrap();
            b
        }
        let original = bytes("1 0 0");
        std::fs::write(&path, &original).unwrap();
        let ctx = egui::Context::default();
        let (worker, rx) = RenderWorker::new(&ctx);
        worker.replace_source(2, path.clone(), bytes("0 1 0"));
        worker.submit(RenderJob {
            id: 1,
            generation: 2,
            path: path.clone(),
            kind: JobKind::Page {
                page_index: 0,
                target_width: 128,
            },
        });
        match rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap() {
            RenderJobResult::Page { result, .. } => assert!(
                result
                    .unwrap()
                    .rgba
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|p| p[1] > 200 && p[0] < 50),
                "Worker must install memory snapshot before foreground rendering"
            ),
            _ => panic!("unexpected result"),
        }
        assert_eq!(std::fs::read(path).unwrap(), original);
    }
    #[test]
    fn thumbnails_are_bounded_and_foreground_precedes_them() {
        let mut queue = JobQueue::default();
        for i in 0..30 {
            queue.submit(job(
                i + 1,
                JobKind::Thumbnail {
                    page_index: i as usize,
                },
            ));
        }
        assert_eq!(queue.len(), 12, "visible thumbnail jobs must be bounded");
        queue.submit(job(
            31,
            JobKind::Page {
                page_index: 100,
                target_width: 1200,
            },
        ));
        assert!(matches!(queue.pop().unwrap().kind, JobKind::Page { .. }));
        assert!(matches!(
            queue.pop().unwrap().kind,
            JobKind::Thumbnail { page_index: 0 }
        ));
    }
    #[test]
    fn thumbnail_scope_bounds_accepted_jobs_even_after_the_worker_pops_them() {
        let mut queue = JobQueue::default();
        for i in 0..40 {
            queue.submit(job(
                i + 1,
                JobKind::Thumbnail {
                    page_index: i as usize,
                },
            ));
            queue.pop();
        }
        assert!(
            queue.thumbnail_ids.len() <= 12,
            "accepted/inflight ID tracking must stay bounded"
        );
    }
    #[test]
    fn text_requests_coalesce_and_follow_foreground_without_blocking_it() {
        let mut queue = JobQueue::default();
        for i in 0..30 {
            queue.submit(job(
                i + 1,
                JobKind::Text {
                    page_index: i as usize,
                },
            ));
        }
        assert_eq!(queue.len(), 1);
        queue.submit(job(
            31,
            JobKind::Page {
                page_index: 29,
                target_width: 1000,
            },
        ));
        assert!(matches!(queue.pop().unwrap().kind, JobKind::Page { .. }));
        let text = queue.pop().unwrap();
        assert!(matches!(text.kind, JobKind::Text { page_index: 29 }));
        assert!(queue.relevant(&text));
        queue.set_view(0);
        assert!(!queue.relevant(&text));
    }
    #[test]
    fn thumbnail_pixels_are_bounded_even_for_a_tall_page() {
        let source = crate::pdf::RenderedPage {
            page_index: 9,
            width: 128,
            height: 1000,
            rgba: vec![255; 128 * 1000 * 4],
        };
        let thumbnail = shrink_thumbnail(source).unwrap();
        assert_eq!(thumbnail.page_index, 9);
        assert!(thumbnail.width <= 128 && thumbnail.height <= 160);
        assert!(thumbnail.is_valid_rgba_buffer());
        assert_eq!(thumbnail.rgba[0], 255);
    }
    #[test]
    fn worker_link_index_reuses_read_document_until_generation_changes() {
        use lopdf::{Document, Object, dictionary};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("links.pdf");
        let mut doc = Document::with_version("1.7");
        let pages = doc.new_object_id();
        let page_ids: Vec<_> = (0..2).map(|_| doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages, "MediaBox" => vec![0.into(), 0.into(), 200.into(), 200.into()] })).collect();
        doc.objects.insert(pages, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => page_ids.iter().copied().map(Object::Reference).collect::<Vec<_>>(), "Count" => 2 }));
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", catalog);
        doc.save(&path).unwrap();
        let (worker, results) = RenderWorker::new(&egui::Context::default());
        for (id, generation, page_index) in [(1, 1, 0), (2, 1, 1), (3, 2, 0)] {
            if id == 2 {
                std::fs::remove_file(&path).unwrap();
            }
            if id == 3 {
                worker.reset(2);
            }
            worker.submit(RenderJob {
                id,
                generation,
                path: path.clone(),
                kind: JobKind::Links { page_index },
            });
            let RenderJobResult::Links {
                id: actual_id,
                generation: actual_generation,
                result,
                ..
            } = results
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap()
            else {
                panic!("expected link result")
            };
            assert_eq!((actual_id, actual_generation), (id, generation));
            if id < 3 {
                assert!(result.unwrap().is_empty());
            } else {
                assert!(
                    result.is_err(),
                    "new generation must reload even the same path"
                );
            }
        }
    }

    #[test]
    fn link_slot_coalesces_and_cancels_superseded_work() {
        let mut queue = JobQueue::default();
        let first = job(1, JobKind::Links { page_index: 0 });
        queue.submit(first.clone());
        let running = queue.pop().unwrap();
        for id in 2..1000 {
            queue.submit(job(
                id,
                JobKind::Links {
                    page_index: id as usize,
                },
            ));
        }
        assert_eq!(queue.len(), 1);
        assert!(!queue.relevant(&running));
        let latest = queue.pop().unwrap();
        assert_eq!(latest.id, 999);
        assert!(queue.relevant(&latest));
        queue.set_view(0);
        assert!(!queue.relevant(&latest));
        queue.reset(2);
        queue.submit(first);
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn foreground_and_tile_precede_links_which_precede_prefetch() {
        let mut queue = JobQueue::default();
        queue.submit(job(
            1,
            JobKind::Page {
                page_index: 0,
                target_width: 1800,
            },
        ));
        queue.submit(job(2, JobKind::Links { page_index: 0 }));
        queue.submit(job(
            3,
            JobKind::Prefetch {
                page_index: 1,
                target_width: 1800,
            },
        ));
        queue.submit(job(
            4,
            JobKind::Tile(TileRequest {
                page_index: 0,
                full_width: 100,
                full_height: 100,
                x: 0,
                y: 0,
                width: 50,
                height: 50,
            }),
        ));
        assert_eq!(
            (0..4).map(|_| queue.pop().unwrap().id).collect::<Vec<_>>(),
            vec![1, 4, 2, 3]
        );
    }

    #[test]
    fn navigation_coalesces_without_an_unbounded_backlog() {
        let mut queue = JobQueue::default();
        for index in 0..1000 {
            queue.submit(job(
                index,
                JobKind::Page {
                    page_index: index as usize,
                    target_width: 1800,
                },
            ));
        }
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.pop().unwrap().id, 999);
        assert!(queue.pop().is_none());
    }
    #[test]
    fn foreground_precedes_prefetch_and_tiles_supersede_obsolete_tiles() {
        let mut queue = JobQueue::default();
        queue.submit(job(
            1,
            JobKind::Page {
                page_index: 0,
                target_width: 1800,
            },
        ));
        queue.submit(job(
            2,
            JobKind::Prefetch {
                page_index: 1,
                target_width: 1800,
            },
        ));
        let request = crate::pdf::TileRequest {
            page_index: 0,
            full_width: 4000,
            full_height: 5000,
            x: 0,
            y: 0,
            width: 500,
            height: 500,
        };
        queue.submit(job(3, JobKind::Tile(request)));
        queue.submit(job(4, JobKind::Tile(request)));
        assert_eq!(queue.pop().unwrap().id, 1);
        assert_eq!(queue.pop().unwrap().id, 4);
        assert_eq!(queue.pop().unwrap().id, 2);
    }
    #[test]
    fn changing_to_a_cached_view_invalidates_inflight_foreground_work() {
        let mut queue = JobQueue::default();
        let old = job(
            1,
            JobKind::Page {
                page_index: 0,
                target_width: 1800,
            },
        );
        queue.submit(old.clone());
        queue.set_view(2);
        assert!(!queue.relevant(&old));
        assert_eq!(queue.len(), 0);
    }
    #[test]
    fn obsolete_document_jobs_and_results_are_invalidated() {
        let mut queue = JobQueue::default();
        let old = job(
            1,
            JobKind::Page {
                page_index: 0,
                target_width: 1800,
            },
        );
        queue.submit(old.clone());
        queue.reset(2);
        assert_eq!(queue.len(), 0);
        assert!(!queue.relevant(&old));
        queue.submit(old);
        assert_eq!(queue.len(), 0);
    }
}
