use super::PdfError;
use crate::core::links::PdfRect;
use std::path::Path;

/// An internal navigation hotspot in native, bottom-left-origin PDF coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct PdfInternalLink {
    pub rect: PdfRect,
    pub target_page: usize,
}

/// Extract internal links from a zero-based page index; targets are also zero-based.
///
/// Supports explicit destinations and GoTo actions, including named destinations.
/// Malformed annotations and external actions are skipped. Invalid source page
/// indexes return `PdfError::Load`. This does not launch URLs or execute actions.
pub fn extract_internal_links(
    path: &Path,
    page_index: usize,
) -> Result<Vec<PdfInternalLink>, PdfError> {
    InternalLinkIndex::load(path)?.links(page_index)
}

/// One parsed document and page map, reusable across navigation within a generation.
pub struct InternalLinkIndex {
    doc: lopdf::Document,
    pages: Vec<lopdf::ObjectId>,
}

impl InternalLinkIndex {
    pub fn load(path: &Path) -> Result<Self, PdfError> {
        super::validate_pdf_path(path)?;
        let doc = lopdf::Document::load(path).map_err(|err| PdfError::Load(err.to_string()))?;
        let pages = doc.get_pages().values().copied().collect();
        Ok(Self { doc, pages })
    }

    pub fn links(&self, page_index: usize) -> Result<Vec<PdfInternalLink>, PdfError> {
        extract_page_links(&self.doc, &self.pages, page_index)
    }
}

fn extract_page_links(
    doc: &lopdf::Document,
    pages: &[lopdf::ObjectId],
    page_index: usize,
) -> Result<Vec<PdfInternalLink>, PdfError> {
    let page_id = pages.get(page_index).ok_or_else(|| {
        PdfError::Load(format!(
            "page index {page_index} is outside document page count {}",
            pages.len()
        ))
    })?;
    let page = doc
        .get_object(*page_id)
        .and_then(lopdf::Object::as_dict)
        .map_err(|err| PdfError::Load(err.to_string()))?;
    let Some(annotations) = page
        .get(b"Annots")
        .ok()
        .and_then(|o| resolve(doc, o))
        .and_then(|o| o.as_array().ok())
    else {
        return Ok(Vec::new());
    };
    let mut links = Vec::new();
    for annotation in annotations {
        let annotation = resolve(doc, annotation);
        let Some(dict) = annotation.and_then(|o| o.as_dict().ok()) else {
            continue;
        };
        if dict.get(b"Subtype").and_then(lopdf::Object::as_name).ok() != Some(b"Link") {
            continue;
        }
        let Some(rect) = dict
            .get(b"Rect")
            .ok()
            .and_then(|o| resolve(doc, o))
            .and_then(|o| o.as_array().ok())
        else {
            continue;
        };
        if rect.len() != 4 {
            continue;
        }
        let coords: Option<Vec<f32>> = rect.iter().map(|o| o.as_float().ok()).collect();
        let Some(c) = coords else {
            continue;
        };
        let rect = PdfRect {
            x: c[0].min(c[2]),
            y: c[1].min(c[3]),
            width: (c[2] - c[0]).abs(),
            height: (c[3] - c[1]).abs(),
        };
        if !c.iter().all(|n| n.is_finite())
            || !rect.is_valid()
            || !rect.width.is_finite()
            || !rect.height.is_finite()
        {
            continue;
        }
        let destination = dict.get(b"Dest").ok().or_else(|| {
            let action = resolve(doc, dict.get(b"A").ok()?)?.as_dict().ok()?;
            (action.get(b"S").ok()?.as_name().ok()? == b"GoTo")
                .then(|| action.get(b"D").ok())
                .flatten()
        });
        let Some(target) = destination.and_then(|o| destination_page(doc, pages, o, 0)) else {
            continue;
        };
        links.push(PdfInternalLink {
            rect,
            target_page: target,
        });
    }
    Ok(links)
}

