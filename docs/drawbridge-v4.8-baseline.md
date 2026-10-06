# Drawbridge v4.8 parity and stability audit

## Verdict

**Glyph is not feature-complete relative to Drawbridge v4.8. Equal production stability and save latency are not established.** Regression-green is not a comparative crash-rate or performance measurement.

Audit reference: GitHub latest-release endpoint returned v4.8; requested tagged source fetched into `/home/daniel/.cache/glyph-reference/Drawbridge-v4.8`, pinned commit `424c1b8a07ec634d56ffc23d1f0f306951d38075`. This audit examines enabled controllers/writer code, not just the stale manual or disabled legacy `ToolMode`.

## Release-specific changes

v4.8 reports safer/faster repeated saving, reduced annotation searches, geometry-aware line/ellipse selection, no-op edit suppression, inline text Undo/Redo and preview/draft cleanup, and consistent drawing defaults.[1]

Tagged `PDFRectangleWriter.swift:21–86` confirms complete source-byte matching, encoded-stream/object verification, owned-record checks and concurrent-source-change checks remain on the save path. Its cached inspection requires matching URL and complete bytes (`:255–258`), expires after 60 seconds and is budgeted at 96 MiB combined PDF/JSON (`:82–84,244–271`).[3] Glyph retains its editable document, but still serializes, reparses and verifies the full staged PDF for every save (`src/pdf/edit_session.rs:600–650`). Different architectures need not use identical caches; comparable whole-app save latency has not been measured.

## Markup parity

| Capability | Drawbridge v4.8 | Current local Glyph |
|---|---|---|
| Rectangle and ellipse creation | Enabled | Implemented: native Square/Circle, explicit AP, Save/reopen |
| Line, arrow, text, polyline, polygon creation | Enabled | Missing |
| Selection and delete | Enabled | Implemented for Glyph-owned rectangle/ellipse only |
| Shape move/resize; endpoint/vertex editing | Available in owned controller | Missing |
| Inline text authoring/editing and draft-local Undo/Redo | Available | Missing |
| Stroke color/width, text font size, polygon fill | Enabled toolbar controls | Missing; red, unfilled, 2 pt only |
| Geometry-aware ellipse selection | Ellipse equation; empty bbox corners excluded | Corrected in follow-up: ellipse interior with screen-space halo; empty corners excluded |
| Shared markup persistence/history | Implemented | Implemented for current shapes plus bookmark titles/page labels |

Drawbridge's toolbar wires all eight modes including Select and seven drawing tools (`RectangleMarkupToolbar.swift:100–145`).[2] Its controller implements geometry-aware hit testing (`RectangleMarkup.swift:577–603`).[5] Glyph's modes remain View/Select/Rectangle/Ellipse (`src/app/markup.rs:4–11`). The original audit found bbox-only ellipse selection; the follow-up now uses `shape_hit` to exclude empty corners. Native corner-click/Delete failed before the fix and passes afterward. See [ellipse selection/save-stress review](ellipse-selection-save-stress-review.md). This closes that interaction gap, not a claim of full reference-product parity.

## Other enabled workflow parity

| Workflow | Current local Glyph | Tagged v4.8 evidence |
|---|---|---|
| Core viewing/search/thumbnails/selectable text/internal links | Implemented; not a complete interaction/performance parity claim | Enabled viewer/menu controls [6] |
| OCR-assisted sheet naming, user-selected number/title zones and pre-apply review | Partial: selectable-text ID heuristic/export only, no zones/OCR/title/review | `AppDelegate.swift:247`, `MainViewController.swift:4821–4912,5074–5108` [6][7] |
| Bookmark rename/delete and optional page-label synchronization | Partial: title rename and separate label editing; no subtree deletion/sync prompt | `MainViewController.swift:863–871,1096–1218` [7] |
| Batch sheet-reference linking | Partial: text-based ID linking/export; no scanned-page OCR/target-zone fallback | `AppDelegate.swift:258`, `MainViewController.swift:5111–5132,5454–5514` [6][7] |
| Reversible appearance-preserving Flatten/Unflatten | Missing; Glyph Flatten button deliberately disabled | Enabled menu and recovery-aware processing [6][8] |
| Verified lossless Reduce | Missing | Enabled menu and processing [6][8] |
| Recent-file reopen/clear | Missing UI | `AppDelegate.swift:195–214` [6] |

Non-markup read-only audit full report: `/home/daniel/.hermes/cache/delegation/subagent-summary-0-20261005_160127_582887.txt`. Parent verified enabled menu/Flatten/Reduce wiring and Glyph's disabled Flatten and current automation buttons. Source inspection is not native execution.

**Do not inflate the baseline:** enabled bookmark add/reparent/reorder and page assembly/conversion were not found. Legacy command wrappers without available menu/control wiring are not v4.8 parity requirements. Glyph may pursue those separately as enhancements.

