# Editing-interaction usability pass

## Reproduced and corrected

1. Immediate typing in Rename appended to the original. Native generated-PDF reproduction typed `A101` without Ctrl+A and displayed `1A101`. Rename now selects the complete original when first opened, for both bookmark and page-label targets. Selection uses character indices, not UTF-8 bytes. A behavioral regression failed with `План 第一A101` and now proves replacement plus subsequent draft continuation without repeated selection.
2. Page-label editing was only discoverable from Document → Rename current page label. Thumbnails now offer a context action for the clicked physical page. It does not change the displayed page. The test first failed because the action was absent, then passed through actual secondary-click/menu-item input with target page 2 and selected page 1.
3. Bookmark context Rename remained enabled after the editable worker session was lost, unlike Document-menu editing. A context-input regression opened a doomed rename under unrecoverable state; it now remains disabled. This is disabled-state consistency, not recovery of lost unsaved worker state.

## Real execution

- Strict Clippy, full tests including ignored (243 passed), optimized build and diff checks passed.
- New `scripts/page-context-qa.py`: eleven native checks passed. Used actual thumbnail popup, immediate typing, exact native clipboard read-back, cancelled inspection of page 1's unchanged label, retained page-1 counter/purple drawing, Undo/Redo, unchanged source bytes before Save, successful Save, exact displayed-pixel preservation across Save, original backup and reopen.
- Existing native page-label QA: nine checks passed; bookmark/editing QA: nine; viewer QA: thirty. Isolation: nine passed. These are generated-fixture correctness checks, not Drawbridge performance benchmarks or comprehensive production compatibility.
- Native clipboard read-back is used for draft exactness because small caption OCR confused `1` and `l`. Visibility matching tolerates that OCR ambiguity, but exact label assertions do not. A thumbnail pixel-equality probe was inappropriate because hover/scrollbar geometry changes card centering; actual native label read-back replaced it. The Save pixel-preservation assertion remains exact.
- Artifacts: `dist/interaction-context`, `dist/interaction-labels`, `dist/interaction-bookmarks`, `dist/interaction-viewer`, plus initial append reproduction in `dist/interaction-red`.
- Independent focused review completed with no concrete new regressions or security issues found. It read the entire untracked editing module, thumbnail wiring and relevant egui behavior, confirming one-time character-based selection, physical-page targeting, operation/modal/lost-session guards and secondary-versus-primary click semantics. Reviewer ran the initial-selection test (one passed) and thumbnail tests (four passed), offline. It did not run PDF-generating context/lost-session regressions or native GUI checks; those were covered by parent verification above. No source edits or GUI/PDF writes were made by the reviewer.

## Installed-build discrepancy

The normal `/home/daniel/.local/bin/glyph` launcher executes `/home/daniel/.local/share/glyph/glyph-bin`, not `/home/daniel/Glyph/target/release/glyph` directly; the desktop entry calls that wrapper. Initially their SHA-256 digests differed. After the user explicitly chose installation with rollback and no launch, the tested release artifact atomically replaced the installed executable. Independent checksum read-back confirmed equality. The prior binary is retained at `/home/daniel/.local/share/glyph/rollback/glyph-bin-20261005T171600Z`, with its original checksum preserved. Launcher/desktop entry were unchanged. No installed-app launch, publication or user-PDF modification occurred. Any already-running instance remains on its old executable; new launches use the updated build.

## Still open

Representative large/scanned drawing sets, actual Save As portal interaction, recovery/backup discoverability, coherent toolbar/footer organization, markup and remaining bookmark workflows. Passing generated-fixture tests does not establish Drawbridge-equivalent stability or polish.
