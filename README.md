# Glyph

Glyph is a native Linux PDF editor for drawing sets, inspired by Drawbridge but built as a Linux-first application.

Target feel: clean, fast, dark, keyboard-friendly, at home on Omarchy.

## Architecture

Glyph is **not** a web wrapper and is **not** a direct Swift/AppKit/PDFKit port.

- UI: Rust native immediate-mode shell with `egui`/`wgpu`
- PDF inspection/write path: Rust + `lopdf` first, `qpdf` integration later for hard PDF rewrites
- Rendering path: PDFium via `pdfium-bundled`, embedded at build time so the app does not need a system PDFium install
- Build path: Orange Pi develops/pushes; GitHub Actions produces x86_64 Linux artifacts

## Current visual MVP

- Open a PDF by file picker, pasted path, drag-and-drop, or CLI argument (`glyph file.pdf`)
- Render the selected page with PDFium
- PDF outline/bookmark sidebar with one-click page jumps
- Page list/sidebar with previous/next navigation
- Drag to pan
- Scroll or +/- to zoom, Fit Page, Reset
- Keyboard shortcuts: Ctrl+O, Arrow/Page keys, Home/End
- Dark native shell

## Install on Omarchy / Arch Linux

Easiest path today:

```bash
curl -fsSL https://github.com/dansc89/Glyph/releases/download/1.0.1/install-glyph-omarchy.sh | sh
glyph
```

The installer creates a normal desktop launcher and a detached `glyph` wrapper. Launching from Terminal now returns control immediately; use `GLYPH_FOREGROUND=1 glyph` only when you deliberately want foreground logs.

AUR-style package recipes are included under `packaging/arch/`:

- `packaging/arch/glyph-pdf-bin/` — binary release recipe intended for AUR publishing as `glyph-pdf-bin`.
- `packaging/arch/PKGBUILD` — source checkout recipe for a future `glyph-pdf-git`/source package.

The plain `glyph` and `glyph-bin` AUR names are already taken by an unrelated ASCII-art project, so this app uses the clearer `glyph-pdf-*` package naming path.

Manual path: download `glyph-omarchy-x86_64.tar.gz` from the release, then run:

```bash
tar -xzf glyph-omarchy-x86_64.tar.gz
cd glyph-omarchy-x86_64
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

That extracts the AppImage payload into your user profile and installs a normal detached `glyph` launcher, so runtime launch does not depend on FUSE or keeping a Terminal window open. See [`docs/omarchy-install.md`](docs/omarchy-install.md) for the release packaging guarantee.

An Arch-friendly source package recipe lives at `packaging/arch/PKGBUILD` for later AUR packaging.

## Drawbridge parity

The living parity checklist is in [`docs/drawbridge-parity.md`](docs/drawbridge-parity.md).

## Releases

Public releases now use the simple Drawbridge-polish train: `1.0`, `1.1`, `1.2`, `1.3`, ... even for small incremental updates. See [`docs/release-policy.md`](docs/release-policy.md).

## Build locally

```bash
cargo test --locked
cargo run --locked
cargo run --locked -- /path/to/file.pdf
```
