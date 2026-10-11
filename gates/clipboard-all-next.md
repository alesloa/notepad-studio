# Gates: copy all file names and paths

Scope: add Notepad++ Copy All Filenames and Copy All File Paths in current-window visual tab order.

- [x] G1: oracle behavior and missing actual menu routes established
  EVIDENCE: RC511-512, NppCommands.cpp:1529-1549/4590-4591 and Common.cpp:1088-1106 reviewed. Full inventory found no main-menu/CATALOG routes. Oracle adds CRLF unless current output already ends with CRLF; no output leaves clipboard unchanged. One local window tab strip preserves independent duplicate buffers.
- [x] G2: actions, menu order, shortcut catalog and macro record/replay implemented
  EVIDENCE: Root reviewed and applied11-file patch1b6cd8a4a789844c654b27b2e10cf832a148e6de3d6e1bb3ff8635efcd77e260. Two Edit actions/menu rows after exact separator, unassigned CATALOG rows, render handlers, stable MacroEvent identities/replay arms. Root replaced stale menu header claims; no toolbar/context additions because oracle has none.
- [x] G3: actual caller tests verify Unicode, untitled labels, duplicate names, reordering, CRLF output and saved macro replay
  EVIDENCE: Root main clipboard_all group8 passed0.08s, log/clipboard-all-main-focused.txt. Real menu dispatch/registered handlers preserve document state, Unicode/untitled labels, duplicate paths, visual reordering and CRLF aggregation; production TOML writer/loader cold playback resolves current labels/order and does not recursively record. Required two-row shortcut oracle fixture correction integrated separately, patchdc79c3db9a9387704a48602834c4e6f7ed030a89ece001cddfa2294ec29d9b1d; leaf failing default test then passed.
- [x] G4: focused tests pass and standalone reviewed patch ready
  EVIDENCE: Exact final leaf passes build, 2426 unit plus 3 integration tests (22 ignored), Clippy without new-file diagnostics and 8 focused tests. Final-v2 patch adce8e1573e723de8b0e46d0d62274cc5b6f83249c7e93647e19261b51f1520f and manifest 2b07b9c1b0d5c1e7fe06ea959938b41a2a72d7faf2f39a7e4d5a021b6ebc93b6 reviewed; only prior defaults/lifetime deltas required. Main now passes 9 focused tests in 0.09s after root cancels pending bookmark copy; combined Windows gates pending.

Final combined main Windows build,2471 unit plus3 integration tests/24 ignored and Clippy pass. Source committed fdf45a0; no new diagnostics. Root expanded the joint test to include MarkCopy and added all three painted tab-popup copy rows, covered by14-test bookmark filter and the full suite. See gates/resumed-parity.md.
