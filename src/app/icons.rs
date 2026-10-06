//! Original, font-independent vector controls. No reference-product assets are used.
use crate::theme;
use egui::{Color32, Pos2, Rect, Stroke, StrokeKind, Vec2};

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
#[allow(dead_code)] // Some actions are currently exposed only through the More menu.
pub(super) enum Icon {
    ArrowLeft,
    ArrowRight,
    ZoomOut,
    ZoomIn,
    FitPage,
    FitWidth,
    Reset,
    View,
    Select,
    Rectangle,
    Ellipse,
    Delete,
    Sidebar,
    Pages,
    Bookmarks,
    Search,
    More,
    Links,
    Help,
    Retry,
}

impl Icon {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::ArrowLeft => "Previous page",
            Self::ArrowRight => "Next page",
            Self::ZoomOut => "Zoom out",
            Self::ZoomIn => "Zoom in",
            Self::FitPage => "Fit page",
            Self::FitWidth => "Fit width",
            Self::Reset => "Reset view",
            Self::View => "View",
            Self::Select => "Select markup",
            Self::Rectangle => "Rectangle",
            Self::Ellipse => "Ellipse",
            Self::Delete => "Delete",
            Self::Sidebar => "Toggle sidebar",
            Self::Pages => "Pages",
            Self::Bookmarks => "Bookmarks",
            Self::Search => "Search",
            Self::More => "More actions",
            Self::Links => "Links",
            Self::Help => "Help",
            Self::Retry => "Retry preview",
        }
    }
    const fn tooltip(self) -> &'static str {
        match self {
            Self::ArrowLeft => "Previous page",
            Self::ArrowRight => "Next page",
            Self::ZoomOut => "Zoom out",
            Self::ZoomIn => "Zoom in",
            Self::FitPage => "Fit page (Ctrl+1)",
            Self::FitWidth => "Fit width (Ctrl+2)",
            Self::Reset => "Reset view",
            Self::View => "View (Escape): pan and select PDF text",
            Self::Select => "Select markup (V): select Glyph shapes",
            Self::Rectangle => "Rectangle (R): drag a red, unfilled, 2 pt rectangle",
            Self::Ellipse => "Ellipse (E): drag a red, unfilled, 2 pt ellipse",
            Self::Delete => "Delete (Delete/Backspace): delete selected Glyph shape",
            Self::Sidebar => "Toggle sidebar",
            Self::Pages => "Pages: physical page thumbnails",
            Self::Bookmarks => "Bookmarks: document outline",
            Self::Search => "Search document text",
            Self::More => "More actions",
            Self::Links => "Links: PDF link tools",
            Self::Help => "Help: keyboard shortcuts and application information",
            Self::Retry => "Retry preview: reload retained markup changes",
        }
    }
}

/// A fixed 24-point target with a persistent identity scoped to the owning UI.
pub(super) fn button(
    ui: &mut egui::Ui,
    icon: Icon,
    enabled: bool,
    selected: bool,
) -> egui::Response {
    button_named(ui, icon, enabled, selected, icon.label(), icon.tooltip())
}

pub(super) fn button_named(
    ui: &mut egui::Ui,
    icon: Icon,
    enabled: bool,
    selected: bool,
    label: &str,
    tooltip: &str,
) -> egui::Response {
    ui.add_enabled(enabled, |ui: &mut egui::Ui| {
        let (_, allocation) = ui.allocate_space(Vec2::splat(24.));
        let rect = Rect::from_center_size(allocation.center(), Vec2::splat(24.));
        let response = ui.interact(
            rect,
            ui.make_persistent_id(("native-icon", icon)),
            egui::Sense::click(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), selected, label)
        });
        if ui.is_rect_visible(rect) {
            let interactive =
                response.hovered() || response.has_focus() || response.is_pointer_button_down_on();
            if selected || interactive {
                ui.painter().rect_stroke(
                    rect.shrink(1.),
                    4.,
                    Stroke::new(
                        1.,
                        theme::color(if selected {
                            theme::ACCENT
                        } else {
                            theme::STROKE_STRONG
                        }),
                    ),
                    StrokeKind::Inside,
                );
            }
            let ink = theme::color(if !ui.is_enabled() {
                theme::TEXT_MUTED
            } else if selected {
                theme::ACCENT
            } else {
                theme::TEXT
            });
            paint(ui.painter(), rect.center(), icon, ink);
        }
        response.on_hover_text(tooltip)
    })
}

