use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PdfRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl PdfRect {
    pub fn area(self) -> f32 {
        (self.width.max(0.0)) * (self.height.max(0.0))
    }

    pub fn is_valid(self) -> bool {
        self.width > 0.0 && self.height > 0.0 && self.x.is_finite() && self.y.is_finite()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkProposal {
    pub from_page: usize,
    pub target_page: usize,
    pub rect: PdfRect,
    pub label: String,
}

impl LinkProposal {
    pub fn is_actionable(&self) -> bool {
        self.from_page != self.target_page && self.rect.is_valid() && !self.label.trim().is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkLabelHit {
    pub from_page: usize,
    pub label: String,
    pub rect: PdfRect,
}

pub fn generate_link_proposals(
    hits: &[LinkLabelHit],
    sheets: &[crate::core::sheet::SheetCandidate],
) -> Vec<LinkProposal> {
    let mut by_id = std::collections::HashMap::new();
    for sheet in sheets {
        by_id.entry(sheet.id.0.clone()).or_insert(sheet.page_index);
    }

    hits.iter()
        .filter_map(|hit| {
            let id = crate::core::sheet::normalize_sheet_id(&hit.label)?;
            let target_page = *by_id.get(&id.0)?;
            let proposal = LinkProposal {
                from_page: hit.from_page,
                target_page,
                rect: hit.rect,
                label: id.0,
            };
            proposal.is_actionable().then_some(proposal)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_self_links_and_invalid_rects() {
        let proposal = LinkProposal {
            from_page: 0,
            target_page: 0,
            rect: PdfRect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
            label: "A-101".to_owned(),
        };
        assert!(!proposal.is_actionable());
    }

    #[test]
    fn generates_links_from_matching_sheet_labels() {
        let hits = vec![LinkLabelHit {
            from_page: 0,
            label: "a-101".to_owned(),
            rect: PdfRect {
                x: 10.0,
                y: 20.0,
                width: 30.0,
                height: 12.0,
            },
        }];
        let sheets = vec![crate::core::sheet::SheetCandidate {
            id: crate::core::sheet::SheetId("A-101".to_owned()),
            title: Some("Floor Plan".to_owned()),
            page_index: 2,
            confidence: 90,
        }];

        let proposals = generate_link_proposals(&hits, &sheets);

        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].target_page, 2);
        assert_eq!(proposals[0].label, "A-101");
    }
}
