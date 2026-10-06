# Ellipse selection and synthetic save-stress increment

Local working-tree increment following the Drawbridge v4.8 audit. Not installed, published, or full Drawbridge feature parity. All new save workflows use generated PDFs; no user PDFs were modified, and native instances used owned Xvfb displays rather than launching on the user desktop.

## Changes

- Ellipse selection now tests its curved interior with an approximately 3-screen-point halo, not its whole rectangular bounding box. Empty corners do not select or authorize Delete. Rectangle behavior, topmost matching shape, current-page restriction and layer ownership remain intact.
- Production-path headless tests cover translated/zoomed pages, ellipse misses selecting an underlying rectangle, invalid geometry, center/edge tolerance and page/layer restrictions.
- Native regression actually failed against the preceding optimized binary: corner click + Delete removed the ellipse (`dist/ellipse-selection-red-ready`, zero remaining red pixels). This is the behavior-level RED; the earlier startup failure was a harness race, not that RED.
- Native markup QA now waits for actual generated-page pixels, with bounded timeout and process/display liveness, rather than assuming window visibility implies completed rendering.
- Added `synthetic_100_page_save_reliability_stress` in `src/pdf/edit_session.rs`: generated 100-page PDF, inherited nonzero crop, four rotations, original streams/font resources, foreign annotation/link, bookmarks/page labels, 200 mixed owned shapes, 64-change shared history, seven Saves and one successful Save As. Covers rejection/retry on read-only source and Save As collision without losing dirty state, identity or history; backups and reopen preserve the expected shapes and unrelated original objects.
- Added `scripts/native-save-stress-qa.py`: a separate 100-page native-app workload seeded with 200 genuine saved Glyph annotations, then three new UI-drawn shapes, Save after each, and native reopen. Canonical original geometry is retained; alternate rotations are tested through the real backend API, not by invalidating saved owned-coordinate metadata.

## Verified execution

- Fresh `git diff --check`, formatting, strict all-target Clippy, all Rust tests including ignored, and optimized build: **291 passed, 0 failed, 0 ignored**.
- **96 fresh native assertions passed**: ellipse mixed/retry/imported-real workflow 32; rectangle equivalent 29; immediate-release ellipse saving 5; rectangle saving 4; native 100-page workload 26. The minimum-window/layout usability script separately passed; its scenarios are not included in the 96 assertion count.
- Three native Saves retain exact preceding PDFs as backups; source remains unchanged before explicit Save. Independent pypdf checks preserve all 100 pages' original text, decoded content streams, media/crop/rotation and foreign link, plus all 200 original owned geometries and appearance streams. Final count is 203, and each new shape has visible native pixels after reopen. Additional artifact validation confirms 200 distinct original annotation object IDs and unchanged original IDs/pages/subtypes/rectangles in the final saved PDF.
- Poppler `pdfinfo` confirms 100 pages, unencrypted PDF 1.4, 200,331 bytes. Original/saved Poppler renders of untouched pages 2 and 100 are pixel-identical. Independent page-1 rendering shows all three new shapes and intact original content/seed annotations.
- Synthetic native observed Save completion: 0.36245 s, 0.31600 s, 0.44276 s. Includes scripted keyboard/polling overhead; not input latency, production-corpus or Drawbridge benchmark.
- Focused component test rerun: passed in 2.81 s; component Saves roughly 167–236 ms in the debug test executable. Initial objects 408; final 818. First saved PDF 606,542 bytes, fifth replacement-cycle save 608,925, final export 609,023. These are exact-workload observations, not a bound on an unlimited edit lifetime.
- Read-only independent review found no blocking security/logic issue. Initial review suggested stronger annotation-ID controls and comparisons; geometry/AP and actual reopened-pixel checks were strengthened. Concise final review verdict is recorded in `deleg_0b0c523b`.

## Reproduction and artifacts

```sh
cargo test --locked synthetic_100_page_save_reliability_stress -- --nocapture --test-threads=1
```

Native tools require the cached Xvfb/xdotool/ffmpeg/PDF libraries used by the existing owned-display scripts. The new stress script needs pypdf and Pillow (`uv run --with pypdf --with pillow python scripts/native-save-stress-qa.py ...`). It intentionally uses an owned generated mixed-annotation seed, not a customer PDF.

- `dist/ellipse-selection-verified/result.json`
- `dist/rectangle-selection-verified/result.json`
- `dist/ellipse-selection-release-verified/result.json`
- `dist/rectangle-selection-release-verified/result.json`
- `dist/selection-layout-result.json`
- `dist/native-save-stress-final/result.json`, `original.pdf`, `synthetic-100-page.pdf`
- `dist/native-save-stress-final/04-reopened-proof.png`, `poppler-saved-page1.png`
- `dist/native-save-stress-final/poppler-{original,saved}-page{2,100}.png`

Backend test PDFs live in owned TempDirs and are removed after the test; the native fixture above is the durable artifact.

## Limits and deployment

- Synthetic simple/vector/text fixtures are not large real architectural/civil sets. No claim of comparative production stability, crash-free operation or equivalent Drawbridge latency.
- Native stress preserves the seed geometry; backend coverage supplies inherited crop/four-rotation stress. Native Wayland/portal, process-kill recovery, disk-full and production-corpus save stress remain separate gates.
- History is capped at 64, **not PDF file/object growth**. Delete removes annotation references but may retain orphaned objects; each replacement in this workload added two document objects. Cleanup requires care around undo/redo and immutable-object save protections.
- Missing line/arrow/text/polyline/polygon, move/resize and styles remain missing. This closes one interaction gap and expands save coverage, not overall parity.
- Built executable SHA-256: `28adec538497db7d76c54b22f632d46179dbfc2a79ee95470a3e7e80911a0a43`.
- Installed executable SHA-256: `e6e1e6975b31d7c4446cb1a3d893e274e1da38bb1dd0746ceaf29dbcb29406cd`; unchanged earlier installed build. Nothing committed, installed or launched on the user desktop.
