use super::*;
use crate::core::links::PdfRect;

#[derive(Clone, Copy, Default, PartialEq)]
pub(super) enum Mode {
    #[default]
    View,
    Select,
    Rectangle,
    Ellipse,
}
#[derive(Default)]
pub(super) struct MarkupState {
    pub mode: Mode,
    pub loaded: bool,
    // Retained pixels may belong to the previous edit revision until installation.
    pub preview_pending: bool,
    pub items: Vec<crate::pdf::ShapeAnnotation>,
    pub selected: Option<lopdf::ObjectId>,
    gesture: BoundingBoxGesture,
}
impl GlyphApp {
    fn choose_markup_mode(&mut self, mode: Mode, ctx: &egui::Context) {
        if !self.can_change_markups() {
            return;
        }
        self.markup.mode = mode;
        self.markup.gesture.draft = None;
        self.markup.selected = None;
        self.selection.clear();
        self.selecting_text = false;
        if mode != Mode::View && !self.markup.loaded {
            self.load_markups(ctx);
        }
        ctx.request_repaint();
    }
    pub(super) fn draw_markup_tools(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.preview_unavailable()
            && ui
                .add_enabled(
                    !self.edit_pending()
                        && self.loading_document.is_none()
                        && self.automation_rx.is_none()
                        && !self.editing_modal_open(),
                    tool_chip_button("Retry preview"),
                )
                .clicked()
        {
            self.load_markups(ctx);
        }
        let ready = self.can_change_markups();
        for (mode, label, hint) in [
            (Mode::View, "View", "Pan and select PDF text; Escape"),
            (Mode::Select, "Select markup", "Select Glyph shapes; V"),
            (
                Mode::Rectangle,
                "Rectangle",
                "Drag a red, unfilled rectangle; R",
            ),
            (Mode::Ellipse, "Ellipse", "Drag a red, unfilled ellipse; E"),
        ] {
            let stroke = if self.markup.mode == mode {
                theme::color(theme::ACCENT)
            } else {
                theme::color(theme::STROKE)
            };
            if ui
                .add_enabled(
                    ready,
                    tool_chip_button(label).stroke(egui::Stroke::new(1., stroke)),
                )
                .on_hover_text(hint)
                .clicked()
            {
                self.choose_markup_mode(mode, ctx);
            }
        }
        if ui
            .add_enabled(
                self.can_delete_selected_markup(),
                tool_chip_button("Delete"),
            )
            .on_hover_text("Delete selected Glyph shape; Delete/Backspace")
            .clicked()
        {
            self.delete_selected_markup(ctx);
        }
        ui.label(
            egui::RichText::new("Red · 2 pt")
                .size(11.)
                .color(theme::color(theme::TEXT_MUTED)),
        );
    }
    pub(super) fn markup_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input()
            || self.editing_modal_open()
            || egui::Popup::is_any_open(ctx)
        {
            return;
        }
        if self.markup.mode != Mode::View
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.markup.gesture.draft = None;
            self.markup.selected = None;
            self.markup.mode = Mode::View;
        }
        if !self.can_change_markups() {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::R)) {
            self.choose_markup_mode(Mode::Rectangle, ctx);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::E)) {
            self.choose_markup_mode(Mode::Ellipse, ctx);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::V)) {
            self.choose_markup_mode(Mode::Select, ctx);
        }
        if ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)
        }) {
            self.delete_selected_markup(ctx);
        }
    }
    pub(super) fn cancel_markup_selection(&mut self) {
        self.markup.selected = None;
        self.markup.gesture.draft = None;
    }
    fn can_delete_selected_markup(&self) -> bool {
        self.can_change_markups()
            && self.markup.mode == Mode::Select
            && self.markup.loaded
            && self.markup.items.iter().any(|a| {
                Some(a.object_id) == self.markup.selected
                    && a.page_index == self.project.selected_page
            })
    }
    fn delete_selected_markup(&mut self, ctx: &egui::Context) {
        if self.can_delete_selected_markup()
            && let Some(id) = self.markup.selected
        {
            self.delete_shape(id, ctx);
        }
    }
    pub(super) fn interact_with_markup(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        page: egui::Rect,
        viewport: egui::Rect,
        page_index: usize,
    ) {
        if self.markup.mode == Mode::View {
            return;
        }
        if !self.can_change_markups()
            || !self.markup.loaded
            || page_index != self.project.selected_page
        {
            self.markup.gesture.draft = None;
            return;
        }
        let ctx = ui.ctx();
        let layer = ui.layer_id();
        if response.hovered() {
            ctx.set_cursor_icon(
                if matches!(self.markup.mode, Mode::Rectangle | Mode::Ellipse) {
                    egui::CursorIcon::Crosshair
                } else {
                    egui::CursorIcon::Default
                },
            );
        }
        if matches!(self.markup.mode, Mode::Rectangle | Mode::Ellipse) {
            let identity = (self.document_generation, page_index);
            // Context callbacks must not nest: input() holds the context lock.
            // Resolve press ownership first, then pass only pure data to the gesture.
            let press = ui
                .input(|i| {
                    i.events.iter().find_map(|e| match e {
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            ..
                        } => Some(*pos),
                        _ => None,
                    })
                })
                .filter(|p| response.enabled() && ctx.layer_id_at(*p) == Some(layer));
            let rectangle = ui.input(|i| {
                self.markup
                    .gesture
                    .update(&i.events, page, viewport, identity, |p| press == Some(p))
            });
            if let Some(rect) = rectangle {
                if self.markup.mode == Mode::Ellipse {
                    self.add_ellipse(page_index, rect, ctx);
                } else {
                    self.add_rectangle(page_index, rect, ctx);
                }
            }
        } else if response.clicked_by(egui::PointerButton::Primary)
            && let Some(p) = response
                .interact_pointer_pos()
                .filter(|p| page.contains(*p) && viewport.contains(*p))
        {
            self.markup.selected = self
                .markup
                .items
                .iter()
                .rev()
                .find(|a| {
                    a.page_index == page_index
                        && shape_hit(a.kind, overlay_screen_rect(a.rect, page), p)
                })
                .map(|a| a.object_id);
        }
    }
    pub(super) fn paint_markup(
        &self,
        painter: &egui::Painter,
        page: egui::Rect,
        page_index: usize,
    ) {
        if let Some((identity, start, end)) = self.markup.gesture.draft
            && identity == (self.document_generation, page_index)
        {
            let r = egui::Rect::from_two_pos(start, end);
            let rect = overlay_screen_rect(
                PdfRect {
                    x: r.left(),
                    y: r.top(),
                    width: r.width(),
                    height: r.height(),
                },
                page,
            );
            if self.markup.mode == Mode::Ellipse {
                painter.add(egui::epaint::Shape::ellipse_stroke(
                    rect.center(),
                    rect.size() / 2.,
                    egui::Stroke::new(2., egui::Color32::from_rgb(220, 30, 30)),
                ));
            } else {
                painter.rect_stroke(
                    rect,
                    0.,
                    egui::Stroke::new(2., egui::Color32::from_rgb(220, 30, 30)),
                    egui::StrokeKind::Inside,
                );
            }
        }
        if self.markup.mode == Mode::Select
            && let Some(item) =
                self.markup.items.iter().find(|a| {
                    a.page_index == page_index && Some(a.object_id) == self.markup.selected
                })
        {
            painter.rect_stroke(
                overlay_screen_rect(item.rect, page).expand(3.),
                0.,
                egui::Stroke::new(1.5, theme::color(theme::ACCENT)),
                egui::StrokeKind::Outside,
            );
        }
    }
    pub(super) fn refresh_markup_render(&mut self, ctx: &egui::Context, snapshot: Option<Vec<u8>>) {
        self.markup.preview_pending = true;
        self.cancel_markup_selection();
        let Some(path) = self.project.document.as_ref().map(|d| d.path.clone()) else {
            return;
        };
        self.document_generation = self.document_generation.wrapping_add(1).max(1);
        if let Some(bytes) = snapshot {
            self.render_worker
                .replace_source(self.document_generation, path, bytes);
        } else {
            self.render_worker.reset(self.document_generation);
        }
        self.page_cache.clear();
        self.page_texture_cache.clear();
        self.page_cache_order.clear();
        self.page_links.clear();
        self.link_cache.clear();
        self.link_cache_order.clear();
        self.pending_links = None;
        self.pending_page_render = None;
        self.pending_tile_render = None;
        self.pending_prefetch_pages.clear();
        self.rendered_tile = None;
        self.tile_texture = None;
        self.thumbnails = thumbnails::ThumbnailState::default();
        self.clear_page_text();
        self.cancel_search();
        self.search_hits.clear();
        self.selected_search_hit = None;
        self.render_selected_page(ctx, BASE_RENDER_WIDTH);
        self.queue_page_links(ctx);
        self.queue_page_text();
    }
}

