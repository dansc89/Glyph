#!/usr/bin/env sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
SOURCE_BIN="${GLYPH_SOURCE_BIN:-$SCRIPT_DIR/glyph}"
DEST_DIR="${GLYPH_INSTALL_DIR:-$HOME/.local/bin}"
APP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/scalable/apps"

if [ ! -f "$SOURCE_BIN" ]; then
  echo "Glyph binary not found: $SOURCE_BIN" >&2
  echo "Run this installer from the extracted Glyph tarball directory." >&2
  exit 1
fi

mkdir -p "$DEST_DIR" "$APP_DIR" "$ICON_DIR"
install -m 755 "$SOURCE_BIN" "$DEST_DIR/glyph"
install -m 644 "$SCRIPT_DIR/glyph.svg" "$ICON_DIR/glyph.svg"

cat > "$APP_DIR/glyph.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Glyph
Comment=Native Linux PDF drawing-set editor
Exec=$DEST_DIR/glyph %f
Icon=glyph
Terminal=false
Categories=Graphics;Viewer;
MimeType=application/pdf;
DESKTOP

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true
fi

echo "Glyph installed to $DEST_DIR/glyph"
echo "Desktop launcher installed to $APP_DIR/glyph.desktop"
echo "Run: glyph"