**Autosave nuance:** tagged `MainViewController+Persistence.swift:561–575` explicitly excludes unsaved new-controller markups from sidecar autosave; sidecar state is not equivalent to a verified saved PDF. Do not claim v4.8 automatically crash-recovers all unsaved drawings.

## Stability evidence and limits

**Verified follow-up:** 291 Rust tests including ignored passed; 96 freshly executed native assertions plus layout/usability passed. Generated backend 100-page/200-shape workload completed seven Saves and one Save As, including rejection/recovery/history checks. A separate native 100-page workload saved three new shapes while preserving 200 originals; independent Poppler renders of untouched pages 2 and 100 are pixel-identical. Production sets and comparative stability remain unproven. Details and current binary hashes: [follow-up review](ellipse-selection-save-stress-review.md).

The following counts/hashes are retained as **historical initial-audit evidence**, not the new build:

- Fresh initial-audit run: `cargo test --locked --quiet -- --include-ignored --test-threads=1`: **285 passed, 0 failed, 0 ignored**, 37.38 seconds. Real regression execution, not a count copied from documentation.
- Existing exact-build native result JSON revalidated: **86 passing assertions**, zero failed, across mixed/repeated shape saves, immediate-release saving, read-only rejection/retry, bookmark and label save/reopen. These were **not rerun** during this read-only parity audit.
- Tested release binary SHA-256: `6269512ec073dad0935c28c2879dfa799b89f406b574ba15b2173b4a0dc68b8a`; matches the prior optimized native-tested build.
- Installed binary SHA-256: `e6e1e6975b31d7c4446cb1a3d893e274e1da38bb1dd0746ceaf29dbcb29406cd`. **The installed app is behind the audited development build.** No installation or user-desktop launch occurred.
- Glyph protections include staged object verification, source fingerprints, Linux atomic exchange, retained displaced-original backups, Save As no-clobber, signature/encryption rejection and dirty/history retention on rejected saves. Known canonical integral-real and same-frame finishing-stroke bugs were corrected and are in the passing suite.
- Drawbridge reports 124 executed regressions, 13 optional skips, manual text/keyboard/save QA, Apple Preview interoperability and independent original-content rendering across four rotations after repeated Save/Flatten/Reduce/Unflatten.[1][4] These are upstream-reported results; Drawbridge's macOS application was not executed here.
- Upstream reported 100-page architectural saves at 1.41 seconds initially and 1.03–1.15 seconds afterward.[1] Different machine/platform/corpus and test coverage prevent comparisons with Glyph's test counts or synthetic screenshot timing.
- Glyph's production mixed-markup save corpus, high-markup-count interaction/RSS/file growth, abrupt process termination, disk-full/commit failures and long-running native Wayland workflows do not yet have equivalent validated evidence. Some unit/failure regressions exist; these broader production workflows remain unproven.
- Native Save As portal interaction remains untested; controlled picker and real Save As PDF I/O tests are not a portal click-through.

## Priority

1. Establish production-style save/reopen, original-content preservation and failure-recovery gates on disposable copies of representative architectural/civil PDFs; include many markups, repeated saves, external modification and independent rendering/readers. Compare initial/repeated save latency without removing preservation checks.
2. Ellipse empty-corner selection is corrected. Next add shape move/resize and styles without losing shared persistence guarantees; measure/reclaim orphan-object growth without breaking Undo or original-object protections.
3. Add line/arrow, text, polyline/polygon as complete tool-to-save-to-reopen vertical increments; include no-op history and draft Undo/cancel behavior when those features exist.
4. Close verified non-markup workflow gaps rather than assuming old backend helpers or legacy source imply an enabled feature.

## Sources

[1] https://github.com/dansc89/Drawbridge/releases/tag/v4.8 — Drawbridge v4.8 release
[2] https://github.com/dansc89/Drawbridge/blob/v4.8/Sources/Drawbridge/RectangleMarkupToolbar.swift
[3] https://github.com/dansc89/Drawbridge/blob/v4.8/Sources/Drawbridge/PDFRectangleWriter.swift
[4] https://github.com/dansc89/Drawbridge/blob/v4.8/docs/MARKUP-INTERACTION-REVIEW-2026-10-05.md
[5] https://github.com/dansc89/Drawbridge/blob/v4.8/Sources/Drawbridge/RectangleMarkup.swift
[6] https://github.com/dansc89/Drawbridge/blob/v4.8/Sources/Drawbridge/AppDelegate.swift
[7] https://github.com/dansc89/Drawbridge/blob/v4.8/Sources/Drawbridge/MainViewController.swift
[8] https://github.com/dansc89/Drawbridge/blob/v4.8/Sources/Drawbridge/MainViewController%2BPDFProcessing.swift
