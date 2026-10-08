fn text_key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}
// Production draw and real retained PDF / renderer; no desktop launch.
fn text_draw_frame(
    app: &mut GlyphApp,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> Option<egui::Rect> {
    let mut out = ctx.run_ui(
        egui::RawInput {
            max_texture_side: Some(8192),
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200., 1000.),
            )),
            events,
            ..Default::default()
        },
        |ui| app.draw(ui),
    );
    let page = app.page_texture.as_ref().and_then(|texture| {
        out.shapes.iter().find_map(|s| match &s.shape {
            egui::epaint::Shape::Mesh(m) if m.texture_id == texture.id() => Some(m.calc_bounds()),
            _ => None,
        })
    });
    out.textures_delta.clear();
    page
}
fn text_begin_fixture(path: &std::path::Path, ctx: &egui::Context) -> GlyphApp {
    let mut app = setup(path, ctx);
    text_draw_frame(&mut app, ctx, vec![]);
    app.load_markups(ctx);
    settle(&mut app, ctx);
    wait_preview(&mut app, ctx);
    app.zoom = 0.5;
    app.pan = egui::Vec2::ZERO;
    app.markup.mode = markup::Mode::Text;
    let mut page = None;
    for _ in 0..4 {
        page = text_draw_frame(&mut app, ctx, vec![]);
    }
    let pos = page.unwrap().min + page.unwrap().size() * 0.2;
    text_draw_frame(
        &mut app,
        ctx,
        vec![egui::Event::PointerMoved(pos), pointer(pos, true)],
    );
    text_draw_frame(&mut app, ctx, vec![pointer(pos, false)]);
    for _ in 0..2 {
        text_draw_frame(&mut app, ctx, vec![]);
    }
    assert!(app.markup.text_draft.is_some());
    app
}
#[test]
fn text_safety_inline_close_open_shortcuts_explain_retained_owner() {
    for key in [egui::Key::W, egui::Key::O] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shortcut-owner.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = text_begin_fixture(&path, &ctx);
        text_draw_frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::Text("Keep me".into()),
                text_key(key, egui::Modifiers::COMMAND),
            ],
        );
        assert!(app.markup.text_draft.is_some());
        assert!(!app.editing.dirty);
        assert!(app.editing.transition.is_none());
        assert!(
            app.status.contains("Apply") && app.status.contains("Cancel"),
            "closing/opening shortcut must explain the retained draft: {}",
            app.status
        );
    }
}
#[test]
fn text_safety_existing_unsaved_decision_keeps_and_saves_draft() {
    for (save, invalid) in [false, true].into_iter().flat_map(|save| {
        [false, true]
            .into_iter()
            .map(move |invalid| (save, invalid))
    }) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("decision.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = text_begin_fixture(&path, &ctx);
        text_draw_frame(
            &mut app,
            &ctx,
            vec![egui::Event::Text(
                if invalid {
                    "Invalid \u{2603}"
                } else {
                    "Decision draft"
                }
                .into(),
            )],
        );
        // Defensive coexistence: transitions can predate draft ownership validation.
        app.editing.transition = Some(Transition::CloseDocument);
        text_draw_frame(&mut app, &ctx, vec![]);
        assert!(
            app.markup.text_draft.is_some(),
            "an existing unsaved decision must not Cancel text"
        );
        if !save && invalid {
            text_draw_frame(
                &mut app,
                &ctx,
                vec![text_key(egui::Key::Escape, egui::Modifiers::NONE)],
            );
        } else {
            text_click_access(&mut app, &ctx, if save { "Save" } else { "Cancel" });
        }
        for _ in 0..4 {
            text_draw_frame(&mut app, &ctx, vec![]);
            settle(&mut app, &ctx);
        }
        if save && !invalid {
            assert!(app.project.document.is_none());
            assert_eq!(
                EditablePdf::open(&path).unwrap().shapes()[0]
                    .text
                    .as_ref()
                    .unwrap()
                    .contents,
                "Decision draft"
            );
        } else {
            assert!(app.project.document.is_some());
            assert!(app.markup.text_draft.is_some());
            assert!(app.editing.transition.is_none());
            assert!(EditablePdf::open(&path).unwrap().shapes().is_empty());
            assert!(!app.edit_pending() && !app.editing.dirty);
            if save {
                let out = text_access_frame(&mut app, &ctx, vec![], egui::vec2(1200., 1000.));
                text_access_bounds(&out, "Edit text on PDF page");
                text_access_bounds(&out, "Apply text");
            }
        }
    }
}
#[test]
fn text_safety_destructive_routes_retain_clean_and_dirty_drafts() {
    for dirty in [false, true] {
        for route in 0..3 {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("draft-close.pdf");
            fixture(&path);
            let ctx = egui::Context::default();
            let mut app = text_begin_fixture(&path, &ctx);
            if dirty {
                app.cancel_inline_text(&ctx);
                app.add_rectangle(
                    0,
                    crate::core::links::PdfRect {
                        x: 0.7,
                        y: 0.7,
                        width: 0.1,
                        height: 0.1,
                    },
                    &ctx,
                );
                settle(&mut app, &ctx);
                wait_preview(&mut app, &ctx);
                app.markup.mode = markup::Mode::Text;
                let page = text_draw_frame(&mut app, &ctx, vec![]).unwrap();
                let pos = page.min + page.size() * 0.2;
                text_draw_frame(
                    &mut app,
                    &ctx,
                    vec![egui::Event::PointerMoved(pos), pointer(pos, true)],
                );
                text_draw_frame(&mut app, &ctx, vec![pointer(pos, false)]);
                text_draw_frame(&mut app, &ctx, vec![]);
            }
            text_draw_frame(
                &mut app,
                &ctx,
                vec![egui::Event::Text("Retain this draft".into())],
            );
            match route {
                0 => {
                    let mut raw = egui::RawInput {
                        max_texture_side: Some(8192),
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1200., 1000.),
                        )),
                        ..Default::default()
                    };
                    raw.viewports
                        .entry(egui::ViewportId::ROOT)
                        .or_default()
                        .events
                        .push(egui::ViewportEvent::Close);
                    let mut out = ctx.run_ui(raw, |ui| app.draw(ui));
                    let cancelled = out.viewport_output[&egui::ViewportId::ROOT]
                        .commands
                        .iter()
                        .any(|c| matches!(c, egui::ViewportCommand::CancelClose));
                    out.textures_delta.clear();
                    assert!(cancelled, "native close must retain draft, dirty={dirty}");
                }
                1 => {
                    app.request_close_document(&ctx);
                    text_draw_frame(&mut app, &ctx, vec![]);
                }
                _ => {
                    app.open_pdf(dir.path().join("replacement.pdf"), &ctx);
                    text_draw_frame(&mut app, &ctx, vec![]);
                }
            }
            assert!(
                app.markup.text_draft.is_some(),
                "draft lost: dirty={dirty} route={route}"
            );
            assert!(app.editing.transition.is_none());
            assert_eq!(
                app.editing.dirty, dirty,
                "draft must not fake committed dirty"
            );
            assert!(
                app.status.contains("Apply") && app.status.contains("Cancel"),
                "{}",
                app.status
            );
            assert!(app.finish_inline_text(&ctx));
            settle(&mut app, &ctx);
            wait_preview(&mut app, &ctx);
            assert!(app.markup.items.iter().any(|a| {
                a.text
                    .as_ref()
                    .is_some_and(|t| t.contents == "Retain this draft")
            }));
        }
    }
}
#[derive(Debug)]
struct TextDroppedPdf(PathBuf);
impl egui::DroppedFile for TextDroppedPdf {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}
fn text_reopen_draft(app: &mut GlyphApp, ctx: &egui::Context, fraction: f32) {
    app.markup.mode = markup::Mode::Text;
    let page = text_draw_frame(app, ctx, vec![]).unwrap();
    let pos = page.min + page.size() * fraction;
    text_draw_frame(
        app,
        ctx,
        vec![egui::Event::PointerMoved(pos), pointer(pos, true)],
    );
    text_draw_frame(app, ctx, vec![pointer(pos, false)]);
    text_draw_frame(app, ctx, vec![]);
    assert!(app.markup.text_draft.is_some());
}
#[test]
fn text_safety_reedit_and_drop_close_open_matrix() {
    for dirty in [false, true] {
        for reedit in [false, true] {
            for route in 0..4 {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("matrix.pdf");
                fixture(&path);
                let replacement = dir.path().join("replacement.pdf");
                fixture(&replacement);
                let ctx = egui::Context::default();
                let mut app = text_begin_fixture(&path, &ctx);
                text_draw_frame(&mut app, &ctx, vec![egui::Event::Text("Original".into())]);
                assert!(app.finish_inline_text(&ctx));
                settle(&mut app, &ctx);
                wait_preview(&mut app, &ctx);
                if !dirty {
                    app.start_edit(Command::Save, &ctx);
                    settle(&mut app, &ctx);
                    wait_preview(&mut app, &ctx);
                }
                let before = std::fs::read(&path).unwrap();
                let shapes = app.markup.items.clone();
                let generation = app.document_generation;
                text_reopen_draft(&mut app, &ctx, if reedit { 0.2 } else { 0.65 });
                text_draw_frame(
                    &mut app,
                    &ctx,
                    vec![
                        text_key(egui::Key::A, egui::Modifiers::COMMAND),
                        egui::Event::Text("Retained replacement".into()),
                    ],
                );
                match route {
                    0 => {
                        let mut raw = egui::RawInput {
                            max_texture_side: Some(8192),
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(1200., 1000.),
                            )),
                            ..Default::default()
                        };
                        raw.viewports
                            .entry(egui::ViewportId::ROOT)
                            .or_default()
                            .events
                            .push(egui::ViewportEvent::Close);
                        let mut out = ctx.run_ui(raw, |ui| app.draw(ui));
                        assert!(
                            out.viewport_output[&egui::ViewportId::ROOT]
                                .commands
                                .iter()
                                .any(|c| matches!(c, egui::ViewportCommand::CancelClose))
                        );
                        out.textures_delta.clear();
                    }
                    1 => {
                        text_click_access(&mut app, &ctx, "Document");
                        text_click_access(&mut app, &ctx, "Close document Ctrl+W");
                    }
                    2 => {
                        let mut out = ctx.run_ui(
                            egui::RawInput {
                                max_texture_side: Some(8192),
                                screen_rect: Some(egui::Rect::from_min_size(
                                    egui::Pos2::ZERO,
                                    egui::vec2(1200., 1000.),
                                )),
                                dropped_files: vec![Arc::new(TextDroppedPdf(replacement.clone()))],
                                ..Default::default()
                            },
                            |ui| app.draw(ui),
                        );
                        out.textures_delta.clear();
                    }
                    _ => {
                        app.choose_pdf(&ctx);
                        text_draw_frame(&mut app, &ctx, vec![]);
                    }
                }
                assert!(
                    app.markup.text_draft.is_some(),
                    "dirty={dirty} reedit={reedit} route={route}"
                );
                assert_eq!(app.document_generation, generation);
                assert!(app.loading_document.is_none());
                assert!(app.editing.transition.is_none());
                assert_eq!(app.editing.dirty, dirty);
                assert_eq!(app.markup.items, shapes);
                assert_eq!(std::fs::read(&path).unwrap(), before);
                let out = text_access_frame(&mut app, &ctx, vec![], egui::vec2(1200., 1000.));
                text_access_bounds(
                    &out,
                    "Apply text or Cancel the text draft before closing or opening another PDF.",
                );
                text_click_access(&mut app, &ctx, "Cancel");
                assert!(app.markup.text_draft.is_none());
                assert_eq!(app.markup.items, shapes);
            }
        }
    }
}
fn text_access_frame(
    app: &mut GlyphApp,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    size: egui::Vec2,
) -> egui::FullOutput {
    ctx.enable_accesskit();
    let mut out = ctx.run_ui(
        egui::RawInput {
            max_texture_side: Some(8192),
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ui| app.draw(ui),
    );
    out.textures_delta.clear();
    out
}
fn text_access_bounds(out: &egui::FullOutput, name: &str) -> egui::Rect {
    let node = out
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some(name) || n.value() == Some(name))
        .unwrap_or_else(|| {
            panic!(
                "missing accessible control {name}; nodes={:?}",
                out.platform_output.accesskit_update
            )
        });
    let r = node.1.bounds().unwrap();
    egui::Rect::from_min_max(
        egui::pos2(r.x0 as f32, r.y0 as f32),
        egui::pos2(r.x1 as f32, r.y1 as f32),
    )
}
fn text_click_access(app: &mut GlyphApp, ctx: &egui::Context, name: &str) -> egui::FullOutput {
    let size = egui::vec2(1200., 1000.);
    let out = text_access_frame(app, ctx, vec![], size);
    let pos = text_access_bounds(&out, name).center();
    text_access_frame(
        app,
        ctx,
        vec![egui::Event::PointerMoved(pos), pointer(pos, true)],
        size,
    );
    text_access_frame(app, ctx, vec![pointer(pos, false)], size)
}
#[test]
fn text_safety_large_multiline_editor_keeps_controls_inside_canvas() {
    for payload in ["Line\n".repeat(100), "x\n".repeat(5000)] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bounded.pdf");
        fixture(&path);
        let before = std::fs::read(&path).unwrap();
        let ctx = egui::Context::default();
        let mut app = text_begin_fixture(&path, &ctx);
        // Type the maximum supported size through the actual DragValue editor.
        text_click_access(&mut app, &ctx, "18 pt");
        text_draw_frame(
            &mut app,
            &ctx,
            vec![
                text_key(egui::Key::A, egui::Modifiers::COMMAND),
                egui::Event::Text("144".into()),
                text_key(egui::Key::Enter, egui::Modifiers::NONE),
            ],
        );
        ctx.memory_mut(|m| m.request_focus(egui::Id::new("inline-markup-text")));
        text_draw_frame(
            &mut app,
            &ctx,
            vec![egui::Event::Paste(payload.clone()), save_key(false)],
        );
        assert!(app.markup.text_draft.is_some());
        assert!(!app.edit_pending());
        let size = egui::vec2(960., 640.);
        for _ in 0..3 {
            text_access_frame(&mut app, &ctx, vec![], size);
        }
        let out = text_access_frame(&mut app, &ctx, vec![], size);
        let nodes = &out.platform_output.accesskit_update.as_ref().unwrap().nodes;
        let canvas = nodes
            .iter()
            .filter_map(|(_, n)| n.bounds())
            .filter(|r| r.x0 > 220. && r.x1 <= 960. && r.y1 <= 640.)
            .max_by(|a, b| {
                ((a.x1 - a.x0) * (a.y1 - a.y0)).total_cmp(&((b.x1 - b.x0) * (b.y1 - b.y0)))
            })
            .unwrap();
        let canvas = egui::Rect::from_min_max(
            egui::pos2(canvas.x0 as f32, canvas.y0 as f32),
            egui::pos2(canvas.x1 as f32, canvas.y1 as f32),
        );
        for name in [
            "Edit text on PDF page",
            "Font size (pt)",
            "144 pt",
            "Apply text",
            "Cancel",
        ] {
            let r = text_access_bounds(&out, name);
            assert!(
                r.is_finite() && canvas.contains_rect(r),
                "{name} inaccessible: {r:?} outside canvas {canvas:?}"
            );
        }
        let text = nodes
            .iter()
            .find(|(_, n)| n.value() == Some(payload.as_str()))
            .expect("full untruncated draft accessible");
        assert_eq!(text.1.value(), Some(payload.as_str()));
        let error = nodes
            .iter()
            .find(|(_, n)| {
                n.role() == egui::accesskit::Role::Label
                    && n.value().is_some_and(|v| v.contains("fit"))
            })
            .expect("fit rejection remains visible");
        let r = error.1.bounds().unwrap();
        assert!(canvas.contains_rect(egui::Rect::from_min_max(
            egui::pos2(r.x0 as f32, r.y0 as f32),
            egui::pos2(r.x1 as f32, r.y1 as f32)
        )));
        eprintln!(
            "TEXT_BOUNDED_UI bytes={} font=144 canvas={canvas:?} apply={:?} cancel={:?} error={r:?}",
            payload.len(),
            text_access_bounds(&out, "Apply text"),
            text_access_bounds(&out, "Cancel")
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
}
#[test]
fn text_safety_real_menu_save_click_finishes_clean_draft() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("menu-owned.pdf");
    fixture(&path);
    let ctx = egui::Context::default();
    let mut app = text_begin_fixture(&path, &ctx);
    text_draw_frame(
        &mut app,
        &ctx,
        vec![egui::Event::Text("Real menu Save".into())],
    );
    text_click_access(&mut app, &ctx, "Document");
    let out = text_click_access(&mut app, &ctx, "Save Ctrl+S");
    assert!(
        out.platform_output.events.iter().any(|e| matches!(
            e,
            egui::output::OutputEvent::Clicked(_)
        ) && e.widget_info().label.as_deref()
            == Some("Save Ctrl+S")
            && e.widget_info().enabled),
        "real menu Save must be enabled and clicked for an owned clean draft"
    );
    settle(&mut app, &ctx);
    wait_preview(&mut app, &ctx);
    let saved = EditablePdf::open(&path).unwrap().shapes();
    assert_eq!(saved[0].text.as_ref().unwrap().contents, "Real menu Save");
    text_reopen_draft(&mut app, &ctx, 0.2);
    text_draw_frame(
        &mut app,
        &ctx,
        vec![
            text_key(egui::Key::A, egui::Modifiers::COMMAND),
            egui::Event::Text("Applied then saved".into()),
        ],
    );
    text_click_access(&mut app, &ctx, "Apply text");
    settle(&mut app, &ctx);
    wait_preview(&mut app, &ctx);
    assert!(app.editing.dirty && app.markup.text_draft.is_none());
    text_click_access(&mut app, &ctx, "Document");
    let out = text_click_access(&mut app, &ctx, "Save Ctrl+S");
    assert!(out.platform_output.events.iter().any(|e| matches!(
        e,
        egui::output::OutputEvent::Clicked(_)
    ) && e.widget_info().label.as_deref()
        == Some("Save Ctrl+S")
        && e.widget_info().enabled));
    settle(&mut app, &ctx);
    wait_preview(&mut app, &ctx);
    assert_eq!(
        EditablePdf::open(&path).unwrap().shapes()[0]
            .text
            .as_ref()
            .unwrap()
            .contents,
        "Applied then saved"
    );
}
#[test]
fn text_safety_document_history_cannot_steal_inline_owner() {
    for redo in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history-owner.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = text_begin_fixture(&path, &ctx);
        text_draw_frame(&mut app, &ctx, vec![egui::Event::Text("First".into())]);
        assert!(app.finish_inline_text(&ctx));
        settle(&mut app, &ctx);
        wait_preview(&mut app, &ctx);
        app.add_rectangle(
            0,
            crate::core::links::PdfRect {
                x: 0.7,
                y: 0.7,
                width: 0.1,
                height: 0.1,
            },
            &ctx,
        );
        settle(&mut app, &ctx);
        wait_preview(&mut app, &ctx);
        app.start_edit(Command::Undo, &ctx);
        settle(&mut app, &ctx);
        wait_preview(&mut app, &ctx);
        assert!(app.editing.session.as_ref().unwrap().can_undo());
        assert!(app.editing.session.as_ref().unwrap().can_redo());
        let shapes = app.markup.items.clone();
        app.markup.mode = markup::Mode::Text;
        let page = text_draw_frame(&mut app, &ctx, vec![]).unwrap();
        let pos = page.min + page.size() * 0.2;
        text_draw_frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(pos), pointer(pos, true)],
        );
        text_draw_frame(&mut app, &ctx, vec![pointer(pos, false)]);
        text_draw_frame(&mut app, &ctx, vec![]);
        text_draw_frame(&mut app, &ctx, vec![egui::Event::Text(" local".into())]);
        let inline_value = |out: &egui::FullOutput| {
            out.platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .find(|(_, n)| n.role() == egui::accesskit::Role::MultilineTextInput)
                .unwrap()
                .1
                .value()
                .unwrap()
                .to_owned()
        };
        let prior = inline_value(&text_access_frame(
            &mut app,
            &ctx,
            vec![],
            egui::vec2(1200., 1000.),
        ));
        let undone = inline_value(&text_access_frame(
            &mut app,
            &ctx,
            vec![text_key(egui::Key::Z, egui::Modifiers::COMMAND)],
            egui::vec2(1200., 1000.),
        ));
        assert_ne!(
            undone, prior,
            "Ctrl+Z belongs to the real text editor stack"
        );
        assert_eq!(app.markup.items, shapes);
        let redone = inline_value(&text_access_frame(
            &mut app,
            &ctx,
            vec![text_key(
                egui::Key::Z,
                egui::Modifiers {
                    shift: true,
                    ..egui::Modifiers::COMMAND
                },
            )],
            egui::vec2(1200., 1000.),
        ));
        assert_eq!(redone, prior);
        text_click_access(&mut app, &ctx, "Document");
        let menu = text_access_frame(&mut app, &ctx, vec![], egui::vec2(1200., 1000.));
        for name in ["Undo Ctrl+Z", "Redo Ctrl+Shift+Z"] {
            let node = menu
                .platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .find(|(_, n)| n.label() == Some(name))
                .unwrap();
            assert!(
                node.1.is_disabled(),
                "document {name} must be disabled while TextEdit owns input"
            );
        }
        text_click_access(&mut app, &ctx, "Document");
        app.start_edit_with_serializer(
            if redo { Command::Redo } else { Command::Undo },
            &ctx,
            EditablePdf::render_snapshot,
        );
        text_draw_frame(&mut app, &ctx, vec![]);
        assert!(
            !app.edit_pending(),
            "document history must not dispatch while inline input owns TextEdit"
        );
        assert!(app.markup.text_draft.is_some());
        assert_eq!(app.markup.items, shapes);
        assert!(app.editing.session.as_ref().unwrap().can_undo());
        assert!(app.editing.session.as_ref().unwrap().can_redo());
    }
}
#[test]
fn text_safety_shift_save_as_same_frame_preserves_source_and_draft() {
    for (accepted, dirty, menu) in [false, true].into_iter().flat_map(|accepted| {
        [false, true].into_iter().flat_map(move |dirty| {
            [false, true]
                .into_iter()
                .map(move |menu| (accepted, dirty, menu))
        })
    }) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shift-source.pdf");
        let copy = dir.path().join("shift-copy.pdf");
        fixture(&path);
        let before = std::fs::read(&path).unwrap();
        let ctx = egui::Context::default();
        let mut app = text_begin_fixture(&path, &ctx);
        if dirty {
            text_draw_frame(&mut app, &ctx, vec![egui::Event::Text("Seed".into())]);
            assert!(app.finish_inline_text(&ctx));
            settle(&mut app, &ctx);
            wait_preview(&mut app, &ctx);
            text_reopen_draft(&mut app, &ctx, 0.2);
            text_draw_frame(
                &mut app,
                &ctx,
                vec![text_key(egui::Key::A, egui::Modifiers::COMMAND)],
            );
        }
        ctx.data_mut(|d| {
            d.insert_temp(
                egui::Id::new("test-save-as-picker"),
                accepted.then(|| copy.clone()),
            )
        });
        if menu {
            text_draw_frame(
                &mut app,
                &ctx,
                vec![egui::Event::Text("Copy this draft".into())],
            );
            text_click_access(&mut app, &ctx, "Document");
            text_click_access(&mut app, &ctx, "Save As… Ctrl+Shift+S");
        } else {
            text_draw_frame(
                &mut app,
                &ctx,
                vec![egui::Event::Text("Copy this draft".into()), save_key(true)],
            );
        }
        settle(&mut app, &ctx);
        wait_preview(&mut app, &ctx);
        for _ in 0..4 {
            text_draw_frame(&mut app, &ctx, vec![]);
            settle(&mut app, &ctx);
        }
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "Save As must never overwrite source"
        );
        if accepted {
            let shapes = EditablePdf::open(&copy).unwrap().shapes();
            assert_eq!(shapes[0].text.as_ref().unwrap().contents, "Copy this draft");
            assert!(!app.editing.dirty);
        } else {
            assert!(!copy.exists());
            assert!(
                app.editing.dirty,
                "cancelled Save As retains committed unsaved text"
            );
            assert_eq!(
                app.markup.items[0].text.as_ref().unwrap().contents,
                "Copy this draft"
            );
            assert!(app.editing.session.as_ref().unwrap().can_undo());
        }
    }
}
#[test]
fn text_safety_save_as_menu_invalid_and_numeric_same_frame_matrix() {
    for dirty in [false, true] {
        for menu in [false, true] {
            for invalid in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("source.pdf");
                fixture(&path);
                let copy = dir.path().join("copy.pdf");
                let before = std::fs::read(&path).unwrap();
                let ctx = egui::Context::default();
                let mut app = text_begin_fixture(&path, &ctx);
                if dirty {
                    text_draw_frame(&mut app, &ctx, vec![egui::Event::Text("Seed".into())]);
                    assert!(app.finish_inline_text(&ctx));
                    settle(&mut app, &ctx);
                    wait_preview(&mut app, &ctx);
                    text_reopen_draft(&mut app, &ctx, 0.2);
                }
                let contents = if invalid {
                    "Unsupported \u{2603}"
                } else {
                    "Copy note"
                };
                text_draw_frame(
                    &mut app,
                    &ctx,
                    vec![
                        text_key(egui::Key::A, egui::Modifiers::COMMAND),
                        egui::Event::Text(contents.into()),
                    ],
                );
                ctx.data_mut(|d| {
                    d.insert_temp(egui::Id::new("test-save-as-picker"), Some(copy.clone()))
                });
                if menu {
                    text_click_access(&mut app, &ctx, "Document");
                    text_click_access(&mut app, &ctx, "Save As… Ctrl+Shift+S");
                } else {
                    text_click_access(&mut app, &ctx, "18 pt");
                    // Numeric TextEdit and Ctrl+Shift+S in one production draw frame.
                    text_draw_frame(
                        &mut app,
                        &ctx,
                        vec![
                            text_key(egui::Key::A, egui::Modifiers::COMMAND),
                            egui::Event::Text("12".into()),
                            save_key(true),
                        ],
                    );
                }
                for _ in 0..4 {
                    settle(&mut app, &ctx);
                    text_draw_frame(&mut app, &ctx, vec![]);
                }
                assert_eq!(std::fs::read(&path).unwrap(), before);
                if invalid {
                    assert!(app.markup.text_draft.is_some());
                    assert!(!app.edit_pending());
                    assert!(!copy.exists());
                    assert!(
                        ctx.data(
                            |d| d.get_temp::<Option<PathBuf>>(egui::Id::new("test-save-as-picker"))
                        )
                        .is_some(),
                        "invalid draft must not reach picker"
                    );
                    assert_eq!(app.editing.dirty, dirty);
                    let out = text_access_frame(&mut app, &ctx, vec![], egui::vec2(1200., 1000.));
                    text_access_bounds(&out, contents);
                    text_click_access(&mut app, &ctx, "Cancel");
                } else {
                    let saved = EditablePdf::open(&copy).unwrap().shapes();
                    assert_eq!(saved[0].text.as_ref().unwrap().contents, contents);
                    assert_eq!(
                        saved[0].text.as_ref().unwrap().size,
                        if menu { 18. } else { 12. }
                    );
                    assert!(
                        ctx.data(
                            |d| d.get_temp::<Option<PathBuf>>(egui::Id::new("test-save-as-picker"))
                        )
                        .is_none()
                    );
                    assert!(!app.editing.dirty);
                }
            }
        }
    }
}
#[test]
fn text_save_as_cancel_preserves_inline_or_committed_unsaved_text() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("picker-text.pdf");
    fixture(&path);
    let before = std::fs::read(&path).unwrap();
    let ctx = egui::Context::default();
    let mut app = text_begin_fixture(&path, &ctx);
    text_draw_frame(&mut app, &ctx, vec![egui::Event::Text("Draft".into())]);
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("test-save-as-picker"), None::<PathBuf>));
    app.save_as_with_picker(&ctx, |_| None);
    assert!(
        app.markup.text_draft.is_some() || app.edit_pending() || app.editing.dirty,
        "picker cancel must preserve unsaved text, not implicitly Cancel it"
    );
    text_draw_frame(&mut app, &ctx, vec![]);
    settle(&mut app, &ctx);
    if app.markup.text_draft.is_some() {
        assert!(app.finish_inline_text(&ctx));
        settle(&mut app, &ctx);
    }
    assert_eq!(app.markup.items[0].text.as_ref().unwrap().contents, "Draft");
    assert!(app.editing.dirty);
    assert_eq!(std::fs::read(&path).unwrap(), before);
}
#[test]
fn text_lifecycle_focus_navigation_modal_and_sheet_picker_cancel_without_history() {
    let dir = tempfile::tempdir().unwrap();
    for action in 0..4 {
        let path = dir.path().join(format!("cancel-text-{action}.pdf"));
        fixture(&path);
        let before = std::fs::read(&path).unwrap();
        let ctx = egui::Context::default();
        let mut app = text_begin_fixture(&path, &ctx);
        text_draw_frame(
            &mut app,
            &ctx,
            vec![egui::Event::Text("Owned draft R E L A T".into())],
        );
        assert!(
            app.markup.mode == markup::Mode::Text,
            "typing letters does not select global tools"
        );
        match action {
            0 => {
                text_draw_frame(&mut app, &ctx, vec![egui::Event::WindowFocused(false)]);
            }
            1 => {
                app.select_page(0, &ctx);
                text_draw_frame(&mut app, &ctx, vec![]);
            }
            2 => {
                app.request_page_label(0, &ctx);
                text_draw_frame(&mut app, &ctx, vec![]);
            }
            _ => {
                app.open_sheet_picker();
                text_draw_frame(&mut app, &ctx, vec![]);
            }
        }
        assert!(
            app.markup.text_draft.is_none(),
            "action {action} must cancel its owned text draft"
        );
        assert_ne!(
            ctx.memory(|m| m.focused()),
            Some(egui::Id::new("inline-markup-text"))
        );
        assert!(!app.edit_pending());
        assert!(!app.editing.dirty);
        assert!(app.markup.items.is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
}
#[test]
fn text_draft_typing_headless_native_frame_latency_and_no_pdf_worker_churn() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("latency-text.pdf");
    fixture(&path);
    let ctx = egui::Context::default();
    let mut app = text_begin_fixture(&path, &ctx);
    let generation = app.document_generation;
    let mut samples = Vec::new();
    for _ in 0..60 {
        let start = std::time::Instant::now();
        text_draw_frame(&mut app, &ctx, vec![egui::Event::Text("a".into())]);
        samples.push(start.elapsed().as_secs_f64() * 1000.);
        assert!(app.markup.text_draft.is_some());
        assert!(!app.edit_pending());
        assert!(!app.markup.preview_pending);
        assert!(!app.editing.dirty);
        assert_eq!(app.document_generation, generation);
    }
    samples.sort_by(f64::total_cmp);
    eprintln!(
        "TEXT_HEADLESS_NATIVE_UI_FRAMES n=60 p50_ms={:.3} p95_ms={:.3} max_ms={:.3}; no edit transactions or PDF preview generations while typing",
        samples[30], samples[57], samples[59]
    );
}
#[test]
fn text_menu_save_finishes_through_canvas_frame_guard() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("menu-text.pdf");
    fixture(&path);
    let ctx = egui::Context::default();
    let mut app = text_begin_fixture(&path, &ctx);
    text_draw_frame(
        &mut app,
        &ctx,
        vec![egui::Event::Text("Menu saved draft".into())],
    );
    app.start_edit(Command::Save, &ctx);
    text_draw_frame(&mut app, &ctx, vec![]);
    settle(&mut app, &ctx);
    wait_preview(&mut app, &ctx);
    let reopened = EditablePdf::open(&path).unwrap().shapes();
    assert_eq!(
        reopened.len(),
        1,
        "menu Save must not save around an uncommitted text editor"
    );
    assert_eq!(
        reopened[0].text.as_ref().unwrap().contents,
        "Menu saved draft"
    );
}
#[test]
fn text_real_draw_batched_typing_save_reopen_edit_move_delete_shared_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("text-draw.pdf");
    fixture(&path);
    let original = std::fs::read(&path).unwrap();
    let ctx = egui::Context::default();
    let mut app = setup(&path, &ctx);
    text_draw_frame(&mut app, &ctx, vec![]);
    app.load_markups(&ctx);
    settle(&mut app, &ctx);
    wait_preview(&mut app, &ctx);
    app.zoom = 0.5;
    app.pan = egui::Vec2::ZERO;
    app.markup.mode = markup::Mode::Text;
    let mut page = None;
    for _ in 0..4 {
        page = text_draw_frame(&mut app, &ctx, vec![]);
    }
    let page = page.unwrap();
    let pos = page.min + page.size() * 0.2;
    text_draw_frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(pos), pointer(pos, true)],
    );
    text_draw_frame(&mut app, &ctx, vec![pointer(pos, false)]);
    for _ in 0..2 {
        text_draw_frame(&mut app, &ctx, vec![]);
    }
    assert!(app.markup.text_draft.is_some());
    assert_eq!(
        ctx.memory(|m| m.focused()),
        Some(egui::Id::new("inline-markup-text"))
    );
    // Actual text widget consumes typed input in the very frame Save is requested.
    text_draw_frame(
        &mut app,
        &ctx,
        vec![egui::Event::Text("Native text".into()), save_key(false)],
    );
    assert!(app.markup.text_draft.is_none(), "{}", app.status);
    assert!(app.edit_pending());
    settle(&mut app, &ctx);
    wait_preview(&mut app, &ctx);
    let saved = EditablePdf::open(&path).unwrap().shapes();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].text.as_ref().unwrap().contents, "Native text");
    assert!(!app.editing.dirty);
    assert_eq!(
        std::fs::read(
            app.editing
                .session
                .as_ref()
                .unwrap()
                .last_backup_path()
                .unwrap()
        )
        .unwrap(),
        original
    );
    // Click existing text with T and edit through real TextEdit, same-frame Ctrl+S.
    assert!(app.can_change_markups(), "not ready: {}", app.status);
    text_draw_frame(
        &mut app,
        &ctx,
        vec![text_key(egui::Key::T, egui::Modifiers::NONE)],
    );
    assert!(
        app.markup.mode == markup::Mode::Text,
        "T reopens the Text tool"
    );
    let page = text_draw_frame(&mut app, &ctx, vec![]).unwrap();
    let rect = app.markup.items[0].rect;
    let inside = egui::pos2(
        page.left() + page.width() * (rect.x + rect.width * 0.2),
        page.top() + page.height() * (rect.y + rect.height * 0.2),
    );
    text_draw_frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::PointerMoved(inside),
            egui::Event::PointerButton {
                pos: inside,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    text_draw_frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerButton {
            pos: inside,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    text_draw_frame(&mut app, &ctx, vec![]);
    text_draw_frame(&mut app, &ctx, vec![]);
    assert!(app.markup.text_draft.is_some());
    text_draw_frame(
        &mut app,
        &ctx,
        vec![text_key(egui::Key::A, egui::Modifiers::COMMAND)],
    );
    text_draw_frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::Text("Re-edited text".into()),
            text_key(egui::Key::S, egui::Modifiers::COMMAND),
        ],
    );
    settle(&mut app, &ctx);
    wait_preview(&mut app, &ctx);
    settle(&mut app, &ctx);
    assert_eq!(
        app.markup.items[0].text.as_ref().unwrap().contents,
        "Re-edited text"
    );
    assert_eq!(
        app.markup.selected,
        Some(app.markup.items[0].object_id),
        "edited text retains selection"
    );
    assert_eq!(EditablePdf::open(&path).unwrap().shapes(), app.markup.items);
    // Existing text uses the same UpdateShape COW/history path, not a replacement annotation.
    let mut edited = saved[0].clone();
    edited.text.as_mut().unwrap().contents = "Edited text".into();
    edited.text.as_mut().unwrap().size = 12.;
    app.update_markup(edited.clone(), &ctx);
    settle(&mut app, &ctx);
    wait_preview(&mut app, &ctx);
    assert_eq!(app.markup.items[0], edited);
    edited.rect.x += 0.1;
    app.update_markup(edited.clone(), &ctx);
    settle(&mut app, &ctx);
    wait_preview(&mut app, &ctx);
    app.delete_shape(edited.object_id, &ctx);
    settle(&mut app, &ctx);
    wait_preview(&mut app, &ctx);
    assert!(app.markup.items.is_empty());
    app.start_edit(Command::Undo, &ctx);
    settle(&mut app, &ctx);
    wait_preview(&mut app, &ctx);
    assert_eq!(app.markup.items, [edited]);
    app.start_edit(Command::Redo, &ctx);
    settle(&mut app, &ctx);
    wait_preview(&mut app, &ctx);
    assert!(app.markup.items.is_empty());
    app.start_edit(Command::Save, &ctx);
    settle(&mut app, &ctx);
    assert!(EditablePdf::open(&path).unwrap().shapes().is_empty());
}
