use eframe::egui;

pub const SURFACE: egui::Color32 = egui::Color32::from_rgb(15, 17, 21);
pub const PANEL: egui::Color32 = egui::Color32::from_rgb(24, 27, 33);
pub const PANEL_RAISED: egui::Color32 = egui::Color32::from_rgb(31, 35, 43);
pub const CONTROL: egui::Color32 = egui::Color32::from_rgb(38, 43, 52);
pub const CONTROL_HOVER: egui::Color32 = egui::Color32::from_rgb(50, 57, 68);
pub const CANVAS: egui::Color32 = egui::Color32::from_rgb(12, 14, 18);
pub const TEXT: egui::Color32 = egui::Color32::from_rgb(232, 235, 240);
pub const TEXT_MUTED: egui::Color32 = egui::Color32::from_rgb(148, 156, 170);
pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(129, 161, 193);
pub const ACCENT_STRONG: egui::Color32 = egui::Color32::from_rgb(153, 188, 222);
pub const STROKE: egui::Color32 = egui::Color32::from_rgb(54, 60, 72);

pub fn install(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = PANEL;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = SURFACE;
    visuals.widgets.active.bg_fill = ACCENT;
    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(48, 54, 64);
    visuals.selection.bg_fill = ACCENT;
    visuals.override_text_color = Some(TEXT);
    ctx.set_visuals(visuals);
}
