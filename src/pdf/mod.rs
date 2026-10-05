pub use automation::SheetAnalysis;
mod automation;
pub mod overlay;
pub mod search;
mod session;
mod text;
use crate::core::links::LinkProposal;
use crate::core::sheet::SheetCandidate;
use lopdf::{Document, Object, ObjectId, dictionary};
use pdfium_bundled::pdfium_render::prelude::{
    PdfBitmap, PdfBitmapFormat, PdfRenderConfig, Pdfium, PdfiumError,
};
use serde::{Deserialize, Serialize};
pub use session::{PdfiumSession, bind_render_pdfium};
use std::collections::HashMap;
use std::path::Path;
pub use text::{PageText, TextGlyph};
use thiserror::Error;

mod links;
pub use links::InternalLinkIndex;
pub use links::PdfInternalLink;
// Retain the stateless extraction API for non-worker callers.
#[allow(unused_imports)]
pub use links::extract_internal_links;

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
    #[error("PDF edit failed: {0}")]
    Edit(String),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfEditReport {
    pub output_path: std::path::PathBuf,
    pub bookmarks_written: usize,
    pub links_written: usize,
    pub annotations_removed: usize,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct LopdfEditEngine;

impl LopdfEditEngine {
    pub fn write_bookmarks(
        &self,
        input: &Path,
        output: &Path,
        bookmarks: &[SheetCandidate],
    ) -> Result<PdfEditReport, PdfError> {
        validate_pdf_path(input)?;
        let mut doc = Document::load(input).map_err(|err| PdfError::Load(err.to_string()))?;
        let written = install_bookmark_outline(&mut doc, bookmarks)?;
        doc.save(output)
            .map_err(|err| PdfError::Edit(err.to_string()))?;
        Ok(PdfEditReport {
            output_path: output.to_path_buf(),
            bookmarks_written: written,
            links_written: 0,
            annotations_removed: 0,
        })
    }

    pub fn write_links(
        &self,
        input: &Path,
        output: &Path,
        links: &[LinkProposal],
    ) -> Result<PdfEditReport, PdfError> {
        validate_pdf_path(input)?;
        let mut doc = Document::load(input).map_err(|err| PdfError::Load(err.to_string()))?;
        let written = install_link_annotations(&mut doc, links)?;
        doc.save(output)
            .map_err(|err| PdfError::Edit(err.to_string()))?;
        Ok(PdfEditReport {
            output_path: output.to_path_buf(),
            bookmarks_written: 0,
            links_written: written,
            annotations_removed: 0,
        })
    }

    pub fn flatten_interactive_annotations(
        &self,
        input: &Path,
        output: &Path,
    ) -> Result<PdfEditReport, PdfError> {
        validate_pdf_path(input)?;
        let mut doc = Document::load(input).map_err(|err| PdfError::Load(err.to_string()))?;
        let removed = remove_interactive_annotations(&mut doc)?;
        doc.save(output)
            .map_err(|err| PdfError::Edit(err.to_string()))?;
        Ok(PdfEditReport {
            output_path: output.to_path_buf(),
            bookmarks_written: 0,
            links_written: 0,
            annotations_removed: removed,
        })
    }
}

fn install_bookmark_outline(
    doc: &mut Document,
    bookmarks: &[SheetCandidate],
) -> Result<usize, PdfError> {
    let pages = doc.get_pages();
    let mut items = Vec::new();
    for bookmark in bookmarks {
        let page_number = (bookmark.page_index + 1) as u32;
        let Some(page_id) = pages.get(&page_number).copied() else {
            continue;
        };
        let title = bookmark
            .title
            .as_deref()
            .filter(|title| !title.trim().is_empty())
            .unwrap_or(&bookmark.id.0)
            .trim()
            .to_owned();
        if title.is_empty() {
            continue;
        }
        items.push((title, page_id));
    }
    if items.is_empty() {
        return Ok(0);
    }

    let outlines_id = doc.new_object_id();
    let item_ids: Vec<ObjectId> = (0..items.len()).map(|_| doc.new_object_id()).collect();
    for (index, ((title, page_id), item_id)) in items.iter().zip(item_ids.iter()).enumerate() {
        let mut item = dictionary! {
            "Title" => Object::string_literal(title.as_str()),
            "Parent" => Object::Reference(outlines_id),
            "Dest" => Object::Array(vec![Object::Reference(*page_id), Object::Name(b"Fit".to_vec())]),
        };
        if index > 0 {
            item.set("Prev", Object::Reference(item_ids[index - 1]));
        }
        if index + 1 < item_ids.len() {
            item.set("Next", Object::Reference(item_ids[index + 1]));
        }
        doc.objects.insert(*item_id, Object::Dictionary(item));
    }

    let outlines = dictionary! {
        "Type" => Object::Name(b"Outlines".to_vec()),
        "First" => Object::Reference(item_ids[0]),
        "Last" => Object::Reference(*item_ids.last().unwrap()),
        "Count" => Object::Integer(item_ids.len() as i64),
    };
    doc.objects
        .insert(outlines_id, Object::Dictionary(outlines));
    doc.catalog_mut()
        .map_err(|err| PdfError::Edit(err.to_string()))?
        .set("Outlines", Object::Reference(outlines_id));
    Ok(item_ids.len())
}

fn install_link_annotations(doc: &mut Document, links: &[LinkProposal]) -> Result<usize, PdfError> {
    let pages = doc.get_pages();
    let mut written = 0usize;
    let mut installed = HashMap::<ObjectId, std::collections::HashSet<(ObjectId, [u32; 4])>>::new();
    for page_id in pages.values() {
        let annotations = doc
            .get_object(*page_id)
            .ok()
            .and_then(|o| o.as_dict().ok())
            .and_then(|p| p.get(b"Annots").ok())
            .and_then(|o| match o {
                Object::Reference(id) => doc.get_object(*id).ok(),
                _ => Some(o),
            })
            .and_then(|o| o.as_array().ok());
        let keys = annotations
            .into_iter()
            .flatten()
            .filter_map(|o| {
                let o = match o {
                    Object::Reference(id) => doc.get_object(*id).ok()?,
                    _ => o,
                };
                let a = o.as_dict().ok()?;
                if a.get(b"GlyphGenerated").and_then(Object::as_bool).ok() != Some(true) {
                    return None;
                }
                let target = a
                    .get(b"Dest")
                    .ok()?
                    .as_array()
                    .ok()?
                    .first()?
                    .as_reference()
                    .ok()?;
                let rect = a.get(b"Rect").ok()?.as_array().ok()?;
                let coords: Vec<_> = rect
                    .iter()
                    .filter_map(|o| o.as_float().ok().map(f32::to_bits))
                    .collect();
                let coords: [u32; 4] = coords.try_into().ok()?;
                Some((target, coords))
            })
            .collect();
        installed.insert(*page_id, keys);
    }
    for link in links.iter().filter(|link| link.is_actionable()) {
        let Some(page_id) = pages.get(&((link.from_page + 1) as u32)).copied() else {
            continue;
        };
        let Some(target_page_id) = pages.get(&((link.target_page + 1) as u32)).copied() else {
            continue;
        };
        let rect = &link.rect;
        let key = (
            target_page_id,
            [rect.x, rect.y, rect.x + rect.width, rect.y + rect.height].map(f32::to_bits),
        );
        if !installed.entry(page_id).or_default().insert(key) {
            continue;
        }
        let annot_id = doc.new_object_id();
        let annotation = dictionary! {
            "GlyphGenerated" => Object::Boolean(true),
            "Type" => Object::Name(b"Annot".to_vec()),
            "Subtype" => Object::Name(b"Link".to_vec()),
            "Rect" => Object::Array(vec![
                Object::Real(rect.x),
                Object::Real(rect.y),
                Object::Real(rect.x + rect.width),
                Object::Real(rect.y + rect.height),
            ]),
            "Border" => Object::Array(vec![Object::Integer(0), Object::Integer(0), Object::Integer(0)]),
            "Dest" => Object::Array(vec![Object::Reference(target_page_id), Object::Name(b"Fit".to_vec())]),
            "Contents" => Object::string_literal(link.label.as_str()),
        };
        doc.objects.insert(annot_id, Object::Dictionary(annotation));
        append_annotation(doc, page_id, annot_id)?;
        written += 1;
    }
    Ok(written)
}

fn append_annotation(
    doc: &mut Document,
    page_id: ObjectId,
    annot_id: ObjectId,
) -> Result<(), PdfError> {
    let indirect = doc
        .get_object(page_id)
        .and_then(Object::as_dict)
        .map_err(|err| PdfError::Edit(err.to_string()))?
        .get(b"Annots")
        .ok()
        .and_then(|o| o.as_reference().ok());
    if let Some(id) = indirect {
        let items = doc
            .get_object_mut(id)
            .and_then(Object::as_array_mut)
            .map_err(|err| PdfError::Edit(format!("Invalid indirect annotation array: {err}")))?;
        items.push(Object::Reference(annot_id));
        return Ok(());
    }
    let page = doc
        .get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|err| PdfError::Edit(err.to_string()))?;
    match page.get_mut(b"Annots") {
        Ok(Object::Array(items)) => items.push(Object::Reference(annot_id)),
        Ok(existing) => {
            let old = existing.clone();
            *existing = Object::Array(vec![old, Object::Reference(annot_id)]);
        }
        Err(_) => {
            page.set("Annots", Object::Array(vec![Object::Reference(annot_id)]));
        }
    }
    Ok(())
}

