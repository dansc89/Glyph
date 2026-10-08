# Drawbridge v6.0 acceptance contract

## Definition of success

Glyph must match **all enabled Drawbridge v6.0 features and complete user workflows**, or improve on them, while remaining a native compact Omarchy application. Passing a feature increment is not project completion. Missing, partial, disabled, or unverified required workflows remain open failures against this contract. No parity percentage is inferred from test counts.

Pinned reference: https://github.com/dansc89/Drawbridge/releases/tag/v6.0, source commit `3396a63a0ef0d680b1479e1bca225da671976577`. Local reference: `/home/daniel/.cache/drawbridge-v6.0-reference`. Release metadata is preserved in `dist/drawbridge-v6.0-reference.json`. Detailed implementation/source audit is separate from this acceptance policy.

## Required workflow families

Reference inventory below is backed by tagged `README.md:17–38,42–66`, not a claim that all controller behavior was audited here:

- Viewer: thumbnails, bookmark and sheet search, document text search/copy, fit modes, navigation history, page-specific selection, scrolling alignment, display-only inversion and native printing.
- Sheet processing: chosen sheet-number/title regions, extraction/OCR, rotations, progress/cancel, review before applying names/bookmarks, ambiguity-safe and idempotent batch links, preserved source/navigation and saved/reopened destinations. The region-only OCR/orientation recovery optimization is part of the current v6.0 release notes.
- Complete markup vocabulary: Pen, Rectangle, Ellipse, Line, Arrow, Polygon, Polyline, inline Text; matching placement/edit/manipulation, color/weight/font/fill controls, input ownership, cancellation, selection/deselection and cross-page centering. Glyph's current ASCII-only Courier text is a partial foundation, not full typography/encoding/wrapping parity.
- Document-wide Markups List: page/comment/author, author configuration, search, jump/center, multiselection and batch deletion with Undo; locked annotations visible; links/forms excluded.
- Imported standard unlocked markups: selection, move and deletion with Undo while preserving source drawing/AP/resource structure. Glyph's current preservation-only handling is not this editing workflow. Locked/flattened objects must remain protected.
- Page/bookmark management: multiselection, confirmed deletion, undo/redo after Save, bookmark deletion without PDF-page deletion, correct destinations/labels/history after structural changes.
- Persistence: Save/Save As, real-copy no-clobber behavior, dirty/draft protection, recoverable failures, original-content/foreign-object preservation, history across successful saves, interoperable saved appearances rather than overlay-only UI.
- Processing/output: appearance-preserving reversible flatten/unflatten after reopen, embedded recovery, protected unsupported/hidden annotations and links/forms, smaller-only lossless reduction with decoded-content verification, unchanged image resolution.
- Native workflow quality: keyboard flow, compact viewer-dominant UI, real cursors/sidebar resizing, usable controls at supported viewport/DPI/theme settings, bounded caches/workers, no UI freezes during editing or processing.

## Verification needed to close each row

1. Source-backed reference behavior and explicit Glyph implementation locations.
2. Behavioral RED→GREEN for implementation changes; distinguish already-green additional coverage and harness fixes.
3. Native input and visible result, not only internal model calls. Tests use generated public fixtures/owned displays; real representative sets require explicit authorized access.
4. Save/reopen and independent-reader structure/appearance verification. Validate crop, rotation, UserUnit, unsupported objects, collisions and failure recovery as applicable.
5. Full portable tests, strict all-target lint, formatting, optimized build, no unexplained excluded regressions. Private unavailable fixtures are disclosed, never counted as passing.
6. Representative matched-workload performance: document size/complexity, scan/text content, render resolution/rotation, cache state, hardware/backend, p50/p95 input latency, total processing latency, save latency and memory are recorded. End-to-end and stage timings stay separate. No benchmark superiority from headless frame tests or release-note timings alone.
7. Native Omarchy/Wayland acceptance, not merely Xvfb. Builds are not automatically installed or launched on the user's desktop.

## Reference limitations do not create imaginary requirements

Tagged `README.md:59–66` explicitly excludes measurement/calibration, secure redaction, snapshot pasting, page combining/conversion, and original-PDF-text editing; these are optional future enhancements, not verified v6.0 parity obligations. Imported text/style/node editing is not currently supported by the reference; standard unlocked imported selection/move/deletion is. Signed/encrypted saving limitations and recovery-dependent unflattening must remain explicit. Closing a document need not preserve session Undo history beyond the reference contract.

## Current verdict

**Not complete parity; at-least-as-good production quality/performance remains unproved.** The accepted text increment is recorded in [text-markup-verification.md](text-markup-verification.md). Earlier v5.5 inventory/implementation documents are historical. No release or installation follows implicitly from this contract.
