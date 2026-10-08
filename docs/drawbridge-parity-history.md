# Historical Drawbridge parity and verification log

This is an archived development record. Version references, test counts, installed-build observations and “unpublished” statements describe their original increments, not the current release. See [current status](drawbridge-parity.md).

# Drawbridge feature parity

Current reference: [`dansc89/Drawbridge` tag `v4.8`](https://github.com/dansc89/Drawbridge/releases/tag/v4.8), pinned source commit `424c1b8a07ec634d56ffc23d1f0f306951d38075`. See [the v4.8 parity/stability audit](drawbridge-v4.8-baseline.md). The earlier [v4.6 baseline](drawbridge-v4.6-baseline.md) and per-increment results below are historical; their test counts are not the current total.

Glyph remains a native Rust/egui desktop application. Drawbridge v4.8 enables a separate eight-tool markup controller (select, rectangle, ellipse, line, arrow, text, polyline, polygon). The disabled legacy `ToolMode` is not its current authoring surface. Glyph now has native rectangle and ellipse annotation increments with shared persistence/history; the complete markup milestone is not implemented. See [ellipse and save reliability verification](ellipse-save-reliability-review.md). Measurement, snapshot paste, page assembly/conversion and other legacy tools are not presumed supported merely because old Swift files exist.

## Current status against v4.8

**Not full feature parity; equal production stability/performance is unproven.** Current local development source passes 285 Rust tests; previously recorded exact-build native reports contain 86 passing assertions. Installed Glyph is behind this build. Rectangle/ellipse saving is implemented, but line/arrow/text/polyline/polygon, manipulation/styles, geometry-aware ellipse selection and broader document workflows remain gaps. See the current audit for source-backed details and validation limits.

## Earlier search/automation increment

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

## Safe bookmark editing increment (local development build)

- Native Document menu and bookmark context-menu Rename, Ctrl+Z/Ctrl+Shift+Z undo/redo, Ctrl+S Save and Ctrl+Shift+S Save As. A dirty marker and Save/Discard/Cancel decisions protect open, close and quit transitions. Editing and serialization run off the UI thread; conflicting automation is blocked.
- Rename changes only an existing outline object's Title. Stable outline object IDs prevent selecting the wrong entry when inspection filters an indirect/empty title or titles are duplicated. Unicode is saved as UTF-16BE; destinations, hierarchy and existing page navigation are retained. That title-only increment did not implement adding/deleting/reparenting bookmarks or page labels. The subsequent page-label increment below adds separate embedded-label editing.
- Save stages and reopens the PDF, verifies objects and outlines, checks source fingerprints and uses Linux atomic exchange. Every overwrite permanently retains the displaced original at a unique sibling `.glyph-backup-*.pdf` path. This is preservation, not a lock on other writers: a detected commit conflict retains both files, leaves the edit checkpoint dirty, and reports the backup path; the edited PDF may already be visible at the source path. Backups are not automatically deleted. Unsupported atomic-exchange filesystems fail closed.
- Save As creates a new destination without clobbering an existing file. Encrypted/signature-bearing documents are rejected rather than silently stripping security. Undo history is bounded to 64 operations and persists across successful saves in the running session, not across app restarts.
- Verification: 201 Rust tests including ignored real-document checks/benchmarks; warnings-denied Clippy, formatting and optimized build passed. Eight X11-isolation regressions and nine real native editing checks passed on an owned dynamically allocated Xvfb display. Save/reopen preserved the generated drawing's rendered pixels exactly. Native Save As picker dispatch and actual save-as PDF I/O have automated coverage; the OS portal itself was not click-tested. Production architectural-set editing remains to be validated.
- Test the local binary with `/home/daniel/Glyph/target/release/glyph`; begin with a copy of a PDF. GitHub 1.2 is the separately verified viewer release and does not include this editing increment. No user-desktop launch or application installation was performed.

## Embedded page-label increment (v4.6 baseline, local development build)

- Document → Rename current page label edits the actual PDF `/PageLabels` metadata, independently of bookmark titles. Numeric defaults and existing Roman/alphabetic/prefixed labels are read in physical page order. A Unicode label is limited to 256 characters; other pages retain their effective labels.
- Label edits participate in the same bounded Undo/Redo history and save checkpoints as bookmark edits. Save/Save As verifies persisted labels and unchanged unrelated PDF objects. Discard restores the saved metadata snapshot. Optional bookmark/label synchronization and automatic reviewed sheet naming remain unimplemented.
- Malformed optional labels do not prevent read-only viewing: numeric labels are displayed instead. Editing stays strict and fails closed rather than overwriting an invalid original label tree.
- Native-close modal ownership and bounded pending-edit polling have regressions. An actual PDF-worker test opens the last page of an 11-page PDF, then a one-page replacement, verifies new-source pixels and clears old navigation history.
- Final verification: **234 Rust tests passed**, including ignored real-document regressions/benchmarks; warnings-denied Clippy, formatting, release compilation and diff checks passed. **Nine native page-label checks**, **nine existing bookmark/save checks**, and **eight display-isolation checks** passed. Generated-fixture label save/reopen retained the embedded label, exact displayed drawing pixels and the displaced original backup.
- Independent read-only source review passed with no blocking security/logic findings. It did not execute GUI/tests. Its nonblocking follow-up is an app-level mixed title/label save → later edits → discard → failed-open checkpoint regression; backend mixed history/save tests already exist.
- Tagged v4.6 has a new enabled markup toolbar despite stale manual/legacy ToolMode wording. See [the corrected v4.6 baseline](drawbridge-v4.6-baseline.md). Full parity and representative interaction/save speed superiority remain unproven. Public 1.2 is still viewer-only; this increment is local and unpublished.

## First rectangle markup increment (local development)

- Rectangle (`R`), select markup (`V`) and Delete/Backspace operate on native Glyph-owned PDF Square annotations with explicit unfilled red normal appearances. Undo/Redo, Save/Save As and save checkpoints share the bookmark/label edit session. Imported non-Glyph annotations remain preserved and are not made destructively editable.
- Preview serialization runs off the UI thread and replaces the retained renderer's PDF document from memory, without writing the source. Inherited crop/rotation geometry has native rendering regressions at all four orthogonal rotations; saved annotations remain visible and owned after reopen.
- The native interaction pass reproduced and fixed an egui input-context deadlock and Escape stealing menu dismissal. Independent app review required cross-page selection and preview-failure corrections; independent re-review found no remaining blockers in those paths. Final source passed 268 Rust tests, strict Clippy and formatting; its optimized binary passed 75 native assertions (including 16 markup checks) and nine display-isolation tests. See [the increment review](rectangle-markup-review.md) for evidence and limits.
- This is not the full eight-tool v4.6 milestone. At that milestone ellipse/line/arrow/text/polyline/polygon, move/resize/vertex/text editing and style controls remained unimplemented. The later ellipse increment implements ellipse creation/persistence; the other tools and manipulation/style gaps remain. Snapshot output is capped at 256 MiB, not total process RAM; detached history objects can grow the file. Production-set and external-reader interoperability remain unverified.
- Public GitHub 1.2 remains viewer-only. This rectangle increment has not been installed or published.

## Remaining parity requirements (not claimed implemented)

1. Extend the title-only editing/persistence foundation above to link edits and broader drawing-set changes, including their undo and conflict handling.
2. Real sheet naming: user-selected title-block number/title zones, selectable-text extraction and OCR fallback, preview/review, duplicate detection, and bookmark hierarchy. Embedded labels can now be edited separately; generated/reviewed bulk naming is not implemented.
3. Enabled v4.8 bookmark parity: subtree deletion with undo and optional corresponding page-label updates. Existing-title rename/undo is implemented locally above. Add/reparent/reorder are possible future Glyph enhancements, not verified enabled v4.8 requirements.
4. Reliable batch sheet-reference linking: actual sheet identifiers (not generated `Page N` labels), zone exclusions, OCR/visual reference recovery, duplicate-target rejection, preview/cancel/rollback and idempotent replacement of generated links.
5. Viewer polish: recent files/session persistence and performance preferences. Thumbnail navigation and drag-select/copy are implemented locally. Viewport history and persistent fit modes are implemented locally.
6. Appearance-preserving reversible flattening: retain links/forms/redactions and unsupported annotations; embed recovery; verify geometry/content; protect signatures/encryption and modified content before unflatten.
7. Verified lossless reduction: qpdf integration, decoded-stream equivalence, preserved navigation/forms/recovery, only replace if smaller.
8. v4.8 markup parity: owned editable rectangle/ellipse/line/arrow/text/polyline/polygon annotations, selection/resize/vertices/text editing, style controls, shared undo, crop/rotation geometry and verified object-preserving saves. Imported consultant annotations must not become destructively editable.

## Verification policy

Every increment requires test-first coverage, all Rust tests, formatting, static checks, an actual GUI run and screenshot review. Do not publish a release or claim full parity until the listed workflows are exercised with real architectural PDF sets, including rotated/cropped and scanned pages. Generated QA PDFs establish regression behavior but do not establish production-corpus parity.