fn remove_interactive_annotations(doc: &mut Document) -> Result<usize, PdfError> {
    let page_ids: Vec<ObjectId> = doc.get_pages().values().copied().collect();
    let mut removed = 0usize;
    for page_id in page_ids {
        let page = doc
            .get_object_mut(page_id)
            .and_then(Object::as_dict_mut)
            .map_err(|err| PdfError::Edit(err.to_string()))?;
        if page.remove(b"Annots").is_some() {
            removed += 1;
        }
    }
    Ok(removed)
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
        let title = None;
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
                page_index: outline_item_target_page(doc, item, page_index_by_id),
                depth,
            });
        }

        if let Ok(child_id) = item.get(b"First").and_then(Object::as_reference) {
            walk_outline_siblings(doc, child_id, depth + 1, page_index_by_id, bookmarks);
        }

        current_id = item.get(b"Next").and_then(Object::as_reference).ok();
    }
}

fn outline_item_target_page(
    doc: &Document,
    item: &lopdf::Dictionary,
    page_index_by_id: &HashMap<ObjectId, usize>,
) -> Option<usize> {
    if let Some(page_index) = outline_target_page(item.get(b"Dest").ok(), page_index_by_id) {
        return Some(page_index);
    }

    let action = item.get(b"A").ok().and_then(resolve_dict_object(doc));
    let dest = action.and_then(|action| action.get(b"D").ok());
    outline_target_page(dest, page_index_by_id)
}

