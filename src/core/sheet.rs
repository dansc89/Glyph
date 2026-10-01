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
    let cleaned = raw
        .trim()
        .replace(' ', "")
        .replace('–', "-")
        .replace('—', "-");
    if cleaned.len() < 2 || cleaned.len() > 24 {
        return None;
    }
    let has_digit = cleaned.chars().any(|c| c.is_ascii_digit());
    let has_letter = cleaned.chars().any(|c| c.is_ascii_alphabetic());
    if has_digit && has_letter {
        Some(SheetId(cleaned.to_uppercase()))
    } else {
        None
    }
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
    fn rejects_non_sheet_noise() {
        assert!(normalize_sheet_id("general notes").is_none());
        assert!(normalize_sheet_id("101").is_none());
    }
}
