use eframe::egui;

pub const SURFACE: egui::Color32 = egui::Color32::from_rgb(24, 24, 26);
pub const PANEL: egui::Color32 = egui::Color32::from_rgb(29, 30, 32);
pub const PANEL_RAISED: egui::Color32 = egui::Color32::from_rgb(37, 38, 41);
pub const CONTROL: egui::Color32 = egui::Color32::from_rgb(47, 48, 52);
pub const CONTROL_HOVER: egui::Color32 = egui::Color32::from_rgb(61, 63, 68);
pub const CANVAS: egui::Color32 = egui::Color32::from_rgb(31, 31, 34);
pub const CARD: egui::Color32 = egui::Color32::from_rgb(33, 34, 37);
pub const TEXT: egui::Color32 = egui::Color32::from_rgb(236, 237, 239);
pub const TEXT_MUTED: egui::Color32 = egui::Color32::from_rgb(166, 170, 176);
pub const TEXT_FAINT: egui::Color32 = egui::Color32::from_rgb(111, 116, 124);
pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(76, 132, 196);
pub const ACCENT_STRONG: egui::Color32 = egui::Color32::from_rgb(110, 160, 220);
pub const ACCENT_SOFT: egui::Color32 = egui::Color32::from_rgb(48, 68, 92);
pub const STROKE: egui::Color32 = egui::Color32::from_rgb(57, 59, 64);
pub const STROKE_STRONG: egui::Color32 = egui::Color32::from_rgb(77, 81, 88);
pub const GREEN: egui::Color32 = egui::Color32::from_rgb(110, 231, 183);
pub const GOLD: egui::Color32 = egui::Color32::from_rgb(251, 191, 36);

pub fn install(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = PANEL;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = SURFACE;
    visuals.widgets.active.bg_fill = ACCENT;
    visuals.widgets.hovered.bg_fill = CONTROL_HOVER;
    visuals.widgets.inactive.bg_fill = CONTROL;
    visuals.widgets.noninteractive.bg_fill = PANEL;
    visuals.widgets.inactive.fg_stroke.color = TEXT;
    visuals.widgets.hovered.fg_stroke.color = TEXT;
    visuals.window_stroke.color = STROKE;
    visuals.window_corner_radius = egui::CornerRadius::same(8);
    visuals.selection.bg_fill = ACCENT;
    visuals.override_text_color = Some(TEXT);
    ctx.set_visuals(visuals);
}
