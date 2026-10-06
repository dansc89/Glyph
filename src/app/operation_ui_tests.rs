use super::*;

fn setup(
    kind: EditKind,
    age: std::time::Duration,
) -> (
    egui::Context,
    GlyphApp,
    mpsc::Sender<RenderJobResult>,
    mpsc::Sender<EditCompletion>,
) {
    let ctx = egui::Context::default();
    let mut app = GlyphApp::with_context(&ctx, None);
    let path = PathBuf::from("controlled-progress-ui.pdf");
    app.project.open_document(
        path.clone(),
        crate::pdf::PdfDocumentSummary {
            page_count: 1,
            pages: Vec::new(),
            bookmarks: Vec::new(),
            title: None,
        },
    );
    let (render_tx, render_rx) = mpsc::channel();
    app.render_result_rx = render_rx;
    let (edit_tx, edit_rx) = mpsc::channel();
    app.editing.pending = Some(PendingEdit {
        kind,
        started: Instant::now().checked_sub(age).unwrap(),
        generation: app.document_generation,
        path,
        receiver: edit_rx,
    });
    app.editing.dirty = true;
    app.status = "Unrelated rendering status".into();
    (ctx, app, render_tx, edit_tx)
}

fn frame(ctx: &egui::Context, app: &mut GlyphApp) -> String {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200., 800.),
            )),
            ..Default::default()
        },
        |ui| app.draw(ui),
    );
    output.textures_delta.clear();
    fn collect(shape: &egui::Shape, text: &mut String) {
        match shape {
            egui::Shape::Text(t) => {
                text.push_str(t.galley.text());
                text.push('\n');
            }
            egui::Shape::Vec(shapes) => {
                for s in shapes {
                    collect(s, text);
                }
            }
            _ => {}
        }
    }
    let mut text = String::new();
    for s in &output.shapes {
        collect(&s.shape, &mut text);
    }
    text
}

#[test]
fn real_draw_footer_reports_saving_despite_unrelated_render_status() {
    let (ctx, mut app, _render_tx, _edit_tx) = setup(EditKind::Save, std::time::Duration::ZERO);
    let text = frame(&ctx, &mut app);
    assert!(text.contains("Saving PDF…"), "{text}");
    assert!(app.edit_pending());
    assert!(app.editing.dirty);
    assert!(app.editing.session.is_none());
}

#[test]
fn real_draw_footer_reports_overdue_work_without_releasing_session_ownership() {
    let (ctx, mut app, _render_tx, _edit_tx) =
        setup(EditKind::Save, std::time::Duration::from_secs(12));
    let text = frame(&ctx, &mut app);
    assert!(text.to_lowercase().contains("still working"), "{text}");
    assert!(app.edit_pending());
    assert!(app.editing.dirty);
    assert!(!app.can_change_markups());
    assert!(app.editing.session.is_none());
}

#[test]
fn real_draw_footer_retains_saved_refresh_state_after_status_is_overwritten() {
    let (ctx, mut app, _render_tx, _edit_tx) = setup(EditKind::Save, std::time::Duration::ZERO);
    app.editing.pending = None;
    app.editing.dirty = false;
    app.markup.preview_pending = true;
    app.editing.saved_preview = Some(SavedPreview {
        generation: app.document_generation,
        path: app.project.document.as_ref().unwrap().path.clone(),
        started: Instant::now(),
    });
    let text = frame(&ctx, &mut app);
    assert!(text.contains("Saved PDF — refreshing preview…"), "{text}");
    assert!(!app.editing.dirty);
    assert!(app.markup.preview_pending);
}

