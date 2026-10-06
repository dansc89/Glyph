# Compact PDF-first UI review

## Scope

Local UI changes only. No release, installation, or normal desktop launch. Native checks run on owned Xvfb displays and generated PDFs; no user drawing is used.

- A single compact toolbar replaces three rows of text controls.
- Original, font-independent vector icons use 24-point targets, tooltips, selected-state outlines, disabled states, and named button metadata.
- Navigation, zoom, fit, page entry, view/select/rectangle/ellipse/delete remain directly available.
- Bookmarks automation, hyperlink generation, link visibility, and the disabled flatten action live in the overflow menu. Existing dirty/edit/pending-operation guards remain.
- The sidebar is narrower and clamped, with icon tabs for pages, bookmarks, and search.
- Title and status bars use less padding. Long filenames and messages truncate rather than wrapping; tooltips expose the full text. Error/status priority is retained. Shortcut help is behind the status help icon.
- Drawbridge's macOS implementation uses SF Symbols. No Apple font or Drawbridge assets were copied; these are original native vector drawings with familiar action meanings.

## Regression evidence

- The old saved executable failed the native compactness gate because its sidebar measured 327 px (>260 px).
- Headless layout regressions cover long filenames, stored wide sidebar sizes, toolbar height, footer height, and viewer area.
- Icon tests cover finite/bounded geometry, 24-point targets, tessellation at 1x/1.5x/2x, named click events, selected and disabled states, and stable IDs scoped to their owning UI.
- Existing ellipse, retry-preview, and navigation tests were changed to probe vector shapes/stable widget IDs rather than vanished text captions; their editing/worker/save/deletion assertions remain.
- A history-label regression failed with `Previous page` instead of `Back`, then passed after using one correctly named metadata/tooltip emission for each history action.

## Execution

`cargo test --locked --quiet -- --include-ignored --skip real_saltair --test-threads=1`:
**299 passed, 0 failed, 2 private-drawing tests filtered out.**

`cargo fmt --all -- --check`, strict all-target Clippy (`-D warnings`), and the optimized build all passed.

Final-binary native checks: **51 passed** (14 layout/navigation/fit, 32 mixed-shape save/recovery, 5 gesture-release save). The **9 QA-isolation checks** also passed. Independent focused re-review passed with no security concerns, logic errors, or remaining suggestions.

- `dist/compact-ui-final/checks.json`: layout, actual paper containment after fit, direct page entry, unique search-input sentinel, minimum and large window sizes, empty window, and unchanged generated input.
- `dist/compact-ui-final-save/result.json`: ellipse/rectangle mixed edits, Undo/Redo/delete, cross-page selection safety, multiple saves and backups, protective read-only refusal with retained edits, retry after restoring permissions, Undo across saved checkpoints, reopen/delete, and Escape cancellation.
- `dist/compact-ui-final-release-save/result.json`: save on gesture-release frame.

The updated save harness uses stable Ctrl+G page entry instead of coordinates for obsolete text navigation buttons. The compact harness OCRs a unique search sentinel only in the sidebar, verifies actual paper bounds with margins in the document pane at both sizes, and excludes the capture cursor.

## Measured native geometry (same 960×640 window, scale 1)

| Metric | Before | After |
|---|---:|---:|
| Sidebar right edge | 327 px | 202 px |
| Canvas top | 187 px | 75 px |
| Canvas area | 236,164 px² | 394,850 px² |
| Window area allocated to canvas | 38.4% | 64.3% |

Canvas area increased **67.2%** in this controlled case. At 1600×1000, the final viewer receives **77.0%** of window area. These are layout measurements, not claims about frame rate or all monitor/DPI configurations.

Before/after screenshots and machine-readable measurements are retained under `dist/compact-ui-baseline/` and `dist/compact-ui-final/`.

## Limits

No claim of complete Drawbridge parity, crash-free operation, or validation of real architectural drawing sets. The installed binary and public release remain unchanged. Unrelated pre-existing changes in the development tree were retained.
