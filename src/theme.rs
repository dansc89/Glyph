use eframe::egui;

#[cfg(test)]
mod modern_tests;

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

#[derive(Clone, Debug, PartialEq)]
struct Palette {
    background: egui::Color32,
    foreground: egui::Color32,
    accent: egui::Color32,
    cyan: egui::Color32,
    green: egui::Color32,
}

impl Palette {
    fn parse(source: &str) -> Option<Self> {
        let mut colors = std::collections::HashMap::new();
        let mut mode_seen = false;
        for line in source.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line.split_once('=')?;
            let key = key.trim();
            if key.is_empty()
                || !key
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            {
                return None;
            }
            let value = value.trim();
            let quote = value.chars().next()?;
            if quote != '\'' && quote != '"' {
                return None;
            }
            let end = value[1..].find(quote)? + 1;
            let quoted = &value[1..end];
            let trailing = value[end + 1..].trim();
            if !trailing.is_empty() && !trailing.starts_with('#') {
                return None;
            }
            if key == "mode" {
                if mode_seen || !matches!(quoted, "dark" | "light") {
                    return None;
                }
                mode_seen = true;
                continue;
            }
            let hex = quoted.strip_prefix('#')?;
            if hex.len() != 6 || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
                return None;
            }
            let rgb = u32::from_str_radix(hex, 16).ok()?;
            let color = egui::Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
            if colors.insert(key, color).is_some() {
                return None;
            }
        }
        Some(Self {
            background: *colors.get("background")?,
            foreground: *colors.get("foreground")?,
            accent: *colors.get("accent")?,
            cyan: colors
                .get("cyan")
                .or_else(|| colors.get("color6"))
                .copied()
                .unwrap_or(*colors.get("accent")?),
            green: colors
                .get("green")
                .or_else(|| colors.get("color2"))
                .copied()
                .unwrap_or(*colors.get("accent")?),
        })
    }
}

thread_local! {
    static PALETTE: std::cell::RefCell<Option<Palette>> = const { std::cell::RefCell::new(None) };
    static WATCH: std::cell::RefCell<Option<(std::path::PathBuf, std::time::Instant)>> = const { std::cell::RefCell::new(None) };
}

/// Resolve Glyph's semantic colors on the UI thread; unknown colors pass through.
pub fn color(default: egui::Color32) -> egui::Color32 {
    PALETTE.with(|palette| {
        let palette = palette.borrow();
        let Some(p) = palette.as_ref() else {
            return default;
        };
        let mix = |a: egui::Color32, b: egui::Color32, amount: u16| {
            let channel = |x: u8, y: u8| {
                ((u16::from(x) * (100 - amount) + u16::from(y) * amount) / 100) as u8
            };
            egui::Color32::from_rgb(
                channel(a.r(), b.r()),
                channel(a.g(), b.g()),
                channel(a.b(), b.b()),
            )
        };
        match default {
            SURFACE | CANVAS => p.background,
            PANEL => mix(p.background, p.foreground, 3),
            CARD => mix(p.background, p.foreground, 5),
            PANEL_RAISED => mix(p.background, p.foreground, 6),
            CONTROL => mix(p.background, p.foreground, 9),
            CONTROL_HOVER => mix(p.background, p.foreground, 15),
            TEXT => p.foreground,
            TEXT_MUTED => mix(p.background, p.foreground, 65),
            TEXT_FAINT => mix(p.background, p.foreground, 45),
            ACCENT => p.accent, // GOLD is the same color constant.
            ACCENT_STRONG => mix(p.accent, p.foreground, 20),
            ACCENT_SOFT => mix(p.background, p.accent, 20),
            CYAN => p.cyan,
            CYAN_SOFT => mix(p.background, p.cyan, 20),
            STROKE => mix(p.background, p.foreground, 15),
            STROKE_STRONG => mix(p.background, p.foreground, 35),
            GREEN => p.green,
            _ => default,
        }
    })
}

fn theme_path(
    xdg: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Option<std::path::PathBuf> {
    let root = xdg
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            home.map(std::path::PathBuf::from)
                .filter(|p| p.is_absolute())
                .map(|p| p.join(".local/state"))
        })?;
    Some(root.join("omarchy/current/theme/colors.toml"))
}

const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

