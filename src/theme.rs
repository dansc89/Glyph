use eframe::egui;

pub const SURFACE: egui::Color32 = egui::Color32::from_rgb(18, 20, 24);
const PANEL: egui::Color32 = egui::Color32::from_rgb(24, 27, 33);
const TEXT: egui::Color32 = egui::Color32::from_rgb(230, 232, 236);
const ACCENT: egui::Color32 = egui::Color32::from_rgb(129, 161, 193);

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
