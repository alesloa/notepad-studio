# Windows automatic first-line title validation

This checkpoint integrates the reviewed title patch over `0e261a97` (previous
product source `ec8ae984`). Main build, full tests and Clippy pass. The user paused
the full goal and requested committing all current changes without pushing.
Optimized controls and native verification remain open.

The change preserves explicit names and saved paths. Original Rust rules mirror
the local Notepad++ first physical line, illegal-character filtering, trimming,
63 UTF-16 unit limit and case-insensitive collision behavior. Empty or conflicting
candidates retain the accepted name. A cutoff unpaired surrogate becomes U+FFFD
in title metadata; document bytes remain intact.

Edits retain their intermediate deletion/insertion naming points. Bounded capture
and background preparation retain history and reject stale results. Copy, Save,
transfer, close and Exit wait for accepted naming metadata. Resource limits affect
naming and leave text, IME, savepoints and Undo/Redo intact.

Root verified all 775 archived candidate-13 inputs and 13 focused evidence files.
Eight focused functions pass, including actual CSV Undo, IME cancellation, window
transfer, close, background disposal and large Source/history smoke controls.
The candidate-12 full run passes 2,984 unit and three integration tests, with
34 ignored. That full result applies to candidate 12; candidate 13 changes three
equivalent warning fixes and strict worker/disposal assertions.

Candidate 13 builds and passes Clippy on a fresh F: target with no input drift.
Its 193 canonical warning pairs exactly match the main baseline. Raw JSON emits
519 located occurrences, including repeated diagnostics; this count is not
comparable to the baseline's 332 formatted-log occurrences.

Root verified the patch's 38 owned paths (10 new Rust files), all live candidate
inputs, and every non-owned input against main after newline normalization.
Reverse and main apply checks pass. The applied 38 paths match the reviewed
candidate after the same normalization. Main also updates the embedded changelog.

## Combined main results

- Build: exit 0, 103.434 seconds.
- Full tests: exit 0, 394.503 seconds; 2,984 unit plus three integration tests,
  with 34 ignored. Unit execution takes 261.49 seconds.
- Clippy: exit 0, 73.562 seconds; 519 located JSON occurrences / 193 distinct
  message-and-first-path pairs. Zero added or removed pairs against the baseline.
- All 969 frozen live inputs and nine retained evidence files independently
  verified; no input drift. Inventory check and all 12 parser tests pass.

The freeze includes the embedded changelog and excludes documentation/README.
Combined input hash: `89d1022098a828579c069cdbf2417f3a89ac66aad4e22123102b2db410ee227e`.
Freeze file: `4c1523ea4807401d2aaad30f79efd1c95f350ad1aebcb5c1a350f1ca102e4ad8`.
Evidence manifest: `a3503b03ef38085c7e1b8d340cbcef337fc40063cd7050c0b70cbfd3e9102a8d`.

Logs and artifacts are retained under
`F:/notepad-studio-builds/goal-20261012-0e261a9/evidence/root/windows-title-main-20261012/`:

- `build.log`: `64af1ec459541a8380d552a6bad5b152bd3a902f5c72c49e6d77fcece5e08e02`.
- `test.log`: `12b334fb042dc905dc1d2bc781fedd6e67cab68849db7082dd2ac9945eca4b2e`.
- `clippy.log`: `9c6a5dd13ec38c9add81683ac8c4ee0c16ba7e0de449a4ec8f01f7c4f76d6a2e`.
- Main test EXE: 177,837,056 bytes,
  `94bcfba9704d938c947b0bb5cea357da03610754d59eecfaada28d96904836df`.
- Main application EXE: 97,867,264 bytes,
  `2356fd76faedb1f6c818befc16dd15eb515df4c19edde1aed7635392dc1b8f6e`.

The default release test-harness compilation was cancelled at the user's pause.
No verified final optimized artifact exists, and none of the six timed controls
ran. Captured profile provenance shows opt-level 3, thin LTO, one codegen unit
and test-harness unwind; it does not establish shipping-abort performance.
No quiet measurement or native Windows result is claimed.
Existing C: caches and packets remain intact; new build outputs, process temporary
files and large artifact archives use the separate F: namespace.
