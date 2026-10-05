//! Allocation-free selection state over worker-extracted page text.
//!
//! The viewer clears this state when changing pages/documents. Geometry and
//! indices are borrowed from PageText; only explicit text copying allocates.
use crate::{core::links::PdfRect, pdf::PageText};
use std::ops::RangeInclusive;

#[derive(Debug, Default)]
pub(super) struct TextSelection {
    endpoints: Option<(usize, usize)>,
}

impl TextSelection {
    pub(super) fn clear(&mut self) {
        self.endpoints = None;
    }
    /// Start only on a valid glyph box; a miss clears any old selection.
    pub(super) fn begin(&mut self, page: &PageText, pos: egui::Pos2) -> bool {
        let hit = page.glyphs.iter().position(|glyph| {
            glyph.rect.is_some_and(|r| {
                valid_rect(r)
                    && pos.x >= r.x
                    && pos.x <= r.x + r.width
                    && pos.y >= r.y
                    && pos.y <= r.y + r.height
            })
        });
        self.endpoints = hit.map(|i| (i, i));
        hit.is_some()
    }
    /// Extend to the closest valid box, including outside-page drag points.
    /// Returns true on a valid endpoint (even if unchanged); distance ties
    /// resolve in reading order. No active selection or nonfinite input fails.
    pub(super) fn extend(&mut self, page: &PageText, pos: egui::Pos2) -> bool {
        if !pos.x.is_finite() || !pos.y.is_finite() {
            return false;
        }
        let Some((anchor, _)) = self.endpoints else {
            return false;
        };
        if anchor >= page.glyphs.len() {
            self.clear();
            return false;
        }
        let hit = page
            .glyphs
            .iter()
            .enumerate()
            .filter_map(|(i, glyph)| {
                let r = glyph.rect.filter(|r| valid_rect(*r))?;
                // Widen before subtracting: finite off-crop coordinates and
                // drag positions can otherwise overflow an f32 difference.
                let dx = (f64::from(r.x) - f64::from(pos.x))
                    .max(0.0)
                    .max(f64::from(pos.x) - f64::from(r.x + r.width));
                let dy = (f64::from(r.y) - f64::from(pos.y))
                    .max(0.0)
                    .max(f64::from(pos.y) - f64::from(r.y + r.height));
                Some((i, dx * dx + dy * dy))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i);
        if let Some(i) = hit {
            self.endpoints = Some((anchor, i));
        }
        hit.is_some()
    }
    /// Select a contiguous alphanumeric/underscore group; punctuation and spaces
    /// remain single glyph selections. This is not a linguistic word segmenter.
    pub(super) fn select_word(&mut self, page: &PageText, pos: egui::Pos2) -> bool {
        if !self.begin(page, pos) {
            return false;
        }
        let (mut start, mut end) = self.endpoints.unwrap();
        let is_word = |i: usize| {
            let text = &page.glyphs[i].text;
            !text.is_empty() && text.chars().all(|c| c.is_alphanumeric() || c == '_')
        };
        if is_word(start) {
            while start > 0 && is_word(start - 1) {
                start -= 1;
            }
            while end + 1 < page.glyphs.len() && is_word(end + 1) {
                end += 1;
            }
        }
        self.endpoints = Some((start, end));
        true
    }

    pub(super) fn range(&self) -> Option<RangeInclusive<usize>> {
        self.endpoints.map(|(a, b)| a.min(b)..=a.max(b))
    }
    pub(super) fn selected_text(&self, page: &PageText) -> String {
        let mut text = String::new();
        if let Some(glyphs) = self.range().and_then(|range| page.glyphs.get(range)) {
            for glyph in glyphs {
                text.push_str(&glyph.text);
            }
        }
        text
    }
}

fn valid_rect(r: PdfRect) -> bool {
    r.x.is_finite()
        && r.y.is_finite()
        && r.width.is_finite()
        && r.height.is_finite()
        && r.width > 0.0
        && r.height > 0.0
        && (r.x + r.width).is_finite()
        && (r.y + r.height).is_finite()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{core::links::PdfRect, pdf::TextGlyph};

    fn glyph(text: &str, x: f32, y: f32) -> TextGlyph {
        TextGlyph {
            text: text.into(),
            rect: Some(PdfRect {
                x,
                y,
                width: 0.1,
                height: 0.1,
            }),
        }
    }

    fn page() -> PageText {
        PageText {
            page_index: 4,
            glyphs: vec![
                glyph("A", 0.1, 0.1),
                glyph(" ", 0.2, 0.1),
                glyph("é", 0.3, 0.1),
                TextGlyph {
                    text: "\r\n\u{2003}".into(),
                    rect: None,
                },
                glyph("字", 0.1, 0.4),
            ],
        }
    }

    #[test]
    fn drag_preserves_unicode_and_unboxed_breaks_in_both_directions() {
        let page = page();
        let mut selection = TextSelection::default();
        for (start, end) in [
            (egui::pos2(0.15, 0.15), egui::pos2(0.15, 0.45)),
            (egui::pos2(0.15, 0.45), egui::pos2(0.15, 0.15)),
        ] {
            assert!(selection.begin(&page, start));
            assert!(selection.extend(&page, end));
            assert_eq!(selection.range(), Some(0..=4));
            assert_eq!(selection.selected_text(&page), "A é\r\n\u{2003}字");
        }
    }

    #[test]
    fn drag_snaps_to_nearest_box_in_gaps_and_outside_page() {
        let page = page();
        let mut selection = TextSelection::default();
        assert!(selection.begin(&page, egui::pos2(0.35, 0.15)));
        for (pos, range) in [
            (egui::pos2(0.15, 0.35), 2..=4),
            (egui::pos2(-10.0, -10.0), 0..=2),
            (egui::pos2(10.0, 0.15), 2..=2),
            (egui::pos2(0.15, 10.0), 2..=4),
        ] {
            assert!(selection.extend(&page, pos));
            assert_eq!(selection.range(), Some(range));
        }
    }

    #[test]
    fn invalid_geometry_is_not_selectable_but_off_crop_boxes_are() {
        let mut page = page();
        let mut selection = TextSelection::default();
        let invalid = [
            PdfRect {
                x: 0.1,
                y: 0.1,
                width: 0.0,
                height: 0.1,
            },
            PdfRect {
                x: 0.1,
                y: 0.1,
                width: 0.1,
                height: 0.0,
            },
            PdfRect {
                x: 0.1,
                y: 0.1,
                width: -0.1,
                height: 0.1,
            },
            PdfRect {
                x: 0.1,
                y: 0.1,
                width: f32::INFINITY,
                height: 0.1,
            },
            PdfRect {
                x: f32::NAN,
                y: 0.1,
                width: 0.1,
                height: 0.1,
            },
            PdfRect {
                x: 0.1,
                y: 0.1,
                width: 0.1,
                height: f32::INFINITY,
            },
        ];
        for r in invalid {
            page.glyphs[0].rect = Some(r);
            assert!(!selection.begin(&page, egui::pos2(0.1, 0.1)));
            assert!(selection.begin(&page, egui::pos2(0.35, 0.15)));
            assert!(selection.extend(&page, egui::pos2(0.1, 0.1)));
            assert_eq!(selection.range(), Some(1..=2));
        }
        page.glyphs[0] = glyph("offcrop", -0.2, 0.1);
        assert!(selection.begin(&page, egui::pos2(0.35, 0.15)));
        assert!(selection.extend(&page, egui::pos2(-0.3, 0.15)));
        assert_eq!(selection.range(), Some(0..=2));
    }

    #[test]
    fn empty_or_textless_pages_are_safe_even_after_selection() {
        let original = page();
        let mut selection = TextSelection::default();
        assert!(selection.begin(&original, egui::pos2(0.15, 0.45)));
        let empty = PageText {
            page_index: 4,
            glyphs: vec![],
        };
        assert_eq!(selection.selected_text(&empty), "");
        assert!(!selection.extend(&empty, egui::pos2(0.5, 0.5)));
        assert_eq!(selection.range(), None);
        assert!(!selection.begin(&empty, egui::pos2(0.5, 0.5)));
        let textless = PageText {
            page_index: 4,
            glyphs: vec![TextGlyph {
                text: "\n".into(),
                rect: None,
            }],
        };
        assert!(!selection.begin(&textless, egui::pos2(0.5, 0.5)));
        assert!(!selection.extend(&textless, egui::pos2(0.5, 0.5)));
        assert_eq!(selection.selected_text(&textless), "");
    }

    #[test]
    fn nonfinite_drag_points_do_not_change_selection() {
        let page = page();
        let mut selection = TextSelection::default();
        assert!(selection.begin(&page, egui::pos2(0.35, 0.15)));
        for pos in [
            egui::pos2(f32::NAN, 0.15),
            egui::pos2(0.15, f32::INFINITY),
            egui::pos2(f32::NEG_INFINITY, 0.15),
        ] {
            assert!(!selection.extend(&page, pos));
            assert_eq!(selection.range(), Some(2..=2));
            assert!(!selection.begin(&page, pos));
            assert_eq!(selection.range(), None);
            assert!(selection.begin(&page, egui::pos2(0.35, 0.15)));
        }
    }

    #[test]
    fn word_selection_stops_at_unicode_whitespace_and_punctuation() {
        let page = PageText {
            page_index: 4,
            glyphs: vec![
                glyph("é", 0.0, 0.1),
                glyph("字", 0.1, 0.1),
                glyph("_", 0.2, 0.1),
                glyph("2", 0.3, 0.1),
                TextGlyph {
                    text: "\u{2003}\n".into(),
                    rect: None,
                },
                glyph("B", 0.5, 0.1),
                glyph("!", 0.6, 0.1),
                glyph("C", 0.7, 0.1),
                glyph(" ", 0.8, 0.1),
            ],
        };
        let mut selection = TextSelection::default();
        for (x, range, text) in [
            (0.05, 0..=3, "é字_2"),
            (0.25, 0..=3, "é字_2"),
            (0.55, 5..=5, "B"),
            (0.65, 6..=6, "!"),
            (0.75, 7..=7, "C"),
            (0.85, 8..=8, " "),
        ] {
            assert!(selection.select_word(&page, egui::pos2(x, 0.15)));
            assert_eq!(selection.range(), Some(range));
            assert_eq!(selection.selected_text(&page), text);
        }
        assert!(!selection.select_word(&page, egui::pos2(0.95, 0.95)));
        assert_eq!(selection.range(), None);
    }

    #[test]
    fn begin_in_empty_space_clears_old_selection() {
        let page = page();
        let mut selection = TextSelection::default();
        assert_eq!(selection.range(), None);
        assert_eq!(selection.selected_text(&page), "");
        assert!(!selection.extend(&page, egui::pos2(0.15, 0.15)));
        assert!(selection.begin(&page, egui::pos2(0.15, 0.15)));
        assert!(!selection.begin(&page, egui::pos2(0.75, 0.75)));
        assert_eq!(selection.range(), None);
        assert_eq!(selection.selected_text(&page), "");
    }

    #[test]
    fn nearest_distance_does_not_overflow_with_extreme_finite_points() {
        let page = PageText {
            page_index: 4,
            glyphs: vec![
                glyph("far", -f32::MAX, 0.1),
                glyph("near", -f32::MAX * 0.5, 0.1),
            ],
        };
        let mut selection = TextSelection::default();
        assert!(selection.begin(&page, egui::pos2(-f32::MAX * 0.5, 0.15)));
        assert!(selection.extend(&page, egui::pos2(f32::MAX, 0.15)));
        assert_eq!(selection.range(), Some(1..=1));
    }

    #[test]
    fn clear_removes_the_active_selection() {
        let page = page();
        let mut selection = TextSelection::default();
        assert!(selection.begin(&page, egui::pos2(0.15, 0.15)));
        selection.clear();
        assert_eq!(selection.range(), None);
        assert_eq!(selection.selected_text(&page), "");
        assert!(!selection.extend(&page, egui::pos2(0.35, 0.15)));
    }

    #[test]
    fn begins_on_glyph_and_returns_its_text() {
        let page = page();
        let mut selection = TextSelection::default();
        assert!(selection.begin(&page, egui::pos2(0.15, 0.15)));
        assert_eq!(selection.range(), Some(0..=0));
        assert_eq!(selection.selected_text(&page), "A");
    }
}
