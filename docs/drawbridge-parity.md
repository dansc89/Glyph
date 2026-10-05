# Drawbridge feature parity

Reference: `dansc89/Drawbridge`, current `README.md`, `USER_MANUAL.md`, and supported commands in `Sources/Drawbridge/`.

Glyph remains a native Rust/egui desktop application. Current Drawbridge intentionally excludes markup creation, measurement, snapshots, combining pages and page conversion; legacy Swift files are not evidence of supported features.

## Current local increment

- Full-document case-insensitive literal text search (`Ctrl+F`), results, previous/next hit, highlighted matches, page progress, cancellation (button/Escape), and stale-result protection.
- Search is limited to selectable text and 1,000 results; scanned pages require a future OCR workflow.
- Internal PDF link navigation, optional link highlights, and browser-style page history (`Alt+Left`/`Alt+Right`, Back/Forward buttons).
- Link extraction handles direct destinations, GoTo actions, indirect annotation structures and named destinations. External actions are not executed.
- Overlay transforms use PDFium so PDF crop boxes and page rotation match the displayed page.
- Automatic sheet-ID detection from selectable text, exact cross-sheet linking, duplicate-target rejection, idempotent generated-link insertion, and collision-safe exports. Background execution reports successful writes, single-sheet/no-reference cases and OCR requirements. This is heuristic detection, not the title-block-zone/review/OCR workflow listed below.
- Disabled the old destructive Flatten button: its backend deleted all annotations, including links, without preserving their appearance. It is NOT appearance-preserving flattening. The legacy removal method remains internal and is not reachable from the UI.

## Automation build verification

The resumed local build passes 90 ordinary tests, plus both manually invoked SALTAIR regressions (detects `L1.11` on the rotated sheet and exports a bookmark with byte-identical rendered pixels at 512-pixel width). All three synthetic performance benchmarks also pass. Tests reopen exported PDFs to verify bookmark names, link targets and native rectangles; repeated linking adds no duplicate generated annotations; indirect annotation arrays retain custom/internal and external links. Automation completion is tested without a native window, including stale document generations and pending document loads. Formatting, Clippy with warnings denied, optimized release compilation and diff checks pass. The desktop app was deliberately not launched; click-through GUI verification remains outstanding. This is a local development build, not a published release.

## Native Omarchy/viewer increment

- Glyph reads the installed Omarchy palette from `$XDG_STATE_HOME/omarchy/current/theme/colors.toml` (default `~/.local/state/omarchy/current/theme/colors.toml`). The flat schema accepts `mode`, named `cyan`/`green`, and older terminal color aliases. Custom surfaces, controls, text, highlights and egui visuals use the palette, including light themes. Missing/malformed/oversized themes fall back safely.
- Theme polling is throttled to one second, with no style replacement when unchanged and no polling while minimized. No desktop theme, hook or system configuration is modified.
- Wayland application ID is `glyph`, matching the desktop launcher/icon name. The app remains Rust/egui/PDFium, not a browser wrapper.
- Fit width (`Ctrl+2`) fills the available horizontal space and top-aligns tall drawings. Fit page is `Ctrl+1`, reset is `Ctrl+0`. Modified view commands work with search focus; fitting is disabled during document inspection to avoid acting on the old document. The later viewer-navigation increment below adds persistent fit modes.
- Verification: 113 tests passed including all ignored real-document checks and benchmarks; formatting, Clippy with warnings denied and release compilation passed. The optimized binary passed 16 isolated viewer checks plus 5 palette checks (installed Miasma theme, private dark theme, live light switch, malformed fallback and recovery). Screenshots were reviewed. Private display processes are terminated by both QA runners. This is not a published release and does not establish complete Drawbridge parity.

## Viewer navigation and wheel zoom increment

