#!/usr/bin/env python3
"""Render the AUR glyph-pdf-bin package files for a Glyph release."""

from __future__ import annotations

import argparse
import re
from pathlib import Path

DEPENDS = [
    "gcc-libs",
    "glibc",
    "fontconfig",
    "mesa",
    "vulkan-icd-loader",
    "wayland",
    "libx11",
    "libxcb",
    "libxcursor",
    "libxi",
    "libxinerama",
    "libxkbcommon",
    "libxkbcommon-x11",
    "libxrandr",
]

PKGDESC = "Native Linux PDF drawing-set editor inspired by Drawbridge"
URL = "https://github.com/dansc89/Glyph"
PKGNAME = "glyph-pdf-bin"
SOURCE_URL = "https://github.com/dansc89/Glyph/releases/download/{version}/glyph-omarchy-x86_64.tar.gz"


def validate_version(version: str) -> str:
    if not re.fullmatch(r"[0-9][0-9A-Za-z._+-]*", version):
        raise SystemExit(f"Invalid release version for AUR pkgver: {version!r}")
    return version


def validate_sha256(value: str) -> str:
    if not re.fullmatch(r"[0-9a-fA-F]{64}", value):
        raise SystemExit(f"Invalid sha256: {value!r}")
    return value.lower()


def render_pkgbuild(version: str, sha256: str) -> str:
    depends = " ".join(f"'{dep}'" for dep in DEPENDS)
    return f"""# Maintainer: dansc89
pkgname={PKGNAME}
pkgver={version}
pkgrel=1
pkgdesc=\"{PKGDESC}\"
arch=('x86_64')
url=\"{URL}\"
license=('MIT')
depends=({depends})
provides=('glyph-pdf')
conflicts=('glyph-pdf')
source=("https://github.com/dansc89/Glyph/releases/download/${{pkgver}}/glyph-omarchy-x86_64.tar.gz")
sha256sums=('{sha256}')

package() {{
  cd \"${{srcdir}}/glyph-omarchy-x86_64\"
  install -Dm755 glyph \"${{pkgdir}}/usr/bin/glyph\"
  install -Dm644 glyph.desktop \"${{pkgdir}}/usr/share/applications/glyph.desktop\"
  install -Dm644 glyph.svg \"${{pkgdir}}/usr/share/icons/hicolor/scalable/apps/glyph.svg\"
  install -Dm644 README.md \"${{pkgdir}}/usr/share/doc/glyph-pdf/README.md\"
}}
"""


def render_srcinfo(version: str, sha256: str) -> str:
    lines = [
        f"pkgbase = {PKGNAME}",
        f"\tpkgdesc = {PKGDESC}",
        f"\tpkgver = {version}",
        "\tpkgrel = 1",
        f"\turl = {URL}",
        "\tarch = x86_64",
        "\tlicense = MIT",
    ]
    lines.extend(f"\tdepends = {dep}" for dep in DEPENDS)
    lines.extend(
        [
            "\tprovides = glyph-pdf",
            "\tconflicts = glyph-pdf",
            f"\tsource = {SOURCE_URL.format(version=version)}",
            f"\tsha256sums = {sha256}",
            "",
            f"pkgname = {PKGNAME}",
            "",
        ]
    )
    return "\n".join(lines)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--version", required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--out-dir", required=True, type=Path)
    args = parser.parse_args()

    version = validate_version(args.version)
    sha256 = validate_sha256(args.sha256)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    (args.out_dir / "PKGBUILD").write_text(render_pkgbuild(version, sha256), encoding="utf-8")
    (args.out_dir / ".SRCINFO").write_text(render_srcinfo(version, sha256), encoding="utf-8")


if __name__ == "__main__":
    main()
