use super::*;
use crate::pdf::{EditableBookmark, EditablePdf, PdfBookmark};

#[derive(Default)]
pub(super) struct EditingState {
    session: Option<EditablePdf>,
    pending: Option<PendingEdit>,
    pub(super) dirty: bool,
    saved_bookmarks: Option<Vec<PdfBookmark>>,
    saved_pages: Option<Vec<crate::pdf::PdfPageInfo>>,
    rename: Option<RenameDialog>,
    transition: Option<Transition>,
    save_before_transition: bool,
    unrecoverable: bool,
    error: Option<String>,
    preview_error: Option<String>,
    deferred_save: Option<DeferredSave>,
    saved_preview: Option<SavedPreview>,
}
struct SavedPreview {
    generation: u64,
    path: PathBuf,
    started: Instant,
}
#[derive(Clone, Copy)]
enum EditKind {
    Edit,
    Save,
    SaveAs,
    LoadMarkups,
}
impl EditKind {
    fn label(self) -> &'static str {
        match self {
            Self::Edit => "Editing document…",
            Self::Save => "Saving PDF…",
            Self::SaveAs => "Saving PDF copy…",
            Self::LoadMarkups => "Loading markups…",
        }
    }
}
struct DeferredSave {
    generation: u64,
    path: PathBuf,
    command: Command,
    inline_intent: Option<markup::SaveIntent>,
}
struct PendingEdit {
    kind: EditKind,
    started: Instant,
    generation: u64,
    path: PathBuf,
    receiver: mpsc::Receiver<EditCompletion>,
}
struct EditCompletion {
    session: Option<EditablePdf>,
    result: Result<bool, PdfError>,
    bookmarks: Vec<EditableBookmark>,
    page_labels: Vec<String>,
    saved: bool,
    dirty: bool,
    shapes: Vec<crate::pdf::ShapeAnnotation>,
    snapshot: Option<Vec<u8>>,
    // A preview failure does not undo a successful mutation or its history entry.
    preview_error: Option<PdfError>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum RenameTarget {
    Bookmark,
    PageLabel,
}
struct RenameDialog {
    target: RenameTarget,
    index: usize,
    original: String,
    value: String,
    focus: bool,
}
enum Transition {
    Open(PathBuf),
    CloseDocument,
    Quit,
}
enum Decision {
    Save,
    Discard,
    Cancel,
}
enum Command {
    Text {
        page: usize,
        rect: crate::core::links::PdfRect,
        text: crate::pdf::TextMarkup,
        style: crate::pdf::ShapeStyle,
    },
    LoadMarkups,
    Line {
        page: usize,
        endpoints: [f32; 4],
        arrow: bool,
    },
    Rectangle {
        page: usize,
        rect: crate::core::links::PdfRect,
    },
    Ellipse {
        page: usize,
        rect: crate::core::links::PdfRect,
    },
    UpdateShape(crate::pdf::ShapeAnnotation),
    DeleteShape(lopdf::ObjectId),
    Rename {
        index: usize,
        original: String,
        title: String,
    },
    PageLabel {
        index: usize,
        original: String,
        title: String,
    },
    Undo,
    Redo,
    Save,
    SaveAs(PathBuf),
}

impl GlyphApp {
    pub(super) fn defer_inline_text_save(
        &mut self,
        intent: markup::SaveIntent,
        ctx: &egui::Context,
    ) {
        self.defer_save(Command::Save, ctx);
        if let Some(request) = &mut self.editing.deferred_save {
            request.inline_intent = Some(intent);
        }
    }
    pub(super) fn validate_inline_text(
        &self,
        page: usize,
        rect: crate::core::links::PdfRect,
        text: &crate::pdf::TextMarkup,
        style: crate::pdf::ShapeStyle,
    ) -> Result<(), PdfError> {
        text.validate()?;
        if let Some(session) = &self.editing.session {
            session.validate_text(page, rect, text, style)?;
        }
        Ok(())
    }
    pub(super) fn add_text_markup(
        &mut self,
        page: usize,
        rect: crate::core::links::PdfRect,
        text: crate::pdf::TextMarkup,
        style: crate::pdf::ShapeStyle,
        ctx: &egui::Context,
    ) {
        self.start_edit(
            Command::Text {
                page,
                rect,
                text,
                style,
            },
            ctx,
        );
    }
    pub(super) fn request_page_label(&mut self, page: usize, ctx: &egui::Context) {
        if self.editing_modal_open()
            || self.editing.pending.is_some()
            || self.editing.unrecoverable
            || self.loading_document.is_some()
            || self.automation_rx.is_some()
        {
            return;
        }
        let Some(info) = self
            .project
            .document
            .as_ref()
            .and_then(|d| d.summary.pages.get(page))
        else {
            return;
        };
        let label = info.label.clone().unwrap_or_else(|| (page + 1).to_string());
        self.editing.rename = Some(RenameDialog {
            target: RenameTarget::PageLabel,
            index: page,
            original: label.clone(),
            value: label,
            focus: true,
        });
        ctx.request_repaint();
    }
    pub(super) fn markup_page_size(&self, page: usize) -> Option<egui::Vec2> {
        let [w, h] = self
            .editing
            .session
            .as_ref()?
            .display_page_size(page)
            .ok()?;
        Some(egui::vec2(w, h))
    }
    pub(super) fn can_change_markups(&self) -> bool {
        self.project.document.is_some()
            && !self.render_worker_failed()
            && !self.markup.preview_pending
            && self.editing.preview_error.is_none()
            && self.loading_document.is_none()
            && self.editing.pending.is_none()
            && self.automation_rx.is_none()
            && !self.editing.unrecoverable
            && !self.editing_modal_open()
    }
    pub(super) fn inline_text_unsaved_decision(&self) -> bool {
        self.editing.transition.is_some()
            && self.editing.rename.is_none()
            && self.project.document.is_some()
            && !self.render_worker_failed()
            && !self.markup.preview_pending
            && self.editing.preview_error.is_none()
            && self.loading_document.is_none()
            && self.editing.pending.is_none()
            && self.automation_rx.is_none()
            && !self.editing.unrecoverable
    }
    pub(super) fn load_markups(&mut self, ctx: &egui::Context) {
        self.start_edit(Command::LoadMarkups, ctx);
    }
    pub(super) fn add_rectangle(
        &mut self,
        page: usize,
        rect: crate::core::links::PdfRect,
        ctx: &egui::Context,
    ) {
        self.start_edit(Command::Rectangle { page, rect }, ctx);
    }

    pub(super) fn add_ellipse(
        &mut self,
        page: usize,
        rect: crate::core::links::PdfRect,
        ctx: &egui::Context,
    ) {
        self.start_edit(Command::Ellipse { page, rect }, ctx);
    }
    pub(super) fn delete_shape(&mut self, id: lopdf::ObjectId, ctx: &egui::Context) {
        self.start_edit(Command::DeleteShape(id), ctx);
    }
    pub(super) fn add_line(
        &mut self,
        page: usize,
        endpoints: [f32; 4],
        arrow: bool,
        ctx: &egui::Context,
    ) {
        if self.can_change_markups() && page == self.project.selected_page {
            self.start_edit(
                Command::Line {
                    page,
                    endpoints,
                    arrow,
                },
                ctx,
            );
        }
    }
    pub(super) fn update_markup(
        &mut self,
        shape: crate::pdf::ShapeAnnotation,
        ctx: &egui::Context,
    ) {
        self.start_edit(Command::UpdateShape(shape), ctx);
    }
    pub(super) fn persistent_edit_error(&self) -> Option<&str> {
        self.editing
            .error
            .as_deref()
            .or(self.editing.preview_error.as_deref())
    }
    pub(super) fn preview_failed(&mut self, error: impl std::fmt::Display) {
        let saved = self
            .editing
            .saved_preview
            .as_ref()
            .is_some_and(|marker| self.editing_progress_owned(marker.generation, &marker.path));
        self.editing.saved_preview = None;
        self.markup.preview_pending = false;
        self.cancel_markup_selection();
        let evidence = if saved {
            "Saved PDF — verified and committed, but preview unavailable"
        } else {
            "Document changes retained but preview unavailable"
        };
        let recovery = if self.render_worker_failed() {
            "Save or Save As if needed, then restart Glyph to restore previews."
        } else {
            "Retry preview, Undo, Save, or Save As to recover."
        };
        self.editing.preview_error = Some(format!("{evidence}: {error}. {recovery}"));
        self.status = self.editing.preview_error.clone().unwrap();
    }
    pub(super) fn preview_ready(&mut self) {
        // Called only when the current page's validated pixels are installed.
        // A failed replacement can advance generation while keeping this PDF;
        // its stale progress token must not keep the current preview gated.
        self.editing.saved_preview = None;
        if self.markup.preview_pending {
            self.markup.preview_pending = false;
            self.editing.preview_error = None;
        }
    }
    fn editing_progress_owned(&self, generation: u64, path: &Path) -> bool {
        generation == self.document_generation
            && self.loading_document.is_none()
            && self
                .project
                .document
                .as_ref()
                .is_some_and(|d| d.path == path)
    }
    pub(super) fn editing_progress_status(&self, now: Instant) -> Option<String> {
        if self.persistent_edit_error().is_some() {
            return None;
        }
        let (label, started) = if let Some(pending) = &self.editing.pending {
            if !self.editing_progress_owned(pending.generation, &pending.path) {
                return None;
            }
            (pending.kind.label(), pending.started)
        } else if let Some(marker) = &self.editing.saved_preview {
            if !self.editing_progress_owned(marker.generation, &marker.path) {
                return None;
            }
            ("Saved PDF — refreshing preview…", marker.started)
        } else {
            return None;
        };
        let elapsed = now.checked_duration_since(started).unwrap_or_default();
        if elapsed >= Duration::from_secs(10) {
            Some(format!(
                "{label} {}s elapsed — still working; no confirmed completion of this operation; not automatically cancelled.",
                elapsed.as_secs()
            ))
        } else {
            Some(label.into())
        }
    }
    pub(super) fn preview_unavailable(&self) -> bool {
        self.editing.preview_error.is_some()
    }
    pub(super) fn edit_pending(&self) -> bool {
        self.editing.pending.is_some()
    }
    pub(super) fn document_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.menu_button("Document", |ui| self.document_menu_contents(ui, ctx));
    }
    fn document_menu_contents(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let can_open = self.loading_document.is_none()
            && self.editing.pending.is_none()
            && self.automation_rx.is_none();
        if ui
            .add_enabled(
                can_open,
                egui::Button::new("Open PDF…").shortcut_text("Ctrl+O"),
            )
            .clicked()
        {
            self.choose_pdf(ctx);
            ui.close();
        }
        ui.separator();
        let ready = self.project.document.is_some()
            && self.loading_document.is_none()
            && self.editing.pending.is_none()
            && self.automation_rx.is_none();
        let can_edit = ready && !self.editing.unrecoverable;
        if ui
            .add_enabled(
                can_edit && (self.editing.dirty || self.markup.text_draft.is_some()),
                egui::Button::new("Save").shortcut_text("Ctrl+S"),
            )
            .clicked()
        {
            self.start_edit(Command::Save, ctx);
            ui.close();
        }
        if ui
            .add_enabled(
                can_edit,
                egui::Button::new("Save As…").shortcut_text("Ctrl+Shift+S"),
            )
            .on_hover_text("Creates a new PDF. Existing destinations are never overwritten.")
            .clicked()
        {
            self.choose_save_as(ctx);
            ui.close();
        }
        ui.separator();
        if ui
            .add_enabled(
                can_edit
                    && self.markup.text_draft.is_none()
                    && self
                        .editing
                        .session
                        .as_ref()
                        .is_some_and(EditablePdf::can_undo),
                egui::Button::new("Undo").shortcut_text("Ctrl+Z"),
            )
            .clicked()
        {
            self.start_edit(Command::Undo, ctx);
            ui.close();
        }
        if ui
            .add_enabled(
                can_edit
                    && self.markup.text_draft.is_none()
                    && self
                        .editing
                        .session
                        .as_ref()
                        .is_some_and(EditablePdf::can_redo),
                egui::Button::new("Redo").shortcut_text("Ctrl+Shift+Z"),
            )
            .clicked()
        {
            self.start_edit(Command::Redo, ctx);
            ui.close();
        }
        ui.separator();
        if ui.add_enabled(can_edit, egui::Button::new("Rename current page label…")).on_hover_text("Edit the embedded PDF label for this page. Page content and bookmarks remain unchanged.").clicked() {
            self.request_page_label(self.project.selected_page, ctx);
            ui.close();
        }
        ui.separator();
        if ui
            .add_enabled(
                ready,
                egui::Button::new("Close document").shortcut_text("Ctrl+W"),
            )
            .clicked()
        {
            self.request_close_document(ctx);
            ui.close();
        }
    }