// Coordinates are original line drawings on a 16-point grid, including stroke.
fn paint(painter: &egui::Painter, center: Pos2, icon: Icon, ink: Color32) {
    let stroke = Stroke::new(1.5, ink);
    let p = |x, y| center + egui::vec2(x, y);
    let line = |a: (f32, f32), b: (f32, f32)| {
        painter.line_segment([p(a.0, a.1), p(b.0, b.1)], stroke);
    };
    let rectangle = |x, y, w, h| {
        painter.rect_stroke(
            Rect::from_min_size(p(x, y), egui::vec2(w, h)),
            1.,
            stroke,
            StrokeKind::Inside,
        );
    };
    let circle = |x, y, r| {
        painter.circle_stroke(p(x, y), r, stroke);
    };
    match icon {
        Icon::ArrowLeft | Icon::ArrowRight => {
            let d = if icon == Icon::ArrowLeft { -1. } else { 1. };
            line((-6. * d, 0.), (6. * d, 0.));
            line((1. * d, -5.), (6. * d, 0.));
            line((1. * d, 5.), (6. * d, 0.));
        }
        Icon::ZoomOut | Icon::ZoomIn | Icon::Search => {
            circle(-2., -2., 4.5);
            line((1.5, 1.5), (6.5, 6.5));
            if icon != Icon::Search {
                line((-4.5, -2.), (0.5, -2.));
            }
            if icon == Icon::ZoomIn {
                line((-2., -4.5), (-2., 0.5));
            }
        }
        Icon::Rectangle => rectangle(-7., -5.5, 14., 11.),
        Icon::Ellipse => {
            painter.add(egui::Shape::ellipse_stroke(
                center,
                egui::vec2(7., 5.5),
                stroke,
            ));
        }
        Icon::FitPage => {
            rectangle(-4., -6., 8., 12.);
            for d in [-1., 1.] {
                line((-7., 4. * d), (-7., 7. * d));
                line((-7., 7. * d), (-4., 7. * d));
                line((7., 4. * d), (7., 7. * d));
                line((7., 7. * d), (4., 7. * d));
            }
        }
        Icon::FitWidth => {
            line((-7., -7.), (-7., 7.));
            line((7., -7.), (7., 7.));
            line((-5., 0.), (5., 0.));
            for d in [-1., 1.] {
                line((2. * d, -3.), (5. * d, 0.));
                line((2. * d, 3.), (5. * d, 0.));
            }
        }
        Icon::Reset | Icon::Retry => {
            // Broken circular arrow, made only of short vector segments.
            for (a, b) in [
                ((-5., -3.), (-2., -6.)),
                ((-2., -6.), (3., -5.)),
                ((3., -5.), (6., -1.)),
                ((6., -1.), (5., 4.)),
                ((5., 4.), (1., 6.)),
                ((1., 6.), (-4., 4.)),
            ] {
                line(a, b);
            }
            line((-5., -3.), (-6., -7.));
            line((-5., -3.), (-1., -3.));
        }
        Icon::View => {
            // Open hand/pan: palm and individually articulated fingers.
            for (a, b) in [
                ((-4., 5.), (-7., 0.)),
                ((-7., 0.), (-5., -1.)),
                ((-5., -1.), (-3., 1.)),
                ((-3., 1.), (-3., -6.)),
                ((-3., -6.), (-1., -6.)),
                ((-1., -6.), (-1., 0.)),
                ((1., 0.), (1., -7.)),
                ((1., -7.), (3., -7.)),
                ((3., -7.), (3., 0.)),
                ((5., 0.), (5., -4.)),
                ((5., -4.), (7., -4.)),
                ((7., -4.), (7., 2.)),
                ((7., 2.), (4., 6.)),
                ((4., 6.), (-4., 5.)),
            ] {
                line(a, b);
            }
        }
        Icon::Select => {
            for (a, b) in [
                ((-5., -7.), (-5., 6.)),
                ((-5., 6.), (-1., 2.)),
                ((-1., 2.), (2., 7.)),
                ((2., 7.), (4., 6.)),
                ((4., 6.), (1., 1.)),
                ((1., 1.), (6., 1.)),
                ((6., 1.), (-5., -7.)),
            ] {
                line(a, b);
            }
        }
        Icon::Delete => {
            line((-6., -4.), (6., -4.));
            rectangle(-2., -7., 4., 3.);
            line((-4., -2.), (-3., 7.));
            line((-3., 7.), (3., 7.));
            line((3., 7.), (4., -2.));
            line((-1., -1.), (-1., 4.));
            line((1., -1.), (1., 4.));
        }
        Icon::Sidebar => {
            rectangle(-7., -6., 14., 12.);
            line((-2., -5.), (-2., 5.));
            line((-5., -2.), (-4., -2.));
            line((-5., 1.), (-4., 1.));
        }
        Icon::Pages => {
            rectangle(-3., -7., 9., 11.);
            line((-6., -4.), (-6., 7.));
            line((-6., 7.), (3., 7.));
            line((-1., -3.), (4., -3.));
            line((-1., 0.), (3., 0.));
        }
        Icon::Bookmarks => {
            for (a, b) in [
                ((-5., -7.), (5., -7.)),
                ((5., -7.), (5., 7.)),
                ((5., 7.), (0., 3.)),
                ((0., 3.), (-5., 7.)),
                ((-5., 7.), (-5., -7.)),
            ] {
                line(a, b);
            }
        }
        Icon::More => {
            for x in [-5., 0., 5.] {
                circle(x, 0., 1.);
            }
        }
        Icon::Links => {
            for (a, b) in [
                ((-1., -3.), (2., -6.)),
                ((2., -6.), (6., -6.)),
                ((6., -6.), (6., -2.)),
                ((6., -2.), (3., 1.)),
                ((-3., -1.), (-6., 2.)),
                ((-6., 2.), (-6., 6.)),
                ((-6., 6.), (-2., 6.)),
                ((-2., 6.), (1., 3.)),
                ((-3., 3.), (3., -3.)),
            ] {
                line(a, b);
            }
        }
        Icon::Help => {
            circle(0., 0., 7.);
            for (a, b) in [
                ((-2., -3.), (-1., -4.)),
                ((-1., -4.), (2., -4.)),
                ((2., -4.), (3., -2.)),
                ((3., -2.), (0., 0.)),
                ((0., 0.), (0., 2.)),
            ] {
                line(a, b);
            }
            circle(0., 4.5, 0.5);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Icon; 20] = [
        Icon::ArrowLeft,
        Icon::ArrowRight,
        Icon::ZoomOut,
        Icon::ZoomIn,
        Icon::FitPage,
        Icon::FitWidth,
        Icon::Reset,
        Icon::View,
        Icon::Select,
        Icon::Rectangle,
        Icon::Ellipse,
        Icon::Delete,
        Icon::Sidebar,
        Icon::Pages,
        Icon::Bookmarks,
        Icon::Search,
        Icon::More,
        Icon::Links,
        Icon::Help,
        Icon::Retry,
    ];

    #[test]
    fn every_icon_has_bounded_finite_vector_ink_and_24_point_target() {
        for dark in [false, true] {
            for icon in ALL {
                for selected in [false, true] {
                    let ctx = egui::Context::default();
                    ctx.set_visuals(if dark {
                        egui::Visuals::dark()
                    } else {
                        egui::Visuals::light()
                    });
                    let mut rect = Rect::NOTHING;
                    let mut output = ctx.run_ui(Default::default(), |ui| {
                        rect = button(ui, icon, true, selected).rect;
                    });
                    output.textures_delta.clear();
                    assert_eq!(rect.size(), Vec2::splat(24.));
                    let mut ink = Rect::NOTHING;
                    for shape in &output.shapes {
                        if matches!(shape.shape, egui::Shape::Noop) {
                            continue;
                        }
                        assert!(
                            !matches!(shape.shape, egui::Shape::Text(_) | egui::Shape::Mesh(_)),
                            "icons must be font/texture independent"
                        );
                        let bounds = shape.shape.visual_bounding_rect();
                        assert!(bounds.is_finite(), "{icon:?}: {bounds:?}");
                        assert!(
                            rect.contains_rect(bounds),
                            "{icon:?}: ink {bounds:?} outside {rect:?}"
                        );
                        ink = ink.union(bounds);
                    }
                    assert!(
                        ink.is_finite() && ink.width() > 0. && ink.height() > 0.,
                        "{icon:?} needs visible ink"
                    );
                    for pixels_per_point in [1., 1.5, 2.] {
                        let primitives = ctx.tessellate(output.shapes.clone(), pixels_per_point);
                        for primitive in primitives {
                            if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive {
                                for vertex in mesh.vertices {
                                    let pixel = vertex.pos * pixels_per_point;
                                    let pixel_bounds = Rect::from_min_max(
                                        rect.min * pixels_per_point,
                                        rect.max * pixels_per_point,
                                    );
                                    assert!(
                                        pixel.is_finite() && pixel_bounds.contains(pixel),
                                        "{icon:?}: pixel {pixel:?} outside {pixel_bounds:?}"
                                    );
                                }
                            }
                        }
                    }
                    if !selected {
                        assert!(
                            ink.width() <= 16. && ink.height() <= 16.,
                            "{icon:?}: {ink:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn clicked_icons_emit_named_button_information_and_disabled_icons_never_click() {
        for icon in ALL {
            for enabled in [false, true] {
                let ctx = egui::Context::default();
                let mut center = Pos2::ZERO;
                let mut stable_id = None;
                let mut clicked = false;
                let mut identified = false;
                for step in 0..5 {
                    let events = match step {
                        3 | 4 => vec![
                            egui::Event::PointerMoved(center),
                            egui::Event::PointerButton {
                                pos: center,
                                button: egui::PointerButton::Primary,
                                pressed: step == 3,
                                modifiers: Default::default(),
                            },
                        ],
                        _ => vec![],
                    };
                    let mut output = ctx.run_ui(
                        egui::RawInput {
                            events,
                            ..Default::default()
                        },
                        |ui| {
                            let response = button(ui, icon, enabled, true);
                            center = response.rect.center();
                            if let Some(id) = stable_id {
                                assert_eq!(id, response.id);
                            }
                            stable_id = Some(response.id);
                            assert_eq!(response.enabled(), enabled);
                            clicked |= response.clicked();
                        },
                    );
                    output.textures_delta.clear();
                    identified |= output.platform_output.events.iter().any(|event| {
                        let info = event.widget_info();
                        info.typ == egui::WidgetType::Button
                            && info.label.as_deref() == Some(icon.label())
                            && info.enabled
                            && info.selected == Some(true)
                    });
                }
                assert_eq!(clicked, enabled, "{icon:?}");
                assert_eq!(identified, enabled, "accessible label missing for {icon:?}");
            }
        }
    }

    #[test]
    fn history_arrows_emit_one_correctly_named_click_event() {
        for (icon, label) in [(Icon::ArrowLeft, "Back"), (Icon::ArrowRight, "Forward")] {
            let ctx = egui::Context::default();
            let mut center = Pos2::ZERO;
            for step in 0..5 {
                let events = if step >= 3 {
                    vec![
                        egui::Event::PointerMoved(center),
                        egui::Event::PointerButton {
                            pos: center,
                            button: egui::PointerButton::Primary,
                            pressed: step == 3,
                            modifiers: Default::default(),
                        },
                    ]
                } else {
                    vec![]
                };
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        center = button_named(ui, icon, true, false, label, label)
                            .rect
                            .center();
                    },
                );
                output.textures_delta.clear();
                if step == 4 {
                    let clicks: Vec<_> = output
                        .platform_output
                        .events
                        .iter()
                        .filter(|e| matches!(e, egui::output::OutputEvent::Clicked(_)))
                        .collect();
                    assert_eq!(clicks.len(), 1);
                    assert_eq!(clicks[0].widget_info().label.as_deref(), Some(label));
                }
            }
        }
    }

    #[test]
    fn sibling_scopes_have_distinct_icon_ids() {
        let ctx = egui::Context::default();
        let mut ids = Vec::new();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            for salt in ["header", "sidebar"] {
                ui.push_id(salt, |ui| ids.push(button(ui, Icon::Pages, true, false).id));
            }
        });
        output.textures_delta.clear();
        assert_ne!(ids[0], ids[1]);
        assert!(Icon::Rectangle.tooltip().contains("2 pt"));
        assert!(Icon::Ellipse.tooltip().contains("2 pt"));
    }
}
