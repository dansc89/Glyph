# Line and Arrow markup: verified development increment

## Scope and source

Drawbridge target: exact `v5.5`, commit `c1c54cbc744539b28c64815c372875f9768b806e`. The comparison baseline is released Glyph 1.5 (`83b85642986f1f1e0da94e9c7661151e2af30cf9`). See the [baseline audit](drawbridge-v5.5-gap-audit.md) and [remaining implementation plan](drawbridge-v5.5-implementation-plan.md).

Implemented and integrated into `/home/daniel/Glyph`:

- **L** selects Line; **A** selects Arrow. Click a start point, then an endpoint. **Escape** cancels an unfinished gesture; text-entry focus does not activate these shortcuts.
- Original compact vector icons; no copied macOS symbols and no added toolbar row.
- Vector `/Line` PDF annotations, standard endpoint/line-ending data, explicit appearance streams and versioned Glyph ownership metadata. Imported unowned Line and Link annotations are preserved, not claimed as editable Glyph marks.
- Direction-aware hit testing, owned selection/deletion, shared Undo/Redo, transactional explicit Save/Save As and existing document/session/recovery guards.
- Horizontal, vertical and both-direction geometry, crop offsets and page rotations are covered.
- Focus loss clears draft and pending press, even without `PointerGone`. Canonical owned appearance validation rejects non-identity/malformed Form `/Matrix` values rather than trusting byte-identical paint commands alone.

## Provenance and integration

Development was isolated at `/home/daniel/.cache/glyph-v55-parity`. Every existing root source file selected for the initial integration was byte-identical to the published comparison baseline. New source paths were absent. Original source was backed up under `dist/root-before-line-arrow-integration` in that worktree; review fixes also have a pre-fix backup. Unrelated tracked/untracked work was retained. Existing package/version files were not replaced: the root development manifest still labels its package 1.1.0; that is not a new release designation.

Final optimized artifact: `/home/daniel/Glyph/dist/glyph-line-arrow-development`.

SHA256: `c6cc94e95e9e964d247a22125daab8df05201686065e2587bee64d46251e20be`.

No install, release, push or user-desktop launch was performed. Native runs used generated PDFs on owned private Xvfb displays. No private drawing content was used or published.

## Observed gates

Fresh final root source:

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --locked --all-targets -- -D warnings`: passed.
- `cargo test --locked --quiet -- --include-ignored --skip real_saltair --test-threads=1`: **353 passed**, 0 failed, **2 private-fixture checks excluded**.
- `cargo build --release --locked`: passed.
- Independent final read-only source re-review reported `passed: true`, with no blocking security/logic findings. Its two original blockers were fixed in a third context with observed behavioral RED/GREEN and regression coverage; fresh final native/Poppler runs followed the fixes.

Final stable artifact, all strict non-record-only reports:

| Gate | Passed checks | Actual reports under root `dist/` |
| --- | ---: | --- |
| Native Line/Arrow | 65 | `line-arrow-final-{0,0-crop,90-crop,180-crop,270-crop}/result.json` |
| Independent Poppler PDF rendering | 30 | matching `-poppler/result.json` |
| Existing Ellipse/save-refusal/retry workflows | 32 | `line-arrow-final-ellipse/result.json` |
| Existing viewer/navigation/zoom/pan workflows | 30 | `line-arrow-final-viewer/results.json` |
| Owned-display isolation tests | 9 | `line-arrow-final-isolation.log` |

Native checks cover committed endpoint mapping, vector appearance, real save/reopen, imported annotations/content preservation, byte-exact pre-save backups, clean Save, deletion/Undo/Redo, cancelled drafts and empty diagonal bounding-box corners. Cases include normal uncropped pages and cropped pages at 0/90/180/270 degrees. Poppler separately renders all four stems and the arrow wings at the second endpoint, and confirms original rendering is pixel-identical outside owned annotation bounds. Summaries are aggregated from actual JSON checks in `dist/line-arrow-final-summary.json`; raw failed and successful attempts remain available.

The saved pre-feature binary fails the corrected native runner at the absent horizontal Line operation; final source passes. Rust RED evidence for the initial feature and review fixes is retained in the isolated worktree. Compile/harness failures are not counted as behavioral TDD evidence. Harness corrections included maintaining cursor ownership between two clicks, measuring the full canvas for landscape crops, accommodating narrow cropped white margins and using a self-contained Pillow pixel iterator. Parallel native startup/navigation was intermittently flaky; the final recorded gates were fresh serialized strict executions, not fabricated or relaxed pass results.

## Limits and next work

This is **not full Drawbridge v5.5 parity**, a release certification, or proof of equal production stability/performance. The actual macOS app was source-audited, not executed on Linux.

- Styles are still fixed red/two-point/unfilled; move/resize, markup properties and Text/Polyline/Polygon remain incomplete.
- In-progress draft arrowheads use a screen-space preview size; committed arrows use PDF-space size capped by length. Preview/style unification is a remaining visual refinement, not claimed fixed.
- Malformed-appearance tests retain dirty/history/checkpoint state and reject writing; they do not separately prove an actual Undo-to-valid-state after arbitrary internal document tampering.
- Multi-document tabs, sheet picker, light/display inversion, printing, safe flatten/unflatten and lossless file reduction remain larger workflows, along with the other audited gaps.
- Private-fixture checks and representative production Wayland/performance comparisons remain unrun.
