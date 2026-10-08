use super::*;

fn fixture(ctx: &egui::Context, count: usize) -> GlyphApp {
    let mut app = GlyphApp::with_context(ctx, None);
    app.project.open_document(
        PathBuf::from("/nonexistent/sheet-picker-fixture.pdf"),
        crate::pdf::PdfDocumentSummary {
            page_count: count,
            pages: (0..count)
                .map(|index| crate::pdf::PdfPageInfo {
                    index,
                    label: Some("A101".into()),
                })
                .collect(),
            bookmarks: vec![],
            title: Some("NOT A SEMANTIC LABEL".into()),
        },
    );
    app
}

fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}
fn picker_frame(app: &mut GlyphApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000., 700.),
            )),
            events,
            ..Default::default()
        },
        |_| {
            app.handle_shortcuts(ctx);
            app.draw_sheet_picker(ctx);
        },
    );
    output.textures_delta.clear();
}

#[test]
fn sheet_picker_hands_save_and_close_commands_to_existing_handlers() {
    for command in [egui::Key::S, egui::Key::W] {
        let ctx = egui::Context::default();
        let mut app = fixture(&ctx, 3);
        picker_frame(
            &mut app,
            &ctx,
            vec![key(egui::Key::L, egui::Modifiers::COMMAND)],
        );
        picker_frame(&mut app, &ctx, vec![key(command, egui::Modifiers::COMMAND)]);
        assert!(
            app.sheet_picker.is_none(),
            "unrelated command {command:?} must dismiss picker and dispatch normally"
        );
        assert!(
            !ctx.input(|i| i.key_pressed(command)),
            "existing command must consume {command:?}"
        );
        if command == egui::Key::W {
            assert!(app.project.document.is_none());
        }
    }
}

#[test]
fn sheet_picker_escape_closes_only_its_owner_and_releases_query_focus() {
    let ctx = egui::Context::default();
    let mut app = fixture(&ctx, 3);
    app.markup.mode = markup::Mode::Arrow;
    app.search_running = true;
    picker_frame(
        &mut app,
        &ctx,
        vec![key(egui::Key::L, egui::Modifiers::COMMAND)],
    );
    picker_frame(
        &mut app,
        &ctx,
        vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
    );
    assert!(app.sheet_picker.is_none(), "Escape closes sheet picker");
    assert_ne!(
        ctx.memory(|m| m.focused()),
        Some(egui::Id::new("sheet_picker_query"))
    );
    assert!(
        app.search_running,
        "sheet Escape must not cancel unrelated text search"
    );
    assert!(app.markup.mode == markup::Mode::Arrow);
    let mut output = ctx.run_ui(
        egui::RawInput {
            events: vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
            ..Default::default()
        },
        |_| {
            app.search_running = false;
            app.markup.mode = markup::Mode::View;
            app.handle_shortcuts(&ctx);
            assert!(
                ctx.input(|i| i.key_pressed(egui::Key::Escape)),
                "no picker must leave Escape to other owners"
            );
        },
    );
    output.textures_delta.clear();
}

#[test]
fn sheet_picker_enter_uses_existing_navigation_history_and_cancels_markup() {
    let ctx = egui::Context::default();
    let mut app = fixture(&ctx, 3);
    app.markup.selected = Some((42, 0));
    app.zoom = 2.;
    app.pan = egui::vec2(19., 31.);
    picker_frame(
        &mut app,
        &ctx,
        vec![key(egui::Key::L, egui::Modifiers::COMMAND)],
    );
    picker_frame(
        &mut app,
        &ctx,
        vec![key(egui::Key::ArrowDown, egui::Modifiers::NONE)],
    );
    assert_eq!(app.project.selected_page, 0, "selection is not navigation");
    picker_frame(
        &mut app,
        &ctx,
        vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
    );
    assert_eq!(
        app.project.selected_page, 1,
        "Return must open selected physical page"
    );
    assert!(app.sheet_picker.is_none());
    assert_eq!(app.zoom, 2.);
    assert_eq!(app.pan, egui::vec2(19., 31.));
    assert_eq!(app.document_generation, 0);
    assert!(!app.editing.dirty);
    assert!(app.markup.selected.is_none());
    app.go_back(&ctx);
    assert_eq!(app.project.selected_page, 0);
}

