# Drawbridge v4.6: source-grounded Glyph baseline

Baseline: [Drawbridge v4.6](https://github.com/dansc89/Drawbridge/releases/tag/v4.6). The tagged checkout was inspected, not the moving main branch. Drawbridge was not executed on this Linux host; its published performance figures were not reproduced.

## Documentation discrepancy resolved

The tagged `USER_MANUAL.md` says markup authoring is unavailable. That is stale. `ToolMode.swift` describes the disabled legacy system, not all current tools. The new system is deliberately independent of it:

- [`RectangleMarkup.swift:120–123`](https://github.com/dansc89/Drawbridge/blob/v4.6/Sources/Drawbridge/RectangleMarkup.swift#L120-L123): Select, Rectangle, Ellipse, Line, Arrow, Text, Polyline, Polygon.
- [`RectangleMarkupToolbar.swift:75–103`](https://github.com/dansc89/Drawbridge/blob/v4.6/Sources/Drawbridge/RectangleMarkupToolbar.swift#L75-L103): real actions, editability guards and active-tool highlighting.
- [`MainViewController.swift:2201–2212`](https://github.com/dansc89/Drawbridge/blob/v4.6/Sources/Drawbridge/MainViewController.swift#L2201-L2212): markup controls are in the default toolbar.
- `MarkupPDFView.swift:865,900,908`: pointer input reaches the new controller.

The earlier claim that v4.6 excluded markup creation was wrong. Disabled legacy tools must not be confused with the enabled replacement controller. Equally, legacy freehand/highlighter/cloud/measurement/snapshot/page-conversion implementations are not proof that those tools are enabled.

## Current workflow comparison

| Workflow | Drawbridge v4.6, source inspection | Glyph local build |
|---|---|---|
| Viewing, fit modes, thumbnails, history, selectable-text search/copy | Implemented; targeted viewer transition/search tests | Implemented; native regression harnesses and Rust tests |
| Existing bookmark rename, Undo/Redo, Save | Implemented | Implemented locally; public viewer 1.2 does not include editing |
| Embedded PDF page-label rename | Sidebar double-click/context menu; optional corresponding bookmark update | **Now implemented separately** through Document → Rename current page label; persisted labels, Undo/Redo, Save/Save As |
| Reciprocal bookmark/label synchronization | Optional prompts on both rename paths | Not implemented; these edits intentionally remain independent |
| Sheet naming | Number/title zones, OCR fallback, review and generated navigation | Selectable-text heuristics only; zone/review/OCR workflow missing |
| Bookmark add/delete/groups/hierarchy | Broader management | Existing-title editing only |
| Reviewed transactional hyperlink changes | Navigation/link writer and document-processing workflows | Heuristic collision-safe export exists; editable-session reviewed batch changes are missing |
| Owned shape/text markup | Seven creation tools plus Select, styles, handles/vertices, text editing, Undo/Redo | **Not implemented** |
| Reversible appearance-preserving flattening | Document-processing workflow | Disabled; deleting annotations is not flattening |
| Verified lossless reduction | Document-processing workflow | Missing |
| Performance | Published real/synthetic application-save figures, not reproduced here | Existing benchmarks pass; representative end-to-end comparison remains unmeasured |

Page-label source: `MainViewController.swift:825–831,1096–1135,1230–1269,4651–4686`; `PDFTKBookmarkWriter.swift:163–174`. Shape writer: `PDFRectangleWriter.swift:33–74`. Default release test gate: `.github/workflows/release-macos.yml:46–56`.

## Completed Glyph increment

- Read effective embedded labels, including decimal/Roman/alphabetic numbering, prefixes and number trees; keep physical page order separate from display labels.
- Native single-page label dialog with focus, Enter/Escape, nonempty Unicode validation and a 256-character input limit. Other pages and bookmark titles remain unchanged.
- Store changes in the existing background editor with shared bounded history and save checkpoints, not a sidebar-only override.
- Undo/Redo refresh labels; discard restores the saved metadata snapshot; saving embeds `/PageLabels` and reopening reads the same label.
- Strict editing rejects malformed label trees instead of silently overwriting them. Read-only viewing falls back to ordinal labels so bad optional navigation metadata does not make an otherwise valid drawing unviewable.
- Existing security, no-clobber Save As, staged verification, conflict handling and retained-backup guarantees apply. They preserve displaced data; they do not provide exclusive locking.
- Native window-close requests respect an existing rename draft or unsaved-document decision. Pending edit workers schedule bounded repaint polling so an unexpected disconnect is discoverable without new pointer/keyboard input.
- Added actual PDF-worker coverage for opening an 11-page document, rendering its last page, then opening a one-page document: page zero and the new source's pixels appear; old navigation history is unavailable. This is transition regression coverage, not a large-set performance benchmark.

## Verification artifacts

- `scripts/page-label-qa.py`: nine assertions on generated PDFs using an owned, validated Xvfb display. Mouse-opened label dialog, keyboard rename/Undo/Redo/Save, source-byte preservation before Save, exact displayed drawing-pixel preservation across Save, retained original backup, and persisted label after native reopen.
- `dist/page-label-final-qa/result.json`, `02-label-dialog.png`, `09-reopened-label.png`.
- `dist/page-label-editing-regression/result.json`: existing nine native bookmark/save/discard checks.
- `dist/page-label-usability-regression-output.json`: exploratory screenshots; not equivalent to a complete usability sign-off.
- Display-isolation regressions remain mandatory. The user desktop and user PDFs were not used for GUI QA.

Final verification: 234 Rust tests (including ignored checks/benchmarks), formatting, warnings-denied Clippy, optimized build and diff checks passed. Nine new native label checks, nine existing editing checks and eight display-isolation checks passed. Independent read-only source review found no blocking security/logic issues; it ran no tests/GUI. Its suggested additional app-level mixed title/label checkpoint-after-failed-open case remains a follow-up. Neither these fixtures nor source review prove full parity or native Wayland/GPU latency.

## Next substantial milestones

1. Full bookmark management and optional synchronized label changes; reviewed zone/OCR sheet naming.
2. Transactional reviewed link editing with cancellation, undo and conflict handling.
3. Owned eight-tool markup milestone with explicit mode UI, annotation ownership, crop/rotation geometry, text/vertex editing and verified vector save.
4. Safe reversible flattening and verified lossless reduction.
5. Representative real drawing-set interaction/save benchmarks and remaining narrow-window, missing-glyph and recovery-discoverability polish.

All changes here are local development work. No new release, installation, user-desktop launch or complete Drawbridge parity is claimed.