    fn save_as_with_picker(
        &mut self,
        ctx: &egui::Context,
        picker: impl FnOnce(rfd::FileDialog) -> Option<PathBuf>,
    ) {
        self.save_as_with_picker_intent(ctx, picker, false);
    }
    fn save_as_with_picker_intent(
        &mut self,
        ctx: &egui::Context,
        picker: impl FnOnce(rfd::FileDialog) -> Option<PathBuf>,
        defer: bool,
    ) {
        if self.request_inline_text_save(markup::SaveIntent::SaveAs, ctx) {
            return;
        }
        if self.editing.unrecoverable {
            self.status = self.editing.error.clone().unwrap_or_default();
            return;
        }
        if (self.editing.pending.is_some() && !defer)
            || self.loading_document.is_some()
            || self.automation_rx.is_some()
        {
            self.status = "Wait for the current operation.".into();
            return;
        }
        let Some(document) = &self.project.document else {
            return;
        };
        let stem = document
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("drawing");
        let mut dialog = rfd::FileDialog::new()
            .add_filter("PDF", &["pdf"])
            .set_file_name(format!("{stem}.edited.pdf"));
        if let Some(parent) = document.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            dialog = dialog.set_directory(parent);
        }
        if let Some(mut path) = picker(dialog) {
            if path.extension().is_none_or(|ext| ext.is_empty()) {
                path.set_extension("pdf");
            }
            if !path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
            {
                self.status = "Save As requires a .pdf filename. Choose a PDF destination.".into();
                self.editing.error = Some(self.status.clone());
                return;
            }
            if defer {
                self.defer_save(Command::SaveAs(path), ctx);
            } else {
                self.start_edit(Command::SaveAs(path), ctx);
            }
        }
    }
    fn choose_save_as(&mut self, ctx: &egui::Context) {
        if self.request_inline_text_save(markup::SaveIntent::SaveAs, ctx) {
            return;
        }
        // Substitute only the OS dialog in headless production-draw tests.
        #[cfg(test)]
        if let Some(destination) =
            ctx.data_mut(|d| d.remove_temp::<Option<PathBuf>>(egui::Id::new("test-save-as-picker")))
        {
            self.save_as_with_picker(ctx, |_| destination);
            return;
        }
        self.save_as_with_picker(ctx, rfd::FileDialog::save_file);
    }

    // Only defer commands that can race a finishing canvas event or a worker.
    // An unfinished press is never converted into document geometry by Save.
    fn save_must_wait_for_frame(&self, ctx: &egui::Context) -> bool {
        self.editing.pending.is_some()
            || ctx.input(|i| {
                i.events.iter().any(|event| {
                    matches!(
                        event,
                        egui::Event::PointerButton {
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            ..
                        }
                    )
                })
            })
    }
    fn defer_save(&mut self, command: Command, ctx: &egui::Context) {
        if self.editing_modal_open()
            || self.editing.unrecoverable
            || self.loading_document.is_some()
            || self.automation_rx.is_some()
        {
            return;
        }
        if let Some(document) = &self.project.document {
            self.editing.deferred_save = Some(DeferredSave {
                generation: self.document_generation,
                path: document.path.clone(),
                command,
                inline_intent: None,
            });
            ctx.request_repaint();
        }
    }
    pub(super) fn finish_save_intent(&mut self, ctx: &egui::Context) {
        let Some(request) = self.editing.deferred_save.as_ref() else {
            return;
        };
        if request.generation != self.document_generation
            || self
                .project
                .document
                .as_ref()
                .is_none_or(|d| d.path != request.path)
            || (self.editing_modal_open() && !self.editing.save_before_transition)
            || self.editing.unrecoverable
            || self.loading_document.is_some()
            || self.automation_rx.is_some()
        {
            self.editing.deferred_save = None;
            return;
        }
        if self.editing.pending.is_some() {
            return;
        }
        let request = self.editing.deferred_save.take().unwrap();
        if matches!(request.inline_intent, Some(markup::SaveIntent::SaveAs)) {
            self.choose_save_as(ctx);
            return;
        }
        self.start_edit(request.command, ctx);
    }
    pub(super) fn handle_edit_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        self.handle_edit_shortcuts_with_picker(ctx, rfd::FileDialog::save_file)
    }
    fn handle_edit_shortcuts_with_picker(
        &mut self,
        ctx: &egui::Context,
        picker: impl FnOnce(rfd::FileDialog) -> Option<PathBuf>,
    ) -> bool {
        if ctx.input_mut(|i| {
            i.consume_key(
                egui::Modifiers {
                    shift: true,
                    ..egui::Modifiers::COMMAND
                },
                egui::Key::S,
            )
        }) {
            self.save_as_with_picker_intent(ctx, picker, self.save_must_wait_for_frame(ctx));
            return true;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::W)) {
            self.request_close_document(ctx);
            return true;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            if self.save_must_wait_for_frame(ctx) {
                self.defer_save(Command::Save, ctx);
            } else {
                self.start_edit(Command::Save, ctx);
            }
            return true;
        }
        if !ctx.egui_wants_keyboard_input()
            && ctx.input_mut(|i| {
                i.consume_key(
                    egui::Modifiers {
                        shift: true,
                        ..egui::Modifiers::COMMAND
                    },
                    egui::Key::Z,
                )
            })
        {
            self.start_edit(Command::Redo, ctx);
            return true;
        }
        if !ctx.egui_wants_keyboard_input()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z))
        {
            self.start_edit(Command::Undo, ctx);
            return true;
        }
        false
    }

    pub(super) fn editing_modal_open(&self) -> bool {
        self.editing.rename.is_some() || self.editing.transition.is_some()
    }
    pub(super) fn page_edit_menu(&mut self, response: &egui::Response, page: usize) {
        let can_edit = self
            .project
            .document
            .as_ref()
            .is_some_and(|d| page < d.summary.pages.len())
            && self.editing.pending.is_none()
            && !self.editing.unrecoverable
            && !self.editing_modal_open()
            && self.loading_document.is_none()
            && self.automation_rx.is_none();
        response.context_menu(|ui| {
            if ui
                .add_enabled(can_edit, egui::Button::new("Rename page label..."))
                .on_hover_text(
                    "Edit this thumbnail's embedded PDF label without changing the displayed page.",
                )
                .clicked()
            {
                self.request_page_label(page, ui.ctx());
                ui.close();
            }
        });
    }
    pub(super) fn bookmark_edit_menu(&mut self, response: &egui::Response, index: usize) {
        response.context_menu(|ui| {
            if ui
                .add_enabled(
                    self.editing.pending.is_none()
                        && !self.editing.unrecoverable
                        && self.loading_document.is_none()
                        && self.automation_rx.is_none(),
                    egui::Button::new("Rename"),
                )
                .clicked()
            {
                if let Some(b) = self
                    .project
                    .document
                    .as_ref()
                    .and_then(|d| d.summary.bookmarks.get(index))
                {
                    self.editing.rename = Some(RenameDialog {
                        target: RenameTarget::Bookmark,
                        index,
                        original: b.title.clone(),
                        value: b.title.clone(),
                        focus: true,
                    });
                    ui.ctx().request_repaint();
                }
                ui.close();
            }
        });
    }
    pub(super) fn editing_dialogs(&mut self, ctx: &egui::Context) {
        self.unsaved_dialog(ctx);
        if let Some(mut dialog) = self.editing.rename.take() {
            let mut apply = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::new("rename_bookmark")).show(ctx, |ui| {
                ui.set_min_width(340.);
                ui.heading(if dialog.target == RenameTarget::PageLabel {
                    "Rename page label"
                } else {
                    "Rename bookmark"
                });
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
                let mut field_output = egui::TextEdit::singleline(&mut dialog.value)
                    .desired_width(340.)
                    .show(ui);
                if dialog.focus {
                    field_output.response.request_focus();
                    field_output
                        .state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::two(
                            egui::text::CCursor::new(0),
                            egui::text::CCursor::new(dialog.value.chars().count()),
                        )));
                    field_output.state.store(ui.ctx(), field_output.response.id);
                    dialog.focus = false;
                }
                let field = field_output.response;
                ui.label(if dialog.target == RenameTarget::PageLabel {
                    "Changes the embedded PDF label, not page content or its position."
                } else {
                    "Hierarchy and destination are preserved."
                });
                let limit = if dialog.target == RenameTarget::PageLabel {
                    crate::pdf::MAX_PAGE_LABEL_CHARS
                } else {
                    EditablePdf::MAX_BOOKMARK_TITLE_CHARS
                };
                let within_limit = dialog.value.trim().chars().take(limit + 1).count() <= limit;
                if !within_limit {
                    ui.colored_label(
                        egui::Color32::LIGHT_RED,
                        format!(
                            "Names are limited to {limit} characters. Your draft is preserved."
                        ),
                    );
                }
                ui.horizontal(|ui| {
                    let valid = !dialog.value.trim().is_empty() && within_limit;
                    apply = ui.add_enabled(valid, egui::Button::new("Rename")).clicked()
                        || (valid && enter && (field.has_focus() || field.lost_focus()));
                    cancel |= ui.button("Cancel").clicked();
                });
            });
            if apply {
                self.start_edit(
                    if dialog.target == RenameTarget::PageLabel {
                        Command::PageLabel {
                            index: dialog.index,
                            original: dialog.original,
                            title: dialog.value,
                        }
                    } else {
                        Command::Rename {
                            index: dialog.index,
                            original: dialog.original,
                            title: dialog.value,
                        }
                    },
                    ctx,
                );
            } else if !cancel {
                self.editing.rename = Some(dialog);
            }
        }
    }
    fn unsaved_dialog(&mut self, ctx: &egui::Context) {
        if self.editing.transition.is_none() {
            return;
        }
        let busy = self.editing.pending.is_some();
        let mut choice = None;
        egui::Modal::new(egui::Id::new("unsaved_document")).show(ctx, |ui| {
            ui.set_min_width(340.);
            ui.heading("Unsaved changes");
            if busy {
                ui.spinner();
                ui.label("Saving safely…");
            } else {
                ui.label("Save your document changes before continuing?");
            }
            if let Some(error) = &self.editing.error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !busy && !self.editing.unrecoverable,
                        egui::Button::new("Save"),
                    )
                    .clicked()
                {
                    choice = Some(Decision::Save);
                }
                if ui
                    .add_enabled(!busy, egui::Button::new("Discard"))
                    .clicked()
                {
                    choice = Some(Decision::Discard);
                }
                if ui.add_enabled(!busy, egui::Button::new("Cancel")).clicked()
                    || (!busy && ui.input(|i| i.key_pressed(egui::Key::Escape)))
                {
                    choice = Some(Decision::Cancel);
                }
            });
        });
        if let Some(choice) = choice {
            self.resolve_unsaved(choice, ctx);
        }
    }
    pub(super) fn guard_window_close(&mut self, ctx: &egui::Context) {
        if !ctx.input(|i| i.viewport().close_requested()) {
            return;
        }
        if self.block_inline_text_transition() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            return;
        }
        if self.editing_modal_open() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.status = "Finish or cancel the current dialog before closing the window.".into();
            return;
        }
        if self.editing.pending.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.status = "Wait for the current edit/save operation to finish.".into();
        } else if self.editing.dirty {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.editing.transition = Some(Transition::Quit);
        }
    }
    fn request_close_document(&mut self, _ctx: &egui::Context) {
        if self.block_inline_text_transition() {
            return;
        }
        if self.editing.pending.is_some() || self.loading_document.is_some() {
            self.status = "Wait for the current document operation.".into();
            return;
        }
        if self.editing.dirty {
            self.editing.transition = Some(Transition::CloseDocument);
        } else {
            self.finish_transition(Transition::CloseDocument, _ctx);
        }
    }
    fn finish_transition(&mut self, transition: Transition, ctx: &egui::Context) {
        match transition {
            Transition::Open(path) => self.open_pdf(path, ctx),
            Transition::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Transition::CloseDocument => {
                self.cancel_search();
                self.search_hits.clear();
                self.selected_search_hit = None;
                self.document_generation = self.document_generation.wrapping_add(1).max(1);
                self.render_worker.reset(self.document_generation);
                self.project = ProjectState::new("Glyph Workspace");
                self.navigation_history = NavigationHistory::default();
                self.editing = EditingState::default();
                self.rendered_page = None;
                self.page_texture = None;
                self.rendered_tile = None;
                self.tile_texture = None;
                self.pending_page_render = None;
                self.pending_tile_render = None;
                self.pending_prefetch_pages.clear();
                self.page_cache.clear();
                self.page_texture_cache.clear();
                self.page_cache_order.clear();
                self.page_links.clear();
                self.link_cache.clear();
                self.link_cache_order.clear();
                self.pending_links = None;
                self.thumbnails = thumbnails::ThumbnailState::default();
                self.clear_page_text();
                self.status = "Document closed.".into();
                ctx.request_repaint();
            }
        }
    }
    fn resolve_unsaved(&mut self, decision: Decision, _ctx: &egui::Context) {
        if self.editing.pending.is_some() {
            return;
        }
        if let Decision::Cancel = decision {
            self.editing.transition = None;
            self.editing.save_before_transition = false;
            return;
        }
        if let Decision::Save = decision {
            if self.editing.unrecoverable {
                self.editing.save_before_transition = false;
                return;
            }
            if self.editing.transition.is_none() {
                return;
            }
            if self.markup.text_draft.is_some() {
                let transition = self.editing.transition.take();
                // The decision owns input, so there is no editable text widget
                // in this frame. Validate/commit its retained draft exactly once.
                if !self.finish_inline_text(_ctx) {
                    self.editing.save_before_transition = false;
                    return; // invalid draft returns to its editable surface
                }
                self.defer_save(Command::Save, _ctx);
                self.editing.transition = transition;
                self.editing.save_before_transition = true;
                return;
            }
            self.editing.save_before_transition = true;
            self.start_edit(Command::Save, _ctx);
            return;
        }
        if let Decision::Discard = decision {
            let Some(transition) = self.editing.transition.take() else {
                return;
            };
            if let (Some(document), Some(bookmarks)) = (
                self.project.document.as_mut(),
                self.editing.saved_bookmarks.take(),
            ) {
                document.summary.bookmarks = bookmarks;
            }
            if let (Some(document), Some(pages)) = (
                self.project.document.as_mut(),
                self.editing.saved_pages.take(),
            ) {
                document.summary.pages = pages;
            }
            self.editing = EditingState::default();
            self.markup = markup::MarkupState::default();
            self.refresh_markup_render(_ctx, None);
            self.finish_transition(transition, _ctx);
        }
    }
    fn start_edit(&mut self, command: Command, ctx: &egui::Context) {
        if matches!(command, Command::Save)
            && self.request_inline_text_save(markup::SaveIntent::Save, ctx)
        {
            return;
        }
        self.start_edit_with_serializer(command, ctx, EditablePdf::render_snapshot);
    }
    fn start_edit_with_serializer(
        &mut self,
        command: Command,
        ctx: &egui::Context,
        serialize: impl FnOnce(&EditablePdf) -> Result<Vec<u8>, PdfError> + Send + 'static,
    ) {
        if matches!(command, Command::Undo | Command::Redo) && self.markup.text_draft.is_some() {
            self.status =
                "Apply text or Cancel the text draft before using document Undo/Redo.".into();
            return;
        }
        if let Command::UpdateShape(shape) = &command
            && (!self.can_change_markups()
                || shape.page_index != self.project.selected_page
                || !self
                    .markup
                    .items
                    .iter()
                    .any(|a| a.object_id == shape.object_id && a.page_index == shape.page_index))
        {
            return;
        }
        if let Command::Line { page, .. } = &command
            && (!self.can_change_markups() || *page != self.project.selected_page)
        {
            return;
        }
        if let Command::Text { page, .. } = &command
            && (!self.can_change_markups()
                || *page != self.project.selected_page
                || self.markup.text_draft.is_some())
        {
            return;
        }
        if self.render_worker_failed() && !matches!(&command, Command::Save | Command::SaveAs(_)) {
            self.status = RENDER_WORKER_FAILURE.into();
            return;
        }
        if self.preview_unavailable()
            && matches!(
                &command,
                Command::Rectangle { .. }
                    | Command::Ellipse { .. }
                    | Command::Line { .. }
                    | Command::DeleteShape(_)
            )
        {
            self.status = self.editing.preview_error.clone().unwrap();
            return;
        }
        if self.editing.unrecoverable {
            self.status = self.editing.error.clone().unwrap_or_else(|| {
                "Editing snapshot cannot be recovered; saving is blocked.".into()
            });
            return;
        }
        if self.editing.pending.is_some()
            || self.loading_document.is_some()
            || self.automation_rx.is_some()
        {
            self.status = "Wait for the current document operation to finish.".into();
            return;
        }

        if matches!(&command, Command::Undo)
            && !self
                .editing
                .session
                .as_ref()
                .is_some_and(EditablePdf::can_undo)
        {
            self.status = "Nothing to undo.".into();
            return;
        }
        if matches!(&command, Command::Redo)
            && !self
                .editing
                .session
                .as_ref()
                .is_some_and(EditablePdf::can_redo)
        {
            self.status = "Nothing to redo.".into();
            return;
        }
        if matches!(&command, Command::Save) && !self.editing.dirty {
            self.status = "No unsaved changes.".into();
            return;
        }
        let Some(document) = self.project.document.as_ref() else {
            return;
        };
        let object_id = match &command {
            Command::Rename { index, .. } => document
                .summary
                .bookmarks
                .get(*index)
                .and_then(|b| b.object_id),
            _ => None,
        };
        let path = document.path.clone();
        let generation = self.document_generation;
        if self.editing.saved_bookmarks.is_none() {
            self.editing.saved_bookmarks = Some(document.summary.bookmarks.clone());
            self.editing.saved_pages = Some(document.summary.pages.clone());
        }
        let mut session = self.editing.session.take();
        let (sender, receiver) = mpsc::sync_channel(1);
        let kind = match &command {
            Command::Save => EditKind::Save,
            Command::SaveAs(_) => EditKind::SaveAs,
            Command::LoadMarkups => EditKind::LoadMarkups,
            _ => EditKind::Edit,
        };
        self.editing.saved_preview = None;
        self.editing.pending = Some(PendingEdit {
            kind,
            started: Instant::now(),
            generation,
            path: path.clone(),
            receiver,
        });
        self.status = match &command {
            Command::Save => "Saving PDF…",
            Command::SaveAs(_) => "Saving PDF copy…",
            Command::LoadMarkups => "Loading markups…",
            _ => "Editing document…",
        }
        .into();
        let ctx = ctx.clone();
        let retry_preview = self.preview_unavailable();
        let creation_style = self.markup.next_style;
        thread::spawn(move || {
            let is_save = matches!(&command, Command::Save | Command::SaveAs(_));
            let before = session
                .as_ref()
                .map(EditablePdf::shapes)
                .unwrap_or_default();
            let load_markups = matches!(&command, Command::LoadMarkups);
            let result = (|| {
                if session.is_none() {
                    session = Some(EditablePdf::open(&path)?);
                }
                let editor = session.as_mut().unwrap();
                match command {
                    Command::Text {
                        page,
                        rect,
                        text,
                        style,
                    } => editor.add_text(page, rect, &text.contents, text.size, style),
                    Command::LoadMarkups => Ok(false),
                    Command::Rectangle { page, rect } => editor.add_shape_styled(
                        page,
                        rect,
                        crate::pdf::ShapeKind::Rectangle,
                        creation_style,
                    ),
                    Command::Ellipse { page, rect } => editor.add_shape_styled(
                        page,
                        rect,
                        crate::pdf::ShapeKind::Ellipse,
                        creation_style,
                    ),
                    Command::Line {
                        page,
                        endpoints,
                        arrow,
                    } => editor.add_line_styled(page, endpoints, arrow, creation_style),
                    Command::UpdateShape(shape) => editor.update_shape(&shape),
                    Command::DeleteShape(id) => editor.delete_shape(id),
                    Command::Rename {
                        index: _,
                        original,
                        title,
                    } => {
                        let bookmark=editor.bookmarks().into_iter().find(|b|Some(b.object_id)==object_id)
                            .ok_or_else(||PdfError::Edit("Bookmark identity missing or changed. Reopen the PDF before editing.".into()))?;
                        if bookmark.title.trim() != original.trim() {
                            return Err(PdfError::Edit(
                                "The bookmark changed on disk. Reopen the PDF before editing."
                                    .into(),
                            ));
                        }
                        editor.rename_bookmark(bookmark.index, &title)
                    }
                    Command::PageLabel {
                        index,
                        original,
                        title,
                    } => {
                        if editor
                            .page_labels()
                            .get(index)
                            .is_none_or(|label| label.trim() != original.trim())
                        {
                            return Err(PdfError::Edit(
                                "The page label changed on disk. Reopen the PDF before editing."
                                    .into(),
                            ));
                        }
                        editor.set_page_label(index, &title)
                    }
                    Command::Undo => Ok(editor.undo()),
                    Command::Redo => Ok(editor.redo()),
                    Command::Save => editor.save().map(|_| true),
                    Command::SaveAs(path) => editor.save_as(&path).map(|_| true),
                }
            })();
            let bookmarks = session.as_ref().map(|s| s.bookmarks()).unwrap_or_default();
            let page_labels = session
                .as_ref()
                .map(|s| s.page_labels())
                .unwrap_or_default();
            let shapes = session
                .as_ref()
                .map(EditablePdf::shapes)
                .unwrap_or_default();
            let mut preview_error = None;
            let snapshot = if result.is_ok()
                && !is_save
                && (retry_preview || load_markups || shapes != before)
            {
                match serialize(session.as_ref().unwrap()) {
                    Ok(bytes) => Some(bytes),
                    Err(e) => {
                        preview_error = Some(e);
                        None
                    }
                }
            } else {
                None
            };
            let dirty = session.as_ref().is_some_and(EditablePdf::is_dirty);
            let _ = sender.send(EditCompletion {
                dirty,
                session,
                saved: is_save && result.is_ok(),
                shapes,
                snapshot,
                preview_error,
                result,
                bookmarks,
                page_labels,
            });
            ctx.request_repaint();
        });
    }
    pub(super) fn apply_edit_results(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.editing.pending.as_ref() else {
            return;
        };
        let outcome = match pending.receiver.try_recv() {
            Ok(outcome) => outcome,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                let owned = self.editing_progress_owned(pending.generation, &pending.path);
                self.editing.pending = None;
                if !owned {
                    return;
                }
                self.editing.saved_preview = None;
                self.markup.preview_pending = false;
                self.editing.unrecoverable = true;
                self.editing.error=Some("Document worker failed. The editing snapshot cannot be recovered; saving is blocked. Discard and reopen the PDF to continue.".into());
                self.status = self.editing.error.clone().unwrap();
                return;
            }
        };
        let pending = self.editing.pending.take().unwrap();
        if pending.generation != self.document_generation
            || self.loading_document.is_some()
            || self
                .project
                .document
                .as_ref()
                .is_none_or(|d| d.path != pending.path)
        {
            return;
        }
        if outcome.result.is_ok() {
            self.editing.error = None;
        }
        self.editing.dirty = outcome.dirty;
        if outcome.result.is_ok()
            && let Some(document) = self.project.document.as_mut()
        {
            // These operations only change titles, never destinations or hierarchy.
            // Preserve navigation forms already resolved by the inspector.
            let titles: std::collections::HashMap<_, _> = outcome
                .bookmarks
                .into_iter()
                .map(|b| (b.object_id, b.title))
                .collect();
            for bookmark in &mut document.summary.bookmarks {
                if let Some(title) = bookmark.object_id.and_then(|id| titles.get(&id)) {
                    bookmark.title = title.trim().to_owned();
                }
            }
        }
        if outcome.result.is_ok()
            && let Some(document) = self.project.document.as_mut()
            && outcome.page_labels.len() == document.summary.pages.len()
        {
            for (page, label) in document.summary.pages.iter_mut().zip(outcome.page_labels) {
                page.label = Some(label);
            }
        }
        if outcome.saved {
            self.editing.preview_error = None;
            self.markup.loaded = true;
            self.markup.preview_pending = true;
            if let (Some(document), Some(session)) =
                (self.project.document.as_mut(), outcome.session.as_ref())
            {
                document.path = session.path().to_owned();
                self.editing.saved_bookmarks = Some(document.summary.bookmarks.clone());
                self.editing.saved_pages = Some(document.summary.pages.clone());
            }
            self.document_generation = self.document_generation.wrapping_add(1).max(1);
            self.editing.saved_preview =
                self.project.document.as_ref().map(|document| SavedPreview {
                    generation: self.document_generation,
                    path: document.path.clone(),
                    started: Instant::now(),
                });
            self.render_worker.reset(self.document_generation);
            self.page_cache.clear();
            self.page_texture_cache.clear();
            self.page_cache_order.clear();
            self.page_links.clear();
            self.link_cache.clear();
            self.link_cache_order.clear();
            self.pending_links = None;
            self.pending_page_render = None;
            self.pending_tile_render = None;
            self.pending_prefetch_pages.clear();
            self.rendered_tile = None;
            self.tile_texture = None;
            self.thumbnails = thumbnails::ThumbnailState::default();
            self.clear_page_text();
            self.cancel_search();
            self.search_hits.clear();
            self.selected_search_hit = None;
            self.render_selected_page(ctx, BASE_RENDER_WIDTH);
            self.queue_page_links(ctx);
            self.queue_page_text();
        }
        self.markup.items = outcome.shapes;
        if self
            .markup
            .selected
            .is_some_and(|id| !self.markup.items.iter().any(|a| a.object_id == id))
        {
            self.markup.selected = None;
        }
        if let Some(snapshot) = outcome.snapshot {
            self.markup.loaded = true;
            self.refresh_markup_render(ctx, Some(snapshot));
        }
        // A successful owned edit can advance the render generation. Carry only
        // its matching explicit Save intent forward; failures never auto-save.
        if let Some(request) = self.editing.deferred_save.as_mut() {
            if outcome.result.is_ok()
                && request.generation == pending.generation
                && request.path == pending.path
            {
                request.generation = self.document_generation;
                // Save As changes the path of this same retained document, not
                // its identity. A subsequent explicit request still belongs to it.
                if let Some(document) = &self.project.document {
                    request.path = document.path.clone();
                }
            } else {
                self.editing.deferred_save = None;
            }
        }
        self.editing.session = outcome.session;
        if outcome.result.is_err() {
            self.editing.save_before_transition = false;
        }
        self.status = match outcome.result {
            Ok(true) if outcome.saved => "Saved PDF — verified and committed.".into(),
            Ok(true) if self.editing.dirty => "Document updated — unsaved changes.".into(),
            Ok(true) => "Document updated.".into(),
            Ok(false) => "No document change.".into(),
            Err(err) => {
                let message = format!("Document edit failed: {err}");
                self.editing.error = Some(message.clone());
                message
            }
        };
        if let Some(error) = outcome.preview_error {
            self.preview_failed(error);
        }
        if self.render_worker_failed() && self.markup.preview_pending {
            self.preview_failed(RENDER_WORKER_FAILURE);
        }
        if outcome.saved && self.editing.save_before_transition {
            self.editing.save_before_transition = false;
            if let Some(transition) = self.editing.transition.take() {
                self.finish_transition(transition, ctx);
            }
        }
        ctx.request_repaint();
    }
    pub(super) fn defer_document_open(&mut self, path: PathBuf) -> bool {
        if self.block_inline_text_transition() {
            return true;
        }
        if self.editing_modal_open() {
            self.status = "Finish or cancel the current dialog before opening another PDF.".into();
            return true;
        }
        if self.editing.pending.is_some() {
            self.status = "Wait for the current edit/save operation to finish.".into();
            return true;
        }
        if self.editing.dirty {
            self.editing.transition = Some(Transition::Open(path));
            return true;
        }
        false
    }
}

