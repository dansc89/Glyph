use eframe::egui;

pub const SURFACE: egui::Color32 = egui::Color32::from_rgb(14, 13, 21);
pub const PANEL: egui::Color32 = egui::Color32::from_rgb(23, 21, 32);
pub const PANEL_RAISED: egui::Color32 = egui::Color32::from_rgb(32, 29, 44);
pub const CONTROL: egui::Color32 = egui::Color32::from_rgb(42, 38, 56);
pub const CONTROL_HOVER: egui::Color32 = egui::Color32::from_rgb(58, 51, 77);
pub const CANVAS: egui::Color32 = egui::Color32::from_rgb(11, 10, 17);
pub const CARD: egui::Color32 = egui::Color32::from_rgb(25, 23, 36);
pub const TEXT: egui::Color32 = egui::Color32::from_rgb(235, 237, 241);
pub const TEXT_MUTED: egui::Color32 = egui::Color32::from_rgb(172, 165, 190);
pub const TEXT_FAINT: egui::Color32 = egui::Color32::from_rgb(104, 96, 124);
pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(139, 92, 246);
pub const ACCENT_STRONG: egui::Color32 = egui::Color32::from_rgb(196, 181, 253);
pub const ACCENT_SOFT: egui::Color32 = egui::Color32::from_rgb(79, 70, 117);
pub const STROKE: egui::Color32 = egui::Color32::from_rgb(54, 48, 72);
pub const STROKE_STRONG: egui::Color32 = egui::Color32::from_rgb(95, 82, 128);
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
