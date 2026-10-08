// Included in editing::tests to exercise the same worker/fixture harness.
#[test]
fn line_arrow_focus_loss_requires_fresh_clicks_and_cancels_pending_release() {
    for key in [egui::Key::L, egui::Key::A] {
        for pending_press in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("focus-line.pdf");
            fixture(&path);
            let original = std::fs::read(&path).unwrap();
            let ctx = egui::Context::default();
            let mut app = setup(&path, &ctx);
            app.markup.loaded = true;
            press(&mut app, &ctx, key, egui::Modifiers::NONE);
            let frame = |app: &mut GlyphApp, events: Vec<egui::Event>| {
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(800., 600.),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        let (page, response) = ui.allocate_exact_size(
                            egui::vec2(400., 300.),
                            egui::Sense::click_and_drag(),
                        );
                        app.interact_with_markup(ui, &response, page, page, 1);
                    },
                );
                out.textures_delta.clear();
            };
            let a = egui::pos2(80., 80.);
            let b = egui::pos2(240., 180.);
            frame(&mut app, vec![]);
            for pressed in [true, false] {
                frame(&mut app, vec![pointer(a, pressed)]);
            }
            if pending_press {
                frame(&mut app, vec![pointer(b, true)]);
            }
            // Pure event frames: no nested Context input/output locks and no PointerGone.
            frame(&mut app, vec![egui::Event::WindowFocused(false)]);
            frame(&mut app, vec![egui::Event::WindowFocused(true)]);
            if pending_press {
                frame(&mut app, vec![pointer(b, false)]);
            } else {
                for pressed in [true, false] {
                    frame(&mut app, vec![pointer(b, pressed)]);
                }
            }
            assert!(!app.edit_pending(), "blur must not dispatch a stale line");
            assert!(!app.editing.dirty);
            assert!(app.markup.items.is_empty());
            assert!(
                app.editing
                    .session
                    .as_ref()
                    .is_none_or(|s| { !s.is_dirty() && !s.can_undo() && !s.can_redo() })
            );
            assert_eq!(std::fs::read(&path).unwrap(), original);
            if pending_press {
                for pressed in [true, false] {
                    frame(&mut app, vec![pointer(b, pressed)]);
                }
                assert!(!app.edit_pending(), "fresh first click is only a draft");
            }
            for pressed in [true, false] {
                frame(&mut app, vec![pointer(a, pressed)]);
            }
            assert!(app.edit_pending(), "fresh second click must dispatch");
            settle(&mut app, &ctx);
            assert_eq!(app.markup.items.len(), 1);
            assert!(app.editing.dirty);
        }
    }
}

#[test]
fn line_arrow_native_frames_worker_save_reopen_select_delete_undo_redo() {
    for (key, kind) in [
        (egui::Key::L, crate::pdf::ShapeKind::Line),
        (egui::Key::A, crate::pdf::ShapeKind::Arrow),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("line-arrow-ui.pdf");
        fixture(&path);
        let original = std::fs::read(&path).unwrap();
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        app.markup.loaded = true;
        press(&mut app, &ctx, key, egui::Modifiers::NONE);
        let frame = |app: &mut GlyphApp, events: Vec<egui::Event>, save: bool| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800., 600.),
                    )),
                    events,
                    max_texture_side: Some(8192),
                    ..Default::default()
                },
                |ui| {
                    if save {
                        app.handle_edit_shortcuts(&ctx);
                    }
                    let (page, response) = ui
                        .allocate_exact_size(egui::vec2(400., 300.), egui::Sense::click_and_drag());
                    app.interact_with_markup(ui, &response, page, page, 1);
                    app.paint_markup(ui.painter(), page, 1);
                    app.finish_save_intent(&ctx);
                },
            );
            out.textures_delta.clear();
        };
        frame(&mut app, vec![], false);
        for pressed in [true, false] {
            frame(
                &mut app,
                vec![pointer(egui::pos2(80., 80.), pressed)],
                false,
            );
        }
        assert!(
            !app.edit_pending() && !app.editing.dirty,
            "start click is not an edit"
        );
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(egui::pos2(240., 180.)),
                pointer(egui::pos2(240., 180.), true),
            ],
            false,
        );
        frame(
            &mut app,
            vec![
                pointer(egui::pos2(240., 180.), false),
                egui::Event::Key {
                    key: egui::Key::S,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                },
            ],
            true,
        );
        settle(&mut app, &ctx);
        assert_eq!(app.markup.items.len(), 1, "{}", app.status);
        assert_eq!(app.markup.items[0].kind, kind);
        assert_eq!(EditablePdf::open(&path).unwrap().shapes(), app.markup.items);
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
        wait_preview(&mut app, &ctx);
        press(&mut app, &ctx, egui::Key::V, egui::Modifiers::NONE);
        frame(&mut app, vec![], false);
        for pressed in [true, false] {
            frame(
                &mut app,
                vec![pointer(egui::pos2(160., 130.), pressed)],
                false,
            );
        }
        assert!(
            app.markup.selected.is_some(),
            "V selects visible line midpoint"
        );
        press(&mut app, &ctx, egui::Key::Delete, egui::Modifiers::NONE);
        settle(&mut app, &ctx);
        assert!(app.markup.items.is_empty() && app.editing.dirty);
        press(&mut app, &ctx, egui::Key::Z, egui::Modifiers::COMMAND);
        settle(&mut app, &ctx);
        assert_eq!(app.markup.items.len(), 1);
        assert!(!app.editing.dirty);
        press(
            &mut app,
            &ctx,
            egui::Key::Z,
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        );
        settle(&mut app, &ctx);
        assert!(app.markup.items.is_empty() && app.editing.dirty);
    }
}

#[test]
fn line_arrow_command_guards_modal_busy_preview_dead_renderer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("line-guards.pdf");
    fixture(&path);
    let ctx = egui::Context::default();
    let mut app = setup(&path, &ctx);
    app.markup.loaded = true;
    app.editing.transition = Some(Transition::CloseDocument);
    app.add_line(1, [0., 0., 1., 1.], false, &ctx);
    assert!(!app.edit_pending());
    app.editing.transition = None;
    app.markup.preview_pending = true;
    app.add_line(1, [0., 0., 1., 1.], false, &ctx);
    assert!(!app.edit_pending());
    app.markup.preview_pending = false;
    app.add_line(0, [0., 0., 1., 1.], false, &ctx);
    assert!(!app.edit_pending(), "wrong page");
    app.load_markups(&ctx);
    app.add_line(1, [0., 0., 1., 1.], false, &ctx);
    settle(&mut app, &ctx);
    assert!(
        app.markup.items.is_empty(),
        "busy worker cannot accept a line"
    );
    let (tx, rx) = mpsc::channel();
    app.render_result_rx = rx;
    drop(tx);
    app.apply_render_results(&ctx);
    app.add_line(1, [0., 0., 1., 1.], false, &ctx);
    assert!(!app.edit_pending());
    assert!(!app.editing.dirty && std::fs::read(&path).unwrap().starts_with(b"%PDF"));
}
