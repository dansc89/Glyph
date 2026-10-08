//! Canonical FreeText using the PDF standard Courier font. Printable ASCII + LF
//! only: unsupported encoding and text that cannot fit are rejected, never replaced.
use super::{PdfError, ShapeAnnotation, ShapeKind, ShapeStyle, rectangles};
use crate::core::links::PdfRect;
use lopdf::{Document, Object, ObjectId, Stream, dictionary};

fn error(e: impl std::fmt::Display) -> PdfError {
    PdfError::Edit(e.to_string())
}
#[derive(Clone, Debug, PartialEq)]
pub struct TextMarkup {
    pub contents: String,
    pub size: f32,
}
impl TextMarkup {
    pub fn validate(&self) -> Result<(), PdfError> {
        if !self.size.is_finite() || !(6. ..=144.).contains(&self.size) {
            return Err(error("Font size must be 6–144 physical PDF points"));
        }
        if self.contents.trim().is_empty()
            || self.contents.len() > 10000
            || !self
                .contents
                .bytes()
                .all(|b| b == b'\n' || (32..=126).contains(&b))
        {
            return Err(error(
                "Text supports printable ASCII and line breaks only (Courier), 1–10000 bytes; no replacement glyphs",
            ));
        }
        Ok(())
    }
}
fn array(n: [f32; 4]) -> Vec<Object> {
    n.into_iter().map(Object::Real).collect()
}
fn unicode(s: &str) -> Object {
    let mut b = vec![0xfe, 0xff];
    for c in s.encode_utf16() {
        b.extend(c.to_be_bytes());
    }
    Object::String(b, lopdf::StringFormat::Hexadecimal)
}
fn unit(doc: &Document, page: ObjectId) -> Result<f32, PdfError> {
    let p = doc
        .get_object(page)
        .and_then(Object::as_dict)
        .map_err(error)?;
    let u = match p.get(b"UserUnit") {
        Ok(v) => rectangles::resolve(doc, v)?.as_float().map_err(error)?,
        Err(_) => 1.,
    };
    if !u.is_finite() || !(0. ..=75000.).contains(&u) || u == 0. {
        return Err(error("Invalid UserUnit"));
    }
    Ok(u)
}
fn font() -> lopdf::Dictionary {
    dictionary! {"Type"=>"Font", "Subtype"=>"Type1", "BaseFont"=>"Courier", "Encoding"=>"WinAnsiEncoding"}
}
pub(super) fn appearance(
    doc: &Document,
    page: ObjectId,
    rect: PdfRect,
    text: &TextMarkup,
    style: ShapeStyle,
) -> Result<(Stream, [f32; 4], String), PdfError> {
    text.validate()?;
    style.validate()?;
    let bounds = rectangles::mapped(doc, page, rect)?;
    let (_, rotation) = rectangles::page_geometry(doc, page)?;
    let u = unit(doc, page)?;
    let f = text.size / u;
    let w = bounds[2] - bounds[0];
    let h = bounds[3] - bounds[1];
    let (dw, dh) = if rotation % 180 == 0 { (w, h) } else { (h, w) };
    let margin = 2. / u;
    if [f, w, h, margin].iter().any(|v| !v.is_finite() || *v <= 0.) {
        return Err(error("Nonfinite or degenerate physical text geometry"));
    }
    let lines: Vec<_> = text.contents.split('\n').collect();
    if lines
        .iter()
        .any(|s| s.len() as f32 * 0.6 * f + margin * 2. > dw)
        || lines.len() as f32 * 1.2 * f + margin * 2. > dh
    {
        return Err(error(
            "Text does not fit this box; reduce font size, shorten lines or enlarge the box",
        ));
    }
    let matrix = match rotation {
        0 => "1 0 0 1 0 0".into(),
        90 => format!("0 1 -1 0 {w} 0"),
        180 => format!("-1 0 0 -1 {w} {h}"),
        _ => format!("0 -1 1 0 0 {h}"),
    };
    let da = format!(
        "/GlyphCourier {f} Tf {} {} {} rg",
        style.rgb[0], style.rgb[1], style.rgb[2]
    );
    let mut operations = vec![lopdf::content::Operation::new("q", vec![])];
    operations.push(lopdf::content::Operation::new(
        "cm",
        matrix
            .split_whitespace()
            .map(|n| Object::Real(n.parse().unwrap()))
            .collect(),
    ));
    operations.push(lopdf::content::Operation::new("BT", vec![]));
    operations.push(lopdf::content::Operation::new(
        "Tf",
        vec![Object::Name(b"GlyphCourier".to_vec()), Object::Real(f)],
    ));
    operations.push(lopdf::content::Operation::new(
        "rg",
        style.rgb.into_iter().map(Object::Real).collect(),
    ));
    for (i, line) in lines.iter().enumerate() {
        let y = dh - margin - f - i as f32 * f * 1.2;
        operations.push(lopdf::content::Operation::new(
            "Tm",
            vec![
                1.into(),
                0.into(),
                0.into(),
                1.into(),
                Object::Real(margin),
                Object::Real(y),
            ],
        ));
        operations.push(lopdf::content::Operation::new(
            "Tj",
            vec![Object::string_literal(*line)],
        ));
    }
    operations.push(lopdf::content::Operation::new("ET", vec![]));
    operations.push(lopdf::content::Operation::new("Q", vec![]));
    let bytes = lopdf::content::Content { operations }
        .encode()
        .map_err(error)?;
    Ok((
        Stream::new(
            dictionary! {"Type"=>"XObject","Subtype"=>"Form","FormType"=>1,
            "BBox"=>array([0.,0.,w,h]), "Resources"=>dictionary!{"Font"=>dictionary!{"GlyphCourier"=>font()}}},
            bytes,
        ),
        bounds,
        da,
    ))
}
pub(super) fn create(
    doc: &mut Document,
    page: ObjectId,
    rect: PdfRect,
    text: &TextMarkup,
    style: ShapeStyle,
) -> Result<ObjectId, PdfError> {
    let (stream, bounds, da) = appearance(doc, page, rect, text, style)?;
    let ap = doc.add_object(stream);
    Ok(doc.add_object(dictionary!{"Type"=>"Annot", "Subtype"=>"FreeText", "GlyphText"=>1,
        "GlyphNormalizedRect"=>array([rect.x,rect.y,rect.width,rect.height]),
        "GlyphTextSize"=>Object::Real(text.size), "Contents"=>unicode(&text.contents), "DA"=>Object::string_literal(da),
        "Rect"=>array(bounds), "P"=>page,"F"=>4, "Q"=>0,
        "GlyphStrokeRGB"=>style.rgb.into_iter().map(Object::Real).collect::<Vec<_>>(), "GlyphStrokeWeight"=>Object::Real(style.weight),
        "C"=>style.rgb.into_iter().map(Object::Real).collect::<Vec<_>>(), "BS"=>dictionary!{"W"=>Object::Real(style.weight),"S"=>"S"},
        "Border"=>vec![0.into(),0.into(),0.into()], "AP"=>dictionary!{"N"=>ap}}))
}
pub(super) fn read(
    doc: &Document,
    page: ObjectId,
    page_index: usize,
    o: &Object,
) -> Result<ShapeAnnotation, PdfError> {
    let d = rectangles::resolve(doc, o)?.as_dict().map_err(error)?;
    if d.get(b"Type").and_then(Object::as_name).map_err(error)? != b"Annot"
        || d.get(b"P").and_then(Object::as_reference).map_err(error)? != page
    {
        return Err(error("Owned text annotation identity mismatch"));
    }
    if d.get(b"GlyphText")
        .and_then(Object::as_i64)
        .map_err(error)?
        != 1
        || d.get(b"Subtype").and_then(Object::as_name).map_err(error)? != b"FreeText"
        || [
            b"GlyphRectangle".as_slice(),
            b"GlyphEllipse",
            b"GlyphLine",
            b"GlyphArrow",
        ]
        .iter()
        .any(|k| d.has(k))
    {
        return Err(error("Malformed owned text"));
    }
    let n = rectangles::numbers(doc, d.get(b"GlyphNormalizedRect").map_err(error)?)?;
    let rect = PdfRect {
        x: n[0],
        y: n[1],
        width: n[2],
        height: n[3],
    };
    let text = TextMarkup {
        contents: lopdf::decode_text_string(rectangles::resolve(
            doc,
            d.get(b"Contents").map_err(error)?,
        )?)
        .map_err(error)?,
        size: rectangles::resolve(doc, d.get(b"GlyphTextSize").map_err(error)?)?
            .as_float()
            .map_err(error)?,
    };
    let style = rectangles::read_style(doc, d)?;
    let (expected, bounds, da) = appearance(doc, page, rect, &text, style)?;
    if rectangles::numbers(doc, d.get(b"Rect").map_err(error)?)?
        .iter()
        .zip(bounds)
        .any(|(a, b)| (*a - b).abs() > 0.001)
        || lopdf::decode_text_string(d.get(b"DA").map_err(error)?).map_err(error)? != da
    {
        return Err(error("Owned text mapping/DA mismatch"));
    }
    let ap = rectangles::resolve(doc, d.get(b"AP").map_err(error)?)?
        .as_dict()
        .map_err(error)?;
    let stream = rectangles::resolve(doc, ap.get(b"N").map_err(error)?)?
        .as_stream()
        .map_err(error)?;
    rectangles::validate_matrix(doc, &stream.dict)?;
    if stream.get_plain_content().map_err(error)? != expected.content
        || stream
            .dict
            .get(b"Type")
            .and_then(Object::as_name)
            .map_err(error)?
            != b"XObject"
        || stream
            .dict
            .get(b"FormType")
            .and_then(Object::as_i64)
            .map_err(error)?
            != 1
        || stream
            .dict
            .get(b"Subtype")
            .and_then(Object::as_name)
            .map_err(error)?
            != b"Form"
        || rectangles::numbers(doc, stream.dict.get(b"BBox").map_err(error)?)?
            != rectangles::numbers(doc, expected.dict.get(b"BBox").map_err(error)?)?
        || rectangles::resolve(doc, stream.dict.get(b"Resources").map_err(error)?)?
            != expected.dict.get(b"Resources").map_err(error)?
    {
        return Err(error("Owned text appearance mismatch"));
    }
    Ok(ShapeAnnotation {
        object_id: o.as_reference().map_err(error)?,
        page_index,
        rect,
        kind: ShapeKind::Text,
        style,
        endpoints: None,
        line_head: None,
        text: Some(text),
    })
}
