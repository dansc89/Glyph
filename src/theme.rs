use eframe::egui;

pub const SURFACE: egui::Color32 = egui::Color32::from_rgb(12, 13, 16);
pub const PANEL: egui::Color32 = egui::Color32::from_rgb(20, 22, 27);
pub const PANEL_RAISED: egui::Color32 = egui::Color32::from_rgb(27, 30, 36);
pub const CONTROL: egui::Color32 = egui::Color32::from_rgb(35, 39, 47);
pub const CONTROL_HOVER: egui::Color32 = egui::Color32::from_rgb(48, 54, 65);
pub const CANVAS: egui::Color32 = egui::Color32::from_rgb(9, 10, 13);
pub const TEXT: egui::Color32 = egui::Color32::from_rgb(235, 237, 241);
pub const TEXT_MUTED: egui::Color32 = egui::Color32::from_rgb(138, 146, 158);
pub const TEXT_FAINT: egui::Color32 = egui::Color32::from_rgb(93, 101, 113);
pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(106, 141, 178);
pub const ACCENT_STRONG: egui::Color32 = egui::Color32::from_rgb(143, 180, 215);
pub const STROKE: egui::Color32 = egui::Color32::from_rgb(45, 50, 60);
pub const STROKE_STRONG: egui::Color32 = egui::Color32::from_rgb(67, 75, 88);
pub const GREEN: egui::Color32 = egui::Color32::from_rgb(104, 171, 122);

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
