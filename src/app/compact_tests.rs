use super::*;

fn layout(size: egui::Vec2, stored_sidebar: Option<f32>) -> (egui::Context, GlyphApp) {
    let ctx = egui::Context::default();
    let mut app = GlyphApp::with_context(&ctx, None);
    app.project.open_document(
        format!("{}-drawing-set.pdf", "Long document name ".repeat(12)).into(),
        crate::pdf::PdfDocumentSummary {
            page_count: 1,
            pages: Vec::new(),
            bookmarks: Vec::new(),
            title: None,
        },
    );
    app.status = "An intentionally long operation error with details: ".repeat(20);
    app.rendered_page = Some(Arc::new(RenderedPage {
        page_index: 0,
        width: 2,
        height: 3,
        rgba: vec![255; 24],
    }));
    app.page_texture = Some(ctx.load_texture(
        "compact-layout-fixture",
        egui::ColorImage::filled([2, 3], egui::Color32::WHITE),
        egui::TextureOptions::LINEAR,
    ));
    app.fit_to_page_requested = true;
    if let Some(width) = stored_sidebar {
        ctx.data_mut(|data| {
            data.insert_persisted(
                egui::Id::new("sheet_sidebar"),
                egui::containers::panel::PanelState {
                    outer_rect: egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, size.y),
                    ),
                },
            )
        });
    }
    // Panel layout caches settle over frames; never touch real PDFs or OS pickers.
    for _ in 0..3 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            },
            |ui| app.draw(ui),
        );
        output.textures_delta.clear();
    }
    (ctx, app)
}

fn panel(ctx: &egui::Context, id: &str) -> egui::Rect {
    egui::containers::panel::PanelState::load(ctx, egui::Id::new(id))
        .unwrap()
        .outer_rect
}

#[test]
fn compact_chrome_leaves_pdf_canvas_height_at_both_window_sizes() {
    for size in [egui::vec2(960., 640.), egui::vec2(1600., 1000.)] {
        let (ctx, app) = layout(size, None);
        let canvas = app.fitted_viewport.unwrap().0;
        println!(
            "window={size:?} canvas={canvas:?} header={} footer={} sidebar={} zoom={}",
            panel(&ctx, "title_bar").height(),
            panel(&ctx, "status_bar").height(),
            panel(&ctx, "sheet_sidebar").width(),
            app.zoom
        );
        assert!(
            canvas.y >= size.y - 120.,
            "PDF canvas loses too much height: {canvas:?} in {size:?}"
        );
        assert!(
            canvas.x <= size.x - panel(&ctx, "sheet_sidebar").width() - 12. + 1.,
            "toolbar must not expand the canvas beyond its panel"
        );
        let page = app.logical_page_size(app.rendered_page.as_ref().unwrap()) * app.zoom;
        assert!(
            page.x <= canvas.x && page.y <= canvas.y,
            "fit page must still fit the allocated canvas"
        );
    }
}

#[test]
fn compact_sidebar_defaults_to_220_and_clamps_legacy_328() {
    for stored in [None, Some(328.)] {
        let (ctx, _) = layout(egui::vec2(960., 640.), stored);
        let width = panel(&ctx, "sheet_sidebar").width();
        println!("stored={stored:?} sidebar={width}");
        assert!(
            (180. ..=280.).contains(&width),
            "sidebar exceeds compact range: {width}"
        );
        if stored.is_none() {
            assert!((width - 220.).abs() < 2., "default sidebar: {width}");
        }
    }
}

#[test]
fn compact_empty_toolbar_clicks_never_change_pdf_view_or_start_work() {
    let ctx = egui::Context::default();
    let mut app = GlyphApp::with_context(&ctx, None);
    let size = egui::vec2(960., 640.);
    let frame = |events: Vec<egui::Event>, app: &mut GlyphApp| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |ui| app.draw(ui),
        );
        output.textures_delta.clear();
    };
    frame(Vec::new(), &mut app);
    frame(Vec::new(), &mut app);
    // Sweep the real inline toolbar targets, not an isolated enabled flag.
    let y = panel(&ctx, "title_bar").bottom() + 6. + 4. + 12.;
    for x in (236..740).step_by(12) {
        let pos = egui::pos2(x as f32, y);
        for pressed in [true, false] {
            frame(
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                &mut app,
            );
        }
        assert_eq!(app.zoom, 1.);
        assert_eq!(app.pan, egui::Vec2::ZERO);
        assert!(!app.fit_to_page_requested && !app.fit_to_width_requested);
        assert!(app.markup.mode == markup::Mode::View);
        assert!(app.automation_rx.is_none() && app.loading_document.is_none());
    }
}

#[test]
fn compact_toolbar_stays_inside_canvas_with_largest_allowed_sidebar() {
    for size in [egui::vec2(960., 640.), egui::vec2(1600., 1000.)] {
        let (ctx, app) = layout(size, Some(328.));
        let canvas = app.fitted_viewport.unwrap().0;
        assert!(canvas.x <= size.x - panel(&ctx, "sheet_sidebar").width() - 11.);
        assert!(canvas.y >= size.y - 120.);
    }
}

#[test]
fn compact_footer_does_not_wrap_long_status_or_shortcuts() {
    for size in [egui::vec2(960., 640.), egui::vec2(1600., 1000.)] {
        let (ctx, _) = layout(size, None);
        let height = panel(&ctx, "status_bar").height();
        println!("window={size:?} footer={height}");
        assert!(
            height <= 30.,
            "footer must remain a single compact line: {height}"
        );
    }
}