- Fit Page and Fit Width now follow viewport resizing and sheet changes. Manual zoom, pan and Reset leave fit mode; unchanged fitted views do not invalidate rendering every frame.
- Back/Forward records page, zoom, pan and fit mode in bounded history. Same-page selections retain forward history; history commands do not mutate state while inspecting a replacement document.
- Direct one-based page entry is available in the viewer toolbar. `Ctrl+G` focuses/selects the current number; Enter navigates, with range validation and normal text-editor arrow behavior.
- Fixed a frame-rate-dependent wheel bug: applying fixed zoom factors to every nonzero smoothing frame magnified one input repeatedly. Zoom now integrates scroll distance while retaining pointer anchoring. A replay of one 60-point event starting at 25% previously ended at 62.95% at 60 FPS and 146.79% at 120 FPS; it now ends at 28.1874% at both rates. These are deterministic input-replay measurements, not native GPU latency measurements.
- Full verification: 129 tests passed including ignored checks/benchmarks, Clippy with warnings denied, formatting and release compilation. Private-display viewer QA passed 26 checks, including resize-aware fitting, direct page entry, and exact restored-view comparison (zero changed pixels); palette QA passed five checks. Stable viewport capture avoids comparing frames while smooth zoom is still settling. Independent review approved the navigation increment.
- This remains a local development build. No desktop launch, system theme change, installation, publication, or ARM64 verification is implied. The thumbnail/text increment below closes two further viewer gaps; full Drawbridge parity is not claimed.

## Thumbnail and selectable-text increment

- The Pages tab displays clickable, virtualized native thumbnails. Only visible rows request work; a replacement scroll scope discards obsolete jobs/results. Foreground page/tile rendering takes priority. Visible rows settle in batches of at most 12 thumbnail requests; at most 64 textures are retained, each no larger than 128×160 pixels. Low-resolution thumbnail rendering shares the retained PDFium session and is downsampled on its owning worker.
- Primary drag starting on PDF text selects characters in PDFium reading order; `Ctrl+C` copies the actual extracted Unicode text, preserving embedded whitespace/newlines. Empty-space drag and middle-button drag still pan; internal link clicks take precedence over selecting their text. Highlight geometry shares the crop/rotation/zoom/pan transform used by the page. Textless/scanned sheets do not gain OCR.
- Only the current page's owned text is retained. Extraction runs on the existing renderer worker, is coalesced, and rejects pages above 200,000 characters instead of silently truncating. Generation/path/page/request guards prevent stale text replacing the current selection. Selection clears on navigation/open; a failed replacement load restores the kept document's rendering and extraction.
- Private-display release QA exercised thumbnail rendering/click navigation and copied `CENTERLINE` from the PDF into the native clipboard, alongside existing navigation, history, fitting, link, search and pan checks. Batched press/drag/release events in one frame have a dedicated regression. Synthetic fixtures do not establish production-corpus parity or native Wayland/GPU latency.
- Final verification: all 158 tests passed, including the ignored real-document regressions and benchmarks. Formatting, Clippy with warnings denied, optimized compilation and diff checks passed. The exact release binary passed 30 isolated viewer checks and five palette checks. Independent re-review approved fixes for tall-sidebar batch starvation and a batched text drag ending outside the canvas. QA processes were terminated.
- This is a local development build; no installation, publication or launch on the user's desktop.

## Remaining parity requirements (not claimed implemented)

1. Safe document editing/persistence: in-memory bookmark/link edits, dirty state, Save/Save As, atomic staged writes, reopen verification, close/cancel semantics, undo and document generation IDs.
2. Real sheet naming: user-selected title-block number/title zones, selectable-text extraction and OCR fallback, preview/review, duplicate detection, embedded PDF page labels, and bookmark hierarchy.
3. Bookmark management: rename/delete entries and groups, optional corresponding page-label updates, undo, named-destination navigation and collapse state.
4. Reliable batch sheet-reference linking: actual sheet identifiers (not generated `Page N` labels), zone exclusions, OCR/visual reference recovery, duplicate-target rejection, preview/cancel/rollback and idempotent replacement of generated links.
5. Viewer polish: recent files/session persistence and performance preferences. Thumbnail navigation and drag-select/copy are implemented locally. Viewport history and persistent fit modes are implemented locally.
6. Appearance-preserving reversible flattening: retain links/forms/redactions and unsupported annotations; embed recovery; verify geometry/content; protect signatures/encryption and modified content before unflatten.
7. Verified lossless reduction: qpdf integration, decoded-stream equivalence, preserved navigation/forms/recovery, only replace if smaller.

## Verification policy

Every increment requires test-first coverage, all Rust tests, formatting, static checks, an actual GUI run and screenshot review. Do not publish a release or claim full parity until the listed workflows are exercised with real architectural PDF sets, including rotated/cropped and scanned pages. Generated QA PDFs establish regression behavior but do not establish production-corpus parity.
