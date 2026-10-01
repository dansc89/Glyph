#![allow(dead_code)]

mod app;
mod core;
mod pdf;
mod theme;

use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    let initial_pdf = std::env::args_os().nth(1).map(PathBuf::from);
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
        Box::new(move |cc| Ok(Box::new(app::GlyphApp::new(cc, initial_pdf.clone())))),
    )
}
