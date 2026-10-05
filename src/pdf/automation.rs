use super::{PdfError, bind_bundled_pdfium, map_pdfium_load_error, validate_pdf_path};
use crate::core::{
    links::{LinkProposal, PdfRect},
    sheet::{SheetCandidate, normalize_sheet_id},
};
use pdfium_bundled::pdfium_render::prelude::PdfPageText;
use std::{collections::HashMap, path::Path};

#[derive(Debug, Clone, PartialEq)]
pub struct SheetAnalysis {
    pub sheets: Vec<SheetCandidate>,
    pub text_page_count: usize,
}

impl super::PdfiumRenderEngine {
    pub fn analyze_sheets(&self, path: &Path) -> Result<super::SheetAnalysis, PdfError> {
        validate_pdf_path(path)?;
        let pdfium = bind_bundled_pdfium()?;
        let document = pdfium
            .load_pdf_from_file(path, None)
            .map_err(map_pdfium_load_error)?;
        let mut candidates = Vec::new();
        let mut text_page_count = 0;
        for page_index in 0..document.pages().len() as usize {
            let page = document
                .pages()
                .get(page_index as i32)
                .map_err(map_pdfium_load_error)?;
            let text = page.text().map_err(map_pdfium_load_error)?;
            let glyphs = glyphs(&text);
            if glyphs.iter().any(|g| g.ch.is_alphanumeric()) {
                text_page_count += 1;
            }
            let words = tokens(&glyphs);
            let anchors: Vec<_> = words
                .iter()
                .filter(|(word, _)| matches!(word.as_str(), "SHEET" | "DRAWING" | "SHT" | "DWG"))
                .filter_map(|(_, range)| bounds(&glyphs[range.clone()]))
                .collect();
            let media = page
                .boundaries()
                .media()
                .map_err(map_pdfium_load_error)?
                .bounds;
            let mut sizes: Vec<_> = glyphs
                .iter()
                .filter(|g| g.ch.is_alphanumeric() && g.font_size.is_finite() && g.font_size > 0.0)
                .map(|g| g.font_size)
                .collect();
            sizes.sort_by(f32::total_cmp);
            let typical = sizes.get(sizes.len() / 2).copied().unwrap_or(12.0).max(1.0);
            let mut best: Option<SheetCandidate> = None;
            let mut ambiguous = false;
            for (word, range) in &words {
                let Some(id) = normalize_sheet_id(word) else {
                    continue;
                };
                let Some(rect) = bounds(&glyphs[range.clone()]) else {
                    continue;
                };
                let x = ((rect.x + rect.width / 2.0 - media.left().value) / media.width().value)
                    .clamp(0.0, 1.0);
                let y = ((rect.y + rect.height / 2.0 - media.bottom().value)
                    / media.height().value)
                    .clamp(0.0, 1.0);
                let edge_x = x < 0.2 || x > 0.8;
                let edge_y = y < 0.2 || y > 0.8;
                let near_anchor = anchors.iter().any(|a| {
                    let dx = (rect.x + rect.width / 2.0 - a.x - a.width / 2.0).abs();
                    let dy = (rect.y + rect.height / 2.0 - a.y - a.height / 2.0).abs();
                    dx < media.width().value * 0.15 && dy < media.height().value * 0.08
                });
                let font = glyphs[range.clone()]
                    .iter()
                    .map(|g| g.font_size)
                    .fold(0.0, f32::max);
                // A corner alone is insufficient evidence: ordinary references also occur there.
                if !near_anchor && font < typical * 1.5 {
                    continue;
                }
                // Anchor proximity dominates font size: body detail references can be huge.
                let score = 15
                    + if edge_x { 15 } else { 0 }
                    + if edge_y { 15 } else { 0 }
                    + if near_anchor { 40 } else { 0 }
                    + ((font / typical).min(3.0) * 5.0) as u8;
                let score = score.min(99);
                if score < 45 {
                    continue;
                }
                if best.as_ref().is_none_or(|b| score > b.confidence) {
                    ambiguous = false;
                    best = Some(SheetCandidate {
                        title: Some(id.0.clone()),
                        id,
                        page_index,
                        confidence: score.min(99),
                    });
                } else if best
                    .as_ref()
                    .is_some_and(|b| score == b.confidence && b.id != id)
                {
                    ambiguous = true;
                }
            }
            if let Some(best) = best.filter(|_| !ambiguous) {
                candidates.push(best);
            }
        }
        Ok(SheetAnalysis {
            sheets: crate::core::sheet::generate_bookmark_titles(
                &candidates,
                document.pages().len() as usize,
            ),
            text_page_count,
        })
    }
}

