# Reliability verification: implemented local tools

Scope: current local Glyph build, not planned Drawbridge markup authoring. No new application features were added in this pass. All GUI runs used generated PDFs on dynamically allocated owned Xvfb servers, not the user desktop.

## Results

- 235 Rust tests passed, including ignored document regressions and benchmarks.
- Formatting, warnings-denied Clippy, optimized release build and diff checks passed.
- 30 native viewer assertions passed: thumbnails, fit modes/resizing, direct page entry, history including exact restored-view comparison, text selection/native clipboard, rapid navigation, internal link clicks, full-document search/focus, high-resolution viewport tiles and middle-button pan.
- Nine native page-label assertions and nine native bookmark/save/discard assertions passed. Before-save source bytes remained unchanged; saved drawing pixels were unchanged; original backup and persisted metadata were verified.
- Nine isolation tests passed. Eight exercise allocation/startup/death/liveness behavior; the added test gates the legacy viewer runner's use of the shared owned-server helper instead of a fixed display.

Artifacts: `dist/reliability-viewer/results.json`, `dist/reliability-labels/result.json`, `dist/reliability-editing/result.json`, screenshots and logs in those directories.

## Stronger combined-workflow coverage

Added `mixed_edits_discard_after_failed_open_restores_latest_saved_checkpoint`:

1. Change a bookmark and page label, then Save.
2. Make further changes to both.
3. Request a missing replacement PDF and choose Discard.
4. Verify failed opening keeps the existing document, restores both latest saved metadata snapshots (not initial values), leaves its saved bytes untouched, and permits a fresh label edit without changing disk bytes before Save.

This passed on its first behavioral run: additional regression coverage for already-correct behavior, not a newly reproduced application defect or claimed RED→GREEN fix. It closes the previous independent review's checkpoint follow-up.

## Defects corrected

The legacy `performance-qa.py` runner still used fixed display `:79`. It could interact with an existing display if allocation failed. A failing safety gate was added, then the runner was migrated to the shared `PrivateXvfb` allocator, validated child liveness and guaranteed cleanup. Its complete native run subsequently passed. This was a test-harness defect, not an application-data-loss finding.

Corrected plural `results.json` references for the label/bookmark runners; those runners write singular `result.json`.

## Limits

These results support the exercised tools and failure paths, not unconditional reliability. Native Save As portal interaction was not manually exercised; dispatch, normalization, no-clobber and PDF persistence have Rust coverage. Representative large/scanned drawing-set UX, Wayland/GPU latency, concurrent noncooperating writers and broader production compatibility remain outside these generated GUI runs. Existing Save preservation is not exclusive locking. Known narrow-window/font/recovery-discoverability polish remains. Drawbridge's shape/text markup tools are not yet implemented in Glyph and are not counted as verified.

No release, installation or user-desktop launch was performed.