// Hit filled interiors even for unfilled annotations, with a screen-space halo.
fn shape_hit(kind: crate::pdf::ShapeKind, bounds: egui::Rect, p: egui::Pos2) -> bool {
    if !bounds.is_finite() || bounds.width() <= 0. || bounds.height() <= 0. || !p.is_finite() {
        return false;
    }
    match kind {
        crate::pdf::ShapeKind::Rectangle => bounds.expand(3.).contains(p),
        crate::pdf::ShapeKind::Ellipse => {
            // Expanding the radii approximates a 3-point halo without making the
            // empty bounding-box corners clickable; tolerance stays fixed at zoom.
            let offset = p - bounds.center();
            let radii = bounds.size() / 2. + egui::vec2(3., 3.);
            let normalized = offset / radii;
            normalized.length_sq() <= 1.
        }
    }
}

#[derive(Default)]
struct BoundingBoxGesture {
    draft: Option<((u64, usize), egui::Pos2, egui::Pos2)>,
}
impl BoundingBoxGesture {
    fn update(
        &mut self,
        events: &[egui::Event],
        page: egui::Rect,
        viewport: egui::Rect,
        identity: (u64, usize),
        owns: impl Fn(egui::Pos2) -> bool,
    ) -> Option<PdfRect> {
        if page.width() <= 0. || page.height() <= 0. || !page.is_finite() {
            self.draft = None;
            return None;
        }
        if self
            .draft
            .as_ref()
            .is_some_and(|(id, _, _)| *id != identity)
        {
            self.draft = None;
        }
        let normalized = |p: egui::Pos2| {
            egui::pos2(
                ((p.x - page.left()) / page.width()).clamp(0., 1.),
                ((p.y - page.top()) / page.height()).clamp(0., 1.),
            )
        };
        for event in events {
            match event {
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    ..
                } => {
                    self.draft = if pos.is_finite()
                        && page.contains(*pos)
                        && viewport.contains(*pos)
                        && owns(*pos)
                    {
                        let p = normalized(*pos);
                        Some((identity, p, p))
                    } else {
                        None
                    };
                }
                egui::Event::PointerMoved(pos) if pos.is_finite() => {
                    if let Some((_, _, end)) = &mut self.draft {
                        *end = normalized(*pos);
                    }
                }
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    ..
                } => {
                    if let Some((_, start, _)) = self.draft.take() {
                        if !pos.is_finite() {
                            continue;
                        }
                        let r = egui::Rect::from_two_pos(start, normalized(*pos));
                        if r.width() * page.width() >= 4. && r.height() * page.height() >= 4. {
                            return Some(PdfRect {
                                x: r.left(),
                                y: r.top(),
                                width: r.width(),
                                height: r.height(),
                            });
                        }
                    }
                }
                egui::Event::PointerGone
                | egui::Event::Key {
                    key: egui::Key::Escape,
                    pressed: true,
                    ..
                } => self.draft = None,
                _ => {}
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn click(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }
    }
    fn app(ctx: &egui::Context) -> GlyphApp {
        let mut app = GlyphApp::with_context(ctx, None);
        app.project.open_document(
            "test.pdf".into(),
            crate::pdf::PdfDocumentSummary {
                page_count: 2,
                pages: vec![],
                bookmarks: vec![],
                title: None,
            },
        );
        app.markup.loaded = true;
        app.markup.mode = Mode::Select;
        app.markup.items.push(crate::pdf::RectangleAnnotation {
            object_id: (9, 0),
            kind: crate::pdf::ShapeKind::Rectangle,
            page_index: 0,
            rect: PdfRect {
                x: 0.1,
                y: 0.1,
                width: 0.2,
                height: 0.2,
            },
        });
        app.markup.selected = Some((9, 0));
        app
    }
    // Drive real egui press/release frames through the production selection path.
    fn select_at(
        app: &mut GlyphApp,
        ctx: &egui::Context,
        page: egui::Rect,
        page_index: usize,
        pos: egui::Pos2,
    ) {
        select_at_with_cover(app, ctx, page, page_index, pos, false);
    }
    fn select_at_with_cover(
        app: &mut GlyphApp,
        ctx: &egui::Context,
        page: egui::Rect,
        page_index: usize,
        pos: egui::Pos2,
        covered: bool,
    ) {
        let mut clicked = false;
        for events in [
            vec![],
            vec![],
            vec![],
            vec![egui::Event::PointerMoved(pos), click(pos, true)],
            vec![click(pos, false)],
        ] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200., 1200.),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    if covered {
                        egui::Area::new(egui::Id::new("markup-cover"))
                            .order(egui::Order::Foreground)
                            .fixed_pos(page.min)
                            .show(ctx, |ui| {
                                ui.allocate_exact_size(page.size(), egui::Sense::click());
                            });
                    }
                    let response =
                        ui.interact(page, egui::Id::new("markup-page"), egui::Sense::click());
                    clicked |= response.clicked_by(egui::PointerButton::Primary);
                    app.interact_with_markup(ui, &response, page, page, page_index);
                },
            );
            output.textures_delta.clear();
        }
        assert_eq!(
            clicked, !covered,
            "real page click must respect layer ownership"
        );
    }
    #[test]
    fn ellipse_selection_rejects_empty_bounding_box_corner() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.markup.items[0].kind = crate::pdf::ShapeKind::Ellipse;
        app.markup.selected = None;
        let page = egui::Rect::from_min_size(egui::pos2(100., 150.), egui::vec2(500., 800.));
        let corner = overlay_screen_rect(app.markup.items[0].rect, page).min;
        select_at(&mut app, &ctx, page, 0, corner);
        assert_eq!(
            app.markup.selected, None,
            "an ellipse's empty bbox corner must not select it"
        );
    }
    #[test]
    fn ellipse_selection_uses_screen_tolerance_at_zoom_and_translation() {
        for size in [egui::vec2(200., 300.), egui::vec2(600., 900.)] {
            let ctx = egui::Context::default();
            let mut app = app(&ctx);
            app.markup.items[0].kind = crate::pdf::ShapeKind::Ellipse;
            let page = egui::Rect::from_min_size(egui::pos2(150., 100.), size);
            let bounds = overlay_screen_rect(app.markup.items[0].rect, page);
            for (pos, hit) in [
                (bounds.center(), true),
                (bounds.center() + bounds.size() * 0.2, true),
                (bounds.right_center(), true),
                (bounds.right_center() + egui::vec2(2.5, 0.), true),
                (bounds.right_center() + egui::vec2(3.5, 0.), false),
                (bounds.center_top() - egui::vec2(0., 2.5), true),
                (bounds.center_top() - egui::vec2(0., 3.5), false),
                (bounds.min, false),
                (bounds.max, false),
            ] {
                // A sentinel proves both hits and misses actually update selection.
                app.markup.selected = Some((99, 0));
                select_at(&mut app, &ctx, page, 0, pos);
                assert_eq!(
                    app.markup.selected,
                    hit.then_some((9, 0)),
                    "size={size:?}, pos={pos:?}"
                );
                assert!(!app.edit_pending() && !app.editing.dirty);
            }
        }
    }
    #[test]
    fn ellipse_corner_falls_through_to_rectangle_on_current_page_only() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        let rectangle = app.markup.items[0].clone();
        let mut ellipse = rectangle.clone();
        ellipse.kind = crate::pdf::ShapeKind::Ellipse;
        ellipse.object_id = (10, 0);
        let mut off_page = rectangle.clone();
        off_page.page_index = 1;
        off_page.object_id = (11, 0);
        app.markup.items.extend([ellipse, off_page]);
        let before = app.markup.items.clone();
        let generation = app.document_generation;
        let page = egui::Rect::from_min_size(egui::pos2(140., 120.), egui::vec2(700., 900.));
        let bounds = overlay_screen_rect(rectangle.rect, page);
        select_at(&mut app, &ctx, page, 0, bounds.min);
        assert_eq!(
            app.markup.selected,
            Some((9, 0)),
            "ellipse corner must fall through; off-page rectangle must be ignored"
        );
        select_at(&mut app, &ctx, page, 0, bounds.center());
        assert_eq!(
            app.markup.selected,
            Some((10, 0)),
            "topmost on-page ellipse wins inside"
        );
        select_at(&mut app, &ctx, page, 0, bounds.min - egui::vec2(2., 2.));
        assert_eq!(
            app.markup.selected,
            Some((9, 0)),
            "rectangle expanded-bbox tolerance stays compatible"
        );
        app.markup.selected = None;
        select_at(&mut app, &ctx, page, 1, bounds.center());
        assert_eq!(
            app.markup.selected, None,
            "an inactive page must not select anything"
        );
        assert_eq!(app.markup.items, before);
        assert_eq!(app.document_generation, generation);
        assert!(!app.edit_pending() && !app.editing.dirty && !app.markup.preview_pending);
    }
    #[test]
    fn ellipse_selection_respects_covering_layer_and_readiness() {
        let page = egui::Rect::from_min_size(egui::pos2(100., 100.), egui::vec2(500., 700.));
        for restriction in 0..4 {
            let ctx = egui::Context::default();
            let mut app = app(&ctx);
            app.markup.items[0].kind = crate::pdf::ShapeKind::Ellipse;
            app.markup.selected = None;
            match restriction {
                1 => app.markup.loaded = false,
                2 => app.markup.preview_pending = true,
                3 => app.markup.mode = Mode::View,
                _ => {}
            }
            let pos = overlay_screen_rect(app.markup.items[0].rect, page).center();
            select_at_with_cover(&mut app, &ctx, page, 0, pos, restriction == 0);
            assert_eq!(app.markup.selected, None, "restriction={restriction}");
            assert!(!app.edit_pending() && !app.editing.dirty);
        }
    }
    #[test]
    fn shape_hit_rejects_nonfinite_and_empty_geometry() {
        use crate::pdf::ShapeKind::{Ellipse, Rectangle};
        let valid = egui::Rect::from_min_size(egui::pos2(10., 10.), egui::vec2(20., 30.));
        for kind in [Ellipse, Rectangle] {
            for bounds in [
                egui::Rect::from_min_size(valid.min, egui::vec2(0., 30.)),
                egui::Rect::from_min_size(valid.min, egui::vec2(20., -1.)),
                egui::Rect::from_min_size(valid.min, egui::vec2(f32::INFINITY, 30.)),
                egui::Rect::from_min_size(egui::pos2(f32::NAN, 10.), valid.size()),
            ] {
                assert!(!shape_hit(kind, bounds, valid.min));
            }
            assert!(!shape_hit(kind, valid, egui::pos2(f32::NAN, 20.)));
            assert!(!shape_hit(kind, valid, egui::pos2(f32::INFINITY, 20.)));
        }
    }
    #[test]
    fn rectangle_navigation_clears_selection_and_draft_only_on_page_change() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.markup.gesture.draft = Some(((0, 0), egui::pos2(0.1, 0.1), egui::pos2(0.2, 0.2)));
        app.select_page_without_history(0, &ctx);
        assert!(app.markup.selected.is_some() && app.markup.gesture.draft.is_some());
        app.select_page_without_history(1, &ctx);
        assert!(
            app.markup.selected.is_none(),
            "navigation must clear selection"
        );
        assert!(
            app.markup.gesture.draft.is_none(),
            "navigation must cancel draft"
        );
    }
    #[test]
    fn rectangle_stale_off_page_selection_cannot_dispatch_delete() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.project.selected_page = 1;
        assert!(
            !app.can_delete_selected_markup(),
            "off-page ID must disable Delete"
        );
        app.delete_selected_markup(&ctx);
        assert!(
            !app.edit_pending(),
            "off-page ID must not reach editing worker"
        );
    }
    #[test]
    fn view_and_popup_leave_escape_to_the_menu_owner() {
        for popup in [false, true] {
            let ctx = egui::Context::default();
            let mut app = GlyphApp::with_context(&ctx, None);
            if popup {
                app.markup.mode = Mode::Rectangle;
            }
            let mut frame = ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Default::default(),
                    }],
                    ..Default::default()
                },
                |_| {
                    if popup {
                        egui::Popup::open_id(&ctx, egui::Id::new("document_menu_test"));
                    }
                    app.markup_shortcuts(&ctx);
                    assert!(
                        ctx.input(|i| i.key_pressed(egui::Key::Escape)),
                        "View mode or an open menu must keep Escape available for its owner"
                    );
                },
            );
            frame.textures_delta.clear();
        }
    }
    #[test]
    fn rectangle_gestures_ignore_wrong_layer_tiny_clicks_and_cancel_on_identity_or_escape() {
        let page = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100., 100.));
        let press = click(egui::pos2(20., 20.), true);
        let release = click(egui::pos2(80., 80.), false);
        let mut g = BoundingBoxGesture::default();
        assert!(
            g.update(
                &[press.clone(), release.clone()],
                page,
                page,
                (1, 0),
                |_| false
            )
            .is_none()
        );
        assert!(
            g.update(
                &[press.clone(), click(egui::pos2(21., 21.), false)],
                page,
                page,
                (1, 0),
                |_| true
            )
            .is_none()
        );
        g.update(std::slice::from_ref(&press), page, page, (1, 0), |_| true);
        assert!(
            g.update(std::slice::from_ref(&release), page, page, (1, 1), |_| true)
                .is_none()
        );
        g.update(std::slice::from_ref(&press), page, page, (1, 0), |_| true);
        assert!(
            g.update(std::slice::from_ref(&release), page, page, (2, 0), |_| true)
                .is_none()
        );
        for cancel in [
            egui::Event::PointerGone,
            egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            },
        ] {
            g.update(std::slice::from_ref(&press), page, page, (1, 0), |_| true);
            assert!(
                g.update(&[cancel, release.clone()], page, page, (1, 0), |_| true)
                    .is_none()
            );
            assert!(g.draft.is_none());
        }
        assert!(
            g.update(
                &[click(egui::pos2(110., 20.), true), release],
                page,
                page,
                (1, 0),
                |_| true
            )
            .is_none()
        );
    }
    #[test]
    fn batched_reverse_drag_clamps_release_to_page_and_commits_once() {
        let page = egui::Rect::from_min_size(egui::pos2(100., 200.), egui::vec2(200., 400.));
        let mut g = BoundingBoxGesture::default();
        let events = vec![
            click(egui::pos2(280., 560.), true),
            egui::Event::PointerMoved(egui::pos2(50., 100.)),
            click(egui::pos2(50., 100.), false),
        ];
        let r = g
            .update(&events, page, page, (1, 0), |_| true)
            .expect("Batched primary gesture must create a PDF rectangle");
        assert!(
            (r.x - 0.).abs() < 0.001
                && (r.y - 0.).abs() < 0.001
                && (r.width - 0.9).abs() < 0.001
                && (r.height - 0.9).abs() < 0.001
        );
        assert!(g.update(&[], page, page, (1, 0), |_| true).is_none());
        assert!(g.draft.is_none());
    }
}
