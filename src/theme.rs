use eframe::egui;

pub const SURFACE: egui::Color32 = egui::Color32::from_rgb(7, 9, 12);
pub const PANEL: egui::Color32 = egui::Color32::from_rgb(12, 15, 20);
pub const PANEL_RAISED: egui::Color32 = egui::Color32::from_rgb(18, 22, 29);
pub const CONTROL: egui::Color32 = egui::Color32::from_rgb(23, 28, 37);
pub const CONTROL_HOVER: egui::Color32 = egui::Color32::from_rgb(35, 42, 54);
pub const CANVAS: egui::Color32 = egui::Color32::from_rgb(9, 12, 16);
pub const CARD: egui::Color32 = egui::Color32::from_rgb(15, 19, 26);
pub const TEXT: egui::Color32 = egui::Color32::from_rgb(235, 238, 243);
pub const TEXT_MUTED: egui::Color32 = egui::Color32::from_rgb(154, 163, 177);
pub const TEXT_FAINT: egui::Color32 = egui::Color32::from_rgb(88, 97, 113);
pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(245, 190, 80);
pub const ACCENT_STRONG: egui::Color32 = egui::Color32::from_rgb(255, 215, 118);
pub const ACCENT_SOFT: egui::Color32 = egui::Color32::from_rgb(58, 42, 18);
pub const CYAN: egui::Color32 = egui::Color32::from_rgb(92, 219, 255);
pub const CYAN_SOFT: egui::Color32 = egui::Color32::from_rgb(16, 47, 57);
pub const STROKE: egui::Color32 = egui::Color32::from_rgb(37, 44, 57);
pub const STROKE_STRONG: egui::Color32 = egui::Color32::from_rgb(79, 88, 104);
pub const GREEN: egui::Color32 = egui::Color32::from_rgb(110, 231, 183);
pub const GOLD: egui::Color32 = ACCENT;

pub fn install(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = PANEL;
    visuals.window_fill = PANEL;
    visuals.faint_bg_color = SURFACE;
    visuals.extreme_bg_color = SURFACE;
    visuals.widgets.active.bg_fill = ACCENT;
    visuals.widgets.active.fg_stroke.color = SURFACE;
    visuals.widgets.hovered.bg_fill = CONTROL_HOVER;
    visuals.widgets.hovered.fg_stroke.color = TEXT;
    visuals.widgets.inactive.bg_fill = CONTROL;
    visuals.widgets.noninteractive.bg_fill = PANEL;
    visuals.widgets.inactive.fg_stroke.color = TEXT;
    visuals.window_stroke.color = STROKE;
    visuals.window_corner_radius = egui::CornerRadius::same(4);
    visuals.menu_corner_radius = egui::CornerRadius::same(4);
    visuals.selection.bg_fill = ACCENT;
    visuals.selection.stroke.color = SURFACE;
    visuals.override_text_color = Some(TEXT);

    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.window_margin = egui::Margin::same(8);
    style
        .text_styles
        .insert(egui::TextStyle::Heading, egui::FontId::proportional(19.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, egui::FontId::monospace(12.5));
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(13.0));
    ctx.set_style_of(egui::Theme::Dark, style);
}

pub fn translucent(color: egui::Color32, alpha: u8) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}