fn outline_target_page(
    dest: Option<&Object>,
    page_index_by_id: &HashMap<ObjectId, usize>,
) -> Option<usize> {
    let dest = dest?;
    match dest {
        Object::Array(items) => items
            .first()
            .and_then(page_object_id)
            .and_then(|page_id| page_index_by_id.get(&page_id).copied()),
        Object::Reference(page_id) => page_index_by_id.get(page_id).copied(),
        _ => None,
    }
}

fn page_object_id(object: &Object) -> Option<ObjectId> {
    match object {
        Object::Reference(page_id) => Some(*page_id),
        Object::Dictionary(_) => None,
        _ => None,
    }
}

fn resolve_dict_object<'a>(
    doc: &'a Document,
) -> impl Fn(&'a Object) -> Option<&'a lopdf::Dictionary> {
    move |object| match object {
        Object::Dictionary(dict) => Some(dict),
        Object::Reference(id) => doc.get_object(*id).and_then(Object::as_dict).ok(),
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

    pub fn generate_sheet_label_links(
        &self,
        path: &Path,
        sheets: &[SheetCandidate],
    ) -> Result<Vec<LinkProposal>, PdfError> {
        automation::generate_links(path, sheets)
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
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| *pixel != [255, 255, 255, 255])
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
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| *pixel != [255, 255, 255, 255])
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

    #[test]
    fn inspection_extracts_action_bookmark_destinations() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("glyph-action-outline.pdf");
        std::fs::write(&path, minimal_pdf_with_action_outline_bytes()).unwrap();

        let summary = LopdfInspectionEngine.inspect(&path).unwrap();

        assert_eq!(summary.page_count, 1);
        assert_eq!(summary.bookmarks.len(), 1);
        assert_eq!(summary.bookmarks[0].title, "A-102 Enlarged Plan");
        assert_eq!(summary.bookmarks[0].page_index, Some(0));
    }

    #[test]
    fn edit_engine_writes_generated_bookmark_outline() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.pdf");
        let output = dir.path().join("bookmarked.pdf");
        std::fs::write(&input, minimal_pdf_bytes()).unwrap();
        let bookmarks = vec![SheetCandidate {
            id: crate::core::sheet::SheetId("A-101".to_owned()),
            title: Some("A-101 Floor Plan".to_owned()),
            page_index: 0,
            confidence: 95,
        }];

        let report = LopdfEditEngine
            .write_bookmarks(&input, &output, &bookmarks)
            .unwrap();
        let summary = LopdfInspectionEngine.inspect(&output).unwrap();

        assert_eq!(report.bookmarks_written, 1);
        assert_eq!(summary.bookmarks.len(), 1);
        assert_eq!(summary.bookmarks[0].title, "A-101 Floor Plan");
        assert_eq!(summary.bookmarks[0].page_index, Some(0));
    }

    #[test]
    fn edit_engine_writes_link_annotations() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.pdf");
        let output = dir.path().join("linked.pdf");
        std::fs::write(&input, minimal_pdf_bytes()).unwrap();
        std::fs::write(&input, minimal_two_page_pdf_bytes()).unwrap();
        let links = vec![LinkProposal {
            from_page: 0,
            target_page: 1,
            rect: crate::core::links::PdfRect {
                x: 20.0,
                y: 30.0,
                width: 40.0,
                height: 12.0,
            },
            label: "A-102".to_owned(),
        }];

        let report = LopdfEditEngine
            .write_links(&input, &output, &links)
            .unwrap();
        let doc = Document::load(&output).unwrap();
        let page_id = *doc.get_pages().get(&1).unwrap();
        let page = doc.get_object(page_id).unwrap().as_dict().unwrap();
        let annots = page.get(b"Annots").unwrap().as_array().unwrap();

        assert_eq!(report.links_written, 1);
        assert_eq!(annots.len(), 1);
    }

    #[test]
    fn flatten_command_removes_interactive_page_annotations() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.pdf");
        let linked = dir.path().join("linked.pdf");
        let flattened = dir.path().join("flattened.pdf");
        std::fs::write(&input, minimal_two_page_pdf_bytes()).unwrap();
        let links = vec![LinkProposal {
            from_page: 0,
            target_page: 1,
            rect: crate::core::links::PdfRect {
                x: 20.0,
                y: 30.0,
                width: 40.0,
                height: 12.0,
            },
            label: "A-102".to_owned(),
        }];
        LopdfEditEngine
            .write_links(&input, &linked, &links)
            .unwrap();

        let report = LopdfEditEngine
            .flatten_interactive_annotations(&linked, &flattened)
            .unwrap();
        let doc = Document::load(&flattened).unwrap();
        let page_id = *doc.get_pages().get(&1).unwrap();
        let page = doc.get_object(page_id).unwrap().as_dict().unwrap();

        assert_eq!(report.annotations_removed, 1);
        assert!(page.get(b"Annots").is_err());
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

    fn minimal_two_page_pdf_bytes() -> Vec<u8> {
        let objects = [
            "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
            "2 0 obj\n<< /Type /Pages /Kids [3 0 R 6 0 R] /Count 2 >>\nendobj\n",
            "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>\nendobj\n",
            "4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
            "5 0 obj\n<< /Length 41 >>\nstream\nBT /F1 24 Tf 50 110 Td (Page 1) Tj ET\nendstream\nendobj\n",
            "6 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << /Font << /F1 4 0 R >> >> /Contents 7 0 R >>\nendobj\n",
            "7 0 obj\n<< /Length 41 >>\nstream\nBT /F1 24 Tf 50 110 Td (Page 2) Tj ET\nendstream\nendobj\n",
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

    fn minimal_pdf_with_action_outline_bytes() -> Vec<u8> {
        let objects = [
            "1 0 obj\n<< /Type /Catalog /Pages 2 0 R /Outlines 6 0 R >>\nendobj\n",
            "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
            "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>\nendobj\n",
            "4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
            "5 0 obj\n<< /Length 41 >>\nstream\nBT /F1 24 Tf 50 110 Td (Glyph) Tj ET\nendstream\nendobj\n",
            "6 0 obj\n<< /Type /Outlines /First 7 0 R /Last 7 0 R /Count 1 >>\nendobj\n",
            "7 0 obj\n<< /Title (A-102 Enlarged Plan) /Parent 6 0 R /A << /S /GoTo /D [3 0 R /XYZ null null null] >> >>\nendobj\n",
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
