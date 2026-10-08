#[test]
fn line_arrow_command_boundary_rejects_modal_and_wrong_page() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("line-boundary.pdf");
    fixture(&path);
    let ctx = egui::Context::default();
    let mut app = setup(&path, &ctx);
    app.editing.transition = Some(Transition::CloseDocument);
    app.start_edit(
        Command::Line {
            page: 1,
            endpoints: [0., 0., 1., 1.],
            arrow: false,
        },
        &ctx,
    );
    assert!(
        !app.edit_pending(),
        "command boundary must enforce modal ownership"
    );
    app.editing.transition = None;
    app.start_edit(
        Command::Line {
            page: 0,
            endpoints: [0., 0., 1., 1.],
            arrow: false,
        },
        &ctx,
    );
    assert!(
        !app.edit_pending(),
        "command boundary must enforce physical page ownership"
    );
}
