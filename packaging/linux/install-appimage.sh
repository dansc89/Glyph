#!/usr/bin/env sh
set -eu

SOURCE="${1:-Glyph-x86_64.AppImage}"
DEST_DIR="${GLYPH_INSTALL_DIR:-$HOME/.local/bin}"
APP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/scalable/apps"

if [ ! -f "$SOURCE" ]; then
  echo "Glyph AppImage not found: $SOURCE" >&2
  echo "Usage: $0 /path/to/Glyph-x86_64.AppImage" >&2
  exit 1
fi

mkdir -p "$DEST_DIR" "$APP_DIR" "$ICON_DIR"
install -m 755 "$SOURCE" "$DEST_DIR/glyph"
install -m 644 "$(dirname "$0")/glyph.svg" "$ICON_DIR/glyph.svg"
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