fn destination_page(
    doc: &lopdf::Document,
    pages: &[lopdf::ObjectId],
    object: &lopdf::Object,
    depth: usize,
) -> Option<usize> {
    if depth >= 32 {
        return None;
    }
    match resolve(doc, object)? {
        lopdf::Object::Array(items) => {
            items.get(1)?.as_name().ok()?;
            items
                .first()?
                .as_reference()
                .ok()
                .and_then(|id| pages.iter().position(|page| *page == id))
        }
        lopdf::Object::Dictionary(dict) => {
            destination_page(doc, pages, dict.get(b"D").ok()?, depth + 1)
        }
        lopdf::Object::Name(name) | lopdf::Object::String(name, _) => {
            let catalog = doc.catalog().ok()?;
            let legacy = catalog
                .get(b"Dests")
                .ok()
                .and_then(|o| resolve(doc, o))
                .and_then(|o| o.as_dict().ok())
                .and_then(|dict| dict.get(name).ok());
            let dest = legacy.or_else(|| {
                let names = resolve(doc, catalog.get(b"Names").ok()?)?.as_dict().ok()?;
                named_destination(doc, names.get(b"Dests").ok()?, name)
            })?;
            destination_page(doc, pages, dest, depth + 1)
        }
        _ => None,
    }
}

fn named_destination<'a>(
    doc: &'a lopdf::Document,
    root: &'a lopdf::Object,
    name: &[u8],
) -> Option<&'a lopdf::Object> {
    let mut pending = vec![root];
    let mut visited = std::collections::HashSet::new();
    for _ in 0..4096 {
        let node = pending.pop()?;
        if let lopdf::Object::Reference(id) = node
            && !visited.insert(*id)
        {
            continue;
        }
        let Some(dict) = resolve(doc, node).and_then(|o| o.as_dict().ok()) else {
            continue;
        };
        if let Some(entries) = dict
            .get(b"Names")
            .ok()
            .and_then(|o| resolve(doc, o))
            .and_then(|o| o.as_array().ok())
        {
            for pair in entries.as_chunks::<2>().0 {
                if pair[0].as_str().ok() == Some(name) {
                    return Some(&pair[1]);
                }
            }
        }
        if let Some(kids) = dict
            .get(b"Kids")
            .ok()
            .and_then(|o| resolve(doc, o))
            .and_then(|o| o.as_array().ok())
        {
            pending.extend(kids.iter().take(4096usize.saturating_sub(pending.len())));
        }
    }
    None
}

