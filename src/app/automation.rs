use crate::pdf::PdfError;
use std::path::{Path, PathBuf};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AutomationKind {
    Bookmarks,
    Hyperlinks,
}
impl AutomationKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Bookmarks => "Auto bookmarks",
            Self::Hyperlinks => "Hyperlinks",
        }
    }
    fn suffix(self) -> &'static str {
        match self {
            Self::Bookmarks => "bookmarked",
            Self::Hyperlinks => "hyperlinked",
        }
    }
}

pub(super) struct AutomationOutcome {
    pub kind: AutomationKind,
    pub message: String,
    pub output: Option<PathBuf>,
}
pub(super) struct AutomationFeedback {
    pub kind: AutomationKind,
    pub message: String,
    pub output: Option<PathBuf>,
    pub busy: bool,
}

pub(super) fn execute(input: &Path, kind: AutomationKind) -> Result<AutomationOutcome, PdfError> {
    let engine = crate::pdf::PdfiumRenderEngine;
    let analysis = engine.analyze_sheets(input)?;
    let detected = analysis.sheets.iter().filter(|s| s.confidence > 0).count();
    if analysis.text_page_count == 0 {
        return Err(PdfError::Edit("No selectable PDF text found. This PDF needs OCR before automatic sheet detection or linking.".into()));
    }
    if kind == AutomationKind::Hyperlinks && analysis.sheets.len() < 2 {
        return Ok(AutomationOutcome{kind,output:None,message:"No cross-sheet targets: this PDF contains one sheet. Hyperlinks connect references to other sheets in the same PDF; no file was changed.".into()});
    }
    if kind == AutomationKind::Hyperlinks && detected == 0 {
        return Ok(AutomationOutcome {
            kind,
            output: None,
            message: "No sheet numbers were detected. No hyperlinks created and no file changed."
                .into(),
        });
    }
    let links = if kind == AutomationKind::Hyperlinks {
        engine.generate_sheet_label_links(input, &analysis.sheets)?
    } else {
        Vec::new()
    };
    if kind == AutomationKind::Hyperlinks && links.is_empty() {
        return Ok(AutomationOutcome {
            kind,
            output: None,
            message: format!(
                "Detected {detected} sheet numbers, but no matching cross-sheet references. Self-links and ambiguous duplicate sheet numbers are skipped. No file changed."
            ),
        });
    }
    let output = reserve_output(input, kind)?;
    let editor = crate::pdf::LopdfEditEngine;
    let report = match kind {
        AutomationKind::Bookmarks => editor.write_bookmarks(input, &output, &analysis.sheets),
        AutomationKind::Hyperlinks => editor.write_links(input, &output, &links),
    };
    match report {
        Ok(report) => Ok(AutomationOutcome {
            kind,
            message: match kind {
                AutomationKind::Bookmarks => format!(
                    "Saved {} bookmarks; detected {detected} sheet numbers. Pages without a detected number use Page N. The saved copy replaces any existing bookmark hierarchy; original PDF preserved.",
                    report.bookmarks_written
                ),
                AutomationKind::Hyperlinks => format!(
                    "Saved {} new hyperlinks from {detected} detected sheets. Existing annotations preserved; identical links are not duplicated. Original PDF preserved.",
                    report.links_written
                ),
            },
            output: Some(output),
        }),
        Err(err) => {
            let _ = std::fs::remove_file(output);
            Err(err)
        }
    }
}

