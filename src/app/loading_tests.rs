use super::*;

fn fixture() -> (egui::Context, GlyphApp, mpsc::Sender<RenderJobResult>) {
    let ctx = egui::Context::default();
    let mut app = GlyphApp::with_context(&ctx, None);
    // The real worker exists, but cannot race the controlled completions.
    let (tx, rx) = mpsc::channel();
    app.render_result_rx = rx;
    (ctx, app, tx)
}

fn loaded(app: &mut GlyphApp) {
    app.project.open_document(
        "loading-fixture.pdf".into(),
        crate::pdf::PdfDocumentSummary {
            page_count: 3,
            pages: Vec::new(),
            bookmarks: Vec::new(),
            title: None,
        },
    );
    app.project.selected_page = 1;
    app.pending_page_render = Some(PendingPageRender {
        id: 42,
        path: "loading-fixture.pdf".into(),
        page_index: 1,
        target_width: BASE_RENDER_WIDTH,
    });
}

fn frame(ctx: &egui::Context, app: &mut GlyphApp) -> String {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(960., 640.),
            )),
            ..Default::default()
        },
        |ui| app.draw(ui),
    );
    output.textures_delta.clear();
    fn spinner(shape: &egui::Shape) -> bool {
        match shape {
            egui::Shape::Path(path) => path.points.len() == 25 && path.stroke.width == 2.5,
            egui::Shape::Vec(shapes) => shapes.iter().any(spinner),
            _ => false,
        }
    }
    let expected_spinner = app.page_texture.is_none()
        && (app.loading_document.is_some()
            || app.pending_page_render.is_some()
            || app.edit_pending());
    assert_eq!(
        output.shapes.iter().any(|shape| spinner(&shape.shape)),
        expected_spinner
    );
    fn text(shape: &egui::Shape, out: &mut String) {
        match shape {
            egui::Shape::Text(t) => {
                out.push_str(t.galley.text());
                out.push('\n');
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    text(shape, out);
                }
            }
            _ => {}
        }
    }
    let mut out = String::new();
    for shape in &output.shapes {
        text(&shape.shape, &mut out);
    }
    out
}

#[test]
fn first_document_inspection_displays_opening_not_drop_prompt() {
    let (ctx, mut app, _tx) = fixture();
    app.loading_document = Some(7);
    let text = frame(&ctx, &mut app);
    assert!(text.contains("Opening PDF…"), "{text}");
    assert!(!text.contains("Drop a PDF\n"));
}

#[test]
fn failed_render_completion_is_unavailable_without_false_busy() {
    let (ctx, mut app, tx) = fixture();
    loaded(&mut app);
    assert!(frame(&ctx, &mut app).contains("Loading page 2 of 3…"));
    tx.send(RenderJobResult::Page {
        generation: app.document_generation,
        id: 42,
        path: "loading-fixture.pdf".into(),
        page_index: 1,
        target_width: BASE_RENDER_WIDTH,
        result: Err(PdfError::MissingFile("controlled preview failure".into())),
    })
    .unwrap();
    let text = frame(&ctx, &mut app);
    assert!(app.preview_unavailable());
    assert!(app.pending_page_render.is_none());
    assert!(
        app.persistent_edit_error()
            .unwrap()
            .contains("controlled preview failure")
    );
    assert!(text.contains("Page preview unavailable"), "{text}");
    assert!(!text.contains("Loading page 2 of 3…"));
    assert!(!text.contains("Drop a PDF\n"));
    assert!(frame(&ctx, &mut app).contains("Page preview unavailable"));
}

#[test]
fn successful_render_completion_replaces_loading_with_page_texture() {
    let (ctx, mut app, tx) = fixture();
    loaded(&mut app);
    assert!(frame(&ctx, &mut app).contains("Loading page 2 of 3…"));
    tx.send(RenderJobResult::Page {
        generation: app.document_generation,
        id: 42,
        path: "loading-fixture.pdf".into(),
        page_index: 1,
        target_width: BASE_RENDER_WIDTH,
        result: Ok(RenderedPage {
            page_index: 1,
            width: usize::from(BASE_RENDER_WIDTH),
            height: 3,
            rgba: vec![255; usize::from(BASE_RENDER_WIDTH) * 3 * 4],
        }),
    })
    .unwrap();
    let text = frame(&ctx, &mut app);
    assert_eq!(app.rendered_page.as_ref().unwrap().page_index, 1);
    assert!(app.page_texture.is_some());
    assert!(app.pending_page_render.is_none());
    assert!(!text.contains("Loading page 2 of 3…"));
    assert!(!text.contains("Page preview unavailable"));
    assert!(!text.contains("Drop a PDF\n"));
}

#[test]
fn opening_has_priority_over_pending_page() {
    let (ctx, mut app, _tx) = fixture();
    loaded(&mut app);
    app.loading_document = Some(7);
    let text = frame(&ctx, &mut app);
    assert!(text.contains("Opening PDF…"));
    assert!(!text.contains("Loading page 2 of 3…"));
}

#[test]
fn disconnected_worker_without_canvas_shows_terminal_error_not_drop_or_loading() {
    let (ctx, mut app, tx) = fixture();
    app.loading_document = Some(7);
    drop(tx);
    let text = frame(&ctx, &mut app);
    assert!(text.contains("Renderer stopped"), "{text}");
    assert!(!text.contains("Drop a PDF\n"), "{text}");
    assert!(!text.contains("Opening PDF…"), "{text}");
    app.status = "Unrelated status".into();
    assert!(frame(&ctx, &mut app).contains("Renderer stopped"));
}

#[test]
fn disconnected_worker_with_document_has_no_loading_thumbnail_requests() {
    let (ctx, mut app, tx) = fixture();
    loaded(&mut app);
    drop(tx);
    let text = frame(&ctx, &mut app);
    assert!(!text.contains("Loading preview"), "{text}");
    assert!(app.pending_text.is_none());
    assert!(app.pending_page_render.is_none());
}

#[test]
fn idle_without_document_keeps_drop_prompt() {
    let (ctx, mut app, _tx) = fixture();
    assert!(frame(&ctx, &mut app).contains("Drop a PDF\n"));
}

#[test]
fn loaded_without_pending_or_texture_is_unavailable() {
    let (ctx, mut app, _tx) = fixture();
    loaded(&mut app);
    app.pending_page_render = None;
    app.status = "Rendering page 2".into(); // Status text is not a busy-state signal.
    let text = frame(&ctx, &mut app);
    assert!(text.contains("Page preview unavailable"));
    assert!(!text.contains("Loading page 2 of 3…"));
    assert!(!text.contains("Drop a PDF\n"));
}

#[test]
fn selected_pending_page_displays_loading_not_drop_prompt() {
    let (ctx, mut app, _tx) = fixture();
    loaded(&mut app);
    let text = frame(&ctx, &mut app);
    assert!(text.contains("Loading page 2 of 3…"), "{text}");
    assert!(text.contains("Your PDF is still open. Preparing this page."));
    assert!(!text.contains("Drop a PDF\n"));
}
