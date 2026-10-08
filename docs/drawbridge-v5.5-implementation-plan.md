# Drawbridge v5.5 parity: historical implementation plan

> Superseded acceptance target: **Drawbridge v6.0**, pinned source `3396a63a0ef0d680b1479e1bca225da671976577`. This document preserves the earlier milestone history, not the current definition of completion. The persistent-text intermediate increment is documented in [text-markup-verification.md](text-markup-verification.md); it does not establish complete v6.0 parity.

## Baseline

Target: Drawbridge `v5.5`, resolved commit `c1c54cbc744539b28c64815c372875f9768b806e`; the latest-release endpoint independently returned v5.5. Glyph baseline: released 1.5, commit `83b85642986f1f1e0da94e9c7661151e2af30cf9`.

The source-backed inventory is [drawbridge-v5.5-gap-audit.md](drawbridge-v5.5-gap-audit.md). That inventory describes the baseline, not later working-tree additions. Source availability, matching interaction, and production reliability/performance are separate verdicts. Native Omarchy integration and compact PDF-first chrome remain constraints throughout.

## Current increment

Line and arrow authoring: two-click start/end, live preview, Escape cancellation, finite normalized geometry through crop/rotation, true vector PDF `/Line` annotations and open-arrow endings, selection/deletion, shared Undo/Redo and transactional Save/reopen. Status: **completed and verified**, including crop/rotation, native persistence, independent PDF rendering and review; see [verification](line-arrow-verification.md). The follow-on geometry/stroke-property and Go to Sheet increment is now verified as development source; see [fresh integration evidence](markup-sheet-verification.md). Adding these increments is not full parity.

## Prioritized follow-on milestones

1. **Geometry and stroke editing — completed/verified development:** move/corner resize and endpoint handles; compact RGB/physical-width properties, retained valid selection, shared history and transactional persistence; preserved foreign annotations and shared/aliased appearances. Defaults for subsequent shapes are session-local; restart-persistent preferences and broader fill/opacity styles remain follow-ons.
2. **Complete review markup vocabulary:** on-page text with editable text/font size, then freehand pen, polyline, polygon and polygon fill. Test fonts/encoding, pointer ownership, cancellation, save/reopen and independent-reader appearances; do not ship overlay-only drawings.
3. **Go to Sheet — completed/verified development:** searchable labels/bookmark titles/physical page numbers, keyboard-safe modal ownership and distinct duplicate labels. Pending: explicit OCR regions, number/title review before applying, progress/cancellation and ambiguity-safe batch links. Existing text extraction is not OCR.
4. **Multiple documents:** tabs preserving each document's page/zoom/navigation/search and owned dirty session; save/discard/cancel across every transition, external-change reload, bounded caches and stale-completion rejection. Do not share one dirty checkpoint across tabs.
5. **Verified document processing and output:** appearance-preserving reversible Flatten/Unflatten, smaller-only lossless reduction with decoded-content verification, native printing at actual size, and display-only inversion. Glyph's disabled legacy annotation-removal routine must never be enabled as a substitute for flattening.

Go to Sheet was delivered in parallel without mutating the source PDF. Measurement, calibration/takeoff and page conversion are not usable v5.5 features and are not required for this baseline.

## Every-increment delivery gates

- Observed behavioral RED → GREEN for new behavior; additional already-green tests are labeled coverage.
- Formatting, strict all-target Clippy, full portable tests including ignored portable regressions/benchmarks, explicit exclusion of unavailable private fixtures, optimized build.
- Native input on an owned dynamically allocated display, using generated documents only, with bounded readiness and complete subprocess cleanup.
- Source bytes unchanged before Save; independently reopened owned metadata and appearances after Save; preserved foreign annotations, content/navigation, exact previous-byte backups and no-clobber Save As.
- Mixed-tool Undo/Redo and finishing gesture plus same-frame Save; stale-page/document completion rejection, preview failure and dead-renderer guards.
- Crop/rotation and horizontal/vertical/reversed geometry; independent PDF-reader rendering rather than metadata inspection alone.
- No normal-desktop app launch, installation, private drawing use, or publishing as an implicit consequence of feature work.

Representative drawing-set, real Omarchy/Wayland interaction and matched competitor benchmarks remain necessary before production-parity or fastest-editor claims. Synthetic passing checks cannot establish those claims.
