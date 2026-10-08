fn picker_gesture_frame(app: &mut GlyphApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let mut output = ctx.run_ui(
        egui::RawInput {
            max_texture_side: Some(8192),
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800., 600.),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            app.handle_shortcuts(ctx);
            let (page, response) =
                ui.allocate_exact_size(egui::vec2(400., 300.), egui::Sense::click_and_drag());
            // Match production: the picker owns input instead of the canvas.
            if app.sheet_picker.is_none() {
                app.interact_with_markup(ui, &response, page, page, 1);
            }
            app.draw_sheet_picker(ctx);
        },
    );
    output.textures_delta.clear();
}

fn picker_gesture_key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

#[test]
fn markup_polish_picker_cancel_requires_two_fresh_line_arrow_clicks() {
    for mode in [markup::Mode::Line, markup::Mode::Arrow] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("picker-line.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        app.markup.loaded = true;
        app.markup.mode = mode;
        picker_gesture_frame(&mut app, &ctx, vec![]);
        let old = egui::pos2(80., 60.);
        picker_gesture_frame(
            &mut app,
            &ctx,
            vec![pointer(old, true), pointer(old, false)],
        );
        assert!(app.editing.pending.is_none());
        picker_gesture_frame(
            &mut app,
            &ctx,
            vec![picker_gesture_key(egui::Key::L, egui::Modifiers::COMMAND)],
        );
        assert!(app.sheet_picker.is_some());
        picker_gesture_frame(
            &mut app,
            &ctx,
            vec![picker_gesture_key(egui::Key::Escape, egui::Modifiers::NONE)],
        );
        assert!(app.sheet_picker.is_none());
        assert!(app.markup.mode == mode);
        picker_gesture_frame(&mut app, &ctx, vec![]);
        let fresh = egui::pos2(160., 120.);
        picker_gesture_frame(
            &mut app,
            &ctx,
            vec![pointer(fresh, true), pointer(fresh, false)],
        );
        assert!(
            app.editing.pending.is_none(),
            "first fresh click must not dispatch a stale shape"
        );
        settle(&mut app, &ctx);
        assert!(app.markup.items.is_empty());
        assert!(!app.editing.dirty);
        let end = egui::pos2(240., 180.);
        picker_gesture_frame(
            &mut app,
            &ctx,
            vec![pointer(end, true), pointer(end, false)],
        );
        assert!(
            app.editing.pending.is_some(),
            "second fresh click must commit"
        );
        settle(&mut app, &ctx);
        assert!(app.editing.dirty);
        assert_eq!(app.markup.items.len(), 1);
        assert_eq!(app.markup.items[0].endpoints, Some([0.4, 0.4, 0.6, 0.6]));
        assert_eq!(
            app.markup.items[0].kind,
            if mode == markup::Mode::Arrow {
                crate::pdf::ShapeKind::Arrow
            } else {
                crate::pdf::ShapeKind::Line
            }
        );
    }
}

