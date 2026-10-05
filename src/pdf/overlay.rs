//! PDFium coordinate mapping for overlays, including crop and rotation.
use super::PdfError;
use crate::core::links::PdfRect;
use pdfium_bundled::pdfium_render::prelude::{PdfPoints, PdfRenderConfig};
use std::path::Path;

/// Convert PDF-coordinate rectangles to top-left-origin unit-page rectangles.
/// PDFium supplies the same crop/rotation transform used by the rendered image.
pub fn normalize_rectangles(
    path: &Path,
    rectangles: &mut [(usize, PdfRect)],
) -> Result<(), PdfError> {
    if rectangles.is_empty() {
        return Ok(());
    }
    super::validate_pdf_path(path)?;
    let pdfium = super::bind_bundled_pdfium()?;
    let document = pdfium
        .load_pdf_from_file(path, None)
        .map_err(super::map_pdfium_load_error)?;
    for (page_index, rect) in rectangles {
        let page = document
            .pages()
            .get(*page_index as i32)
            .map_err(|err| PdfError::Render(err.to_string()))?;
        normalize_page_rectangle(&page, rect)?;
    }
    Ok(())
}

pub(super) fn normalize_page_rectangle(
    page: &pdfium_bundled::pdfium_render::prelude::PdfPage<'_>,
    rect: &mut PdfRect,
) -> Result<(), PdfError> {
    let config = PdfRenderConfig::new().set_fixed_size(10_000, 10_000);
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for (x, y) in [
        (rect.x, rect.y),
        (rect.x + rect.width, rect.y),
        (rect.x, rect.y + rect.height),
        (rect.x + rect.width, rect.y + rect.height),
    ] {
        let (x, y) = page
            .points_to_pixels(PdfPoints::new(x), PdfPoints::new(y), &config)
            .map_err(|err| PdfError::Render(err.to_string()))?;
        let xy = [x as f32 / 10_000., y as f32 / 10_000.];
        for axis in 0..2 {
            min[axis] = min[axis].min(xy[axis]);
            max[axis] = max[axis].max(xy[axis]);
        }
    }
    *rect = PdfRect {
        x: min[0],
        y: min[1],
        width: max[0] - min[0],
        height: max[1] - min[1],
    };
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{Document, Object, Stream, dictionary};
    #[test]
    fn overlays_use_pdfium_crop_and_rotation_geometry() {
        let dir = tempfile::tempdir().unwrap();
        for rotation in [0, 90, 180, 270] {
            let path = dir.path().join(format!("{rotation}.pdf"));
            let mut doc = Document::with_version("1.7");
            let pages_id = doc.new_object_id();
            let content = doc.add_object(Stream::new(dictionary! {}, vec![]));
            let page = doc.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages_id,
                "MediaBox" => vec![0.into(),0.into(),600.into(),800.into()],
                "CropBox" => vec![100.into(),200.into(),500.into(),600.into()],
                "Rotate" => rotation, "Resources" => dictionary!{}, "Contents" => content,
            });
            doc.objects.insert(
                pages_id,
                Object::Dictionary(dictionary! {
                    "Type" => "Pages", "Kids" => vec![Object::Reference(page)], "Count" => 1,
                }),
            );
            let catalog = doc.add_object(dictionary! {"Type" => "Catalog", "Pages" => pages_id});
            doc.trailer.set("Root", catalog);
            doc.save(&path).unwrap();
            let mut rects = vec![(
                0,
                PdfRect {
                    x: 100.,
                    y: 200.,
                    width: 400.,
                    height: 400.,
                },
            )];
            normalize_rectangles(&path, &mut rects).unwrap();
            let rect = rects[0].1;
            assert!(
                rect.x.abs() < 0.002 && rect.y.abs() < 0.002,
                "{rotation}: {rect:?}"
            );
            assert!((rect.width - 1.).abs() < 0.002 && (rect.height - 1.).abs() < 0.002);
        }
    }
}
