# First native rectangle markup increment

## Scope

This is a local development increment, not Drawbridge parity or a published release.

- Rectangle (`R`) draws an unfilled red PDF Square annotation with a native normal-appearance Form stream and a 2-point border.
- Select markup (`V`) selects attached Glyph-owned rectangles; Delete/Backspace removes the selected rectangle. Foreign/imported annotations are preserved, not exposed as destructively editable.
- Escape cancels the drawing/selection mode without stealing dismissal from an open menu or text field. View restores ordinary PDF text selection and primary-drag pan; middle-button pan remains available.
- Reverse-direction gestures, clipped release outside the page, tiny-click rejection, layer ownership and document/page identity have regression coverage.
- Bookmarks, embedded page labels and rectangles share bounded history (64 operations), saved checkpoints, Save/Save As and existing conflict/backup protections. Source bytes are unchanged before Save.
- Geometry is normalized to the displayed cropped/rotated page. Native PDFium tests cover inherited crop origins and all four orthogonal rotations, appearance pixels, unfilled centers, save/reopen and owned-only deletion.
- Serialization is on the editing worker. Preview bytes are installed into the retained PDFium document on its owning render worker; page, tile, text and thumbnail requests reuse it. No temporary PDF is written over the source merely to preview a shape.

## Concrete defects found during integration

1. The initial canvas ownership callback queried egui context from inside `ui.input`, deadlocking on the first native primary press. Ownership now resolves before the pure-data gesture callback. A frame-completion watchdog and native drawing checks cover the path.
2. The initial Escape shortcut consumed menu dismissal even in View mode. The next Bookmarks click merely dismissed the still-open Document menu. A compiling behavioral RED, scoped shortcut fix, native bookmark workflow and regression cover this.
3. Independent review found selection surviving page navigation, allowing an invisible off-page rectangle to be deleted. The native pre-fix regression reproduced the missing first-page rectangle after Delete on page 2. Review corrections and final verification are recorded below.
4. Independent review found mutation/preview failure conflation: an applied edit could be retained while reported as failed and shown against stale pixels. The completion now distinguishes successful mutation from failed preview, retains dirty/history state, reports retained changes, and blocks canvas mutation until a successful current-generation native Page texture installation (conservatively, tile-only success does not unlock tools). Retry preview, Undo and Save recovery have behavioral coverage. Link normalization now obeys the same failed-source gate as page/text rendering; invalid snapshot regressions prove there is no silent disk fallback.

## Verification status

Fresh final execution after review corrections:

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --locked --all-targets -- -D warnings`: passed.
- `cargo test --locked --quiet -- --include-ignored --test-threads=1`: 268 passed, zero failures/ignored.
- `cargo build --release --locked`: passed; executable `target/release/glyph`, SHA-256 `bff04f50ded760eea3e4dd4cecdbcb20699ca2c040e2f5cf729adeca2788fb7c`.
- Native optimized checks: 16 rectangle, 9 bookmark, 9 embedded-label, 11 context-label and 30 viewer assertions, all passed (75 total). Generated fixture reports are in `dist/rectangle-*-verified`.
- Seven native layout screenshot/OCR scenarios completed; minimum-window and reopened-rectangle images inspected. These are not counted as asserted GUI tests.
- Nine owned-display isolation tests passed. Fixture app processes exited; no user-desktop launches or user-PDF writes.
- Cropped proof of the actual reopened native window: `dist/rectangle-verified/rectangle-reopened-proof.png`.

The initial 14-check workflow and 261-test suite preceded the review corrections and are superseded by the results above.

Backend independent review found no blocking correctness/security issue. App/preview review required the corrections above. Independent re-review found no remaining blocking issue in those correction paths. Its targeted runs passed 19 rectangle, 3 invalid-snapshot, 15 render-worker, 57 editing and 5 markup tests; these overlapping selections are not added to the full-suite count. The reviewer did not run native GUI checks; parent execution results are recorded above. Snapshot-size failure uses an injected serializer seam, not a genuinely oversized end-to-end fixture.

An independent Poppler render of the actual native-saved PDF also displayed the annotation at the requested normalized bounds (within 0.015 tolerance). Pixel comparison against the retained original backup confirmed the rectangle interior was unchanged/unfilled. This verifies saved appearance with a second renderer, not complete external-reader editing interoperability.

## Limits

- Only rectangle creation, selection and deletion are in this increment. Ellipse, line, arrow, text, polyline, polygon, resize/move and style controls are not implemented.
- The 256 MiB snapshot limit bounds serialized output, not total RAM: the editable document, its serialization clone, native document and bitmap/cache storage also consume memory.
- Detached rectangle/appearance objects remain for stable history identity and are not garbage-collected after history eviction or abandoned redo. Repeated create/delete cycles can grow the file even with few visible annotations.
- Additional geometry cases (crop/media intersection, mixed overrides and unusual rotations/origins), representative large/scanned sets, native Wayland/GPU latency and external-reader interoperability remain unverified. Tiny shapes can clip the fixed-width border.
- Save is preservation, not mutual exclusion against noncooperating writers; existing retained-backup/conflict limitations apply. Actual Save As portal interaction remains unverified.
- One isolated X11 app launch failed with `XOpenDisplayFailed`; the retry launched successfully. Cause remains unconfirmed. This is distinct from the reproduced canvas deadlock.
- No launch on the user's desktop, user-PDF modification, commit, publication or installation is performed by this increment. The installed wrapper still resolves the previous interaction build, not the newly rebuilt development executable.
