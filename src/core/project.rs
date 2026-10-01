use crate::pdf::PdfDocumentSummary;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenDocument {
    pub path: PathBuf,
    pub summary: PdfDocumentSummary,
}

impl OpenDocument {
    pub fn display_name(&self) -> String {
        self.path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Untitled.pdf")
            .to_owned()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectState {
    pub name: String,
    pub document: Option<OpenDocument>,
    pub selected_page: usize,
}

impl ProjectState {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            document: None,
            selected_page: 0,
        }
    }

    pub fn open_document(&mut self, path: PathBuf, summary: PdfDocumentSummary) {
        self.selected_page = 0;
        self.document = Some(OpenDocument { path, summary });
    }
}
