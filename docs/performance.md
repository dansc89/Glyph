# Viewer performance refactor

## Scope

Native Rust/egui/PDFium viewer, local development branch. No app is opened on the user's desktop by the QA runner; it uses a private Xvfb display and terminates its processes. No release publication or installation is part of this work.

## Measured findings and changes

Measurements below are real local **debug/test-profile microbenchmarks** using generated fixtures. They are not a comparison with Bluebeam or Drawbridge, and do not establish performance on complex production drawings. GPU uploads are not included in the CPU cache/sidebar tests.

| Test | Before | After | Scope |
|---|---:|---:|---|
| 200 cached 1800×2400 raster lookups | 177.426 ms | 0.039223 ms | `benchmark_cache_lookup`; eliminated Vec bitmap copying with shared immutable Arc ownership |
| 20 UI frames with a 10,000-sheet sidebar | 3665.934 ms | 27.676 ms | `benchmark_large_document_sidebar`; virtualized rows and removed whole-document label cloning |
| 30 repeated 512-pixel page renders | 4667.463 ms stateless | 2.313 ms retained session | `benchmark_30_cached_session_vs_stateless`; synthetic PDF, includes repeated stateless binding/load overhead; session loaded document once |

Before values were captured before the corresponding refactor; after values above are from the final verification run. The stateless-versus-session benchmark runs both strategies in that final run. These timings are individual observed runs, not statistical distributions. They should not be described as a whole-app speedup. Repeat benchmarks in a quiet environment, with both before/after in the same profile and with representative files, before making production performance claims.

## Architecture

- One renderer thread owns the PDFium binding and a retained document session. Generation changes reload even when a file path is unchanged.
- Latest-request-wins foreground/tile/link scheduling; selected-page work precedes background prefetch. Bounded rendered-result queue prevents an unbounded bitmap backlog. PDFium's thread-safe binding serializes native calls anyway; spawning many raster threads was not parallel rendering.
- One coalescing link slot shares the worker. Its parsed link index and PDFium transform document are retained by generation/path; normalized page links have a separate seven-page cache. Superseded results are rejected before they can overwrite current hotspots.
- Shared raster ownership and retained texture handles mean warm page navigation neither clones the full bitmap nor uploads an unchanged texture again.
- Cache is bounded by seven pages and a conservative 128 MiB budget reserving both CPU rasters and GPU copies. Active view/tile, in-flight results, decoded document content and driver memory are additional, so this is **not a total process-RSS limit**. Oversized pages are excluded from the cache.
- Full-page high-resolution rerenders are suppressed at high zoom; only the visible viewport tile is refined after the interaction debounce.
- Page, bookmark and search sidebars construct only visible rows. Page navigation preserves zoom/pan. Initial document opening requests a fitted page rather than a clipped oversized view.
- Document inspection runs off the UI thread; stale load results cannot replace a newer request.

## Regression verification

Final verification: 74 ordinary tests passed; all three manually invoked benchmarks passed; formatting, clippy with warnings denied, release compilation and diff checks passed. Twelve isolated release-GUI assertions passed. The independent re-review approved fixes for pending tile cancellation, same-path stale link results, and unbounded link workers. No GUI or QA server remained running afterward.

Run:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked -- --test-threads=1
cargo test --locked benchmark -- --ignored --nocapture --test-threads=1
cargo build --release --locked
```

Isolated GUI QA requires Xvfb, xdotool, xclip, ffmpeg, tesseract and Pillow:

```sh
python3 scripts/performance-qa.py --binary target/release/glyph --out dist/performance-qa-release
```

The runner verifies initial fitted rendering, fit-width top alignment and fit-page keyboard shortcuts, correct next/previous pages, cached Back/Forward, rapid navigation, internal link navigation, three-page text search, search keyboard focus, and high-zoom tile status. Screenshots support visual review of pan/render geometry. Captured GUI action times include PNG capture and polling and **must not be used as input-to-display latency measurements**.

For the Omarchy palette increment, run `python3 scripts/native-theme-qa.py --binary target/release/glyph --out dist/native-theme-qa`. This uses a second private X display and tests the installed palette, isolated live dark/light switching, malformed fallback and recovery without changing the user's theme. The latest viewer-navigation verification passed 129 tests (including all ignored checks) and 31 isolated GUI checks across both runners. Viewer QA additionally verifies persistent fitting on resize, Ctrl+G page entry, and a zero-difference restored zoom/pan snapshot. Snapshot comparisons wait for stable geometry before navigating so unconsumed smoothed input cannot affect the next view. The Xvfb renderer uses software fallback; this is correctness verification, not native Wayland/GPU latency measurement.

A deterministic wheel-input replay exposed a frame-rate bug: one 60-point scroll starting at 25% zoom ended at 62.95% at 60 FPS and 146.79% at 120 FPS. Distance-integrated zoom now ends at 28.1874% at both rates. Run `cargo test --locked a_single_wheel_event_has_bounded_frame_rate_independent_zoom -- --nocapture` to reproduce the fixed behavior.

## Thumbnail and text-selection resource limits

Thumbnail cards remain virtualized: a visible row scope submits bounded batches of at most 12 jobs, with foreground pages/tiles ahead of text, links, thumbnails and adjacent-page prefetch. Scroll-scope replacement cancels obsolete pending results. Thumbnails share the retained renderer session; final CPU/GPU thumbnail images are bounded to 128×160 pixels, with a 64-texture cache. Rendering first uses a low-width raster and downsampling occurs on the renderer thread; transient raster/document/driver memory remains additional to the cache budget.

Selectable text is extracted once for the current page, on that same worker, and is not cloned per frame. Pages above 200,000 characters return a visible error rather than allocating a partial selection. Character hit testing and highlights still traverse glyphs; representative dense-text drawings need profiling before claiming negligible frame cost. Selection copy allocates only on an explicit clipboard request. Cropped/rotated geometry is tested against rendered ink. The release QA runner additionally exercises clickable previews and verifies the native clipboard contents after a real text drag.

A current test-profile repeat observed 20 frames with a 10,000-sheet thumbnail sidebar at 24.713408 ms, 200 shared raster-cache lookups at 38.541 µs, and 30 retained-session 512-pixel renders at 2.417388 ms versus 4.580458512 s stateless. These are individual synthetic/component measurements, not production speedups or input-to-display latency.

## Remaining work

- Benchmark representative vector-heavy and scanned architectural sets: cold open, page-change p50/p95, zoom/pan frame times, RSS/VRAM, and cancellation behavior.
- Bookmark/hyperlink generation now runs off the UI thread with a busy indicator and persistent completion feedback. Per-page progress and cancellation are still outstanding; native text scans can contend with rendering through the PDFium lock.
- Retain same-scale tiles during small pans; currently view changes invalidate the tile even if its overscan margin could serve the viewport.
- Search still has a separate bounded-result, cancellable scan and normalization stage. Interactive rendering can contend with PDFium search calls; large-document search indexing and grouped transforms deserve separate profiling.
- Feature parity remains tracked in `drawbridge-parity.md`; performance work does not imply complete feature parity or production-grade lossless PDF editing.
