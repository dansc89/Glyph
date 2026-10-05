use super::*;

#[test]
fn opening_search_and_an_arrow_in_the_same_frame_does_not_navigate_pdf() {
    check_search_shortcut(false);
}

#[test]
fn released_control_modifier_does_not_lose_search_shortcut() {
    check_search_shortcut(true);
}

#[test]
fn control_one_requests_fit_page() {
    let ctx = egui::Context::default();
    let mut app = GlyphApp::with_context(&ctx, None);
    let modifiers = egui::Modifiers::COMMAND;
    let mut output = ctx.run_ui(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::Num1,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
            ..Default::default()
        },
        |_| app.handle_shortcuts(&ctx),
    );
    output.textures_delta.clear();
    assert!(app.fit_to_page_requested, "Ctrl+1 must request fit page");
}

#[test]
fn control_two_fits_width_and_aligns_tall_page_to_top() {
    let ctx = egui::Context::default();
    let mut app = GlyphApp::with_context(&ctx, None);
    app.rendered_page = Some(Arc::new(RenderedPage {
        page_index: 0,
        width: 2,
        height: 4,
        rgba: vec![255; 32],
    }));
    app.page_texture = Some(ctx.load_texture(
        "fit-fixture",
        egui::ColorImage::filled([2, 4], egui::Color32::WHITE),
        egui::TextureOptions::LINEAR,
    ));
    app.zoom = 0.1;
    app.pan = egui::vec2(90., -90.);
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1440., 920.),
            )),
            events: vec![egui::Event::Key {
                key: egui::Key::Num2,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }],
            ..Default::default()
        },
        |ui| app.draw(ui),
    );
    output.textures_delta.clear();
    assert!(
        app.zoom > 0.3,
        "Ctrl+2 must enlarge tall page to viewport width"
    );
    assert_eq!(app.pan.x, 0.);
    assert!(
        app.pan.y > 0.,
        "fit-width must show the top, not the middle of tall drawings"
    );
}

#[test]
fn reset_cancels_deferred_fit_requests() {
    let ctx = egui::Context::default();
    let mut app = GlyphApp::with_context(&ctx, None);
    app.fit_to_width_requested = true;
    app.fit_to_page_requested = true;
    app.reset_view();
    assert!(
        !app.fit_to_width_requested && !app.fit_to_page_requested,
        "reset must not be overridden when a pending page render arrives"
    );
}

#[test]
fn modified_fit_shortcuts_work_while_search_requests_keyboard_focus() {
    for key in [egui::Key::Num1, egui::Key::Num2] {
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        app.search_focus_requested = true;
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                }],
                ..Default::default()
            },
            |_| app.handle_shortcuts(&ctx),
        );
        output.textures_delta.clear();
        assert!(
            if key == egui::Key::Num1 {
                app.fit_to_page_requested
            } else {
                app.fit_to_width_requested
            },
            "modified view commands must not be swallowed by search focus"
        );
    }
}

#[test]
fn fitting_is_disabled_during_document_inspection() {
    let ctx = egui::Context::default();
    let mut app = GlyphApp::with_context(&ctx, None);
    app.loading_document = Some(7);
    let mut output = ctx.run_ui(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::Num2,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }],
            ..Default::default()
        },
        |_| app.handle_shortcuts(&ctx),
    );
    output.textures_delta.clear();
    assert!(
        !app.fit_to_width_requested,
        "loading-time fit must not target the old document"
    );
}

#[test]
fn fit_width_tracks_viewport_resize_without_another_shortcut() {
    let ctx = egui::Context::default();
    let mut app = GlyphApp::with_context(&ctx, None);
    app.rendered_page = Some(Arc::new(RenderedPage {
        page_index: 0,
        width: 2,
        height: 4,
        rgba: vec![255; 32],
    }));
    app.page_texture = Some(ctx.load_texture(
        "persistent-fit",
        egui::ColorImage::filled([2, 4], egui::Color32::WHITE),
        egui::TextureOptions::LINEAR,
    ));
    let mut first = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000., 800.),
            )),
            events: vec![egui::Event::Key {
                key: egui::Key::Num2,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }],
            ..Default::default()
        },
        |ui| app.draw(ui),
    );
    first.textures_delta.clear();
    let first_zoom = app.zoom;
    let mut second = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1440., 920.),
            )),
            ..Default::default()
        },
        |ui| app.draw(ui),
    );
    second.textures_delta.clear();
    assert!(
        app.zoom > first_zoom + 0.1,
        "fit width should follow the larger viewport"
    );
}

