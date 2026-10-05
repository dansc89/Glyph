//! Worker-extracted selectable text, in PDFium reading order.
use crate::core::links::PdfRect;

#[derive(Debug, Clone, PartialEq)]
pub struct PageText {
    pub page_index: usize,
    pub glyphs: Vec<TextGlyph>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextGlyph {
    pub text: String,
    /// Top-left-origin unit-page bounds, matching rendered crop and rotation.
    /// Missing or degenerate character boxes have no selectable geometry.
    pub rect: Option<PdfRect>,
}

#[cfg(test)]
mod tests {
    use crate::pdf::{PdfiumSession, bind_render_pdfium};
    use lopdf::{Document, Object, Stream, dictionary};
    use std::path::Path;

    fn write_pdf(path: &Path, content: &[u8], rotation: i32, crop: bool) {
        let mut doc = Document::with_version("1.7");
        let pages = doc.new_object_id();
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
            "Encoding" => "WinAnsiEncoding",
        });
        let content = doc.add_object(Stream::new(dictionary! {}, content.to_vec()));
        let mut page_dict = dictionary! {
            "Type" => "Page", "Parent" => pages,
            "MediaBox" => vec![0.into(), 0.into(), 600.into(), 800.into()],
            "Rotate" => rotation,
            "Resources" => dictionary! {"Font" => dictionary! {"F1" => font}},
            "Contents" => content,
        };
        if crop {
            page_dict.set(
                "CropBox",
                vec![100.into(), 200.into(), 500.into(), 600.into()],
            );
        }
        let page = doc.add_object(page_dict);
        doc.objects.insert(
            pages,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1,
            }),
        );
        let catalog = doc.add_object(dictionary! {"Type" => "Catalog", "Pages" => pages});
        doc.trailer.set("Root", catalog);
        doc.save(path).unwrap();
    }

    #[test]
    fn rejects_oversized_text_without_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.pdf");
        // PDFium limits each text object to 32767 characters; use separate rows.
        let content = (0..21)
            .map(|row| {
                format!(
                    "BT /F1 0.001 Tf 150 {} Td ({}) Tj ET\n",
                    500 - row,
                    "A".repeat(10_000)
                )
            })
            .collect::<String>();
        write_pdf(&path, content.as_bytes(), 0, false);
        let pdfium = bind_render_pdfium().unwrap();
        let document = pdfium.load_pdf_from_file(&path, None).unwrap();
        let page = document.pages().get(0).unwrap();
        let native_count = page.text().unwrap().len();
        assert!(native_count > 200_000, "fixture count {native_count}");
        let mut session = PdfiumSession::new(&pdfium);
        let result = session.extract_page_text(&path, 1, 0);
        assert!(
            result.is_err(),
            "oversized page must not return truncated or unbounded glyphs: actual count {:?}",
            result.as_ref().map(|text| text.glyphs.len())
        );
        let message = result.unwrap_err().to_string();
        assert!(
            message.contains(&native_count.to_string()) && message.contains("200000"),
            "{message}"
        );
    }

    #[test]
    fn textless_page_has_no_glyphs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.pdf");
        write_pdf(&path, b"", 0, false);
        let pdfium = bind_render_pdfium().unwrap();
        let text = PdfiumSession::new(&pdfium)
            .extract_page_text(&path, 1, 0)
            .unwrap();
        assert_eq!(text.page_index, 0);
        assert!(text.glyphs.is_empty());
    }

    #[test]
    fn preserves_multiline_whitespace_and_unicode_without_boxes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("unicode.pdf");
        write_pdf(
            &path,
            b"BT /F1 24 Tf 150 500 Td (Caf\\351 au) Tj 0 -40 Td (lait) Tj ET",
            0,
            false,
        );
        let pdfium = bind_render_pdfium().unwrap();
        let text = PdfiumSession::new(&pdfium)
            .extract_page_text(&path, 1, 0)
            .unwrap();
        assert_eq!(
            text.glyphs
                .iter()
                .map(|g| g.text.as_str())
                .collect::<String>(),
            "Café au\r\nlait"
        );
        assert_eq!(text.glyphs.len(), 13);
        for glyph in text
            .glyphs
            .iter()
            .filter(|g| g.text == "\r" || g.text == "\n")
        {
            assert_eq!(glyph.rect, None);
        }
    }

    #[test]
    fn rejects_out_of_bounds_indices_before_narrowing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("text.pdf");
        write_pdf(&path, b"", 0, false);
        let pdfium = bind_render_pdfium().unwrap();
        let mut session = PdfiumSession::new(&pdfium);
        for index in [1, i32::MAX as usize + 1, usize::MAX] {
            let error = session.extract_page_text(&path, 1, index).unwrap_err();
            assert!(matches!(error, crate::pdf::PdfError::Render(_)));
            assert!(error.to_string().contains(&format!("page index {index}")));
        }
        assert_eq!(session.document_load_count(), 1);
    }

    #[test]
    fn text_and_renders_share_document_and_reload_by_generation_or_path() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.pdf");
        let second = dir.path().join("second.pdf");
        write_pdf(&first, b"BT /F1 24 Tf 150 500 Td (First) Tj ET", 0, false);
        write_pdf(&second, b"BT /F1 24 Tf 150 500 Td (Second) Tj ET", 0, false);
        let pdfium = bind_render_pdfium().unwrap();
        let mut session = PdfiumSession::new(&pdfium);
        session.render_page(&first, 1, 0, 128).unwrap();
        for _ in 0..3 {
            let text = session.extract_page_text(&first, 1, 0).unwrap();
            assert_eq!(
                text.glyphs
                    .iter()
                    .map(|g| g.text.as_str())
                    .collect::<String>(),
                "First"
            );
        }
        assert_eq!(session.document_load_count(), 1);
        let replacement = dir.path().join("replacement.pdf");
        write_pdf(
            &replacement,
            b"BT /F1 24 Tf 150 500 Td (Updated) Tj ET",
            0,
            false,
        );
        std::fs::rename(&replacement, &first).unwrap();
        let text = session.extract_page_text(&first, 2, 0).unwrap();
        assert_eq!(
            text.glyphs
                .iter()
                .map(|g| g.text.as_str())
                .collect::<String>(),
            "Updated"
        );
        assert_eq!(session.document_load_count(), 2);
        let text = session.extract_page_text(&second, 2, 0).unwrap();
        assert_eq!(
            text.glyphs
                .iter()
                .map(|g| g.text.as_str())
                .collect::<String>(),
            "Second"
        );
        session.render_page(&second, 2, 0, 128).unwrap();
        assert_eq!(session.document_load_count(), 3);
    }

    #[test]
    fn cropped_rotated_glyphs_match_overlay_transform_and_rendered_ink() {
        let dir = tempfile::tempdir().unwrap();
        let pdfium = bind_render_pdfium().unwrap();
        let mut session = PdfiumSession::new(&pdfium);
        for rotation in [0, 90, 180, 270] {
            let path = dir.path().join(format!("{rotation}.pdf"));
            write_pdf(&path, b"BT /F1 24 Tf 150 500 Td (H) Tj ET", rotation, true);
            let document = pdfium.load_pdf_from_file(&path, None).unwrap();
            let page = document.pages().get(0).unwrap();
            let page_text = page.text().unwrap();
            let bounds = page_text.chars().get(0).unwrap().tight_bounds().unwrap();
            let mut expected = [(
                0,
                crate::core::links::PdfRect {
                    x: bounds.left().value,
                    y: bounds.bottom().value,
                    width: bounds.width().value,
                    height: bounds.height().value,
                },
            )];
            crate::pdf::overlay::normalize_rectangles(&path, &mut expected).unwrap();
            let text = session.extract_page_text(&path, 1, 0).unwrap();
            assert_eq!(text.glyphs.len(), 1);
            let rect = text.glyphs[0].rect.unwrap();
            assert_eq!(rect, expected[0].1, "rotation {rotation}");
            let rendered = session.render_page(&path, 1, 0, 800).unwrap();
            let mut ink = 0;
            for y in 0..rendered.height {
                for x in 0..rendered.width {
                    let pixel = &rendered.rgba[(y * rendered.width + x) * 4..][..3];
                    if pixel.iter().all(|v| *v < 128) {
                        ink += 1;
                        let nx = x as f32 / rendered.width as f32;
                        let ny = y as f32 / rendered.height as f32;
                        assert!(
                            nx >= rect.x - 0.003
                                && nx <= rect.x + rect.width + 0.003
                                && ny >= rect.y - 0.003
                                && ny <= rect.y + rect.height + 0.003,
                            "rotation {rotation}: ink ({nx}, {ny}) outside {rect:?}"
                        );
                    }
                }
            }
            assert!(ink > 0, "render must contain actual ink");
        }
        assert_eq!(session.document_load_count(), 4);
    }

    #[test]
    fn extracts_characters_in_reading_order_with_normalized_boxes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("text.pdf");
        write_pdf(
            &path,
            b"BT /F1 24 Tf 150 500 Td (Hello world) Tj ET",
            0,
            false,
        );
        let pdfium = bind_render_pdfium().unwrap();
        let mut session = PdfiumSession::new(&pdfium);
        let text = session.extract_page_text(&path, 1, 0).unwrap();
        assert_eq!(text.page_index, 0);
        assert_eq!(
            text.glyphs
                .iter()
                .map(|g| g.text.as_str())
                .collect::<String>(),
            "Hello world"
        );
        assert_eq!(text.glyphs.len(), 11);
        for glyph in text.glyphs.iter().filter(|g| g.text != " ") {
            let rect = glyph.rect.expect("visible character has a box");
            assert!(rect.x > 0. && rect.y > 0. && rect.width > 0. && rect.height > 0.);
            assert!(rect.x + rect.width < 1. && rect.y + rect.height < 1.);
        }
    }
}
