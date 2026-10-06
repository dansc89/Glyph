use super::PdfError;
#[cfg(test)]
use lopdf::dictionary;
use lopdf::{Document, Object, ObjectId};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

fn fingerprint(path: &Path) -> Result<[u8; 32], PdfError> {
    let mut file = std::fs::File::open(path).map_err(edit_error)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(edit_error)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(hash.finalize().into())
}

/// A stable, zero-based outline index for the lifetime of an edit session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditableBookmark {
    pub object_id: ObjectId,
    pub index: usize,
    pub title: String,
    pub page_index: Option<usize>,
    pub depth: usize,
}
struct Entry {
    id: ObjectId,
    title: String,
    page_index: Option<usize>,
    depth: usize,
}
struct Change {
    index: usize,
    before: Object,
    after: Object,
    before_title: String,
    after_title: String,
}
enum HistoryChange {
    Shape {
        page: ObjectId,
        before: Option<Object>,
        after: Option<Object>,
    },
    Title(Box<Change>),
    PageLabel {
        index: usize,
        before: String,
        after: String,
    },
}
/// In-memory bookmark-title, page-label and native shape edits to an existing PDF.
///
/// Encrypted PDFs and signature-bearing documents are rejected, not rewritten
/// with an assertion that their security would survive. Malformed page-label
/// trees and malformed annotation/page geometry also fail closed on open.
/// Bounded shared history stores title, label and annotation-array changes;
/// successful saves retain history and advance all editable-state
/// checkpoints. `save_as()` exports without altering the old source.
///
/// Save requires Linux atomic rename exchange (unsupported systems/filesystems
/// fail closed). Each exchange leaves the displaced source inode at a unique
/// sibling `.glyph-backup-*.pdf` path permanently, even on success; the caller
/// can discover the most recent one with `last_backup_path()`. Backups are never
/// automatically deleted: writers holding old file descriptors may continue
/// writing to them after Save returns.
///
/// Validation occurs before staging and after exchange on the displaced source.
/// A detected commit race reports a conflict and its backup path, retaining the
/// dirty checkpoint; the edited PDF may already be visible at the source path.
/// There is deliberately no rollback that could overwrite a third writer.
/// This preserves displaced data, not mutual exclusion: later non-cooperating
/// writes may still change either inode, and writes after verification cannot be
/// detected retroactively. It assumes the directory/backup names are not being
/// maliciously removed or replaced. Atomic visibility is not a directory-fsync
/// power-loss durability guarantee. Save As uses no-clobber persistence.
pub struct EditablePdf {
    doc: Document,
    source_hash: [u8; 32],
    permissions: std::fs::Permissions,
    path: PathBuf,
    last_backup: Option<PathBuf>,
    entries: Vec<Entry>,
    checkpoint: Vec<String>,
    labels: Vec<String>,
    initial_labels: Vec<String>,
    label_checkpoint: Vec<String>,
    shape_checkpoint: Vec<super::ShapeAnnotation>,
    original_page_labels: Option<Object>,
    undo: Vec<HistoryChange>,
    redo: Vec<HistoryChange>,
}
fn edit_error(e: impl std::fmt::Display) -> PdfError {
    PdfError::Edit(e.to_string())
}
fn unicode_title(title: &str) -> Object {
    let mut bytes = vec![0xfe, 0xff];
    for unit in title.encode_utf16() {
        bytes.extend(unit.to_be_bytes());
    }
    Object::String(bytes, lopdf::StringFormat::Hexadecimal)
}
fn resolve<'a>(doc: &'a Document, mut object: &'a Object) -> Option<&'a Object> {
    let mut seen = std::collections::HashSet::new();
    while let Object::Reference(id) = object {
        if !seen.insert(*id) || seen.len() > 256 {
            return None;
        }
        object = doc.get_object(*id).ok()?;
    }
    Some(object)
}
fn target_page(
    doc: &Document,
    item: &lopdf::Dictionary,
    pages: &std::collections::HashMap<ObjectId, usize>,
) -> Option<usize> {
    let dest = if let Ok(dest) = item.get(b"Dest") {
        dest
    } else {
        let action = resolve(doc, item.get(b"A").ok()?)?.as_dict().ok()?;
        if resolve(doc, action.get(b"S").ok()?)?.as_name().ok()? != b"GoTo" {
            return None;
        }
        action.get(b"D").ok()?
    };
    let dest = resolve(doc, dest)?;
    let first = dest.as_array().ok()?.first()?;
    pages.get(&first.as_reference().ok()?).copied()
}
fn entries(doc: &Document) -> Result<Vec<Entry>, PdfError> {
    let pages: std::collections::HashMap<_, _> = doc
        .get_pages()
        .values()
        .enumerate()
        .map(|(i, id)| (*id, i))
        .collect();
    let catalog = doc.catalog().map_err(edit_error)?;
    let Some(outline) = catalog.get(b"Outlines").ok() else {
        return Ok(vec![]);
    };
    let root = doc
        .get_object(outline.as_reference().map_err(edit_error)?)
        .and_then(Object::as_dict)
        .map_err(edit_error)?;
    let mut stack = Vec::new();
    if let Ok(first) = root.get(b"First") {
        stack.push((first.as_reference().map_err(edit_error)?, 0));
    }
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    while let Some((id, depth)) = stack.pop() {
        if !seen.insert(id) || seen.len() > 10000 || depth > 256 {
            return Err(edit_error("cyclic or oversized outline"));
        }
        let item = doc
            .get_object(id)
            .and_then(Object::as_dict)
            .map_err(edit_error)?;
        let title_object = resolve(doc, item.get(b"Title").map_err(edit_error)?)
            .ok_or_else(|| edit_error("cyclic or invalid bookmark title reference"))?;
        let title = lopdf::decode_text_string(title_object).map_err(edit_error)?;
        let page_index = target_page(doc, item, &pages).filter(|i| *i < pages.len());
        result.push(Entry {
            id,
            title,
            page_index,
            depth,
        });
        if let Ok(next) = item.get(b"Next") {
            stack.push((next.as_reference().map_err(edit_error)?, depth));
        }
        if let Ok(first) = item.get(b"First") {
            stack.push((first.as_reference().map_err(edit_error)?, depth + 1));
        }
    }
    Ok(result)
}
fn reject_security(doc: &Document) -> Result<(), PdfError> {
    if doc.trailer.has(b"Encrypt") || doc.was_encrypted() {
        return Err(edit_error("encrypted PDFs cannot be edited safely"));
    }
    // Scan direct nested values and every indirect object, including orphaned
    // signature dictionaries and inherited /FT /Sig field declarations.
    let mut stack: Vec<(&Object, usize)> = doc.objects.values().map(|o| (o, 0)).collect();
    for (_, value) in doc.trailer.iter() {
        stack.push((value, 0));
    }
    while let Some((object, depth)) = stack.pop() {
        if depth > 256 {
            return Err(edit_error("security inspection nesting limit exceeded"));
        }
        match object {
            Object::Name(name)
                if [b"Sig".as_slice(), b"DocMDP", b"FieldMDP"].contains(&name.as_slice()) =>
            {
                return Err(edit_error(
                    "signed PDFs or signature fields cannot be edited safely",
                ));
            }
            Object::Array(array) => stack.extend(array.iter().map(|o| (o, depth + 1))),
            Object::Dictionary(dict) | Object::Stream(lopdf::Stream { dict, .. }) => {
                if dict.has(b"ByteRange") || dict.has(b"SigFlags") {
                    return Err(edit_error(
                        "signed PDFs or signature fields cannot be edited safely",
                    ));
                }
                stack.extend(dict.iter().map(|(_, value)| (value, depth + 1)));
            }
            _ => {}
        }
    }
    Ok(())
}
// Directional comparison of an in-memory value against its serialized reload.
// lopdf writes Real via f32 Display: an integral token like "612" reloads as
// Integer. Accept only that exact emitted i64 text, never a float cast of an
// original Integer (which would hide neighboring integers above 2^24).
fn serialized_object_equal(original: &Object, reloaded: &Object) -> bool {
    match (original, reloaded) {
        (Object::Real(value), Object::Integer(integer)) => {
            value.is_finite() && value.to_string().parse::<i64>().ok() == Some(*integer)
        }
        (Object::Array(a), Object::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| serialized_object_equal(a, b))
        }
        (Object::Dictionary(a), Object::Dictionary(b)) => serialized_dictionary_equal(a, b),
        (Object::Stream(a), Object::Stream(b)) => {
            serialized_dictionary_equal(&a.dict, &b.dict) && a.content == b.content
        }
        _ => original == reloaded,
    }
}

fn serialized_dictionary_equal(a: &lopdf::Dictionary, b: &lopdf::Dictionary) -> bool {
    a.len() == b.len()
        && a.iter().all(|(key, value)| {
            b.get(key)
                .is_ok_and(|other| serialized_object_equal(value, other))
        })
}

