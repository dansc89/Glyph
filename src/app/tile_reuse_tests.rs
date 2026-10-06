use super::*;

fn tile(request: TileRequest) -> RenderedTile {
    RenderedTile {
        page_index: request.page_index,
        full_width: request.full_width,
        full_height: request.full_height,
        x: request.x,
        y: request.y,
        width: request.width,
        height: request.height,
        rgba: vec![0; request.width * request.height * 4],
    }
}

fn fixture(ppp: f32) -> (egui::Context, GlyphApp, egui::Rect, egui::Rect) {
    let ctx = egui::Context::default();
    ctx.set_pixels_per_point(ppp);
    // Apply the requested scale before querying pixels_per_point.
    let mut output = ctx.run_ui(Default::default(), |_| {});
    output.textures_delta.clear();
    let mut app = GlyphApp::with_context(&ctx, None);
    let (_tx, rx) = mpsc::channel();
    app.render_result_rx = rx;
    app.project.open_document(
        "tile-reuse-fixture.pdf".into(),
        crate::pdf::PdfDocumentSummary {
            page_count: 2,
            pages: Vec::new(),
            bookmarks: Vec::new(),
            title: None,
        },
    );
    app.zoom = 3.0;
    app.page_aspect_ratio = Some(1.0);
    app.last_view_change = None;
    assert!(app.loading_document.is_none());
    assert!(!app.edit_pending());
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0));
    let page = egui::Rect::from_center_size(viewport.center(), egui::vec2(5400.0, 5400.0));
    let request =
        visible_tile_request(0, Some(1.0), 3.0, ctx.pixels_per_point(), viewport, page).unwrap();
    app.rendered_tile = Some(tile(request));
    assert!(app.rendered_tile.as_ref().unwrap().is_valid_rgba_buffer());
    (ctx, app, viewport, page)
}

#[test]
fn actual_coverage_edge_requires_new_job() {
    let (ctx, mut app, viewport, page) = fixture(1.0);
    let coverage = tile_screen_rect(app.rendered_tile.as_ref().unwrap(), page);
    // Move left coverage edge just beyond the viewport by less than one raster pixel.
    let moved = page.translate(egui::vec2(viewport.left() - coverage.left() + 0.01, 0.0));
    let before = app.next_render_job_id;
    app.ensure_visible_tile(&ctx, viewport, moved);
    assert_eq!(app.next_render_job_id, before + 1);
    assert!(app.pending_tile_render.is_some());
}

#[test]
fn wrong_page_or_raster_dimensions_never_reuse() {
    for mismatch in 0..3 {
        let (ctx, mut app, viewport, page) = fixture(1.0);
        let tile = app.rendered_tile.as_mut().unwrap();
        match mismatch {
            0 => tile.page_index += 1,
            1 => tile.full_width += 1,
            _ => tile.full_height += 1,
        }
        let before = app.next_render_job_id;
        app.ensure_visible_tile(&ctx, viewport, page);
        assert_eq!(app.next_render_job_id, before + 1, "mismatch {mismatch}");
        assert!(app.pending_tile_render.is_some());
    }
}

#[test]
fn fractional_hidpi_pan_reuses_but_scale_change_does_not() {
    let (ctx, mut app, viewport, page) = fixture(1.75);
    assert_eq!(ctx.pixels_per_point(), 1.75);
    let before = app.next_render_job_id;
    app.ensure_visible_tile(&ctx, viewport, page.translate(egui::vec2(8.125, -3.25)));
    assert_eq!(app.next_render_job_id, before);
    assert!(app.pending_tile_render.is_none());
    ctx.set_pixels_per_point(2.0);
    let mut output = ctx.run_ui(Default::default(), |_| {});
    output.textures_delta.clear();
    app.ensure_visible_tile(&ctx, viewport, page);
    assert_eq!(app.next_render_job_id, before + 1);
}

