use super::*;
impl GlyphApp {
    pub(super) fn interact_with_page_text(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        canvas: egui::Rect,
    ) {
        if self.loading_document.is_some() {
            return;
        }
        let Some(rendered) = self.rendered_page.as_ref() else {
            return;
        };
        let page_rect = egui::Rect::from_center_size(
            canvas.center() + self.pan,
            self.logical_page_size(rendered) * self.zoom,
        );
        let Some(text) = self
            .page_text
            .as_ref()
            .filter(|t| t.page_index == rendered.page_index)
        else {
            return;
        };
        let normalized = |p: egui::Pos2| {
            egui::pos2(
                (p.x - page_rect.left()) / page_rect.width(),
                (p.y - page_rect.top()) / page_rect.height(),
            )
        };
        let press = ui.input(|i| {
            i.events.iter().find_map(|event| match event {
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    ..
                } => Some(*pos),
                _ => None,
            })
        });
        // A batched release may end outside the canvas. Route by the original
        // press and its layer, not the frame's final hover position.
        let press = press.filter(|p| {
            response.rect.contains(*p) && ui.ctx().layer_id_at(*p) == Some(ui.layer_id())
        });
        if press.is_some() {
            if let Some(start) = press.filter(|p| page_rect.contains(*p)) {
                let on_link = self
                    .page_links
                    .iter()
                    .any(|link| overlay_screen_rect(link.rect, page_rect).contains(start));
                if on_link {
                    self.selection.clear();
                    self.selecting_text = false;
                } else {
                    self.selecting_text = self.selection.begin(text, normalized(start));
                }
            } else {
                self.selection.clear();
                self.selecting_text = false;
            }
        }
        if self.selecting_text {
            if let Some(pointer) = ui.input(|i| i.pointer.interact_pos()) {
                self.selection.extend(text, normalized(pointer));
            }
            if ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary)) {
                self.selecting_text = false;
            }
        }
        if response.hovered()
            && response.hover_pos().is_some_and(|p| {
                text.glyphs
                    .iter()
                    .filter_map(|g| g.rect)
                    .any(|r| overlay_screen_rect(r, page_rect).contains(p))
            })
        {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        }
    }
    pub(super) fn paint_text_selection(
        &self,
        painter: &egui::Painter,
        page_rect: egui::Rect,
        viewport: egui::Rect,
    ) {
        if self.loading_document.is_some() {
            return;
        }
        if let (Some(text), Some(range)) = (&self.page_text, self.selection.range()) {
            for index in range {
                if let Some(rect) = text.glyphs.get(index).and_then(|g| g.rect) {
                    let rect = overlay_screen_rect(rect, page_rect);
                    if viewport.intersects(rect) {
                        painter.rect_filled(
                            rect,
                            0.,
                            theme::translucent(theme::color(theme::ACCENT), 90),
                        );
                    }
                }
            }
        }
    }

    pub(super) fn copy_pdf_selection(&self, ctx: &egui::Context) -> bool {
        if self.loading_document.is_some() {
            return false;
        }
        let Some(page) = self
            .page_text
            .as_ref()
            .filter(|p| p.page_index == self.project.selected_page)
        else {
            return false;
        };
        let selected = self.selection.selected_text(page);
        if selected.is_empty() {
            return false;
        }
        ctx.copy_text(selected);
        true
    }

    pub(super) fn clear_page_text(&mut self) {
        self.page_text = None;
        self.pending_text = None;
        self.text_error = None;
        self.selection.clear();
        self.selecting_text = false;
    }
    pub(super) fn queue_page_text(&mut self) {
        if self.loading_document.is_some()
            || self.page_text.is_some()
            || self.text_error.is_some()
            || self.pending_text.is_some()
        {
            return;
        }
        if let Some(document) = &self.project.document {
            let page_index = self.project.selected_page;
            let id = self.next_render_job_id;
            self.next_render_job_id = id.wrapping_add(1).max(1);
            self.pending_text = Some((id, page_index));
            self.render_worker.submit(RenderJob {
                id,
                generation: self.document_generation,
                path: document.path.clone(),
                kind: JobKind::Text { page_index },
            });
        }
    }

    pub(super) fn apply_page_text_result(
        &mut self,
        id: u64,
        generation: u64,
        path: PathBuf,
        page: usize,
        result: Result<crate::pdf::PageText, PdfError>,
    ) {
        if self.loading_document.is_some()
            || generation != self.document_generation
            || self.project.selected_page != page
            || !self
                .project
                .document
                .as_ref()
                .is_some_and(|d| d.path == path)
            || self.pending_text != Some((id, page))
        {
            return;
        }
        self.pending_text = None;
        match result {
            Ok(text) if text.page_index == page => {
                self.page_text = Some(Arc::new(text));
                self.text_error = None;
            }
            Ok(_) => self.text_error = Some("Text belongs to another page".into()),
            Err(error) => self.text_error = Some(error.to_string()),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_handles_press_drag_and_release_batched_in_one_frame() {
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        app.zoom = 100. / BASE_RENDER_WIDTH as f32;
        app.rendered_page = Some(Arc::new(RenderedPage {
            page_index: 0,
            width: 2,
            height: 2,
            rgba: vec![255; 16],
        }));
        app.page_text = Some(Arc::new(crate::pdf::PageText {
            page_index: 0,
            glyphs: vec![
                crate::pdf::TextGlyph {
                    text: "A".into(),
                    rect: Some(crate::core::links::PdfRect {
                        x: 0.1,
                        y: 0.1,
                        width: 0.1,
                        height: 0.1,
                    }),
                },
                crate::pdf::TextGlyph {
                    text: "字".into(),
                    rect: Some(crate::core::links::PdfRect {
                        x: 0.4,
                        y: 0.1,
                        width: 0.1,
                        height: 0.1,
                    }),
                },
            ],
        }));
        let mut canvas = egui::Rect::NOTHING;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(200., 200.), egui::Sense::click_and_drag());
            canvas = rect;
            app.interact_with_page_text(ui, &response, rect);
        });
        output.textures_delta.clear();
        let start = canvas.center() + egui::vec2(-35., -35.);
        for end in [
            canvas.center() + egui::vec2(-5., -35.),
            egui::pos2(canvas.right() + 30., start.y),
        ] {
            app.selection.clear();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: vec![
                        egui::Event::PointerMoved(start),
                        egui::Event::PointerButton {
                            pos: start,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: egui::Modifiers::NONE,
                        },
                        egui::Event::PointerMoved(end),
                        egui::Event::PointerButton {
                            pos: end,
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ui| {
                    let (rect, response) = ui
                        .allocate_exact_size(egui::vec2(200., 200.), egui::Sense::click_and_drag());
                    app.interact_with_page_text(ui, &response, rect);
                },
            );
            output.textures_delta.clear();
            assert_eq!(
                app.selection.selected_text(app.page_text.as_ref().unwrap()),
                "A字"
            );
            assert!(!app.selecting_text);
        }
    }
    #[test]
    fn failed_replacement_load_restores_text_and_rendering_for_the_kept_document() {
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        app.project.open_document(
            "kept.pdf".into(),
            crate::pdf::PdfDocumentSummary {
                page_count: 2,
                pages: Vec::new(),
                bookmarks: Vec::new(),
                title: None,
            },
        );
        app.loading_document = Some(99);
        app.apply_inspection(
            99,
            "bad.pdf".into(),
            Err(PdfError::Render("broken fixture".into())),
            &ctx,
        );
        assert_eq!(
            app.project.document.as_ref().unwrap().path,
            PathBuf::from("kept.pdf")
        );
        assert!(
            app.pending_page_render.is_some(),
            "cancelled previous render must resume"
        );
        assert_eq!(app.pending_text.map(|p| p.1), Some(0));
        assert!(app.status.contains("Load failed"));
    }
    #[test]
    fn navigation_clears_old_text_and_requests_the_new_page() {
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        app.project.open_document(
            "text.pdf".into(),
            crate::pdf::PdfDocumentSummary {
                page_count: 2,
                pages: Vec::new(),
                bookmarks: Vec::new(),
                title: None,
            },
        );
        app.page_text = Some(Arc::new(crate::pdf::PageText {
            page_index: 0,
            glyphs: Vec::new(),
        }));
        app.select_page(1, &ctx);
        assert!(
            app.page_text.is_none(),
            "previous page text must not remain selectable"
        );
        assert_eq!(app.pending_text.map(|p| p.1), Some(1));
    }
    #[test]
    fn pdf_copy_event_puts_selected_unicode_text_on_clipboard_output() {
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        let text = crate::pdf::PageText {
            page_index: 0,
            glyphs: vec![crate::pdf::TextGlyph {
                text: "café".into(),
                rect: Some(crate::core::links::PdfRect {
                    x: 0.1,
                    y: 0.1,
                    width: 0.1,
                    height: 0.1,
                }),
            }],
        };
        assert!(app.selection.begin(&text, egui::pos2(0.15, 0.15)));
        app.page_text = Some(Arc::new(text));
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Copy],
                ..Default::default()
            },
            |ui| app.draw(ui),
        );
        output.textures_delta.clear();
        assert!(
            output
                .platform_output
                .commands
                .iter()
                .any(|c| matches!(c,egui::OutputCommand::CopyText(s) if s=="café")),
            "PDF copy must emit real clipboard text"
        );
        output.textures_delta.clear();
    }
    #[test]
    fn current_text_result_installs_but_stale_requests_do_not_replace_it() {
        let ctx = egui::Context::default();
        let mut app = GlyphApp::with_context(&ctx, None);
        app.project.open_document(
            "text.pdf".into(),
            crate::pdf::PdfDocumentSummary {
                page_count: 2,
                pages: Vec::new(),
                bookmarks: Vec::new(),
                title: None,
            },
        );
        app.document_generation = 7;
        app.pending_text = Some((10, 0));
        let text = crate::pdf::PageText {
            page_index: 0,
            glyphs: vec![crate::pdf::TextGlyph {
                text: "actual page text".into(),
                rect: None,
            }],
        };
        app.apply_page_text_result(10, 7, "text.pdf".into(), 0, Ok(text.clone()));
        assert_eq!(*app.page_text.as_ref().unwrap().as_ref(), text);
        app.pending_text = Some((11, 0));
        for (id, generation, path, page) in [
            (10, 7, "text.pdf", 0),
            (11, 6, "text.pdf", 0),
            (11, 7, "other.pdf", 0),
            (11, 7, "text.pdf", 1),
        ] {
            app.apply_page_text_result(
                id,
                generation,
                path.into(),
                page,
                Ok(crate::pdf::PageText {
                    page_index: page,
                    glyphs: Vec::new(),
                }),
            );
            assert_eq!(*app.page_text.as_ref().unwrap().as_ref(), text);
        }
        app.loading_document = Some(99);
        app.apply_page_text_result(
            11,
            7,
            "text.pdf".into(),
            0,
            Ok(crate::pdf::PageText {
                page_index: 0,
                glyphs: Vec::new(),
            }),
        );
        assert_eq!(*app.page_text.as_ref().unwrap().as_ref(), text);
    }
}
