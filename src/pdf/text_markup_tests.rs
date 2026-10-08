use crate::core::links::PdfRect;
fn text_rect() -> PdfRect {
    PdfRect {
        x: 0.1,
        y: 0.1,
        width: 0.7,
        height: 0.3,
    }
}
fn text_ap(s: &EditablePdf, id: ObjectId) -> ObjectId {
    s.doc
        .get_object(id)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"AP")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"N")
        .unwrap()
        .as_reference()
        .unwrap()
}
#[test]
fn text_pdf_rotated_crop_userunit_physical_font_and_independent_render() {
    use crate::pdf::{PdfRenderEngine, PdfiumRenderEngine};
    let dir = tempfile::tempdir().unwrap();
    for rotation in [0, 90, 180, 270] {
        for u in [0.5, 1., 2., 10.] {
            let path = dir.path().join(format!("text-{rotation}-{u}.pdf"));
            fixture(&path);
            let mut d = Document::load(&path).unwrap();
            let page = *d.get_pages().values().next().unwrap();
            let p = d.get_object_mut(page).unwrap().as_dict_mut().unwrap();
            p.set("Rotate", rotation);
            p.set("UserUnit", Object::Real(u));
            d.save(&path).unwrap();
            let baseline = PdfiumRenderEngine.render_page(&path, 0, 2048).unwrap();
            let mut s = EditablePdf::open(&path).unwrap();
            s.add_text(
                0,
                text_rect(),
                "Text",
                12.,
                super::super::ShapeStyle::default(),
            )
            .unwrap();
            let shape = s.shapes()[0].clone();
            let ap = text_ap(&s, shape.object_id);
            let stream = s.doc.get_object(ap).unwrap().as_stream().unwrap();
            assert!(!stream.dict.has(b"Matrix"));
            let ops = lopdf::content::Content::decode(&stream.content)
                .unwrap()
                .operations;
            let tf = ops.iter().find(|o| o.operator == "Tf").unwrap();
            assert_eq!(tf.operands[1].as_float().unwrap() * u, 12.);
            s.save().unwrap();
            let image = PdfiumRenderEngine.render_page(&path, 0, 2048).unwrap();
            let mut red = 0;
            for y in 0..image.height {
                for x in 0..image.width {
                    let i = ((y * image.width + x) * 4) as usize;
                    let c = &image.rgba[i..i + 4];
                    if c[0] > 150 && c[1] < 120 && c[2] < 120 {
                        red += 1;
                        assert!(
                            x as f32 >= shape.rect.x * image.width as f32 - 2.
                                && x as f32
                                    <= (shape.rect.x + shape.rect.width) * image.width as f32 + 2.
                        );
                        assert!(
                            y as f32 >= shape.rect.y * image.height as f32 - 2.
                                && y as f32
                                    <= (shape.rect.y + shape.rect.height) * image.height as f32
                                        + 2.
                        );
                    }
                    let inside = x as f32 >= shape.rect.x * image.width as f32 - 2.
                        && x as f32 <= (shape.rect.x + shape.rect.width) * image.width as f32 + 2.
                        && y as f32 >= shape.rect.y * image.height as f32 - 2.
                        && y as f32
                            <= (shape.rect.y + shape.rect.height) * image.height as f32 + 2.;
                    if !inside {
                        assert_eq!(&image.rgba[i..i + 4], &baseline.rgba[i..i + 4]);
                    }
                }
            }
            assert!(
                red > 0,
                "real Courier ink must render at rotation {rotation} UserUnit {u}"
            );
            assert_eq!(EditablePdf::open(&path).unwrap().shapes(), [shape]);
        }
    }
}
#[test]
fn text_pdf_rejects_unsupported_encoding_overflow_and_nonfinite_without_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("invalid-text.pdf");
    fixture(&path);
    let mut s = EditablePdf::open(&path).unwrap();
    let before = s.doc.objects.clone();
    for (text, size) in [
        ("caf\u{e9}", 12.),
        ("emoji \u{1f600}", 12.),
        ("tab\t", 12.),
        ("\r", 12.),
        ("", 12.),
        ("valid", f32::NAN),
        ("valid", f32::INFINITY),
        ("valid", 0.),
        ("valid", 145.),
        (
            "Long line that does not fit this tiny crop area at huge size",
            144.,
        ),
    ] {
        assert!(
            s.add_text(
                0,
                text_rect(),
                text,
                size,
                super::super::ShapeStyle::default()
            )
            .is_err()
        );
        assert_eq!(s.doc.objects, before);
        assert!(!s.can_undo());
        assert!(!s.is_dirty());
    }
}
#[test]
fn text_pdf_shared_appearance_cow_preserves_foreign_metadata_and_undo() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shared-text.pdf");
    fixture(&path);
    let mut s = EditablePdf::open(&path).unwrap();
    s.add_text(
        0,
        text_rect(),
        "Original",
        12.,
        super::super::ShapeStyle::default(),
    )
    .unwrap();
    let mut shape = s.shapes()[0].clone();
    let ap = text_ap(&s, shape.object_id);
    let original_stream = s.doc.objects[&ap].clone();
    s.doc
        .get_object_mut(shape.object_id)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("NM", Object::string_literal("Opaque identity"));
    let foreign=s.doc.add_object(dictionary!{"Type"=>"Annot","Subtype"=>"FreeText","Contents"=>unicode_title("Foreign Unicode \u{65e5}"),"Rect"=>vec![10.into(),10.into(),50.into(),50.into()],"AP"=>dictionary!{"N"=>ap},"Custom"=>Object::string_literal("preserve")});
    let page = *s.doc.get_pages().values().next().unwrap();
    let (_, mut a) = super::super::rectangles::annots(&s.doc, page).unwrap();
    a.push(foreign.into());
    s.doc
        .get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", a);
    let foreign_before = s.doc.objects[&foreign].clone();
    shape.text.as_mut().unwrap().contents = "Edited".into();
    shape.text.as_mut().unwrap().size = 18.;
    shape.rect.x += 0.05;
    assert!(s.update_shape(&shape).unwrap());
    let replacement = text_ap(&s, shape.object_id);
    assert_ne!(replacement, ap);
    assert_eq!(s.doc.objects[&ap], original_stream);
    assert_eq!(s.doc.objects[&foreign], foreign_before);
    assert_eq!(
        lopdf::decode_text_string(
            s.doc
                .get_object(shape.object_id)
                .unwrap()
                .as_dict()
                .unwrap()
                .get(b"NM")
                .unwrap()
        )
        .unwrap(),
        "Opaque identity"
    );
    assert!(s.undo());
    assert!(!s.doc.objects.contains_key(&replacement));
    assert_eq!(text_ap(&s, shape.object_id), ap);
    assert!(s.redo());
    assert_eq!(s.shapes(), [shape.clone()]);
    s.save().unwrap();
    let reopened = EditablePdf::open(&path).unwrap();
    assert_eq!(reopened.shapes(), [shape]);
    assert_eq!(reopened.doc.objects[&foreign], foreign_before);
    assert!(serialized_object_equal(
        &original_stream,
        &reopened.doc.objects[&ap]
    ));
}
#[test]
fn text_pdf_wrong_form_type_and_parent_identity_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("type-text.pdf");
    fixture(&path);
    let mut s = EditablePdf::open(&path).unwrap();
    s.add_text(
        0,
        text_rect(),
        "Identity",
        12.,
        super::super::ShapeStyle::default(),
    )
    .unwrap();
    let id = s.shapes()[0].object_id;
    let ap = text_ap(&s, id);
    let original = s.doc.clone();
    s.doc
        .get_object_mut(ap)
        .unwrap()
        .as_stream_mut()
        .unwrap()
        .dict
        .set("Type", "Font");
    assert!(
        super::super::rectangles::read(&s.doc).is_err(),
        "wrong form Type must fail ownership validation"
    );
    s.doc = original.clone();
    s.doc
        .get_object_mut(ap)
        .unwrap()
        .as_stream_mut()
        .unwrap()
        .dict
        .set("FormType", 2);
    assert!(super::super::rectangles::read(&s.doc).is_err());
    s.doc = original;
    s.doc
        .get_object_mut(id)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("P", (999, 0));
    assert!(super::super::rectangles::read(&s.doc).is_err());
}
#[test]
fn text_pdf_nonidentity_appearance_matrix_is_rejected_but_foreign_is_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("matrix-text.pdf");
    fixture(&path);
    let mut s = EditablePdf::open(&path).unwrap();
    s.add_text(
        0,
        text_rect(),
        "Matrix",
        12.,
        super::super::ShapeStyle::default(),
    )
    .unwrap();
    let ap = text_ap(&s, s.shapes()[0].object_id);
    for value in [Object::Real(f32::NAN), 2.into()] {
        s.doc
            .get_object_mut(ap)
            .unwrap()
            .as_stream_mut()
            .unwrap()
            .dict
            .set(
                "Matrix",
                vec![value, 0.into(), 0.into(), 1.into(), 0.into(), 0.into()],
            );
        assert!(super::super::rectangles::read(&s.doc).is_err());
        assert!(s.is_dirty());
    }
}