#[test]
fn sheet_picker_command_l_focuses_query_and_typing_lra_does_not_navigate() {
    let ctx = egui::Context::default();
    let mut app = fixture(&ctx, 3);
    picker_frame(
        &mut app,
        &ctx,
        vec![
            key(egui::Key::L, egui::Modifiers::COMMAND),
            key(egui::Key::ArrowRight, egui::Modifiers::NONE),
        ],
    );
    assert!(app.sheet_picker.is_some(), "Ctrl+L must open sheet picker");
    assert_eq!(
        ctx.memory(|m| m.focused()),
        Some(egui::Id::new("sheet_picker_query"))
    );
    picker_frame(&mut app, &ctx, vec![egui::Event::Text("LRA".into())]);
    assert_eq!(app.sheet_picker.as_ref().unwrap().query, "LRA");
    assert_eq!(app.project.selected_page, 0);
    assert!(!app.editing.dirty);
}

#[test]
fn sheet_picker_rejects_lost_ownership_and_cancels_stale_dialogs() {
    for cause in 0..8 {
        let ctx = egui::Context::default();
        let mut app = fixture(&ctx, 3);
        app.open_sheet_picker();
        // Inject an external competing lifecycle transition, not a user edit
        // through the modal. Central command guards may correctly reject that.
        let picker = app.sheet_picker.take();
        match cause {
            0 => app.document_generation += 1,
            1 => app.project.document.as_mut().unwrap().path = "other.pdf".into(),
            2 => app.loading_document = Some(99),
            3 => app.render_worker_dead = true,
            4 => app.request_page_label(0, &ctx),
            5 => app.load_markups(&ctx),
            6 => {
                let (_tx, rx) = mpsc::channel();
                app.automation_rx = Some((0, "other.pdf".into(), rx));
            }
            _ => app.project.document = None,
        }
        app.sheet_picker = picker;
        app.validate_sheet_picker(&ctx);
        assert!(
            app.sheet_picker.is_none(),
            "stale dialog survived cause {cause}"
        );
        if cause != 0 && cause != 1 {
            app.open_sheet_picker();
            assert!(
                app.sheet_picker.is_none(),
                "ineligible document opened cause {cause}"
            );
        }
    }
}

#[test]
fn sheet_picker_searches_physical_pages_labels_and_valid_bookmarks() {
    let ctx = egui::Context::default();
    let mut app = fixture(&ctx, 3);
    let summary = &mut app.project.document.as_mut().unwrap().summary;
    summary.pages[2].label = None;
    for (title, page_index) in [
        ("Roof PLAN", Some(2)),
        ("Roof PLAN", Some(2)),
        ("Invalid", Some(99)),
        ("Untargeted", None),
    ] {
        summary.bookmarks.push(crate::pdf::PdfBookmark {
            object_id: None,
            title: title.into(),
            page_index,
            depth: 0,
        });
    }
    app.open_sheet_picker();
    let picker = app.sheet_picker.as_mut().unwrap();
    for (query, expected) in [
        ("a101", vec![0, 1]),
        ("roof plan", vec![2]),
        ("page 3", vec![2]),
        ("invalid", vec![]),
        ("untargeted", vec![]),
        ("NOT A SEMANTIC", vec![]),
    ] {
        picker.query = query.into();
        picker.filter();
        assert_eq!(picker.matches, expected, "query: {query}");
    }
    assert_eq!(app.project.selected_page, 0);
    assert!(!app.editing.dirty);
}

#[test]
fn sheet_picker_open_is_read_only() {
    let ctx = egui::Context::default();
    let mut app = fixture(&ctx, 3);
    app.zoom = 2.;
    app.open_sheet_picker();
    assert!(
        app.sheet_picker.is_some(),
        "eligible document must open the sheet picker"
    );
    assert_eq!(app.project.selected_page, 0);
    assert_eq!(app.zoom, 2.);
    assert!(!app.editing.dirty);
    assert_eq!(app.document_generation, 0);
}

#[test]
fn sheet_picker_production_draw_installs_query_focus_and_consumes_enter() {
    let ctx = egui::Context::default();
    let mut app = fixture(&ctx, 3);
    let (_tx, rx) = mpsc::channel();
    app.render_result_rx = rx;
    for events in [vec![key(egui::Key::L, egui::Modifiers::COMMAND)], vec![]] {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000., 700.),
                )),
                events,
                ..Default::default()
            },
            |ui| app.draw(ui),
        );
        output.textures_delta.clear();
    }
    assert_eq!(
        ctx.memory(|m| m.focused()),
        Some(egui::Id::new("sheet_picker_query")),
        "production draw must show the picker"
    );
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000., 700.),
            )),
            events: vec![
                key(egui::Key::ArrowDown, egui::Modifiers::NONE),
                key(egui::Key::Enter, egui::Modifiers::NONE),
            ],
            ..Default::default()
        },
        |ui| app.draw(ui),
    );
    output.textures_delta.clear();
    assert_eq!(app.project.selected_page, 1);
    assert!(app.sheet_picker.is_none());
    assert!(!ctx.input(|i| i.key_pressed(egui::Key::Enter)));
}