#[cfg(test)]
#[path = "operation_ui_tests.rs"]
mod operation_ui_tests;

#[cfg(test)]
mod tests {
    use super::*;
    include!("line_arrow_tests.rs");
    include!("line_arrow_boundary_tests.rs");
    include!("editable_markup_tests.rs");
    include!("text_markup_tests.rs");
    fn finishing_frame(app: &mut GlyphApp, ctx: &egui::Context, events: Vec<egui::Event>) {
        finishing_frame_with_picker(app, ctx, events, |_| panic!("unexpected native picker"));
    }
    fn finishing_frame_with_picker(
        app: &mut GlyphApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        mut picker: impl FnMut(rfd::FileDialog) -> Option<PathBuf>,
    ) {
        let mut out = ctx.run_ui(
            egui::RawInput {
                max_texture_side: Some(8192),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800., 600.),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                if !app.handle_edit_shortcuts_with_picker(ctx, &mut picker) {
                    app.handle_shortcuts(ctx);
                }
                let (page, response) =
                    ui.allocate_exact_size(egui::vec2(400., 300.), egui::Sense::click_and_drag());
                app.interact_with_markup(ui, &response, page, page, 1);
                app.finish_save_intent(ctx);
            },
        );
        out.textures_delta.clear();
    }
    fn save_key(shift: bool) -> egui::Event {
        egui::Event::Key {
            key: egui::Key::S,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                shift,
                ..egui::Modifiers::COMMAND
            },
        }
    }
    #[test]
    fn finishing_release_and_save_same_frame_retains_both_shapes() {
        for ellipse in [false, true] {
            for dirty in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("finishing.pdf");
                fixture(&path);
                let ctx = egui::Context::default();
                let mut app = setup(&path, &ctx);
                app.add_rectangle(
                    1,
                    crate::core::links::PdfRect {
                        x: 0.1,
                        y: 0.1,
                        width: 0.2,
                        height: 0.2,
                    },
                    &ctx,
                );
                settle(&mut app, &ctx);
                if !dirty {
                    app.start_edit(Command::Save, &ctx);
                    settle(&mut app, &ctx);
                }
                let before = std::fs::read(&path).unwrap();
                app.markup.loaded = true;
                app.markup.preview_pending = false;
                app.markup.mode = if ellipse {
                    markup::Mode::Ellipse
                } else {
                    markup::Mode::Rectangle
                };
                finishing_frame(&mut app, &ctx, vec![]);
                finishing_frame(&mut app, &ctx, vec![pointer(egui::pos2(60., 60.), true)]);
                assert_eq!(std::fs::read(&path).unwrap(), before);
                finishing_frame(
                    &mut app,
                    &ctx,
                    vec![pointer(egui::pos2(180., 180.), false), save_key(false)],
                );
                settle(&mut app, &ctx);
                assert_eq!(
                    app.markup.items.len(),
                    2,
                    "ellipse={ellipse}, dirty={dirty}: {}",
                    app.status
                );
                assert_eq!(
                    EditablePdf::open(&path).unwrap().shapes().len(),
                    2,
                    "finishing stroke must be persisted"
                );
                assert!(!app.editing.dirty);
            }
        }
    }
    #[test]
    fn finishing_release_and_save_as_same_frame_includes_shape_or_retains_it_on_cancel() {
        for ellipse in [false, true] {
            for dirty in [false, true] {
                for cancel in [false, true] {
                    let dir = tempfile::tempdir().unwrap();
                    let path = dir.path().join("source.pdf");
                    let copy = dir.path().join("copy.pdf");
                    fixture(&path);
                    let ctx = egui::Context::default();
                    let mut app = setup(&path, &ctx);
                    app.add_rectangle(
                        1,
                        crate::core::links::PdfRect {
                            x: 0.1,
                            y: 0.1,
                            width: 0.2,
                            height: 0.2,
                        },
                        &ctx,
                    );
                    settle(&mut app, &ctx);
                    if !dirty {
                        app.start_edit(Command::Save, &ctx);
                        settle(&mut app, &ctx);
                    }
                    let before = std::fs::read(&path).unwrap();
                    app.markup.loaded = true;
                    app.markup.preview_pending = false;
                    app.markup.mode = if ellipse {
                        markup::Mode::Ellipse
                    } else {
                        markup::Mode::Rectangle
                    };
                    finishing_frame(&mut app, &ctx, vec![]);
                    finishing_frame(&mut app, &ctx, vec![pointer(egui::pos2(60., 60.), true)]);
                    let mut picked = false;
                    finishing_frame_with_picker(
                        &mut app,
                        &ctx,
                        vec![pointer(egui::pos2(180., 180.), false), save_key(true)],
                        |_| {
                            picked = true;
                            if cancel { None } else { Some(copy.clone()) }
                        },
                    );
                    assert!(picked);
                    settle(&mut app, &ctx);
                    assert_eq!(
                        app.markup.items.len(),
                        2,
                        "ellipse={ellipse}, dirty={dirty}, cancel={cancel}: {}",
                        app.status
                    );
                    assert_eq!(
                        std::fs::read(&path).unwrap(),
                        before,
                        "Save As must not write the source"
                    );
                    if cancel {
                        assert!(!copy.exists());
                        assert!(app.editing.dirty);
                        assert_eq!(app.project.document.as_ref().unwrap().path, path);
                    } else {
                        assert_eq!(EditablePdf::open(&copy).unwrap().shapes().len(), 2);
                        assert!(!app.editing.dirty);
                        assert_eq!(app.project.document.as_ref().unwrap().path, copy);
                    }
                }
            }
        }
    }
    #[test]
    fn finishing_save_intent_waits_for_real_edit_worker_without_writing_early() {
        for save_as in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("pending.pdf");
            let copy = dir.path().join("copy.pdf");
            fixture(&path);
            let before = std::fs::read(&path).unwrap();
            let ctx = egui::Context::default();
            let mut app = setup(&path, &ctx);
            let (release, wait) = mpsc::channel();
            app.start_edit_with_serializer(
                Command::Ellipse {
                    page: 1,
                    rect: crate::core::links::PdfRect {
                        x: 0.1,
                        y: 0.1,
                        width: 0.2,
                        height: 0.2,
                    },
                },
                &ctx,
                move |session| {
                    wait.recv_timeout(Duration::from_secs(5)).unwrap();
                    session.render_snapshot()
                },
            );
            finishing_frame_with_picker(&mut app, &ctx, vec![save_key(save_as)], |_| {
                Some(copy.clone())
            });
            assert!(app.editing.deferred_save.is_some());
            assert_eq!(std::fs::read(&path).unwrap(), before);
            assert!(!copy.exists());
            release.send(()).unwrap();
            settle(&mut app, &ctx);
            let saved = if save_as { &copy } else { &path };
            assert_eq!(EditablePdf::open(saved).unwrap().shapes().len(), 1);
            assert!(!app.editing.dirty);
            if save_as {
                assert_eq!(std::fs::read(&path).unwrap(), before);
            }
        }
    }
    #[test]
    fn finishing_save_as_intent_survives_an_inflight_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending-save.pdf");
        let copy = dir.path().join("copy.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        app.add_rectangle(
            1,
            crate::core::links::PdfRect {
                x: 0.1,
                y: 0.1,
                width: 0.2,
                height: 0.2,
            },
            &ctx,
        );
        settle(&mut app, &ctx);
        app.start_edit(Command::Save, &ctx);
        finishing_frame_with_picker(&mut app, &ctx, vec![save_key(true)], |_| Some(copy.clone()));
        assert!(app.editing.deferred_save.is_some());
        settle(&mut app, &ctx);
        assert!(
            copy.exists(),
            "explicit Save As intent must survive current Save completion"
        );
        assert_eq!(EditablePdf::open(&copy).unwrap().shapes().len(), 1);
        assert_eq!(app.project.document.as_ref().unwrap().path, copy);
        assert!(!app.editing.dirty);
    }
    #[test]
    fn finishing_press_only_save_never_fabricates_uncommitted_geometry() {
        for ellipse in [false, true] {
            for dirty in [false, true] {
                for save_as in [false, true] {
                    let dir = tempfile::tempdir().unwrap();
                    let path = dir.path().join("press.pdf");
                    let copy = dir.path().join("copy.pdf");
                    fixture(&path);
                    let ctx = egui::Context::default();
                    let mut app = setup(&path, &ctx);
                    if dirty {
                        app.add_rectangle(
                            1,
                            crate::core::links::PdfRect {
                                x: 0.1,
                                y: 0.1,
                                width: 0.2,
                                height: 0.2,
                            },
                            &ctx,
                        );
                        settle(&mut app, &ctx);
                    }
                    app.markup.loaded = true;
                    app.markup.preview_pending = false;
                    app.markup.mode = if ellipse {
                        markup::Mode::Ellipse
                    } else {
                        markup::Mode::Rectangle
                    };
                    finishing_frame(&mut app, &ctx, vec![]);
                    finishing_frame_with_picker(
                        &mut app,
                        &ctx,
                        vec![pointer(egui::pos2(60., 60.), true), save_key(save_as)],
                        |_| Some(copy.clone()),
                    );
                    settle(&mut app, &ctx);
                    let saved = if save_as { &copy } else { &path };
                    assert_eq!(
                        EditablePdf::open(saved).unwrap().shapes().len(),
                        usize::from(dirty)
                    );
                    assert!(!app.editing.dirty);
                }
            }
        }
    }
    #[test]
    fn finishing_save_intent_drops_on_failed_edit_stale_identity_or_modal_transition() {
        for guard in 0..5 {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("guard.pdf");
            fixture(&path);
            let before = std::fs::read(&path).unwrap();
            let ctx = egui::Context::default();
            let mut app = setup(&path, &ctx);
            app.add_rectangle(
                if guard == 0 { 99 } else { 1 },
                crate::core::links::PdfRect {
                    x: 0.1,
                    y: 0.1,
                    width: 0.2,
                    height: 0.2,
                },
                &ctx,
            );
            finishing_frame(&mut app, &ctx, vec![save_key(false)]);
            assert!(app.editing.deferred_save.is_some());
            match guard {
                1 => app.document_generation += 1,
                2 => {
                    app.project.document.as_mut().unwrap().path = dir.path().join("replacement.pdf")
                }
                3 => app.editing.transition = Some(Transition::CloseDocument),
                4 => {
                    app.editing.rename = Some(RenameDialog {
                        target: RenameTarget::PageLabel,
                        index: 1,
                        original: "2".into(),
                        value: "draft".into(),
                        focus: true,
                    })
                }
                _ => {}
            }
            settle(&mut app, &ctx);
            assert!(app.editing.deferred_save.is_none());
            assert_eq!(std::fs::read(&path).unwrap(), before);
            assert_eq!(EditablePdf::open(&path).unwrap().shapes().len(), 0);
            if guard == 0 {
                assert!(app.editing.error.is_some());
            }
        }
    }
    #[test]
    fn finishing_real_draw_frames_save_after_canvas_release() {
        for ellipse in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("draw.pdf");
            fixture(&path);
            let ctx = egui::Context::default();
            let mut app = setup(&path, &ctx);
            app.add_rectangle(
                1,
                crate::core::links::PdfRect {
                    x: 0.1,
                    y: 0.1,
                    width: 0.2,
                    height: 0.2,
                },
                &ctx,
            );
            settle(&mut app, &ctx);
            // Real renderer installation, not a fabricated raster or copied UI.
            let mut warm = ctx.run_ui(
                egui::RawInput {
                    max_texture_side: Some(8192),
                    ..Default::default()
                },
                |_| {},
            );
            warm.textures_delta.clear();
            wait_preview(&mut app, &ctx);
            app.zoom = 0.5;
            app.pan = egui::Vec2::ZERO;
            app.markup.mode = if ellipse {
                markup::Mode::Ellipse
            } else {
                markup::Mode::Rectangle
            };
            let mut page = None;
            for _ in 0..3 {
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        max_texture_side: Some(8192),
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1200., 1000.),
                        )),
                        ..Default::default()
                    },
                    |ui| app.draw(ui),
                );
                let texture = app.page_texture.as_ref().unwrap().id();
                page = out.shapes.iter().find_map(|s| match &s.shape {
                    egui::epaint::Shape::Mesh(mesh) if mesh.texture_id == texture => {
                        Some(mesh.calc_bounds())
                    }
                    _ => None,
                });
                out.textures_delta.clear();
            }
            let page = page.expect("real draw must paint installed PDF page");
            for events in [
                vec![pointer(page.center() - egui::vec2(40., 40.), true)],
                vec![
                    pointer(page.center() + egui::vec2(40., 40.), false),
                    save_key(false),
                ],
            ] {
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        max_texture_side: Some(8192),
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1200., 1000.),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| app.draw(ui),
                );
                out.textures_delta.clear();
            }
            let deadline = Instant::now() + Duration::from_secs(5);
            while (app.edit_pending() || app.editing.deferred_save.is_some())
                && Instant::now() < deadline
            {
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        max_texture_side: Some(8192),
                        ..Default::default()
                    },
                    |ui| app.draw(ui),
                );
                out.textures_delta.clear();
                thread::sleep(Duration::from_millis(2));
            }
            assert!(!app.edit_pending());
            assert!(app.editing.deferred_save.is_none());
            assert_eq!(app.markup.items.len(), 2, "{}", app.status);
            assert_eq!(EditablePdf::open(&path).unwrap().shapes().len(), 2);
            assert!(!app.editing.dirty);
        }
    }
    #[test]
    fn ellipse_shortcut_owned_drag_preview_worker_save_and_select_delete() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ellipse-ui.pdf");
        fixture(&path);
        let original = std::fs::read(&path).unwrap();
        let ctx = egui::Context::default();
        let mut frame = ctx.run_ui(
            egui::RawInput {
                max_texture_side: Some(8192),
                ..Default::default()
            },
            |_| {},
        );
        frame.textures_delta.clear();
        let mut app = setup(&path, &ctx);
        app.load_markups(&ctx);
        settle(&mut app, &ctx);
        wait_preview(&mut app, &ctx);
        press(&mut app, &ctx, egui::Key::E, egui::Modifiers::NONE);
        assert!(
            app.markup.mode == markup::Mode::Ellipse,
            "E must choose native ellipse drawing"
        );
        let mut oval = false;
        let mut toolbar = false;
        for events in [
            vec![],
            vec![pointer(egui::pos2(80., 80.), true)],
            vec![egui::Event::PointerMoved(egui::pos2(240., 180.))],
            vec![pointer(egui::pos2(240., 180.), false)],
        ] {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800., 600.),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let (page, response) = ui
                        .allocate_exact_size(egui::vec2(400., 300.), egui::Sense::click_and_drag());
                    app.interact_with_markup(ui, &response, page, page, 1);
                    app.paint_markup(ui.painter(), page, 1);
                    app.draw_markup_tools(ui, &ctx);
                },
            );
            fn check(shape: &egui::epaint::Shape, oval: &mut bool, toolbar: &mut bool) {
                match shape {
                    egui::epaint::Shape::Ellipse(e) if e.radius.x > 20. => {
                        assert_eq!(e.fill, egui::Color32::TRANSPARENT);
                        let expected =
                            egui::Rect::from_two_pos(egui::pos2(80., 80.), egui::pos2(240., 180.));
                        // Normalized screen round-trip introduces subpixel f32 error.
                        assert!(
                            (e.center - expected.center()).length() < 0.0001,
                            "ghost stays in display coordinates"
                        );
                        assert!(
                            (e.radius
                                - (expected.size() / 2. - egui::Vec2::splat(e.stroke.width / 2.)))
                            .length()
                                < 0.0001
                        );
                        assert!(
                            (e.stroke.width - 2. * 400. / 600.).abs() < 0.0001,
                            "ellipse preview must use PDF points, not fixed overlay pixels"
                        );
                        *oval = true;
                    }
                    egui::epaint::Shape::Ellipse(e) if e.radius == egui::vec2(7., 5.5) => {
                        assert_eq!(e.fill, egui::Color32::TRANSPARENT);
                        assert_eq!(e.stroke.width, 1.5);
                        // Pending worker work disables the controls; witness the
                        // selected, enabled toolbar ink before that transition.
                        *toolbar |= e.stroke.color == crate::theme::color(crate::theme::ACCENT);
                    }
                    egui::epaint::Shape::Vec(v) => {
                        for s in v {
                            check(s, oval, toolbar);
                        }
                    }
                    _ => {}
                }
            }
            for s in &out.shapes {
                check(&s.shape, &mut oval, &mut toolbar);
            }
            out.textures_delta.clear();
        }
        assert!(app.markup.mode == markup::Mode::Ellipse);
        assert!(
            oval && toolbar,
            "ellipse ghost and toolbar must be real ellipse UI"
        );
        settle(&mut app, &ctx);
        assert_eq!(app.markup.items.len(), 1);
        assert_eq!(app.markup.items[0].kind, crate::pdf::ShapeKind::Ellipse);
        assert!(app.editing.dirty);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        app.start_edit(Command::Save, &ctx);
        settle(&mut app, &ctx);
        assert!(!app.editing.dirty);
        assert_eq!(EditablePdf::open(&path).unwrap().shapes(), app.markup.items);
        wait_preview(&mut app, &ctx);
        press(&mut app, &ctx, egui::Key::V, egui::Modifiers::NONE);
        for events in [
            vec![],
            vec![pointer(egui::pos2(150., 120.), true)],
            vec![pointer(egui::pos2(150., 120.), false)],
        ] {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    let (page, response) = ui
                        .allocate_exact_size(egui::vec2(400., 300.), egui::Sense::click_and_drag());
                    app.interact_with_markup(ui, &response, page, page, 1);
                },
            );
            out.textures_delta.clear();
        }
        assert!(app.markup.selected.is_some(), "V selects persisted ellipse");
        press(&mut app, &ctx, egui::Key::Delete, egui::Modifiers::NONE);
        settle(&mut app, &ctx);
        assert!(app.markup.items.is_empty() && app.editing.dirty);
        app.start_edit(Command::Undo, &ctx);
        settle(&mut app, &ctx);
        assert!(!app.editing.dirty && app.markup.items.len() == 1);
    }
    #[test]
    fn saving_owned_edits_after_renderer_death_commits_without_false_preview_spinner() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dead-renderer-save.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        let mut session = EditablePdf::open(&path).unwrap();
        session
            .rename_bookmark(0, "Saved despite renderer death")
            .unwrap();
        app.editing.session = Some(session);
        app.editing.dirty = true;
        let (tx, rx) = mpsc::channel();
        app.render_result_rx = rx;
        drop(tx);
        app.apply_render_results(&ctx);
        app.start_edit(Command::Save, &ctx);
        assert!(
            app.edit_pending(),
            "saving must remain available for owned changes"
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.edit_pending() && Instant::now() < deadline {
            app.apply_edit_results(&ctx);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(!app.edit_pending() && !app.editing.dirty);
        assert_eq!(
            EditablePdf::open(&path).unwrap().bookmarks()[0].title,
            "Saved despite renderer death"
        );
        assert!(app.editing.session.is_some() && !app.editing.unrecoverable);
        assert!(
            !app.markup.preview_pending,
            "dead renderer cannot refresh; do not spin forever"
        );
        assert!(app.editing_progress_status(Instant::now()).is_none());
        assert!(app.persistent_edit_error().unwrap().contains("Saved PDF"));
    }

    #[test]
    fn progress_start_save_has_operation_label() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        let mut session = EditablePdf::open(&path).unwrap();
        session.rename_bookmark(0, "Changed").unwrap();
        app.editing.session = Some(session);
        app.editing.dirty = true;
        app.start_edit(Command::Save, &ctx);
        assert_eq!(app.status, "Saving PDF…");
    }

    #[test]
    fn progress_warning_and_verified_save_preview_are_owned() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.pdf");
        let copy = dir.path().join("copy.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        let mut session = EditablePdf::open(&path).unwrap();
        session.rename_bookmark(0, "Changed").unwrap();
        app.editing.dirty = true;
        // Hold completion under test control: no sleep and no synthetic backend result.
        let (sender, receiver) = mpsc::sync_channel(1);
        app.editing.pending = Some(PendingEdit {
            kind: EditKind::Edit,
            started: Instant::now(),
            generation: app.document_generation,
            path: path.clone(),
            receiver,
        });
        let now = Instant::now();
        assert_eq!(
            app.editing_progress_status(now).as_deref(),
            Some("Editing document…")
        );
        let warning = app
            .editing_progress_status(now + Duration::from_secs(11))
            .unwrap();
        assert!(
            warning.contains("still working")
                && warning.contains("no confirmed completion")
                && warning.contains("not automatically cancelled"),
            "{warning}"
        );
        app.apply_edit_results(&ctx);
        assert!(app.editing.pending.is_some() && app.editing.dirty && !app.can_change_markups());
        session.save_as(&copy).unwrap();
        sender
            .send(EditCompletion {
                dirty: session.is_dirty(),
                bookmarks: session.bookmarks(),
                page_labels: session.page_labels(),
                shapes: session.shapes(),
                session: Some(session),
                result: Ok(true),
                saved: true,
                snapshot: None,
                preview_error: None,
            })
            .unwrap();
        app.apply_edit_results(&ctx);
        assert!(!app.editing.dirty);
        assert_eq!(app.project.document.as_ref().unwrap().path, copy);
        app.status = "Rendered / search finished".into();
        assert_eq!(
            app.editing_progress_status(Instant::now()).as_deref(),
            Some("Saved PDF — refreshing preview…")
        );
        let generation = app.document_generation;
        app.document_generation += 1;
        assert!(app.editing_progress_status(Instant::now()).is_none());
        app.document_generation = generation;
        assert!(
            app.editing_progress_status(Instant::now()).is_some(),
            "stale progress is hidden, not mistaken for completed current pixels"
        );
        app.preview_failed("test render failure");
        assert!(app.editing_progress_status(Instant::now()).is_none());
        assert!(app.persistent_edit_error().unwrap().contains("Saved PDF"));
        assert!(!app.markup.preview_pending);
    }

    #[test]
    fn progress_disconnect_clears_spinner_and_keeps_unrecoverable_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        let (sender, receiver) = mpsc::channel();
        app.editing.pending = Some(PendingEdit {
            kind: EditKind::Save,
            started: Instant::now(),
            generation: app.document_generation,
            path: path.clone(),
            receiver,
        });
        app.markup.preview_pending = true;
        drop(sender);
        app.apply_edit_results(&ctx);
        assert!(!app.markup.preview_pending);
        assert!(app.editing.unrecoverable && app.editing.pending.is_none());
        assert!(app.editing_progress_status(Instant::now()).is_none());
        assert!(
            app.persistent_edit_error()
                .unwrap()
                .contains("saving is blocked")
        );
    }

    #[test]
    fn progress_clock_labels_precedence_and_ready_reset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        let now = Instant::now();
        for kind in [
            EditKind::Save,
            EditKind::SaveAs,
            EditKind::Edit,
            EditKind::LoadMarkups,
        ] {
            let (_sender, receiver) = mpsc::channel();
            app.editing.pending = Some(PendingEdit {
                kind,
                started: now,
                generation: app.document_generation,
                path: path.clone(),
                receiver,
            });
            assert_eq!(
                app.editing_progress_status(now - Duration::from_secs(1))
                    .as_deref(),
                Some(kind.label())
            );
            assert_eq!(
                app.editing_progress_status(now + Duration::from_secs(9))
                    .as_deref(),
                Some(kind.label())
            );
            assert!(
                app.editing_progress_status(now + Duration::from_secs(10))
                    .unwrap()
                    .contains("still working")
            );
            app.editing.error = Some("Persistent failure".into());
            assert!(app.editing_progress_status(now).is_none());
            app.editing.error = None;
        }
        app.editing.pending = None;
        app.markup.preview_pending = true;
        app.preview_ready();
        assert!(
            app.editing_progress_status(now).is_none(),
            "normal edits must not claim saved"
        );
        app.editing.saved_preview = Some(SavedPreview {
            generation: app.document_generation,
            path: path.clone(),
            started: now,
        });
        app.markup.preview_pending = true;
        app.project.document.as_mut().unwrap().path = dir.path().join("other.pdf");
        app.preview_ready();
        assert!(
            !app.markup.preview_pending && app.editing.saved_preview.is_none(),
            "validated current installation releases stale progress tokens"
        );
        app.project.document.as_mut().unwrap().path = path;
        app.preview_ready();
        assert!(!app.markup.preview_pending && app.editing.saved_preview.is_none());
        app.editing = EditingState::default();
        assert!(app.editing_progress_status(now).is_none());
    }

    fn pointer(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }
    }
    fn wait_preview(app: &mut GlyphApp, ctx: &egui::Context) {
        let until = Instant::now() + Duration::from_secs(8);
        while !app.can_change_markups() && Instant::now() < until {
            app.apply_render_results(ctx);
            thread::sleep(Duration::from_millis(2));
        }
        assert!(app.can_change_markups(), "{}", app.status);
    }
    fn summary() -> crate::pdf::PdfDocumentSummary {
        crate::pdf::PdfDocumentSummary {
            page_count: 3,
            pages: vec![],
            bookmarks: vec![PdfBookmark {
                object_id: None,
                title: "Sheet 1".into(),
                page_index: Some(0),
                depth: 0,
            }],
            title: None,
        }
    }
    fn fixture(path: &Path) {
        use lopdf::{Document, Object, Stream, dictionary};
        let mut doc = Document::with_version("1.7");
        let pages = doc.new_object_id();
        let font =
            doc.add_object(dictionary! {"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica"});
        let mut ids = Vec::new();
        for _ in 0..3 {
            let content = doc.add_object(Stream::new(
                dictionary! {},
                b"BT /F1 20 Tf 72 200 Td (UNCHANGED DRAWING) Tj ET".to_vec(),
            ));
            ids.push(doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"MediaBox"=>vec![0.into(),0.into(),600.into(),800.into()],"Resources"=>dictionary!{"Font"=>dictionary!{"F1"=>font}},"Contents"=>content}));
        }
        doc.objects.insert(pages,Object::Dictionary(dictionary!{"Type"=>"Pages","Count"=>3,"Kids"=>ids.iter().map(|id|Object::Reference(*id)).collect::<Vec<_>>()}));
        let outlines = doc.new_object_id();
        let entry=doc.add_object(dictionary!{"Title"=>Object::string_literal("Sheet 1"),"Parent"=>outlines,"Dest"=>vec![Object::Reference(ids[0]),Object::Name(b"Fit".to_vec())]});
        doc.objects.insert(
            outlines,
            Object::Dictionary(
                dictionary! {"Type"=>"Outlines","First"=>entry,"Last"=>entry,"Count"=>1},
            ),
        );
        let root =
            doc.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages,"Outlines"=>outlines});
        doc.trailer.set("Root", root);
        doc.save(path).unwrap();
    }
    fn setup(path: &Path, ctx: &egui::Context) -> GlyphApp {
        let mut app = GlyphApp::with_context(ctx, None);
        let summary =
            crate::pdf::PdfEngine::inspect(&crate::pdf::LopdfInspectionEngine, path).unwrap();
        app.project.open_document(path.to_owned(), summary);
        app.document_generation = 10;
        app.project.selected_page = 1;
        app.zoom = 2.5;
        app.pan = egui::vec2(25., 40.);
        app
    }
    fn settle(app: &mut GlyphApp, ctx: &egui::Context) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while (app.editing.pending.is_some() || app.editing.deferred_save.is_some())
            && Instant::now() < deadline
        {
            app.apply_edit_results(ctx);
            app.finish_save_intent(ctx);
            thread::sleep(Duration::from_millis(2));
        }
        assert!(
            app.editing.pending.is_none(),
            "background edit did not complete"
        );
    }
    fn click_retry_preview(app: &mut GlyphApp, ctx: &egui::Context) {
        let mut response = None;
        for _ in 0..3 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                let id = ui.make_persistent_id(("native-icon", crate::app::icons::Icon::Retry));
                app.draw_markup_tools(ui, ctx);
                response = ctx.read_response(id);
            });
            out.textures_delta.clear();
        }
        let response =
            response.expect("Preview error must expose a mouse-accessible Retry preview button");
        assert!(response.enabled());
        assert_eq!(response.rect.size(), egui::Vec2::splat(24.));
        let pos = response.rect.center();
        let mut out = ctx.run_ui(
            egui::RawInput {
                events: vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Default::default(),
                    },
                ],
                ..Default::default()
            },
            |ui| app.draw_markup_tools(ui, ctx),
        );
        out.textures_delta.clear();
        assert!(
            app.edit_pending(),
            "Retry preview click must dispatch LoadMarkups"
        );
    }
    #[test]
    fn shape_snapshot_failure_retains_edit_and_recovers_by_retry_undo_or_save() {
        for (recovery, ellipse) in
            (0..4).flat_map(|r| [false, true].into_iter().map(move |e| (r, e)))
        {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("preview.pdf");
            fixture(&path);
            let original = std::fs::read(&path).unwrap();
            let ctx = egui::Context::default();
            let mut frame = ctx.run_ui(
                egui::RawInput {
                    max_texture_side: Some(8192),
                    ..Default::default()
                },
                |_| {},
            );
            frame.textures_delta.clear();
            let mut app = setup(&path, &ctx);
            app.markup.loaded = true;
            app.start_edit_with_serializer(
                if ellipse {
                    Command::Ellipse {
                        page: 1,
                        rect: crate::core::links::PdfRect {
                            x: 0.1,
                            y: 0.1,
                            width: 0.2,
                            height: 0.2,
                        },
                    }
                } else {
                    Command::Rectangle {
                        page: 1,
                        rect: crate::core::links::PdfRect {
                            x: 0.1,
                            y: 0.1,
                            width: 0.2,
                            height: 0.2,
                        },
                    }
                },
                &ctx,
                |editor| {
                    assert!(editor.is_dirty() && editor.can_undo());
                    Err(PdfError::Edit("serializer capped writer failure".into()))
                },
            );
            let completion = app
                .editing
                .pending
                .as_ref()
                .unwrap()
                .receiver
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            assert!(
                matches!(completion.result, Ok(true)),
                "preview failure must not turn committed mutation into Err"
            );
            assert!(completion.preview_error.is_some() && completion.snapshot.is_none());
            let (tx, rx) = mpsc::channel();
            tx.send(completion).unwrap();
            app.editing.pending.as_mut().unwrap().receiver = rx;
            settle(&mut app, &ctx);
            assert!(
                app.status
                    .contains("changes retained but preview unavailable"),
                "{}",
                app.status
            );
            assert!(
                app.persistent_edit_error()
                    .unwrap()
                    .contains("serializer capped writer failure")
            );
            assert!(app.editing.dirty && app.editing.session.as_ref().unwrap().can_undo());
            assert_eq!(app.markup.items.len(), 1);
            assert!(!app.can_change_markups());
            assert_eq!(std::fs::read(&path).unwrap(), original);
            for mode in [
                markup::Mode::Rectangle,
                markup::Mode::Ellipse,
                markup::Mode::Select,
            ] {
                app.markup.mode = mode;
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        events: vec![
                            egui::Event::PointerMoved(egui::pos2(50., 50.)),
                            egui::Event::PointerButton {
                                pos: egui::pos2(50., 50.),
                                button: egui::PointerButton::Primary,
                                pressed: true,
                                modifiers: Default::default(),
                            },
                            egui::Event::PointerButton {
                                pos: egui::pos2(150., 150.),
                                button: egui::PointerButton::Primary,
                                pressed: false,
                                modifiers: Default::default(),
                            },
                        ],
                        ..Default::default()
                    },
                    |ui| {
                        let (page, response) = ui.allocate_exact_size(
                            egui::vec2(200., 200.),
                            egui::Sense::click_and_drag(),
                        );
                        app.interact_with_markup(ui, &response, page, page, 1);
                    },
                );
                out.textures_delta.clear();
                assert!(!app.edit_pending() && app.markup.selected.is_none());
            }
            app.delete_shape(app.markup.items[0].object_id, &ctx);
            assert!(
                !app.edit_pending(),
                "preview unavailable must block deletion dispatch"
            );
            match recovery {
                0 => click_retry_preview(&mut app, &ctx),
                1 => app.start_edit(Command::Undo, &ctx),
                2 => app.start_edit(Command::Save, &ctx),
                _ => app.start_edit(Command::SaveAs(dir.path().join("copy.pdf")), &ctx),
            }
            settle(&mut app, &ctx);
            let until = Instant::now() + Duration::from_secs(8);
            while !app.can_change_markups() && Instant::now() < until {
                app.apply_render_results(&ctx);
                thread::sleep(Duration::from_millis(2));
            }
            assert!(app.can_change_markups(), "{}", app.status);
            assert!(app.persistent_edit_error().is_none(), "{}", app.status);
            assert_eq!(app.markup.items.len(), usize::from(recovery != 1));
            assert_eq!(app.editing.dirty, recovery == 0);
        }
    }
    #[test]
    fn rectangle_pdfium_failed_snapshot_disables_stale_canvas_until_retry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pdfium.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut frame = ctx.run_ui(
            egui::RawInput {
                max_texture_side: Some(8192),
                ..Default::default()
            },
            |_| {},
        );
        frame.textures_delta.clear();
        let mut app = setup(&path, &ctx);
        app.load_markups(&ctx);
        settle(&mut app, &ctx);
        let until = Instant::now() + Duration::from_secs(8);
        while app.markup.preview_pending && Instant::now() < until {
            app.apply_render_results(&ctx);
            thread::sleep(Duration::from_millis(2));
        }
        assert!(app.rendered_page.is_some());
        app.refresh_markup_render(&ctx, Some(b"invalid PDF snapshot".to_vec()));
        assert!(
            !app.can_change_markups(),
            "pending snapshot must gate old raster"
        );
        let until = Instant::now() + Duration::from_secs(8);
        while app.pending_page_render.is_some() && Instant::now() < until {
            app.apply_render_results(&ctx);
            thread::sleep(Duration::from_millis(2));
        }
        assert!(
            app.preview_unavailable(),
            "PDFium failure must expose explicit recovery, not silently keep stale pixels"
        );
        assert!(!app.can_change_markups());
        assert!(
            app.persistent_edit_error()
                .unwrap()
                .contains("preview unavailable")
        );
        app.load_markups(&ctx);
        settle(&mut app, &ctx);
        let until = Instant::now() + Duration::from_secs(8);
        while !app.can_change_markups() && Instant::now() < until {
            app.apply_render_results(&ctx);
            thread::sleep(Duration::from_millis(2));
        }
        assert!(app.can_change_markups());
        assert!(!app.preview_unavailable());
    }
    #[test]
    fn rectangle_canvas_press_does_not_reenter_context_lock() {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("press.pdf");
            fixture(&path);
            let ctx = egui::Context::default();
            let mut app = setup(&path, &ctx);
            app.markup.loaded = true;
            app.markup.mode = markup::Mode::Rectangle;
            for pressed in [false, true] {
                let events = if pressed {
                    vec![
                        egui::Event::PointerMoved(egui::pos2(150., 150.)),
                        egui::Event::PointerButton {
                            pos: egui::pos2(150., 150.),
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: Default::default(),
                        },
                    ]
                } else {
                    vec![]
                };
                let mut frame = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(500., 500.),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        let (page, response) = ui.allocate_exact_size(
                            egui::vec2(400., 400.),
                            egui::Sense::click_and_drag(),
                        );
                        app.interact_with_markup(ui, &response, page, page, 1);
                    },
                );
                frame.textures_delta.clear();
            }
            tx.send(()).unwrap();
        });
        rx.recv_timeout(Duration::from_secs(5)).expect(
            "Rectangle input must finish without nested egui context callbacks deadlocking",
        );
    }
    #[test]
    fn rectangle_worker_roundtrip_uses_shared_history_and_render_revision() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shapes.pdf");
        fixture(&path);
        let original = std::fs::read(&path).unwrap();
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        let revision = app.document_generation;
        app.load_markups(&ctx);
        settle(&mut app, &ctx);
        assert!(app.markup.loaded, "Markup must load on the editing worker");
        let r = crate::core::links::PdfRect {
            x: 0.1,
            y: 0.2,
            width: 0.3,
            height: 0.4,
        };
        app.add_rectangle(1, r, &ctx);
        settle(&mut app, &ctx);
        assert_eq!(app.markup.items.len(), 1);
        assert!(app.editing.dirty);
        assert!(app.document_generation > revision);
        let id = app.markup.items[0].object_id;
        app.start_edit(Command::Undo, &ctx);
        settle(&mut app, &ctx);
        assert!(app.markup.items.is_empty());
        assert!(!app.editing.dirty);
        app.start_edit(Command::Redo, &ctx);
        settle(&mut app, &ctx);
        assert_eq!(app.markup.items.len(), 1);
        app.delete_shape(id, &ctx);
        settle(&mut app, &ctx);
        assert!(app.markup.items.is_empty());
        app.start_edit(Command::Undo, &ctx);
        settle(&mut app, &ctx);
        assert_eq!(app.markup.items.len(), 1);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            original,
            "Unsaved annotations must not mutate source"
        );
        app.start_edit(Command::Save, &ctx);
        settle(&mut app, &ctx);
        assert!(!app.editing.dirty);
        assert_eq!(EditablePdf::open(&path).unwrap().shapes().len(), 1);
    }
    #[test]
    fn save_as_shortcut_calls_picker_without_saving_original() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let original = std::fs::read(&path).unwrap();
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        let mut called = false;
        let modifiers = egui::Modifiers {
            ctrl: true,
            command: true,
            shift: true,
            ..Default::default()
        };
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::S,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
                ..Default::default()
            },
            |ui| {
                app.handle_edit_shortcuts_with_picker(ui.ctx(), |_| {
                    called = true;
                    None
                });
            },
        );
        output.textures_delta.clear();
        assert!(
            called,
            "Ctrl+Shift+S must invoke Save As, not Save or nothing"
        );
        assert!(app.editing.pending.is_none());
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
    #[test]
    fn save_as_without_extension_produces_reopenable_pdf() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        let chosen = dir.path().join("copy");
        app.save_as_with_picker(&ctx, |_| Some(chosen.clone()));
        settle(&mut app, &ctx);
        let saved = app.project.document.as_ref().unwrap().path.clone();
        assert!(
            crate::pdf::PdfEngine::inspect(&crate::pdf::LopdfInspectionEngine, &saved).is_ok(),
            "Save As must produce a filename Glyph can reopen: {}",
            saved.display()
        );
        assert_eq!(saved, chosen.with_extension("pdf"));
    }
    #[test]
    fn save_as_wrong_extension_is_rejected_before_background_work() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        let chosen = dir.path().join("copy.txt");
        app.save_as_with_picker(&ctx, |_| Some(chosen.clone()));
        let pending = app.editing.pending.is_some();
        settle(&mut app, &ctx);
        assert!(
            !pending,
            "Unsupported Save As extension must be rejected before starting a write"
        );
        assert!(!chosen.exists());
        assert_eq!(app.project.document.as_ref().unwrap().path, path);
        assert!(
            app.persistent_edit_error()
                .is_some_and(|m| m.contains(".pdf filename")),
            "Invalid destination error must remain visible independently of renderer status"
        );
    }
    #[test]
    fn long_render_status_does_not_wrap_into_a_log_block() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        app.status = format!(
            "Rendered page 2 of 3 - {}",
            "very-long-project-name-".repeat(20)
        );
        for _ in 0..2 {
            let mut warm = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(960., 640.),
                    )),
                    ..Default::default()
                },
                |ui| app.draw(ui),
            );
            warm.textures_delta.clear();
        }
        app.status = format!(
            "Rendered page 2 of 3 - {}",
            "very-long-project-name-".repeat(20)
        );
        let mut o = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(960., 640.),
                )),
                ..Default::default()
            },
            |ui| app.draw(ui),
        );
        fn bounded(s: &egui::epaint::Shape) -> Option<bool> {
            match s {
                egui::epaint::Shape::Text(t)
                    if t.galley.text().starts_with("Rendered page 2 of 3 -") =>
                {
                    Some(t.galley.rows.len() == 1 && t.pos.x + t.galley.size().x <= 960.)
                }
                egui::epaint::Shape::Vec(v) => v.iter().find_map(bounded),
                _ => None,
            }
        }
        let good = o
            .shapes
            .iter()
            .find_map(|s| bounded(&s.shape))
            .unwrap_or(false);
        o.textures_delta.clear();
        assert!(
            good,
            "Render progress must be a single bounded status line, not a filename log block"
        );
    }
    #[test]
    fn long_page_label_stays_inside_thumbnail_card() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        app.project.document.as_mut().unwrap().summary.pages[0].label =
            Some("Architectural-sheet-name-".repeat(10));
        let mut bounded = false;
        for _ in 0..3 {
            let mut o = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(320., 640.),
                    )),
                    ..Default::default()
                },
                |ui| app.thumbnail_sidebar(ui),
            );
            fn bounded_caption(s: &egui::epaint::Shape) -> Option<bool> {
                match s {
                    egui::epaint::Shape::Text(t)
                        if t.galley.text().starts_with("1  Architectural") =>
                    {
                        Some(
                            t.pos.x >= 0.
                                && t.pos.x + t.galley.size().x <= 320.
                                && t.galley.rows.len() == 1,
                        )
                    }
                    egui::epaint::Shape::Vec(v) => v.iter().find_map(bounded_caption),
                    _ => None,
                }
            }
            bounded = o
                .shapes
                .iter()
                .find_map(|s| bounded_caption(&s.shape))
                .unwrap_or(false);
            o.textures_delta.clear();
        }
        assert!(
            bounded,
            "A long sheet label must remain a single bounded caption, not clip both ends"
        );
    }
    #[test]
    fn navigation_controls_use_font_independent_vector_controls() {
        fn collect(s: &egui::epaint::Shape, lines: &mut Vec<([egui::Pos2; 2], egui::Stroke)>) {
            match s {
                egui::epaint::Shape::LineSegment { points, stroke } => {
                    lines.push((*points, *stroke));
                }
                egui::epaint::Shape::Text(t) => {
                    assert!(!matches!(t.galley.text(), "< Prev" | "Next >"));
                    assert!(
                        !t.galley.text().contains(['←', '→', '◀', '▶']),
                        "Navigation must not depend on missing arrow glyphs in the desktop font"
                    );
                }
                egui::epaint::Shape::Vec(v) => {
                    for s in v {
                        collect(s, lines);
                    }
                }
                _ => {}
            }
        }
        // Locate the actual sidebar arrows by their three vector strokes, not
        // separately rendered icons or text/font metrics. Their centers are the
        // centers of the native 24-point pointer targets.
        fn arrow_centers(lines: &[([egui::Pos2; 2], egui::Stroke)], d: f32) -> Vec<egui::Pos2> {
            lines
                .iter()
                .filter_map(|(points, stroke)| {
                    if stroke.width != 1.5 || points[1] - points[0] != egui::vec2(12. * d, 0.) {
                        return None;
                    }
                    let center = points[0] + egui::vec2(6. * d, 0.);
                    let target = egui::Rect::from_center_size(center, egui::Vec2::splat(24.));
                    let sidebar =
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(280., 640.));
                    if !sidebar.contains_rect(target) {
                        return None;
                    }
                    for y in [-5., 5.] {
                        let expected = [center + egui::vec2(d, y), center + egui::vec2(6. * d, 0.)];
                        let (points, _) = lines
                            .iter()
                            .find(|(points, s)| *points == expected && *s == *stroke)?;
                        assert!(
                            target.contains_rect(
                                egui::Rect::from_two_pos(points[0], points[1])
                                    .expand(stroke.width / 2.)
                            )
                        );
                    }
                    assert!(target.contains_rect(
                        egui::Rect::from_two_pos(points[0], points[1]).expand(stroke.width / 2.)
                    ));
                    Some(center)
                })
                .collect()
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        let frame = |app: &mut GlyphApp, events| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(960., 640.),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| app.draw(ui),
            );
            out.textures_delta.clear();
            out
        };
        let mut out = frame(&mut app, vec![]);
        for _ in 0..2 {
            out = frame(&mut app, vec![]);
        }
        let mut lines = Vec::new();
        for s in &out.shapes {
            let mut shape_lines = Vec::new();
            collect(&s.shape, &mut shape_lines);
            // The app also has a central toolbar; only sidebar-clipped ink
            // proves the sidebar navigation pair is actually wired.
            if s.clip_rect.max.x <= 280. {
                lines.extend(shape_lines);
            }
        }
        let previous = arrow_centers(&lines, -1.);
        let next = arrow_centers(&lines, 1.);
        assert_eq!(
            previous.len(),
            1,
            "Actual app sidebar must paint Previous arrow"
        );
        assert_eq!(next.len(), 1, "Actual app sidebar must paint Next arrow");
        assert_eq!(previous[0].y, next[0].y);
        assert!(previous[0].x + 24. <= next[0].x);
        assert_eq!(app.project.selected_page, 1);
        for (pos, label, expected_page) in
            [(previous[0], "Previous page", 0), (next[0], "Next page", 1)]
        {
            frame(
                &mut app,
                vec![egui::Event::PointerMoved(pos), pointer(pos, true)],
            );
            let out = frame(
                &mut app,
                vec![egui::Event::PointerMoved(pos), pointer(pos, false)],
            );
            assert_eq!(
                app.project.selected_page, expected_page,
                "{label} must navigate by mouse"
            );
            assert!(
                out.platform_output.events.iter().any(|event| {
                    let info = event.widget_info();
                    matches!(event, egui::output::OutputEvent::Clicked(_))
                        && info.typ == egui::WidgetType::Button
                        && info.label.as_deref() == Some(label)
                        && info.enabled
                        && info.selected == Some(false)
                }),
                "{label} must emit named button information on the actual app click"
            );
        }
    }
    #[test]
    fn lost_edit_session_disables_bookmark_context_rename() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("context.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        app.project.selected_page = 0;
        app.editing.unrecoverable = true;
        fn find(s: &egui::epaint::Shape) -> Option<egui::Pos2> {
            match s {
                egui::epaint::Shape::Text(t) if t.galley.text() == "Rename" => {
                    Some(t.pos + t.galley.size() / 2.)
                }
                egui::epaint::Shape::Vec(v) => v.iter().find_map(find),
                _ => None,
            }
        }
        let mut frame = |events| {
            let mut o = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800., 600.),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let (_, response) =
                        ui.allocate_exact_size(egui::vec2(300., 160.), egui::Sense::click());
                    app.bookmark_edit_menu(&response, 0);
                },
            );
            let p = o.shapes.iter().find_map(|s| find(&s.shape));
            o.textures_delta.clear();
            p
        };
        for _ in 0..3 {
            frame(vec![]);
        }
        fn click(pos: egui::Pos2, button: egui::PointerButton) -> Vec<egui::Event> {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ]
        }
        frame(click(egui::pos2(40., 40.), egui::PointerButton::Secondary));
        frame(vec![]);
        let point = frame(vec![]).expect("Right-click must expose page label editing");
        frame(click(point, egui::PointerButton::Primary));
        assert!(
            app.editing.rename.is_none(),
            "A lost editable session must disable bookmark Rename, not offer a doomed action"
        );
    }
    #[test]
    fn page_context_rename_targets_clicked_page_without_navigation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("context.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        app.project.selected_page = 0;
        fn find(s: &egui::epaint::Shape) -> Option<egui::Pos2> {
            match s {
                egui::epaint::Shape::Text(t) if t.galley.text() == "Rename page label..." => {
                    Some(t.pos + t.galley.size() / 2.)
                }
                egui::epaint::Shape::Vec(v) => v.iter().find_map(find),
                _ => None,
            }
        }
        let mut frame = |events| {
            let mut o = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800., 600.),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let (_, response) =
                        ui.allocate_exact_size(egui::vec2(300., 160.), egui::Sense::click());
                    app.page_edit_menu(&response, 1);
                },
            );
            let p = o.shapes.iter().find_map(|s| find(&s.shape));
            o.textures_delta.clear();
            p
        };
        for _ in 0..3 {
            frame(vec![]);
        }
        fn click(pos: egui::Pos2, button: egui::PointerButton) -> Vec<egui::Event> {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ]
        }
        frame(click(egui::pos2(40., 40.), egui::PointerButton::Secondary));
        frame(vec![]);
        let point = frame(vec![]).expect("Right-click must expose page label editing");
        frame(click(point, egui::PointerButton::Primary));
        let dialog = app
            .editing
            .rename
            .as_ref()
            .expect("Context action must open rename");
        assert!(dialog.target == RenameTarget::PageLabel);
        assert_eq!(dialog.index, 1);
        assert_eq!(dialog.original, "2");
        assert_eq!(app.project.selected_page, 0);
    }
    #[test]
    fn opening_rename_selects_original_once_for_immediate_replacement() {
        for target in [RenameTarget::Bookmark, RenameTarget::PageLabel] {
            let ctx = egui::Context::default();
            let mut app = GlyphApp::with_context(&ctx, None);
            app.editing.rename = Some(RenameDialog {
                target,
                index: 0,
                original: "План 第一".into(),
                value: "План 第一".into(),
                focus: true,
            });
            for text in [None, None, None, Some("A101"), Some("-continued")] {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1200., 800.),
                        )),
                        events: text
                            .map(|s| vec![egui::Event::Text(s.into())])
                            .unwrap_or_default(),
                        ..Default::default()
                    },
                    |ui| app.editing_dialogs(ui.ctx()),
                );
                output.textures_delta.clear();
                if text == Some("A101") {
                    assert_eq!(
                        app.editing.rename.as_ref().unwrap().value,
                        "A101",
                        "Typing immediately must replace the original Unicode name"
                    );
                }
            }
            assert_eq!(
                app.editing.rename.as_ref().unwrap().value,
                "A101-continued",
                "Later typing must not reselect the edited draft"
            );
        }
    }
    #[test]
    fn narrow_toolbar_keeps_fit_width_on_one_line() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        for enabled in [true, false] {
            let mut rows = 0;
            let mut right = 0.;
            for _ in 0..3 {
                let mut o = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(90., 400.),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        ui.horizontal_wrapped(|ui| {
                            tool_chip(ui, "Reset");
                            right = ui
                                .add_enabled(enabled, tool_chip_button("Fit width"))
                                .rect
                                .right();
                        });
                    },
                );
                fn find(s: &egui::epaint::Shape) -> Option<usize> {
                    match s {
                        egui::epaint::Shape::Text(t) if t.galley.text() == "Fit width" => {
                            Some(t.galley.rows.len())
                        }
                        egui::epaint::Shape::Vec(v) => v.iter().find_map(find),
                        _ => None,
                    }
                }
                rows = o.shapes.iter().find_map(|s| find(&s.shape)).unwrap_or(0);
                o.textures_delta.clear();
            }
            assert_eq!(
                rows, 1,
                "Toolbar buttons must wrap as whole controls, never letter stacks"
            );
            assert!(right <= 90., "Button overflows toolbar: {right}");
        }
    }
    #[test]
    fn long_filename_keeps_sidebar_counter_visible() {
        fn counters(s: &egui::epaint::Shape) -> usize {
            match s {
                egui::epaint::Shape::Text(t) => usize::from(
                    t.galley.text() == "Page 2 / 3" && t.pos.x + t.galley.size().x <= 960.,
                ),
                egui::epaint::Shape::Vec(v) => v.iter().map(counters).sum(),
                _ => 0,
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join(format!("{}.pdf", "long-project-name-".repeat(10)));
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        let mut count = 0;
        for _ in 0..3 {
            let mut o = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(960., 640.),
                    )),
                    ..Default::default()
                },
                |ui| app.draw(ui),
            );
            count = o.shapes.iter().map(|s| counters(&s.shape)).sum();
            o.textures_delta.clear();
        }
        assert_eq!(
            count, 2,
            "Both header and sidebar page counters must remain visible with a long filename"
        );
    }
    #[test]
    fn long_filename_keeps_header_page_counter_inside_small_viewport() {
        fn counter_visible(shape: &egui::epaint::Shape) -> bool {
            match shape {
                egui::epaint::Shape::Text(t) => {
                    t.pos.y < 44.
                        && t.galley.text() == "Page 2 / 3"
                        && t.pos.x >= 0.
                        && t.pos.x + t.galley.size().x <= 960.
                }
                egui::epaint::Shape::Vec(v) => v.iter().any(counter_visible),
                _ => false,
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join(format!("{}.pdf", "long-project-name-".repeat(10)));
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        let mut visible = false;
        for _ in 0..3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(960., 640.),
                    )),
                    ..Default::default()
                },
                |ui| app.draw(ui),
            );
            visible = output.shapes.iter().any(|s| counter_visible(&s.shape));
            output.textures_delta.clear();
        }
        assert!(
            visible,
            "A long filename must not push the page counter outside the native window"
        );
    }
    #[test]
    fn empty_document_menu_offers_open_pdf() {
        fn has_open(shape: &egui::epaint::Shape) -> bool {
            match shape {
                egui::epaint::Shape::Text(t) => t.galley.text() == "Open PDF…",
                egui::epaint::Shape::Vec(v) => v.iter().any(has_open),
                _ => false,
            }
        }
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            app.document_menu_contents(ui, &ctx)
        });
        let visible = output.shapes.iter().any(|s| has_open(&s.shape));
        output.textures_delta.clear();
        assert!(
            visible,
            "The empty Document menu must expose Open PDF for mouse users"
        );
    }
    #[test]
    fn empty_undo_redo_does_not_load_pdf_or_start_worker() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        app.start_edit(Command::Undo, &ctx);
        let pending = app.editing.pending.is_some();
        settle(&mut app, &ctx);
        assert!(
            !pending,
            "Empty Undo must not load a PDF or spawn a document worker"
        );
        app.start_edit(Command::Redo, &ctx);
        assert!(app.editing.pending.is_none());
        assert!(app.editing.session.is_none());
    }
    #[test]
    fn real_worker_switch_from_last_of_eleven_pages_to_one_page_renders_new_source() {
        use lopdf::{Document, Object, Stream, dictionary};
        fn make(path: &std::path::Path, count: usize, green: bool) {
            let mut doc = Document::with_version("1.7");
            let pages = doc.new_object_id();
            let mut kids = vec![];
            for _ in 0..count {
                let stream = doc.add_object(Stream::new(
                    dictionary! {},
                    if green {
                        b"0 0.8 0 rg 0 0 100 100 re f".to_vec()
                    } else {
                        b"0.8 0 0 rg 0 0 100 100 re f".to_vec()
                    },
                ));
                let page = doc.add_object(dictionary! {"Type"=>"Page", "Parent"=>pages, "MediaBox"=>vec![0.into(),0.into(),100.into(),100.into()], "Resources"=>dictionary! {}, "Contents"=>stream});
                kids.push(Object::Reference(page));
            }
            doc.objects.insert(
                pages,
                dictionary! {"Type"=>"Pages","Kids"=>kids,"Count"=>count as i64}.into(),
            );
            let catalog = doc.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages});
            doc.trailer.set("Root", catalog);
            doc.save(path).unwrap();
        }
        fn wait(app: &mut GlyphApp, ctx: &egui::Context, expected: &std::path::Path, page: usize) {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(8);
            loop {
                app.apply_render_results(ctx);
                if app.loading_document.is_none()
                    && app
                        .project
                        .document
                        .as_ref()
                        .is_some_and(|d| d.path == expected)
                    && app
                        .rendered_page
                        .as_ref()
                        .is_some_and(|r| r.page_index == page)
                {
                    return;
                }
                assert!(
                    std::time::Instant::now() < until,
                    "actual PDF worker must render replacement, not leave a blank/stale canvas: {}",
                    app.status
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let large = dir.path().join("eleven.pdf");
        let small = dir.path().join("one.pdf");
        make(&large, 11, false);
        make(&small, 1, true);
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        app.open_pdf(large.clone(), &ctx);
        wait(&mut app, &ctx, &large, 0);
        app.select_page(10, &ctx);
        wait(&mut app, &ctx, &large, 10);
        assert!(app.navigation_history.can_back());
        app.open_pdf(small.clone(), &ctx);
        wait(&mut app, &ctx, &small, 0);
        assert_eq!(app.project.selected_page, 0);
        assert!(!app.navigation_history.can_back());
        assert!(!app.navigation_history.can_forward());
        assert_eq!(app.project.document.as_ref().unwrap().summary.page_count, 1);
        let r = app.rendered_page.as_ref().unwrap();
        let center = 4 * (r.width * (r.height / 2) + r.width / 2);
        assert!(
            r.rgba[center + 1] > 180 && r.rgba[center] < 30,
            "replacement must show new green PDF, not the old red drawing"
        );
    }
    #[test]
    fn malformed_page_labels_allow_viewing_but_editing_fails_closed() {
        use lopdf::dictionary;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let mut doc = lopdf::Document::load(&path).unwrap();
        doc.catalog_mut().unwrap().set("PageLabels", dictionary! {"Nums" => vec![lopdf::Object::Integer(0), lopdf::Object::Dictionary(dictionary! {"S" => "UnknownNumberingStyle"})]});
        doc.save(&path).unwrap();
        let original = std::fs::read(&path).unwrap();
        let summary = crate::pdf::PdfEngine::inspect(&crate::pdf::LopdfInspectionEngine, &path)
            .expect("Malformed optional page labels must not stop viewing a valid drawing");
        assert_eq!(summary.pages[0].label.as_deref(), Some("1"));
        assert!(
            EditablePdf::open(&path).is_err(),
            "Editing malformed labels must fail closed instead of overwriting them with numeric defaults"
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
    #[test]
    fn pending_edit_schedules_bounded_polling_without_user_input() {
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        let (_sender, receiver) = mpsc::channel();
        app.editing.pending = Some(PendingEdit {
            kind: EditKind::Edit,
            started: Instant::now(),
            generation: 1,
            path: PathBuf::from("not-opened.pdf"),
            receiver,
        });
        let delays = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let callback_delays = delays.clone();
        ctx.set_request_repaint_callback(move |info| {
            callback_delays.lock().unwrap().push(info.delay)
        });
        for _ in 0..3 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                app.apply_edit_results(ui.ctx())
            });
            out.textures_delta.clear();
        }
        assert!(
            delays
                .lock()
                .unwrap()
                .iter()
                .any(|delay| *delay > std::time::Duration::ZERO
                    && *delay <= std::time::Duration::from_millis(100)),
            "Pending edit must schedule a bounded wake-up, even if its worker never sends/repaints"
        );
        app.editing.pending = None;
        delays.lock().unwrap().clear();
        for _ in 0..3 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                app.apply_edit_results(ui.ctx())
            });
            out.textures_delta.clear();
        }
        assert!(
            !delays
                .lock()
                .unwrap()
                .iter()
                .any(|delay| *delay > std::time::Duration::ZERO
                    && *delay <= std::time::Duration::from_millis(100)),
            "Finished edits must not keep polling"
        );
    }
    #[test]
    fn native_window_close_preserves_active_rename_and_unsaved_decision() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        app.request_page_label(0, &ctx);
        app.editing.rename.as_mut().unwrap().value = "Uncommitted label".into();
        app.editing.dirty = true;
        let close_input = || {
            let mut raw = egui::RawInput::default();
            raw.viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .events
                .push(egui::ViewportEvent::Close);
            raw
        };
        let mut out = ctx.run_ui(close_input(), |ui| app.guard_window_close(ui.ctx()));
        let cancelled = out.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .any(|cmd| matches!(cmd, egui::ViewportCommand::CancelClose));
        out.textures_delta.clear();
        assert!(cancelled);
        assert!(
            app.editing.transition.is_none(),
            "OS close must not create a second modal while a draft owns input"
        );
        assert_eq!(
            app.editing.rename.as_ref().unwrap().value,
            "Uncommitted label"
        );
        app.editing.rename = None;
        app.editing.transition = Some(Transition::CloseDocument);
        let mut out = ctx.run_ui(close_input(), |ui| app.guard_window_close(ui.ctx()));
        out.textures_delta.clear();
        assert!(
            matches!(app.editing.transition, Some(Transition::CloseDocument)),
            "OS close must preserve an existing unsaved decision"
        );
    }
    #[test]
    fn document_menu_exposes_current_page_label_edit() {
        fn has_action(shape: &egui::epaint::Shape) -> bool {
            match shape {
                egui::epaint::Shape::Text(t) => {
                    t.galley.text().contains("Rename current page label")
                }
                egui::epaint::Shape::Vec(v) => v.iter().any(has_action),
                _ => false,
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        let mut found = false;
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                app.document_menu_contents(ui, &ctx)
            });
            found |= out.shapes.iter().any(|s| has_action(&s.shape));
            out.textures_delta.clear();
        }
        assert!(
            found,
            "Page-label editing must have a mouse-accessible Document menu action"
        );
    }
    #[test]
    fn page_label_dialog_applies_without_renaming_bookmark_then_undo_redo_and_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let original = std::fs::read(&path).unwrap();
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        app.request_page_label(0, &ctx);
        app.editing.rename.as_mut().unwrap().value = "A-101".into();
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                app.editing_dialogs(ui.ctx())
            });
            out.textures_delta.clear();
        }
        let mut out = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| app.editing_dialogs(ui.ctx()),
        );
        out.textures_delta.clear();
        settle(&mut app, &ctx);
        let doc = app.project.document.as_ref().unwrap();
        assert_eq!(
            doc.summary.pages[0].label.as_deref(),
            Some("A-101"),
            "Page-label dialog must update the label, not a bookmark title"
        );
        assert_eq!(doc.summary.bookmarks[0].title, "Sheet 1");
        assert!(app.editing.dirty);
        assert!(
            app.status.contains("Document updated"),
            "Label edits must not claim a bookmark was changed"
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
        app.start_edit(Command::Undo, &ctx);
        settle(&mut app, &ctx);
        assert_eq!(
            app.project.document.as_ref().unwrap().summary.pages[0]
                .label
                .as_deref(),
            Some("1")
        );
        assert!(!app.editing.dirty);
        app.start_edit(Command::Redo, &ctx);
        settle(&mut app, &ctx);
        assert_eq!(
            app.project.document.as_ref().unwrap().summary.pages[0]
                .label
                .as_deref(),
            Some("A-101")
        );
        app.start_edit(Command::Save, &ctx);
        settle(&mut app, &ctx);
        assert!(!app.editing.dirty);
        let reopened =
            crate::pdf::PdfEngine::inspect(&crate::pdf::LopdfInspectionEngine, &path).unwrap();
        assert_eq!(reopened.pages[0].label.as_deref(), Some("A-101"));
        assert_eq!(reopened.bookmarks[0].title, "Sheet 1");
    }
    #[test]
    fn oversized_page_label_draft_remains_in_page_label_dialog() {
        fn label_heading(shape: &egui::epaint::Shape) -> bool {
            match shape {
                egui::epaint::Shape::Text(t) => t.galley.text().contains("Rename page label"),
                egui::epaint::Shape::Vec(v) => v.iter().any(label_heading),
                _ => false,
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        app.request_page_label(0, &ctx);
        app.editing.rename.as_mut().unwrap().value = "X".repeat(257);
        let mut heading = false;
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                app.editing_dialogs(ui.ctx())
            });
            heading |= out.shapes.iter().any(|s| label_heading(&s.shape));
            out.textures_delta.clear();
        }
        let mut out = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| app.editing_dialogs(ui.ctx()),
        );
        out.textures_delta.clear();
        assert!(
            heading,
            "Page-label edits must identify their own dialog, not a bookmark edit"
        );
        assert!(
            app.editing.rename.is_some(),
            "Over-limit draft must not be dismissed"
        );
        assert_eq!(app.editing.rename.as_ref().unwrap().value.len(), 257);
        assert!(!app.edit_pending());
    }
    #[test]
    fn page_label_dialog_targets_physical_page_and_keeps_modal_ownership() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        app.request_page_label(0, &ctx);
        let dialog = app
            .editing
            .rename
            .as_ref()
            .expect("Page-label edit must open a native rename dialog");
        assert!(dialog.target == RenameTarget::PageLabel);
        assert_eq!(dialog.index, 0);
        assert_eq!(dialog.original, "1");
        assert_eq!(dialog.value, "1");
        assert!(app.defer_document_open(dir.path().join("other.pdf")));
        app.request_page_label(99, &ctx);
        assert_eq!(app.editing.rename.as_ref().unwrap().index, 0);
        assert!(app.loading_document.is_none());
    }
    #[test]
    fn disconnected_worker_cannot_silently_save_unedited_original() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let original = std::fs::read(&path).unwrap();
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        app.start_edit(
            Command::Rename {
                index: 0,
                original: "Sheet 1".into(),
                title: "Unsaved renamed sheet".into(),
            },
            &ctx,
        );
        settle(&mut app, &ctx);
        assert!(app.editing.dirty);
        app.editing.session.take();
        let (sender, receiver) = mpsc::channel();
        drop(sender);
        app.editing.pending = Some(PendingEdit {
            kind: EditKind::Edit,
            started: Instant::now(),
            generation: app.document_generation,
            path: path.clone(),
            receiver,
        });
        app.apply_edit_results(&ctx);
        app.start_edit(Command::Save, &ctx);
        let saving = app.editing.pending.is_some();
        settle(&mut app, &ctx);
        assert!(
            !saving,
            "A lost editable snapshot must block Save, not reload and save the unedited original"
        );
        assert!(app.editing.dirty);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(app.status.contains("cannot be recovered"));
        app.save_as_with_picker(&ctx, |_| {
            panic!("An unrecoverable edit session must not open a destination picker")
        });
    }
    #[test]
    fn save_failure_remains_visible_after_rendering_updates_status() {
        fn has_error(shape: &egui::epaint::Shape) -> bool {
            match shape {
                egui::epaint::Shape::Text(t) => t.galley.text().contains("simulated save conflict"),
                egui::epaint::Shape::Vec(v) => v.iter().any(has_error),
                _ => false,
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        let mut session = EditablePdf::open(&path).unwrap();
        session.rename_bookmark(0, "Unsaved title").unwrap();
        let (sender, receiver) = mpsc::channel();
        sender
            .send(EditCompletion {
                session: Some(session),
                result: Err(PdfError::Edit("simulated save conflict".into())),
                bookmarks: vec![],
                page_labels: vec![],
                saved: false,
                dirty: true,
                shapes: vec![],
                snapshot: None,
                preview_error: None,
            })
            .unwrap();
        app.editing.pending = Some(PendingEdit {
            kind: EditKind::Edit,
            started: Instant::now(),
            generation: app.document_generation,
            path,
            receiver,
        });
        app.editing.transition = Some(Transition::CloseDocument);
        app.editing.save_before_transition = true;
        app.apply_edit_results(&ctx);
        app.status = "Rendered page 2 of 3".into();
        let mut visible = false;
        for _ in 0..3 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                app.editing_dialogs(ui.ctx())
            });
            visible = output.shapes.iter().any(|s| has_error(&s.shape));
            output.textures_delta.clear();
        }
        assert!(
            visible,
            "Save error must persist in the unsaved dialog independently of rendering status"
        );
        app.editing.transition = None;
        visible = false;
        for _ in 0..3 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.draw(ui));
            visible = output.shapes.iter().any(|s| has_error(&s.shape));
            output.textures_delta.clear();
        }
        assert!(
            visible,
            "Save error must also persist in the main viewer after rendering updates status"
        );
    }
    #[test]
    fn incoming_pdf_does_not_replace_a_modal_action_or_rename_draft() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let incoming = dir.path().join("incoming.pdf");
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        app.editing.dirty = true;
        app.editing.transition = Some(Transition::CloseDocument);
        assert!(app.defer_document_open(incoming.clone()));
        assert!(
            matches!(app.editing.transition, Some(Transition::CloseDocument)),
            "File drop must not replace an existing modal transition"
        );
        app.editing.transition = None;
        app.editing.dirty = false;
        app.editing.rename = Some(RenameDialog {
            target: RenameTarget::Bookmark,
            index: 0,
            original: "Sheet 1".into(),
            value: "Uncommitted draft".into(),
            focus: false,
        });
        assert!(
            app.defer_document_open(incoming),
            "An open request must wait while a rename dialog owns input"
        );
        assert_eq!(
            app.editing.rename.as_ref().unwrap().value,
            "Uncommitted draft"
        );
        assert!(app.loading_document.is_none());
    }
    #[test]
    fn oversized_rename_preserves_draft_and_explains_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let original = std::fs::read(&path).unwrap();
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        let draft = "a".repeat(4097);
        app.editing.rename = Some(RenameDialog {
            target: RenameTarget::Bookmark,
            index: 0,
            original: "Sheet 1".into(),
            value: draft.clone(),
            focus: true,
        });
        let mut first = ctx.run_ui(egui::RawInput::default(), |ui| {
            app.editing_dialogs(ui.ctx())
        });
        first.textures_delta.clear();
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| app.editing_dialogs(ui.ctx()),
        );
        output.textures_delta.clear();
        settle(&mut app, &ctx);
        assert!(
            app.editing.rename.is_some(),
            "An oversized bookmark title must retain the rename dialog and user's draft"
        );
        assert_eq!(app.editing.rename.as_ref().unwrap().value, draft);
        assert!(!app.editing.dirty);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        fn has_limit(shape: &egui::epaint::Shape) -> bool {
            match shape {
                egui::epaint::Shape::Text(t) => t.galley.text().contains("4096"),
                egui::epaint::Shape::Vec(v) => v.iter().any(has_limit),
                _ => false,
            }
        }
        assert!(
            output.shapes.iter().any(|s| has_limit(&s.shape)),
            "The rename dialog must explain the title limit"
        );
    }
    #[test]
    fn closing_document_clears_document_scoped_navigation_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        let before = app.view_location();
        let after = ViewLocation { page: 0, ..before };
        app.navigation_history.visit(before, after);
        app.project.selected_page = 0;
        assert!(app.navigation_history.can_back());
        app.request_close_document(&ctx);
        assert!(app.project.document.is_none());
        assert!(
            !app.navigation_history.can_back(),
            "Close must clear stale Back navigation"
        );
        assert!(!app.navigation_history.can_forward());
    }
    #[test]
    fn bookmark_rename_is_background_work_and_keeps_disk_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let original = std::fs::read(&path).unwrap();
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        app.start_edit(
            Command::Rename {
                index: 0,
                original: "Sheet 1".into(),
                title: "A101 Floor plan".into(),
            },
            &ctx,
        );
        assert!(
            app.editing.pending.is_some(),
            "rename must run through bounded background work"
        );
        settle(&mut app, &ctx);
        assert!(app.editing.dirty, "{}", app.status);
        assert_eq!(
            app.project.document.as_ref().unwrap().summary.bookmarks[0].title,
            "A101 Floor plan"
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(app.project.selected_page, 1);
        assert_eq!(app.zoom, 2.5);
        assert_eq!(app.pan, egui::vec2(25., 40.));
    }
    #[test]
    fn app_undo_restores_saved_title_without_writing_pdf() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let original = std::fs::read(&path).unwrap();
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        app.start_edit(
            Command::Rename {
                index: 0,
                original: "Sheet 1".into(),
                title: "A101".into(),
            },
            &ctx,
        );
        settle(&mut app, &ctx);
        app.start_edit(Command::Undo, &ctx);
        settle(&mut app, &ctx);
        assert_eq!(
            app.project.document.as_ref().unwrap().summary.bookmarks[0].title,
            "Sheet 1",
            "{}",
            app.status
        );
        assert!(!app.editing.dirty);
        assert_eq!(std::fs::read(path).unwrap(), original);
    }
    #[test]
    fn app_redo_restores_the_unsaved_rename() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        app.start_edit(
            Command::Rename {
                index: 0,
                original: "Sheet 1".into(),
                title: "A101".into(),
            },
            &ctx,
        );
        settle(&mut app, &ctx);
        app.start_edit(Command::Undo, &ctx);
        settle(&mut app, &ctx);
        app.start_edit(Command::Redo, &ctx);
        settle(&mut app, &ctx);
        assert_eq!(
            app.project.document.as_ref().unwrap().summary.bookmarks[0].title,
            "A101",
            "{}",
            app.status
        );
        assert!(app.editing.dirty);
    }
    #[test]
    fn app_save_reopens_the_rename_and_preserves_viewport() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        app.start_edit(
            Command::Rename {
                index: 0,
                original: "Sheet 1".into(),
                title: "A101".into(),
            },
            &ctx,
        );
        settle(&mut app, &ctx);
        let generation = app.document_generation;
        app.start_edit(Command::Save, &ctx);
        settle(&mut app, &ctx);
        assert!(!app.editing.dirty, "{}", app.status);
        assert_eq!(
            crate::pdf::PdfEngine::inspect(&crate::pdf::LopdfInspectionEngine, &path)
                .unwrap()
                .bookmarks[0]
                .title,
            "A101"
        );
        assert!(app.document_generation > generation);
        assert_eq!(app.project.selected_page, 1);
        assert_eq!(app.zoom, 2.5);
        assert_eq!(app.pan, egui::vec2(25., 40.));
    }
    #[test]
    fn app_save_as_changes_the_path_without_overwriting_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let output = dir.path().join("copy.pdf");
        let original = std::fs::read(&path).unwrap();
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        app.start_edit(
            Command::Rename {
                index: 0,
                original: "Sheet 1".into(),
                title: "A101".into(),
            },
            &ctx,
        );
        settle(&mut app, &ctx);
        app.start_edit(Command::SaveAs(output.clone()), &ctx);
        settle(&mut app, &ctx);
        assert_eq!(
            app.project.document.as_ref().unwrap().path,
            output,
            "{}",
            app.status
        );
        assert!(!app.editing.dirty);
        assert_eq!(std::fs::read(path).unwrap(), original);
        assert_eq!(
            crate::pdf::PdfEngine::inspect(&crate::pdf::LopdfInspectionEngine, &output)
                .unwrap()
                .bookmarks[0]
                .title,
            "A101"
        );
    }
    #[test]
    fn clean_save_does_not_queue_io() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        app.start_edit(Command::Save, &ctx);
        assert!(
            app.editing.pending.is_none(),
            "saving a clean document must not rewrite it"
        );
    }
    #[test]
    fn closing_a_dirty_document_requires_a_decision() {
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        app.project.open_document("current.pdf".into(), summary());
        app.editing.dirty = true;
        app.request_close_document(&ctx);
        assert!(
            matches!(app.editing.transition, Some(Transition::CloseDocument)),
            "close must ask before dropping edits"
        );
        assert!(app.project.document.is_some());
    }
    #[test]
    fn clean_document_closes_without_a_prompt() {
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        app.project.open_document("current.pdf".into(), summary());
        app.request_close_document(&ctx);
        assert!(
            app.project.document.is_none(),
            "clean document should close"
        );
        assert!(app.editing.transition.is_none());
    }
    #[test]
    fn cancelling_close_keeps_unsaved_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        app.start_edit(
            Command::Rename {
                index: 0,
                original: "Sheet 1".into(),
                title: "A101".into(),
            },
            &ctx,
        );
        settle(&mut app, &ctx);
        app.request_close_document(&ctx);
        app.resolve_unsaved(Decision::Cancel, &ctx);
        assert!(
            app.editing.transition.is_none(),
            "cancel must dismiss the close request"
        );
        assert!(app.editing.dirty);
        assert_eq!(
            app.project.document.unwrap().summary.bookmarks[0].title,
            "A101"
        );
    }
    #[test]
    fn mixed_edits_discard_after_failed_open_restores_latest_saved_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        let r = crate::core::links::PdfRect {
            x: 0.1,
            y: 0.2,
            width: 0.3,
            height: 0.4,
        };
        app.add_rectangle(1, r, &ctx);
        settle(&mut app, &ctx);
        app.add_ellipse(1, r, &ctx);
        settle(&mut app, &ctx);
        app.start_edit(
            Command::Rename {
                index: 0,
                original: "Sheet 1".into(),
                title: "Saved title".into(),
            },
            &ctx,
        );
        settle(&mut app, &ctx);
        app.start_edit(
            Command::PageLabel {
                index: 0,
                original: "1".into(),
                title: "Saved label".into(),
            },
            &ctx,
        );
        settle(&mut app, &ctx);
        app.start_edit(Command::Save, &ctx);
        settle(&mut app, &ctx);
        assert!(!app.editing.dirty);
        let saved = std::fs::read(&path).unwrap();
        let saved_shapes = EditablePdf::open(&path).unwrap().shapes();
        assert_eq!(saved_shapes.len(), 2);
        app.delete_shape(saved_shapes[1].object_id, &ctx);
        settle(&mut app, &ctx);
        app.start_edit(
            Command::Rename {
                index: 0,
                original: "Saved title".into(),
                title: "Discarded title".into(),
            },
            &ctx,
        );
        settle(&mut app, &ctx);
        app.start_edit(
            Command::PageLabel {
                index: 0,
                original: "Saved label".into(),
                title: "Discarded label".into(),
            },
            &ctx,
        );
        settle(&mut app, &ctx);
        assert!(app.editing.dirty);
        app.open_pdf(dir.path().join("missing.pdf"), &ctx);
        assert!(app.editing.transition.is_some());
        app.resolve_unsaved(Decision::Discard, &ctx);
        let until = Instant::now() + Duration::from_secs(5);
        while app.loading_document.is_some() && Instant::now() < until {
            app.apply_render_results(&ctx);
            thread::sleep(Duration::from_millis(2));
        }
        assert!(app.loading_document.is_none());
        assert!(!app.editing.dirty);
        assert!(app.editing.session.is_none());
        let doc = app.project.document.as_ref().unwrap();
        assert_eq!(doc.path, path);
        assert_eq!(doc.summary.bookmarks[0].title, "Saved title");
        assert_eq!(doc.summary.pages[0].label.as_deref(), Some("Saved label"));
        assert_eq!(std::fs::read(&path).unwrap(), saved);
        assert_eq!(EditablePdf::open(&path).unwrap().shapes(), saved_shapes);
        assert!(app.markup.items.is_empty() && !app.markup.loaded);
        app.start_edit(
            Command::PageLabel {
                index: 0,
                original: "Saved label".into(),
                title: "Recovered label".into(),
            },
            &ctx,
        );
        settle(&mut app, &ctx);
        assert!(app.editing.dirty);
        assert_eq!(
            app.project.document.as_ref().unwrap().summary.pages[0]
                .label
                .as_deref(),
            Some("Recovered label")
        );
        assert_eq!(std::fs::read(&path).unwrap(), saved);
    }
    #[test]
    fn discard_before_replacement_restores_the_saved_bookmark_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let original = std::fs::read(&path).unwrap();
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        app.start_edit(
            Command::Rename {
                index: 0,
                original: "Sheet 1".into(),
                title: "A101".into(),
            },
            &ctx,
        );
        settle(&mut app, &ctx);
        app.open_pdf(dir.path().join("missing.pdf"), &ctx);
        app.resolve_unsaved(Decision::Discard, &ctx);
        assert_eq!(
            app.project.document.as_ref().unwrap().summary.bookmarks[0].title,
            "Sheet 1",
            "discard must restore the kept document before replacement inspection"
        );
        assert!(!app.editing.dirty);
        assert!(app.loading_document.is_some());
        assert_eq!(std::fs::read(path).unwrap(), original);
    }
    #[test]
    fn save_before_close_commits_then_closes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        app.start_edit(
            Command::Rename {
                index: 0,
                original: "Sheet 1".into(),
                title: "A101".into(),
            },
            &ctx,
        );
        settle(&mut app, &ctx);
        app.request_close_document(&ctx);
        app.resolve_unsaved(Decision::Save, &ctx);
        settle(&mut app, &ctx);
        assert!(
            app.project.document.is_none(),
            "save must complete before close: {}",
            app.status
        );
        assert_eq!(
            crate::pdf::PdfEngine::inspect(&crate::pdf::LopdfInspectionEngine, &path)
                .unwrap()
                .bookmarks[0]
                .title,
            "A101"
        );
    }
    #[test]
    fn native_window_close_is_cancelled_for_dirty_edits() {
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        app.editing.dirty = true;
        let mut input = egui::RawInput::default();
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .events
            .push(egui::ViewportEvent::Close);
        let mut output = ctx.run_ui(input, |ui| app.guard_window_close(ui.ctx()));
        output.textures_delta.clear();
        assert!(
            output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .any(|c| matches!(c, egui::ViewportCommand::CancelClose)),
            "native close must be cancelled until edits are resolved"
        );
        assert!(matches!(app.editing.transition, Some(Transition::Quit)));
    }
    fn press(app: &mut GlyphApp, ctx: &egui::Context, key: egui::Key, modifiers: egui::Modifiers) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
                ..Default::default()
            },
            |_| app.handle_shortcuts(ctx),
        );
        output.textures_delta.clear();
    }
    #[test]
    fn shortcut_undo_dispatches_real_document_undo() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        app.start_edit(
            Command::Rename {
                index: 0,
                original: "Sheet 1".into(),
                title: "A101".into(),
            },
            &ctx,
        );
        settle(&mut app, &ctx);
        press(&mut app, &ctx, egui::Key::Z, egui::Modifiers::COMMAND);
        settle(&mut app, &ctx);
        assert_eq!(
            app.project.document.unwrap().summary.bookmarks[0].title,
            "Sheet 1",
            "Ctrl+Z must undo document edits"
        );
    }
    #[test]
    fn shortcut_redo_restores_the_undone_rename() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        app.start_edit(
            Command::Rename {
                index: 0,
                original: "Sheet 1".into(),
                title: "A101".into(),
            },
            &ctx,
        );
        settle(&mut app, &ctx);
        app.start_edit(Command::Undo, &ctx);
        settle(&mut app, &ctx);
        press(
            &mut app,
            &ctx,
            egui::Key::Z,
            egui::Modifiers {
                shift: true,
                ..egui::Modifiers::COMMAND
            },
        );
        settle(&mut app, &ctx);
        assert_eq!(
            app.project.document.unwrap().summary.bookmarks[0].title,
            "A101",
            "Ctrl+Shift+Z must redo"
        );
    }
    #[test]
    fn shortcut_save_commits_the_current_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        app.start_edit(
            Command::Rename {
                index: 0,
                original: "Sheet 1".into(),
                title: "A101".into(),
            },
            &ctx,
        );
        settle(&mut app, &ctx);
        press(&mut app, &ctx, egui::Key::S, egui::Modifiers::COMMAND);
        settle(&mut app, &ctx);
        assert!(!app.editing.dirty, "Ctrl+S must save");
        assert_eq!(
            crate::pdf::PdfEngine::inspect(&crate::pdf::LopdfInspectionEngine, &path)
                .unwrap()
                .bookmarks[0]
                .title,
            "A101"
        );
    }
    #[test]
    fn shortcut_close_requests_unsaved_resolution() {
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        app.project.open_document("current.pdf".into(), summary());
        app.editing.dirty = true;
        press(&mut app, &ctx, egui::Key::W, egui::Modifiers::COMMAND);
        assert!(
            matches!(app.editing.transition, Some(Transition::CloseDocument)),
            "Ctrl+W must use the guarded close path"
        );
    }
    #[test]
    fn save_as_picker_dispatches_selected_destination() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        let destination = dir.path().join("copy.pdf");
        fixture(&path);
        let before = std::fs::read(&path).unwrap();
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        app.start_edit(
            Command::Rename {
                index: 0,
                original: "Sheet 1".into(),
                title: "A101".into(),
            },
            &ctx,
        );
        settle(&mut app, &ctx);
        // Only the OS file picker is substituted; inspection, editing and saving are real.
        app.save_as_with_picker(&ctx, |_| Some(destination.clone()));
        settle(&mut app, &ctx);
        assert!(
            destination.exists(),
            "Selected Save As destination must be written"
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(
            crate::pdf::PdfEngine::inspect(&crate::pdf::LopdfInspectionEngine, &destination)
                .unwrap()
                .bookmarks[0]
                .title,
            "A101"
        );
    }
    #[test]
    fn group_bookmark_rows_accept_context_menu_input() {
        let ctx = egui::Context::default();
        let mut enabled = false;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            enabled = bookmark_row(ui, "Architecture", 0, None, false).enabled();
        });
        output.textures_delta.clear();
        assert!(enabled, "Group bookmarks need context-menu editing");
    }
    #[test]
    fn automation_cannot_export_a_stale_source_while_edits_are_unsaved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        app.start_edit(
            Command::Rename {
                index: 0,
                original: "Sheet 1".into(),
                title: "A101".into(),
            },
            &ctx,
        );
        settle(&mut app, &ctx);
        app.start_automation(AutomationKind::Hyperlinks, &ctx);
        assert!(
            app.automation_rx.is_none(),
            "Unsaved edits must not be bypassed by automation"
        );
    }
    #[test]
    fn document_heading_marks_unsaved_edits() {
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        app.project.open_document("current.pdf".into(), summary());
        app.editing.dirty = true;
        assert!(
            app.window_title().contains("current.pdf *"),
            "Unsaved document needs a visible dirty marker"
        );
    }
    #[test]
    fn rename_preserves_existing_bookmark_navigation_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let mut pdf = lopdf::Document::load(&path).unwrap();
        let page = *pdf.get_pages().values().next().unwrap();
        let outlines = pdf
            .catalog()
            .unwrap()
            .get(b"Outlines")
            .unwrap()
            .as_reference()
            .unwrap();
        let first = pdf
            .get_object(outlines)
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"First")
            .unwrap()
            .as_reference()
            .unwrap();
        pdf.get_object_mut(first)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Dest", lopdf::Object::Reference(page));
        pdf.save(&path).unwrap();
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        assert_eq!(
            app.project.document.as_ref().unwrap().summary.bookmarks[0].page_index,
            Some(0)
        );
        app.start_edit(
            Command::Rename {
                index: 0,
                original: "Sheet 1".into(),
                title: "A101".into(),
            },
            &ctx,
        );
        settle(&mut app, &ctx);
        assert_eq!(
            app.project.document.as_ref().unwrap().summary.bookmarks[0].page_index,
            Some(0),
            "Title-only editing must retain bookmark navigation"
        );
    }
    #[test]
    fn duplicate_indirect_title_does_not_rename_the_wrong_outline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.pdf");
        fixture(&path);
        let mut pdf = lopdf::Document::load(&path).unwrap();
        let outlines = pdf
            .catalog()
            .unwrap()
            .get(b"Outlines")
            .unwrap()
            .as_reference()
            .unwrap();
        let first = pdf
            .get_object(outlines)
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"First")
            .unwrap()
            .as_reference()
            .unwrap();
        let sibling = pdf.get_object(first).unwrap().clone();
        let second = pdf.add_object(sibling);
        let indirect = pdf.add_object(lopdf::Object::string_literal("Sheet 1"));
        let item = pdf.get_object_mut(first).unwrap().as_dict_mut().unwrap();
        item.set("Title", lopdf::Object::Reference(indirect));
        item.set("Next", lopdf::Object::Reference(second));
        pdf.get_object_mut(second)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Prev", lopdf::Object::Reference(first));
        let root = pdf.get_object_mut(outlines).unwrap().as_dict_mut().unwrap();
        root.set("Last", lopdf::Object::Reference(second));
        root.set("Count", 2);
        pdf.save(&path).unwrap();
        let ctx = egui::Context::default();
        let mut app = setup(&path, &ctx);
        let visible = app
            .project
            .document
            .as_ref()
            .unwrap()
            .summary
            .bookmarks
            .iter()
            .position(|b| b.object_id == Some(second))
            .unwrap();
        app.start_edit(
            Command::Rename {
                index: visible,
                original: "Sheet 1".into(),
                title: "A101".into(),
            },
            &ctx,
        );
        settle(&mut app, &ctx);
        let rows = app.editing.session.as_ref().unwrap().bookmarks();
        assert_eq!(
            rows[0].title, "Sheet 1",
            "The indirectly titled sibling must remain unchanged"
        );
        assert_eq!(rows[1].title, "A101");
    }
    #[test]
    fn save_failure_is_visible_inside_unsaved_dialog() {
        fn contains(shape: &egui::epaint::Shape, text: &str) -> bool {
            match shape {
                egui::epaint::Shape::Text(t) => t.galley.text().contains(text),
                egui::epaint::Shape::Vec(shapes) => shapes.iter().any(|s| contains(s, text)),
                _ => false,
            }
        }
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        app.editing.dirty = true;
        app.editing.transition = Some(Transition::Quit);
        app.status = "Document edit failed: source changed externally".into();
        app.editing.error = Some(app.status.clone());
        let mut visible = false;
        for _ in 0..3 {
            let mut output =
                ctx.run_ui(egui::RawInput::default(), |ui| app.unsaved_dialog(ui.ctx()));
            visible |= output
                .shapes
                .iter()
                .any(|s| contains(&s.shape, "source changed externally"));
            output.textures_delta.clear();
        }
        assert!(
            visible,
            "Save failure must be visible in the decision dialog"
        );
    }
    #[test]
    fn document_switch_is_blocked_while_an_edit_is_inflight() {
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        app.project.open_document("current.pdf".into(), summary());
        let (_tx, receiver) = mpsc::sync_channel(1);
        app.editing.pending = Some(PendingEdit {
            kind: EditKind::Edit,
            started: Instant::now(),
            generation: app.document_generation,
            path: "current.pdf".into(),
            receiver,
        });
        app.open_pdf("replacement.pdf".into(), &ctx);
        assert!(
            app.loading_document.is_none(),
            "cannot switch while a worker owns unsaved changes"
        );
    }
    #[test]
    fn dirty_document_open_is_deferred_until_explicit_resolution() {
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        app.project.open_document("current.pdf".into(), summary());
        app.editing.dirty = true;
        app.open_pdf("replacement.pdf".into(), &ctx);
        assert!(
            app.loading_document.is_none(),
            "dirty edits must not be silently abandoned"
        );
        assert!(matches!(app.editing.transition, Some(Transition::Open(_))));
        assert_eq!(
            app.project.document.as_ref().unwrap().path,
            PathBuf::from("current.pdf")
        );
    }
}