fn load_palette(path: &std::path::Path) -> Option<Palette> {
    use std::io::Read;
    const MAX_BYTES: u64 = 64 * 1024;
    let file = std::fs::File::open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut source = String::new();
    file.take(MAX_BYTES + 1).read_to_string(&mut source).ok()?;
    if source.len() as u64 > MAX_BYTES {
        return None;
    }
    Palette::parse(&source)
}

fn refresh_at(ctx: &egui::Context, now: std::time::Instant) -> bool {
    // The next visible draw catches up; do not wake a minimized window for polling.
    if ctx.input(|input| input.viewport().minimized == Some(true)) {
        return false;
    }
    let (path, remaining) = WATCH.with(|watch| {
        let mut watch = watch.borrow_mut();
        let Some((path, last_check)) = watch.as_mut() else {
            return (None, None);
        };
        let elapsed = now.saturating_duration_since(*last_check);
        if elapsed < POLL_INTERVAL {
            return (None, Some(POLL_INTERVAL - elapsed));
        }
        *last_check = now;
        (Some(path.clone()), Some(POLL_INTERVAL))
    });
    if let Some(remaining) = remaining {
        ctx.request_repaint_after(remaining);
    }
    let Some(path) = path else {
        return false;
    };
    let next = load_palette(&path);
    let changed = PALETTE.with(|palette| {
        let mut palette = palette.borrow_mut();
        if *palette == next {
            false
        } else {
            *palette = next;
            true
        }
    });
    if changed {
        apply_style(ctx);
        ctx.request_repaint();
    }
    changed
}

/// Check for theme changes (called from draw on the UI thread).
pub fn refresh(ctx: &egui::Context) {
    refresh_at(ctx, std::time::Instant::now());
}

fn install_from(ctx: &egui::Context, path: Option<std::path::PathBuf>, now: std::time::Instant) {
    let palette = path.as_deref().and_then(load_palette);
    PALETTE.with(|p| *p.borrow_mut() = palette);
    WATCH.with(|watch| *watch.borrow_mut() = path.map(|p| (p, now)));
    apply_style(ctx);
}

/// Load the active Omarchy palette, otherwise preserve Glyph's dark fallback.
pub fn install(ctx: &egui::Context) {
    install_from(
        ctx,
        theme_path(std::env::var_os("XDG_STATE_HOME"), std::env::var_os("HOME")),
        std::time::Instant::now(),
    );
}

