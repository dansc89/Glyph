# Persistent text development verification

## Verdict and target

The scoped text increment is integrated and verified. **This is not complete Drawbridge v6.0 parity.** The acceptance target is [Drawbridge v6.0](https://github.com/dansc89/Drawbridge/releases/tag/v6.0), pinned source `3396a63a0ef0d680b1479e1bca225da671976577`. Earlier v5.5 plans are historical.

## Implemented

Text (T) places/edits an owned PDF FreeText annotation, with compact physical-point font-size editing, selection, move/corner resize, Delete and shared transactional Undo/Redo/Save/reopen. Explicit Apply or Cancel is required before destructive close/open/drop replacement while a draft owns input. Actual menu Close and actual Save As picker cancellation were exercised on an owned Xvfb. Save As cancellation retains committed dirty text; it never itself overwrites the source or silently discards text. Invalid drafts stay editable. Document history cannot steal the local editor stack. Large multiline editors scroll without hiding controls.

## Scope limitations — parity gaps, not substitutes

Courier printable ASCII plus line breaks only; maximum 10,000 bytes, font size 6–144 physical points. Unsupported encoding and non-fitting text are rejected explicitly, not silently replaced/truncated. Unicode/fallback fonts, Helvetica-equivalent typography, wrapping/clipping behavior and reference interaction parity are not complete. No competitor-superiority or full workflow claim is made.

## Fresh integrated-root acceptance

- Full portable suite: **418 passed, 0 failed**, including ignored portable tests; two `real_saltair` private-fixture tests excluded, not passed.
- Strict all-target Clippy, whole-crate formatting check, whitespace check, optimized build: passed.
- Native text: **32 checks**; existing markup regressions: **19 checks**; sheet navigation: **46 checks**. Actual controls, generated documents, bounded subprocesses, private Xvfb only.
- Independent PDF/Poppler/OCR: strict pass on native-saved document, exact Contents/font/color, visible text, valid appearance/resources, original decoded page streams and foreign annotations preserved; raster pixels identical outside owned annotation bounds. Native geometry uses pixel-derived tolerances; optional 0.01-point expected-Rect checking was not used by the render verifier.
- Independent frozen-source safety review approved with no P0/P1/P2 blocker. Source/backend crop/rotation/UserUnit cases are in portable tests, not a claim that every native transform was tested here.

## Reproducible evidence

Binary: `dist/glyph-text-development`

SHA256: `18d82cef369d26851928b8d1bf4c0637836b39b6e020155600891be052799c3d`

Root evidence manifest: `dist/text-verification.json`. Root logs: `dist/text-root-{full,clippy,fmt,build}.log`; native `dist/text-root-native{,-markups,-sheets}/result.json`; independent PDF `dist/text-root-independent-pdf/result.json`. Isolated review: `/home/daniel/.cache/glyph-v55-parity/dist/text-final-independent-review.md`.

Exact prior source backups: `/home/daniel/Glyph/dist/text-integration-backup-0wb1b0m_`. Integrated source hashes match the reviewed isolated epoch; preexisting root Cargo version/lock preserved. Installed executable unchanged; no normal-desktop launch, release, commit or public documentation push. Native Xvfb is not full Omarchy/Wayland acceptance. Representative matched drawing-set performance comparisons remain required for an at-least-as-good verdict.
