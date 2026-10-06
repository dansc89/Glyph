# Native layout polish pass

## Delivered changes

- Long filenames no longer expand the sidebar or push its page counter offscreen. The filename is single-line/ellipsized with full-path hover; page position has its own line.
- Fit Page/Fit Width stay as whole buttons and wrap to the next toolbar row. Shared button styling uses direct `add_enabled` dispatch, not a nested disabled UI scope. The initial Extend-only fix still overflowed a 90 px test toolbar; strengthened bounds coverage caught that and the owning layout was corrected.
- Long embedded sheet labels remain one-line captions inside thumbnail cards, retaining the physical page index and full text on hover.
- Previous/Next controls and history hints use ASCII, avoiding missing arrow glyphs in the active desktop font.
- Long render-status messages are single-line/ellipsized with full-message hover instead of multi-line filename logs. Persistent failure color/state and unsaved-dialog error presentation are retained. Overall footer organization still merits refinement.

## Behavioral evidence

Observed failures before corrections: long-name sidebar exposed only one of two page counters; nested Fit Width displayed five text rows; long caption exceeded card bounds; navigation lacked font-independent labels; settled long footer status occupied seven text rows. All corresponding regressions now pass. Whole-button bounds also failed on the first toolbar fix (right edge 129.40625 in a 90 px viewport), then passed with direct enabled-button layout. Footer checks warm up panel layout before asserting geometry; the initial un-warmed probe was not valid evidence.

## Verification

- 240 Rust tests including ignored passed.
- Formatting, warnings-denied Clippy, optimized release build and diff checks passed.
- Actual optimized build passed 30 native viewer, nine page-label and nine bookmark/editing assertions; nine isolation checks passed.
- Seven exploratory screenshots cover minimum window, Document menu, search, long filename plus long embedded label, empty viewer/menu and expanded sidebar. These screenshots are additional evidence, not seven automated assertions.
- Visually inspected `dist/polish-layout/04-long-filename.png` and `07-wide-sidebar.png`: filename/counter fit; captions are bounded; Fit Width stays complete and moves to the next row with expanded sidebar. An initial batched drag did not resize the panel; the corrected gesture spans input frames and visibly expands it.
- Bookmark QA's click target moved with the added page-counter line. The runner was updated to the observed tab center, with its existing bookmark/save/pixel-preservation assertions retained. OCR tab selection was attempted but did not reliably recognize the low-contrast tab; pixel-based fixture targeting remains a harness limitation.
- One extra isolated exploratory launch failed with Winit `XOpenDisplayFailed`; a repeat using the normal terminal runner succeeded. Cause is unconfirmed and not claimed fixed. The complete build/native verification chain itself passed.

Artifacts: `dist/polish-viewer/results.json`, `dist/polish-labels/result.json`, `dist/polish-editing/result.json`, `dist/polish-layout/observations.json` and associated screenshots/logs. Independent read-only review identified the intermediate nested-scope toolbar overflow and its row-count-only test blind spot. The parent had already reproduced that overflow and implemented the same direct-enabled-button remedy before the review arrived. Read-back confirmed the final code; the bounds/one-line regression also passed for both enabled and disabled states. The reviewer found no other concrete new issues in the reviewed sidebar/caption/navigation changes and ran no tests or GUI. The footer adjustment was added later and is covered by parent verification, not that source-review scope.

## Remaining scope

This addresses visible defects, not full Drawbridge polish/parity. Contextual editing discoverability, backup/recovery UX, footer/action organization, representative drawing-set responsiveness, production-corpus coverage and remaining markup/bookmark workflows are still open. No publication, installation change or user-desktop launch occurred. Test PDFs were generated fixtures only.
