use super::*;
use crate::core::links::PdfRect;

#[derive(Clone, Copy, Default, PartialEq)]
pub(super) enum Mode {
    #[default]
    View,
    Select,
    Rectangle,
    Ellipse,
    Line,
    Arrow,
    Text,
}
#[derive(Default)]
pub(super) struct MarkupState {
    pub mode: Mode,
    pub loaded: bool,
    // Retained pixels may belong to the previous edit revision until installation.
    pub preview_pending: bool,
    pub items: Vec<crate::pdf::ShapeAnnotation>,
    pub selected: Option<lopdf::ObjectId>,
    pub next_style: crate::pdf::ShapeStyle,
    pub(super) text_draft: Option<TextDraft>,
    text_save_requested: Option<SaveIntent>,
    properties: Option<PropertiesDraft>,
    pub page_points: Option<(usize, egui::Vec2)>,
    edit_gesture: EditGesture,
    gesture: BoundingBoxGesture,
    line_gesture: LineGesture,
}
#[derive(Clone, Copy)]
pub(super) enum SaveIntent {
    Save,
    SaveAs,
}
pub(super) struct TextDraft {
    identity: (u64, usize),
    original: Option<crate::pdf::ShapeAnnotation>,
    rect: PdfRect,
    text: crate::pdf::TextMarkup,
    style: crate::pdf::ShapeStyle,
    focus: bool,
    error: Option<String>,
}
struct PropertiesDraft {
    identity: (u64, usize),
    selected: Option<lopdf::ObjectId>,
    style: crate::pdf::ShapeStyle,
}
impl GlyphApp {
    // A draft is a separate unsaved owner, never a fabricated PDF dirty bit.
    pub(super) fn block_inline_text_transition(&mut self) -> bool {
        if let Some(draft) = &mut self.markup.text_draft {
            let message =
                "Apply text or Cancel the text draft before closing or opening another PDF.";
            draft.error = Some(message.into());
            self.status = message.into();
            return true;
        }
        false
    }
    pub(super) fn cancel_inline_text(&mut self, ctx: &egui::Context) {
        if self.markup.text_draft.take().is_some() {
            ctx.memory_mut(|m| m.surrender_focus(egui::Id::new("inline-markup-text")));
        }
        self.markup.text_save_requested = None;
    }
    pub(super) fn request_inline_text_save(
        &mut self,
        intent: SaveIntent,
        ctx: &egui::Context,
    ) -> bool {
        if self.markup.text_draft.is_none() {
            return false;
        }
        self.markup.text_save_requested = Some(intent);
        ctx.request_repaint();
        true
    }
    pub(super) fn inline_text_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        self.validate_text_owner(ctx);
        if self.markup.text_draft.is_none() {
            return false;
        }
        if self.editing_modal_open() {
            return true; // the unsaved decision, not TextEdit, owns Escape/Save
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.cancel_inline_text(ctx);
            self.cancel_markup_gestures();
            ctx.memory_mut(|m| m.surrender_focus(egui::Id::new("inline-markup-text")));
        } else if ctx.input_mut(|i| {
            i.consume_key(
                egui::Modifiers {
                    shift: true,
                    ..egui::Modifiers::COMMAND
                },
                egui::Key::S,
            )
        }) {
            self.markup.text_save_requested = Some(SaveIntent::SaveAs);
        } else if ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::COMMAND, egui::Key::W)
                || i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)
        }) {
            self.block_inline_text_transition();
        } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            // Process TextEdit first: text input and Save may arrive in one frame.
            self.markup.text_save_requested = Some(SaveIntent::Save);
        }
        true
    }
    fn validate_text_owner(&mut self, ctx: &egui::Context) {
        if self
            .markup
            .text_draft
            .as_ref()
            .is_some_and(|d| d.identity != (self.document_generation, self.project.selected_page))
            || (!self.can_change_markups() && !self.inline_text_unsaved_decision())
            || ctx.input(|i| {
                i.events
                    .iter()
                    .any(|e| matches!(e, egui::Event::WindowFocused(false)))
            })
        {
            self.markup.text_draft = None;
            self.markup.text_save_requested = None;
            ctx.memory_mut(|m| m.surrender_focus(egui::Id::new("inline-markup-text")));
        }
    }
    fn draw_inline_text(&mut self, ctx: &egui::Context, page: egui::Rect, viewport: egui::Rect) {
        if self.editing_modal_open() {
            return; // retain the draft while an existing unsaved decision owns input
        }
        let Some(draft) = &mut self.markup.text_draft else {
            return;
        };
        let bounds = overlay_screen_rect(draft.rect, page);
        let scale = self
            .markup
            .page_points
            .map_or(1., |(_, size)| page.width() / size.x);
        let mut apply = false;
        let mut cancel = false;
        if !viewport.is_finite() || !viewport.is_positive() {
            return;
        }
        // TextEdit's desired rows/size is a minimum, not a maximum. Its full
        // content must scroll independently of the always-accessible controls.
        let width = bounds
            .width()
            .clamp(180., 500.)
            .min((viewport.width() - 28.).max(1.));
        let text_height = bounds
            .height()
            .clamp(60., 220.)
            .min((viewport.height() - 190.).max(1.));
        let popup_size = egui::vec2(width + 14., text_height + 176.);
        let max_pos = (viewport.max - popup_size - egui::vec2(7., 7.)).max(viewport.min);
        let position = bounds
            .min
            .max(viewport.min + egui::vec2(7., 7.))
            .min(max_pos);
        egui::Area::new(egui::Id::new("inline-markup-editor"))
            .order(egui::Order::Foreground)
            .constrain_to(viewport)
            .fixed_pos(position)
            .show(ctx, |ui| {
                ui.set_clip_rect(viewport);
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(width);
                    let text_label = ui.label("Edit text on PDF page");
                    let response = egui::ScrollArea::vertical()
                        .id_salt("inline-text-scroll")
                        .max_height(text_height)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut draft.text.contents)
                                    .desired_width(width)
                                    .desired_rows(3)
                                    .id(egui::Id::new("inline-markup-text"))
                                    .font(egui::FontId::monospace(
                                        (draft.text.size * scale).clamp(8., 96.),
                                    )),
                            )
                        })
                        .inner;
                    let response = response.labelled_by(text_label.id);
                    if draft.focus {
                        response.request_focus();
                        draft.focus = false;
                    }
                    ui.horizontal(|ui| {
                        let label = ui.label("Font size (pt)");
                        ui.add(
                            egui::DragValue::new(&mut draft.text.size)
                                .range(6. ..=144.)
                                .suffix(" pt"),
                        )
                        .labelled_by(label.id);
                    });
                    ui.label("Courier · printable ASCII + line breaks");
                    if let Some(error) = &draft.error {
                        egui::ScrollArea::vertical()
                            .id_salt("inline-text-error-scroll")
                            .max_height(48.)
                            .show(ui, |ui| {
                                ui.colored_label(egui::Color32::RED, error);
                            });
                    }
                    ui.horizontal(|ui| {
                        apply = ui.button("Apply text").clicked();
                        cancel = ui.button("Cancel").clicked();
                    });
                });
            });
        if cancel {
            self.cancel_markup_gestures();
            ctx.memory_mut(|m| m.surrender_focus(egui::Id::new("inline-markup-text")));
        } else if (apply || self.markup.text_save_requested.is_some())
            && self.finish_inline_text(ctx)
        {
            let save = self.markup.text_save_requested.take();
            if let Some(intent) = save {
                self.defer_inline_text_save(intent, ctx);
            }
        } else {
            self.markup.text_save_requested = None;
        }
    }
    pub(super) fn finish_inline_text(&mut self, ctx: &egui::Context) -> bool {
        let Some(mut draft) = self.markup.text_draft.take() else {
            return true;
        };
        if !self.can_change_markups()
            || draft.identity != (self.document_generation, self.project.selected_page)
        {
            return false;
        }
        if let Err(error) =
            self.validate_inline_text(draft.identity.1, draft.rect, &draft.text, draft.style)
        {
            draft.error = Some(error.to_string());
            self.markup.text_draft = Some(draft);
            return false;
        }
        if let Some(mut shape) = draft.original {
            shape.text = Some(draft.text);
            shape.style = draft.style;
            self.update_markup(shape, ctx);
        } else {
            self.add_text_markup(draft.identity.1, draft.rect, draft.text, draft.style, ctx);
        }
        self.markup.mode = Mode::Select;
        ctx.memory_mut(|m| m.surrender_focus(egui::Id::new("inline-markup-text")));
        true
    }
    fn choose_markup_mode(&mut self, mode: Mode, ctx: &egui::Context) {
        if !self.can_change_markups() {
            return;
        }
        self.markup.mode = mode;
        self.markup.text_draft = None;
        self.markup.text_save_requested = None;
        self.markup.edit_gesture = EditGesture::default();
        self.markup.properties = None;
        self.markup.gesture.draft = None;
        self.markup.line_gesture = LineGesture::default();
        self.markup.selected = None;
        self.selection.clear();
        self.selecting_text = false;
        if mode != Mode::View && !self.markup.loaded {
            self.load_markups(ctx);
        }
        ctx.request_repaint();
    }
    pub(super) fn draw_markup_tools(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.validate_text_owner(ctx);
        use super::icons::{self, Icon};
        if self.preview_unavailable()
            && icons::button(
                ui,
                Icon::Retry,
                !self.edit_pending()
                    && self.loading_document.is_none()
                    && self.automation_rx.is_none()
                    && !self.editing_modal_open(),
                false,
            )
            .clicked()
        {
            self.load_markups(ctx);
        }
        let ready = self.can_change_markups();
        if !ready
            || ctx.input(|i| {
                i.events
                    .iter()
                    .any(|e| matches!(e, egui::Event::WindowFocused(false)))
            })
        {
            self.markup.edit_gesture = EditGesture::default();
            self.markup.gesture = BoundingBoxGesture::default();
            self.markup.line_gesture = LineGesture::default();
        }
        for (mode, icon) in [
            (Mode::View, Icon::View),
            (Mode::Select, Icon::Select),
            (Mode::Rectangle, Icon::Rectangle),
            (Mode::Ellipse, Icon::Ellipse),
            (Mode::Line, Icon::Line),
            (Mode::Arrow, Icon::Arrow),
            (Mode::Text, Icon::Text),
        ] {
            if icons::button(ui, icon, ready, self.markup.mode == mode).clicked() {
                self.choose_markup_mode(mode, ctx);
            }
        }
        self.draw_markup_properties(ui, ctx);
        if icons::button(ui, Icon::Delete, self.can_delete_selected_markup(), false).clicked() {
            self.delete_selected_markup(ctx);
        }
    }
    fn draw_markup_properties(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let identity = (self.document_generation, self.project.selected_page);
        let ready = self.can_change_markups();
        let blurred = ctx.input(|i| {
            i.events
                .iter()
                .any(|e| matches!(e, egui::Event::WindowFocused(false)))
        });
        if !ready
            || blurred
            || self
                .markup
                .properties
                .as_ref()
                .is_some_and(|d| d.identity != identity || d.selected != self.markup.selected)
        {
            self.markup.properties = None;
        }
        ui.push_id("markup-properties", |ui| {
            let response = icons::button_named(
                ui,
                icons::Icon::More,
                ready,
                false,
                "Markup properties",
                "Markup properties: RGB color and stroke width in PDF points",
            );
            if response.clicked() {
                let shape = self.markup.items.iter().find(|a| {
                    self.markup.mode == Mode::Select
                        && a.page_index == identity.1
                        && Some(a.object_id) == self.markup.selected
                });
                self.markup.properties = Some(PropertiesDraft {
                    identity,
                    selected: shape.map(|a| a.object_id),
                    style: shape.map_or(self.markup.next_style, |a| a.style),
                });
                self.markup.edit_gesture = EditGesture::default();
                self.markup.gesture = BoundingBoxGesture::default();
                self.markup.line_gesture = LineGesture::default();
            }
            let mut apply = false;
            if ready && let Some(draft) = &mut self.markup.properties {
                egui::Popup::menu(&response)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| {
                        ui.set_width(208.);
                        ui.label(if draft.selected.is_some() {
                            "Selected markup"
                        } else {
                            "Next shapes"
                        });
                        for (channel, label) in
                            draft.style.rgb.iter_mut().zip(["Red", "Green", "Blue"])
                        {
                            ui.horizontal(|ui| {
                                ui.label(label);
                                ui.add(
                                    egui::DragValue::new(channel)
                                        .range(0. ..=1.)
                                        .speed(0.01)
                                        .fixed_decimals(2),
                                )
                                .on_hover_text(format!("{label}: 0 to 1"));
                            });
                        }
                        ui.horizontal(|ui| {
                            ui.label("Width (pt)");
                            ui.add(
                                egui::DragValue::new(&mut draft.style.weight)
                                    .range(0.1..=64.)
                                    .speed(0.1)
                                    .suffix(" pt"),
                            );
                        });
                        ui.horizontal(|ui| {
                            apply = ui
                                .add_enabled(
                                    draft.style.validate().is_ok(),
                                    egui::Button::new("Apply"),
                                )
                                .clicked();
                            if apply || ui.button("Cancel").clicked() {
                                ui.close();
                            }
                        });
                    });
            }
            if apply && let Some(draft) = self.markup.properties.take() {
                if let Some(id) = draft.selected {
                    if let Some(mut shape) = self
                        .markup
                        .items
                        .iter()
                        .find(|a| a.object_id == id && a.page_index == identity.1)
                        .cloned()
                    {
                        shape.style = draft.style;
                        self.update_markup(shape, ctx);
                    }
                } else {
                    self.markup.next_style = draft.style;
                }
            }
        });
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
            self.markup.edit_gesture = EditGesture::default();
            self.markup.gesture.draft = None;
            self.markup.line_gesture = LineGesture::default();
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
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::L)) {
            self.choose_markup_mode(Mode::Line, ctx);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::A)) {
            self.choose_markup_mode(Mode::Arrow, ctx);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::V)) {
            self.choose_markup_mode(Mode::Select, ctx);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::T)) {
            self.choose_markup_mode(Mode::Text, ctx);
        }
        if ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)
        }) {
            self.delete_selected_markup(ctx);
        }
    }
    // Input-owner transitions discard drafts, not the selected object or tool.
    pub(super) fn cancel_markup_gestures(&mut self) {
        self.markup.text_draft = None;
        self.markup.text_save_requested = None;
        self.markup.properties = None;
        self.markup.edit_gesture = EditGesture::default();
        self.markup.gesture = BoundingBoxGesture::default();
        self.markup.line_gesture = LineGesture::default();
    }
    pub(super) fn cancel_markup_selection(&mut self) {
        self.markup.selected = None;
        self.markup.page_points = None;
        self.cancel_markup_gestures();
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
        self.validate_text_owner(ui.ctx());
        if self.markup.text_draft.is_some() {
            self.draw_inline_text(ui.ctx(), page, viewport);
            return;
        }
        if self.markup.mode == Mode::View {
            self.markup.edit_gesture = EditGesture::default();
            return;
        }
        if !self.can_change_markups()
            || egui::Popup::is_any_open(ui.ctx())
            || !self.markup.loaded
            || page_index != self.project.selected_page
        {
            self.markup.edit_gesture = EditGesture::default();
            self.markup.gesture.draft = None;
            self.markup.line_gesture = LineGesture::default();
            return;
        }
        if self.markup.page_points.is_none_or(|(p, _)| p != page_index) {
            self.markup.page_points = self
                .markup_page_size(page_index)
                .map(|size| (page_index, size));
        }
        let ctx = ui.ctx();
        let layer = ui.layer_id();
        if response.hovered() {
            ctx.set_cursor_icon(
                if matches!(
                    self.markup.mode,
                    Mode::Rectangle | Mode::Ellipse | Mode::Line | Mode::Arrow
                ) {
                    egui::CursorIcon::Crosshair
                } else {
                    egui::CursorIcon::Default
                },
            );
        }
        if self.markup.mode == Mode::Text {
            if response.clicked_by(egui::PointerButton::Primary)
                && let Some(pos) = response
                    .interact_pointer_pos()
                    .filter(|p| page.contains(*p) && viewport.contains(*p))
            {
                let original = self
                    .markup
                    .items
                    .iter()
                    .rev()
                    .find(|a| {
                        a.page_index == page_index
                            && a.kind == crate::pdf::ShapeKind::Text
                            && overlay_screen_rect(a.rect, page).contains(pos)
                    })
                    .cloned();
                let x = ((pos.x - page.left()) / page.width()).clamp(0., 0.98);
                let y = ((pos.y - page.top()) / page.height()).clamp(0., 0.98);
                let rect = original.as_ref().map_or(
                    PdfRect {
                        x,
                        y,
                        width: 0.5f32.min(1. - x),
                        height: 0.25f32.min(1. - y),
                    },
                    |a| a.rect,
                );
                self.markup.selected = original.as_ref().map(|a| a.object_id);
                self.markup.text_draft = Some(TextDraft {
                    identity: (self.document_generation, page_index),
                    rect,
                    text: original.as_ref().and_then(|a| a.text.clone()).unwrap_or(
                        crate::pdf::TextMarkup {
                            contents: String::new(),
                            size: 18.,
                        },
                    ),
                    style: original
                        .as_ref()
                        .map_or(self.markup.next_style, |a| a.style),
                    original,
                    focus: true,
                    error: None,
                });
                ctx.request_repaint();
            }
        } else if matches!(self.markup.mode, Mode::Line | Mode::Arrow) {
            let events = ui.input(|i| i.events.clone());
            let owned: Vec<_> = events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        ..
                    } if response.enabled() && ctx.layer_id_at(*pos) == Some(layer) => Some(*pos),
                    _ => None,
                })
                .collect();
            let identity = (self.document_generation, page_index);
            if let Some(endpoints) =
                self.markup
                    .line_gesture
                    .update(&events, page, viewport, identity, |p| owned.contains(&p))
            {
                self.add_line(page_index, endpoints, self.markup.mode == Mode::Arrow, ctx);
            }
        } else if matches!(self.markup.mode, Mode::Rectangle | Mode::Ellipse) {
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
        } else {
            let events = ui.input(|i| i.events.clone());
            let owned: Vec<_> = events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        ..
                    }
                    | egui::Event::PointerMoved(pos)
                        if response.enabled() && ctx.layer_id_at(*pos) == Some(layer) =>
                    {
                        Some(*pos)
                    }
                    _ => None,
                })
                .collect();
            if let Some(shape) = self.markup.edit_gesture.update(
                &events,
                page,
                viewport,
                (self.document_generation, page_index),
                &self.markup.items,
                &mut self.markup.selected,
                |p| owned.contains(&p),
            ) {
                self.update_markup(shape, ctx);
            }
        }
    }
    pub(super) fn paint_markup(
        &self,
        painter: &egui::Painter,
        page: egui::Rect,
        page_index: usize,
    ) {
        let scale = self
            .markup
            .page_points
            .filter(|(i, _)| *i == page_index)
            .map_or(1., |(_, size)| page.width() / size.x);
        let stroke = |style: crate::pdf::ShapeStyle| {
            egui::Stroke::new(
                style.weight * scale,
                egui::Color32::from_rgb(
                    (style.rgb[0] * 255.).round() as u8,
                    (style.rgb[1] * 255.).round() as u8,
                    (style.rgb[2] * 255.).round() as u8,
                ),
            )
        };
        let paint_box = |rect: egui::Rect, kind: crate::pdf::ShapeKind, s: egui::Stroke| {
            if kind == crate::pdf::ShapeKind::Ellipse {
                painter.add(egui::epaint::Shape::ellipse_stroke(
                    rect.center(),
                    (rect.size() / 2. - egui::Vec2::splat(s.width / 2.)).max(egui::Vec2::ZERO),
                    s,
                ));
            } else {
                painter.rect_stroke(rect, 0., s, egui::StrokeKind::Inside);
            }
        };
        let paint_line = |endpoints: [f32; 4], arrow: bool, s: egui::Stroke| {
            let [x, y, z, t] = endpoints;
            let a = screen_point(egui::pos2(x, y), page);
            let b = screen_point(egui::pos2(z, t), page);
            painter.line_segment([a, b], s);
            if arrow && a != b {
                let direction = (b - a).normalized();
                let side = egui::vec2(-direction.y, direction.x);
                let size = (10. * scale).min(a.distance(b) * 0.4);
                for sign in [-1., 1.] {
                    painter.line_segment([b, b - direction * size + side * (sign * size * 0.5)], s);
                }
            }
        };
        if let Some((identity, start, end)) = self.markup.gesture.draft
            && identity == (self.document_generation, page_index)
        {
            let r = egui::Rect::from_two_pos(start, end);
            paint_box(
                overlay_screen_rect(
                    PdfRect {
                        x: r.left(),
                        y: r.top(),
                        width: r.width(),
                        height: r.height(),
                    },
                    page,
                ),
                if self.markup.mode == Mode::Ellipse {
                    crate::pdf::ShapeKind::Ellipse
                } else {
                    crate::pdf::ShapeKind::Rectangle
                },
                stroke(self.markup.next_style),
            );
        }
        if let Some((identity, start, end)) = self.markup.line_gesture.draft
            && identity == (self.document_generation, page_index)
        {
            paint_line(
                [start.x, start.y, end.x, end.y],
                self.markup.mode == Mode::Arrow,
                stroke(self.markup.next_style),
            );
        }
        let preview = self
            .markup
            .edit_gesture
            .draft
            .as_ref()
            .filter(|d| d.identity == (self.document_generation, page_index))
            .map(|d| &d.preview);
        if let Some(item) = preview {
            if let Some(endpoints) = item.endpoints {
                paint_line(
                    endpoints,
                    item.kind == crate::pdf::ShapeKind::Arrow,
                    stroke(item.style),
                );
            } else {
                paint_box(
                    overlay_screen_rect(item.rect, page),
                    item.kind,
                    stroke(item.style),
                );
            }
        }
        if self.markup.mode == Mode::Select
            && let Some(item) = preview.or_else(|| {
                self.markup.items.iter().find(|a| {
                    a.page_index == page_index && Some(a.object_id) == self.markup.selected
                })
            })
        {
            painter.rect_stroke(
                overlay_screen_rect(item.rect, page).expand(3.),
                0.,
                egui::Stroke::new(1.5, theme::color(theme::ACCENT)),
                egui::StrokeKind::Outside,
            );
            for p in handles(item, page) {
                painter.rect_filled(
                    egui::Rect::from_center_size(p, egui::Vec2::splat(6.)),
                    1.,
                    theme::color(theme::ACCENT),
                );
            }
        }
    }
    pub(super) fn refresh_markup_render(&mut self, ctx: &egui::Context, snapshot: Option<Vec<u8>>) {
        self.markup.preview_pending = true;
        let selected = self.markup.selected.filter(|id| {
            self.markup
                .items
                .iter()
                .any(|item| item.object_id == *id && item.page_index == self.project.selected_page)
        });
        self.cancel_markup_selection();
        self.markup.selected = selected;
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

fn annotation_hit(item: &crate::pdf::ShapeAnnotation, page: egui::Rect, p: egui::Pos2) -> bool {
    if !matches!(
        item.kind,
        crate::pdf::ShapeKind::Line | crate::pdf::ShapeKind::Arrow
    ) {
        return shape_hit(item.kind, overlay_screen_rect(item.rect, page), p);
    }
    if !p.is_finite() || !page.is_finite() {
        return false;
    }
    let Some([x, y, z, t]) = item.endpoints else {
        return false;
    };
    let a = screen_point(egui::pos2(x, y), page);
    let b = screen_point(egui::pos2(z, t), page);
    let hit = |a: egui::Pos2, b: egui::Pos2| {
        let delta = b - a;
        let length = delta.length_sq();
        if !length.is_finite() || length <= 0. {
            return false;
        }
        let t = ((p - a).dot(delta) / length).clamp(0., 1.);
        p.distance(a + delta * t) <= 4.
    };
    hit(a, b)
        || item.line_head.is_some_and(|[x, y, z, t]| {
            hit(screen_point(egui::pos2(x, y), page), b)
                || hit(b, screen_point(egui::pos2(z, t), page))
        })
}

// Hit filled interiors even for unfilled annotations, with a screen-space halo.
fn shape_hit(kind: crate::pdf::ShapeKind, bounds: egui::Rect, p: egui::Pos2) -> bool {
    if !bounds.is_finite() || bounds.width() <= 0. || bounds.height() <= 0. || !p.is_finite() {
        return false;
    }
    match kind {
        crate::pdf::ShapeKind::Rectangle | crate::pdf::ShapeKind::Text => {
            bounds.expand(3.).contains(p)
        }
        crate::pdf::ShapeKind::Line | crate::pdf::ShapeKind::Arrow => false,
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
struct EditGesture {
    draft: Option<EditDraft>,
}
struct EditDraft {
    identity: (u64, usize),
    start: egui::Pos2,
    original: crate::pdf::ShapeAnnotation,
    preview: crate::pdf::ShapeAnnotation,
    handle: Option<usize>,
}
fn handles(a: &crate::pdf::ShapeAnnotation, page: egui::Rect) -> Vec<egui::Pos2> {
    if let Some([x, y, z, t]) = a.endpoints {
        vec![
            screen_point(egui::pos2(x, y), page),
            screen_point(egui::pos2(z, t), page),
        ]
    } else {
        let r = overlay_screen_rect(a.rect, page);
        vec![
            r.left_top(),
            r.right_top(),
            r.right_bottom(),
            r.left_bottom(),
        ]
    }
}
impl EditGesture {
    #[allow(clippy::too_many_arguments)]
    fn update(
        &mut self,
        events: &[egui::Event],
        page: egui::Rect,
        viewport: egui::Rect,
        identity: (u64, usize),
        items: &[crate::pdf::ShapeAnnotation],
        selected: &mut Option<lopdf::ObjectId>,
        owns: impl Fn(egui::Pos2) -> bool,
    ) -> Option<crate::pdf::ShapeAnnotation> {
        if !page.is_finite()
            || page.width() <= 0.
            || page.height() <= 0.
            || self.draft.as_ref().is_some_and(|d| d.identity != identity)
        {
            self.draft = None;
        }
        let normalized = |p: egui::Pos2| {
            egui::pos2(
                (p.x - page.left()) / page.width(),
                (p.y - page.top()) / page.height(),
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
                    self.draft = None;
                    if !pos.is_finite()
                        || !page.contains(*pos)
                        || !viewport.contains(*pos)
                        || !owns(*pos)
                    {
                        continue;
                    }
                    let selected_handle = items
                        .iter()
                        .find(|a| a.page_index == identity.1 && Some(a.object_id) == *selected)
                        .and_then(|a| {
                            handles(a, page)
                                .iter()
                                .position(|p| p.distance(*pos) <= 6.)
                                .map(|h| (a, h))
                        });
                    let item = selected_handle.map(|(a, _)| a).or_else(|| {
                        items
                            .iter()
                            .rev()
                            .find(|a| a.page_index == identity.1 && annotation_hit(a, page, *pos))
                    });
                    *selected = item.map(|a| a.object_id);
                    if let Some(a) = item {
                        self.draft = Some(EditDraft {
                            identity,
                            start: normalized(*pos),
                            original: a.clone(),
                            preview: a.clone(),
                            handle: selected_handle.map(|(_, h)| h),
                        });
                    }
                }
                egui::Event::PointerMoved(pos) => {
                    if !pos.is_finite() || !owns(*pos) {
                        self.draft = None;
                        continue;
                    }
                    if let Some(d) = &mut self.draft {
                        d.move_to(normalized(*pos));
                    }
                }
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    ..
                } => {
                    if let Some(mut d) = self.draft.take()
                        && pos.is_finite()
                        && page.contains(*pos)
                        && viewport.contains(*pos)
                        && owns(*pos)
                    {
                        d.move_to(normalized(*pos));
                        if d.preview != d.original {
                            return Some(d.preview);
                        }
                    }
                }
                egui::Event::PointerGone
                | egui::Event::WindowFocused(false)
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
impl EditDraft {
    fn move_to(&mut self, p: egui::Pos2) {
        let r = self.original.rect;
        self.preview = self.original.clone();
        if p == self.start {
            return;
        }
        if let Some(h) = self.handle {
            let anchor = if let Some(n) = self.original.endpoints {
                egui::pos2(n[h * 2], n[h * 2 + 1])
            } else {
                match h {
                    0 => egui::pos2(r.x, r.y),
                    1 => egui::pos2(r.x + r.width, r.y),
                    2 => egui::pos2(r.x + r.width, r.y + r.height),
                    _ => egui::pos2(r.x, r.y + r.height),
                }
            };
            let p = (anchor + (p - self.start)).clamp(egui::Pos2::ZERO, egui::pos2(1., 1.));
            if let Some(mut endpoints) = self.original.endpoints {
                endpoints[h * 2] = p.x;
                endpoints[h * 2 + 1] = p.y;
                if endpoints[0] == endpoints[2] && endpoints[1] == endpoints[3] {
                    return;
                }
                self.preview.endpoints = Some(endpoints);
                self.preview.rect = PdfRect {
                    x: endpoints[0].min(endpoints[2]),
                    y: endpoints[1].min(endpoints[3]),
                    width: (endpoints[2] - endpoints[0]).abs(),
                    height: (endpoints[3] - endpoints[1]).abs(),
                };
                self.preview.line_head = None; // regenerated from physical page geometry by the PDF backend
            } else {
                let opposite = match h {
                    0 => egui::pos2(r.x + r.width, r.y + r.height),
                    1 => egui::pos2(r.x, r.y + r.height),
                    2 => egui::pos2(r.x, r.y),
                    _ => egui::pos2(r.x + r.width, r.y),
                };
                let bounds = egui::Rect::from_two_pos(p, opposite);
                if bounds.width() <= 0. || bounds.height() <= 0. {
                    return;
                }
                self.preview.rect = PdfRect {
                    x: bounds.left(),
                    y: bounds.top(),
                    width: bounds.width(),
                    height: bounds.height(),
                };
            }
            return;
        }
        let (min, max) = if let Some([x, y, z, t]) = self.original.endpoints {
            (
                egui::pos2(x.min(z), y.min(t)),
                egui::pos2(x.max(z), y.max(t)),
            )
        } else {
            (
                egui::pos2(r.x, r.y),
                egui::pos2(r.x + r.width, r.y + r.height),
            )
        };
        let delta = (p - self.start).clamp(-min.to_vec2(), egui::vec2(1. - max.x, 1. - max.y));
        self.preview = self.original.clone();
        self.preview.rect.x += delta.x;
        self.preview.rect.y += delta.y;
        if let Some([x, y, z, t]) = self.original.endpoints {
            self.preview.endpoints = Some([x + delta.x, y + delta.y, z + delta.x, t + delta.y]);
        }
        if let Some([x, y, z, t]) = self.original.line_head {
            self.preview.line_head = Some([x + delta.x, y + delta.y, z + delta.x, t + delta.y]);
        }
    }
}
fn screen_point(p: egui::Pos2, page: egui::Rect) -> egui::Pos2 {
    page.min + p.to_vec2() * page.size()
}
#[derive(Default)]
struct LineGesture {
    draft: Option<((u64, usize), egui::Pos2, egui::Pos2)>,
    press: Option<((u64, usize), egui::Pos2)>,
}
impl LineGesture {
    fn update(
        &mut self,
        events: &[egui::Event],
        page: egui::Rect,
        viewport: egui::Rect,
        identity: (u64, usize),
        owns: impl Fn(egui::Pos2) -> bool,
    ) -> Option<[f32; 4]> {
        if !page.is_finite() || page.width() <= 0. || page.height() <= 0. {
            *self = Self::default();
            return None;
        }
        if self.draft.is_some_and(|(id, _, _)| id != identity)
            || self.press.is_some_and(|(id, _)| id != identity)
        {
            *self = Self::default();
        }
        let normalized = |p: egui::Pos2| {
            egui::pos2(
                (p.x - page.left()) / page.width(),
                (p.y - page.top()) / page.height(),
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
                    if pos.is_finite()
                        && page.contains(*pos)
                        && viewport.contains(*pos)
                        && owns(*pos)
                    {
                        self.press = Some((identity, *pos));
                    } else {
                        *self = Self::default();
                    }
                }
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    ..
                } => {
                    if self.press.take().is_some()
                        && pos.is_finite()
                        && page.contains(*pos)
                        && viewport.contains(*pos)
                        && owns(*pos)
                    {
                        let p = normalized(*pos);
                        if let Some((_, start, _)) = self.draft.take() {
                            if start != p {
                                return Some([start.x, start.y, p.x, p.y]);
                            }
                        } else {
                            self.draft = Some((identity, p, p));
                        }
                    } else {
                        *self = Self::default();
                    }
                }
                egui::Event::PointerMoved(p) if p.is_finite() => {
                    if let Some((_, _, end)) = &mut self.draft {
                        *end = normalized(*p).clamp(egui::Pos2::ZERO, egui::pos2(1., 1.));
                    }
                }
                egui::Event::PointerGone
                | egui::Event::WindowFocused(false)
                | egui::Event::Key {
                    key: egui::Key::Escape,
                    pressed: true,
                    ..
                } => *self = Self::default(),
                _ => {}
            }
        }
        None
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
                | egui::Event::WindowFocused(false)
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
    fn properties_frame(
        app: &mut GlyphApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let mut out = ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                ui.horizontal(|ui| app.draw_markup_properties(ui, ctx));
            },
        );
        out.textures_delta.clear();
        out
    }
    fn properties_button_pos(app: &mut GlyphApp, ctx: &egui::Context) -> egui::Pos2 {
        let mut pos = egui::Pos2::ZERO;
        for _ in 0..3 {
            let mut out = ctx.run_ui(Default::default(), |ui| {
                ui.horizontal(|ui| {
                    let id = ui
                        .push_id("markup-properties", |ui| {
                            ui.make_persistent_id(("native-icon", icons::Icon::More))
                        })
                        .inner;
                    app.draw_markup_properties(ui, ctx);
                    pos = ctx.read_response(id).unwrap().rect.center();
                });
            });
            out.textures_delta.clear();
        }
        pos
    }
    fn popup_action_pos(out: &egui::FullOutput, label: &str) -> egui::Pos2 {
        out.shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::epaint::Shape::Text(t) if t.galley.job.text == label => {
                    Some(t.pos + t.galley.size() / 2.)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing popup action {label}"))
    }
    #[test]
    fn markup_polish_properties_apply_and_cancel_route_real_ui_and_pdf_worker() {
        use crate::pdf::{ShapeKind, ShapeStyle};
        for kind in [
            ShapeKind::Rectangle,
            ShapeKind::Ellipse,
            ShapeKind::Line,
            ShapeKind::Arrow,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("properties.pdf");
            let mut doc = lopdf::Document::with_version("1.7");
            let pages = doc.new_object_id();
            let page=doc.add_object(lopdf::dictionary! {"Type"=>"Page","Parent"=>pages,"MediaBox"=>vec![0.into(),0.into(),600.into(),800.into()],"Resources"=>lopdf::dictionary! {}});
            doc.objects.insert(
                pages,
                lopdf::dictionary! {"Type"=>"Pages","Kids"=>vec![page.into()],"Count"=>1}.into(),
            );
            let catalog = doc.add_object(lopdf::dictionary! {"Type"=>"Catalog","Pages"=>pages});
            doc.trailer.set("Root", catalog);
            doc.save(&path).unwrap();
            let mut session = crate::pdf::EditablePdf::open(&path).unwrap();
            if matches!(kind, ShapeKind::Line | ShapeKind::Arrow) {
                session
                    .add_line(0, [0.2, 0.2, 0.6, 0.6], kind == ShapeKind::Arrow)
                    .unwrap();
            } else {
                session
                    .add_shape(
                        0,
                        PdfRect {
                            x: 0.2,
                            y: 0.2,
                            width: 0.4,
                            height: 0.4,
                        },
                        kind,
                    )
                    .unwrap();
            }
            session.save().unwrap();
            let ctx = egui::Context::default();
            let mut initial = ctx.run_ui(
                egui::RawInput {
                    max_texture_side: Some(8192),
                    ..Default::default()
                },
                |_| {},
            );
            initial.textures_delta.clear();
            let mut app = GlyphApp::with_context(&ctx, None);
            let summary =
                crate::pdf::PdfEngine::inspect(&crate::pdf::LopdfInspectionEngine, &path).unwrap();
            app.project.open_document(path.clone(), summary);
            app.load_markups(&ctx);
            let settle = |app: &mut GlyphApp| {
                let deadline = Instant::now() + Duration::from_secs(5);
                while (app.edit_pending() || app.markup.preview_pending)
                    && Instant::now() < deadline
                {
                    app.apply_edit_results(&ctx);
                    app.apply_render_results(&ctx);
                    thread::sleep(Duration::from_millis(2));
                }
                assert!(app.can_change_markups(), "{}", app.status);
            };
            settle(&mut app);
            app.markup.mode = Mode::Select;
            let before = app.markup.items[0].clone();
            app.markup.selected = Some(before.object_id);
            let pos = properties_button_pos(&mut app, &ctx);
            properties_frame(&mut app, &ctx, vec![click(pos, true), click(pos, false)]);
            let out = properties_frame(&mut app, &ctx, vec![]);
            let cancel = popup_action_pos(&out, "Cancel");
            let style = ShapeStyle {
                rgb: [0.2, 0.4, 0.8],
                weight: 5.,
            };
            app.markup.properties.as_mut().unwrap().style = style;
            properties_frame(
                &mut app,
                &ctx,
                vec![click(cancel, true), click(cancel, false)],
            );
            assert_eq!(app.markup.items, vec![before.clone()]);
            assert!(!app.edit_pending());
            properties_frame(&mut app, &ctx, vec![]);
            let pos = properties_button_pos(&mut app, &ctx);
            properties_frame(
                &mut app,
                &ctx,
                vec![
                    egui::Event::PointerMoved(pos),
                    click(pos, true),
                    click(pos, false),
                ],
            );
            let out = properties_frame(&mut app, &ctx, vec![]);
            let apply = popup_action_pos(&out, "Apply");
            app.markup.properties.as_mut().unwrap().style = style;
            properties_frame(
                &mut app,
                &ctx,
                vec![click(apply, true), click(apply, false)],
            );
            assert!(
                app.edit_pending(),
                "Apply must dispatch the retained PDF worker"
            );
            settle(&mut app);
            let after = app.markup.items[0].clone();
            assert_eq!(after.style, style);
            assert_eq!(after.object_id, before.object_id);
            // The shared worker path is used for Undo, not an overlay-local copy.
            let mut out = ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: egui::Key::S,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::COMMAND,
                    }],
                    ..Default::default()
                },
                |_| {
                    app.handle_edit_shortcuts(&ctx);
                },
            );
            out.textures_delta.clear();
            settle(&mut app);
            assert_eq!(
                crate::pdf::EditablePdf::open(&path).unwrap().shapes(),
                vec![after]
            );
            let mut out = ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: egui::Key::Z,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::COMMAND,
                    }],
                    ..Default::default()
                },
                |_| {
                    app.handle_edit_shortcuts(&ctx);
                },
            );
            out.textures_delta.clear();
            settle(&mut app);
            assert_eq!(
                app.markup.items,
                vec![before],
                "one Undo reverses properties Apply"
            );
        }
    }
    #[test]
    fn markup_polish_handle_halo_keeps_pointer_offset_without_geometry_jump() {
        let ctx = egui::Context::default();
        let app = app(&ctx);
        let page = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500., 800.));
        for kind in [
            crate::pdf::ShapeKind::Rectangle,
            crate::pdf::ShapeKind::Ellipse,
            crate::pdf::ShapeKind::Line,
            crate::pdf::ShapeKind::Arrow,
        ] {
            let mut shape = app.markup.items[0].clone();
            shape.kind = kind;
            if matches!(
                kind,
                crate::pdf::ShapeKind::Line | crate::pdf::ShapeKind::Arrow
            ) {
                shape.endpoints = Some([0.1, 0.1, 0.3, 0.3]);
            }
            let start = handles(&shape, page)[0] + egui::vec2(2., 2.);
            let end = start + egui::vec2(10., 20.);
            let mut g = EditGesture::default();
            let mut selected = Some(shape.object_id);
            let result = g
                .update(
                    &[click(start, true), click(end, false)],
                    page,
                    page,
                    (1, 0),
                    &[shape],
                    &mut selected,
                    |_| true,
                )
                .unwrap();
            let (x, y) = result
                .endpoints
                .map_or((result.rect.x, result.rect.y), |n| (n[0], n[1]));
            assert!(
                (x - 0.12).abs() < 0.00001 && (y - 0.125).abs() < 0.00001,
                "halo press must retain endpoint/corner offset: {kind:?} {x} {y}"
            );
        }
    }
    #[test]
    fn markup_polish_picker_handoff_resets_all_drafts_only_on_success() {
        let ctx = egui::Context::default();
        for blocked in [true, false] {
            let mut app = app(&ctx);
            let shape = app.markup.items[0].clone();
            let identity = (app.document_generation, app.project.selected_page);
            let start = egui::pos2(0.2, 0.2);
            let end = egui::pos2(0.4, 0.4);
            app.markup.selected = Some(shape.object_id);
            app.markup.mode = Mode::Select;
            app.markup.properties = Some(PropertiesDraft {
                identity,
                selected: app.markup.selected,
                style: shape.style,
            });
            app.markup.edit_gesture.draft = Some(EditDraft {
                identity,
                start,
                original: shape.clone(),
                preview: shape.clone(),
                handle: Some(0),
            });
            app.markup.gesture.draft = Some((identity, start, end));
            app.markup.line_gesture.draft = Some((identity, start, end));
            app.markup.line_gesture.press = Some((identity, start));
            app.markup.preview_pending = blocked;
            app.open_sheet_picker();
            assert_eq!(app.sheet_picker.is_none(), blocked);
            assert_eq!(app.markup.edit_gesture.draft.is_some(), blocked);
            assert_eq!(app.markup.gesture.draft.is_some(), blocked);
            assert_eq!(app.markup.line_gesture.draft.is_some(), blocked);
            assert_eq!(app.markup.line_gesture.press.is_some(), blocked);
            assert_eq!(app.markup.properties.is_some(), blocked);
            assert_eq!(app.markup.selected, Some(shape.object_id));
            assert!(app.markup.mode == Mode::Select);
        }
    }

    #[test]
    fn markup_polish_gate_and_navigation_cancel_edit_drafts() {
        let ctx = egui::Context::default();
        for restriction in 0..3 {
            let mut app = app(&ctx);
            let shape = app.markup.items[0].clone();
            app.markup.edit_gesture.draft = Some(EditDraft {
                identity: (app.document_generation, 0),
                start: egui::pos2(0.2, 0.2),
                original: shape.clone(),
                preview: shape,
                handle: None,
            });
            if restriction == 0 {
                app.cancel_markup_selection();
            } else {
                app.markup.preview_pending = true;
                let mut out = ctx.run_ui(Default::default(), |ui| {
                    let (page, response) = ui
                        .allocate_exact_size(egui::vec2(300., 400.), egui::Sense::click_and_drag());
                    if restriction == 2 {
                        // The modal/worker gate must cancel even with no canvas.
                        app.draw_markup_tools(ui, &ctx);
                    } else {
                        app.interact_with_markup(ui, &response, page, page, 0);
                    }
                });
                out.textures_delta.clear();
            }
            assert!(
                app.markup.edit_gesture.draft.is_none(),
                "blocked/navigation draft must not resume later"
            );
        }
        let page = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300., 400.));
        let mut g = BoundingBoxGesture::default();
        g.update(
            &[click(egui::pos2(60., 80.), true)],
            page,
            page,
            (1, 0),
            |_| true,
        );
        assert!(
            g.update(
                &[
                    egui::Event::WindowFocused(false),
                    click(egui::pos2(120., 160.), false)
                ],
                page,
                page,
                (1, 0),
                |_| true
            )
            .is_none(),
            "blur without PointerGone must cancel box drafts"
        );
    }
    #[test]
    fn markup_polish_edit_preview_paints_physical_stroke_and_handles() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        let page = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300., 400.));
        app.markup.page_points = Some((0, egui::vec2(600., 800.)));
        let mut shape = app.markup.items[0].clone();
        shape.style = crate::pdf::ShapeStyle {
            rgb: [0.2, 0.4, 0.8],
            weight: 6.,
        };
        app.markup.edit_gesture.draft = Some(EditDraft {
            identity: (app.document_generation, 0),
            start: egui::pos2(0.2, 0.2),
            original: shape.clone(),
            preview: shape,
            handle: None,
        });
        let mut out = ctx.run_ui(Default::default(), |ui| {
            app.paint_markup(ui.painter(), page, 0)
        });
        out.textures_delta.clear();
        let painted=out.shapes.iter().any(|s| matches!(&s.shape,egui::epaint::Shape::Rect(r) if r.stroke.width == 3. && r.stroke.color == egui::Color32::from_rgb(51,102,204)));
        assert!(
            painted,
            "6 PDF points must preview at 3 screen points at half scale"
        );
    }
    #[test]
    fn markup_polish_properties_popover_applies_next_style_only_on_apply() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.markup.selected = None;
        let mut pos = egui::Pos2::ZERO;
        let mut found = false;
        for _ in 0..3 {
            let mut out = ctx.run_ui(Default::default(), |ui| {
                ui.horizontal(|ui| {
                    let id = ui
                        .push_id("markup-properties", |ui| {
                            ui.make_persistent_id(("native-icon", super::super::icons::Icon::More))
                        })
                        .inner;
                    app.draw_markup_tools(ui, &ctx);
                    if let Some(r) = ctx.read_response(id) {
                        found = true;
                        pos = r.rect.center();
                        assert_eq!(r.rect.size(), egui::Vec2::splat(24.));
                    }
                });
            });
            out.textures_delta.clear();
        }
        assert!(found, "compact Markup properties control must exist");
        let mut out = ctx.run_ui(
            egui::RawInput {
                events: vec![click(pos, true), click(pos, false)],
                ..Default::default()
            },
            |ui| {
                ui.horizontal(|ui| app.draw_markup_tools(ui, &ctx));
            },
        );
        out.textures_delta.clear();
        assert!(
            egui::Popup::is_any_open(&ctx),
            "properties must open as a popover"
        );
        assert_eq!(app.markup.next_style, crate::pdf::ShapeStyle::default());
        assert!(!app.edit_pending());
        app.document_generation += 1;
        let mut out = ctx.run_ui(Default::default(), |ui| {
            ui.horizontal(|ui| app.draw_markup_tools(ui, &ctx));
        });
        out.textures_delta.clear();
        assert!(
            !egui::Popup::is_any_open(&ctx),
            "stale properties popover must close on document generation change"
        );
    }
    #[test]
    fn markup_toolbar_is_seven_compact_targets_without_persistent_style_label() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        let mut bounds = egui::Rect::NOTHING;
        let mut output = ctx.run_ui(Default::default(), |ui| {
            bounds = ui
                .horizontal(|ui| app.draw_markup_tools(ui, &ctx))
                .response
                .rect;
        });
        output.textures_delta.clear();
        assert_eq!(
            bounds.height(),
            24.,
            "markup tools must use 24-point targets"
        );
        assert!(
            bounds.width() > 216. && bounds.width() <= 248.,
            "eight icons including properties replace text chips and the style label: {bounds:?}"
        );
    }

    fn click(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }
    }
    #[test]
    fn text_tool_shortcut_is_native_and_compact() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.markup.mode = Mode::View;
        let mut out = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::T,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }],
                ..Default::default()
            },
            |_| app.markup_shortcuts(&ctx),
        );
        out.textures_delta.clear();
        assert!(
            app.markup.mode != Mode::View,
            "T must activate the text tool"
        );
    }
    #[test]
    fn text_click_places_an_owned_inline_draft_without_dirtying_pdf() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.markup.mode = Mode::Text;
        let page = egui::Rect::from_min_size(egui::pos2(100., 100.), egui::vec2(500., 800.));
        select_at(&mut app, &ctx, page, 0, egui::pos2(200., 200.));
        assert!(
            app.markup.text_draft.is_some(),
            "click must place an inline text editor"
        );
        assert!(!app.edit_pending());
        assert!(!app.editing.dirty);
    }
    #[test]
    fn text_inline_apply_dispatches_one_transaction() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.markup.mode = Mode::Text;
        let page = egui::Rect::from_min_size(egui::pos2(100., 100.), egui::vec2(500., 800.));
        select_at(&mut app, &ctx, page, 0, egui::pos2(200., 200.));
        app.markup.text_draft.as_mut().unwrap().text.contents = "Native text".into();
        assert!(
            app.finish_inline_text(&ctx),
            "Apply must finish the text edit"
        );
        assert!(
            app.edit_pending(),
            "Apply uses the shared transaction worker"
        );
        assert!(app.markup.text_draft.is_none());
    }
    #[test]
    fn text_inline_editor_requests_real_text_focus_and_escape_cancels() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.markup.mode = Mode::Text;
        let page = egui::Rect::from_min_size(egui::pos2(100., 100.), egui::vec2(500., 800.));
        select_at(&mut app, &ctx, page, 0, egui::pos2(200., 200.));
        let mut out = ctx.run_ui(Default::default(), |ui| {
            let response = ui.interact(page, egui::Id::new("markup-page"), egui::Sense::click());
            app.interact_with_markup(ui, &response, page, page, 0);
        });
        out.textures_delta.clear();
        assert_eq!(
            ctx.memory(|m| m.focused()),
            Some(egui::Id::new("inline-markup-text")),
            "on-page editor must own keyboard input"
        );
        let mut out = ctx.run_ui(
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
            |_| app.handle_shortcuts(&ctx),
        );
        out.textures_delta.clear();
        assert!(app.markup.text_draft.is_none());
        assert!(!app.edit_pending());
    }
    #[test]
    fn text_inline_large_paste_is_retained_and_explicitly_rejected_not_truncated() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.markup.mode = Mode::Text;
        let page = egui::Rect::from_min_size(egui::pos2(100., 100.), egui::vec2(500., 800.));
        select_at(&mut app, &ctx, page, 0, egui::pos2(200., 200.));
        for events in [vec![], vec![egui::Event::Text("X".repeat(10001))]] {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    let response =
                        ui.interact(page, egui::Id::new("markup-page"), egui::Sense::click());
                    app.interact_with_markup(ui, &response, page, page, 0);
                },
            );
            out.textures_delta.clear();
        }
        assert_eq!(
            app.markup.text_draft.as_ref().unwrap().text.contents.len(),
            10001,
            "large paste must not silently lose characters"
        );
        assert!(!app.finish_inline_text(&ctx));
        assert!(app.markup.text_draft.as_ref().unwrap().error.is_some());
        assert!(!app.edit_pending());
    }
    #[test]
    fn line_arrow_shortcuts_choose_native_tools() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        for (key, mode) in [(egui::Key::L, Mode::Line), (egui::Key::A, Mode::Arrow)] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Default::default(),
                    }],
                    ..Default::default()
                },
                |_| app.markup_shortcuts(&ctx),
            );
            output.textures_delta.clear();
            assert!(
                app.markup.mode == mode,
                "L/A must select two-click native tools"
            );
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
            text: None,
            object_id: (9, 0),
            kind: crate::pdf::ShapeKind::Rectangle,
            style: crate::pdf::ShapeStyle::default(),
            endpoints: None,
            line_head: None,
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
    #[test]
    fn markup_polish_numeric_property_click_retains_popup_owner() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        let button = properties_button_pos(&mut app, &ctx);
        properties_frame(
            &mut app,
            &ctx,
            vec![click(button, true), click(button, false)],
        );
        let output = properties_frame(&mut app, &ctx, vec![]);
        let red_value = popup_action_pos(&output, "1.00");
        properties_frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(red_value), click(red_value, true)],
        );
        properties_frame(&mut app, &ctx, vec![click(red_value, false)]);
        assert!(
            egui::Popup::is_any_open(&ctx),
            "clicking a numeric field must not dismiss the properties form"
        );
        let output = properties_frame(&mut app, &ctx, vec![]);
        assert!(popup_action_pos(&output, "Green").is_finite());
        assert_eq!(app.markup.selected, Some((9, 0)));
        assert!(!app.edit_pending());
    }

    #[test]
    fn markup_polish_refresh_retains_only_valid_same_page_selection() {
        for (selected, page, expected) in [
            (Some((9, 0)), 0, Some((9, 0))),
            (Some((999, 0)), 0, None),
            (Some((9, 0)), 1, None),
            (None, 0, None),
        ] {
            let ctx = egui::Context::default();
            let mut app = app(&ctx);
            app.markup.selected = selected;
            app.project.selected_page = page;
            app.refresh_markup_render(&ctx, None);
            assert_eq!(app.markup.selected, expected);
            assert!(app.markup.edit_gesture.draft.is_none());
            assert!(app.markup.gesture.draft.is_none());
            assert!(app.markup.line_gesture.draft.is_none());
        }
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
    fn line_arrow_two_native_clicks_dispatch_only_after_end() {
        for mode in [Mode::Line, Mode::Arrow] {
            let ctx = egui::Context::default();
            let mut app = app(&ctx);
            app.markup.mode = mode;
            let page = egui::Rect::from_min_size(egui::pos2(100., 100.), egui::vec2(500., 800.));
            select_at(&mut app, &ctx, page, 0, egui::pos2(200., 200.));
            assert!(!app.edit_pending(), "first click is only a draft");
            select_at(&mut app, &ctx, page, 0, egui::pos2(400., 500.));
            assert!(
                app.edit_pending(),
                "second click must dispatch the shared edit worker"
            );
        }
    }
    #[test]
    fn line_arrow_cancel_and_generation_guard() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.markup.mode = Mode::Line;
        let page = egui::Rect::from_min_size(egui::pos2(100., 100.), egui::vec2(500., 800.));
        select_at(&mut app, &ctx, page, 0, egui::pos2(200., 200.));
        assert!(app.markup.line_gesture.draft.is_some());
        app.cancel_markup_selection();
        assert!(
            app.markup.line_gesture.draft.is_none(),
            "page/tool cancellation must discard a line start"
        );
        select_at(&mut app, &ctx, page, 0, egui::pos2(200., 200.));
        app.document_generation += 1;
        select_at(&mut app, &ctx, page, 0, egui::pos2(400., 500.));
        assert!(
            !app.edit_pending(),
            "generation change cannot finish an old line"
        );
    }
    #[test]
    fn line_arrow_select_visible_segment_not_empty_bbox() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.markup.items[0].kind = crate::pdf::ShapeKind::Line;
        app.markup.items[0].endpoints = Some([0.1, 0.1, 0.3, 0.3]);
        let page = egui::Rect::from_min_size(egui::pos2(100., 100.), egui::vec2(500., 800.));
        app.markup.selected = None;
        select_at(&mut app, &ctx, page, 0, egui::pos2(200., 260.));
        assert_eq!(
            app.markup.selected,
            Some((9, 0)),
            "visible line midpoint must select"
        );
        select_at(&mut app, &ctx, page, 0, egui::pos2(150., 340.));
        assert_eq!(
            app.markup.selected, None,
            "empty bbox corner must not select"
        );
    }
    #[test]
    fn line_arrow_toolbar_contains_two_additional_compact_targets() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        let mut bounds = egui::Rect::NOTHING;
        let mut output = ctx.run_ui(Default::default(), |ui| {
            bounds = ui
                .horizontal(|ui| app.draw_markup_tools(ui, &ctx))
                .response
                .rect;
        });
        output.textures_delta.clear();
        assert!(
            bounds.width() > 216. && bounds.width() <= 248.,
            "eight compact tools including properties: {bounds:?}"
        );
        assert_eq!(bounds.height(), 24.);
    }
    #[test]
    fn line_arrow_gesture_batched_clicks_escape_page_layer_and_coincident() {
        let page = egui::Rect::from_min_size(egui::pos2(100., 100.), egui::vec2(500., 800.));
        let a = egui::pos2(200., 200.);
        let b = egui::pos2(400., 500.);
        let mut g = LineGesture::default();
        assert_eq!(
            g.update(
                &[
                    click(a, true),
                    click(a, false),
                    click(b, true),
                    click(b, false)
                ],
                page,
                page,
                (1, 0),
                |_| true
            ),
            Some([0.2, 0.125, 0.6, 0.5])
        );
        assert!(g.draft.is_none());
        for cancellation in [
            egui::Event::PointerGone,
            egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            },
        ] {
            g.update(
                &[click(a, true), click(a, false)],
                page,
                page,
                (1, 0),
                |_| true,
            );
            g.update(&[cancellation], page, page, (1, 0), |_| true);
            assert!(g.draft.is_none());
        }
        g.update(
            &[click(a, true), click(a, false)],
            page,
            page,
            (1, 0),
            |_| true,
        );
        assert!(
            g.update(
                &[click(b, true), click(b, false)],
                page,
                page,
                (1, 1),
                |_| true
            )
            .is_none(),
            "physical page guard"
        );
        assert!(
            g.update(
                &[click(a, true), click(a, false)],
                page,
                page,
                (1, 1),
                |_| false
            )
            .is_none()
        );
        assert!(g.draft.is_none());
        assert!(
            g.update(
                &[
                    click(a, true),
                    click(a, false),
                    click(a, true),
                    click(a, false)
                ],
                page,
                page,
                (1, 1),
                |_| true
            )
            .is_none(),
            "coincident points rejected"
        );
        assert!(g.draft.is_none());
    }
    #[test]
    fn line_arrow_selection_hits_arrow_head_with_fixed_screen_distance() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        let item = &mut app.markup.items[0];
        item.kind = crate::pdf::ShapeKind::Arrow;
        item.endpoints = Some([0.1, 0.5, 0.9, 0.5]);
        item.line_head = Some([0.7, 0.4, 0.7, 0.6]);
        let page = egui::Rect::from_min_size(egui::pos2(100., 100.), egui::vec2(500., 800.));
        for (p, hit) in [
            (egui::pos2(500., 460.), true),
            (egui::pos2(300., 503.), true),
            (egui::pos2(300., 505.), false),
            (egui::pos2(150., 420.), false),
        ] {
            select_at(&mut app, &ctx, page, 0, p);
            assert_eq!(app.markup.selected, hit.then_some((9, 0)));
        }
        select_at_with_cover(&mut app, &ctx, page, 0, egui::pos2(300., 500.), true);
        assert!(app.markup.selected.is_none());
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
        // No selection: this assertion exercises shape picking, not the newly
        // visible selected ellipse's corner resize handle.
        app.markup.selected = None;
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