struct Glyph {
    ch: char,
    bounds: Option<PdfRect>,
    font_size: f32,
}

fn glyphs(text: &PdfPageText<'_>) -> Vec<Glyph> {
    text.chars()
        .iter()
        .map(|ch| {
            let bounds = ch.tight_bounds().ok().map(|r| PdfRect {
                x: r.left().value,
                y: r.bottom().value,
                width: r.width().value,
                height: r.height().value,
            });
            Glyph {
                ch: ch.unicode_char().unwrap_or('\0').to_ascii_uppercase(),
                bounds,
                font_size: ch.scaled_font_size().value,
            }
        })
        .collect()
}

fn word_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '.' | '-' | '_' | '–' | '—')
}

fn tokens(glyphs: &[Glyph]) -> Vec<(String, std::ops::Range<usize>)> {
    let mut tokens = Vec::new();
    let mut start = 0;
    while start < glyphs.len() {
        if !word_char(glyphs[start].ch) {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while end < glyphs.len() && word_char(glyphs[end].ch) {
            end += 1;
        }
        let mut token_end = end;
        while token_end > start && glyphs[token_end - 1].ch == '.' {
            token_end -= 1;
        }
        let word = glyphs[start..token_end].iter().map(|g| g.ch).collect();
        tokens.push((word, start..token_end));
        start = end;
    }
    tokens
}

fn bounds(glyphs: &[Glyph]) -> Option<PdfRect> {
    let mut rects = glyphs
        .iter()
        .filter_map(|g| g.bounds)
        .filter(|r| r.is_valid());
    let mut r = rects.next()?;
    for next in rects {
        let right = (r.x + r.width).max(next.x + next.width);
        let top = (r.y + r.height).max(next.y + next.height);
        r.x = r.x.min(next.x);
        r.y = r.y.min(next.y);
        r.width = right - r.x;
        r.height = top - r.y;
    }
    Some(r)
}

pub(super) fn generate_links(
    path: &Path,
    sheets: &[SheetCandidate],
) -> Result<Vec<LinkProposal>, PdfError> {
    validate_pdf_path(path)?;
    let pdfium = bind_bundled_pdfium()?;
    let document = pdfium
        .load_pdf_from_file(path, None)
        .map_err(map_pdfium_load_error)?;
    let mut destinations = HashMap::<String, Option<usize>>::new();
    for sheet in sheets {
        if sheet.confidence == 0 || sheet.page_index >= document.pages().len() as usize {
            continue;
        }
        if let Some(id) = normalize_sheet_id(&sheet.id.0) {
            destinations
                .entry(id.0)
                .and_modify(|page| {
                    if *page != Some(sheet.page_index) {
                        *page = None;
                    }
                })
                .or_insert(Some(sheet.page_index));
        }
    }
    let mut links = Vec::new();
    for page_index in 0..document.pages().len() as usize {
        let page = document
            .pages()
            .get(page_index as i32)
            .map_err(map_pdfium_load_error)?;
        let text = page.text().map_err(map_pdfium_load_error)?;
        let glyphs = glyphs(&text);
        for (word, range) in tokens(&glyphs) {
            let Some(id) = normalize_sheet_id(&word) else {
                continue;
            };
            let Some(Some(target_page)) = destinations.get(&id.0) else {
                continue;
            };
            if *target_page == page_index {
                continue;
            }
            if let Some(rect) = bounds(&glyphs[range]) {
                // PDFium character boxes are native, unrotated page coordinates.
                // Do not apply viewer rotation or subtract the CropBox origin.
                links.push(LinkProposal {
                    from_page: page_index,
                    target_page: *target_page,
                    rect,
                    label: id.0,
                });
            }
        }
    }
    Ok(links)
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::core::sheet::SheetId;
    use lopdf::Stream;

    fn fixture(path: &Path, bodies: &[&str], rotation: i64) {
        let mut doc = Document::with_version("1.7");
        let pages_id = doc.new_object_id();
        let font = doc
            .add_object(dictionary! {"Type"=>"Font", "Subtype"=>"Type1", "BaseFont"=>"Helvetica"});
        let kids: Vec<_> = bodies.iter().map(|body| {
            let content = doc.add_object(Stream::new(dictionary! {}, body.as_bytes().to_vec()));
            Object::Reference(doc.add_object(dictionary! {
                "Type"=>"Page", "Parent"=>pages_id, "MediaBox"=>vec![0.into(),0.into(),600.into(),800.into()],
                "CropBox"=>vec![10.into(),20.into(),590.into(),780.into()], "Rotate"=>rotation,
                "Resources"=>dictionary! {"Font"=>dictionary! {"F1"=>font}}, "Contents"=>content
            }))
        }).collect();
        doc.objects.insert(
            pages_id,
            Object::Dictionary(
                dictionary! {"Type"=>"Pages", "Count"=>kids.len() as i64, "Kids"=>kids},
            ),
        );
        let catalog = doc.add_object(dictionary! {"Type"=>"Catalog", "Pages"=>pages_id});
        doc.trailer.set("Root", catalog);
        doc.save(path).unwrap();
    }

    fn sheets(ids: &[&str]) -> Vec<SheetCandidate> {
        ids.iter()
            .enumerate()
            .map(|(page_index, id)| SheetCandidate {
                id: SheetId((*id).into()),
                title: Some((*id).into()),
                page_index,
                confidence: 90,
            })
            .collect()
    }

    #[test]
    fn generated_links_preserve_indirect_annotation_arrays_and_custom_links() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.pdf");
        let output = dir.path().join("linked.pdf");
        fixture(&input, &["BT /F1 12 Tf 60 400 Td (See A-101) Tj ET", ""], 0);
        let proposals = PdfiumRenderEngine
            .generate_sheet_label_links(&input, &sheets(&["E2.01", "A-101"]))
            .unwrap();
        let mut doc = Document::load(&input).unwrap();
        let pages = doc.get_pages();
        let source = pages[&1];
        let custom = doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Link",
            "Rect" => vec![10.into(), 20.into(), 30.into(), 40.into()],
            "Dest" => vec![Object::Reference(pages[&2]), Object::Name(b"Fit".to_vec())]
        });
        let external = doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Link",
            "Rect" => vec![40.into(), 50.into(), 60.into(), 70.into()],
            "A" => dictionary! {"S" => "URI", "URI" => Object::string_literal("https://example.com")}
        });
        let annotations =
            doc.add_object(vec![Object::Reference(custom), Object::Reference(external)]);
        doc.get_object_mut(source)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Annots", annotations);
        doc.save(&input).unwrap();
        assert_eq!(
            LopdfEditEngine
                .write_links(&input, &output, &proposals)
                .unwrap()
                .links_written,
            1
        );
        assert_eq!(extract_internal_links(&output, 0).unwrap().len(), 2);
        let saved = Document::load(&output).unwrap();
        assert_eq!(
            saved.get_object(custom).unwrap(),
            doc.get_object(custom).unwrap()
        );
        assert_eq!(
            saved.get_object(external).unwrap(),
            doc.get_object(external).unwrap()
        );
        let repeated = dir.path().join("repeated.pdf");
        assert_eq!(
            LopdfEditEngine
                .write_links(&output, &repeated, &proposals)
                .unwrap()
                .links_written,
            0
        );
        assert_eq!(extract_internal_links(&repeated, 0).unwrap().len(), 2);
    }

    #[test]
    fn generated_bookmarks_and_clickable_links_roundtrip_and_repeat_without_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.pdf");
        let bookmarked = dir.path().join("bookmarked.pdf");
        let linked = dir.path().join("linked.pdf");
        let repeated = dir.path().join("repeated.pdf");
        fixture(
            &input,
            &[
                "BT /F1 10 Tf 440 95 Td (SHEET NUMBER) Tj ET BT /F1 24 Tf 440 55 Td (A-101) Tj ET BT /F1 12 Tf 60 400 Td (See E2.01.) Tj ET",
                "BT /F1 10 Tf 440 95 Td (SHEET NUMBER) Tj ET BT /F1 24 Tf 440 55 Td (E2.01) Tj ET BT /F1 12 Tf 60 400 Td (See A-101.) Tj ET",
            ],
            270,
        );
        let analysis = PdfiumRenderEngine.analyze_sheets(&input).unwrap();
        let proposals = PdfiumRenderEngine
            .generate_sheet_label_links(&input, &analysis.sheets)
            .unwrap();
        assert_eq!(proposals.len(), 2);
        let report = LopdfEditEngine
            .write_bookmarks(&input, &bookmarked, &analysis.sheets)
            .unwrap();
        assert_eq!(report.bookmarks_written, 2);
        assert_eq!(
            LopdfEditEngine
                .write_links(&bookmarked, &linked, &proposals)
                .unwrap()
                .links_written,
            2
        );
        let summary = LopdfInspectionEngine.inspect(&linked).unwrap();
        assert_eq!(
            summary
                .bookmarks
                .iter()
                .map(|b| (b.title.as_str(), b.page_index))
                .collect::<Vec<_>>(),
            vec![("A-101", Some(0)), ("E2.01", Some(1))]
        );
        for proposal in &proposals {
            let links = extract_internal_links(&linked, proposal.from_page).unwrap();
            assert_eq!(links.len(), 1);
            assert_eq!(links[0].target_page, proposal.target_page);
            assert!((links[0].rect.x - proposal.rect.x).abs() < 0.001);
            assert!((links[0].rect.y - proposal.rect.y).abs() < 0.001);
            assert!((links[0].rect.width - proposal.rect.width).abs() < 0.001);
            assert!((links[0].rect.height - proposal.rect.height).abs() < 0.001);
        }
        assert_eq!(
            LopdfEditEngine
                .write_links(&linked, &repeated, &proposals)
                .unwrap()
                .links_written,
            0
        );
        assert_eq!(extract_internal_links(&repeated, 0).unwrap().len(), 1);
        assert_eq!(
            LopdfInspectionEngine
                .inspect(&repeated)
                .unwrap()
                .bookmarks
                .len(),
            2
        );
    }

    #[test]
    #[ignore = "read-only local SALTAIR regression fixture"]
    fn real_saltair_rotated_titleblock_is_l1_11() {
        let path = Path::new(
            "/home/daniel/Downloads/SALTAIR - ARCH - PC CORR - GREEN PLAN - 260923 - LAND.pdf",
        );
        let analysis = PdfiumRenderEngine.analyze_sheets(path).unwrap();
        assert_eq!(analysis.text_page_count, 1);
        assert_eq!(analysis.sheets.len(), 1);
        assert_eq!(analysis.sheets[0].id.0, "L1.11", "{analysis:?}");
        assert_eq!(analysis.sheets[0].title.as_deref(), Some("L1.11"));
        assert!(
            PdfiumRenderEngine
                .generate_sheet_label_links(path, &analysis.sheets)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn unknown_and_textless_pages_have_only_zero_confidence_fallbacks() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("unknown.pdf");
        fixture(
            &input,
            &[
                "",
                "BT /F1 12 Tf 50 400 Td (MODEL123 AHU-101 See A-101) Tj ET",
            ],
            0,
        );
        let analysis = PdfiumRenderEngine.analyze_sheets(&input).unwrap();
        assert_eq!(analysis.text_page_count, 1);
        assert_eq!(analysis.sheets.len(), 2);
        assert!(analysis.sheets.iter().all(|s| s.confidence == 0));
        fixture(&input, &[""], 0);
        assert_eq!(
            PdfiumRenderEngine
                .analyze_sheets(&input)
                .unwrap()
                .text_page_count,
            0
        );
    }

    #[test]
    fn zero_confidence_fallbacks_never_become_link_destinations_or_hide_real_sheets() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("fallbacks.pdf");
        fixture(
            &input,
            &["BT /F1 12 Tf 60 400 Td (See P100) Tj ET", "", ""],
            0,
        );
        let mut candidates = sheets(&["A-101", "P100"]);
        candidates[1].confidence = 0;
        assert!(
            PdfiumRenderEngine
                .generate_sheet_label_links(&input, &candidates)
                .unwrap()
                .is_empty()
        );
        candidates.push(SheetCandidate {
            id: SheetId("P100".into()),
            title: None,
            page_index: 2,
            confidence: 90,
        });
        let links = PdfiumRenderEngine
            .generate_sheet_label_links(&input, &candidates)
            .unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target_page, 2);
    }

    #[test]
    fn ambiguous_destinations_and_self_references_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("duplicates.pdf");
        fixture(
            &input,
            &["BT /F1 12 Tf 60 400 Td (See A-101 and E2.01) Tj ET", "", ""],
            0,
        );
        let mut destinations = sheets(&["E2.01", "A-101", "a-101"]);
        assert!(
            PdfiumRenderEngine
                .generate_sheet_label_links(&input, &destinations)
                .unwrap()
                .is_empty()
        );
        destinations[2].id.0 = "L1.11".into();
        assert_eq!(
            PdfiumRenderEngine
                .generate_sheet_label_links(&input, &destinations)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn equally_supported_distinct_sheet_ids_remain_undetected() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("ambiguous-title.pdf");
        fixture(
            &input,
            &[
                "BT /F1 10 Tf 440 95 Td (SHEET NUMBER) Tj ET BT /F1 24 Tf 440 55 Td (A-101) Tj ET BT /F1 24 Tf 440 125 Td (E2.01) Tj ET",
            ],
            0,
        );
        let analysis = PdfiumRenderEngine.analyze_sheets(&input).unwrap();
        assert_eq!(analysis.sheets[0].confidence, 0);
    }

    #[test]
    fn ordinary_corner_reference_is_not_a_sheet_title() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("corner.pdf");
        fixture(
            &input,
            &[
                "BT /F1 12 Tf 60 400 Td (GENERAL NOTES AND REFERENCES) Tj ET BT /F1 12 Tf 500 55 Td (See A-101) Tj ET",
            ],
            0,
        );
        let analysis = PdfiumRenderEngine.analyze_sheets(&input).unwrap();
        assert_eq!(analysis.sheets[0].confidence, 0);
    }

    #[test]
    fn detects_titleblock_ids_over_larger_body_references_on_rotated_pages() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("titles.pdf");
        let bodies: Vec<_> = ["A-101", "A1.01", "E2.01", "L1.11"].iter().map(|id| format!("BT /F1 40 Tf 60 400 Td (See S1.01) Tj ET BT /F1 10 Tf 440 95 Td (SHEET NUMBER) Tj ET BT /F1 24 Tf 440 55 Td ({id}) Tj ET")).collect();
        fixture(
            &input,
            &bodies.iter().map(String::as_str).collect::<Vec<_>>(),
            270,
        );
        let analysis = PdfiumRenderEngine.analyze_sheets(&input).unwrap();
        assert_eq!(analysis.text_page_count, 4);
        assert_eq!(
            analysis
                .sheets
                .iter()
                .map(|s| s.id.0.as_str())
                .collect::<Vec<_>>(),
            ["A-101", "A1.01", "E2.01", "L1.11"]
        );
        assert!(
            analysis
                .sheets
                .iter()
                .all(|s| s.title.as_deref() == Some(s.id.0.as_str()) && s.confidence > 0)
        );
    }

    #[test]
    fn exact_references_in_mixed_text_use_only_identifier_glyph_bounds() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("refs.pdf");
        fixture(
            &input,
            &[
                "BT /F1 12 Tf 60 400 Td (See A-101 and A1.01; not A-1010 or XA-101 or A-101-X.) Tj ET",
                "",
                "",
            ],
            270,
        );
        let links = PdfiumRenderEngine
            .generate_sheet_label_links(&input, &sheets(&["E2.01", "A-101", "A1.01"]))
            .unwrap();
        assert_eq!(links.len(), 2, "{links:?}");
        assert_eq!(links[0].target_page, 1);
        assert_eq!(links[1].target_page, 2);
        assert!(
            links[0].rect.x > 80.0 && links[0].rect.x < 90.0,
            "{:?}",
            links[0].rect
        );
        assert!(links[0].rect.y > 390.0 && links[0].rect.y < 410.0);
        assert!(links[0].rect.width < 40.0);
    }
}
