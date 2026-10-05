use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetCandidate {
    pub id: SheetId,
    pub title: Option<String>,
    pub page_index: usize,
    pub confidence: u8,
}

pub fn normalize_sheet_id(raw: &str) -> Option<SheetId> {
    let cleaned = raw.trim().replace(' ', "").replace(['–', '—'], "-");
    let cleaned = cleaned.to_uppercase();
    let prefix_len = cleaned.bytes().take_while(u8::is_ascii_alphabetic).count();
    let prefix = &cleaned[..prefix_len];
    // Discipline designators, not arbitrary equipment names or words.
    if !matches!(
        prefix,
        "A" | "AD" | "AS" | "C" | "E" | "F" | "G" | "I" | "L" | "M" | "P" | "S" | "T" | "Q"
    ) {
        return None;
    }
    let number = cleaned[prefix_len..]
        .strip_prefix('-')
        .unwrap_or(&cleaned[prefix_len..]);
    let valid = if let Some((major, minor)) = number.split_once('.') {
        (1..=2).contains(&major.len())
            && minor.len() == 2
            && major.bytes().all(|c| c.is_ascii_digit())
            && minor.bytes().all(|c| c.is_ascii_digit())
    } else {
        number.len() == 3 && number.bytes().all(|c| c.is_ascii_digit())
    };
    valid.then_some(SheetId(cleaned))
}

pub fn generate_bookmark_titles(
    candidates: &[SheetCandidate],
    page_count: usize,
) -> Vec<SheetCandidate> {
    let mut by_page: Vec<Option<SheetCandidate>> = vec![None; page_count];
    for candidate in candidates {
        if candidate.page_index >= page_count {
            continue;
        }
        let replace = by_page[candidate.page_index]
            .as_ref()
            .map(|existing| candidate.confidence > existing.confidence)
            .unwrap_or(true);
        if replace {
            by_page[candidate.page_index] = Some(candidate.clone());
        }
    }
    by_page
        .into_iter()
        .enumerate()
        .map(|(page_index, candidate)| {
            candidate.unwrap_or_else(|| SheetCandidate {
                id: SheetId(format!("P{}", page_index + 1)),
                title: Some(format!("Page {}", page_index + 1)),
                page_index,
                confidence: 0,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_common_sheet_ids() {
        assert_eq!(normalize_sheet_id(" a-101 ").unwrap().0, "A-101");
        assert_eq!(normalize_sheet_id("E 2.01").unwrap().0, "E2.01");
    }

    #[test]
    fn rejects_words_and_equipment_model_codes() {
        for noise in [
            "MODEL123",
            "AHU-101",
            "PUMP2.01",
            "A-1010",
            "X123",
            "A-101-LOW",
        ] {
            assert!(normalize_sheet_id(noise).is_none(), "{noise}");
        }
        for id in ["A-101", "A1.01", "E2.01", "L1.11"] {
            assert_eq!(normalize_sheet_id(id).unwrap().0, id);
        }
    }

    #[test]
    fn rejects_non_sheet_noise() {
        assert!(normalize_sheet_id("general notes").is_none());
        assert!(normalize_sheet_id("101").is_none());
    }

    #[test]
    fn generates_one_bookmark_candidate_per_page() {
        let candidates = vec![
            SheetCandidate {
                id: SheetId("A-101".to_owned()),
                title: Some("Floor Plan".to_owned()),
                page_index: 1,
                confidence: 80,
            },
            SheetCandidate {
                id: SheetId("A-101-LOW".to_owned()),
                title: Some("Low confidence".to_owned()),
                page_index: 1,
                confidence: 10,
            },
        ];

        let generated = generate_bookmark_titles(&candidates, 3);

        assert_eq!(generated.len(), 3);
        assert_eq!(generated[0].id.0, "P1");
        assert_eq!(generated[1].id.0, "A-101");
        assert_eq!(generated[2].title.as_deref(), Some("Page 3"));
    }
}
