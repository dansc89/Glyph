use super::*;

#[test]
fn modern_omarchy_named_palette_accepts_mode_and_named_channels() {
    let source = "mode = \"dark\"\nbackground = \"#222222\"\nforeground = \"#c2c2b0\"\naccent = \"#78824b\"\ncyan = \"#c9a554\"\ngreen = \"#5f875f\"\n";
    let palette = Palette::parse(source).expect("current Omarchy palette schema");
    assert_eq!(palette.cyan, egui::Color32::from_rgb(201, 165, 84));
    assert_eq!(palette.green, egui::Color32::from_rgb(95, 135, 95));
}

#[test]
fn modern_active_palette_defaults_to_state_directory() {
    assert_eq!(
        theme_path(None, Some("/home/test".into())),
        Some(std::path::PathBuf::from(
            "/home/test/.local/state/omarchy/current/theme/colors.toml"
        ))
    );
}

#[test]
fn installed_omarchy_palette_is_readable_when_present() {
    let path = theme_path(std::env::var_os("XDG_STATE_HOME"), std::env::var_os("HOME"));
    if let Some(path) = path.filter(|path| path.is_file()) {
        assert!(
            load_palette(&path).is_some(),
            "installed theme must not silently fall back: {}",
            path.display()
        );
    }
}
