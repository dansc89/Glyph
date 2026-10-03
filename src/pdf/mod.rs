use lopdf::{Document, Object, ObjectId};
use pdfium_bundled::pdfium_render::prelude::{
    PdfBitmap, PdfBitmapFormat, PdfRenderConfig, Pdfium, PdfiumError,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

const MAX_RENDER_HEIGHT: i32 = 8_192;

#[derive(Debug, Error)]
pub enum PdfError {
    #[error("file does not exist: {0}")]
    MissingFile(String),
    #[error("not a PDF path: {0}")]
    NotPdf(String),
    #[error("PDF load failed: {0}")]
    Load(String),
    #[error("PDF render failed: {0}")]
    Render(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PdfPageInfo {
    pub index: usize,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PdfBookmark {
    pub title: String,
    pub page_index: Option<usize>,
    pub depth: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PdfDocumentSummary {
    pub page_count: usize,
    pub pages: Vec<PdfPageInfo>,
    pub bookmarks: Vec<PdfBookmark>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderedPage {
    pub page_index: usize,
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

impl RenderedPage {
    pub fn is_valid_rgba_buffer(&self) -> bool {
        valid_rgba_buffer(self.width, self.height, self.rgba.len())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderedTile {
    pub page_index: usize,
    pub full_width: usize,
    pub full_height: usize,
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

impl RenderedTile {
    pub fn is_valid_rgba_buffer(&self) -> bool {
        valid_rgba_buffer(self.width, self.height, self.rgba.len())
    }

    pub fn contains(&self, other: &TileRequest) -> bool {
        self.page_index == other.page_index
            && self.full_width == other.full_width
            && self.full_height == other.full_height
            && self.x <= other.x
            && self.y <= other.y
            && self.x + self.width >= other.x + other.width
            && self.y + self.height >= other.y + other.height
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileRequest {
    pub page_index: usize,
    pub full_width: usize,
    pub full_height: usize,
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

fn valid_rgba_buffer(width: usize, height: usize, byte_len: usize) -> bool {
    width > 0 && height > 0 && byte_len == width * height * 4
}

pub trait PdfEngine {
    fn inspect(&self, path: &Path) -> Result<PdfDocumentSummary, PdfError>;
}

pub trait PdfRenderEngine {
    fn render_page(
        &self,
        path: &Path,
        page_index: usize,
        target_width: u16,
    ) -> Result<RenderedPage, PdfError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct LopdfInspectionEngine;

#[derive(Debug, Default, Clone, Copy)]
pub struct PdfiumRenderEngine;

impl PdfEngine for LopdfInspectionEngine {
    fn inspect(&self, path: &Path) -> Result<PdfDocumentSummary, PdfError> {
        validate_pdf_path(path)?;
        let doc = Document::load(path).map_err(|err| PdfError::Load(err.to_string()))?;
        let page_map = doc.get_pages();
        let page_count = page_map.len();
        let pages = (0..page_count)
            .map(|index| PdfPageInfo {
                index,
                label: Some(format!("Page {}", index + 1)),
            })
            .collect();
        let page_index_by_id = page_map
            .iter()
            .map(|(page_number, object_id)| (*object_id, (*page_number as usize).saturating_sub(1)))
            .collect();
        let bookmarks = extract_bookmarks(&doc, &page_index_by_id);
        let title = doc.trailer.get(b"Info").ok().and_then(|_| None);
        Ok(PdfDocumentSummary {
            page_count,
            pages,
            bookmarks,
            title,
        })
    }
}

fn extract_bookmarks(
    doc: &Document,
    page_index_by_id: &HashMap<ObjectId, usize>,
) -> Vec<PdfBookmark> {
    let Ok(catalog) = doc.catalog() else {
        return Vec::new();
    };
    let Ok(outlines_id) = catalog.get(b"Outlines").and_then(Object::as_reference) else {
        return Vec::new();
    };
    let Ok(outlines) = doc.get_object(outlines_id).and_then(Object::as_dict) else {
        return Vec::new();
    };
    let Ok(first_id) = outlines.get(b"First").and_then(Object::as_reference) else {
        return Vec::new();
    };

    let mut bookmarks = Vec::new();
    walk_outline_siblings(doc, first_id, 0, page_index_by_id, &mut bookmarks);
    bookmarks
}

fn walk_outline_siblings(
    doc: &Document,
    first_id: ObjectId,
    depth: usize,
    page_index_by_id: &HashMap<ObjectId, usize>,
    bookmarks: &mut Vec<PdfBookmark>,
) {
    let mut current_id = Some(first_id);
    let mut guard = 0usize;

    while let Some(item_id) = current_id {
        guard += 1;
        if guard > 10_000 {
            break;
        }

        let Ok(item) = doc.get_object(item_id).and_then(Object::as_dict) else {
            break;
        };

        if let Some(title) = item.get(b"Title").ok().and_then(pdf_string_to_utf8) {
            bookmarks.push(PdfBookmark {
                title,
                page_index: outline_target_page(item.get(b"Dest").ok(), page_index_by_id),
                depth,
            });
        }

        if let Ok(child_id) = item.get(b"First").and_then(Object::as_reference) {
            walk_outline_siblings(doc, child_id, depth + 1, page_index_by_id, bookmarks);
        }

        current_id = item.get(b"Next").and_then(Object::as_reference).ok();
    }
}

fn outline_target_page(
    dest: Option<&Object>,
    page_index_by_id: &HashMap<ObjectId, usize>,
) -> Option<usize> {
    let dest = dest?;
    match dest {
        Object::Array(items) => items
            .first()
            .and_then(|item| item.as_reference().ok())
            .and_then(|page_id| page_index_by_id.get(&page_id).copied()),
        Object::Reference(page_id) => page_index_by_id.get(page_id).copied(),
        _ => None,
    }
}

fn pdf_string_to_utf8(object: &Object) -> Option<String> {
    let bytes = object.as_str().ok()?;
    let title = String::from_utf8_lossy(bytes).trim().to_owned();
    (!title.is_empty()).then_some(title)
}

impl PdfiumRenderEngine {
    pub fn render_tile(&self, path: &Path, request: TileRequest) -> Result<RenderedTile, PdfError> {
        validate_pdf_path(path)?;
        if request.width == 0 || request.height == 0 {
            return Err(PdfError::Render(
                "tile request has empty dimensions".to_owned(),
            ));
        }
        let pdfium = bind_bundled_pdfium()?;
        let document = pdfium
            .load_pdf_from_file(path, None)
            .map_err(map_pdfium_load_error)?;
        let pages = document.pages();
        if request.page_index >= pages.len() as usize {
            return Err(PdfError::Render(format!(
                "page {} is outside document page count {}",
                request.page_index + 1,
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
        let render_config = PdfRenderConfig::new()
            .set_target_width(request.full_width as i32)
            .set_maximum_height(request.full_height as i32)
            .set_origin(-(request.x as i32), -(request.y as i32))
            .render_form_data(true)
            .render_annotations(true);
        page.render_into_bitmap_with_config(&mut bitmap, &render_config)
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
            return Err(PdfError::Render(format!(
                "renderer returned invalid tile RGBA buffer: {}x{} with {} bytes",
                rendered.width,
                rendered.height,
                rendered.rgba.len()
            )));
        }
        Ok(rendered)
    }
}

impl PdfRenderEngine for PdfiumRenderEngine {
    fn render_page(
        &self,
        path: &Path,
        page_index: usize,
        target_width: u16,
    ) -> Result<RenderedPage, PdfError> {
        validate_pdf_path(path)?;
        let pdfium = bind_bundled_pdfium()?;
        let document = pdfium
            .load_pdf_from_file(path, None)
            .map_err(map_pdfium_load_error)?;
        let pages = document.pages();
        if page_index >= pages.len() as usize {
            return Err(PdfError::Render(format!(
                "page {} is outside document page count {}",
                page_index + 1,
                pages.len()
            )));
        }
        let page = pages
            .get(page_index as i32)
            .map_err(|err| PdfError::Render(err.to_string()))?;
        let target_width = target_width.max(64);
        let render_config = PdfRenderConfig::new()
            .set_target_width(target_width as i32)
            .set_maximum_height(MAX_RENDER_HEIGHT)
            .render_form_data(true)
            .render_annotations(true);
        let bitmap = page
            .render_with_config(&render_config)
            .map_err(|err| PdfError::Render(err.to_string()))?;
        let rendered = RenderedPage {
            page_index,
            width: bitmap.width() as usize,
            height: bitmap.height() as usize,
            rgba: bitmap.as_rgba_bytes(),
        };
        if !rendered.is_valid_rgba_buffer() {
            return Err(PdfError::Render(format!(
                "renderer returned invalid RGBA buffer: {}x{} with {} bytes",
                rendered.width,
                rendered.height,
                rendered.rgba.len()
            )));
        }
        Ok(rendered)
    }
}

fn bind_bundled_pdfium() -> Result<Pdfium, PdfError> {
    match pdfium_bundled::bind_bundled() {
        Ok(pdfium) => Ok(pdfium),
        Err(pdfium_bundled::Error::Bind { reason, .. })
            if reason.contains("PdfiumLibraryBindingsAlreadyInitialized") =>
        {
            Ok(Pdfium::default())
        }
        Err(err) => Err(PdfError::Render(err.to_string())),
    }
}

fn validate_pdf_path(path: &Path) -> Result<(), PdfError> {
    if !path.exists() {
        return Err(PdfError::MissingFile(path.display().to_string()));
    }
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| !ext.eq_ignore_ascii_case("pdf"))
        .unwrap_or(true)
    {
        return Err(PdfError::NotPdf(path.display().to_string()));
    }
    Ok(())
}

fn map_pdfium_load_error(err: PdfiumError) -> PdfError {
    PdfError::Load(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn rejects_missing_files() {
        let engine = LopdfInspectionEngine;
        let err = engine
            .inspect(&PathBuf::from("/does/not/exist.pdf"))
            .unwrap_err();
        assert!(matches!(err, PdfError::MissingFile(_)));
    }

    #[test]
    fn rejects_non_pdf_extensions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, "not a pdf").unwrap();
        let engine = LopdfInspectionEngine;
        let err = engine.inspect(&path).unwrap_err();
        assert!(matches!(err, PdfError::NotPdf(_)));
    }

    #[test]
    fn rendered_page_buffer_validation_checks_rgba_size() {
        let ok = RenderedPage {
            page_index: 0,
            width: 2,
            height: 2,
            rgba: vec![255; 16],
        };
        assert!(ok.is_valid_rgba_buffer());

        let bad = RenderedPage {
            page_index: 0,
            width: 2,
            height: 2,
            rgba: vec![255; 15],
        };
        assert!(!bad.is_valid_rgba_buffer());
    }

    #[test]
    fn pdfium_renders_a_real_pdf_page_to_rgba_and_reuses_existing_binding() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("glyph-smoke.pdf");
        std::fs::write(&path, minimal_pdf_bytes()).unwrap();

        let rendered = PdfiumRenderEngine.render_page(&path, 0, 320).unwrap();
        let rendered_again = PdfiumRenderEngine.render_page(&path, 0, 320).unwrap();

        assert_eq!(rendered.page_index, 0);
        assert!(rendered.width >= 300);
        assert!(rendered.height >= 300);
        assert!(rendered.is_valid_rgba_buffer());
        assert!(rendered_again.is_valid_rgba_buffer());
        assert!(
            rendered
                .rgba
                .chunks_exact(4)
                .any(|pixel| pixel != [255, 255, 255, 255])
        );
    }

    #[test]
    fn pdfium_renders_viewport_tile_at_high_resolution() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("glyph-tile.pdf");
        std::fs::write(&path, minimal_pdf_bytes()).unwrap();

        let request = TileRequest {
            page_index: 0,
            full_width: 3_200,
            full_height: 3_200,
            x: 500,
            y: 1_400,
            width: 512,
            height: 512,
        };
        let tile = PdfiumRenderEngine.render_tile(&path, request).unwrap();

        assert_eq!(tile.page_index, 0);
        assert_eq!(tile.full_width, 3_200);
        assert_eq!(tile.full_height, 3_200);
        assert_eq!(tile.x, 500);
        assert_eq!(tile.y, 1_400);
        assert!(tile.is_valid_rgba_buffer());
        assert!(tile.contains(&request));
        assert!(
            tile.rgba
                .chunks_exact(4)
                .any(|pixel| pixel != [255, 255, 255, 255])
        );
    }

    #[test]
    fn inspection_extracts_pdf_outline_bookmarks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("glyph-outline.pdf");
        std::fs::write(&path, minimal_pdf_with_outline_bytes()).unwrap();

        let summary = LopdfInspectionEngine.inspect(&path).unwrap();

        assert_eq!(summary.page_count, 1);
        assert_eq!(summary.bookmarks.len(), 1);
        assert_eq!(summary.bookmarks[0].title, "A-101 Floor Plan");
        assert_eq!(summary.bookmarks[0].page_index, Some(0));
        assert_eq!(summary.bookmarks[0].depth, 0);
    }

    fn minimal_pdf_bytes() -> Vec<u8> {
        let objects = [
            "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
            "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
            "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>\nendobj\n",
            "4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
            "5 0 obj\n<< /Length 41 >>\nstream\nBT /F1 24 Tf 50 110 Td (Glyph) Tj ET\nendstream\nendobj\n",
        ];
        let mut pdf = String::from("%PDF-1.4\n");
        let mut offsets = vec![0usize];
        for object in objects {
            offsets.push(pdf.len());
            pdf.push_str(object);
        }
        let xref_offset = pdf.len();
        pdf.push_str("xref\n0 6\n0000000000 65535 f \n");
        for offset in offsets.iter().skip(1) {
            pdf.push_str(&format!("{offset:010} 00000 n \n"));
        }
        pdf.push_str(&format!(
            "trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n"
        ));
        pdf.into_bytes()
    }

    fn minimal_pdf_with_outline_bytes() -> Vec<u8> {
        let objects = [
            "1 0 obj\n<< /Type /Catalog /Pages 2 0 R /Outlines 6 0 R >>\nendobj\n",
            "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
            "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>\nendobj\n",
            "4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
            "5 0 obj\n<< /Length 41 >>\nstream\nBT /F1 24 Tf 50 110 Td (Glyph) Tj ET\nendstream\nendobj\n",
            "6 0 obj\n<< /Type /Outlines /First 7 0 R /Last 7 0 R /Count 1 >>\nendobj\n",
            "7 0 obj\n<< /Title (A-101 Floor Plan) /Parent 6 0 R /Dest [3 0 R /Fit] >>\nendobj\n",
        ];
        let mut pdf = String::from("%PDF-1.4\n");
        let mut offsets = vec![0usize];
        for object in objects {
            offsets.push(pdf.len());
            pdf.push_str(object);
        }
        let xref_offset = pdf.len();
        pdf.push_str("xref\n0 8\n0000000000 65535 f \n");
        for offset in offsets.iter().skip(1) {
            pdf.push_str(&format!("{offset:010} 00000 n \n"));
        }
        pdf.push_str(&format!(
            "trailer\n<< /Size 8 /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n"
        ));
        pdf.into_bytes()
    }
}
