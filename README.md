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

Use the Omarchy tarball first. It installs from a fresh Omarchy desktop without asking for FUSE or other extra packages.

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

An Arch-friendly source package recipe lives at `packaging/arch/PKGBUILD` for later AUR packaging.

## Drawbridge parity

The living parity checklist is in [`docs/drawbridge-parity.md`](docs/drawbridge-parity.md).

## Build locally

```bash
cargo test --locked
cargo run --locked
cargo run --locked -- /path/to/file.pdf
```
