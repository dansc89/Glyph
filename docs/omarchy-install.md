# Omarchy install path

Fastest install from a fresh Omarchy desktop:

```bash
curl -fsSL https://github.com/dansc89/Glyph/releases/download/1.0/install-glyph-omarchy.sh | sh
glyph
```

The script downloads the Omarchy tarball, verifies its checksum when `sha256sum` is available, installs to the user profile, and prints the launch command. It also installs a desktop/menu launcher and a detached `glyph` wrapper, so launching from Terminal does not require leaving that Terminal open. Use `GLYPH_FOREGROUND=1 glyph` only for debugging logs.

Glyph's Omarchy-first release artifact is:

```text
glyph-omarchy-x86_64.tar.gz
```

Use this on a fresh Omarchy desktop when the user should not have to install FUSE, PDFium, GTK/WebKit, OpenSSL, or other add-on packages just to launch Glyph.

```bash
tar -xzf glyph-omarchy-x86_64.tar.gz
cd glyph-omarchy-x86_64
./install-glyph.sh
glyph
```

What the tarball contains:

- `glyph` — the release binary.
- `glyph.desktop` — desktop launcher metadata.
- `glyph.svg` — app icon.
- `install-glyph.sh` — user-local installer. It writes only to `~/.local/bin`, `~/.local/share/applications`, and `~/.local/share/icons` unless `GLYPH_INSTALL_DIR` is set.

The AppImage remains available, but it is not the primary Omarchy path because many fresh systems do not ship the legacy FUSE userspace package expected by AppImage runtimes. If a user wants the AppImage path without installing FUSE, ship `install-glyph-appimage.sh` beside `Glyph-x86_64.AppImage` and run:

```bash
./install-glyph-appimage.sh ./Glyph-x86_64.AppImage
glyph
```

That installer uses AppImage's built-in `--appimage-extract` mode, installs the extracted payload under `~/.local/share/glyph/appimage-extracted`, and creates a detached `glyph` wrapper in `~/.local/bin`. It does not require FUSE at runtime or an open Terminal window after launch.

Release CI guards this promise by:

- using `pdfium-bundled` so no system PDFium package is needed;
- failing if `ldd target/release/glyph` reports missing libraries;
- failing if the binary links GTK, GDK, WebKit, OpenSSL, system PDFium, or FUSE;
- packaging and syntax-checking both Omarchy tarball and AppImage installers;
- publishing SHA256 checksums for every release artifact.

Baseline graphics/windowing libraries from the desktop OS are still expected, as with any native Linux GUI app. On Omarchy those are part of the desktop environment, not extra Glyph install steps.

## AUR / pacman path

Yes: Omarchy is Arch-based, so the clean package-manager path is an AUR package installable with an AUR helper such as `yay`/`paru`, then launched like any other desktop app.

Prepared package recipes live in the repo:

- `packaging/arch/glyph-pdf-bin/` — binary-release package recipe, intended for AUR publication as `glyph-pdf-bin`.
- `packaging/arch/PKGBUILD` — source-build recipe, intended to become a `glyph-pdf-git`/source package path later.

The AUR names `glyph` and `glyph-bin` are already occupied by an unrelated ASCII-art/video project, so Glyph PDF should use `glyph-pdf-bin` instead of trying to claim `glyph`.

Once published to AUR, the intended Omarchy command is:

```bash
yay -S glyph-pdf-bin
glyph
```

The AUR package installs the same detached launcher behavior: `/usr/bin/glyph` starts the real binary in the background by default, while `GLYPH_FOREGROUND=1 glyph` keeps logs attached for debugging.

Until then, the one-line release installer remains the easiest tested path:

```bash
curl -fsSL https://github.com/dansc89/Glyph/releases/download/1.0/install-glyph-omarchy.sh | sh
glyph
```