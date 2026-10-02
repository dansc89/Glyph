#!/usr/bin/env sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
SOURCE_BIN="${GLYPH_SOURCE_BIN:-$SCRIPT_DIR/glyph}"
DEST_DIR="${GLYPH_INSTALL_DIR:-$HOME/.local/bin}"
LIB_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/glyph"
APP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/scalable/apps"
REAL_BIN="$LIB_DIR/glyph-bin"
WRAPPER="$DEST_DIR/glyph"

if [ ! -f "$SOURCE_BIN" ]; then
  echo "Glyph binary not found: $SOURCE_BIN" >&2
  echo "Run this installer from the extracted Glyph tarball directory." >&2
  exit 1
fi

mkdir -p "$DEST_DIR" "$LIB_DIR" "$APP_DIR" "$ICON_DIR"
install -m 755 "$SOURCE_BIN" "$REAL_BIN"
install -m 644 "$SCRIPT_DIR/glyph.svg" "$ICON_DIR/glyph.svg"

cat > "$WRAPPER" <<EOF
#!/usr/bin/env sh
set -eu
REAL_BIN="$REAL_BIN"

if [ "\${GLYPH_FOREGROUND:-0}" = "1" ]; then
  exec "\$REAL_BIN" "\$@"
fi

if command -v setsid >/dev/null 2>&1; then
  setsid "\$REAL_BIN" "\$@" >/dev/null 2>&1 &
else
  nohup "\$REAL_BIN" "\$@" >/dev/null 2>&1 &
fi
EOF
chmod +x "$WRAPPER"

cat > "$APP_DIR/glyph.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Glyph
Comment=Native Linux PDF drawing-set editor
Exec=$WRAPPER %f
Icon=glyph
Terminal=false
Categories=Graphics;Viewer;
MimeType=application/pdf;
DESKTOP

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true
fi

echo "Glyph installed to $WRAPPER"
echo "Desktop launcher installed to $APP_DIR/glyph.desktop"
echo "Run: glyph"
echo "Terminal launches detach automatically. For foreground logs: GLYPH_FOREGROUND=1 glyph"