#[test]
fn markup_polish_picker_cancels_box_move_resize_before_release() {
    for operation in 0..4 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("picker-drag.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        picker_gesture_frame(&mut app, &ctx, vec![]);
        if operation >= 2 {
            app.add_rectangle(
                1,
                crate::core::links::PdfRect {
                    x: 0.2,
                    y: 0.2,
                    width: 0.4,
                    height: 0.4,
                },
                &ctx,
            );
            settle(&mut app, &ctx);
            app.start_edit(Command::Save, &ctx);
            settle(&mut app, &ctx);
            wait_preview(&mut app, &ctx);
            app.markup.selected = Some(app.markup.items[0].object_id);
        }
        app.markup.loaded = true;
        let mode = match operation {
            0 => markup::Mode::Rectangle,
            1 => markup::Mode::Ellipse,
            _ => markup::Mode::Select,
        };
        app.markup.mode = mode;
        let before = app.markup.items.clone();
        let selected = app.markup.selected;
        picker_gesture_frame(&mut app, &ctx, vec![]);
        let start = if operation == 2 {
            egui::pos2(168., 128.)
        } else {
            egui::pos2(80., 60.)
        };
        let end = start + egui::vec2(40., 30.);
        picker_gesture_frame(&mut app, &ctx, vec![pointer(start, true)]);
        picker_gesture_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(end)]);
        assert!(app.editing.pending.is_none());
        picker_gesture_frame(
            &mut app,
            &ctx,
            vec![picker_gesture_key(egui::Key::L, egui::Modifiers::COMMAND)],
        );
        assert!(app.sheet_picker.is_some());
        picker_gesture_frame(
            &mut app,
            &ctx,
            vec![picker_gesture_key(egui::Key::Escape, egui::Modifiers::NONE)],
        );
        picker_gesture_frame(&mut app, &ctx, vec![]);
        picker_gesture_frame(&mut app, &ctx, vec![pointer(end, false)]);
        assert!(
            app.editing.pending.is_none(),
            "cancelled drag {operation} must not dispatch on release"
        );
        settle(&mut app, &ctx);
        assert_eq!(app.markup.items, before);
        assert_eq!(app.markup.selected, selected);
        assert!(app.markup.mode == mode);
        assert!(!app.editing.dirty);
    }
}

#[test]
fn markup_polish_selected_corner_or_endpoint_drag_changes_geometry_once() {
    use crate::pdf::ShapeKind;
    for kind in [
        ShapeKind::Rectangle,
        ShapeKind::Ellipse,
        ShapeKind::Line,
        ShapeKind::Arrow,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("resize.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        if matches!(kind, ShapeKind::Line | ShapeKind::Arrow) {
            app.add_line(1, [0.2, 0.2, 0.6, 0.6], kind == ShapeKind::Arrow, &ctx);
        } else {
            app.start_edit(
                if kind == ShapeKind::Rectangle {
                    Command::Rectangle {
                        page: 1,
                        rect: crate::core::links::PdfRect {
                            x: 0.2,
                            y: 0.2,
                            width: 0.4,
                            height: 0.4,
                        },
                    }
                } else {
                    Command::Ellipse {
                        page: 1,
                        rect: crate::core::links::PdfRect {
                            x: 0.2,
                            y: 0.2,
                            width: 0.4,
                            height: 0.4,
                        },
                    }
                },
                &ctx,
            );
        }
        settle(&mut app, &ctx);
        finishing_frame(&mut app, &ctx, vec![]);
        wait_preview(&mut app, &ctx);
        let before = app.markup.items[0].clone();
        app.markup.mode = markup::Mode::Select;
        app.markup.selected = Some(before.object_id);
        // run_ui allocates this harness's page at (0, 0), size 400 x 300.
        finishing_frame(&mut app, &ctx, vec![pointer(egui::pos2(80., 60.), true)]);
        finishing_frame(&mut app, &ctx, vec![pointer(egui::pos2(120., 90.), false)]);
        settle(&mut app, &ctx);
        let after = &app.markup.items[0];
        if let Some([x, y, z, t]) = after.endpoints {
            assert!((x - 0.3).abs() < 0.001 && (y - 0.3).abs() < 0.001);
            assert_eq!([z, t], [0.6, 0.6], "other endpoint must stay fixed");
        } else {
            assert!(
                (after.rect.x - 0.3).abs() < 0.001 && (after.rect.y - 0.3).abs() < 0.001,
                "selected corner must resize {kind:?}"
            );
            assert!(
                (after.rect.width - 0.3).abs() < 0.001 && (after.rect.height - 0.3).abs() < 0.001
            );
        }
        app.start_edit(Command::Undo, &ctx);
        settle(&mut app, &ctx);
        assert_eq!(
            app.markup.items,
            vec![before],
            "one Undo reverses complete drag"
        );
    }
}

