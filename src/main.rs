#![allow(dead_code)]

mod app;
mod core;
mod pdf;
mod theme;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Glyph")
            .with_inner_size([1440.0, 920.0])
            .with_min_inner_size([960.0, 640.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Glyph",
        options,
        Box::new(|cc| Ok(Box::new(app::GlyphApp::new(cc)))),
    )
}
