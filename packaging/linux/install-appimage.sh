#!/usr/bin/env sh
set -eu

SOURCE="${1:-Glyph-x86_64.AppImage}"
APP_NAME="glyph"
DEST_DIR="${GLYPH_INSTALL_DIR:-$HOME/.local/bin}"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/glyph"
APP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/scalable/apps"
INSTALL_ROOT="$DATA_DIR/appimage-extracted"
WRAPPER="$DEST_DIR/$APP_NAME"

if [ ! -f "$SOURCE" ]; then
  echo "Glyph AppImage not found: $SOURCE" >&2
  echo "Usage: $0 /path/to/Glyph-x86_64.AppImage" >&2
  exit 1
fi

case "$SOURCE" in
  /*) SOURCE_ABS="$SOURCE" ;;
  *) SOURCE_ABS="$(CDPATH= cd -- "$(dirname -- "$SOURCE")" && pwd)/$(basename -- "$SOURCE")" ;;
esac

mkdir -p "$DEST_DIR" "$APP_DIR" "$ICON_DIR" "$DATA_DIR"

TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/glyph-appimage.XXXXXX")
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT INT TERM

chmod +x "$SOURCE_ABS"
(
  cd "$TMP_DIR"
  "$SOURCE_ABS" --appimage-extract >/dev/null
)

rm -rf "$INSTALL_ROOT"
mkdir -p "$INSTALL_ROOT"
cp -R "$TMP_DIR/squashfs-root/." "$INSTALL_ROOT/"

cat > "$WRAPPER" <<EOF
#!/usr/bin/env sh
exec "$INSTALL_ROOT/AppRun" "\$@"
EOF
chmod +x "$WRAPPER"

if [ -f "$(dirname "$0")/glyph.svg" ]; then
  install -m 644 "$(dirname "$0")/glyph.svg" "$ICON_DIR/glyph.svg"
elif [ -f "$INSTALL_ROOT/glyph.svg" ]; then
  install -m 644 "$INSTALL_ROOT/glyph.svg" "$ICON_DIR/glyph.svg"
fi

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

echo "Glyph AppImage extracted to $INSTALL_ROOT"
echo "No FUSE package is required; the launcher runs the extracted AppImage payload."
echo "Glyph installed to $WRAPPER"
echo "Desktop launcher installed to $APP_DIR/glyph.desktop"