#[test]
fn current_preview_install_releases_stale_saved_marker_after_generation_change() {
    let (ctx, mut app, _render_tx, _edit_tx) = setup(EditKind::Save, std::time::Duration::ZERO);
    app.editing.pending = None;
    app.editing.dirty = false;
    app.markup.preview_pending = true;
    app.editing.saved_preview = Some(SavedPreview {
        generation: app.document_generation,
        path: app.project.document.as_ref().unwrap().path.clone(),
        started: Instant::now(),
    });
    // A failed replacement inspection keeps this document but changes its
    // render generation. Only its newly validated page installation is ready.
    app.document_generation = app.document_generation.wrapping_add(1).max(1);
    app.install_texture(
        &ctx,
        Arc::new(RenderedPage {
            page_index: 0,
            width: BASE_RENDER_WIDTH as usize,
            height: 1,
            rgba: vec![255; BASE_RENDER_WIDTH as usize * 4],
        }),
    );
    assert!(
        !app.markup.preview_pending,
        "current pixels must release the preview gate"
    );
    assert!(app.editing.saved_preview.is_none());
}

#[test]
fn renderer_disconnect_blocks_new_markups_without_falsely_losing_edit_session() {
    let (ctx, mut app, render_tx, _edit_tx) = setup(EditKind::Edit, std::time::Duration::ZERO);
    app.editing.pending = None;
    app.markup.loaded = true;
    app.markup.preview_pending = false;
    drop(render_tx);
    app.apply_render_results(&ctx);
    assert!(app.render_worker_failed());
    assert!(
        !app.can_change_markups(),
        "cannot edit invisible changes with a dead renderer"
    );
    assert!(
        !app.editing.unrecoverable,
        "renderer loss is not editable-session loss"
    );
    assert!(app.editing.dirty);
}

#[test]
fn stale_page_completion_cannot_release_current_saved_preview() {
    let (ctx, mut app, render_tx, _edit_tx) = setup(EditKind::Save, std::time::Duration::ZERO);
    app.editing.pending = None;
    app.markup.preview_pending = true;
    let path = app.project.document.as_ref().unwrap().path.clone();
    app.editing.saved_preview = Some(SavedPreview {
        generation: app.document_generation,
        path: path.clone(),
        started: Instant::now(),
    });
    app.pending_page_render = Some(PendingPageRender {
        id: 999,
        path: path.clone(),
        page_index: 0,
        target_width: BASE_RENDER_WIDTH,
    });
    render_tx
        .send(RenderJobResult::Page {
            id: 998,
            generation: app.document_generation.wrapping_sub(1),
            path,
            page_index: 0,
            target_width: BASE_RENDER_WIDTH,
            result: Ok(RenderedPage {
                page_index: 0,
                width: 2,
                height: 2,
                rgba: vec![255; 16],
            }),
        })
        .unwrap();
    app.apply_render_results(&ctx);
    assert!(app.markup.preview_pending && app.editing.saved_preview.is_some());
    assert_eq!(app.pending_page_render.as_ref().unwrap().id, 999);
}

#[test]
fn dead_renderer_rejects_direct_metadata_mutation_before_moving_session() {
    let (ctx, mut app, render_tx, _edit_tx) = setup(EditKind::Edit, std::time::Duration::ZERO);
    app.editing.pending = None;
    drop(render_tx);
    app.apply_render_results(&ctx);
    app.start_edit(
        Command::PageLabel {
            index: 0,
            original: "1".into(),
            title: "2".into(),
        },
        &ctx,
    );
    assert!(
        !app.edit_pending(),
        "direct mutation must be gated, not only toolbar buttons"
    );
    assert!(app.editing.dirty && !app.editing.unrecoverable);
}

#[test]
fn real_draw_footer_keeps_persistent_failure_ahead_of_progress() {
    let (ctx, mut app, _render_tx, _edit_tx) = setup(EditKind::Save, std::time::Duration::ZERO);
    app.editing.error = Some("SAVE FAILURE SENTINEL".into());
    let text = frame(&ctx, &mut app);
    assert!(text.contains("SAVE FAILURE SENTINEL"), "{text}");
    assert_eq!(app.persistent_edit_error(), Some("SAVE FAILURE SENTINEL"));
}
