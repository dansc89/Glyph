# Markup editing and Go to Sheet verification

Verified development source, 2026-10-07. **Not a new release or full Drawbridge v5.5 parity claim.** Released downloads remain Glyph 1.5. Local package/version metadata was preserved rather than silently bumping or installing a development release.

## Delivered

- Select and move Glyph-owned rectangles, ellipses, lines and arrows.
- Corner-resize boxes and drag line/arrow endpoints; keep valid selection through preview refresh.
- Compact Markup properties popup: RGB stroke color and physical width in points, for selection or subsequent new shapes. Apply uses shared PDF history; Cancel does not mutate the PDF.
- Go to Sheet via Ctrl+L or the document menu: physical page numbers, actual page labels and bookmark titles; duplicate labels stay distinct, arrows/Enter select, Escape returns input ownership.
- Picker opening discards pending markup drafts, including creation/move/resize/endpoint and properties drafts, while preserving valid selection and current tool.
- Annotation appearance copy-on-write protects shared foreign appearances/page XObjects/reference chains. Preserve annotation identity, flags, comments/custom metadata, auxiliary appearances and border metadata through edit, Undo/Redo and Save.

## Final gates, integrated main checkout

- **393 portable Rust tests passed**, including synthetic ignored regressions; the two private `real_saltair` tests were deliberately excluded.
- `cargo fmt --check`, strict all-target Clippy (`-D warnings`), and locked optimized build passed.
- **19 native markup checks**: creation, move, corner/endpoint edits, numeric properties, saved geometry/style, Undo/Redo, close/reopen, foreign annotations/content preservation, and the picker cancellation fence. Actual numeric input widgets were exercised, not injected backend responses.
- **46 native Go to Sheet checks**: labels/bookmarks/numbers/duplicate targets, keyboard/modal ownership, no-match/Escape, normal input restoration, clean owned-display cleanup. Source PDF SHA-256 stayed unchanged.
- **11 independent Poppler/PDF checks** on the native-saved fixture: stored geometry/color/width, AP styles, rasterized shapes, underlying page content and foreign annotation preservation.
- Independent headless reviewer reproduction was rerun against the corrected source: after first Arrow click → picker → Escape → one fresh click, no worker starts, retained shapes remain zero and the PDF stays clean. Metadata Undo/Redo/Save checks also passed.

All desktop QA used generated documents and owned Xvfb displays; no app was launched on the user's desktop and no private drawing set was used. Counts above are separate suites, not a combined performance score.

## Artifact and evidence

Development binary: `dist/glyph-markup-sheet-development`.

SHA-256: `67e11cda986f408d1a8c0da66056b02462164a70df8d308a6cbc2d70d069066c`.

`dist/markup-sheet-verification.json` records integrated source hashes, source backup path, native result metadata and artifact identity. Root logs: `dist/polish-root-{full-tests,clippy,fmt,build}.log`. Detailed native/Poppler captures remain in `/home/daniel/.cache/glyph-v55-parity/dist/polish-integrated-*`.

The pre-integration snapshot was verified before source copying. Existing untracked `src/app/editing.rs` and all unrelated checkout changes were preserved; no reset/stash/staging or blanket formatting was used.

## Still outside this increment

Text, polyline/polygon and broader markup styles; complete Tool Chest/markup-list and document-processing workflows. Measurement/calibration would require separate product scope, not inference from this increment. No new app release, installed-binary replacement, production Wayland benchmark, or universal "fastest PDF editor" claim.
