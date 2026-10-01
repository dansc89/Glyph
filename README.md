# Glyph

Glyph is a native Linux PDF editor for drawing sets, inspired by Drawbridge but built as a Linux-first application.

Target feel: clean, fast, dark, keyboard-friendly, at home on Omarchy.

## Architecture

Glyph is **not** a web wrapper and is **not** a direct Swift/AppKit/PDFKit port.

- UI: Rust native immediate-mode shell with `egui`/`wgpu`
- PDF metadata/write path: Rust + `lopdf` first, `qpdf` integration later for hard PDF rewrites
- Rendering path: isolated behind a PDF engine trait so PDFium or MuPDF can be added/swapped without rewriting the app
- Build path: Orange Pi develops/pushes; GitHub Actions produces x86_64 Linux artifacts

## First milestones

1. Native dark shell, opens PDF metadata, page/sidebar model.
2. Real PDF page rendering with PDFium or MuPDF.
3. Smooth zoom/pan and drawing-set navigation.
4. Sheet detection/bookmark generation.
5. Link overlay preview and exported PDF verification.

## Build locally

```bash
cargo test
cargo run
```
