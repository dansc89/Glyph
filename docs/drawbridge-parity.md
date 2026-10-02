# Drawbridge → Glyph parity checklist

Glyph is a Linux/Rust sister app, not a direct AppKit/PDFKit port. This checklist maps the Mac Drawbridge feature surface into native Glyph work.

## Implemented in Glyph

- Native dark PDF workspace shell.
- Load PDF by file picker, pasted path, CLI argument, desktop file `%f`, and drag-and-drop.
- PDFium page rendering with real-PDF render smoke test.
- Page sidebar, previous/next navigation, Home/End, arrow/page-key navigation.
- Zoom, reset, fit-page, drag panning.
- Omarchy-friendly AppImage/tar.gz path plus `.deb` and Arch `PKGBUILD` starter.
- PDF outline/bookmark extraction and sidebar jump targets.

## Next parity slices

### 1. Drawing-set navigation

- Better page labels from PDF page-label dictionaries.
- Sheet ID extraction/normalization from page text and titles.
- Searchable sheet list with thumbnails.
- Recent files and last-viewed page/zoom state.

### 2. Search

- Full-document text search with hit list.
- Per-page highlighted search hits.
- Search history and keyboard flow.

### 3. Links / AEC workflow

- Link overlay rendering on pages.
- Detect internal PDF links and jump destinations.
- Drawbridge-style sheet-to-sheet linking workflow.
- Create/edit/remove link rectangles.
- Export/save updated link annotations.

### 4. Bookmarks / outlines

- Preserve hierarchy and collapsed state.
- Named destinations and action destinations, not just direct `/Dest` arrays.
- Bookmark creation/editing/removal.
- Export/save outline changes.

### 5. PDF processing/export

- Combine PDFs/drawing sets.
- Flatten annotations.
- Optimized/mobile PDF export.
- Image/JPEG-style export flow.
- Verify exported PDFs by reopening and inspecting structure.

### 6. Markup/editor tools

- Select/pan modes.
- Rect/ellipse/line/arrow/freehand/text tools.
- Text note editing.
- Undo/redo stack.
- Save/export annotations safely.

## Quality bar

- Visual-first. No fake CLI-only milestones.
- Numbered release train only from here forward: `1.0`, `1.1`, `1.2`, `1.3`, ...
- Small polish updates are still real releases; do not wait for large batches if the app is better.
- Local tests for core PDF/document logic.
- CI-built x86_64 AppImage, `.deb`, and tar.gz before telling user to install.
- Release assets downloaded back and SHA256 verified before claiming release readiness.
- Actual Omarchy visual testing remains required on an x86_64 desktop.
