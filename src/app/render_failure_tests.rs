use super::*;

fn fixture() -> (egui::Context, GlyphApp, mpsc::Sender<RenderJobResult>) {
    let ctx = egui::Context::default();
    let mut app = GlyphApp::with_context(&ctx, None);
    let (tx, rx) = mpsc::channel();
    app.render_result_rx = rx;
    (ctx, app, tx)
}

fn loaded(app: &mut GlyphApp) {
    app.project.open_document(
        "kept.pdf".into(),
        crate::pdf::PdfDocumentSummary {
            page_count: 2,
            pages: vec![],
            bookmarks: vec![],
            title: None,
        },
    );
    app.editing.dirty = true;
    app.pending_page_render = Some(PendingPageRender {
        id: 7,
        path: "kept.pdf".into(),
        page_index: 0,
        target_width: BASE_RENDER_WIDTH,
    });
}

#[test]
fn dead_worker_rejects_open_and_preview_requests_without_discarding_document() {
    let (ctx, mut app, tx) = fixture();
    loaded(&mut app);
    drop(tx);
    app.apply_render_results(&ctx);
    app.reset_view();
    app.render_worker.reset(99);
    assert!(app.render_worker_failed());
    app.open_pdf("replacement.pdf".into(), &ctx);
    assert!(
        app.loading_document.is_none(),
        "dead worker must not inspect replacements"
    );
    assert_eq!(
        app.project.document.as_ref().unwrap().path,
        PathBuf::from("kept.pdf")
    );
    assert!(app.editing.dirty);
    assert!(
        !app.editing_modal_open(),
        "rejected open must not take over editing"
    );
    app.select_page(1, &ctx);
    assert_eq!(app.project.selected_page, 0);
    app.render_selected_page(&ctx, BASE_RENDER_WIDTH);
    app.queue_page_links(&ctx);
    app.queue_adjacent_page_prefetch();
    assert!(app.pending_page_render.is_none());
    assert!(app.pending_links.is_none());
    assert!(app.pending_prefetch_pages.is_empty());
    assert!(app.status.contains("Renderer stopped"));
}

#[test]
fn queued_success_is_drained_before_disconnect_and_retains_last_page() {
    let (ctx, mut app, tx) = fixture();
    loaded(&mut app);
    tx.send(RenderJobResult::Page {
        generation: app.document_generation,
        id: 7,
        path: "kept.pdf".into(),
        page_index: 0,
        target_width: BASE_RENDER_WIDTH,
        result: Ok(RenderedPage {
            page_index: 0,
            width: 1,
            height: 1,
            rgba: vec![255; 4],
        }),
    })
    .unwrap();
    drop(tx);
    app.apply_render_results(&ctx);
    assert!(app.render_worker_failed());
    assert!(app.page_texture.is_some());
    assert_eq!(app.rendered_page.as_ref().unwrap().rgba, vec![255; 4]);
    assert!(app.editing.dirty);
    assert!(!app.edit_pending());
    assert!(app.pending_prefetch_pages.is_empty());
}

#[test]
fn failed_replacement_then_disconnect_keeps_existing_document() {
    let (ctx, mut app, tx) = fixture();
    loaded(&mut app);
    app.loading_document = Some(8);
    tx.send(RenderJobResult::Inspection {
        id: 8,
        path: "replacement.pdf".into(),
        result: Err(PdfError::MissingFile("controlled".into())),
    })
    .unwrap();
    drop(tx);
    app.apply_render_results(&ctx);
    assert!(app.render_worker_failed());
    assert!(app.loading_document.is_none());
    assert!(app.pending_page_render.is_none());
    assert_eq!(
        app.project.document.as_ref().unwrap().path,
        PathBuf::from("kept.pdf")
    );
    assert!(app.editing.dirty);
}

#[test]
fn live_empty_receiver_is_healthy_but_disconnect_ends_inspection() {
    let (ctx, mut app, tx) = fixture();
    app.loading_document = Some(42);
    app.apply_render_results(&ctx);
    assert_eq!(app.loading_document, Some(42));
    drop(tx);
    app.apply_render_results(&ctx);
    assert_eq!(
        app.loading_document, None,
        "disconnected worker must end inspection"
    );
    assert!(app.status.contains("Renderer stopped"), "{}", app.status);
}
