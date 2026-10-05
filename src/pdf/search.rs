use crate::core::links::PdfRect;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Maximum number of matches retained by a single search.
pub const MAX_SEARCH_HITS: usize = 1_000;

#[derive(Debug, Clone, PartialEq)]
pub struct PdfSearchHit {
    /// Zero-based document page index.
    pub page_index: usize,
    /// Trimmed PDFium text from the matched bounds (a display snippet).
    pub text: String,
    /// PDF user-space points, with a bottom-left origin; not normalized screen bounds.
    /// Use the page's PDFium points-to-pixels transform for crop/rotation-aware overlays.
    pub rects: Vec<PdfRect>,
}

/// Search all pages in reading order for a case-insensitive literal substring.
///
/// Blank queries and pre-cancelled requests return no hits without opening the file.
/// Cancellation returns accumulated hits; callers can discard them when cancelling.
/// Progress is `(completed_pages, total_pages)`, initially `(0, total_pages)` and
/// then once per completely searched page. Cancellation or the hit cap may stop
/// progress short of `total_pages`. PDFium calls themselves cannot be interrupted.
pub fn search_pdf(
    path: &Path,
    query: &str,
    cancel: &AtomicBool,
    progress: impl Fn(usize, usize),
) -> Result<Vec<PdfSearchHit>, super::PdfError> {
    use super::PdfError;
    use pdfium_bundled::pdfium_render::prelude::PdfSearchOptions;

    if query.trim().is_empty() || cancel.load(Ordering::Relaxed) {
        return Ok(Vec::new());
    }
    super::validate_pdf_path(path)?;
    let pdfium = super::bind_bundled_pdfium()?;
    let document = pdfium
        .load_pdf_from_file(path, None)
        .map_err(|err| PdfError::Load(err.to_string()))?;
    let total = document.pages().len() as usize;
    let mut hits = Vec::new();
    progress(0, total);
    for page_index in 0..total {
        if cancel.load(Ordering::Relaxed) {
            return Ok(hits);
        }
        let page = document
            .pages()
            .get(page_index as i32)
            .map_err(|err| PdfError::Load(err.to_string()))?;
        let text = page.text().map_err(|err| PdfError::Load(err.to_string()))?;
        let search = text
            .search(query, &PdfSearchOptions::new())
            .map_err(|err| PdfError::Load(err.to_string()))?;
        while !cancel.load(Ordering::Relaxed) {
            let Some(segments) = search.find_next() else {
                break;
            };
            let mut rects = Vec::new();
            let mut matched = String::new();
            for index in 0..segments.len() {
                if cancel.load(Ordering::Relaxed) {
                    return Ok(hits);
                }
                let segment = segments
                    .get(index)
                    .map_err(|err| PdfError::Load(err.to_string()))?;
                matched.push_str(&segment.text());
                let bounds = segment.bounds();
                rects.push(PdfRect {
                    x: bounds.left().value,
                    y: bounds.bottom().value,
                    width: bounds.width().value,
                    height: bounds.height().value,
                });
            }
            hits.push(PdfSearchHit {
                page_index,
                text: matched.trim().to_owned(),
                rects,
            });
            if hits.len() >= MAX_SEARCH_HITS {
                return Ok(hits);
            }
        }
        if cancel.load(Ordering::Relaxed) {
            return Ok(hits);
        }
        progress(page_index + 1, total);
    }
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::content::{Content, Operation};
    use lopdf::{Document, Object, Stream, dictionary};
    use std::cell::RefCell;

    fn fixture(pages: &[&[u8]]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("search.pdf");
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
            "Encoding" => "WinAnsiEncoding",
        });
        let resources = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font } });
        let mut kids = Vec::new();
        for bytes in pages {
            let content = Content {
                operations: vec![
                    Operation::new("BT", vec![]),
                    Operation::new("Tf", vec!["F1".into(), 12.into()]),
                    Operation::new("Td", vec![72.into(), 700.into()]),
                    Operation::new(
                        "Tj",
                        vec![Object::String(bytes.to_vec(), lopdf::StringFormat::Literal)],
                    ),
                    Operation::new("ET", vec![]),
                ],
            }
            .encode()
            .unwrap();
            let stream = doc.add_object(Stream::new(dictionary! {}, content));
            let page = doc.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages_id, "Contents" => stream,
                "Resources" => resources, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            });
            kids.push(Object::Reference(page));
        }
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Count" => kids.len() as i64, "Kids" => kids,
            }),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog);
        doc.save(&path).unwrap();
        (dir, path)
    }

    #[test]
    fn pre_cancelled_search_returns_without_opening_a_document() {
        let result = search_pdf(
            Path::new("/missing/search.pdf"),
            "alpha",
            &AtomicBool::new(true),
            |_, _| panic!("cancelled search reported progress"),
        );
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn cancellation_after_a_page_preserves_partial_hits_without_visiting_next_page() {
        let (_dir, path) = fixture(&[b"alpha", b"alpha", b"alpha"]);
        let cancel = AtomicBool::new(false);
        let progress = RefCell::new(Vec::new());
        let hits = search_pdf(&path, "alpha", &cancel, |done, total| {
            progress.borrow_mut().push((done, total));
            if done == 1 {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        })
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(*progress.borrow(), vec![(0, 3), (1, 3)]);
    }

    #[test]
    fn blank_queries_do_not_open_document_or_report_progress() {
        for query in ["", " \t\n", "\u{2003}"] {
            let hits = search_pdf(
                Path::new("/missing/search.pdf"),
                query,
                &AtomicBool::new(false),
                |_, _| panic!("blank query reported progress"),
            )
            .unwrap();
            assert!(hits.is_empty());
        }
    }

    #[test]
    fn limits_results_without_searching_remaining_pages() {
        let many = b"a ".repeat(1_001);
        let (_dir, path) = fixture(&[&many, b"a"]);
        let progress = RefCell::new(Vec::new());
        let hits = search_pdf(&path, "a", &AtomicBool::new(false), |done, total| {
            progress.borrow_mut().push((done, total));
        })
        .unwrap();
        assert_eq!(hits.len(), 1_000);
        assert_eq!(*progress.borrow(), vec![(0, 2)]);
    }

    #[test]
    fn finds_unicode_text_using_pdfium_unicode_search() {
        let (_dir, path) = fixture(&[b"caf\xe9 CAF\xc9"]);
        let hits = search_pdf(&path, "café", &AtomicBool::new(false), |_, _| {}).unwrap();
        assert_eq!(
            hits.iter().map(|hit| hit.text.as_str()).collect::<Vec<_>>(),
            vec!["café", "CAFÉ"]
        );
        assert!(hits.iter().all(|hit| !hit.rects.is_empty()));
    }

    #[test]
    fn treats_regex_metacharacters_as_literal_text() {
        let (_dir, path) = fixture(&[b"a.b axb a[b] a+b"]);
        for query in ["a.b", "a[b]", "a+b"] {
            let hits = search_pdf(&path, query, &AtomicBool::new(false), |_, _| {}).unwrap();
            assert_eq!(hits.len(), 1, "query: {query}");
            assert_eq!(hits[0].text, query);
        }
    }

    #[test]
    fn misses_still_report_each_completed_page() {
        let (_dir, path) = fixture(&[b"alpha", b"beta"]);
        let progress = RefCell::new(Vec::new());
        let hits = search_pdf(&path, "missing", &AtomicBool::new(false), |done, total| {
            progress.borrow_mut().push((done, total));
        })
        .unwrap();
        assert!(hits.is_empty());
        assert_eq!(*progress.borrow(), vec![(0, 2), (1, 2), (2, 2)]);
    }

    #[test]
    fn non_blank_queries_validate_path_and_propagate_load_errors() {
        assert!(matches!(
            search_pdf(
                Path::new("/missing/search.pdf"),
                "a",
                &AtomicBool::new(false),
                |_, _| {}
            ),
            Err(super::super::PdfError::MissingFile(_))
        ));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken.pdf");
        std::fs::write(&path, b"not a PDF").unwrap();
        assert!(matches!(
            search_pdf(&path, "a", &AtomicBool::new(false), |_, _| {}),
            Err(super::super::PdfError::Load(_))
        ));
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, b"not a PDF").unwrap();
        assert!(matches!(
            search_pdf(&path, "a", &AtomicBool::new(false), |_, _| {}),
            Err(super::super::PdfError::NotPdf(_))
        ));
    }

    #[test]
    fn finds_case_insensitive_substrings_across_pages_with_pdf_bounds() {
        let (_dir, path) = fixture(&[b"Alpha alphabet ALPHA", b"unrelated", b"alpha"]);
        let progress = RefCell::new(Vec::new());
        let hits = search_pdf(&path, "aLpHa", &AtomicBool::new(false), |done, total| {
            progress.borrow_mut().push((done, total));
        })
        .unwrap();
        assert_eq!(hits.len(), 4);
        assert_eq!(
            hits.iter().map(|hit| hit.page_index).collect::<Vec<_>>(),
            vec![0, 0, 0, 2]
        );
        assert_eq!(
            hits.iter().map(|hit| hit.text.as_str()).collect::<Vec<_>>(),
            vec!["Alpha", "alpha", "ALPHA", "alpha"]
        );
        for hit in &hits {
            assert!(!hit.rects.is_empty());
            for rect in &hit.rects {
                assert!(rect.is_valid(), "{rect:?}");
                assert!(rect.x >= 72.0 && rect.x + rect.width <= 612.0);
                assert!(rect.y >= 695.0 && rect.y + rect.height <= 715.0);
            }
        }
        assert_eq!(*progress.borrow(), vec![(0, 3), (1, 3), (2, 3), (3, 3)]);
    }
}
