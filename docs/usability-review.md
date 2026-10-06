# Glyph usability review

## Scope and verdict

This pass reviews the local native development build, not the public 1.2 viewer release. Drawbridge's published user manual is the workflow reference: <https://github.com/dansc89/Drawbridge/blob/main/USER_MANUAL.md>. A retrieved copy is in `dist/drawbridge-usability-reference.md`.

Glyph is not yet certified as fully polished, at feature parity, or faster than Drawbridge. This pass fixes concrete interaction and failure-state defects; comparative runtime benchmarks and representative large drawing-set testing remain outstanding.

## Confirmed and fixed

| Priority | Finding | Change and evidence |
|---|---|---|
| High | A disconnected editing worker dropped its owned session while claiming edits survived; a subsequent Save could load the original and clear dirty state. | Explicit unrecoverable state blocks further edit/save commands and the Save As picker. Failure text is truthful. Discard/close/reopen remains possible. Regression injects channel loss after a successful unsaved rename and verifies Save cannot clear dirty state or change the source. **This is fail-closed handling, not automatic snapshot recovery.** |
| High | Save failure text depended on the shared rendering-progress status, allowing later rendering messages to hide the error, including in the unsaved decision dialog. | Operation-owned error state remains visible in the footer and unsaved dialog despite subsequent rendering status. Regression covers both views. |
| Medium | Incoming files could replace an existing unsaved transition or bypass an active rename draft. | The deferred-open gate preserves active modal ownership. Regression covers close/save decisions and uncommitted rename drafts. |
| Medium | Ctrl+Shift+S did not invoke Save As. | Shift-specific shortcut is consumed before Ctrl+S; controlled-picker regression verifies dispatch without writing the source. Earlier claims that this shortcut worked were incorrect. |
| Medium | Save As accepted extensionless or non-PDF destinations, producing files that normal PDF opening/drop handling could reject. | Extensionless destinations gain `.pdf`; explicit non-PDF extensions are rejected before starting a worker, with persistent feedback. Regression reopens the normalized PDF and checks source preservation. |
| Medium | No clickable Open action was available in the Document menu, including the empty state. | Enabled Open PDF action with Ctrl+O hint; native screenshots and UI regression verify discoverability. The empty canvas itself still has no direct Open button. |
| Medium | An over-limit rename closed the dialog and discarded the user's draft on validation failure. | The shared backend title limit is checked in the dialog. Invalid drafts remain editable, with an explanation and disabled Rename. Enter-key regression verifies retained draft and unchanged PDF. |
| Medium | Undo/Redo could start a PDF worker with no applicable history; menu readiness reflected session existence rather than stack availability. | Separate stack availability queries, no-op handling without opening a PDF, and per-action enabled state. Regressions cover stack transitions and empty history. |
| Medium | Long filenames pushed the top-header page counter outside a minimum-sized window. | Bounded, elided title with full-title hover text. UI regression and before/after native screenshots verify the page counter remains visible. This does **not** solve sidebar clipping. |
| Low | Close left document-scoped navigation history behind. | Close clears Back/Forward history. Regression verifies stale navigation is unavailable after closing. |

## Remaining confirmed or bounded follow-ups

1. **Small-window layout:** at 960×640, long filenames widen the sidebar and clip the document-card count; Fit width can wrap into a tall, fragmented label. Footer wrapping also consumes extra canvas height. See `dist/usability-final/04-long-filename.png`.
2. **Glyph/font fallback:** visible missing-symbol boxes in navigation arrows and shortcut help. A native font-fallback or plain-text alternative needs review across Omarchy themes.
3. **Action presentation:** empty-window automation/view controls can appear available despite lacking a document. Review enabled-state consistency and add a direct empty-canvas Open button.
4. **Recovery/discoverability:** retained save backups are still hidden sibling files, without a recovery browser. Worker failure now prevents misleading/destructive retry, but does not reconstruct a lost unsaved session. Explicit error dismissal and persistent unsaved-state presentation with long filenames need further polish.
5. **Performance/parity:** no side-by-side Drawbridge benchmark was run. Representative large drawings, HiDPI/multi-monitor behavior, accessibility, and the documented remaining editing feature gaps still need dedicated passes.

## Actual verification

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --locked --all-targets -- -D warnings`: passed.
- `cargo test --locked --quiet -- --include-ignored --test-threads=1`: **213 passed, zero failures, zero ignored**.
- `cargo build --locked --release`: passed; executable is `target/release/glyph`.
- `git diff --check`: passed.
- `python3 scripts/test-editing-qa-isolation.py`: **8 passed**.
- `python3 scripts/editing-qa.py --binary target/release/glyph --out dist/usability-editing-final`: **9 native checks passed**, covering rename, undo, Save, unchanged rendered pixels after bookmark saving, Cancel/Discard and reopen persistence.
- `python3 scripts/usability-qa.py --binary target/release/glyph --out dist/usability-final`: **6 screenshot/interaction scenarios completed** (not six comprehensive assertions): minimum window, Document menu, search focus, long filename, empty window and empty Document menu.
- Owned Xvfb allocation and generated PDFs only; the application was not launched on the user's display, and user PDFs were not edited.
- Save As routing/filename and worker-loss cases use controlled callbacks or injected failures. This is not a native portal integration test or evidence of a naturally occurring worker panic.

The OCR editing harness needed a bounded, inverted sidebar crop for Rename on the current theme. The menu existed; whole-window OCR missed it. The final native checks passed after improving OCR, not by substituting fabricated UI results.

No commit, push, release, or installer update was performed. Pre-existing unpublished editing changes remain in the working tree. Independent final source review completed with `passed: true`, no security concerns and no blocking logic errors. The reviewer did not run tests or GUI checks. Two non-blocking follow-ups were identified: schedule repaint/polling while edits are pending so unexpected worker exit is surfaced without another input event; add coverage for native window-close requests while rename or unsaved-transition dialogs already own input.