#[test]
fn page_clipping_only_requires_pixels_on_page() {
    let (ctx, mut app, viewport, _) = fixture(1.0);
    for origin in [egui::pos2(100.0, 50.0), egui::pos2(-4900.0, -5000.0)] {
        let page = egui::Rect::from_min_size(origin, egui::vec2(5400.0, 5400.0));
        let request = visible_tile_request(
            0,
            Some(1.0),
            app.zoom,
            ctx.pixels_per_point(),
            viewport,
            page,
        )
        .unwrap();
        app.rendered_tile = Some(tile(request));
        let before = app.next_render_job_id;
        app.ensure_visible_tile(&ctx, viewport, page.translate(egui::vec2(8.0, 8.0)));
        assert_eq!(app.next_render_job_id, before);
        assert!(app.pending_tile_render.is_none());
    }
}

#[test]
fn unchanged_capped_request_does_not_schedule_an_identical_job() {
    let (ctx, mut app, _, _) = fixture(1.0);
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(3000.0, 2500.0));
    let page = egui::Rect::from_center_size(viewport.center(), egui::vec2(5400.0, 5400.0));
    let request = visible_tile_request(
        0,
        Some(1.0),
        app.zoom,
        ctx.pixels_per_point(),
        viewport,
        page,
    )
    .unwrap();
    assert_eq!(request.width, TILE_RENDER_MAX_EDGE);
    app.rendered_tile = Some(tile(request));
    assert!(!tile_screen_rect(app.rendered_tile.as_ref().unwrap(), page).contains_rect(viewport));
    let before = app.next_render_job_id;
    app.ensure_visible_tile(&ctx, viewport, page);
    assert_eq!(
        app.next_render_job_id, before,
        "unchanged capped request scheduled an identical job"
    );
    assert!(app.pending_tile_render.is_none());
}

#[test]
fn max_edge_truncation_cannot_reuse_uncovered_visible_pixels() {
    let (ctx, mut app, _, _) = fixture(1.0);
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(3000.0, 2500.0));
    let page = egui::Rect::from_center_size(viewport.center(), egui::vec2(5400.0, 5400.0));
    let request = visible_tile_request(
        0,
        Some(1.0),
        app.zoom,
        ctx.pixels_per_point(),
        viewport,
        page,
    )
    .unwrap();
    assert_eq!(request.width, TILE_RENDER_MAX_EDGE);
    app.rendered_tile = Some(tile(request));
    let moved = page.translate(egui::vec2(8.0, 0.0));
    let moved_request = visible_tile_request(
        0,
        Some(1.0),
        app.zoom,
        ctx.pixels_per_point(),
        viewport,
        moved,
    )
    .unwrap();
    let existing = app.rendered_tile.as_ref().unwrap();
    assert!(!existing.contains(&moved_request));
    assert!(!tile_screen_rect(existing, moved).contains_rect(viewport));
    let before = app.next_render_job_id;
    app.ensure_visible_tile(&ctx, viewport, moved);
    assert_eq!(app.next_render_job_id, before + 1);
    assert!(app.pending_tile_render.is_some());
}

#[test]
fn sixty_paused_pans_count_real_scheduled_jobs() {
    let (ctx, mut app, viewport, page) = fixture(1.0);
    let before = app.next_render_job_id;
    for step in 1..=60 {
        let moved = page.translate(egui::vec2(step as f32 * 8.0, 0.0));
        app.last_view_change = Some(Instant::now() - VIEW_RERENDER_IDLE - Duration::from_millis(1));
        app.ensure_visible_tile(&ctx, viewport, moved);
        // Deterministic successful completion: retain real scheduling, not worker timing.
        if let Some(pending) = app.pending_tile_render.take() {
            app.rendered_tile = Some(tile(pending.request));
        }
    }
    let jobs = app.next_render_job_id - before;
    println!("60 paused eight-point pans: {jobs} real tile submissions");
    assert!(jobs <= 8, "padding should amortize jobs, got {jobs}");
}

#[test]
fn paused_eight_point_pan_reuses_existing_visible_coverage() {
    let (ctx, mut app, viewport, page) = fixture(1.0);
    let moved = page.translate(egui::vec2(8.0, 0.0));
    assert!(tile_screen_rect(app.rendered_tile.as_ref().unwrap(), moved).contains_rect(viewport));
    let before = app.next_render_job_id;
    app.ensure_visible_tile(&ctx, viewport, moved);
    assert_eq!(
        app.next_render_job_id, before,
        "covered pan scheduled a new tile job"
    );
    assert!(app.pending_tile_render.is_none());
}
