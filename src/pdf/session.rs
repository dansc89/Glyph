use super::{
    MAX_RENDER_HEIGHT, PdfBitmap, PdfBitmapFormat, PdfError, PdfRenderConfig, Pdfium, RenderedPage,
    RenderedTile, TileRequest, bind_bundled_pdfium, map_pdfium_load_error, validate_pdf_path,
};
use std::path::Path;

/// Bind PDFium on the render worker before constructing a session on that worker.
pub fn bind_render_pdfium() -> Result<Pdfium, PdfError> {
    bind_bundled_pdfium()
}

/// A single-worker session retaining one document for `(path, generation)`.
/// Bump generation after edits/replacement, even if the path is unchanged.
/// Pages and bitmaps are scoped to each render call: no self-referential page
/// cache, leaked lifetimes, or unsafe code. At most one page handle is open.
pub struct PdfiumSession<'a> {
    pdfium: &'a Pdfium,
    document: Option<pdfium_bundled::pdfium_render::prelude::PdfDocument<'a>>,
    loads: usize,
    generation: Option<u64>,
    path: std::path::PathBuf,
    // Keep PDFium handles on their worker even with thread-safe bindings.
    _worker_only: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl<'a> PdfiumSession<'a> {
    /// Extract owned character data using the worker's retained document.
    /// PDFium reading order includes generated whitespace and line breaks; boxes
    /// use the rendered page's crop/rotation transform. Rejects pages over
    /// 200000 characters before allocating the character/result collections.
    pub fn extract_page_text(
        &mut self,
        path: &Path,
        generation: u64,
        page_index: usize,
    ) -> Result<super::PageText, PdfError> {
        self.ensure_document(path, generation)?;
        let pages = self.document.as_ref().unwrap().pages();
        if page_index >= pages.len() as usize {
            return Err(PdfError::Render(format!(
                "page index {page_index} is outside document page count {}",
                pages.len()
            )));
        }
        let page = pages
            .get(page_index as i32)
            .map_err(|err| PdfError::Render(err.to_string()))?;
        let text = page
            .text()
            .map_err(|err| PdfError::Render(err.to_string()))?;
        let count = text.len();
        if !(0..=200_000).contains(&count) {
            return Err(PdfError::Render(format!(
                "page {page_index} has {count} text characters; selectable text limit is 200000"
            )));
        }
        let chars = text.chars();
        let mut glyphs = Vec::with_capacity(chars.len());
        for character in chars.iter() {
            let mut rect = character
                .tight_bounds()
                .ok()
                .map(|bounds| crate::core::links::PdfRect {
                    x: bounds.left().value,
                    y: bounds.bottom().value,
                    width: bounds.width().value,
                    height: bounds.height().value,
                })
                .filter(|rect| {
                    [rect.x, rect.y, rect.width, rect.height]
                        .iter()
                        .all(|v| v.is_finite())
                        && rect.width > 0.
                        && rect.height > 0.
                });
            if let Some(rect) = &mut rect {
                super::overlay::normalize_page_rectangle(&page, rect)?;
            }
            glyphs.push(super::TextGlyph {
                text: character
                    .unicode_string()
                    .unwrap_or_else(|| "\u{fffd}".to_owned()),
                rect,
            });
        }
        Ok(super::PageText { page_index, glyphs })
    }

    /// Normalize one page's overlays using this worker's retained document.
    pub fn normalize_rectangles(
        &mut self,
        path: &Path,
        generation: u64,
        page_index: usize,
        rectangles: &mut [crate::core::links::PdfRect],
    ) -> Result<(), PdfError> {
        if rectangles.is_empty() {
            return Ok(());
        }
        self.ensure_document(path, generation)?;
        let pages = self.document.as_ref().unwrap().pages();
        if page_index >= pages.len() as usize {
            return Err(PdfError::Render(format!(
                "page index {page_index} is outside document page count {}",
                pages.len()
            )));
        }
        let page = pages
            .get(page_index as i32)
            .map_err(|err| PdfError::Render(err.to_string()))?;
        for rect in rectangles {
            super::overlay::normalize_page_rectangle(&page, rect)?;
        }
        Ok(())
    }

    pub fn render_tile(
        &mut self,
        path: &Path,
        generation: u64,
        request: TileRequest,
    ) -> Result<RenderedTile, PdfError> {
        validate_pdf_path(path)?;
        let dimensions = [
            request.full_width,
            request.full_height,
            request.width,
            request.height,
        ];
        let fits = dimensions
            .iter()
            .all(|&size| size > 0 && size <= i32::MAX as usize)
            && request
                .x
                .checked_add(request.width)
                .is_some_and(|end| end <= request.full_width)
            && request
                .y
                .checked_add(request.height)
                .is_some_and(|end| end <= request.full_height)
            && request
                .width
                .checked_mul(request.height)
                .and_then(|pixels| pixels.checked_mul(4))
                .is_some_and(|bytes| bytes <= i32::MAX as usize);
        if !fits {
            return Err(PdfError::Render(
                "tile request has invalid dimensions or bounds".to_owned(),
            ));
        }
        self.ensure_document(path, generation)?;
        let pages = self.document.as_ref().unwrap().pages();
        if request.page_index >= pages.len() as usize {
            return Err(PdfError::Render(format!(
                "page index {} is outside document page count {}",
                request.page_index,
                pages.len()
            )));
        }
        let page = pages
            .get(request.page_index as i32)
            .map_err(|err| PdfError::Render(err.to_string()))?;
        let mut bitmap = PdfBitmap::empty(
            request.width as i32,
            request.height as i32,
            PdfBitmapFormat::BGRA,
        )
        .map_err(|err| PdfError::Render(err.to_string()))?;
        let config = PdfRenderConfig::new()
            .set_target_width(request.full_width as i32)
            .set_maximum_height(request.full_height as i32)
            .set_origin(-(request.x as i32), -(request.y as i32))
            .render_form_data(true)
            .render_annotations(true);
        page.render_into_bitmap_with_config(&mut bitmap, &config)
            .map_err(|err| PdfError::Render(err.to_string()))?;
        let rendered = RenderedTile {
            page_index: request.page_index,
            full_width: request.full_width,
            full_height: request.full_height,
            x: request.x,
            y: request.y,
            width: bitmap.width() as usize,
            height: bitmap.height() as usize,
            rgba: bitmap.as_rgba_bytes(),
        };
        if !rendered.is_valid_rgba_buffer() {
            return Err(PdfError::Render(
                "renderer returned invalid tile RGBA buffer".to_owned(),
            ));
        }
        Ok(rendered)
    }
    pub fn new(pdfium: &'a Pdfium) -> Self {
        Self {
            pdfium,
            document: None,
            loads: 0,
            generation: None,
            path: std::path::PathBuf::new(),
            _worker_only: std::marker::PhantomData,
        }
    }

    /// Number of successfully loaded documents (failed attempts do not count).
    pub fn document_load_count(&self) -> usize {
        self.loads
    }

    fn ensure_document(&mut self, path: &Path, generation: u64) -> Result<(), PdfError> {
        validate_pdf_path(path)?;
        if self.document.is_none() || self.generation != Some(generation) || self.path != path {
            self.document = None;
            self.generation = None;
            self.document = Some(
                self.pdfium
                    .load_pdf_from_file(path, None)
                    .map_err(map_pdfium_load_error)?,
            );
            self.generation = Some(generation);
            self.path = path.to_path_buf();
            self.loads += 1;
        }
        Ok(())
    }

    pub fn render_page(
        &mut self,
        path: &Path,
        generation: u64,
        page_index: usize,
        width: u16,
    ) -> Result<RenderedPage, PdfError> {
        self.ensure_document(path, generation)?;
        let pages = self.document.as_ref().unwrap().pages();
        if page_index >= pages.len() as usize {
            return Err(PdfError::Render(format!(
                "page index {page_index} is outside document page count {}",
                pages.len()
            )));
        }
        let page = pages
            .get(page_index as i32)
            .map_err(|err| PdfError::Render(err.to_string()))?;
        let config = PdfRenderConfig::new()
            .set_target_width(width.max(64) as i32)
            .set_maximum_height(MAX_RENDER_HEIGHT)
            .render_form_data(true)
            .render_annotations(true);
        let bitmap = page
            .render_with_config(&config)
            .map_err(|err| PdfError::Render(err.to_string()))?;
        let rendered = RenderedPage {
            page_index,
            width: bitmap.width() as usize,
            height: bitmap.height() as usize,
            rgba: bitmap.as_rgba_bytes(),
        };
        if !rendered.is_valid_rgba_buffer() {
            return Err(PdfError::Render(
                "renderer returned invalid RGBA buffer".to_owned(),
            ));
        }
        Ok(rendered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::{PdfRenderEngine, PdfiumRenderEngine};
    use lopdf::{Document, Object, Stream, dictionary};

    fn write_pdf(path: &Path, color: &str) {
        let mut doc = Document::with_version("1.7");
        let pages = doc.new_object_id();
        let content = doc.add_object(Stream::new(
            dictionary! {},
            format!("{color} rg 20 20 160 160 re f").into_bytes(),
        ));
        let page = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages,
            "MediaBox" => vec![0.into(), 0.into(), 200.into(), 200.into()],
            "Resources" => dictionary! {}, "Contents" => content,
        });
        doc.objects.insert(
            pages,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1,
            }),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", catalog);
        doc.save(path).unwrap();
    }

    #[test]
    fn normalization_reuses_render_document_and_reloads_generation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("page.pdf");
        write_pdf(&path, "1 0 0");
        let pdfium = bind_render_pdfium().unwrap();
        let mut session = PdfiumSession::new(&pdfium);
        session.render_page(&path, 1, 0, 128).unwrap();
        let rect = crate::core::links::PdfRect {
            x: 20.,
            y: 20.,
            width: 160.,
            height: 160.,
        };
        let mut expected = [(0, rect)];
        super::super::overlay::normalize_rectangles(&path, &mut expected).unwrap();
        for _ in 0..3 {
            let mut rects = [rect, rect];
            session
                .normalize_rectangles(&path, 1, 0, &mut rects)
                .unwrap();
            assert_eq!(rects, [expected[0].1; 2]);
        }
        assert_eq!(session.document_load_count(), 1);
        assert!(
            session
                .normalize_rectangles(&path, 1, usize::MAX, &mut [rect])
                .is_err()
        );
        session
            .normalize_rectangles(&path, 2, 0, &mut [rect])
            .unwrap();
        assert_eq!(session.document_load_count(), 2);
    }

    #[test]
    fn invalid_files_do_not_poison_session_or_count_failed_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("page.pdf");
        let pdfium = bind_render_pdfium().unwrap();
        let mut session = PdfiumSession::new(&pdfium);
        assert!(matches!(
            session.render_page(&path, 1, 0, 128),
            Err(PdfError::MissingFile(_))
        ));
        let text = dir.path().join("not.txt");
        std::fs::write(&text, "bad").unwrap();
        assert!(matches!(
            session.render_page(&text, 1, 0, 128),
            Err(PdfError::NotPdf(_))
        ));
        std::fs::write(&path, "bad").unwrap();
        assert!(matches!(
            session.render_page(&path, 1, 0, 128),
            Err(PdfError::Load(_))
        ));
        assert_eq!(session.document_load_count(), 0);
        write_pdf(&path, "1 0 0");
        session.render_page(&path, 1, 0, 128).unwrap();
        std::fs::write(&path, "bad").unwrap();
        assert!(matches!(
            session.render_page(&path, 2, 0, 128),
            Err(PdfError::Load(_))
        ));
        assert_eq!(session.document_load_count(), 1);
        write_pdf(&path, "0 0 1");
        assert_eq!(
            session.render_page(&path, 2, 0, 128).unwrap(),
            PdfiumRenderEngine.render_page(&path, 0, 128).unwrap()
        );
        assert_eq!(session.document_load_count(), 2);
    }

    #[test]
    fn page_indices_are_checked_without_overflow_and_width_is_clamped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("page.pdf");
        write_pdf(&path, "1 0 0");
        let pdfium = bind_render_pdfium().unwrap();
        let mut session = PdfiumSession::new(&pdfium);
        for index in [1, usize::MAX] {
            assert!(matches!(
                session.render_page(&path, 1, index, 128),
                Err(PdfError::Render(_))
            ));
            let request = TileRequest {
                page_index: index,
                full_width: 128,
                full_height: 128,
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            };
            assert!(matches!(
                session.render_tile(&path, 1, request),
                Err(PdfError::Render(_))
            ));
        }
        assert_eq!(
            session.render_page(&path, 1, 0, 0).unwrap(),
            PdfiumRenderEngine.render_page(&path, 0, 0).unwrap()
        );
        assert_eq!(session.document_load_count(), 1);
    }

    #[test]
    #[ignore = "manual timing benchmark; run with --ignored --nocapture"]
    fn benchmark_30_cached_session_vs_stateless() {
        use std::{hint::black_box, time::Instant};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("page.pdf");
        write_pdf(&path, "1 0 0");
        let pdfium = bind_render_pdfium().unwrap();
        let mut session = PdfiumSession::new(&pdfium);
        session.render_page(&path, 1, 0, 512).unwrap();
        PdfiumRenderEngine.render_page(&path, 0, 512).unwrap();
        let start = Instant::now();
        for _ in 0..30 {
            black_box(session.render_page(&path, 1, 0, 512).unwrap());
        }
        let cached = start.elapsed();
        let start = Instant::now();
        for _ in 0..30 {
            black_box(PdfiumRenderEngine.render_page(&path, 0, 512).unwrap());
        }
        let stateless = start.elapsed();
        println!(
            "30 page renders at 512px: cached session={cached:?}, stateless={stateless:?}, document loads={}",
            session.document_load_count()
        );
        assert_eq!(session.document_load_count(), 1);
    }

    #[test]
    fn rejects_invalid_tile_bounds_before_loading_or_allocating() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("page.pdf");
        write_pdf(&path, "1 0 0");
        let pdfium = bind_render_pdfium().unwrap();
        let mut session = PdfiumSession::new(&pdfium);
        let valid = TileRequest {
            page_index: 0,
            full_width: 128,
            full_height: 128,
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        };
        for request in [
            TileRequest {
                full_width: 0,
                ..valid
            },
            TileRequest {
                full_height: 0,
                ..valid
            },
            TileRequest { width: 0, ..valid },
            TileRequest { height: 0, ..valid },
            TileRequest { x: 100, ..valid },
            TileRequest { y: 100, ..valid },
            TileRequest {
                x: usize::MAX,
                ..valid
            },
            TileRequest {
                width: usize::MAX,
                ..valid
            },
            TileRequest {
                full_width: usize::MAX,
                ..valid
            },
            TileRequest {
                full_height: usize::MAX,
                ..valid
            },
            TileRequest {
                full_width: 65536,
                full_height: 65536,
                width: 65536,
                height: 65536,
                ..valid
            },
        ] {
            assert!(
                matches!(
                    session.render_tile(&path, 1, request),
                    Err(PdfError::Render(_))
                ),
                "accepted invalid request: {request:?}"
            );
            assert_eq!(session.document_load_count(), 0);
        }
    }

    #[test]
    fn repeated_tiles_share_page_document_and_match_stateless_rgba() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("page.pdf");
        write_pdf(&path, "1 0 0");
        let pdfium = bind_render_pdfium().unwrap();
        let mut session = PdfiumSession::new(&pdfium);
        for x in [0, 16, 32, 0] {
            let request = TileRequest {
                page_index: 0,
                full_width: 128,
                full_height: 128,
                x,
                y: 24,
                width: 64,
                height: 64,
            };
            assert_eq!(
                session.render_tile(&path, 7, request).unwrap(),
                PdfiumRenderEngine.render_tile(&path, request).unwrap()
            );
            session.render_page(&path, 7, 0, 128).unwrap();
        }
        assert_eq!(session.document_load_count(), 1);
        session
            .render_tile(
                &path,
                8,
                TileRequest {
                    page_index: 0,
                    full_width: 128,
                    full_height: 128,
                    x: 0,
                    y: 0,
                    width: 64,
                    height: 64,
                },
            )
            .unwrap();
        assert_eq!(session.document_load_count(), 2);
    }

    #[test]
    fn path_change_reloads_even_with_same_generation() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.pdf");
        let second = dir.path().join("second.pdf");
        write_pdf(&first, "1 0 0");
        write_pdf(&second, "0 1 0");
        let pdfium = bind_render_pdfium().unwrap();
        let mut session = PdfiumSession::new(&pdfium);
        session.render_page(&first, 1, 0, 128).unwrap();
        assert_eq!(
            session.render_page(&second, 1, 0, 128).unwrap(),
            PdfiumRenderEngine.render_page(&second, 0, 128).unwrap()
        );
        session.render_page(&first, 1, 0, 128).unwrap();
        assert_eq!(session.document_load_count(), 3);
    }

    #[test]
    fn generation_change_reloads_same_path_after_file_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("page.pdf");
        write_pdf(&path, "1 0 0");
        let pdfium = bind_render_pdfium().unwrap();
        let mut session = PdfiumSession::new(&pdfium);
        let before = session.render_page(&path, 1, 0, 128).unwrap();
        let replacement = dir.path().join("replacement.pdf");
        write_pdf(&replacement, "0 0 1");
        std::fs::rename(replacement, &path).unwrap();
        assert_eq!(session.render_page(&path, 1, 0, 128).unwrap(), before);
        let after = session.render_page(&path, 2, 0, 128).unwrap();
        assert_ne!(before.rgba, after.rgba);
        assert_eq!(
            after,
            PdfiumRenderEngine.render_page(&path, 0, 128).unwrap()
        );
        assert_eq!(session.document_load_count(), 2);
        session.render_page(&path, 3, 0, 128).unwrap();
        assert_eq!(session.document_load_count(), 3);
    }

    #[test]
    fn repeated_pages_reuse_document_and_match_stateless_rgba() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("page.pdf");
        write_pdf(&path, "1 0 0");
        let pdfium = bind_render_pdfium().unwrap();
        let mut session = PdfiumSession::new(&pdfium);
        assert_eq!(session.document_load_count(), 0);
        let expected = PdfiumRenderEngine.render_page(&path, 0, 128).unwrap();
        for _ in 0..3 {
            assert_eq!(session.render_page(&path, 7, 0, 128).unwrap(), expected);
        }
        assert_eq!(session.document_load_count(), 1);
    }
}