fn resolve<'a>(
    doc: &'a lopdf::Document,
    mut object: &'a lopdf::Object,
) -> Option<&'a lopdf::Object> {
    for _ in 0..32 {
        match object {
            lopdf::Object::Reference(id) => object = doc.objects.get(id)?,
            _ => return Some(object),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{Document, Object, ObjectId, dictionary};

    fn fixture() -> (Document, [ObjectId; 2]) {
        let mut doc = Document::with_version("1.7");
        let pages = doc.new_object_id();
        let ids = [
            doc.add_object(dictionary! {"Type" => "Page", "Parent" => pages, "MediaBox" => vec![0.into(), 0.into(), 600.into(), 800.into()]}),
            doc.add_object(dictionary! {"Type" => "Page", "Parent" => pages, "MediaBox" => vec![0.into(), 0.into(), 600.into(), 800.into()]}),
        ];
        doc.objects.insert(pages, Object::Dictionary(dictionary! {"Type" => "Pages", "Kids" => ids.iter().copied().map(Object::Reference).collect::<Vec<_>>(), "Count" => 2}));
        let root = doc.add_object(dictionary! {"Type" => "Catalog", "Pages" => pages});
        doc.trailer.set("Root", root);
        (doc, ids)
    }

    fn destination(page: ObjectId) -> Object {
        Object::Array(vec![Object::Reference(page), Object::Name(b"Fit".to_vec())])
    }

    fn annotation(dest: Object) -> Object {
        Object::Dictionary(dictionary! {
            "Subtype" => "Link", "Rect" => vec![10.into(), 20.into(), 50.into(), 60.into()], "Dest" => dest
        })
    }

    fn attach(doc: &mut Document, page: ObjectId, annotations: Vec<Object>) {
        doc.get_object_mut(page)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Annots", annotations);
    }

    fn extract(doc: &mut Document, page_index: usize) -> Result<Vec<PdfInternalLink>, PdfError> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("links.pdf");
        doc.save(&path).unwrap();
        extract_internal_links(&path, page_index)
    }

    #[test]
    fn index_reuses_loaded_document_without_reading_path_again() {
        let (mut doc, ids) = fixture();
        attach(&mut doc, ids[0], vec![annotation(destination(ids[1]))]);
        attach(&mut doc, ids[1], vec![annotation(destination(ids[0]))]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("links.pdf");
        doc.save(&path).unwrap();
        let index = InternalLinkIndex::load(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        for _ in 0..3 {
            assert_eq!(index.links(0).unwrap()[0].target_page, 1);
            assert_eq!(index.links(1).unwrap()[0].target_page, 0);
        }
        assert!(index.links(usize::MAX).is_err());
    }

    #[test]
    fn skips_cyclic_and_dangling_references_without_losing_valid_links() {
        let (mut doc, ids) = fixture();
        let cycle = doc.new_object_id();
        doc.objects.insert(cycle, Object::Reference(cycle));
        doc.catalog_mut().unwrap().set(
            "Dests",
            dictionary! {"loop" => Object::Name(b"loop".to_vec())},
        );
        let tree = doc.new_object_id();
        doc.objects.insert(
            tree,
            Object::Dictionary(dictionary! {"Kids" => vec![Object::Reference(tree)]}),
        );
        doc.catalog_mut()
            .unwrap()
            .set("Names", dictionary! {"Dests" => tree});
        attach(
            &mut doc,
            ids[0],
            vec![
                Object::Reference(cycle),
                Object::Reference((9999, 0)),
                annotation(Object::Name(b"loop".to_vec())),
                annotation(Object::string_literal("missing")),
                annotation(Object::Reference(cycle)),
                annotation(destination(ids[1])),
            ],
        );
        assert_eq!(extract(&mut doc, 0).unwrap().len(), 1);
    }

    #[test]
    fn ignores_invalid_destinations_and_non_link_annotations() {
        let (mut doc, ids) = fixture();
        let mut non_link = annotation(destination(ids[1]));
        non_link.as_dict_mut().unwrap().set("Subtype", "Text");
        attach(
            &mut doc,
            ids[0],
            vec![
                annotation(Object::Array(vec![Object::Reference(ids[1])])),
                annotation(destination((9999, 0))),
                annotation(Object::Array(vec![
                    Object::Integer(99),
                    Object::Name(b"Fit".to_vec()),
                ])),
                annotation(Object::Name(b"missing".to_vec())),
                non_link,
                annotation(destination(ids[1])),
            ],
        );
        assert_eq!(extract(&mut doc, 0).unwrap().len(), 1);
    }

    #[test]
    fn rejects_out_of_range_page_indexes_without_overflow() {
        let (mut doc, _) = fixture();
        for index in [2, usize::MAX] {
            assert!(matches!(extract(&mut doc, index), Err(PdfError::Load(_))));
        }
        assert!(extract(&mut doc, 0).unwrap().is_empty());
    }

    #[test]
    fn validates_paths_and_reports_load_errors() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            extract_internal_links(&dir.path().join("missing.pdf"), 0),
            Err(PdfError::MissingFile(_))
        ));
        let text = dir.path().join("not.txt");
        std::fs::write(&text, "not pdf").unwrap();
        assert!(matches!(
            extract_internal_links(&text, 0),
            Err(PdfError::NotPdf(_))
        ));
        let pdf = dir.path().join("broken.pdf");
        std::fs::write(&pdf, "not pdf").unwrap();
        assert!(matches!(
            extract_internal_links(&pdf, 0),
            Err(PdfError::Load(_))
        ));
    }

    #[test]
    fn resolves_named_destinations_from_legacy_dictionary_and_name_tree() {
        let (mut doc, ids) = fixture();
        let legacy = doc.add_object(dictionary! {"chapter" => destination(ids[1])});
        doc.catalog_mut().unwrap().set("Dests", legacy);
        let leaf = doc.add_object(dictionary! {"Names" => vec![Object::string_literal("section"), Object::Dictionary(dictionary! {"D" => destination(ids[0])})]});
        let tree = doc.add_object(dictionary! {"Kids" => vec![Object::Reference(leaf)]});
        let names = doc.add_object(dictionary! {"Dests" => tree});
        doc.catalog_mut().unwrap().set("Names", names);
        let direct = annotation(Object::Name(b"chapter".to_vec()));
        let mut action = annotation(Object::Null);
        action.as_dict_mut().unwrap().remove(b"Dest");
        action.as_dict_mut().unwrap().set(
            "A",
            dictionary! {"S" => "GoTo", "D" => Object::string_literal("section")},
        );
        attach(&mut doc, ids[0], vec![direct, action]);
        assert_eq!(
            extract(&mut doc, 0)
                .unwrap()
                .iter()
                .map(|l| l.target_page)
                .collect::<Vec<_>>(),
            vec![1, 0]
        );
    }

    #[test]
    fn rejects_malformed_rectangles_and_normalizes_reversed_corners() {
        let (mut doc, ids) = fixture();
        let rects = vec![
            vec![0.into(), 0.into(), 0.into(), 5.into()],
            vec![0.into(), 0.into(), 5.into()],
            vec![0.into(), Object::Null, 5.into(), 5.into()],
            vec![50.into(), 60.into(), 10.into(), 20.into()],
        ];
        let annotations = rects
            .into_iter()
            .map(|rect| {
                let mut link = annotation(destination(ids[1]));
                link.as_dict_mut().unwrap().set("Rect", rect);
                link
            })
            .collect();
        attach(&mut doc, ids[0], annotations);
        let links = extract(&mut doc, 0).unwrap();
        assert_eq!(
            links,
            vec![PdfInternalLink {
                rect: PdfRect {
                    x: 10.0,
                    y: 20.0,
                    width: 40.0,
                    height: 40.0
                },
                target_page: 1
            }]
        );
    }

    #[test]
    fn follows_indirect_annotation_arrays_rects_actions_and_destinations() {
        let (mut doc, ids) = fixture();
        let dest = doc.add_object(destination(ids[1]));
        let action = doc.add_object(dictionary! {"S" => "GoTo", "D" => dest});
        let rect = doc.add_object(Object::Array(vec![
            10.into(),
            20.into(),
            50.into(),
            60.into(),
        ]));
        let link = doc.add_object(dictionary! {"Subtype" => "Link", "Rect" => rect, "A" => action});
        let annots = doc.add_object(Object::Array(vec![Object::Reference(link)]));
        doc.get_object_mut(ids[0])
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Annots", annots);
        assert_eq!(extract(&mut doc, 0).unwrap().len(), 1);
    }

    #[test]
    fn extracts_goto_action_but_ignores_external_actions() {
        let (mut doc, ids) = fixture();
        let mut annotations = Vec::new();
        for kind in ["GoTo", "URI", "GoToR", "Launch"] {
            let mut link = annotation(destination(ids[1]));
            let dict = link.as_dict_mut().unwrap();
            dict.remove(b"Dest");
            dict.set("A", dictionary! {"S" => kind, "D" => destination(ids[1]), "URI" => Object::string_literal("https://example.com")});
            annotations.push(link);
        }
        attach(&mut doc, ids[0], annotations);
        let links = extract(&mut doc, 0).unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target_page, 1);
    }

    #[test]
    fn extracts_direct_destination_in_pdf_coordinates() {
        let (mut doc, ids) = fixture();
        let link = doc.add_object(annotation(destination(ids[1])));
        attach(&mut doc, ids[0], vec![Object::Reference(link)]);
        assert_eq!(
            extract(&mut doc, 0).unwrap(),
            vec![PdfInternalLink {
                rect: PdfRect {
                    x: 10.0,
                    y: 20.0,
                    width: 40.0,
                    height: 40.0
                },
                target_page: 1
            }]
        );
    }
}