impl EditablePdf {
    pub fn add_ellipse(
        &mut self,
        page: usize,
        rect: crate::core::links::PdfRect,
    ) -> Result<bool, PdfError> {
        self.add_shape(page, rect, super::ShapeKind::Ellipse)
    }
    /// Both native owned shapes, including annotations read from persisted PDFs.
    pub fn shapes(&self) -> Vec<super::ShapeAnnotation> {
        super::rectangles::read(&self.doc).expect("validated shapes")
    }
    /// Compatibility API returning rectangles only.
    pub fn rectangles(&self) -> Vec<super::RectangleAnnotation> {
        self.shapes()
            .into_iter()
            .filter(|a| a.kind == super::ShapeKind::Rectangle)
            .collect()
    }
    pub fn add_rectangle(
        &mut self,
        page: usize,
        rect: crate::core::links::PdfRect,
    ) -> Result<bool, PdfError> {
        self.add_shape(page, rect, super::ShapeKind::Rectangle)
    }
    pub fn add_shape(
        &mut self,
        page: usize,
        rect: crate::core::links::PdfRect,
        kind: super::ShapeKind,
    ) -> Result<bool, PdfError> {
        let page_id = self
            .doc
            .get_pages()
            .values()
            .nth(page)
            .copied()
            .ok_or_else(|| edit_error("page index out of range"))?;
        let (before, mut annots) = super::rectangles::annots(&self.doc, page_id)?;
        let id = super::rectangles::create(&mut self.doc, page_id, rect, kind)?;
        annots.push(Object::Reference(id));
        let change = HistoryChange::Shape {
            page: page_id,
            before,
            after: Some(Object::Array(annots)),
        };
        self.apply(&change, true);
        self.push_change(change);
        Ok(true)
    }
    /// Removes only a currently attached Glyph rectangle; unknown IDs are no-ops.
    pub fn delete_rectangle(&mut self, object_id: ObjectId) -> Result<bool, PdfError> {
        if !self.rectangles().iter().any(|a| a.object_id == object_id) {
            return Ok(false);
        }
        self.delete_shape(object_id)
    }
    /// Removes either currently attached Glyph shape; foreign/detached IDs are no-ops.
    pub fn delete_shape(&mut self, object_id: ObjectId) -> Result<bool, PdfError> {
        let Some(rectangle) = self.shapes().into_iter().find(|r| r.object_id == object_id) else {
            return Ok(false);
        };
        let page = *self
            .doc
            .get_pages()
            .values()
            .nth(rectangle.page_index)
            .unwrap();
        let (before, mut annots) = super::rectangles::annots(&self.doc, page)?;
        annots.retain(|o| o.as_reference().ok() != Some(object_id));
        let change = HistoryChange::Shape {
            page,
            before,
            after: Some(Object::Array(annots)),
        };
        self.apply(&change, true);
        self.push_change(change);
        Ok(true)
    }
    /// Serializes a detached snapshot without changing source, history or checkpoint.
    /// Output is capped at 256 MiB while writing, not after an unbounded allocation.
    pub fn render_snapshot(&self) -> Result<Vec<u8>, PdfError> {
        self.snapshot_with_limit(256 * 1024 * 1024)
    }
    fn snapshot_with_limit(&self, limit: usize) -> Result<Vec<u8>, PdfError> {
        struct Capped {
            bytes: Vec<u8>,
            limit: usize,
        }
        impl std::io::Write for Capped {
            fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
                if data.len() > self.limit.saturating_sub(self.bytes.len()) {
                    return Err(std::io::Error::other("render snapshot exceeds size limit"));
                }
                self.bytes.extend_from_slice(data);
                Ok(data.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut output = Capped {
            bytes: Vec::new(),
            limit,
        };
        self.doc.clone().save_to(&mut output).map_err(edit_error)?;
        Ok(output.bytes)
    }
    pub const MAX_BOOKMARK_TITLE_CHARS: usize = 4096;
    pub fn open(path: &Path) -> Result<Self, PdfError> {
        let source_hash = fingerprint(path)?;
        let doc = Document::load(path).map_err(|e| PdfError::Load(e.to_string()))?;
        if fingerprint(path)? != source_hash {
            return Err(edit_error("source changed externally while opening"));
        }
        reject_security(&doc)?;
        let shape_checkpoint = super::rectangles::read(&doc)?;
        let entries = entries(&doc)?;
        let checkpoint = entries.iter().map(|e| e.title.clone()).collect();
        let labels = super::page_labels::read(&doc)?;
        let original_page_labels = doc
            .catalog()
            .map_err(edit_error)?
            .get(b"PageLabels")
            .ok()
            .cloned();
        Ok(Self {
            doc,
            source_hash,
            permissions: std::fs::metadata(path).map_err(edit_error)?.permissions(),
            path: path.to_owned(),
            last_backup: None,
            entries,
            checkpoint,
            initial_labels: labels.clone(),
            label_checkpoint: labels.clone(),
            labels,
            original_page_labels,
            shape_checkpoint,
            undo: vec![],
            redo: vec![],
        })
    }
    pub fn bookmarks(&self) -> Vec<EditableBookmark> {
        self.entries
            .iter()
            .enumerate()
            .map(|(index, e)| EditableBookmark {
                object_id: e.id,
                index,
                title: e.title.clone(),
                page_index: e.page_index,
                depth: e.depth,
            })
            .collect()
    }
    pub const MAX_PAGE_LABEL_CHARS: usize = super::MAX_PAGE_LABEL_CHARS;
    /// Effective labels in physical page order (numeric defaults included).
    pub fn page_labels(&self) -> Vec<String> {
        self.labels.clone()
    }
    /// Set one page's literal Unicode label. Trims surrounding whitespace;
    /// rejects empty/oversized input. A no-op leaves both history stacks intact.
    pub fn set_page_label(&mut self, page_index: usize, label: &str) -> Result<bool, PdfError> {
        let label = label.trim();
        if label.is_empty()
            || label.chars().take(Self::MAX_PAGE_LABEL_CHARS + 1).count()
                > Self::MAX_PAGE_LABEL_CHARS
        {
            return Err(edit_error("page label must contain 1–256 characters"));
        }
        let before = self
            .labels
            .get(page_index)
            .ok_or_else(|| edit_error("page index out of range"))?;
        if before == label {
            return Ok(false);
        }
        let change = HistoryChange::PageLabel {
            index: page_index,
            before: before.clone(),
            after: label.to_owned(),
        };
        self.apply(&change, true);
        self.push_change(change);
        Ok(true)
    }
    pub fn rename_bookmark(&mut self, index: usize, title: &str) -> Result<bool, PdfError> {
        let title = title.trim();
        if title.is_empty()
            || title
                .chars()
                .take(Self::MAX_BOOKMARK_TITLE_CHARS + 1)
                .count()
                > Self::MAX_BOOKMARK_TITLE_CHARS
        {
            return Err(edit_error("bookmark title must contain 1–4096 characters"));
        }
        let entry = self
            .entries
            .get(index)
            .ok_or_else(|| edit_error("bookmark index out of range"))?;
        if title == entry.title {
            return Ok(false);
        }
        let item = self
            .doc
            .get_object_mut(entry.id)
            .and_then(Object::as_dict_mut)
            .map_err(edit_error)?;
        let change = Change {
            index,
            before: item.get(b"Title").map_err(edit_error)?.clone(),
            after: unicode_title(title),
            before_title: entry.title.clone(),
            after_title: title.to_owned(),
        };
        item.set("Title", change.after.clone());
        self.entries[index].title = title.to_owned();
        self.push_change(HistoryChange::Title(Box::new(change)));
        Ok(true)
    }
    fn push_change(&mut self, change: HistoryChange) {
        self.undo.push(change);
        if self.undo.len() > 64 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    pub fn is_dirty(&self) -> bool {
        self.entries
            .iter()
            .map(|e| &e.title)
            .ne(self.checkpoint.iter())
            || self.labels != self.label_checkpoint
            || self.shapes() != self.shape_checkpoint
    }
    fn apply(&mut self, change: &HistoryChange, forward: bool) {
        let c = match change {
            HistoryChange::Shape {
                page,
                before,
                after,
            } => {
                let value = if forward { after } else { before };
                let dict = self
                    .doc
                    .get_object_mut(*page)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap();
                match value {
                    Some(o) => dict.set("Annots", o.clone()),
                    None => {
                        dict.remove(b"Annots");
                    }
                }
                return;
            }
            HistoryChange::Title(c) => c,
            HistoryChange::PageLabel {
                index,
                before,
                after,
            } => {
                self.labels[*index] = if forward {
                    after.clone()
                } else {
                    before.clone()
                };
                self.rebuild_page_labels();
                return;
            }
        };
        let item = self
            .doc
            .get_object_mut(self.entries[c.index].id)
            .unwrap()
            .as_dict_mut()
            .unwrap();
        item.set(
            "Title",
            if forward {
                c.after.clone()
            } else {
                c.before.clone()
            },
        );
        self.entries[c.index].title = if forward {
            c.after_title.clone()
        } else {
            c.before_title.clone()
        };
    }
    fn rebuild_page_labels(&mut self) {
        // The original catalog value is retained once, and its referenced tree
        // objects are never modified. Direct replacement avoids orphan growth.
        let replacement = if self.labels == self.initial_labels {
            self.original_page_labels.clone()
        } else {
            let mut nums = Vec::with_capacity(self.labels.len() * 2);
            for (index, label) in self.labels.iter().enumerate() {
                nums.push(Object::Integer(index as i64));
                let mut range = lopdf::Dictionary::new();
                range.set("P", unicode_title(label));
                nums.push(Object::Dictionary(range));
            }
            let mut tree = lopdf::Dictionary::new();
            tree.set("Nums", nums);
            Some(Object::Dictionary(tree))
        };
        let catalog = self.doc.catalog_mut().expect("validated editable catalog");
        match replacement {
            Some(value) => catalog.set("PageLabels", value),
            None => {
                catalog.remove(b"PageLabels");
            }
        }
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo(&mut self) -> bool {
        let Some(c) = self.undo.pop() else {
            return false;
        };
        self.apply(&c, false);
        self.redo.push(c);
        true
    }
    pub fn redo(&mut self) -> bool {
        let Some(c) = self.redo.pop() else {
            return false;
        };
        self.apply(&c, true);
        self.undo.push(c);
        true
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// The permanent displaced-source backup from the most recent exchange,
    /// including an exchange that reported a conflict. Pre-commit failures and
    /// Save As leave this value unchanged. Older backups also remain on disk.
    pub fn last_backup_path(&self) -> Option<&Path> {
        self.last_backup.as_deref()
    }
    fn stage(&mut self, path: &Path) -> Result<tempfile::NamedTempFile, PdfError> {
        let expected_shapes = super::rectangles::read(&self.doc)?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut temp = tempfile::Builder::new()
            .prefix(".glyph-backup-")
            .suffix(".pdf")
            .tempfile_in(parent)
            .map_err(edit_error)?;
        self.doc.save_to(temp.as_file_mut()).map_err(edit_error)?;
        temp.as_file().sync_all().map_err(edit_error)?;
        let saved = Document::load(temp.path()).map_err(edit_error)?;
        if super::rectangles::read(&saved)? != expected_shapes {
            return Err(edit_error("staged PDF shape verification failed"));
        }
        if saved.get_pages() != self.doc.get_pages() {
            return Err(edit_error("staged PDF page verification failed"));
        }
        let saved_entries = entries(&saved)?;
        if super::page_labels::read(&saved)? != self.labels {
            return Err(edit_error("staged PDF page label verification failed"));
        }
        if saved_entries.len() != self.entries.len()
            || saved_entries.iter().zip(&self.entries).any(|(a, b)| {
                a.id != b.id
                    || a.title != b.title
                    || a.depth != b.depth
                    || a.page_index != b.page_index
            })
        {
            return Err(edit_error("staged PDF outline verification failed"));
        }
        for (id, object) in &self.doc.objects {
            // These are serialization scaffolding, not document content.
            if object
                .type_name()
                .ok()
                .is_some_and(|t| [b"XRef".as_slice(), b"ObjStm", b"Linearized"].contains(&t))
            {
                continue;
            }
            let other = saved.get_object(*id).map_err(edit_error)?;
            if !serialized_object_equal(object, other) {
                return Err(edit_error(format!(
                    "staged PDF object {id:?} changed unexpectedly"
                )));
            }
        }
        Ok(temp)
    }
    fn checkpoint(&mut self) {
        self.checkpoint = self.entries.iter().map(|e| e.title.clone()).collect();
        self.label_checkpoint.clone_from(&self.labels);
        self.shape_checkpoint = self.shapes();
    }
    fn check_source(&self) -> Result<(), PdfError> {
        let metadata = std::fs::symlink_metadata(&self.path).map_err(edit_error)?;
        if metadata.file_type().is_symlink() {
            return Err(edit_error("refusing to overwrite a symlink"));
        }
        if !metadata.is_file() || metadata.permissions().readonly() {
            return Err(edit_error(
                "source is not a regular writable file (read-only)",
            ));
        }
        if fingerprint(&self.path)? != self.source_hash {
            return Err(edit_error("source changed externally; refusing overwrite"));
        }
        Ok(())
    }
    pub fn save(&mut self) -> Result<(), PdfError> {
        self.save_before_commit(|| {})
    }
    fn save_before_commit(&mut self, before_commit: impl FnOnce()) -> Result<(), PdfError> {
        let path = self.path.clone();
        self.check_source()?;
        let temp = self.stage(&path)?;
        let next_hash = fingerprint(temp.path())?;
        self.check_source()?;
        let permissions = std::fs::metadata(&path).map_err(edit_error)?.permissions();
        temp.as_file()
            .set_permissions(permissions.clone())
            .map_err(edit_error)?;
        temp.as_file().sync_all().map_err(edit_error)?;
        // Disarm temporary-file cleanup BEFORE the exchange: after it succeeds
        // this name owns the displaced source, including writers' open inodes.
        let (_staged_file, backup) = temp.keep().map_err(edit_error)?;
        // Private deterministic test seam, immediately before filesystem commit.
        before_commit();
        #[cfg(target_os = "linux")]
        let exchange = rustix::fs::renameat_with(
            rustix::fs::CWD,
            &backup,
            rustix::fs::CWD,
            &path,
            rustix::fs::RenameFlags::EXCHANGE,
        )
        .map_err(edit_error);
        #[cfg(not(target_os = "linux"))]
        let exchange: Result<(), PdfError> = Err(edit_error(
            "safe overwrite requires Linux atomic exchange; use Save As",
        ));
        if let Err(error) = exchange {
            // No exchange occurred; this is still our staged PDF, not a source.
            let cleanup = std::fs::remove_file(&backup);
            return Err(edit_error(format!(
                "atomic exchange failed (source not replaced): {error}; staged path {} (cleanup: {cleanup:?})",
                backup.display()
            )));
        }
        self.last_backup = Some(backup.clone());
        let displaced_check = (|| {
            let metadata = std::fs::symlink_metadata(&backup).map_err(edit_error)?;
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.permissions().readonly()
            {
                return Err(edit_error(
                    "displaced source is not a regular writable file",
                ));
            }
            if fingerprint(&backup)? != self.source_hash {
                return Err(edit_error("source changed externally"));
            }
            Ok(())
        })();
        if let Err(error) = displaced_check {
            // Never roll back over a third writer. The edited file may already
            // be visible; keep the checkpoint dirty and preserve BOTH names.
            return Err(edit_error(format!(
                "save conflict after atomic exchange: {error}; displaced source retained permanently at {}; edited PDF may be visible at {}; no rollback attempted",
                backup.display(),
                path.display()
            )));
        }
        self.permissions = permissions;
        self.source_hash = next_hash;
        self.checkpoint();
        Ok(())
    }
    pub fn save_as(&mut self, path: &Path) -> Result<(), PdfError> {
        let temp = self.stage(path)?;
        let next_hash = fingerprint(temp.path())?;
        temp.as_file()
            .set_permissions(self.permissions.clone())
            .map_err(edit_error)?;
        temp.as_file().sync_all().map_err(edit_error)?;
        temp.persist_noclobber(path).map_err(edit_error)?;
        self.path = path.to_owned();
        self.source_hash = next_hash;
        self.checkpoint();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Owned synthetic component fixture: no user documents, rendering or GUI.
    fn synthetic_hundred_page_fixture(path: &Path) {
        let mut d = Document::with_version("1.7");
        let pages = d.new_object_id();
        let font = d.add_object(dictionary! {
            "Type"=>"Font", "Subtype"=>"Type1", "BaseFont"=>"Helvetica"
        });
        let resources = d.add_object(dictionary! {
            "Font"=>dictionary! {"F1"=>font},
            "ProcSet"=>vec![Object::Name(b"PDF".to_vec()), Object::Name(b"Text".to_vec())]
        });
        let mut kids = Vec::new();
        for page in 0..100 {
            let mut content = format!(
                "q 0.8 G 40 50 520 680 re S Q\nBT /F1 16 Tf 50 710 Td (Synthetic drawing sheet {:03}) Tj ET\n",
                page + 1
            );
            // Meaningful distinct text and linework on every page, not empty streams.
            for row in 0..40 {
                content.push_str(&format!(
                    "BT /F1 9 Tf 50 {} Td (Sheet {:03} - detail {:02}: synthetic reliability specimen) Tj ET\nq 0.7 G 50 {} m 550 {} l S Q\n",
                    680 - row * 14, page + 1, row + 1, 674 - row * 14, 674 - row * 14
                ));
            }
            let stream = d.add_object(lopdf::Stream::new(dictionary! {}, content.into_bytes()));
            let p = d.add_object(dictionary! {
                "Type"=>"Page", "Parent"=>pages, "Rotate"=>(page % 4 * 90) as i64,
                "Contents"=>stream
            });
            let foreign = d.add_object(dictionary! {
                "Type"=>"Annot", "Subtype"=>"Text", "Rect"=>vec![50.into(),60.into(),70.into(),80.into()],
                "Contents"=>Object::string_literal(format!("Foreign note {}", page + 1)), "P"=>p
            });
            let link = d.add_object(dictionary! {
                "Type"=>"Annot", "Subtype"=>"Link", "Rect"=>vec![50.into(),700.into(),300.into(),725.into()],
                "A"=>dictionary! {"S"=>"URI", "URI"=>Object::string_literal("https://example.invalid/synthetic")},
                "Border"=>vec![0.into(),0.into(),0.into()], "P"=>p
            });
            d.get_object_mut(p).unwrap().as_dict_mut().unwrap().set(
                "Annots",
                vec![Object::Reference(foreign), Object::Reference(link)],
            );
            kids.push(Object::Reference(p));
        }
        d.objects.insert(
            pages,
            dictionary! {
                "Type"=>"Pages", "Kids"=>kids.clone(), "Count"=>100,
                "MediaBox"=>vec![0.into(),0.into(),612.into(),792.into()],
                "CropBox"=>vec![20.into(),30.into(),592.into(),762.into()], "Resources"=>resources
            }
            .into(),
        );
        let outlines = d.new_object_id();
        let item = d.add_object(dictionary! {
            "Title"=>Object::string_literal("Synthetic original bookmark"), "Parent"=>outlines,
            "Dest"=>vec![kids[0].clone(), Object::Name(b"Fit".to_vec())], "F"=>2
        });
        d.objects.insert(
            outlines,
            dictionary! {
                "Type"=>"Outlines", "First"=>item, "Last"=>item, "Count"=>1
            }
            .into(),
        );
        let labels = d.add_object(dictionary! {
            "Nums"=>vec![Object::Integer(0), Object::Dictionary(dictionary! {
                "S"=>"D", "P"=>Object::string_literal("S-"), "St"=>1
            })]
        });
        let root = d.add_object(dictionary! {
            "Type"=>"Catalog", "Pages"=>pages, "Outlines"=>outlines, "PageLabels"=>labels
        });
        d.trailer.set("Root", root);
        d.save(path).unwrap();
    }

    fn synthetic_assert_reopen(s: &EditablePdf, baseline: &Document) {
        let reopened = EditablePdf::open(s.path()).unwrap();
        assert!(!reopened.is_dirty());
        assert_eq!(
            reopened.shapes(),
            s.shapes(),
            "exact IDs, pages, kinds and geometry"
        );
        assert_eq!(reopened.bookmarks(), s.bookmarks());
        assert_eq!(reopened.page_labels(), s.page_labels());
        assert_eq!(reopened.doc.get_pages(), baseline.get_pages());
        let root = baseline
            .trailer
            .get(b"Root")
            .unwrap()
            .as_reference()
            .unwrap();
        let bookmark = s.bookmarks()[0].object_id;
        let pages: std::collections::HashSet<_> = baseline.get_pages().values().copied().collect();
        for (id, original) in &baseline.objects {
            if original.type_name().ok() == Some(b"XRef".as_slice()) {
                continue;
            }
            let saved = reopened.doc.get_object(*id).unwrap();
            if *id == root || *id == bookmark || pages.contains(id) {
                let mut expected = original.as_dict().unwrap().clone();
                let mut actual = saved.as_dict().unwrap().clone();
                let key = if *id == root {
                    b"PageLabels".as_slice()
                } else if *id == bookmark {
                    b"Title".as_slice()
                } else {
                    b"Annots".as_slice()
                };
                if pages.contains(id) {
                    let old = expected.get(key).unwrap().as_array().unwrap();
                    let new = actual.get(key).unwrap().as_array().unwrap();
                    assert_eq!(&new[..old.len()], old, "foreign note/link still attached");
                    assert!(!actual.has(b"CropBox") && !actual.has(b"Resources"));
                }
                expected.remove(key);
                actual.remove(key);
                assert_eq!(expected, actual, "unchanged dictionary fields {id:?}");
            } else {
                // Includes all original content stream bytes/dictionaries, fonts,
                // inherited crop/resources, link actions and old label tree.
                assert_eq!(original, saved, "unrelated baseline object {id:?}");
            }
        }
    }

    fn synthetic_save_checked(s: &mut EditablePdf, baseline: &Document) -> std::time::Duration {
        let prior = std::fs::read(s.path()).unwrap();
        let history = (s.undo.len(), s.redo.len());
        let start = std::time::Instant::now();
        s.save().unwrap();
        let elapsed = start.elapsed();
        assert!(!s.is_dirty());
        assert_eq!((s.undo.len(), s.redo.len()), history);
        assert_eq!(std::fs::read(s.last_backup_path().unwrap()).unwrap(), prior);
        synthetic_assert_reopen(s, baseline);
        elapsed
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn synthetic_100_page_save_reliability_stress() {
        use crate::core::links::PdfRect;
        use crate::pdf::ShapeKind;
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("synthetic-100-pages.pdf");
        synthetic_hundred_page_fixture(&source);
        let original = std::fs::read(&source).unwrap();
        let baseline = Document::load(&source).unwrap();
        let start = std::time::Instant::now();
        let mut s = EditablePdf::open(&source).unwrap();
        let open_elapsed = start.elapsed();
        let initial_objects = s.doc.objects.len();
        assert_eq!(s.doc.get_pages().len(), 100);
        assert_eq!(s.page_labels()[99], "S-100");
        let start = std::time::Instant::now();
        let mut expected = Vec::new();
        for page in 0..100 {
            for (kind, x) in [(ShapeKind::Rectangle, 0.125), (ShapeKind::Ellipse, 0.5)] {
                let rect = PdfRect {
                    x,
                    y: 0.125 + (page % 4) as f32 * 0.125,
                    width: 0.125,
                    height: 0.125,
                };
                match kind {
                    ShapeKind::Rectangle => assert!(s.add_rectangle(page, rect).unwrap()),
                    ShapeKind::Ellipse => assert!(s.add_ellipse(page, rect).unwrap()),
                }
                expected.push((page, kind, rect));
            }
        }
        let edit_elapsed = start.elapsed();
        assert_eq!(
            s.shapes()
                .iter()
                .map(|a| (a.page_index, a.kind, a.rect))
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(s.rectangles().len(), 100);
        assert_eq!(s.shapes().len(), 200);
        s.rename_bookmark(0, "Synthetic renamed 図面").unwrap();
        s.set_page_label(99, "Synthetic last sheet").unwrap();
        assert_eq!(s.undo.len(), 64);
        assert_eq!(
            s.doc.objects.len(),
            initial_objects + 400,
            "two objects per shape; labels allocate none"
        );
        assert!(s.is_dirty());
        assert_eq!(std::fs::read(&source).unwrap(), original);
        // Exhaust the actual shared API: history, not document growth, is capped.
        let edited = s.shapes();
        for _ in 0..64 {
            assert!(s.undo());
        }
        assert!(!s.undo());
        assert_eq!(s.redo.len(), 64);
        assert_eq!(s.shapes().len(), 138);
        for _ in 0..64 {
            assert!(s.redo());
        }
        assert!(!s.redo());
        assert_eq!(s.shapes(), edited);
        let first_save = synthetic_save_checked(&mut s, &baseline);
        let first_bytes = std::fs::read(&source).unwrap().len();
        let mut sizes = vec![first_bytes];
        let mut timings = Vec::new();
        let mut backups = vec![s.last_backup_path().unwrap().to_owned()];
        for cycle in 0..5 {
            let saved_shapes = s.shapes();
            let saved_bookmarks = s.bookmarks();
            let saved_labels = s.page_labels();
            let prior = std::fs::read(&source).unwrap();
            let count = s.doc.objects.len();
            assert!(s.delete_shape(saved_shapes[cycle].object_id).unwrap());
            assert!(
                s.add_shape(
                    cycle * 19,
                    PdfRect {
                        x: 0.25,
                        y: 0.625,
                        width: 0.25,
                        height: 0.125
                    },
                    if cycle % 2 == 0 {
                        ShapeKind::Ellipse
                    } else {
                        ShapeKind::Rectangle
                    }
                )
                .unwrap()
            );
            s.rename_bookmark(0, &format!("Synthetic cycle {cycle}"))
                .unwrap();
            s.set_page_label(cycle, &format!("Cycle-{cycle}")).unwrap();
            assert_eq!(
                s.doc.objects.len(),
                count + 2,
                "deleted shape objects remain; do not claim constant growth"
            );
            for _ in 0..4 {
                assert!(s.undo());
            }
            assert!(!s.is_dirty(), "shared undo reaches saved checkpoint");
            assert_eq!(s.shapes(), saved_shapes);
            assert_eq!(s.bookmarks(), saved_bookmarks);
            assert_eq!(s.page_labels(), saved_labels);
            for _ in 0..4 {
                assert!(s.redo());
            }
            assert!(s.is_dirty());
            assert_eq!(std::fs::read(&source).unwrap(), prior);
            timings.push(synthetic_save_checked(&mut s, &baseline));
            backups.push(s.last_backup_path().unwrap().to_owned());
            assert_eq!(s.shapes().len(), 200);
            assert_eq!(s.undo.len(), 64);
            assert!(s.undo() && s.is_dirty());
            assert!(s.redo() && !s.is_dirty());
            sizes.push(std::fs::read(&source).unwrap().len());
        }
        assert_eq!(s.doc.objects.len(), initial_objects + 410);
        // Explicit generous byte budget for these five added shapes only, not
        // an assertion that an arbitrary editing lifetime has bounded file size.
        assert!(sizes.iter().all(|n| *n <= first_bytes + 5 * 8192));
        let stable = std::fs::read(&source).unwrap();
        s.set_page_label(50, "Recovery label").unwrap();
        s.rename_bookmark(0, "Pending redo").unwrap();
        assert!(s.undo());
        let history = (s.undo.len(), s.redo.len());
        let checkpoint = (
            s.checkpoint.clone(),
            s.label_checkpoint.clone(),
            s.shape_checkpoint.clone(),
        );
        let permissions = std::fs::metadata(&source).unwrap().permissions();
        let mut readonly = permissions.clone();
        readonly.set_readonly(true);
        std::fs::set_permissions(&source, readonly).unwrap();
        let error = s.save().unwrap_err().to_string();
        assert!(error.contains("read-only"), "{error}");
        assert_eq!(s.path(), source);
        assert!(s.is_dirty());
        assert_eq!((s.undo.len(), s.redo.len()), history);
        assert_eq!(
            (
                s.checkpoint.clone(),
                s.label_checkpoint.clone(),
                s.shape_checkpoint.clone()
            ),
            checkpoint
        );
        assert_eq!(std::fs::read(&source).unwrap(), stable);
        assert_eq!(s.last_backup_path().unwrap(), backups.last().unwrap());
        assert!(s.undo() && !s.is_dirty());
        assert!(s.redo() && s.is_dirty());
        std::fs::set_permissions(&source, permissions).unwrap();
        let recovery_save = synthetic_save_checked(&mut s, &baseline);
        backups.push(s.last_backup_path().unwrap().to_owned());
        let pre_export = std::fs::read(&source).unwrap();
        s.set_page_label(75, "Export checkpoint").unwrap();
        let history = (s.undo.len(), s.redo.len());
        let collision = dir.path().join("collision.pdf");
        std::fs::write(&collision, &original).unwrap();
        assert!(s.save_as(&collision).is_err());
        assert_eq!(s.path(), source);
        assert!(s.is_dirty());
        assert_eq!((s.undo.len(), s.redo.len()), history);
        assert_eq!(std::fs::read(&source).unwrap(), pre_export);
        assert_eq!(std::fs::read(&collision).unwrap(), original);
        let copy = dir.path().join("synthetic-export.pdf");
        let start = std::time::Instant::now();
        s.save_as(&copy).unwrap();
        let export_elapsed = start.elapsed();
        assert_eq!(s.path(), copy);
        assert!(!s.is_dirty());
        assert_eq!((s.undo.len(), s.redo.len()), history);
        synthetic_assert_reopen(&s, &baseline);
        assert!(s.undo() && s.is_dirty());
        assert!(s.redo() && !s.is_dirty());
        assert_eq!(std::fs::read(&source).unwrap(), pre_export);
        assert!(backups.iter().all(|p| p.exists()));
        let unique: std::collections::HashSet<_> = backups.iter().collect();
        assert_eq!(unique.len(), 7);
        println!(
            "synthetic component only: pages={} shapes={} rectangles={} ellipses={} saves={} save_as_successes=1 history_cap={} objects_initial={} objects_final={} source_initial_bytes={} first_saved_bytes={} repeated_saved_bytes={sizes:?} export_bytes={} open={open_elapsed:?} initial_edits={edit_elapsed:?} initial_save={first_save:?} repeated_saves={timings:?} recovery_save={recovery_save:?} save_as={export_elapsed:?} outputSource={} outputExport={} (owned TempDir paths expire after test; not whole-app metrics)",
            s.doc.get_pages().len(),
            s.shapes().len(),
            s.rectangles().len(),
            s.shapes()
                .iter()
                .filter(|a| a.kind == ShapeKind::Ellipse)
                .count(),
            backups.len(),
            s.undo.len(),
            initial_objects,
            s.doc.objects.len(),
            original.len(),
            first_bytes,
            std::fs::read(&copy).unwrap().len(),
            source.display(),
            copy.display()
        );
    }

    // Write the lexical PDF directly: Document::save would canonicalize the
    // integral real tokens before the imported-number regression can run.
    fn imported_real_fixture(path: &Path, media_reals: bool, bbox_reals: bool) {
        let media = if media_reals {
            "0.0 0.0 612.0 792.0"
        } else {
            "0 0 612 792"
        };
        let bbox = if bbox_reals {
            "0.0 0.0 100.0 40.0"
        } else {
            "0 0 100 40"
        };
        let appearance = b"q 0 0 1 rg 0 0 100 40 re f Q";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R /Outlines 8 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [{media}] /Resources 7 0 R /Contents 4 0 R /Annots [5 0 R] >>"),
            "<< /Length 0 >>\nstream\n\nendstream".to_owned(),
            "<< /Type /Annot /Subtype /Square /Rect [20 20 120 60] /AP << /N 6 0 R >> /Contents (foreign) >>".to_owned(),
            format!("<< /Type /XObject /Subtype /Form /BBox [{bbox}] /Resources 7 0 R /Length {} >>\nstream\n{}\nendstream", appearance.len(), std::str::from_utf8(appearance).unwrap()),
            "<< /Font << /F1 10 0 R >> /ProcSet [/PDF /Text] >>".to_owned(),
            "<< /Type /Outlines /First 9 0 R /Last 9 0 R /Count 1 >>".to_owned(),
            "<< /Title (Original) /Parent 8 0 R /Dest [3 0 R /Fit] >>".to_owned(),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        ];
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = vec![0];
        for (i, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
        }
        let xref = bytes.len();
        bytes.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes(),
        );
        for offset in offsets.iter().skip(1) {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                offsets.len()
            )
            .as_bytes(),
        );
        std::fs::write(path, bytes).unwrap();
    }

    fn imported_reals_save_history(save_as: bool) {
        for (media_reals, bbox_reals) in [(true, false), (false, true), (true, true)] {
            let dir = tempfile::tempdir().unwrap();
            let source = dir.path().join("imported.pdf");
            let copy = dir.path().join("copy.pdf");
            imported_real_fixture(&source, media_reals, bbox_reals);
            let original = std::fs::read(&source).unwrap();
            let mut s = EditablePdf::open(&source).unwrap();
            let before = s.doc.clone();
            let media = before
                .get_object((3, 0))
                .unwrap()
                .as_dict()
                .unwrap()
                .get(b"MediaBox")
                .unwrap()
                .as_array()
                .unwrap();
            assert_eq!(matches!(media[0], Object::Real(_)), media_reals);
            let bbox = before
                .get_object((6, 0))
                .unwrap()
                .as_stream()
                .unwrap()
                .dict
                .get(b"BBox")
                .unwrap()
                .as_array()
                .unwrap();
            assert_eq!(matches!(bbox[0], Object::Real(_)), bbox_reals);
            let r = crate::core::links::PdfRect {
                x: 0.125,
                y: 0.25,
                width: 0.25,
                height: 0.375,
            };
            s.add_rectangle(0, r).unwrap();
            s.add_ellipse(0, r).unwrap();
            s.rename_bookmark(0, "Imported 図面").unwrap();
            s.set_page_label(0, "A101").unwrap();
            assert_eq!(std::fs::read(&source).unwrap(), original);
            if save_as {
                s.save_as(&copy).unwrap();
            } else {
                s.save().unwrap();
            }
            let active = if save_as { &copy } else { &source };
            assert_eq!(s.path(), active);
            assert!(!s.is_dirty());
            assert_eq!(s.undo.len(), 4);
            for _ in 0..4 {
                assert!(s.undo());
            }
            assert!(s.is_dirty());
            assert!(s.shapes().is_empty());
            for _ in 0..4 {
                assert!(s.redo());
            }
            assert!(!s.is_dirty());
            for _ in 0..2 {
                s.save().unwrap();
                let reopened = EditablePdf::open(active).unwrap();
                assert_eq!(reopened.shapes(), s.shapes());
                assert_eq!(reopened.bookmarks()[0].title, "Imported 図面");
                assert_eq!(reopened.page_labels(), ["A101"]);
                // Unrelated foreign annotation, AP resources and font stay exact.
                for id in [(5, 0), (7, 0), (10, 0)] {
                    assert_eq!(
                        reopened.doc.get_object(id).unwrap(),
                        before.get_object(id).unwrap()
                    );
                }
                let ap = reopened
                    .doc
                    .get_object((6, 0))
                    .unwrap()
                    .as_stream()
                    .unwrap();
                let original_ap = before.get_object((6, 0)).unwrap().as_stream().unwrap();
                assert_eq!(ap.content, original_ap.content);
                let mut expected = original_ap.dict.clone();
                expected.set("BBox", vec![0.into(), 0.into(), 100.into(), 40.into()]);
                assert_eq!(ap.dict, expected);
                let annots = reopened
                    .doc
                    .get_object((3, 0))
                    .unwrap()
                    .as_dict()
                    .unwrap()
                    .get(b"Annots")
                    .unwrap()
                    .as_array()
                    .unwrap();
                assert_eq!(annots[0], Object::Reference((5, 0)));
                assert_eq!(annots.len(), 3);
            }
            if save_as {
                assert_eq!(std::fs::read(&source).unwrap(), original);
            }
            s.delete_shape(s.shapes()[1].object_id).unwrap();
            s.save().unwrap();
            assert!(s.undo() && s.is_dirty());
            assert!(s.redo() && !s.is_dirty());
            assert_eq!(EditablePdf::open(active).unwrap().rectangles().len(), 1);
            assert_eq!(EditablePdf::open(active).unwrap().shapes().len(), 1);
        }
    }

    #[test]
    fn imported_integral_reals_save_preserves_foreign_appearance_and_history() {
        imported_reals_save_history(false);
    }

    #[test]
    fn imported_integral_reals_save_as_preserves_foreign_appearance_and_history() {
        imported_reals_save_history(true);
    }

    #[test]
    fn serialized_equality_allows_only_exact_display_integer_conversion() {
        for value in [
            0.0,
            -0.0,
            612.0,
            -792.0,
            16_777_216.0,
            16_777_218.0,
            10_000_000_000.0,
        ] {
            let emitted = value.to_string().parse::<i64>().unwrap();
            assert!(serialized_object_equal(
                &Object::Real(value),
                &Object::Integer(emitted)
            ));
            assert!(!serialized_object_equal(
                &Object::Real(value),
                &Object::Integer(emitted + 1)
            ));
            assert!(!serialized_object_equal(
                &Object::Integer(emitted),
                &Object::Real(value)
            ));
            // Exercise the actual lopdf writer/parser, not only Display.
            let mut doc = Document::with_version("1.7");
            let id = doc.add_object(Object::Real(value));
            let root = doc.add_object(dictionary! {"Type"=>"Catalog", "Number"=>id});
            doc.trailer.set("Root", root);
            let mut bytes = Vec::new();
            doc.save_to(&mut bytes).unwrap();
            let saved = Document::load_mem(&bytes).unwrap();
            assert_eq!(saved.get_object(id).unwrap(), &Object::Integer(emitted));
        }
        // These neighbors collapse to the same f32: neither Integer->Integer
        // corruption nor Integer->Real replacement may ever be accepted.
        assert!(!serialized_object_equal(
            &Object::Integer(16_777_217),
            &Object::Integer(16_777_216)
        ));
        assert!(!serialized_object_equal(
            &Object::Integer(16_777_217),
            &Object::Real(16_777_216.0)
        ));
        assert!(!serialized_object_equal(
            &Object::Real(16_777_216.0),
            &Object::Integer(16_777_217)
        ));
        for value in [
            0.25,
            -1.5,
            f32::MAX,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
        ] {
            assert!(!serialized_object_equal(
                &Object::Real(value),
                &Object::Integer(0)
            ));
        }
        assert!(!serialized_object_equal(
            &Object::Real(0.25),
            &Object::Real(0.250001)
        ));
    }

    #[test]
    fn serialized_equality_preserves_nested_structure_and_stream_bytes() {
        let original: Object = dictionary! {
            "Nested"=>vec![Object::Dictionary(dictionary! {"Value"=>Object::Real(612.0)}), Object::Reference((7, 2))],
            "Bytes"=>Object::string_literal(b"foreign bytes".to_vec()),
            "Name"=>"Foreign"
        }.into();
        let mut canonical = original.as_dict().unwrap().clone();
        canonical.set(
            "Nested",
            vec![
                Object::Dictionary(dictionary! {"Value"=>612}),
                Object::Reference((7, 2)),
            ],
        );
        assert!(serialized_object_equal(
            &original,
            &Object::Dictionary(canonical.clone())
        ));
        for case in 0..7 {
            let mut changed = canonical.clone();
            match case {
                0 => {
                    changed.remove(b"Name");
                }
                1 => changed.set("Extra", Object::Null),
                2 => changed.set("Bytes", Object::string_literal(b"foreign byteS".to_vec())),
                3 => changed.set("Name", "Other"),
                4 => changed.set(
                    "Nested",
                    vec![
                        Object::Dictionary(dictionary! {"Value"=>612}),
                        Object::Reference((7, 3)),
                    ],
                ),
                5 => changed.set(
                    "Nested",
                    vec![
                        Object::Reference((7, 2)),
                        Object::Dictionary(dictionary! {"Value"=>612}),
                    ],
                ),
                _ => changed.set(
                    "Nested",
                    vec![Object::Dictionary(dictionary! {"Value"=>612})],
                ),
            }
            assert!(
                !serialized_object_equal(&original, &Object::Dictionary(changed)),
                "case {case}"
            );
        }
        let a = lopdf::Stream::new(
            dictionary! {"BBox"=>vec![Object::Real(0.0), Object::Real(100.0)], "Resources"=>Object::Reference((7, 0))},
            b"q Q".to_vec(),
        );
        let mut b = a.clone();
        b.dict
            .set("BBox", vec![Object::Integer(0), Object::Integer(100)]);
        assert!(serialized_object_equal(
            &Object::Stream(a.clone()),
            &Object::Stream(b.clone())
        ));
        b.content[0] = b'Q';
        assert!(!serialized_object_equal(
            &Object::Stream(a.clone()),
            &Object::Stream(b.clone())
        ));
        b.content.clone_from(&a.content);
        b.dict.set("Resources", Object::Reference((8, 0)));
        assert!(!serialized_object_equal(
            &Object::Stream(a),
            &Object::Stream(b)
        ));
    }

    #[test]
    fn imported_real_failed_serialization_keeps_source_path_checkpoint_and_history() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.pdf");
        let copy = dir.path().join("copy.pdf");
        imported_real_fixture(&source, true, true);
        let original = std::fs::read(&source).unwrap();
        let mut s = EditablePdf::open(&source).unwrap();
        s.rename_bookmark(0, "Changed").unwrap();
        // Invalid non-PDF number forces serialization/reload verification to fail.
        s.doc.add_object(Object::Real(f32::NAN));
        let checkpoint = s.checkpoint.clone();
        let source_hash = s.source_hash;
        for save_as in [false, true] {
            let result = if save_as { s.save_as(&copy) } else { s.save() };
            assert!(result.is_err());
            assert_eq!(s.path(), source);
            assert_eq!(s.checkpoint, checkpoint);
            assert_eq!(s.source_hash, source_hash);
            assert!(s.is_dirty() && s.can_undo() && !s.can_redo());
            assert_eq!(s.undo.len(), 1);
            assert!(s.last_backup_path().is_none());
            assert_eq!(std::fs::read(&source).unwrap(), original);
            assert!(!copy.exists());
        }
        assert!(s.undo() && !s.is_dirty());
        assert!(s.redo() && s.is_dirty());
    }

    #[test]
    fn ellipse_geometry_rejection_and_foreign_delete_preserve_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ellipse-validation.pdf");
        fixture(&path);
        let source = std::fs::read(&path).unwrap();
        let mut s = EditablePdf::open(&path).unwrap();
        let r = crate::core::links::PdfRect {
            x: 0.125,
            y: 0.25,
            width: 0.25,
            height: 0.375,
        };
        s.add_ellipse(0, r).unwrap();
        let id = s.shapes()[0].object_id;
        assert!(
            !s.delete_rectangle(id).unwrap(),
            "legacy rectangle-only delete cannot remove ellipse"
        );
        assert!(s.undo());
        let objects = s.doc.objects.clone();
        for invalid in [
            crate::core::links::PdfRect {
                width: f32::NAN,
                ..r
            },
            crate::core::links::PdfRect { height: 0., ..r },
            crate::core::links::PdfRect { x: 0.9, ..r },
            crate::core::links::PdfRect {
                width: f32::MIN_POSITIVE,
                ..r
            },
        ] {
            assert!(s.add_ellipse(0, invalid).is_err());
            assert_eq!(s.doc.objects, objects);
            assert!(s.can_redo() && !s.is_dirty());
        }
        assert!(s.add_ellipse(1, r).is_err());
        assert!(!s.delete_shape(id).unwrap());
        assert!(!s.delete_shape((9999, 0)).unwrap());
        assert!(s.can_redo() && !s.can_undo() && !s.is_dirty());
        assert_eq!(s.doc.objects, objects);
        assert_eq!(std::fs::read(&path).unwrap(), source);
        assert!(s.redo());
        assert_eq!(s.shapes()[0].object_id, id);
    }
    #[test]
    fn ellipse_malformed_owned_schema_fails_closed_without_source_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ellipse-malformed.pdf");
        for case in 0..6 {
            fixture(&path);
            let mut s = EditablePdf::open(&path).unwrap();
            s.add_ellipse(
                0,
                crate::core::links::PdfRect {
                    x: 0.125,
                    y: 0.25,
                    width: 0.25,
                    height: 0.375,
                },
            )
            .unwrap();
            let id = s.shapes()[0].object_id;
            let a = s.doc.get_object_mut(id).unwrap().as_dict_mut().unwrap();
            match case {
                0 => a.set("GlyphEllipse", 2),
                1 => a.set("GlyphRectangle", 1),
                2 => a.set("Subtype", "Square"),
                3 => a.set("AP", dictionary! {}),
                4 => a.set("Rect", vec![0.into(), 0.into(), 0.into(), 0.into()]),
                _ => a.set(
                    "GlyphNormalizedRect",
                    vec![0.into(), 0.into(), 2.into(), 1.into()],
                ),
            }
            let source = std::fs::read(&path).unwrap();
            assert!(
                s.save().is_err(),
                "case {case} must fail before persistence"
            );
            assert_eq!(s.path(), path);
            assert!(s.can_undo());
            assert_eq!(std::fs::read(&path).unwrap(), source);
            // Deliberately serialize malformed fixture, not the protected source.
            let malformed = dir.path().join(format!("bad-{case}.pdf"));
            s.doc.save(&malformed).unwrap();
            let original = std::fs::read(&malformed).unwrap();
            assert!(EditablePdf::open(&malformed).is_err(), "case {case}");
            assert_eq!(std::fs::read(&malformed).unwrap(), original);
        }
    }
    #[test]
    fn ellipse_mixed_history_repeated_save_branch_and_rejections() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mixed.pdf");
        let copy = dir.path().join("copy.pdf");
        fixture(&path);
        let source = std::fs::read(&path).unwrap();
        let mut s = EditablePdf::open(&path).unwrap();
        let r = crate::core::links::PdfRect {
            x: 0.125,
            y: 0.25,
            width: 0.25,
            height: 0.375,
        };
        s.add_ellipse(0, r).unwrap();
        assert!(s.is_dirty(), "ellipse must participate in dirty checkpoint");
        let ellipse = s.shapes()[0].object_id;
        s.add_rectangle(0, r).unwrap();
        s.rename_bookmark(0, "図面 🏠").unwrap();
        s.set_page_label(0, "A101").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), source);
        s.save().unwrap();
        let first = std::fs::read(&path).unwrap();
        assert!(!s.is_dirty());
        assert_eq!(s.rectangles().len(), 1);
        assert_eq!(EditablePdf::open(&path).unwrap().shapes(), s.shapes());
        assert!(
            s.delete_shape(ellipse).unwrap(),
            "native ellipse can be deleted"
        );
        s.save().unwrap();
        assert!(!s.is_dirty());
        assert!(s.undo());
        assert!(s.is_dirty());
        assert!(s.redo());
        assert!(!s.is_dirty());
        assert!(s.undo());
        s.save_as(&copy).unwrap();
        assert_eq!(s.path(), copy);
        assert!(!s.is_dirty());
        let reopened = EditablePdf::open(&copy).unwrap();
        assert_eq!(reopened.shapes(), s.shapes());
        assert_eq!(reopened.bookmarks()[0].title, "図面 🏠");
        assert_eq!(reopened.page_labels(), ["A101"]);
        s.set_page_label(0, "branch").unwrap();
        assert!(!s.can_redo());
        let history = (s.undo.len(), s.redo.len());
        let checkpoint = s.shapes();
        let saved_copy = std::fs::read(&copy).unwrap();
        assert!(s.save_as(&path).is_err());
        assert_eq!(s.path(), copy);
        assert!(s.is_dirty());
        assert_eq!((s.undo.len(), s.redo.len()), history);
        let mut external = saved_copy.clone();
        external.extend(b"\n% external\n");
        std::fs::write(&copy, &external).unwrap();
        assert!(s.save().is_err());
        assert_eq!(s.path(), copy);
        assert!(s.is_dirty());
        assert_eq!(s.shapes(), checkpoint);
        assert_eq!((s.undo.len(), s.redo.len()), history);
        assert_eq!(std::fs::read(&copy).unwrap(), external);
        assert!(s.undo());
        assert!(
            !s.is_dirty(),
            "undo to saved checkpoint is clean even after rejected save"
        );
        assert!(s.redo());
        // Dropping a branched session discards only its in-memory edits.
        drop(s);
        assert_eq!(std::fs::read(&copy).unwrap(), external);
        assert_ne!(std::fs::read(&path).unwrap(), first);
        assert_eq!(EditablePdf::open(&path).unwrap().shapes().len(), 1);
    }
    #[test]
    fn ellipse_persists_native_circle_without_source_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ellipse.pdf");
        fixture(&path);
        let source = std::fs::read(&path).unwrap();
        let mut s = EditablePdf::open(&path).unwrap();
        let r = crate::core::links::PdfRect {
            x: 0.125,
            y: 0.25,
            width: 0.25,
            height: 0.375,
        };
        s.add_ellipse(0, r).unwrap();
        let id = super::super::rectangles::read(&s.doc).unwrap()[0].object_id;
        let a = s.doc.get_object(id).unwrap().as_dict().unwrap();
        assert_eq!(a.get(b"Subtype").unwrap().as_name().unwrap(), b"Circle");
        assert_eq!(a.get(b"GlyphEllipse").unwrap().as_i64().unwrap(), 1);
        assert!(!a.has(b"IC"));
        assert_eq!(
            a.get(b"C").unwrap(),
            &Object::Array(vec![1.into(), 0.into(), 0.into()])
        );
        assert_eq!(
            a.get(b"BS")
                .unwrap()
                .as_dict()
                .unwrap()
                .get(b"W")
                .unwrap()
                .as_i64()
                .unwrap(),
            2
        );
        let ap = a
            .get(b"AP")
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"N")
            .unwrap()
            .as_reference()
            .unwrap();
        let ap = s.doc.get_object(ap).unwrap().as_stream().unwrap();
        let operations = lopdf::content::Content::decode(&ap.content)
            .unwrap()
            .operations;
        assert_eq!(operations.iter().filter(|op| op.operator == "c").count(), 4);
        assert_eq!(operations.iter().filter(|op| op.operator == "S").count(), 1);
        assert!(!operations.iter().any(|op| {
            ["f", "F", "f*", "B", "B*", "b", "b*", "re"].contains(&op.operator.as_str())
        }));
        assert_eq!(std::fs::read(&path).unwrap(), source);
        s.save().unwrap();
        let reopened = EditablePdf::open(&path).unwrap();
        assert_eq!(
            super::super::rectangles::read(&reopened.doc).unwrap(),
            super::super::rectangles::read(&s.doc).unwrap()
        );
    }
    #[test]
    fn rectangle_add_persists_square_and_preserves_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rect.pdf");
        fixture(&path);
        let original = std::fs::read(&path).unwrap();
        let mut s = EditablePdf::open(&path).unwrap();
        let rect = crate::core::links::PdfRect {
            x: 0.1,
            y: 0.2,
            width: 0.3,
            height: 0.4,
        };
        assert!(s.add_rectangle(0, rect).unwrap());
        assert_eq!(s.rectangles().len(), 1);
        assert_eq!(s.rectangles()[0].rect, rect);
        let id = s.rectangles()[0].object_id;
        assert_eq!(
            s.doc
                .get_object(id)
                .unwrap()
                .as_dict()
                .unwrap()
                .get(b"Subtype")
                .unwrap()
                .as_name()
                .unwrap(),
            b"Square"
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
        s.save().unwrap();
        assert_eq!(
            EditablePdf::open(&path).unwrap().rectangles(),
            s.rectangles()
        );
    }
    #[test]
    fn rectangle_shared_history_checkpoints() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.pdf");
        fixture(&path);
        let mut s = EditablePdf::open(&path).unwrap();
        s.add_rectangle(
            0,
            crate::core::links::PdfRect {
                x: 0.1,
                y: 0.2,
                width: 0.3,
                height: 0.4,
            },
        )
        .unwrap();
        assert!(s.is_dirty(), "rectangle marks dirty");
        s.rename_bookmark(0, "changed").unwrap();
        s.set_page_label(0, "label").unwrap();
        s.save().unwrap();
        assert!(!s.is_dirty());
        assert!(s.undo());
        assert!(s.undo());
        assert!(s.undo());
        assert!(s.rectangles().is_empty());
        assert!(s.is_dirty());
        assert!(s.redo());
        assert!(s.redo());
        assert!(s.redo());
        assert!(!s.is_dirty());
        for _ in 0..70 {
            s.add_rectangle(
                0,
                crate::core::links::PdfRect {
                    x: 0.1,
                    y: 0.2,
                    width: 0.3,
                    height: 0.4,
                },
            )
            .unwrap();
        }
        assert_eq!(s.undo.len(), 64);
    }
    #[test]
    fn rectangle_delete_only_owned_preserves_indirect_arrays() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("delete.pdf");
        fixture(&path);
        let mut d = Document::load(&path).unwrap();
        let page = *d.get_pages().values().next().unwrap();
        let original = d
            .get_object(page)
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Annots")
            .unwrap()
            .clone();
        let foreign = original.as_array().unwrap()[0].as_reference().unwrap();
        let array = d.add_object(original.clone());
        d.get_object_mut(page)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Annots", array);
        d.save(&path).unwrap();
        let mut s = EditablePdf::open(&path).unwrap();
        s.add_rectangle(
            0,
            crate::core::links::PdfRect {
                x: 0.1,
                y: 0.2,
                width: 0.3,
                height: 0.4,
            },
        )
        .unwrap();
        let id = s.rectangles()[0].object_id;
        assert!(!s.delete_rectangle(foreign).unwrap());
        assert!(s.delete_rectangle(id).unwrap(), "owned rectangle deleted");
        assert!(s.rectangles().is_empty());
        assert_eq!(s.doc.get_object(array).unwrap(), &original);
        assert!(s.undo());
        assert_eq!(s.rectangles()[0].object_id, id);
        assert!(s.undo());
        assert!(!s.is_dirty());
        assert_eq!(
            s.doc
                .get_object(page)
                .unwrap()
                .as_dict()
                .unwrap()
                .get(b"Annots")
                .unwrap(),
            &Object::Reference(array)
        );
        assert!(s.redo());
        assert!(s.redo());
        s.save().unwrap();
        assert!(EditablePdf::open(&path).unwrap().rectangles().is_empty());
        assert_eq!(
            Document::load(&path).unwrap().get_object(array).unwrap(),
            &original
        );
    }
    #[test]
    fn shape_snapshot_native_rotated_crop_rendering() {
        use crate::pdf::{PdfRenderEngine, PdfiumRenderEngine};
        let dir = tempfile::tempdir().unwrap();
        for (rotation, kind) in [0, 90, 180, 270].into_iter().flat_map(|r| {
            [
                super::super::ShapeKind::Rectangle,
                super::super::ShapeKind::Ellipse,
            ]
            .into_iter()
            .map(move |k| (r, k))
        }) {
            let path = dir.path().join(format!("r{rotation}{kind:?}.pdf"));
            fixture(&path);
            let mut d = Document::load(&path).unwrap();
            let page = *d.get_pages().values().next().unwrap();
            let p = d.get_object_mut(page).unwrap().as_dict_mut().unwrap();
            let parent = p.get(b"Parent").unwrap().as_reference().unwrap();
            for key in [b"MediaBox".as_slice(), b"CropBox", b"Rotate"] {
                p.remove(key);
            }
            let p = d.get_object_mut(parent).unwrap().as_dict_mut().unwrap();
            p.set("MediaBox", vec![0.into(), 0.into(), 400.into(), 300.into()]);
            p.set(
                "CropBox",
                vec![30.into(), 40.into(), 350.into(), 240.into()],
            );
            p.set("Rotate", rotation);
            d.save(&path).unwrap();
            let source = std::fs::read(&path).unwrap();
            let baseline = PdfiumRenderEngine.render_page(&path, 0, 640).unwrap();
            let mut s = EditablePdf::open(&path).unwrap();
            let rect = crate::core::links::PdfRect {
                x: 0.125,
                y: 0.25,
                width: 0.25,
                height: 0.375,
            };
            s.add_shape(0, rect, kind).unwrap();
            let bytes = s.render_snapshot().unwrap();
            assert!(bytes.starts_with(b"%PDF"), "snapshot is PDF");
            let snapshot = dir.path().join(format!("snapshot{rotation}.pdf"));
            std::fs::write(&snapshot, bytes).unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), source);
            let image = PdfiumRenderEngine.render_page(&snapshot, 0, 640).unwrap();
            assert_eq!(
                (image.width, image.height),
                (baseline.width, baseline.height)
            );
            let mut red = vec![];
            for y in 0..image.height {
                for x in 0..image.width {
                    let i = (y * image.width + x) * 4;
                    let c = &image.rgba[i..i + 4];
                    if c[0] > 180 && c[1] < 80 && c[2] < 80 {
                        red.push((x, y));
                    }
                }
            }
            assert!(
                !red.is_empty(),
                "rotation {rotation}: native AP renders red"
            );
            let bounds = [
                red.iter().map(|p| p.0).min().unwrap() as f32 / image.width as f32,
                red.iter().map(|p| p.1).min().unwrap() as f32 / image.height as f32,
                red.iter().map(|p| p.0).max().unwrap() as f32 / image.width as f32,
                red.iter().map(|p| p.1).max().unwrap() as f32 / image.height as f32,
            ];
            for (a, b) in bounds.into_iter().zip([0.125, 0.25, 0.375, 0.625]) {
                assert!(
                    (a - b).abs() < 0.015,
                    "rotation {rotation}: bounds {bounds:?}"
                );
            }
            let center = ((image.height as f32 * 0.4375) as usize * image.width
                + (image.width as f32 * 0.25) as usize)
                * 4;
            assert_eq!(
                &image.rgba[center..center + 4],
                &baseline.rgba[center..center + 4],
                "unfilled center"
            );
            if kind == super::super::ShapeKind::Ellipse {
                // Ellipse must not be the rectangular bounding box: no red at
                // its corners, and each red pixel lies on the cubic oval.
                for &(x, y) in &red {
                    let nx = x as f32 / image.width as f32;
                    let ny = y as f32 / image.height as f32;
                    let radial = ((nx - 0.25) / 0.125).powi(2) + ((ny - 0.4375) / 0.1875).powi(2);
                    assert!(
                        (0.82..1.12).contains(&radial),
                        "rotation {rotation}: red pixel ({nx},{ny}) radial {radial}"
                    );
                }
            }
            s.save().unwrap();
            assert_eq!(
                image,
                PdfiumRenderEngine.render_page(&path, 0, 640).unwrap()
            );
            let mut reopened = EditablePdf::open(&path).unwrap();
            assert_eq!(reopened.shapes(), s.shapes());
            let id = reopened.shapes()[0].object_id;
            reopened.delete_shape(id).unwrap();
            reopened.save().unwrap();
            assert_eq!(
                baseline,
                PdfiumRenderEngine.render_page(&path, 0, 640).unwrap()
            );
        }
    }
    #[test]
    fn rectangle_staging_rejects_malformed_owned_appearance() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("malformed.pdf");
        fixture(&path);
        let mut s = EditablePdf::open(&path).unwrap();
        s.add_rectangle(
            0,
            crate::core::links::PdfRect {
                x: 0.1,
                y: 0.2,
                width: 0.3,
                height: 0.4,
            },
        )
        .unwrap();
        let id = s.rectangles()[0].object_id;
        s.doc
            .get_object_mut(id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("AP", dictionary! {});
        let original = std::fs::read(&path).unwrap();
        assert!(s.save().is_err(), "staging checks rectangle semantics");
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
    #[test]
    fn rectangle_rejects_underflowed_pdf_geometry_transactionally() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tiny.pdf");
        fixture(&path);
        let mut s = EditablePdf::open(&path).unwrap();
        let count = s.doc.objects.len();
        assert!(
            s.add_rectangle(
                0,
                crate::core::links::PdfRect {
                    x: 0.5,
                    y: 0.5,
                    width: f32::MIN_POSITIVE,
                    height: 0.1
                }
            )
            .is_err()
        );
        assert_eq!(s.doc.objects.len(), count);
        assert!(!s.can_undo());
        assert!(!s.is_dirty());
    }
    #[test]
    fn rectangle_open_rejects_invalid_owned_form_bbox() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bbox.pdf");
        fixture(&path);
        let mut s = EditablePdf::open(&path).unwrap();
        s.add_rectangle(
            0,
            crate::core::links::PdfRect {
                x: 0.1,
                y: 0.2,
                width: 0.3,
                height: 0.4,
            },
        )
        .unwrap();
        s.save().unwrap();
        let mut d = Document::load(&path).unwrap();
        let id = s.rectangles()[0].object_id;
        let ap = d
            .get_object(id)
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"AP")
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"N")
            .unwrap()
            .as_reference()
            .unwrap();
        d.get_object_mut(ap)
            .unwrap()
            .as_stream_mut()
            .unwrap()
            .dict
            .set("BBox", vec![0.into(), 0.into(), 0.into(), 10.into()]);
        d.save(&path).unwrap();
        let original = std::fs::read(&path).unwrap();
        assert!(
            EditablePdf::open(&path).is_err(),
            "degenerate owned AP fails closed"
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
    #[test]
    fn rectangle_validation_and_snapshot_limit_leave_state_unchanged() {
        use crate::core::links::PdfRect;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("validation.pdf");
        fixture(&path);
        let mut s = EditablePdf::open(&path).unwrap();
        let original = std::fs::read(&path).unwrap();
        for rect in [
            PdfRect {
                x: f32::NAN,
                y: 0.,
                width: 0.1,
                height: 0.1,
            },
            PdfRect {
                x: 0.,
                y: f32::INFINITY,
                width: 0.1,
                height: 0.1,
            },
            PdfRect {
                x: 0.,
                y: 0.,
                width: f32::INFINITY,
                height: 0.1,
            },
            PdfRect {
                x: 0.,
                y: 0.,
                width: 0.1,
                height: f32::NAN,
            },
            PdfRect {
                x: -0.1,
                y: 0.,
                width: 0.1,
                height: 0.1,
            },
            PdfRect {
                x: 0.,
                y: -0.1,
                width: 0.1,
                height: 0.1,
            },
            PdfRect {
                x: 0.9,
                y: 0.,
                width: 0.2,
                height: 0.1,
            },
            PdfRect {
                x: 0.,
                y: 0.9,
                width: 0.1,
                height: 0.2,
            },
            PdfRect {
                x: 0.,
                y: 0.,
                width: 0.,
                height: 0.1,
            },
            PdfRect {
                x: 0.,
                y: 0.,
                width: 0.1,
                height: -0.1,
            },
        ] {
            assert!(s.add_rectangle(0, rect).is_err());
        }
        let rect = PdfRect {
            x: 0.125,
            y: 0.25,
            width: 0.25,
            height: 0.375,
        };
        assert!(s.add_rectangle(1, rect).is_err());
        assert!(!s.is_dirty());
        assert!(!s.can_undo());
        s.add_rectangle(0, rect).unwrap();
        let id = s.rectangles()[0].object_id;
        s.undo();
        assert!(!s.delete_rectangle(id).unwrap());
        assert!(s.can_redo());
        s.redo();
        let objects = s.doc.objects.clone();
        let checkpoint = s.shape_checkpoint.clone();
        let bytes = s.render_snapshot().unwrap();
        assert!(s.snapshot_with_limit(bytes.len() - 1).is_err());
        assert_eq!(s.snapshot_with_limit(bytes.len()).unwrap(), bytes);
        assert_eq!(s.doc.objects, objects);
        assert_eq!(s.shape_checkpoint, checkpoint);
        assert!(s.is_dirty());
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
    #[test]
    fn rectangle_malformed_arrays_and_inheritance_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.pdf");
        for case in 0..6 {
            fixture(&path);
            let mut d = Document::load(&path).unwrap();
            let page = *d.get_pages().values().next().unwrap();
            let cycle = d.new_object_id();
            d.objects.insert(cycle, Object::Reference(cycle));
            let p = d.get_object_mut(page).unwrap().as_dict_mut().unwrap();
            match case {
                0 => p.set("Annots", 3),
                1 => p.set("Annots", cycle),
                2 => p.set("Annots", vec![Object::Integer(3)]),
                3 => p.set(
                    "Annots",
                    vec![Object::Dictionary(
                        dictionary! {"GlyphRectangle"=>1,"Subtype"=>"Square"},
                    )],
                ),
                4 => p.set("Parent", page),
                _ => p.set("Rotate", 45),
            }
            d.save(&path).unwrap();
            let original = std::fs::read(&path).unwrap();
            assert!(EditablePdf::open(&path).is_err(), "case {case}");
            assert_eq!(std::fs::read(&path).unwrap(), original);
        }
    }
    #[test]
    fn rectangle_save_as_preserves_unrelated_objects_and_conflict_guards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.pdf");
        let copy = dir.path().join("copy.pdf");
        fixture(&path);
        let original = std::fs::read(&path).unwrap();
        let before = Document::load(&path).unwrap();
        let mut s = EditablePdf::open(&path).unwrap();
        s.add_rectangle(
            0,
            crate::core::links::PdfRect {
                x: 0.125,
                y: 0.25,
                width: 0.25,
                height: 0.375,
            },
        )
        .unwrap();
        s.rename_bookmark(0, "Rectangle title").unwrap();
        s.set_page_label(0, "A101").unwrap();
        s.save_as(&copy).unwrap();
        assert!(!s.is_dirty());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        let reopened = EditablePdf::open(&copy).unwrap();
        assert_eq!(reopened.rectangles(), s.rectangles());
        assert_eq!(reopened.page_labels(), ["A101"]);
        let after = Document::load(&copy).unwrap();
        let root = before.trailer.get(b"Root").unwrap().as_reference().unwrap();
        let page = *before.get_pages().values().next().unwrap();
        for (id, o) in &before.objects {
            if *id == root
                || *id == page
                || *id == s.entries[0].id
                || o.type_name().ok() == Some(b"XRef".as_slice())
            {
                continue;
            }
            match o {
                Object::Stream(a) => {
                    let b = after.get_object(*id).unwrap().as_stream().unwrap();
                    assert_eq!(a.dict, b.dict);
                    assert_eq!(a.content, b.content);
                }
                _ => assert_eq!(o, after.get_object(*id).unwrap()),
            }
        }
        let mut expected = before.get_object(page).unwrap().as_dict().unwrap().clone();
        expected.set(
            "Annots",
            after
                .get_object(page)
                .unwrap()
                .as_dict()
                .unwrap()
                .get(b"Annots")
                .unwrap()
                .clone(),
        );
        assert_eq!(
            &Object::Dictionary(expected),
            after.get_object(page).unwrap()
        );
        s.delete_rectangle(s.rectangles()[0].object_id).unwrap();
        let saved = std::fs::read(&copy).unwrap();
        assert!(s.save_as(&copy).is_err());
        assert_eq!(std::fs::read(&copy).unwrap(), saved);
        assert!(s.is_dirty());
        let mut external = saved;
        external.extend(b"\n% external\n");
        std::fs::write(&copy, &external).unwrap();
        assert!(s.save().is_err());
        assert_eq!(std::fs::read(&copy).unwrap(), external);
        assert!(s.is_dirty());
        assert!(s.undo());
        assert!(!s.is_dirty());
    }
    #[test]
    fn rectangle_open_rejects_degenerate_native_rect_with_close_normalized_mapping() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("native.pdf");
        fixture(&path);
        let mut s = EditablePdf::open(&path).unwrap();
        s.add_rectangle(
            0,
            crate::core::links::PdfRect {
                x: 0.1,
                y: 0.2,
                width: 0.000001,
                height: 0.4,
            },
        )
        .unwrap();
        s.save().unwrap();
        let id = s.rectangles()[0].object_id;
        let mut d = Document::load(&path).unwrap();
        let a = d
            .get_object_mut(id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .get_mut(b"Rect")
            .unwrap()
            .as_array_mut()
            .unwrap();
        a[3] = a[1].clone();
        d.save(&path).unwrap();
        assert!(
            EditablePdf::open(&path).is_err(),
            "native Rect must be nondegenerate even within mapping tolerance"
        );
    }
    fn fixture(path: &Path) {
        let mut d = Document::with_version("1.7");
        let pages = d.new_object_id();
        let content = d.add_object(lopdf::Stream::new(
            dictionary! {},
            b"0 0 1 rg 20 20 80 80 re f".to_vec(),
        ));
        let p = d.add_object(dictionary! {"Type"=>"Page", "Parent"=>pages, "MediaBox"=>vec![0.into(),0.into(),300.into(),300.into()], "CropBox"=>vec![10.into(),10.into(),290.into(),290.into()], "Rotate"=>90, "Contents"=>content, "Resources"=>dictionary!{}});
        let link = d.add_object(dictionary!{"Type"=>"Annot", "Subtype"=>"Link", "Rect"=>vec![20.into(),20.into(),100.into(),100.into()], "Dest"=>vec![p.into(),Object::Name(b"Fit".to_vec())]});
        d.get_object_mut(p)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Annots", vec![Object::Reference(link)]);
        d.objects.insert(
            pages,
            dictionary! {"Type"=>"Pages", "Kids"=>vec![p.into()], "Count"=>1}.into(),
        );
        let outlines = d.new_object_id();
        let item = d.add_object(dictionary!{"Title"=>Object::string_literal("Original"), "Parent"=>outlines, "Dest"=>vec![p.into(),Object::Name(b"Fit".to_vec())], "F"=>2, "C"=>vec![1.into(),0.into(),0.into()]});
        d.objects.insert(
            outlines,
            dictionary! {"Type"=>"Outlines", "First"=>item, "Last"=>item, "Count"=>1}.into(),
        );
        let root =
            d.add_object(dictionary! {"Type"=>"Catalog", "Pages"=>pages, "Outlines"=>outlines});
        d.trailer.set("Root", root);
        d.save(path).unwrap();
    }
    #[test]
    fn page_label_original_tree_restoration_and_no_object_growth() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("labels.pdf");
        let copy = dir.path().join("copy.pdf");
        fixture(&path);
        let mut d = Document::load(&path).unwrap();
        let page = *d.get_pages().values().next().unwrap();
        let page_dict = d.get_object(page).unwrap().as_dict().unwrap().clone();
        let parent = page_dict.get(b"Parent").unwrap().as_reference().unwrap();
        let p2 = d.add_object(page_dict.clone());
        let p3 = d.add_object(page_dict);
        let pages = d.get_object_mut(parent).unwrap().as_dict_mut().unwrap();
        pages.set("Kids", vec![Object::Reference(page), p2.into(), p3.into()]);
        pages.set("Count", 3);
        let prefix = d.add_object(unicode_title("図面-"));
        let nums = d.add_object(vec![
            0.into(),
            dictionary! {"S"=>"r"}.into(),
            2.into(),
            dictionary! {"S"=>"D", "P"=>prefix, "St"=>9}.into(),
        ]);
        let leaf = d.add_object(dictionary! {"Nums"=>nums, "Limits"=>vec![0.into(),2.into()]});
        let kids = d.add_object(vec![Object::Reference(leaf)]);
        let tree = d.add_object(dictionary! {"Kids"=>kids});
        d.catalog_mut().unwrap().set("PageLabels", tree);
        d.save(&path).unwrap();
        let mut s = EditablePdf::open(&path).unwrap();
        let object_count = s.doc.objects.len();
        assert_eq!(s.page_labels(), ["i", "ii", "図面-9"]);
        for i in 0..100 {
            s.set_page_label(1, &format!("Label {i}")).unwrap();
        }
        assert_eq!(s.doc.objects.len(), object_count);
        s.set_page_label(1, "ii").unwrap();
        assert!(!s.is_dirty());
        assert_eq!(
            s.doc.catalog().unwrap().get(b"PageLabels").unwrap(),
            &Object::Reference(tree)
        );
        s.set_page_label(0, "Renamed").unwrap();
        assert_eq!(s.page_labels(), ["Renamed", "ii", "図面-9"]);
        s.save_as(&copy).unwrap();
        let saved = Document::load(&copy).unwrap();
        for id in [prefix, nums, leaf, kids, tree] {
            assert_eq!(saved.get_object(id).unwrap(), d.get_object(id).unwrap());
        }
        assert!(s.undo());
        assert!(s.is_dirty());
        assert_eq!(
            s.doc.catalog().unwrap().get(b"PageLabels").unwrap(),
            &Object::Reference(tree)
        );
        s.save().unwrap();
        assert_eq!(
            EditablePdf::open(&copy).unwrap().page_labels(),
            ["i", "ii", "図面-9"]
        );
        assert_eq!(
            Document::load(&copy)
                .unwrap()
                .catalog()
                .unwrap()
                .get(b"PageLabels")
                .unwrap(),
            &Object::Reference(tree)
        );
    }
    #[test]
    fn page_label_malformed_input_fails_closed_without_touching_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("labels.pdf");
        fixture(&path);
        let mut d = Document::load(&path).unwrap();
        let id = d.new_object_id();
        d.objects
            .insert(id, dictionary! {"Kids"=>vec![Object::Reference(id)]}.into());
        d.catalog_mut().unwrap().set("PageLabels", id);
        d.save(&path).unwrap();
        let original = std::fs::read(&path).unwrap();
        assert!(EditablePdf::open(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
    #[test]
    fn page_label_source_conflict_and_no_clobber_preserve_dirty_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("labels.pdf");
        let existing = dir.path().join("existing.pdf");
        fixture(&path);
        let original = std::fs::read(&path).unwrap();
        std::fs::write(&existing, b"existing destination").unwrap();
        let mut s = EditablePdf::open(&path).unwrap();
        s.set_page_label(0, "A-101").unwrap();
        assert!(s.save_as(&existing).is_err());
        assert_eq!(std::fs::read(&existing).unwrap(), b"existing destination");
        assert_eq!(s.path(), path);
        assert!(s.is_dirty());
        let mut changed = original;
        changed.extend(b"\n% external change\n");
        std::fs::write(&path, &changed).unwrap();
        assert!(
            s.save()
                .unwrap_err()
                .to_string()
                .contains("changed externally")
        );
        assert_eq!(std::fs::read(&path).unwrap(), changed);
        assert_eq!(s.page_labels(), ["A-101"]);
        assert!(s.is_dirty());
        assert!(s.last_backup_path().is_none());
    }
    #[test]
    fn page_label_validation_bounds_and_shared_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("labels.pdf");
        fixture(&path);
        let mut s = EditablePdf::open(&path).unwrap();
        assert_eq!(EditablePdf::MAX_PAGE_LABEL_CHARS, 256);
        assert!(s.set_page_label(0, " \n ").is_err());
        assert!(s.set_page_label(0, &"字".repeat(257)).is_err());
        assert!(s.set_page_label(1, "valid").is_err());
        assert!(!s.is_dirty());
        assert!(!s.can_undo());
        assert!(s.set_page_label(0, &"字".repeat(256)).unwrap());
        assert!(s.undo());
        assert!(!s.is_dirty());
        assert!(!s.set_page_label(0, " 1 ").unwrap());
        assert!(s.can_redo(), "no-op does not discard redo");
        for i in 0..70 {
            if i % 2 == 0 {
                s.rename_bookmark(0, &format!("title {i}")).unwrap();
            } else {
                s.set_page_label(0, &format!("label {i}")).unwrap();
            }
        }
        assert_eq!(s.undo.len(), 64);
        let mut count = 0;
        while s.undo() {
            count += 1;
        }
        assert_eq!(count, 64);
        assert!(s.redo());
        s.set_page_label(0, "branch").unwrap();
        assert!(!s.can_redo());
    }
    #[test]
    fn page_label_mixed_history_save_reopen_preserves_page_objects() {
        use crate::pdf::{LopdfInspectionEngine, PdfEngine, PdfRenderEngine, PdfiumRenderEngine};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("labels.pdf");
        let copy = dir.path().join("copy.pdf");
        fixture(&path);
        let original = std::fs::read(&path).unwrap();
        let before = Document::load(&path).unwrap();
        let pixels = PdfiumRenderEngine.render_page(&path, 0, 128).unwrap();
        let mut s = EditablePdf::open(&path).unwrap();
        assert_eq!(s.page_labels(), ["1"]);
        s.rename_bookmark(0, "Title edit").unwrap();
        assert!(s.set_page_label(0, "図面 🏠").unwrap());
        assert!(s.undo());
        assert_eq!(s.page_labels(), ["1"]);
        assert_eq!(s.bookmarks()[0].title, "Title edit");
        assert!(s.redo());
        s.save_as(&copy).unwrap();
        assert!(!s.is_dirty());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(EditablePdf::open(&copy).unwrap().page_labels(), ["図面 🏠"]);
        assert_eq!(
            LopdfInspectionEngine.inspect(&copy).unwrap().pages[0]
                .label
                .as_deref(),
            Some("図面 🏠")
        );
        assert_eq!(
            pixels,
            PdfiumRenderEngine.render_page(&copy, 0, 128).unwrap()
        );
        let after = Document::load(&copy).unwrap();
        let root = before.trailer.get(b"Root").unwrap().as_reference().unwrap();
        for (id, object) in &before.objects {
            if *id == root
                || *id == s.entries[0].id
                || object.type_name().ok() == Some(b"XRef".as_slice())
            {
                continue;
            }
            match object {
                Object::Stream(a) => {
                    let b = after.get_object(*id).unwrap().as_stream().unwrap();
                    assert_eq!(a.dict, b.dict);
                    assert_eq!(a.content, b.content);
                }
                _ => assert_eq!(object, after.get_object(*id).unwrap()),
            }
        }
        assert!(s.undo());
        assert!(s.is_dirty());
        assert!(s.undo());
        assert_eq!(s.bookmarks()[0].title, "Original");
        assert!(s.redo());
        assert!(s.redo());
        assert!(!s.is_dirty());
        s.set_page_label(0, "A-101").unwrap();
        s.save().unwrap();
        assert_eq!(EditablePdf::open(&copy).unwrap().page_labels(), ["A-101"]);
        assert!(s.undo());
        assert!(s.is_dirty());
        assert!(s.redo());
        assert!(!s.is_dirty());
    }
    #[test]
    fn page_label_inspector_defaults_to_numeric() {
        use crate::pdf::{LopdfInspectionEngine, PdfEngine};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("labels.pdf");
        fixture(&path);
        assert_eq!(
            LopdfInspectionEngine.inspect(&path).unwrap().pages[0]
                .label
                .as_deref(),
            Some("1")
        );
    }
    #[test]
    fn history_availability_tracks_save_and_undo_redo() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let mut editor = EditablePdf::open(&path).unwrap();
        assert!(!editor.can_undo());
        assert!(!editor.can_redo());
        editor.rename_bookmark(0, "A101").unwrap();
        assert!(editor.can_undo(), "A rename must enable Undo");
        assert!(!editor.can_redo());
        editor.save().unwrap();
        assert!(editor.can_undo());
        assert!(editor.undo());
        assert!(!editor.can_undo());
        assert!(editor.can_redo());
        assert!(editor.redo());
        assert!(editor.can_undo());
        assert!(!editor.can_redo());
    }
    #[test]
    fn rename_unicode_is_in_memory_and_history_tracks_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.pdf");
        fixture(&p);
        let before = std::fs::read(&p).unwrap();
        let mut s = EditablePdf::open(&p).unwrap();
        assert_eq!(s.bookmarks().len(), 1);
        assert_eq!(s.bookmarks()[0].page_index, Some(0));
        assert!(s.rename_bookmark(0, "  図面 🏠  ").unwrap());
        assert_eq!(s.bookmarks()[0].title, "図面 🏠");
        assert!(s.is_dirty());
        assert_eq!(std::fs::read(&p).unwrap(), before);
        assert!(s.undo());
        assert!(!s.is_dirty());
        assert!(s.redo());
        assert!(s.is_dirty());
        assert!(!s.rename_bookmark(0, "図面 🏠").unwrap());
        assert!(s.undo());
        assert!(!s.undo());
    }
    #[test]
    fn validates_titles_and_bounds_history() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.pdf");
        fixture(&p);
        let mut s = EditablePdf::open(&p).unwrap();
        assert!(s.rename_bookmark(0, " \n ").is_err());
        assert!(s.rename_bookmark(0, &"字".repeat(4097)).is_err());
        assert!(s.rename_bookmark(99, "valid").is_err());
        assert!(!s.is_dirty());
        assert!(!s.undo());
        assert!(s.rename_bookmark(0, &"字".repeat(4096)).unwrap());
        for i in 0..70 {
            s.rename_bookmark(0, &format!("Title {i}")).unwrap();
        }
        let mut n = 0;
        while s.undo() {
            n += 1;
        }
        assert_eq!(n, 64);
        assert!(s.redo());
        assert!(s.rename_bookmark(0, "branch").unwrap());
        assert!(!s.redo());
    }
    #[test]
    fn save_reopens_unicode_and_preserves_objects_and_render() {
        use crate::pdf::{PdfRenderEngine, PdfiumRenderEngine};
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.pdf");
        fixture(&p);
        let before = Document::load(&p).unwrap();
        let rendered = PdfiumRenderEngine.render_page(&p, 0, 128).unwrap();
        let mut s = EditablePdf::open(&p).unwrap();
        s.rename_bookmark(0, "図面 🏠").unwrap();
        s.save().unwrap();
        assert!(!s.is_dirty());
        let reopened = EditablePdf::open(&p).unwrap();
        assert_eq!(reopened.bookmarks()[0].title, "図面 🏠");
        let after = Document::load(&p).unwrap();
        for (id, object) in &before.objects {
            if object
                .type_name()
                .ok()
                .is_some_and(|t| [b"XRef".as_slice(), b"ObjStm", b"Linearized"].contains(&t))
            {
                continue;
            }
            if *id == s.entries[0].id {
                let mut expected = object.as_dict().unwrap().clone();
                expected.set("Title", unicode_title("図面 🏠"));
                assert_eq!(
                    after.get_object(*id).unwrap(),
                    &Object::Dictionary(expected)
                );
            } else if let Object::Stream(stream) = object {
                let saved = after.get_object(*id).unwrap().as_stream().unwrap();
                assert_eq!(saved.dict, stream.dict);
                assert_eq!(saved.content, stream.content);
            } else {
                assert_eq!(after.get_object(*id).unwrap(), object);
            }
        }
        assert_eq!(
            rendered,
            PdfiumRenderEngine.render_page(&p, 0, 128).unwrap()
        );
        assert!(s.undo());
        assert!(s.is_dirty());
        assert!(s.redo());
        assert!(!s.is_dirty());
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn successful_save_exposes_permanent_unique_backup_for_late_fd_writers() {
        use std::io::Write;
        use std::os::unix::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.pdf");
        fixture(&p);
        let original = std::fs::read(&p).unwrap();
        let original_inode = std::fs::metadata(&p).unwrap().ino();
        let mut writer = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        let mut s = EditablePdf::open(&p).unwrap();
        assert!(s.last_backup_path().is_none());
        s.rename_bookmark(0, "Edited").unwrap();
        s.save().unwrap();
        let backup = s
            .last_backup_path()
            .expect("successful saves expose backup")
            .to_owned();
        assert_eq!(backup.parent(), p.parent());
        assert!(
            backup
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with(".glyph-backup-")
        );
        assert_eq!(std::fs::metadata(&backup).unwrap().ino(), original_inode);
        assert_eq!(std::fs::read(&backup).unwrap(), original);
        let saved = std::fs::read(&p).unwrap();
        writer.write_all(b"\n% late old-fd write\n").unwrap();
        writer.sync_all().unwrap();
        let mut late = original;
        late.extend(b"\n% late old-fd write\n");
        assert_eq!(std::fs::read(&backup).unwrap(), late);
        assert_eq!(std::fs::read(&p).unwrap(), saved);
        assert!(!s.is_dirty());
        s.rename_bookmark(0, "Second").unwrap();
        s.save().unwrap();
        let second = s.last_backup_path().unwrap().to_owned();
        assert_ne!(backup, second);
        assert_eq!(std::fs::read(&second).unwrap(), saved);
        drop(s);
        drop(writer);
        assert_eq!(std::fs::read(&backup).unwrap(), late);
        assert_eq!(std::fs::read(&second).unwrap(), saved);
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn replacement_at_commit_reports_conflict_and_retains_external_inode() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.pdf");
        fixture(&p);
        let mut s = EditablePdf::open(&p).unwrap();
        let checkpoint_hash = s.source_hash;
        s.rename_bookmark(0, "Edited").unwrap();
        let replacement = dir.path().join("external.pdf");
        fixture(&replacement);
        let mut external = std::fs::read(&replacement).unwrap();
        external.extend(b"\n% external replacement\n");
        std::fs::write(&replacement, &external).unwrap();
        let result = s.save_before_commit(|| std::fs::rename(&replacement, &p).unwrap());
        assert!(result.is_err(), "commit race must not silently succeed");
        let error = result.unwrap_err().to_string();
        let backup = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path != &p)
            .expect("displaced external inode must have a permanent backup");
        assert_eq!(std::fs::read(&backup).unwrap(), external);
        assert!(error.contains("conflict"), "{error}");
        assert!(error.contains(backup.to_str().unwrap()), "{error}");
        assert_eq!(
            EditablePdf::open(&p).unwrap().bookmarks()[0].title,
            "Edited"
        );
        assert!(s.is_dirty());
        assert_eq!(s.source_hash, checkpoint_hash);
        assert!(s.undo());
        assert!(!s.is_dirty());
        assert!(s.redo());
        drop(s);
        assert_eq!(std::fs::read(&backup).unwrap(), external);
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn in_place_write_at_commit_is_retained_on_conflict() {
        use std::io::Write;
        use std::os::unix::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.pdf");
        fixture(&p);
        let inode = std::fs::metadata(&p).unwrap().ino();
        let mut external = std::fs::read(&p).unwrap();
        external.extend(b"\n% concurrent in-place write\n");
        let mut writer = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        let mut s = EditablePdf::open(&p).unwrap();
        s.rename_bookmark(0, "Edited").unwrap();
        let error = s
            .save_before_commit(|| {
                writer
                    .write_all(b"\n% concurrent in-place write\n")
                    .unwrap();
                writer.sync_all().unwrap();
            })
            .unwrap_err()
            .to_string();
        let backup = s.last_backup_path().unwrap().to_owned();
        assert_eq!(std::fs::metadata(&backup).unwrap().ino(), inode);
        assert_eq!(std::fs::read(&backup).unwrap(), external);
        assert!(error.contains(backup.to_str().unwrap()));
        assert!(s.is_dirty());
        // A later third writer is never rolled back or overwritten by recovery.
        std::fs::write(&p, b"third writer").unwrap();
        assert!(s.save().is_err());
        assert_eq!(std::fs::read(&p).unwrap(), b"third writer");
        assert_eq!(std::fs::read(&backup).unwrap(), external);
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn raced_symlink_is_preserved_without_writing_its_target() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.pdf");
        let target = dir.path().join("target.pdf");
        fixture(&p);
        fixture(&target);
        let untouched = std::fs::read(&target).unwrap();
        let mut s = EditablePdf::open(&p).unwrap();
        s.rename_bookmark(0, "Edited").unwrap();
        let error = s
            .save_before_commit(|| {
                std::fs::remove_file(&p).unwrap();
                symlink(&target, &p).unwrap();
            })
            .unwrap_err()
            .to_string();
        let backup = s.last_backup_path().unwrap();
        assert!(
            std::fs::symlink_metadata(backup)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_link(backup).unwrap(), target);
        assert_eq!(std::fs::read(&target).unwrap(), untouched);
        assert!(error.contains("conflict"));
        assert!(s.is_dirty());
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn missing_source_at_commit_fails_closed_without_installing_staged_pdf() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.pdf");
        let moved = dir.path().join("moved.pdf");
        fixture(&p);
        let original = std::fs::read(&p).unwrap();
        let mut s = EditablePdf::open(&p).unwrap();
        s.rename_bookmark(0, "Edited").unwrap();
        let error = s
            .save_before_commit(|| std::fs::rename(&p, &moved).unwrap())
            .unwrap_err()
            .to_string();
        assert!(error.contains("source not replaced"));
        assert!(!p.exists());
        assert_eq!(std::fs::read(&moved).unwrap(), original);
        assert!(s.last_backup_path().is_none());
        assert!(s.is_dirty());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[test]
    fn external_change_rejects_save_without_losing_session() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.pdf");
        fixture(&p);
        let mut s = EditablePdf::open(&p).unwrap();
        s.rename_bookmark(0, "New").unwrap();
        let mut bytes = std::fs::read(&p).unwrap();
        bytes.extend(b"\n% external change\n");
        std::fs::write(&p, &bytes).unwrap();
        assert!(s.save().unwrap_err().to_string().contains("externally"));
        assert_eq!(std::fs::read(&p).unwrap(), bytes);
        assert!(s.is_dirty());
        assert_eq!(s.path(), p);
        assert!(s.undo());
        assert!(!s.is_dirty());
        assert!(s.redo());
    }
    #[test]
    fn save_as_creates_new_file_keeps_history_and_refuses_collisions() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.pdf");
        let output = dir.path().join("b.pdf");
        fixture(&p);
        let original = std::fs::read(&p).unwrap();
        let mut s = EditablePdf::open(&p).unwrap();
        s.rename_bookmark(0, "New").unwrap();
        s.save_as(&output).unwrap();
        assert_eq!(s.path(), output);
        assert!(!s.is_dirty());
        assert_eq!(
            EditablePdf::open(&output).unwrap().bookmarks()[0].title,
            "New"
        );
        assert_eq!(std::fs::read(&p).unwrap(), original);
        assert!(s.undo());
        let saved = std::fs::read(&output).unwrap();
        assert!(s.save_as(&output).is_err());
        assert!(s.save_as(&p).is_err());
        assert_eq!(std::fs::read(&output).unwrap(), saved);
        assert_eq!(std::fs::read(&p).unwrap(), original);
        assert_eq!(s.path(), output);
        assert!(s.is_dirty());
        assert!(s.redo());
        assert!(!s.is_dirty());
        s.rename_bookmark(0, "Again").unwrap();
        s.save().unwrap();
        assert!(!s.is_dirty());
    }
    #[cfg(unix)]
    #[test]
    fn save_preserves_permissions_and_rejects_readonly_or_symlink() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.pdf");
        fixture(&p);
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o640)).unwrap();
        let mut s = EditablePdf::open(&p).unwrap();
        s.rename_bookmark(0, "New").unwrap();
        s.save().unwrap();
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o640
        );
        s.rename_bookmark(0, "Next").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o440)).unwrap();
        let saved = std::fs::read(&p).unwrap();
        assert!(s.save().unwrap_err().to_string().contains("read-only"));
        assert!(s.is_dirty());
        assert_eq!(std::fs::read(&p).unwrap(), saved);
        let alias = dir.path().join("alias.pdf");
        symlink(&p, &alias).unwrap();
        let mut linked = EditablePdf::open(&alias).unwrap();
        linked.rename_bookmark(0, "Alias").unwrap();
        assert!(linked.save().unwrap_err().to_string().contains("symlink"));
        assert!(
            std::fs::symlink_metadata(&alias)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(linked.save_as(&alias).is_err());
        assert_eq!(std::fs::read(&p).unwrap(), saved);
    }
    #[test]
    fn refuses_signature_objects_and_nested_fields() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("signed.pdf");
        for object in [
            Object::Dictionary(
                dictionary! {"Type" => "Sig", "ByteRange" => vec![0.into(), 1.into(), 2.into(), 3.into()]},
            ),
            Object::Dictionary(
                dictionary! {"Fields" => vec![Object::Dictionary(dictionary! {"FT" => "Sig"})]},
            ),
        ] {
            fixture(&p);
            let mut d = Document::load(&p).unwrap();
            d.add_object(object);
            d.save(&p).unwrap();
            let before = std::fs::read(&p).unwrap();
            let error = EditablePdf::open(&p)
                .err()
                .expect("signature must be rejected");
            assert!(error.to_string().contains("signed"));
            assert_eq!(std::fs::read(&p).unwrap(), before);
        }
    }
    #[test]
    fn refuses_empty_password_encryption_even_after_automatic_decryption() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("encrypted.pdf");
        fixture(&p);
        let mut d = Document::load(&p).unwrap();
        d.trailer.set(
            "ID",
            vec![
                Object::string_literal("fixture-id"),
                Object::string_literal("fixture-id"),
            ],
        );
        let state = lopdf::EncryptionState::try_from(lopdf::EncryptionVersion::V2 {
            document: &d,
            owner_password: "",
            user_password: "",
            key_length: 128,
            permissions: lopdf::Permissions::all(),
        })
        .unwrap();
        d.encrypt(&state).unwrap();
        d.save(&p).unwrap();
        let before = std::fs::read(&p).unwrap();
        let error = EditablePdf::open(&p)
            .err()
            .expect("encryption must be rejected");
        assert!(error.to_string().contains("encrypted"));
        assert_eq!(std::fs::read(&p).unwrap(), before);
    }
    #[test]
    fn resolves_indirect_internal_action_but_not_remote_targets() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.pdf");
        fixture(&p);
        let mut s = EditablePdf::open(&p).unwrap();
        let id = s.entries[0].id;
        let page = *s.doc.get_pages().values().next().unwrap();
        let destination = s
            .doc
            .add_object(vec![Object::Reference(page), Object::Name(b"Fit".to_vec())]);
        let action = s
            .doc
            .add_object(dictionary! {"S" => "GoTo", "D" => destination});
        let item = s.doc.get_object_mut(id).unwrap().as_dict_mut().unwrap();
        item.remove(b"Dest");
        item.set("A", action);
        s.doc.save(&p).unwrap();
        let internal = EditablePdf::open(&p).unwrap();
        assert_eq!(internal.bookmarks()[0].page_index, Some(0));
        s.doc
            .get_object_mut(action)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("S", "GoToR");
        s.doc.save(&p).unwrap();
        assert_eq!(
            EditablePdf::open(&p).unwrap().bookmarks()[0].page_index,
            None
        );
    }
    #[test]
    fn nested_outlines_with_indirect_unicode_titles_preserve_hierarchy_and_actions() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.pdf");
        fixture(&p);
        let mut base = EditablePdf::open(&p).unwrap();
        let parent = base.entries[0].id;
        let page = *base.doc.get_pages().values().next().unwrap();
        let title = base.doc.add_object(unicode_title("子 🏠"));
        let action = base.doc.add_object(dictionary! {"S" => "GoTo", "D" => vec![Object::Reference(page), Object::Name(b"Fit".to_vec())]});
        let child = base.doc.add_object(dictionary! {"Title" => title, "Parent" => parent, "A" => action, "F" => 1, "C" => vec![0.into(), 1.into(), 0.into()]});
        let item = base
            .doc
            .get_object_mut(parent)
            .unwrap()
            .as_dict_mut()
            .unwrap();
        item.set("First", child);
        item.set("Last", child);
        item.set("Count", -1);
        base.doc.save(&p).unwrap();
        let mut s = EditablePdf::open(&p).unwrap();
        assert_eq!(s.bookmarks()[1].title, "子 🏠");
        assert_eq!(s.bookmarks()[1].depth, 1);
        assert_eq!(s.bookmarks()[1].page_index, Some(0));
        let original_child = s.doc.get_object(child).unwrap().as_dict().unwrap().clone();
        s.rename_bookmark(1, "改名").unwrap();
        s.save().unwrap();
        let saved = Document::load(&p).unwrap();
        let mut expected = original_child;
        expected.set("Title", unicode_title("改名"));
        assert_eq!(
            saved.get_object(child).unwrap(),
            &Object::Dictionary(expected)
        );
        assert_eq!(
            saved.get_object(action).unwrap(),
            base.doc.get_object(action).unwrap()
        );
        assert_eq!(
            saved.get_object(parent).unwrap(),
            base.doc.get_object(parent).unwrap()
        );
        assert_eq!(
            saved.get_object(title).unwrap(),
            base.doc.get_object(title).unwrap()
        );
        let mut cyclic = saved;
        cyclic
            .get_object_mut(child)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("First", parent);
        cyclic.save(&p).unwrap();
        assert!(
            EditablePdf::open(&p)
                .err()
                .unwrap()
                .to_string()
                .contains("cyclic")
        );
    }
    #[cfg(unix)]
    #[test]
    fn save_as_preserves_source_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.pdf");
        let output = dir.path().join("b.pdf");
        fixture(&p);
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o640)).unwrap();
        let mut s = EditablePdf::open(&p).unwrap();
        s.rename_bookmark(0, "New").unwrap();
        s.save_as(&output).unwrap();
        assert_eq!(
            std::fs::metadata(&output).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
    #[test]
    fn opens_real_document() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.pdf");
        fixture(&p);
        assert!(EditablePdf::open(&p).is_ok());
    }
}
