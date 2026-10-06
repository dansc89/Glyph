# Ellipse markup and save reliability increment

Local working-tree increment; not an installed or published release. This follows the rectangle milestone, not full Drawbridge markup parity.

## Scope

- `E` / Ellipse draws an unfilled red 2 pt native PDF `/Circle` annotation. `R` retains Rectangle and `V` selects Glyph-owned shapes for Delete/Backspace.
- Both tools use shared bounded history and saved-state checkpoints with bookmark/page-label edits. Unsaved edits are rendered from snapshots without modifying source PDFs.
- New ellipse ownership is `/GlyphEllipse 1`; existing `/GlyphRectangle 1` Square annotations remain readable. Explicit normal appearance streams use four cubic segments, not a rectangular stand-in.
- Existing preview-unavailable gates, retained-change feedback, Retry preview and cross-page deletion protection remain required.

## Reproduced save failures — corrected

1. Valid imported integer-valued real numbers (`/MediaBox [0.0 0.0 612.0 792.0]`, also foreign appearance `/BBox`) serialize into integer syntax. Raw object-type equality consequently rejects both Save and Save As despite unchanged values. The native RED fixture passed editing/history checks, then failed at Save commit; source bytes were preserved. Fix must allow only the writer's exact Real-to-Integer canonicalization recursively, not tolerances or lossy integer-to-float comparison.
2. A finishing pointer release and Ctrl+S in one frame can start the Save worker before canvas processing, suppress the final gesture and leave a clean checkpoint missing the stroke. A deterministic headless actual-egui regression reproduced one persisted/displayed shape instead of two. The fast native probe happened to pass and is not sufficient evidence against the deterministic reproduction. Save intent must run after gesture processing and survive the asynchronous mutation until completion.

## Native test harness additions

`scripts/markup-qa.py` accepts:

- `--shape rectangle|ellipse`: complete draw/history/delete/navigation/save/reopen workflow; ellipse adds a curved-appearance assertion.
- `--repeat-save`: add the other shape to the same page, save again, check backups, Undo across the saved checkpoint and save the restored first shape.
- `--read-only-retry`: with repeated saves, intentionally make the generated fixture read-only; require persistent error feedback, no crash, unchanged saved bytes and retained unsaved shape, then restore permissions and retry.
- `--imported-reals`: generated raw lexical real-number MediaBox fixture with a rebuilt valid xref; no user PDF manipulation.
- `--save-on-release`: a fast native release/Save/reopen probe; deterministic Rust frame-order tests are authoritative for the race.

Before the two save fixes, optimized ellipse and rectangle mixed-state workflows passed 30 and 29 assertions respectively. Independent Poppler rendered the saved ellipse as a curved, unfilled red outline; independent pypdf inspection found a native Circle and a Circle+Square in the prior-version backup. These are baseline native results, not the final remediated-build gate.

## Final verification

- `git diff --check`, `cargo fmt --all -- --check`, and `cargo clippy --locked --all-targets -- -D warnings`: passed.
- `cargo test --locked --quiet -- --include-ignored --test-threads=1`: **285 passed, 0 failed, 0 ignored**. Includes actual PDFium crop/rotation checks and deterministic R/E release+Save/Save As frame-order regressions.
- `cargo build --release --locked`: passed; optimized binary SHA-256 `6269512ec073dad0935c28c2879dfa799b89f406b574ba15b2173b4a0dc68b8a`.
- Final optimized workflows: **86 native assertions passed** (ellipse mixed/repeated/read-only recovery on imported-real fixture 30; rectangle counterpart 29; immediate release+Save ellipse 5 and rectangle 4; bookmark edit/save/reopen 9; page labels 9). Reports are under `dist/ellipse-save-verified`, `rectangle-save-verified`, `ellipse-release-save-verified`, `rectangle-release-save-verified`, `ellipse-bookmarks-verified`, and `ellipse-page-label-verified`.
- **9 display-isolation tests passed**. `dist/ellipse-layout-verified` contains seven captured/OCR-reviewed layout scenarios, not seven additional assertion counts.
- Independent final pypdf inspection: saved `/Circle` with `/GlyphEllipse 1`, four parsed cubic operations, stroke-only AP; prior backup `.glyph-backup-0nKJ2c.pdf` contains both Circle and Square. Poppler independently rendered the final saved oval.
- Independent ellipse and save-fix reviews: passed, no blocking security/logic finding. Save review full JSON: `/home/daniel/.hermes/cache/delegation/subagent-summary-0-20261005_154609_864600.txt`. Ellipse review trace: `/home/daniel/.hermes/cache/delegation/live/deleg_3ecabeae/task-0.log`.
- Document-scoped deferred save intent is drained after canvas processing and after successful owned worker completion. Stale identity/generation, failure and modal transitions cancel it. One deferred slot is used: a later accepted request replaces an earlier intent; shortcut handling checks Save As before Save.
- Nonblocking review suggestions: direct simultaneous-intent and no-annotation-release tests, stronger picker-cancellation Undo/Redo assertions, and closer ghost/native zoom-dependent stroke fidelity.

The normal Glyph launcher has not been changed; its checksum differs from the optimized local build. No user-desktop instance was launched and no user PDFs were modified.

## Limits

Fixed red 2 pt appearance only; no resize/style controls, imported annotation editing, text/highlight/freehand/line tools, comprehensive drawing-set stress or power-loss durability guarantee. Save As is exercised with backend and injected picker tests, not the platform portal dialog. Signed/encrypted PDFs and genuine source conflicts/read-only targets remain protective refusals rather than unsafe overwrite support. Failure feedback is required to retain edits and explain safe recovery; no claim is made that Glyph cannot crash or that these Glyph bugs explain Drawbridge's failures.
