# Glyph product roadmap

Glyph is a standalone native Linux PDF workspace for drawing sets and technical documents.

## Implemented in Glyph

- Native dark PDF workspace shell.
- Load PDF by file picker, pasted path, CLI argument, desktop file `%f`, and drag-and-drop.
- PDFium page rendering with real-PDF render smoke test.
- Page sidebar, previous/next navigation, Home/End, arrow/page-key navigation.
- Zoom, reset, fit-page, drag panning.
- Omarchy-friendly AppImage/tar.gz path plus `.deb` and Arch `PKGBUILD` starter.
- PDF outline/bookmark extraction and sidebar jump targets.

## Next usability slices

### 1. Viewer performance and feel

- Replace whole-page raster zoom with a tiled or retained-resolution rendering path.
- Keep pan/scroll interaction responsive while high-resolution page tiles render in the background.
- Cache nearby pages and visible zoom levels.
- Add clearer loading/progress states for large sheets.

### 2. Drawing-set navigation

- Better page labels from PDF page-label dictionaries.
- Sheet ID extraction/normalization from page text and titles.
- Searchable sheet list with thumbnails.
- Recent files and last-viewed page/zoom state.

### 3. Search

- Full-document text search with hit list.
- Per-page highlighted search hits.
- Search history and keyboard flow.

### 4. Links / AEC workflow

- Link overlay rendering on pages.
- Detect internal PDF links and jump destinations.
- Sheet-to-sheet linking workflow.
- Create/edit/remove link rectangles.
- Export/save updated link annotations.

### 5. Bookmarks / outlines

- Preserve hierarchy and collapsed state.
- Named destinations and action destinations, not just direct `/Dest` arrays.
- Bookmark creation/editing/removal.
- Export/save outline changes.

### 6. PDF processing/export

- Combine PDFs/drawing sets.
- Flatten annotations.
- Optimized/mobile PDF export.
- Image/JPEG-style export flow.
- Verify exported PDFs by reopening and inspecting structure.

### 7. Markup/editor tools

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
