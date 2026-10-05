#![allow(dead_code)]

mod app;
mod core;
mod pdf;
mod theme;

use std::path::PathBuf;

fn native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Glyph")
            .with_app_id("glyph")
            .with_inner_size([1440.0, 920.0])
            .with_min_inner_size([960.0, 640.0]),
        ..Default::default()
    }
}

fn main() -> eframe::Result<()> {
    let initial_pdf = std::env::args_os().nth(1).map(PathBuf::from);
    let options = native_options();

    eframe::run_native(
        "Glyph",
        options,
        Box::new(move |cc| Ok(Box::new(app::GlyphApp::new(cc, initial_pdf.clone())))),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn wayland_app_id_matches_desktop_entry() {
        assert_eq!(
            super::native_options().viewport.app_id.as_deref(),
            Some("glyph")
        );
    }
}