#[test]
fn reset_leaves_persistent_fit_mode() {
    let ctx = egui::Context::default();
    let mut app = GlyphApp::with_context(&ctx, None);
    app.rendered_page = Some(Arc::new(RenderedPage {
        page_index: 0,
        width: 2,
        height: 4,
        rgba: vec![255; 32],
    }));
    app.fit_to_width_requested = true;
    app.apply_view_fit(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(700., 500.),
    ));
    app.reset_view();
    app.apply_view_fit(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(1000., 500.),
    ));
    assert_eq!(
        app.zoom, 1.0,
        "reset must leave fit mode even across resize"
    );
    assert_eq!(app.pan, egui::Vec2::ZERO);
}

#[test]
fn wheel_zoom_leaves_fit_mode_so_resize_does_not_reset_the_view() {
    let ctx = egui::Context::default();
    let mut app = GlyphApp::with_context(&ctx, None);
    app.rendered_page = Some(Arc::new(RenderedPage {
        page_index: 0,
        width: 2,
        height: 4,
        rgba: vec![255; 32],
    }));
    app.page_texture = Some(ctx.load_texture(
        "wheel-fit",
        egui::ColorImage::filled([2, 4], egui::Color32::WHITE),
        egui::TextureOptions::LINEAR,
    ));
    app.fit_to_width_requested = true;
    let mut first = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1440., 920.),
            )),
            ..Default::default()
        },
        |ui| app.draw(ui),
    );
    first.textures_delta.clear();
    let first_zoom = app.zoom;
    let mut second = ctx.run_ui(
        egui::RawInput {
            events: vec![
                egui::Event::PointerMoved(egui::pos2(900., 400.)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: egui::vec2(0., 60.),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        },
        |ui| app.draw(ui),
    );
    second.textures_delta.clear();
    assert!(app.zoom > first_zoom, "wheel should still zoom");
    assert_eq!(app.fit_mode, FitMode::Manual);
    let zoom = app.zoom;
    app.apply_view_fit(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(700., 500.),
    ));
    assert_eq!(app.zoom, zoom);
}

#[test]
fn back_and_forward_restore_zoom_and_pan_not_only_page() {
    let ctx = egui::Context::default();
    let mut app = GlyphApp::with_context(&ctx, None);
    app.project.open_document(
        "history-fixture.pdf".into(),
        crate::pdf::PdfDocumentSummary {
            page_count: 3,
            pages: Vec::new(),
            bookmarks: Vec::new(),
            title: None,
        },
    );
    app.zoom = 2.;
    app.pan = egui::vec2(90., -130.);
    app.select_page(1, &ctx);
    app.zoom = 3.;
    app.pan = egui::vec2(-40., 60.);
    app.go_back(&ctx);
    assert_eq!(app.project.selected_page, 0);
    assert_eq!(app.zoom, 2.);
    assert_eq!(app.pan, egui::vec2(90., -130.));
    app.go_forward(&ctx);
    assert_eq!(app.project.selected_page, 1);
    assert_eq!(app.zoom, 3.);
    assert_eq!(app.pan, egui::vec2(-40., 60.));
}

#[test]
fn control_g_focuses_direct_page_entry_and_keeps_arrow_keys_in_the_editor() {
    let ctx = egui::Context::default();
    let mut app = GlyphApp::with_context(&ctx, None);
    app.project.open_document(
        "entry-fixture.pdf".into(),
        crate::pdf::PdfDocumentSummary {
            page_count: 3,
            pages: Vec::new(),
            bookmarks: Vec::new(),
            title: None,
        },
    );
    app.project.selected_page = 1;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1440., 920.),
            )),
            events: vec![
                egui::Event::Key {
                    key: egui::Key::G,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                },
                egui::Event::Key {
                    key: egui::Key::ArrowRight,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        },
        |ui| app.draw(ui),
    );
    output.textures_delta.clear();
    assert_eq!(
        app.project.selected_page, 1,
        "opening page entry must not navigate on the same frame's arrow"
    );
    assert!(
        ctx.egui_wants_keyboard_input(),
        "Ctrl+G should focus a text editor"
    );
}

