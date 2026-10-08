use super::*;

struct SheetEntry {
    label: String,
    titles: Vec<String>,
    searchable: String,
}

pub(super) struct SheetPicker {
    pub(super) query: String,
    pub(super) matches: Vec<usize>,
    entries: Vec<SheetEntry>,
    pub(super) selected: usize,
    scroll_selected: bool,
    focus_requested: bool,
    last_query: Option<String>,
    path: PathBuf,
    generation: u64,
}
impl SheetPicker {
    pub(super) fn filter(&mut self) {
        if self.last_query.as_ref() == Some(&self.query) {
            return;
        }
        let normalized = self.query.to_lowercase();
        let terms: Vec<_> = normalized.split_whitespace().collect();
        self.matches = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| terms.iter().all(|term| entry.searchable.contains(term)))
            .map(|(page, _)| page)
            .collect();
        self.selected = 0;
        self.scroll_selected = true;
        self.last_query = Some(self.query.clone());
    }
}
impl GlyphApp {
    pub(super) fn request_keyboard_quit(&mut self, ctx: &egui::Context) {
        if self.loading_document.is_some() || self.automation_rx.is_some() {
            self.status = "Wait for the current document operation.".into();
            return;
        }
        // Reuse the existing window-close decision owner rather than bypassing
        // dirty/pending guards with an unconditional programmatic Close.
        let viewport = ctx.viewport_id();
        let events = ctx.input_mut(|input| {
            let info = input.raw.viewports.get_mut(&viewport).unwrap();
            std::mem::replace(&mut info.events, vec![egui::ViewportEvent::Close])
        });
        self.guard_window_close(ctx);
        ctx.input_mut(|input| input.raw.viewports.get_mut(&viewport).unwrap().events = events);
        if !self.edit_pending() && !self.editing.dirty && !self.editing_modal_open() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
    pub(super) fn close_sheet_picker(&mut self, ctx: &egui::Context) {
        self.sheet_picker = None;
        ctx.memory_mut(|memory| memory.surrender_focus(egui::Id::new("sheet_picker_query")));
    }
    pub(super) fn draw_sheet_picker(&mut self, ctx: &egui::Context) {
        self.validate_sheet_picker(ctx);
        let Some(picker) = &mut self.sheet_picker else {
            return;
        };
        let (up, down, enter) = ctx.input_mut(|input| {
            (
                input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                input.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            )
        });
        let mut chosen = None;
        egui::Modal::new(egui::Id::new("sheet_picker")).show(ctx, |ui| {
            ui.set_width(480.);
            ui.heading("Go to Sheet");
            let response = ui.add(
                egui::TextEdit::singleline(&mut picker.query)
                    .id(egui::Id::new("sheet_picker_query"))
                    .hint_text("Sheet number, title, or page number")
                    .desired_width(f32::INFINITY),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Find a sheet")
            });
            if picker.focus_requested {
                response.request_focus();
                picker.focus_requested = false;
            }
            picker.filter();
            if up {
                picker.selected = picker.selected.saturating_sub(1);
            }
            if down {
                picker.selected = (picker.selected + 1).min(picker.matches.len().saturating_sub(1));
            }
            picker.scroll_selected |= up || down;
            let row_height = 28.;
            let mut scroll = egui::ScrollArea::vertical()
                .id_salt("sheet_picker_rows")
                .max_height(280.);
            if picker.scroll_selected {
                scroll = scroll.vertical_scroll_offset(
                    picker.selected as f32 * (row_height + ui.spacing().item_spacing.y),
                );
                picker.scroll_selected = false;
            }
            scroll.show_rows(ui, row_height, picker.matches.len(), |ui, rows| {
                for row in rows {
                    let page = picker.matches[row];
                    let entry = &picker.entries[page];
                    ui.push_id(page, |ui| {
                        let text = format!(
                            "{}   {}   {}",
                            page + 1,
                            entry.label,
                            entry.titles.join(" • ")
                        );
                        let response = ui.add_sized(
                            [ui.available_width(), row_height],
                            egui::Button::selectable(picker.selected == row, text),
                        );
                        if response.clicked() {
                            picker.selected = row;
                        }
                        if response.double_clicked() {
                            chosen = Some(page);
                        }
                    });
                }
            });
            ui.label(format!(
                "{} of {} sheets · ↑↓ select · Return opens",
                picker.matches.len(),
                picker.entries.len()
            ));
            if picker.matches.is_empty() {
                ui.label("No matching sheets");
            }
            if ui
                .add_enabled(!picker.matches.is_empty(), egui::Button::new("Go to Sheet"))
                .clicked()
                || enter
            {
                chosen = picker.matches.get(picker.selected).copied();
            }
        });
        if let Some(page) = chosen {
            self.sheet_picker = None;
            ctx.memory_mut(|memory| memory.surrender_focus(egui::Id::new("sheet_picker_query")));
            self.select_page(page, ctx);
        }
    }
    pub(super) fn sheet_picker_ready(&mut self) -> bool {
        // Read source readiness without counting this dialog as a competing modal.
        // This remains valid when the central edit guard also includes sheet_picker.
        let picker = self.sheet_picker.take();
        let ready = self.can_change_markups()
            && self.automation_feedback.is_none()
            && self.page_count().is_some_and(|count| count > 0);
        self.sheet_picker = picker;
        ready
    }
    pub(super) fn validate_sheet_picker(&mut self, ctx: &egui::Context) {
        let ready = self.sheet_picker_ready();
        if self.sheet_picker.as_ref().is_some_and(|picker| {
            !ready
                || picker.generation != self.document_generation
                || !self
                    .project
                    .document
                    .as_ref()
                    .is_some_and(|document| document.path == picker.path)
        }) {
            self.sheet_picker = None;
            ctx.memory_mut(|memory| memory.surrender_focus(egui::Id::new("sheet_picker_query")));
        }
    }
    pub(super) fn open_sheet_picker(&mut self) {
        if !self.sheet_picker_ready() {
            return;
        }
        let Some(document) = &self.project.document else {
            return;
        };
        let summary = &document.summary;
        let mut entries: Vec<_> = (0..summary.page_count)
            .map(|page| SheetEntry {
                label: summary
                    .pages
                    .get(page)
                    .and_then(|info| info.label.clone())
                    .unwrap_or_default(),
                titles: vec![],
                searchable: String::new(),
            })
            .collect();
        for bookmark in &summary.bookmarks {
            if let Some(page) = bookmark.page_index
                && let Some(entry) = entries.get_mut(page)
                && !bookmark.title.is_empty()
                && !entry.titles.contains(&bookmark.title)
            {
                entry.titles.push(bookmark.title.clone());
            }
        }
        for (page, entry) in entries.iter_mut().enumerate() {
            entry.searchable = format!(
                "{} {} page {}",
                entry.label,
                entry.titles.join(" "),
                page + 1
            )
            .to_lowercase();
        }
        let mut picker = SheetPicker {
            query: String::new(),
            matches: vec![],
            entries,
            selected: 0,
            scroll_selected: true,
            focus_requested: true,
            last_query: None,
            path: document.path.clone(),
            generation: self.document_generation,
        };
        picker.filter();
        // Only a successful input-owner handoff cancels pending markup work.
        self.cancel_markup_gestures();
        self.sheet_picker = Some(picker);
    }
}