fn reserve_output(input: &Path, kind: AutomationKind) -> Result<PathBuf, PdfError> {
    let mut stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("glyph-document")
        .to_owned();
    // Strip only our trailing export suffixes, not arbitrary user filenames.
    loop {
        let shorter = stem
            .strip_suffix(".glyph-bookmarked")
            .or_else(|| stem.strip_suffix(".glyph-hyperlinked"));
        if let Some(base) = shorter {
            stem = base.to_owned();
        } else {
            break;
        }
    }
    for index in 1..=10000 {
        let sequence = if index == 1 {
            String::new()
        } else {
            format!("-{index}")
        };
        let output = input.with_file_name(format!("{stem}.glyph-{}{sequence}.pdf", kind.suffix()));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)
        {
            Ok(_) => return Ok(output),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Err(PdfError::Edit(format!(
                    "Cannot create output {}: {err}",
                    output.display()
                )));
            }
        }
    }
    Err(PdfError::Edit(
        "Too many existing export files; choose a new source name.".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::{PdfEngine, PdfRenderEngine};
    fn fixture(path: &Path, bodies: &[&str]) {
        use lopdf::{Document, Object, Stream, dictionary};
        let mut doc = Document::with_version("1.7");
        let pages_id = doc.new_object_id();
        let font = doc
            .add_object(dictionary! {"Type"=>"Font", "Subtype"=>"Type1", "BaseFont"=>"Helvetica"});
        let kids: Vec<_> = bodies.iter().map(|body| {
            let content = doc.add_object(Stream::new(dictionary! {}, body.as_bytes().to_vec()));
            Object::Reference(doc.add_object(dictionary! {
                "Type"=>"Page", "Parent"=>pages_id, "MediaBox"=>vec![0.into(),0.into(),600.into(),800.into()],
                "Resources"=>dictionary! {"Font"=>dictionary! {"F1"=>font}}, "Contents"=>content
            }))
        }).collect();
        doc.objects.insert(
            pages_id,
            Object::Dictionary(
                dictionary! {"Type"=>"Pages", "Count"=>kids.len() as i64, "Kids"=>kids},
            ),
        );
        let catalog = doc.add_object(dictionary! {"Type"=>"Catalog", "Pages"=>pages_id});
        doc.trailer.set("Root", catalog);
        doc.save(path).unwrap();
    }

    #[test]
    #[ignore = "read-only local SALTAIR fixture; exports only into a temporary directory"]
    fn real_saltair_export_has_l1_11_bookmark_and_unchanged_rendering() {
        let source = Path::new(
            "/home/daniel/Downloads/SALTAIR - ARCH - PC CORR - GREEN PLAN - 260923 - LAND.pdf",
        );
        let original = std::fs::read(source).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("saltair.pdf");
        std::fs::write(&input, &original).unwrap();
        let outcome = execute(&input, AutomationKind::Bookmarks).unwrap();
        let output = outcome.output.unwrap();
        let summary = crate::pdf::LopdfInspectionEngine.inspect(&output).unwrap();
        assert_eq!(summary.bookmarks.len(), 1);
        assert_eq!(summary.bookmarks[0].title, "L1.11");
        assert_eq!(summary.bookmarks[0].page_index, Some(0));
        let engine = crate::pdf::PdfiumRenderEngine;
        assert_eq!(
            engine.render_page(&input, 0, 512).unwrap(),
            engine.render_page(&output, 0, 512).unwrap()
        );
        assert_eq!(std::fs::read(source).unwrap(), original);
        let no_links = execute(&input, AutomationKind::Hyperlinks).unwrap();
        assert!(no_links.output.is_none());
        assert!(no_links.message.contains("one sheet"));
    }

    #[test]
    fn automation_exports_reopen_with_sheet_names_and_cross_sheet_links() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("plan.pdf");
        fixture(
            &input,
            &[
                "BT /F1 10 Tf 440 95 Td (SHEET NUMBER) Tj ET BT /F1 24 Tf 440 55 Td (A-101) Tj ET BT /F1 12 Tf 60 400 Td (See E2.01.) Tj ET",
                "BT /F1 10 Tf 440 95 Td (SHEET NUMBER) Tj ET BT /F1 24 Tf 440 55 Td (E2.01) Tj ET BT /F1 12 Tf 60 400 Td (See A-101.) Tj ET",
            ],
        );
        let original = std::fs::read(&input).unwrap();
        let bookmarks_outcome = execute(&input, AutomationKind::Bookmarks).unwrap();
        assert!(
            bookmarks_outcome
                .message
                .contains("replaces any existing bookmark hierarchy")
        );
        let bookmarked = bookmarks_outcome.output.unwrap();
        let summary = crate::pdf::LopdfInspectionEngine
            .inspect(&bookmarked)
            .unwrap();
        assert_eq!(
            summary
                .bookmarks
                .iter()
                .map(|b| b.title.as_str())
                .collect::<Vec<_>>(),
            ["A-101", "E2.01"]
        );
        let linked = execute(&bookmarked, AutomationKind::Hyperlinks)
            .unwrap()
            .output
            .unwrap();
        assert_eq!(
            crate::pdf::extract_internal_links(&linked, 0).unwrap()[0].target_page,
            1
        );
        assert_eq!(
            crate::pdf::extract_internal_links(&linked, 1).unwrap()[0].target_page,
            0
        );
        let repeated = execute(&linked, AutomationKind::Hyperlinks).unwrap();
        assert!(repeated.message.contains("Saved 0 new hyperlinks"));
        assert_eq!(
            crate::pdf::extract_internal_links(&repeated.output.unwrap(), 0)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(std::fs::read(&input).unwrap(), original);
    }

    #[test]
    fn textless_and_single_sheet_jobs_explain_no_output_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("plan.pdf");
        fixture(&input, &[""]);
        for kind in [AutomationKind::Bookmarks, AutomationKind::Hyperlinks] {
            assert!(
                execute(&input, kind)
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("needs OCR")
            );
        }
        fixture(
            &input,
            &["BT /F1 10 Tf 440 95 Td (SHEET NUMBER) Tj ET BT /F1 24 Tf 440 55 Td (L1.11) Tj ET"],
        );
        let outcome = execute(&input, AutomationKind::Hyperlinks).unwrap();
        assert!(outcome.output.is_none());
        assert!(outcome.message.contains("one sheet"));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn reserved_outputs_never_overwrite_original_or_previous_export() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir
            .path()
            .join("plan.glyph-bookmarked.glyph-bookmarked.pdf");
        std::fs::write(&input, b"original").unwrap();
        let existing = dir.path().join("plan.glyph-bookmarked.pdf");
        std::fs::write(&existing, b"previous").unwrap();
        let first = reserve_output(&input, AutomationKind::Bookmarks).unwrap();
        let second = reserve_output(&input, AutomationKind::Bookmarks).unwrap();
        assert_ne!(first, input);
        assert_ne!(first, existing);
        assert_ne!(first, second);
        assert_eq!(std::fs::read(input).unwrap(), b"original");
        assert_eq!(std::fs::read(existing).unwrap(), b"previous");
        assert!(
            !first
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains("bookmarked.glyph-bookmarked")
        );
    }
}