#[test]
fn entering_a_page_number_navigates_and_preserves_history() {
    let ctx = egui::Context::default();
    let mut app = GlyphApp::with_context(&ctx, None);
    app.project.open_document(
        "entry-fixture.pdf".into(),
        crate::pdf::PdfDocumentSummary {
            page_count: 3,
            pages: Vec::new(),
            bookmarks: Vec::new(),
            title: None,
        },
    );
    app.page_entry = "3".to_owned();
    ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("direct-page-entry")));
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1440., 920.),
            )),
            events: vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        },
        |ui| app.draw(ui),
    );
    output.textures_delta.clear();
    assert_eq!(app.project.selected_page, 2);
    assert!(app.navigation_history.can_back());
}

#[test]
fn invalid_page_entry_reports_range_without_navigation() {
    for entry in ["0", "4", "garbage", "999999999999999999999999999"] {
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        app.project.open_document(
            "entry-fixture.pdf".into(),
            crate::pdf::PdfDocumentSummary {
                page_count: 3,
                pages: Vec::new(),
                bookmarks: Vec::new(),
                title: None,
            },
        );
        app.page_entry = entry.to_owned();
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("direct-page-entry")));
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| app.draw(ui),
        );
        output.textures_delta.clear();
        assert_eq!(app.project.selected_page, 0);
        assert!(!app.navigation_history.can_back());
        assert_eq!(app.status, "Enter a page number from 1 to 3.");
    }
}

#[test]
fn page_shortcut_selects_current_number_for_replacement() {
    let ctx = egui::Context::default();
    let mut app = GlyphApp::with_context(&ctx, None);
    app.project.open_document(
        "entry-fixture.pdf".into(),
        crate::pdf::PdfDocumentSummary {
            page_count: 3,
            pages: Vec::new(),
            bookmarks: Vec::new(),
            title: None,
        },
    );
    let mut output = ctx.run_ui(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::G,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }],
            ..Default::default()
        },
        |ui| app.draw(ui),
    );
    output.textures_delta.clear();
    let mut output = ctx.run_ui(
        egui::RawInput {
            events: vec![
                egui::Event::Text("3".to_owned()),
                egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        },
        |ui| app.draw(ui),
    );
    output.textures_delta.clear();
    assert_eq!(
        app.project.selected_page, 2,
        "typing after Ctrl+G should replace, not append to, the current number"
    );
}

#[test]
fn persistent_fit_tracks_new_page_geometry_and_does_no_work_when_unchanged() {
    let ctx = egui::Context::default();
    let mut app = GlyphApp::with_context(&ctx, None);
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000., 500.));
    app.rendered_page = Some(Arc::new(RenderedPage {
        page_index: 0,
        width: 2,
        height: 4,
        rgba: vec![255; 32],
    }));
    app.fit_to_width_requested = true;
    app.apply_view_fit(viewport);
    let first_pan = app.pan;
    let changed_at = app.last_view_change;
    app.apply_view_fit(viewport);
    assert_eq!(
        app.last_view_change, changed_at,
        "unchanged fit must not invalidate tiles every frame"
    );
    app.rendered_page = Some(Arc::new(RenderedPage {
        page_index: 1,
        width: 4,
        height: 2,
        rgba: vec![255; 32],
    }));
    app.apply_view_fit(viewport);
    assert_eq!(app.fit_mode, FitMode::Width);
    assert!(
        app.pan.y < first_pan.y,
        "new landscape sheet should not inherit portrait top alignment"
    );
    app.fit_to_page_requested = true;
    app.apply_view_fit(viewport);
    let first_zoom = app.zoom;
    app.apply_view_fit(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(500., 250.),
    ));
    assert!(app.zoom < first_zoom, "fit page must follow resize too");
    assert_eq!(app.pan, egui::Vec2::ZERO);
}