#[test]
fn sheet_picker_blocks_background_navigation_without_relying_on_text_focus() {
    let ctx = egui::Context::default();
    let mut app = fixture(&ctx, 3);
    app.open_sheet_picker();
    app.select_page(2, &ctx);
    assert_eq!(
        app.project.selected_page, 0,
        "background routes cannot navigate through picker"
    );
    app.reset_view();
    assert!(app.sheet_picker.is_some());
}

#[test]
fn sheet_picker_command_g_hands_focus_to_existing_page_entry() {
    let ctx = egui::Context::default();
    let mut app = fixture(&ctx, 3);
    picker_frame(
        &mut app,
        &ctx,
        vec![key(egui::Key::L, egui::Modifiers::COMMAND)],
    );
    picker_frame(
        &mut app,
        &ctx,
        vec![key(egui::Key::G, egui::Modifiers::COMMAND)],
    );
    assert!(app.sheet_picker.is_none());
    assert!(app.page_entry_focus_requested);
    assert_ne!(
        ctx.memory(|m| m.focused()),
        Some(egui::Id::new("sheet_picker_query"))
    );
}

#[test]
fn sheet_picker_quit_uses_unsaved_close_guard() {
    let ctx = egui::Context::default();
    let mut app = fixture(&ctx, 3);
    app.editing.dirty = true;
    picker_frame(
        &mut app,
        &ctx,
        vec![key(egui::Key::L, egui::Modifiers::COMMAND)],
    );
    picker_frame(
        &mut app,
        &ctx,
        vec![key(egui::Key::Q, egui::Modifiers::COMMAND)],
    );
    assert!(app.sheet_picker.is_none());
    assert!(
        app.editing_modal_open(),
        "Quit must present the existing unsaved decision"
    );
    assert!(app.editing.dirty);
    assert!(app.project.document.is_some());
    assert!(!ctx.input(|i| i.key_pressed(egui::Key::Q)));
}

#[test]
fn sheet_picker_open_handoff_retains_dirty_document_and_uses_open_guard() {
    let ctx = egui::Context::default();
    let mut app = fixture(&ctx, 3);
    app.editing.dirty = true;
    picker_frame(
        &mut app,
        &ctx,
        vec![key(egui::Key::L, egui::Modifiers::COMMAND)],
    );
    let mut called = false;
    let mut output = ctx.run_ui(
        egui::RawInput {
            events: vec![key(egui::Key::O, egui::Modifiers::COMMAND)],
            ..Default::default()
        },
        |_| {
            app.handle_shortcuts_with_open_picker(&ctx, |app, ctx| {
                called = true;
                assert!(app.sheet_picker.is_none());
                app.open_pdf(PathBuf::from("/nonexistent/replacement.pdf"), ctx);
            });
        },
    );
    output.textures_delta.clear();
    assert!(called);
    assert!(app.editing_modal_open());
    assert!(app.editing.dirty);
    assert_eq!(
        app.project.document.as_ref().unwrap().path,
        PathBuf::from("/nonexistent/sheet-picker-fixture.pdf")
    );
}

#[test]
fn sheet_picker_duplicate_labels_bookmarks_numbers_and_no_matches_confirm_physically() {
    for (query, down, expected) in [
        ("a101", true, Some(1)),
        ("roof detail", false, Some(2)),
        ("page 3", false, Some(2)),
        ("827493unknown", false, None),
    ] {
        let ctx = egui::Context::default();
        let mut app = fixture(&ctx, 3);
        app.project
            .document
            .as_mut()
            .unwrap()
            .summary
            .bookmarks
            .push(crate::pdf::PdfBookmark {
                object_id: None,
                title: "Roof Detail".into(),
                page_index: Some(2),
                depth: 0,
            });
        picker_frame(
            &mut app,
            &ctx,
            vec![key(
                egui::Key::L,
                egui::Modifiers {
                    ctrl: true,
                    command: true,
                    ..Default::default()
                },
            )],
        );
        picker_frame(&mut app, &ctx, vec![egui::Event::Text(query.into())]);
        if down {
            picker_frame(
                &mut app,
                &ctx,
                vec![key(egui::Key::ArrowDown, egui::Modifiers::NONE)],
            );
        }
        picker_frame(
            &mut app,
            &ctx,
            vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
        );
        assert_eq!(
            app.project.selected_page,
            expected.unwrap_or(0),
            "query {query}"
        );
        assert_eq!(app.sheet_picker.is_none(), expected.is_some());
        assert!(!app.editing.dirty);
    }
}

