//! Owned PDF Line annotations; all geometry is computed before allocating objects.
use super::{PdfError, ShapeAnnotation, ShapeKind, rectangles};
use crate::core::links::PdfRect;
use lopdf::{Document, Object, ObjectId, Stream, dictionary};

fn error(message: &str) -> PdfError {
    PdfError::Edit(message.into())
}
fn number(n: f32) -> Object {
    Object::Real(n)
}
fn array(n: [f32; 4]) -> Vec<Object> {
    n.into_iter().map(number).collect()
}
pub(super) fn validate(n: [f32; 4]) -> Result<(), PdfError> {
    if n.iter().any(|v| !v.is_finite() || !(0. ..=1.).contains(v)) || (n[0] == n[2] && n[1] == n[3])
    {
        return Err(error("invalid normalized line endpoints"));
    }
    Ok(())
}
struct Geometry {
    points: [f32; 4],
    head: Option<[f32; 4]>,
    bounds: [f32; 4],
    rect: PdfRect,
    normalized_head: Option<[f32; 4]>,
}
fn geometry(
    doc: &Document,
    page: ObjectId,
    n: [f32; 4],
    arrow: bool,
    style: super::ShapeStyle,
) -> Result<Geometry, PdfError> {
    style.validate()?;
    validate(n)?;
    let (b, rotation) = rectangles::page_geometry(doc, page)?;
    let (w, h) = (b[2] - b[0], b[3] - b[1]);
    let point = |x: f32, y: f32| {
        let (u, v) = match rotation {
            0 => (x, 1. - y),
            90 => (y, x),
            180 => (1. - x, y),
            _ => (1. - y, 1. - x),
        };
        [b[0] + u * w, b[1] + v * h]
    };
    let inverse = |x: f32, y: f32| {
        let (u, v) = ((x - b[0]) / w, (y - b[1]) / h);
        match rotation {
            0 => [u, 1. - v],
            90 => [v, u],
            180 => [1. - u, v],
            _ => [1. - v, 1. - u],
        }
    };
    let a = point(n[0], n[1]);
    let z = point(n[2], n[3]);
    let (dx, dy) = (z[0] - a[0], z[1] - a[1]);
    let length = dx.hypot(dy);
    if !length.is_finite() || length == 0. {
        return Err(error("degenerate mapped line"));
    }
    let head = arrow.then(|| {
        let size = 10f32.min(length * 0.4);
        let (ux, uy) = (dx / length, dy / length);
        [
            z[0] - size * ux - size * 0.5 * uy,
            z[1] - size * uy + size * 0.5 * ux,
            z[0] - size * ux + size * 0.5 * uy,
            z[1] - size * uy - size * 0.5 * ux,
        ]
    });
    let mut bounds = [
        a[0].min(z[0]),
        a[1].min(z[1]),
        a[0].max(z[0]),
        a[1].max(z[1]),
    ];
    if let Some(h) = head {
        for p in [[h[0], h[1]], [h[2], h[3]]] {
            bounds[0] = bounds[0].min(p[0]);
            bounds[1] = bounds[1].min(p[1]);
            bounds[2] = bounds[2].max(p[0]);
            bounds[3] = bounds[3].max(p[1]);
        }
    }
    // Half of the 2pt stroke, plus a safe antialias margin. Never clamp to the page.
    bounds = [
        bounds[0] - (style.weight / 2. + 1.),
        bounds[1] - (style.weight / 2. + 1.),
        bounds[2] + (style.weight / 2. + 1.),
        bounds[3] + (style.weight / 2. + 1.),
    ];
    if bounds.iter().any(|v| !v.is_finite()) || bounds[2] <= bounds[0] || bounds[3] <= bounds[1] {
        return Err(error("invalid line bounds"));
    }
    let p = inverse(bounds[0], bounds[1]);
    let q = inverse(bounds[2], bounds[3]);
    let normalized_head = head.map(|h| {
        let p = inverse(h[0], h[1]);
        let q = inverse(h[2], h[3]);
        [p[0], p[1], q[0], q[1]]
    });
    Ok(Geometry {
        points: [a[0], a[1], z[0], z[1]],
        head,
        bounds,
        rect: PdfRect {
            x: p[0].min(q[0]),
            y: p[1].min(q[1]),
            width: (p[0] - q[0]).abs(),
            height: (p[1] - q[1]).abs(),
        },
        normalized_head,
    })
}
fn appearance(g: &Geometry, style: super::ShapeStyle) -> Vec<u8> {
    let [x, y, _, _] = g.bounds;
    let [a, b, c, d] = g.points;
    let mut commands = format!(
        "q 1 0 0 RG 2 w 1 J 1 j {} {} m {} {} l S\n",
        a - x,
        b - y,
        c - x,
        d - y
    );
    if let Some([e, f, h, i]) = g.head {
        commands.push_str(&format!(
            "{} {} m {} {} l {} {} l S\n",
            e - x,
            f - y,
            c - x,
            d - y,
            h - x,
            i - y
        ));
    }
    commands.push_str("Q\n");
    commands
        .replacen("q 1 0 0 RG 2 w", &style.prefix(), 1)
        .into_bytes()
}
pub(super) fn create(
    doc: &mut Document,
    page: ObjectId,
    n: [f32; 4],
    arrow: bool,
) -> Result<ObjectId, PdfError> {
    create_styled(doc, page, n, arrow, super::ShapeStyle::default())
}
pub(super) fn create_styled(
    doc: &mut Document,
    page: ObjectId,
    n: [f32; 4],
    arrow: bool,
    style: super::ShapeStyle,
) -> Result<ObjectId, PdfError> {
    let g = geometry(doc, page, n, arrow, style)?;
    let [x, y, z, t] = g.bounds;
    let ap=doc.add_object(Stream::new(dictionary! {"Type"=>"XObject","Subtype"=>"Form","FormType"=>1,"BBox"=>array([0.,0.,z-x,t-y]),"Resources"=>dictionary!{}},appearance(&g, style)));
    Ok(doc.add_object(dictionary! {"Type"=>"Annot","Subtype"=>"Line",if arrow {"GlyphArrow"} else {"GlyphLine"}=>1,
        "GlyphNormalizedEndpoints"=>array(n),"Rect"=>array(g.bounds),"L"=>array(g.points),
        "LE"=>vec![Object::Name(b"None".to_vec()),Object::Name(if arrow {b"OpenArrow".to_vec()} else {b"None".to_vec()})],
        "P"=>page,"F"=>4,"GlyphStrokeRGB"=>style.rgb.into_iter().map(number).collect::<Vec<_>>(),"GlyphStrokeWeight"=>number(style.weight),
        "C"=>style.rgb.into_iter().map(number).collect::<Vec<_>>(),"BS"=>dictionary!{"W"=>number(style.weight),"S"=>"S"},"AP"=>dictionary!{"N"=>ap}}))
}
pub(super) fn read(
    doc: &Document,
    page: ObjectId,
    page_index: usize,
    o: &Object,
) -> Result<ShapeAnnotation, PdfError> {
    let d = rectangles::resolve(doc, o)?
        .as_dict()
        .map_err(|e| PdfError::Edit(e.to_string()))?;
    let arrow = d.has(b"GlyphArrow");
    if d.has(b"GlyphRectangle") || d.has(b"GlyphEllipse") || (arrow && d.has(b"GlyphLine")) {
        return Err(error("ambiguous owned line"));
    }
    let marker = if arrow {
        b"GlyphArrow".as_slice()
    } else {
        b"GlyphLine".as_slice()
    };
    let err = |e: lopdf::Error| PdfError::Edit(e.to_string());
    if d.get(marker).and_then(Object::as_i64).map_err(err)? != 1
        || d.get(b"Subtype").and_then(Object::as_name).map_err(err)? != b"Line"
    {
        return Err(error("malformed owned line"));
    }
    let n = rectangles::numbers(doc, d.get(b"GlyphNormalizedEndpoints").map_err(err)?)?;
    let style = rectangles::read_style(doc, d)?;
    let g = geometry(doc, page, n, arrow, style)?;
    for (key, expected) in [(b"L".as_slice(), g.points), (b"Rect".as_slice(), g.bounds)] {
        let actual = rectangles::numbers(doc, d.get(key).map_err(err)?)?;
        if actual
            .iter()
            .zip(expected)
            .any(|(a, b)| (*a - b).abs() > 0.001)
        {
            return Err(error("owned line mapping mismatch"));
        }
    }
    let le = rectangles::resolve(doc, d.get(b"LE").map_err(err)?)?
        .as_array()
        .map_err(err)?;
    if le.len() != 2
        || le[0].as_name().map_err(err)? != b"None"
        || le[1].as_name().map_err(err)?
            != if arrow {
                b"OpenArrow".as_slice()
            } else {
                b"None".as_slice()
            }
    {
        return Err(error("contradictory line ending"));
    }
    let ap = rectangles::resolve(doc, d.get(b"AP").map_err(err)?)?
        .as_dict()
        .map_err(err)?;
    let stream = rectangles::resolve(doc, ap.get(b"N").map_err(err)?)?
        .as_stream()
        .map_err(err)?;
    // Canonical owned geometry assumes the PDF default identity Form matrix.
    // Resolve references with cycle/missing-object checks, but never approximate a transform.
    if stream.dict.has(b"Matrix") {
        let matrix = rectangles::resolve(doc, stream.dict.get(b"Matrix").map_err(err)?)?
            .as_array()
            .map_err(err)?;
        if matrix.len() != 6 {
            return Err(error("invalid owned line appearance matrix"));
        }
        for (value, expected) in matrix.iter().zip([1., 0., 0., 1., 0., 0.]) {
            let value = rectangles::resolve(doc, value)?.as_float().map_err(err)?;
            if !value.is_finite() || value != expected {
                return Err(error("invalid owned line appearance matrix"));
            }
        }
    }
    let expected = [0., 0., g.bounds[2] - g.bounds[0], g.bounds[3] - g.bounds[1]];
    let actual = rectangles::numbers(doc, stream.dict.get(b"BBox").map_err(err)?)?;
    if stream
        .dict
        .get(b"Subtype")
        .and_then(Object::as_name)
        .map_err(err)?
        != b"Form"
        || actual
            .iter()
            .zip(expected)
            .any(|(a, b)| (*a - b).abs() > 0.001)
        || stream.get_plain_content().map_err(err)? != appearance(&g, style)
    {
        return Err(error("invalid owned line appearance"));
    }
    Ok(ShapeAnnotation {
        text: None,
        object_id: o.as_reference().map_err(err)?,
        page_index,
        rect: g.rect,
        kind: if arrow {
            ShapeKind::Arrow
        } else {
            ShapeKind::Line
        },
        style,
        endpoints: Some(n),
        line_head: g.normalized_head,
    })
}