fn apply_style(ctx: &egui::Context) {
    // Compare perceived brightness rather than assuming every Omarchy theme is dark.
    let brightness =
        |c: egui::Color32| 299 * u32::from(c.r()) + 587 * u32::from(c.g()) + 114 * u32::from(c.b());
    let theme = if brightness(color(SURFACE)) > brightness(color(TEXT)) {
        egui::Theme::Light
    } else {
        egui::Theme::Dark
    };
    ctx.set_theme(theme);
    let mut visuals = if theme == egui::Theme::Light {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    visuals.panel_fill = color(PANEL);
    visuals.window_fill = color(PANEL);
    visuals.faint_bg_color = color(SURFACE);
    visuals.extreme_bg_color = color(SURFACE);
    visuals.widgets.active.bg_fill = color(ACCENT);
    visuals.widgets.active.fg_stroke.color = color(SURFACE);
    visuals.widgets.hovered.bg_fill = color(CONTROL_HOVER);
    visuals.widgets.hovered.fg_stroke.color = color(TEXT);
    visuals.widgets.inactive.bg_fill = color(CONTROL);
    visuals.widgets.noninteractive.bg_fill = color(PANEL);
    visuals.widgets.inactive.fg_stroke.color = color(TEXT);
    visuals.window_stroke.color = color(STROKE);
    visuals.window_corner_radius = egui::CornerRadius::same(4);
    visuals.menu_corner_radius = egui::CornerRadius::same(4);
    visuals.selection.bg_fill = color(ACCENT);
    visuals.selection.stroke.color = color(SURFACE);
    visuals.override_text_color = Some(color(TEXT));

    let mut style = (*ctx.style_of(theme)).clone();
    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(4.0, 4.0);
    style.spacing.button_padding = egui::vec2(6.0, 3.0);
    style.spacing.window_margin = egui::Margin::same(8);
    style
        .text_styles
        .insert(egui::TextStyle::Heading, egui::FontId::proportional(16.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, egui::FontId::proportional(11.5));
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(12.0));
    ctx.set_style_of(theme, style);
}

pub fn translucent(color: egui::Color32, alpha: u8) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

#[cfg(test)]
mod tests {
    use super::*;
    const DARK: &str = "background = \"#101820\"\nforeground = '#eef0f2'\naccent = \"#ab8030\" # comment\ncolor6 = \"#20b0c0\"\ncolor2 = \"#30c080\"\n";

    const LIGHT: &str =
        "background = \"#faf8f0\"\nforeground = \"#182020\"\naccent = \"#805000\"\n";

    #[test]
    fn minimized_ui_defers_poll_until_visible() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("colors.toml");
        std::fs::write(&path, DARK).unwrap();
        let ctx = egui::Context::default();
        let now = std::time::Instant::now();
        install_from(&ctx, Some(path.clone()), now);
        std::fs::write(&path, LIGHT).unwrap();
        ctx.input_mut(|input| {
            input
                .raw
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .minimized = Some(true)
        });
        assert!(!refresh_at(&ctx, now + POLL_INTERVAL));
        assert_eq!(ctx.theme(), egui::Theme::Dark);
        ctx.input_mut(|input| {
            input
                .raw
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .minimized = Some(false)
        });
        assert!(refresh_at(&ctx, now + POLL_INTERVAL));
        assert_eq!(ctx.theme(), egui::Theme::Light);
    }

    #[test]
    fn oversized_theme_falls_back_without_partial_palette() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("colors.toml");
        std::fs::write(&path, format!("{DARK}{}", "# comment\n".repeat(10_000))).unwrap();
        let ctx = egui::Context::default();
        install_from(&ctx, Some(path), std::time::Instant::now());
        assert_eq!(color(SURFACE), SURFACE);
        assert_eq!(ctx.theme(), egui::Theme::Dark);
    }

    #[test]
    fn missing_and_malformed_theme_launch_with_original_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("colors.toml");
        let ctx = egui::Context::default();
        for source in [None, Some("broken"), Some("background = \"#ffffff\"")] {
            if let Some(source) = source {
                std::fs::write(&path, source).unwrap();
            }
            install_from(&ctx, Some(path.clone()), std::time::Instant::now());
            assert_eq!(color(SURFACE), SURFACE);
            assert_eq!(color(TEXT), TEXT);
            assert_eq!(ctx.theme(), egui::Theme::Dark);
            assert_eq!(ctx.style_of(egui::Theme::Dark).visuals.panel_fill, PANEL);
        }
    }

    #[test]
    fn refresh_polls_once_per_second_and_reloads_real_theme() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("colors.toml");
        std::fs::write(&path, DARK).unwrap();
        let ctx = egui::Context::default();
        let now = std::time::Instant::now();
        install_from(&ctx, Some(path.clone()), now);
        std::fs::write(&path, LIGHT).unwrap();
        assert!(!refresh_at(
            &ctx,
            now + std::time::Duration::from_millis(999)
        ));
        assert_eq!(color(SURFACE), Palette::parse(DARK).unwrap().background);
        assert!(refresh_at(&ctx, now + std::time::Duration::from_secs(1)));
        assert_eq!(color(SURFACE), Palette::parse(LIGHT).unwrap().background);
        assert_eq!(ctx.theme(), egui::Theme::Light);
        assert!(!refresh_at(&ctx, now + std::time::Duration::from_secs(2)));
        std::fs::write(&path, "malformed").unwrap();
        assert!(!refresh_at(
            &ctx,
            now + std::time::Duration::from_millis(2999)
        ));
        assert_eq!(ctx.theme(), egui::Theme::Light);
        assert!(refresh_at(&ctx, now + std::time::Duration::from_secs(3)));
        assert_eq!(color(SURFACE), SURFACE);
        assert_eq!(ctx.theme(), egui::Theme::Dark);
        std::fs::write(&path, DARK).unwrap();
        assert!(refresh_at(&ctx, now + std::time::Duration::from_secs(4)));
        std::fs::remove_file(&path).unwrap();
        assert!(refresh_at(&ctx, now + std::time::Duration::from_secs(5)));
        assert_eq!(color(ACCENT), ACCENT);
        std::fs::write(&path, LIGHT).unwrap();
        assert!(refresh_at(&ctx, now + std::time::Duration::from_secs(6)));
    }

    #[test]
    fn launch_loads_real_theme_and_forces_light_or_dark() {
        let dir = tempfile::tempdir().unwrap();
        let path = theme_path(Some(dir.path().as_os_str().into()), None).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let ctx = egui::Context::default();
        for (source, expected_theme) in [(LIGHT, egui::Theme::Light), (DARK, egui::Theme::Dark)] {
            std::fs::write(&path, source).unwrap();
            install_from(&ctx, Some(path.clone()), std::time::Instant::now());
            assert_eq!(ctx.theme(), expected_theme);
            assert_eq!(color(SURFACE), Palette::parse(source).unwrap().background);
            let style = ctx.style_of(ctx.theme());
            let visuals = &style.visuals;
            assert_eq!(visuals.dark_mode, expected_theme == egui::Theme::Dark);
            assert_eq!(visuals.panel_fill, color(PANEL));
            assert_eq!(visuals.override_text_color, Some(color(TEXT)));
            assert_eq!(visuals.selection.bg_fill, color(ACCENT));
            assert_eq!(
                ctx.style_of(expected_theme).visuals.panel_fill,
                color(PANEL)
            );
        }
    }

    #[test]
    fn resolves_xdg_path_with_home_fallback() {
        let suffix = "omarchy/current/theme/colors.toml";
        assert_eq!(
            theme_path(Some("/config".into()), Some("/home/test".into())),
            Some(std::path::PathBuf::from("/config").join(suffix))
        );
        for xdg in [None, Some("".into()), Some("relative".into())] {
            assert_eq!(
                theme_path(xdg, Some("/home/test".into())),
                Some(std::path::PathBuf::from("/home/test/.local/state").join(suffix))
            );
        }
        assert_eq!(theme_path(None, None), None);
        assert_eq!(theme_path(None, Some("".into())), None);
    }

    #[test]
    fn maps_all_semantic_colors_on_ui_thread() {
        let palette = Palette::parse(DARK).unwrap();
        PALETTE.with(|p| *p.borrow_mut() = Some(palette.clone()));
        assert_eq!(color(SURFACE), palette.background);
        assert_eq!(color(CANVAS), palette.background);
        assert_eq!(color(TEXT), palette.foreground);
        assert_eq!(color(ACCENT), palette.accent);
        assert_eq!(color(GOLD), color(ACCENT));
        assert_eq!(color(CYAN), palette.cyan);
        assert_eq!(color(GREEN), palette.green);
        for semantic in [
            PANEL,
            PANEL_RAISED,
            CONTROL,
            CONTROL_HOVER,
            CARD,
            TEXT_MUTED,
            TEXT_FAINT,
            ACCENT_STRONG,
            ACCENT_SOFT,
            CYAN_SOFT,
            STROKE,
            STROKE_STRONG,
        ] {
            assert_ne!(color(semantic), semantic, "{semantic:?}");
        }
        let unknown = egui::Color32::from_rgb(1, 2, 3);
        assert_eq!(color(unknown), unknown);
        std::thread::spawn(|| assert_eq!(color(SURFACE), SURFACE))
            .join()
            .unwrap();
        PALETTE.with(|p| *p.borrow_mut() = None);
        assert_eq!(color(SURFACE), SURFACE);
    }

    #[test]
    fn malformed_palette_fails_closed() {
        for invalid in [
            "",
            "background = \"#123456\"",
            "garbage",
            "[colors]",
            "accent = \"#zzzzzz\"",
        ] {
            assert!(Palette::parse(invalid).is_none());
        }
        for suffix in [
            "background = \"#123456\"",
            "color2 = \"#bad\"",
            " = \"#123456\"",
            "color6 = \"#123456\" trailing",
        ] {
            assert!(
                Palette::parse(&format!("{DARK}{suffix}\n")).is_none(),
                "{suffix}"
            );
        }
    }

    #[test]
    fn parses_flat_quoted_omarchy_colors() {
        let palette = Palette::parse(DARK).expect("valid Omarchy theme");
        assert_eq!(palette.background, egui::Color32::from_rgb(16, 24, 32));
        assert_eq!(palette.foreground, egui::Color32::from_rgb(238, 240, 242));
        assert_eq!(palette.accent, egui::Color32::from_rgb(171, 128, 48));
        assert_eq!(palette.cyan, egui::Color32::from_rgb(32, 176, 192));
        assert_eq!(palette.green, egui::Color32::from_rgb(48, 192, 128));
    }
}
