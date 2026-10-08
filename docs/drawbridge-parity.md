# Drawbridge feature parity

Reference target: [Drawbridge v5.5](https://github.com/dansc89/Drawbridge/releases/tag/v5.5), pinned source commit `c1c54cbc744539b28c64815c372875f9768b806e`.

**Glyph does not yet have full feature parity. Equal production stability and performance are not established.** Glyph remains a native Rust/egui/PDFium application with Omarchy integration, not a web wrapper or a direct platform port.

## Published release: Glyph 1.5

The [1.5 downloads](https://github.com/dansc89/Glyph/releases/tag/1.5) include:

- PDF viewing, persistent Fit Page/Fit Width, zoom/pan and page/viewport history.
- Virtualized page thumbnails, bookmark navigation and direct page-number entry.
- Selectable-text search, highlighting, selection/copy and whole-word double-click selection. Scanned pages do not gain OCR.
- Internal hyperlink navigation, heuristic sheet-ID detection, automatic bookmarks and collision-safe cross-sheet-link exports. These are not reviewed OCR/title-block-zone workflows.
- Native Glyph-owned rectangle/ellipse annotations, selection/deletion and shared Undo/Redo. Imported annotations are preserved, not automatically made editable.
- Existing bookmark-title and embedded page-label editing.
- Guarded Save/Save As with staged validation, source-conflict checks and retained backups.
- Compact PDF-first controls and live Omarchy dark/light palettes.
- Loading/save-stage feedback, advisory overdue warnings, renderer-failure guards and high-zoom tile reuse. Restoring previews after renderer death requires restarting Glyph; saving owned changes remains available.

See [1.3 editing/markup verification](releases/1.3.md), [1.4 compact UI](releases/1.4.md), and [1.5 reliability verification and limits](releases/1.5.md). The 1.5 development verification records **335 portable Rust tests**, with two private drawing regressions excluded. These are release-specific counts, not a live total for ongoing development.

## Newer local development — not in 1.5 downloads

Persistent two-click Line/Arrow tools have been implemented and verified separately, including shared history, transactional saving, crop/rotation cases and independent PDF rendering. This work has not been published as a new binary release.

Move/resize, endpoint editing, editable stroke properties and searchable Go to Sheet are ongoing polish work. Their interrupted implementation must pass fresh review and native/persistence checks before being advertised as completed or included in a release. Building a local executable does not update the installed application automatically.

## Remaining maturity gaps

- Complete editable-markup lifecycle: manipulation/styles, text boxes, pen/polyline/polygon and geometry-aware editing.
- Searchable Go to Sheet and broader document/session navigation, including tabs/session persistence.
- Reviewed sheet naming with selectable title-block zones, OCR fallback, duplicate detection and hierarchy management.
- Broader bookmark editing and reviewed batch cross-sheet linking with exclusions, cancellation and rollback.
- Printing and other drawing-set/document workflows.
- Appearance-preserving, reversible Flatten/Unflatten. The old destructive annotation-removal action is disabled and is not a flattening solution.
- Verified lossless reduction and document inversion where appropriate.
- Representative architectural/scanned PDF sets, native Wayland/mixed-DPI behavior, dense-text responsiveness and production performance validation.

Dormant or disabled legacy Drawbridge code is not proof of an enabled v5.5 feature. Likewise, generated fixtures establish regression behavior, not universal interoperability or production-corpus parity.

## Verification policy

Each increment requires behavioral test-first coverage, formatting/static checks, source review, native interaction on owned displays, and save/reopen/content-preservation checks where it changes PDF data. Independent-reader rendering validates exported appearance rather than relying solely on Glyph overlays.

No claim of full parity, guaranteed crash-free operation, bounded filesystem/PDFium waits, or being the fastest editor is made. Private drawing fixtures are not uploaded to CI or published as screenshots.

## Historical records

The [archived development log](drawbridge-parity-history.md) preserves earlier v4.6/v4.8 comparisons, exact-build test counts and installation/publication observations. Its “local,” “unpublished,” “current,” and version statements describe those historical increments, not today's release status.
