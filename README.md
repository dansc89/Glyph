# Glyph

Glyph is a PDF editor for Arch Linux.

Target feel: clean, fast, dark, keyboard-friendly, and native.

## Architecture

Glyph is **not** a web wrapper. It is a native Rust desktop app.

- UI: Rust native immediate-mode shell with `egui`/`wgpu`
- PDF inspection/write path: Rust + `lopdf` first, `qpdf` integration later for hard PDF rewrites
- Rendering path: PDFium via `pdfium-bundled`, embedded at build time so the app does not need a system PDFium install
- Development: native Rust builds/tests on an x86_64 Linux desktop; GitHub Actions produces Linux release artifacts

## Current visual MVP

- Load a PDF by file picker, pasted path, drag-and-drop, or CLI argument (`glyph file.pdf`)
- Render the selected page with PDFium
- PDF outline/bookmark sidebar with one-click page jumps
- Page list/sidebar with previous/next navigation
- Drag to pan
- Scroll or +/- to zoom, Fit Page (`Ctrl+1`), Fit Width (`Ctrl+2`), Reset (`Ctrl+0`)
- Keyboard shortcuts: Ctrl+O, Arrow/Page keys, Home/End
- Dark native shell

## Install on Arch Linux

Easiest path today:

```bash
curl -fsSL https://github.com/dansc89/Glyph/releases/download/1.2/install-glyph-arch.sh | sh
glyph
```

The installer creates a normal desktop launcher and a detached `glyph` wrapper. Launching from Terminal now returns control immediately; use `GLYPH_FOREGROUND=1 glyph` only when you deliberately want foreground logs.

AUR-style package recipes are included under `packaging/arch/`:

- `packaging/arch/glyph-pdf-bin/` — binary release recipe intended for AUR publishing as `glyph-pdf-bin`.
- `packaging/arch/PKGBUILD` — source checkout recipe for a future `glyph-pdf-git`/source package.

The plain `glyph` and `glyph-bin` AUR names are already taken by an unrelated ASCII-art project, so this app uses the clearer `glyph-pdf-*` package naming path.

Manual path: download `glyph-arch-x86_64.tar.gz` from the release, then run:

```bash
tar -xzf glyph-arch-x86_64.tar.gz
cd glyph-arch-x86_64
./install-glyph.sh
glyph
```

The AppImage is still published as an alternate portable artifact:

```bash
chmod +x Glyph-x86_64.AppImage
./Glyph-x86_64.AppImage
```

From a source checkout, the tarball installer source lives at `packaging/linux/install-tar.sh`.

If you only have the AppImage and your system does not have AppImage/FUSE support, use the included no-FUSE installer instead:

```bash
./install-glyph-appimage.sh ./Glyph-x86_64.AppImage
glyph
```

That extracts the AppImage payload into your user profile and installs a normal detached `glyph` launcher, so runtime launch does not depend on FUSE or keeping a Terminal window open. See [`docs/arch-linux-install.md`](docs/arch-linux-install.md) for the release packaging guarantee.

An Arch-friendly source package recipe lives at `packaging/arch/PKGBUILD` for later AUR packaging.

## Local development features

The current development branch adds full-document selectable-text search (Ctrl+F), highlighted results, internal hyperlink navigation, optional link highlights, and Back/Forward history (Alt+Left/Alt+Right). Search runs in the background and can be cancelled; OCR is not yet implemented. The Pages tab now has clickable, virtualized thumbnails. Drag over PDF text and use Ctrl+C to copy it; drag empty space or use the middle button to pan. Text extraction and preview rendering share the background renderer and use bounded resources.

Auto bookmarks now detects sheet identifiers from selectable title-block text, rather than PDF page numbers. Hyperlinks matches exact cross-sheet references and skips self-links and ambiguous destinations. Both actions run in the background, show completion/no-op/error feedback, and save uniquely named copies without overwriting the source or previous exports. Scanned PDFs require OCR, which is not implemented. Generated links are idempotent and existing annotations are retained. Auto bookmarks replaces the saved copy’s existing bookmark hierarchy; the source hierarchy stays untouched. Sheet naming is heuristic: equally supported IDs stay undetected, and fallback page names never become hyperlink targets.

Flatten is temporarily disabled: the former implementation removed annotations rather than preserving their appearance. Do not use it as a flattening solution.

The supported Drawbridge parity target and outstanding gaps are tracked in [`docs/drawbridge-parity.md`](docs/drawbridge-parity.md). These changes are not a published release.

## Omarchy integration

The local build follows Omarchy's active colors (including light themes) and refreshes when the theme changes. It reads the staged palette under `$XDG_STATE_HOME/omarchy/current/theme/colors.toml`, defaulting to `~/.local/state/omarchy/current/theme/colors.toml`; it does not change desktop settings. The Wayland app ID matches `glyph.desktop`. Fit Width (`Ctrl+2`) top-aligns tall sheets; Fit Page (`Ctrl+1`) and Reset (`Ctrl+0`) work even while the search field has focus. Fitting is unavailable while inspecting a newly opened document. Fit modes now follow resizing and sheet changes until you zoom, pan or reset. Back/Forward restores the viewport as well as the page. Use `Ctrl+G` to enter a page number directly. Wheel zoom integrates input distance rather than smoothing-frame count.

## Performance

The local viewer refactor uses a retained PDFium session, bounded render scheduling, shared raster/texture caches, virtualized sidebars, background document inspection, and preserved zoom/pan when navigating sheets. Benchmarks, verification commands and limitations are in [`docs/performance.md`](docs/performance.md). Use the optimized `target/release/glyph` build for normal use, not the debug build.

## Product roadmap

The living usability and feature roadmap is in [`docs/product-roadmap.md`](docs/product-roadmap.md).

## Releases

Public releases use a simple numbered train: `1.0`, `1.1`, `1.2`, `1.3`, ... even for small incremental updates. See [`docs/release-policy.md`](docs/release-policy.md).

## Build locally

```bash
cargo test --locked
cargo run --locked
cargo run --locked -- /path/to/file.pdf
```