#[test]
fn sheet_picker_virtualizes_large_results_without_truncating_physical_matches() {
    let ctx = egui::Context::default();
    let mut app = fixture(&ctx, 10_000);
    app.open_sheet_picker();
    assert_eq!(app.sheet_picker.as_ref().unwrap().matches.len(), 10_000);
    let mut painted_rows = 0;
    for _ in 0..3 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000., 700.),
                )),
                ..Default::default()
            },
            |_| app.draw_sheet_picker(&ctx),
        );
        painted_rows = output.shapes.iter().filter(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().contains("A101"))).count();
        output.textures_delta.clear();
    }
    assert!(
        painted_rows > 0 && painted_rows <= 14,
        "only visible/overscan rows paint: {painted_rows}"
    );
    let picker = app.sheet_picker.as_mut().unwrap();
    picker.query = "page 10000".into();
    picker.filter();
    assert_eq!(picker.matches, vec![9999]);
    picker_frame(
        &mut app,
        &ctx,
        vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
    );
    assert_eq!(app.project.selected_page, 9999);
}

#[test]
fn sheet_picker_background_history_routes_do_not_consume_history_or_change_view() {
    let ctx = egui::Context::default();
    let mut app = fixture(&ctx, 3);
    app.select_page(1, &ctx);
    app.zoom = 2.;
    app.open_sheet_picker();
    app.go_back(&ctx);
    assert_eq!(app.zoom, 2., "picker blocks background history restoration");
    app.close_sheet_picker(&ctx);
    app.go_back(&ctx);
    assert_eq!(
        app.project.selected_page, 0,
        "blocked route must not consume history"
    );
}

#[test]
fn sheet_picker_unmodified_tool_and_page_keys_stay_with_query_owner() {
    let ctx = egui::Context::default();
    let mut app = fixture(&ctx, 3);
    app.markup.mode = markup::Mode::Arrow;
    picker_frame(
        &mut app,
        &ctx,
        vec![key(egui::Key::L, egui::Modifiers::COMMAND)],
    );
    ctx.memory_mut(|m| m.surrender_focus(egui::Id::new("sheet_picker_query")));
    picker_frame(
        &mut app,
        &ctx,
        vec![
            key(egui::Key::L, egui::Modifiers::NONE),
            key(egui::Key::R, egui::Modifiers::NONE),
            key(egui::Key::A, egui::Modifiers::NONE),
            key(egui::Key::PageDown, egui::Modifiers::NONE),
            key(egui::Key::End, egui::Modifiers::NONE),
        ],
    );
    assert_eq!(app.project.selected_page, 0);
    assert!(app.markup.mode == markup::Mode::Arrow);
    assert!(app.sheet_picker.is_some());
    assert!(!app.editing.dirty);
}

#[test]
fn sheet_picker_quit_clean_and_busy_use_guarded_viewport_commands_and_restore_input() {
    for cause in 0..4 {
        let ctx = egui::Context::default();
        let mut app = fixture(&ctx, 3);
        app.open_sheet_picker();
        match cause {
            1 => {
                app.close_sheet_picker(&ctx);
                app.load_markups(&ctx);
            }
            2 => app.loading_document = Some(99),
            3 => {
                let (_tx, rx) = mpsc::channel();
                app.automation_rx = Some((0, "busy.pdf".into(), rx));
            }
            _ => {}
        }
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![key(egui::Key::Q, egui::Modifiers::COMMAND)],
                ..Default::default()
            },
            |_| {
                app.handle_shortcuts(&ctx);
                assert!(
                    !ctx.input(|i| i.viewport().close_requested()),
                    "keyboard close adapter must restore native input"
                );
            },
        );
        let close = output
            .viewport_output
            .values()
            .flat_map(|v| &v.commands)
            .any(|command| matches!(command, egui::ViewportCommand::Close));
        output.textures_delta.clear();
        assert_eq!(close, cause == 0, "only clean idle Quit may close: {cause}");
        assert!(app.project.document.is_some());
    }
}
