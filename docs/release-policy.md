# Glyph release policy

Glyph uses simple public release numbers from here forward:

```text
1.0, 1.1, 1.2, 1.3, ...
```

No public `0.x`, `alpha`, `beta`, `rc`, or patch-style release names for normal updates. Small polish passes still get the next minor release number.

## Version mapping

- GitHub release/tag names: `1.0`, `1.1`, `1.2`, ...
- Cargo/package metadata: matching semver patch form, e.g. release `1.0` uses Cargo version `1.0.0`, release `1.1` uses `1.1.0`.

## Release quality bar

Every numbered release should include:

- Arch-first `glyph-arch-x86_64.tar.gz`.
- `Glyph-x86_64.AppImage` as alternate portable artifact.
- `install-glyph-appimage.sh` for no-FUSE AppImage extraction installs.
- `glyph-x86_64.deb`.
- `glyph-x86_64-unknown-linux-gnu.tar.gz` compatibility tarball.
- `SHA256SUMS`.
- CI pass for formatting, tests, release build, runtime dependency guard, and packaging.
- Download-back verification before announcing a release as ready.

## 1.x product direction

The `1.x` train is not just version churn. Each release should make Glyph faster, cleaner, safer, and more useful for real drawing-set review: responsive navigation, a stronger drawing-set workflow, better PDF search/linking, safer save/export behavior, and a calmer Arch-native user experience.