#[test]
fn history_does_not_change_while_loading_a_replacement_document() {
    let ctx = egui::Context::default();
    let mut app = GlyphApp::with_context(&ctx, None);
    app.project.open_document(
        "history-fixture.pdf".into(),
        crate::pdf::PdfDocumentSummary {
            page_count: 3,
            pages: Vec::new(),
            bookmarks: Vec::new(),
            title: None,
        },
    );
    app.select_page(1, &ctx);
    app.loading_document = Some(123);
    app.go_back(&ctx);
    assert_eq!(app.project.selected_page, 1);
    assert!(app.navigation_history.can_back());
    assert!(!app.navigation_history.can_forward());
}

#[test]
fn a_single_wheel_event_has_bounded_frame_rate_independent_zoom() {
    fn replay(fps: usize) -> f32 {
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        app.zoom = 0.25;
        app.rendered_page = Some(Arc::new(RenderedPage {
            page_index: 0,
            width: 2,
            height: 4,
            rgba: vec![255; 32],
        }));
        app.page_texture = Some(ctx.load_texture(
            "wheel-rate",
            egui::ColorImage::filled([2, 4], egui::Color32::WHITE),
            egui::TextureOptions::LINEAR,
        ));
        for frame in 0..=fps {
            let mut events = vec![egui::Event::PointerMoved(egui::pos2(900., 400.))];
            if frame == 1 {
                events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: egui::vec2(0., 60.),
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(frame as f64 / fps as f64),
                    predicted_dt: 1. / fps as f32,
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1440., 920.),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| app.draw(ui),
            );
            output.textures_delta.clear();
        }
        app.zoom
    }
    let sixty = replay(60);
    let one_twenty = replay(120);
    println!("single_60_point_wheel_zoom: 60fps={sixty}, 120fps={one_twenty}");
    assert!(
        sixty > 0.25 && sixty < 0.35,
        "one modest wheel event must not multiply zoom repeatedly: {sixty}"
    );
    assert!(
        (sixty - one_twenty).abs() < 0.001,
        "same wheel input at 60/120 FPS should zoom equally: {sixty} vs {one_twenty}"
    );
}

#[test]
fn history_restores_fit_modes_and_same_page_keeps_forward_history() {
    for mode in [FitMode::Page, FitMode::Width] {
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        app.project.open_document(
            "history-fixture.pdf".into(),
            crate::pdf::PdfDocumentSummary {
                page_count: 3,
                pages: Vec::new(),
                bookmarks: Vec::new(),
                title: None,
            },
        );
        app.fit_mode = mode;
        app.select_page(1, &ctx);
        app.fit_mode = FitMode::Manual;
        app.zoom = 2.;
        app.pan = egui::vec2(10., 20.);
        app.go_back(&ctx);
        assert_eq!(app.fit_mode, mode);
        assert!(app.navigation_history.can_forward());
        app.select_page(0, &ctx);
        assert!(
            app.navigation_history.can_forward(),
            "same page selection must not discard forward history"
        );
        app.go_forward(&ctx);
        assert_eq!(app.project.selected_page, 1);
        assert_eq!(app.fit_mode, FitMode::Manual);
        assert_eq!(app.zoom, 2.);
        assert_eq!(app.pan, egui::vec2(10., 20.));
    }
}

fn check_search_shortcut(release_control: bool) {
    let ctx = egui::Context::default();
    let mut app = GlyphApp::with_context(&ctx, None);
    app.project.open_document(
        "shortcut-fixture.pdf".into(),
        crate::pdf::PdfDocumentSummary {
            page_count: 3,
            pages: Vec::new(),
            bookmarks: Vec::new(),
            title: None,
        },
    );
    app.project.selected_page = 2;
    let modifiers = egui::Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    };
    let mut input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1440., 920.),
        )),
        events: vec![
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::Key {
                key: egui::Key::F,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            },
            egui::Event::Key {
                key: egui::Key::ArrowLeft,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            },
        ],
        ..Default::default()
    };
    if release_control {
        input
            .events
            .insert(2, egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    }
    let mut output = ctx.run_ui(input, |ui| app.draw(ui));
    output.textures_delta.clear();
    assert_eq!(app.navigation_tab, NavigationTab::Search);
    assert_eq!(
        app.project.selected_page, 2,
        "opening search must route arrows to the text box, not PDF navigation"
    );
}
