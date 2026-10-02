#!/bin/sh
set -eu

REPO="${GLYPH_REPO:-dansc89/Glyph}"
TAG="${GLYPH_VERSION:-1.0.6}"
ASSET="glyph-omarchy-x86_64.tar.gz"
BASE_URL="https://github.com/${REPO}/releases/download/${TAG}"
INSTALL_DIR="${GLYPH_INSTALL_DIR:-$HOME/.local/bin}"

command_exists() {
  command -v "$1" >/dev/null 2>&1
}

download() {
  url="$1"
  out="$2"
  if command_exists curl; then
    curl -fL --retry 3 --connect-timeout 20 -o "$out" "$url"
  elif command_exists wget; then
    wget -O "$out" "$url"
  else
    echo "Need curl or wget to download Glyph." >&2
    exit 1
  fi
}

need() {
  if ! command_exists "$1"; then
    echo "Need '$1' to install Glyph." >&2
    exit 1
  fi
}

need tar
need mktemp

TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/glyph-install.XXXXXX")
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT INT TERM

cd "$TMP_DIR"
echo "Downloading Glyph ${TAG}..."
download "$BASE_URL/$ASSET" "$ASSET"

if command_exists sha256sum; then
  if download "$BASE_URL/SHA256SUMS" SHA256SUMS; then
    grep "  $ASSET\$" SHA256SUMS > SHA256SUMS.glyph
    sha256sum -c SHA256SUMS.glyph
  fi
fi

echo "Installing Glyph..."
tar -xzf "$ASSET"
cd glyph-omarchy-x86_64
sh ./install-glyph.sh

if command_exists update-desktop-database; then
  update-desktop-database "${XDG_DATA_HOME:-$HOME/.local/share}/applications" >/dev/null 2>&1 || true
fi

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    echo ""
    echo "Note: $INSTALL_DIR is not on PATH in this shell. Open a new terminal or add it to PATH."
    ;;
esac

echo ""
echo "Glyph installed. Run: glyph"
