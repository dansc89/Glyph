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
}
