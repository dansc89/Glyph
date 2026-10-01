use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PdfError {
    #[error("file does not exist: {0}")]
    MissingFile(String),
    #[error("not a PDF path: {0}")]
    NotPdf(String),
    #[error("PDF load failed: {0}")]
    Load(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PdfPageInfo {
    pub index: usize,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PdfDocumentSummary {
    pub page_count: usize,
    pub pages: Vec<PdfPageInfo>,
    pub title: Option<String>,
}

pub trait PdfEngine {
    fn inspect(&self, path: &Path) -> Result<PdfDocumentSummary, PdfError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct LopdfInspectionEngine;

impl PdfEngine for LopdfInspectionEngine {
    fn inspect(&self, path: &Path) -> Result<PdfDocumentSummary, PdfError> {
        if !path.exists() {
            return Err(PdfError::MissingFile(path.display().to_string()));
        }
        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| !ext.eq_ignore_ascii_case("pdf"))
            .unwrap_or(true)
        {
            return Err(PdfError::NotPdf(path.display().to_string()));
        }
        let doc = lopdf::Document::load(path).map_err(|err| PdfError::Load(err.to_string()))?;
        let page_count = doc.get_pages().len();
        let pages = (0..page_count)
            .map(|index| PdfPageInfo {
                index,
                label: Some(format!("Page {}", index + 1)),
            })
            .collect();
        let title = doc.trailer.get(b"Info").ok().and_then(|_| None);
        Ok(PdfDocumentSummary {
            page_count,
            pages,
            title,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn rejects_missing_files() {
        let engine = LopdfInspectionEngine;
        let err = engine
            .inspect(&PathBuf::from("/does/not/exist.pdf"))
            .unwrap_err();
        assert!(matches!(err, PdfError::MissingFile(_)));
    }

    #[test]
    fn rejects_non_pdf_extensions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, "not a pdf").unwrap();
        let engine = LopdfInspectionEngine;
        let err = engine.inspect(&path).unwrap_err();
        assert!(matches!(err, PdfError::NotPdf(_)));
    }
}
