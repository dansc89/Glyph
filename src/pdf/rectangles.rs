use super::PdfError;
use crate::core::links::PdfRect;
use lopdf::{Document, Object, ObjectId, Stream, dictionary};
use std::collections::HashSet;

/// Native Glyph-owned shape kind; ownership is metadata, not authentication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeKind {
    Rectangle,
    Ellipse,
    Line,
    Arrow,
    Text,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeStyle {
    pub rgb: [f32; 3],
    pub weight: f32,
}
impl Default for ShapeStyle {
    fn default() -> Self {
        Self {
            rgb: [1., 0., 0.],
            weight: 2.,
        }
    }
}
impl ShapeStyle {
    pub fn validate(self) -> Result<(), PdfError> {
        if self
            .rgb
            .iter()
            .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
            || !self.weight.is_finite()
            || self.weight <= 0.
            || self.weight > 64.
        {
            return Err(error("invalid shape stroke style"));
        }
        Ok(())
    }
    pub(super) fn prefix(self) -> String {
        format!(
            "q {} {} {} RG {} w",
            self.rgb[0], self.rgb[1], self.rgb[2], self.weight
        )
    }
}
pub(super) fn read_style(doc: &Document, d: &lopdf::Dictionary) -> Result<ShapeStyle, PdfError> {
    let style = match (d.has(b"GlyphStrokeRGB"), d.has(b"GlyphStrokeWeight")) {
        (false, false) => ShapeStyle::default(),
        (true, true) => {
            let rgb = resolve(doc, d.get(b"GlyphStrokeRGB").map_err(error)?)?
                .as_array()
                .map_err(error)?;
            if rgb.len() != 3 {
                return Err(error("invalid stroke RGB"));
            }
            let mut color = [0.; 3];
            for (a, b) in color.iter_mut().zip(rgb) {
                *a = resolve(doc, b)?.as_float().map_err(error)?;
            }
            ShapeStyle {
                rgb: color,
                weight: resolve(doc, d.get(b"GlyphStrokeWeight").map_err(error)?)?
                    .as_float()
                    .map_err(error)?,
            }
        }
        _ => return Err(error("incomplete stroke style metadata")),
    };
    style.validate()?;
    let color = resolve(doc, d.get(b"C").map_err(error)?)?
        .as_array()
        .map_err(error)?;
    if color.len() != 3 {
        return Err(error("invalid native stroke color"));
    }
    for (a, b) in color.iter().zip(style.rgb) {
        if resolve(doc, a)?.as_float().map_err(error)? != b {
            return Err(error("stroke color mismatch"));
        }
    }
    let bs = resolve(doc, d.get(b"BS").map_err(error)?)?
        .as_dict()
        .map_err(error)?;
    if resolve(doc, bs.get(b"W").map_err(error)?)?
        .as_float()
        .map_err(error)?
        != style.weight
    {
        return Err(error("stroke weight mismatch"));
    }
    Ok(style)
}
/// `rect` is the bounding box in normalized 0..1 display coordinates, top-left
/// origin, after inherited crop and clockwise page rotation.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeAnnotation {
    pub object_id: ObjectId,
    pub page_index: usize,
    pub rect: PdfRect,
    pub kind: ShapeKind,
    pub style: ShapeStyle,
    pub endpoints: Option<[f32; 4]>,
    pub line_head: Option<[f32; 4]>,
    pub text: Option<super::TextMarkup>,
}
/// Compatibility name for callers of the original rectangle API.
pub type RectangleAnnotation = ShapeAnnotation;
fn error(e: impl std::fmt::Display) -> PdfError {
    PdfError::Edit(e.to_string())
}
pub(super) fn resolve<'a>(doc: &'a Document, mut o: &'a Object) -> Result<&'a Object, PdfError> {
    let mut seen = HashSet::new();
    while let Object::Reference(id) = o {
        if !seen.insert(*id) || seen.len() > 256 {
            return Err(error("cyclic annotation reference"));
        }
        o = doc.get_object(*id).map_err(error)?;
    }
    Ok(o)
}
fn inherited(doc: &Document, page: ObjectId, key: &[u8]) -> Result<Option<Object>, PdfError> {
    let mut current = page;
    let mut seen = HashSet::new();
    let mut value = None;
    loop {
        if !seen.insert(current) || seen.len() > 256 {
            return Err(error("cyclic page inheritance"));
        }
        let d = doc
            .get_object(current)
            .and_then(Object::as_dict)
            .map_err(error)?;
        if value.is_none() {
            value = d.get(key).ok().cloned();
        }
        match d.get(b"Parent") {
            Ok(parent) => current = parent.as_reference().map_err(error)?,
            Err(_) => return Ok(value),
        }
    }
}
pub(super) fn numbers(doc: &Document, o: &Object) -> Result<[f32; 4], PdfError> {
    let a = resolve(doc, o)?.as_array().map_err(error)?;
    if a.len() != 4 {
        return Err(error("rectangle requires four coordinates"));
    }
    let mut n = [0.; 4];
    for (i, o) in a.iter().enumerate() {
        n[i] = resolve(doc, o)?.as_float().map_err(error)?;
    }
    if !n.iter().all(|x| x.is_finite()) {
        return Err(error("nonfinite rectangle"));
    }
    Ok(n)
}
pub(super) fn validate(r: PdfRect) -> Result<(), PdfError> {
    if ![r.x, r.y, r.width, r.height].iter().all(|v| v.is_finite())
        || r.x < 0.
        || r.y < 0.
        || r.width <= 0.
        || r.height <= 0.
        || r.x + r.width > 1.
        || r.y + r.height > 1.
    {
        return Err(error("invalid normalized rectangle"));
    }
    Ok(())
}
pub(super) fn page_geometry(doc: &Document, page: ObjectId) -> Result<([f32; 4], i64), PdfError> {
    let media = inherited(doc, page, b"MediaBox")?.ok_or_else(|| error("missing MediaBox"))?;
    let m = numbers(doc, &media)?;
    let mut b = match inherited(doc, page, b"CropBox")? {
        Some(c) => numbers(doc, &c)?,
        None => m,
    };
    if m[2] <= m[0] || m[3] <= m[1] || b[2] <= b[0] || b[3] <= b[1] {
        return Err(error("degenerate page box"));
    }
    b = [
        b[0].max(m[0]),
        b[1].max(m[1]),
        b[2].min(m[2]),
        b[3].min(m[3]),
    ];
    if b[2] <= b[0] || b[3] <= b[1] {
        return Err(error("empty cropped page"));
    }
    let rotation = match inherited(doc, page, b"Rotate")? {
        Some(r) => resolve(doc, &r)?.as_i64().map_err(error)?,
        None => 0,
    };
    if rotation % 90 != 0 {
        return Err(error("unsupported page rotation"));
    }
    Ok((b, rotation.rem_euclid(360)))
}
pub(super) fn mapped(doc: &Document, page: ObjectId, r: PdfRect) -> Result<[f32; 4], PdfError> {
    validate(r)?;
    let (b, rotation) = page_geometry(doc, page)?;
    let (w, h) = (b[2] - b[0], b[3] - b[1]);
    let point = |x: f32, y: f32| {
        let (u, v) = match rotation {
            0 => (x, 1. - y),
            90 => (y, x),
            180 => (1. - x, y),
            270 => (1. - y, 1. - x),
            _ => unreachable!(),
        };
        (b[0] + u * w, b[1] + v * h)
    };
    let a = point(r.x, r.y);
    let c = point(r.x + r.width, r.y + r.height);
    let bounds = [a.0.min(c.0), a.1.min(c.1), a.0.max(c.0), a.1.max(c.1)];
    if !bounds.iter().all(|v| v.is_finite()) || bounds[2] <= bounds[0] || bounds[3] <= bounds[1] {
        return Err(error("degenerate mapped rectangle"));
    }
    Ok(bounds)
}
pub(super) fn annots(
    doc: &Document,
    page: ObjectId,
) -> Result<(Option<Object>, Vec<Object>), PdfError> {
    let d = doc
        .get_object(page)
        .and_then(Object::as_dict)
        .map_err(error)?;
    let original = d.get(b"Annots").ok().cloned();
    let a = match &original {
        Some(o) => resolve(doc, o)?.as_array().map_err(error)?.clone(),
        None => vec![],
    };
    for o in &a {
        resolve(doc, o)?.as_dict().map_err(error)?;
    }
    Ok((original, a))
}
pub(super) fn read(doc: &Document) -> Result<Vec<ShapeAnnotation>, PdfError> {
    let mut result = vec![];
    let mut seen = HashSet::new();
    for (page_index, page) in doc.get_pages().values().enumerate() {
        page_geometry(doc, *page)?;
        for o in annots(doc, *page)?.1 {
            let d = resolve(doc, &o)?.as_dict().map_err(error)?;
            if d.has(b"GlyphText") {
                let item = super::free_text::read(doc, *page, page_index, &o)?;
                if !seen.insert(item.object_id) {
                    return Err(error("duplicate owned text"));
                }
                result.push(item);
                continue;
            }
            if d.has(b"GlyphLine") || d.has(b"GlyphArrow") {
                let item = super::lines::read(doc, *page, page_index, &o)?;
                if !seen.insert(item.object_id) {
                    return Err(error("duplicate owned shape"));
                }
                result.push(item);
                continue;
            }
            let (kind, marker, subtype) = match (d.has(b"GlyphRectangle"), d.has(b"GlyphEllipse")) {
                (false, false) => continue,
                (true, false) => (
                    ShapeKind::Rectangle,
                    b"GlyphRectangle".as_slice(),
                    b"Square".as_slice(),
                ),
                (false, true) => (
                    ShapeKind::Ellipse,
                    b"GlyphEllipse".as_slice(),
                    b"Circle".as_slice(),
                ),
                (true, true) => return Err(error("ambiguous owned shape")),
            };
            if d.get(marker).and_then(Object::as_i64).map_err(error)? != 1
                || d.get(b"Subtype").and_then(Object::as_name).map_err(error)? != subtype
            {
                return Err(error("malformed owned shape"));
            }
            let id = o.as_reference().map_err(error)?;
            if !seen.insert(id) {
                return Err(error("duplicate owned shape"));
            }
            let n = numbers(doc, d.get(b"GlyphNormalizedRect").map_err(error)?)?;
            let rect = PdfRect {
                x: n[0],
                y: n[1],
                width: n[2],
                height: n[3],
            };
            let expected = mapped(doc, *page, rect)?;
            let actual = numbers(doc, d.get(b"Rect").map_err(error)?)?;
            if actual[2] <= actual[0] || actual[3] <= actual[1] {
                return Err(error("degenerate owned shape Rect"));
            }
            if expected
                .iter()
                .zip(actual)
                .any(|(a, b)| (*a - b).abs() > 0.001)
            {
                return Err(error("owned shape mapping mismatch"));
            }
            let ap = resolve(doc, d.get(b"AP").map_err(error)?)?
                .as_dict()
                .map_err(error)?;
            let stream = resolve(doc, ap.get(b"N").map_err(error)?)?
                .as_stream()
                .map_err(error)?;
            if stream
                .dict
                .get(b"Subtype")
                .and_then(Object::as_name)
                .map_err(error)?
                != b"Form"
                || stream.content.is_empty()
            {
                return Err(error("invalid shape appearance"));
            }
            validate_matrix(doc, &stream.dict)?;
            let bbox = numbers(doc, stream.dict.get(b"BBox").map_err(error)?)?;
            if bbox[2] <= bbox[0] || bbox[3] <= bbox[1] {
                return Err(error("degenerate owned shape appearance BBox"));
            }
            result.push(ShapeAnnotation {
                object_id: id,
                page_index,
                rect,
                kind,
                style: read_style(doc, d)?,
                endpoints: None,
                line_head: None,
                text: None,
            });
        }
    }
    Ok(result)
}
pub(super) fn validate_matrix(doc: &Document, d: &lopdf::Dictionary) -> Result<(), PdfError> {
    if let Ok(o) = d.get(b"Matrix") {
        let m = resolve(doc, o)?.as_array().map_err(error)?;
        if m.len() != 6 {
            return Err(error("invalid owned appearance matrix"));
        }
        for (o, expected) in m.iter().zip([1., 0., 0., 1., 0., 0.]) {
            let n = resolve(doc, o)?.as_float().map_err(error)?;
            if !n.is_finite() || n != expected {
                return Err(error("invalid owned appearance matrix"));
            }
        }
    }
    Ok(())
}
fn pdf_number(n: f32) -> Object {
    if n.fract() == 0. && (n as f64).abs() < i64::MAX as f64 {
        Object::Integer(n as i64)
    } else {
        Object::Real(n)
    }
}
pub(super) fn create(
    doc: &mut Document,
    page: ObjectId,
    rect: PdfRect,
    kind: ShapeKind,
) -> Result<ObjectId, PdfError> {
    create_styled(doc, page, rect, kind, ShapeStyle::default())
}
pub(super) fn create_styled(
    doc: &mut Document,
    page: ObjectId,
    rect: PdfRect,
    kind: ShapeKind,
    style: ShapeStyle,
) -> Result<ObjectId, PdfError> {
    style.validate()?;
    let b = mapped(doc, page, rect)?;
    let (w, h) = (b[2] - b[0], b[3] - b[1]);
    let bytes = appearance(w, h, kind, style)?;
    let appearance = doc.add_object(Stream::new(
        dictionary! {"Type" => "XObject", "Subtype" => "Form", "FormType" => 1,
            "BBox" => vec![0.into(), 0.into(), pdf_number(w), pdf_number(h)], "Resources" => dictionary! {}}, bytes));
    Ok(doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => if kind == ShapeKind::Rectangle { "Square" } else { "Circle" },
        if kind == ShapeKind::Rectangle { "GlyphRectangle" } else { "GlyphEllipse" } => 1,
        "GlyphNormalizedRect" => vec![pdf_number(rect.x), pdf_number(rect.y), pdf_number(rect.width), pdf_number(rect.height)],
        "GlyphStrokeRGB" => style.rgb.into_iter().map(pdf_number).collect::<Vec<_>>(),
        "GlyphStrokeWeight" => pdf_number(style.weight),
        "Rect" => b.into_iter().map(pdf_number).collect::<Vec<_>>(), "P" => page, "F" => 4,
        "C" => style.rgb.into_iter().map(pdf_number).collect::<Vec<_>>(),
        "BS" => dictionary! {"W" => pdf_number(style.weight), "S" => "S"},
        "Border" => vec![0.into(), 0.into(), pdf_number(style.weight)],
        "AP" => dictionary! {"N" => appearance}}))
}
fn appearance(w: f32, h: f32, kind: ShapeKind, style: ShapeStyle) -> Result<Vec<u8>, PdfError> {
    let (ix, iy) = (
        (style.weight / 2.).min(w / 4.),
        (style.weight / 2.).min(h / 4.),
    );
    let bytes = match kind {
        ShapeKind::Rectangle => format!(
            "q 1 0 0 RG 2 w {ix} {iy} {} {} re S Q\n",
            w - 2. * ix,
            h - 2. * iy
        ),
        ShapeKind::Ellipse => {
            let (cx, cy) = (w / 2., h / 2.);
            let (rx, ry) = (cx - ix, cy - iy);
            let (kx, ky) = (rx * 0.552_284_8, ry * 0.552_284_8);
            format!(
                "q 1 0 0 RG 2 w {} {cy} m {} {} {} {} {cx} {} c {} {} {ix} {} {ix} {cy} c {ix} {} {} {iy} {cx} {iy} c {} {iy} {} {} {} {cy} c h S Q\n",
                w - ix,
                w - ix,
                cy + ky,
                cx + kx,
                h - iy,
                h - iy,
                cx - kx,
                h - iy,
                cy + ky,
                cy - ky,
                cx - kx,
                cx + kx,
                w - ix,
                cy - ky,
                w - ix
            )
        }
        ShapeKind::Line | ShapeKind::Arrow | ShapeKind::Text => {
            return Err(error("shape requires specialized content"));
        }
    };
    Ok(bytes
        .replacen("q 1 0 0 RG 2 w", &style.prefix(), 1)
        .into_bytes())
}