#[test]
fn markup_polish_drag_move_previews_then_one_edit_and_release_frame_save() {
    use crate::pdf::ShapeKind;
    for kind in [
        ShapeKind::Rectangle,
        ShapeKind::Ellipse,
        ShapeKind::Line,
        ShapeKind::Arrow,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("move.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        match kind {
            ShapeKind::Rectangle => app.add_rectangle(
                1,
                crate::core::links::PdfRect {
                    x: 0.2,
                    y: 0.2,
                    width: 0.4,
                    height: 0.4,
                },
                &ctx,
            ),
            ShapeKind::Ellipse => app.add_ellipse(
                1,
                crate::core::links::PdfRect {
                    x: 0.2,
                    y: 0.2,
                    width: 0.4,
                    height: 0.4,
                },
                &ctx,
            ),
            _ => app.add_line(1, [0.2, 0.2, 0.6, 0.6], kind == ShapeKind::Arrow, &ctx),
        }
        settle(&mut app, &ctx);
        app.start_edit(Command::Save, &ctx);
        settle(&mut app, &ctx);
        finishing_frame(&mut app, &ctx, vec![]);
        wait_preview(&mut app, &ctx);
        let before = app.markup.items[0].clone();
        app.markup.mode = markup::Mode::Select;
        app.markup.selected = Some(before.object_id);
        finishing_frame(&mut app, &ctx, vec![]);
        finishing_frame(&mut app, &ctx, vec![pointer(egui::pos2(168., 128.), true)]);
        finishing_frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(egui::pos2(208., 158.))],
        );
        assert_eq!(
            app.markup.items,
            vec![before.clone()],
            "drag preview must not mutate PDF"
        );
        assert!(!app.editing.dirty);
        finishing_frame(
            &mut app,
            &ctx,
            vec![pointer(egui::pos2(208., 158.), false), save_key(false)],
        );
        settle(&mut app, &ctx);
        let after = app.markup.items[0].clone();
        assert_ne!(after, before, "release must move {kind:?}: {}", app.status);
        assert_eq!(after.object_id, before.object_id);
        assert_eq!(
            EditablePdf::open(&path).unwrap().shapes(),
            vec![after.clone()]
        );
        assert!(!app.editing.dirty);
        app.start_edit(Command::Undo, &ctx);
        settle(&mut app, &ctx);
        assert_eq!(app.markup.items, vec![before]);
        assert!(app.editing.dirty);
        app.start_edit(Command::Redo, &ctx);
        settle(&mut app, &ctx);
        assert_eq!(app.markup.items, vec![after]);
        assert!(!app.editing.dirty);
    }
}

#[test]
fn markup_polish_next_style_is_one_creation_entry_and_survives_save() {
    use crate::pdf::{ShapeKind, ShapeStyle};
    for kind in [
        ShapeKind::Rectangle,
        ShapeKind::Ellipse,
        ShapeKind::Line,
        ShapeKind::Arrow,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("style.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        let style = ShapeStyle {
            rgb: [0.2, 0.4, 0.8],
            weight: 5.,
        };
        app.markup.next_style = style;
        let rect = crate::core::links::PdfRect {
            x: 0.2,
            y: 0.2,
            width: 0.4,
            height: 0.4,
        };
        match kind {
            ShapeKind::Rectangle => app.add_rectangle(1, rect, &ctx),
            ShapeKind::Ellipse => app.add_ellipse(1, rect, &ctx),
            _ => app.add_line(1, [0.2, 0.2, 0.6, 0.6], kind == ShapeKind::Arrow, &ctx),
        }
        settle(&mut app, &ctx);
        assert_eq!(
            app.markup.items[0].style, style,
            "next-shape properties must be persisted by creation"
        );
        app.start_edit(Command::Save, &ctx);
        settle(&mut app, &ctx);
        assert_eq!(EditablePdf::open(&path).unwrap().shapes(), app.markup.items);
        app.start_edit(Command::Undo, &ctx);
        settle(&mut app, &ctx);
        assert!(
            app.markup.items.is_empty(),
            "creation including style is exactly one history entry"
        );
        app.start_edit(Command::Redo, &ctx);
        settle(&mut app, &ctx);
        assert_eq!(app.markup.items[0].style, style);
    }
}
