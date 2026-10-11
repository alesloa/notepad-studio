# Notepad Studio correctness and parity audit

Continue the authorized clean-room Rust/gpui work. The primary acceptance ledger is GATES.md. Prior completed batches and measured results are in log/devlog.md; change.log records user-visible changes.

## Current ownership

- Root: review branch commits, integrate exact source hunks, run the full main-tree build/test/clippy gate, update records and commit each batch. Completion macro helper/tests remain root-owned.
- language_menu: completion and inline input routing. Brief-list behavior integrated; next leaf: optional user API loading and manual Function lists. Root owns the pure registry parser.
- doc_parser: CSV pending-cell Save/close/tab-switch/recovery correctness. Own CSV model/actions and surgical save/close hooks; retain tab UUID across asynchronous operations.
- undo_readonly: macro playback context batch ready; next leaf: File menu actions backed by existing tab helpers and safe native external launching.

Shared-file edits are surgical, re-read before integration. Agent branches are applied by commit, excluding conflicting changelog numbering; root gates the combined tree before committing. Rust text changes preserve encoding/EOL, Unicode and read-only behavior. Native runtime evidence is recorded separately from headless TestWindow evidence.

## Continuing branches

1. Completion: placement -> live filtering -> mouse/keyboard lifecycle -> replay context -> native visual validation.
2. Documents: saved-state history -> rich input history -> CSV exact history -> pending cell persistence -> macro/view boundaries.
3. Parity: enumerate Notepad++ menu behavior against implemented actions -> close recorded gaps -> platform build/runtime checks.

## Status

- 2026-10-10: main 71d33f5; 1301 unit tests + 1 integration passed, 8 ignored. Build and clippy passed with no new touched-code warnings. Run, composite savepoints, rich Unicode/input history and CSV exact-source history integrated. Completion routing, selected rich Tab and pending-cell persistence remain active.

- 2026-10-10: reviewed selected rich Tab 003146fc integrated; 1302 units + 1 integration passed, 8 ignored; build/clippy passed with no new warnings. G4 now met.

- 2026-10-10: reviewed completion routing 181087cf integrated; 1312 units + 1 integration passed, 8 ignored; build/clippy passed with no new warnings. G2 met. Inline input dispatch and macro/CSV persistence leaves continue.

- 2026-10-10: reviewed CSV pending-cell fec4d0c/e257957 integrated; 1325 units + 1 integration passed, 8 ignored; build/clippy passed with no new touched-code warnings. Save/close/switch now include pending text; G3 remains open for recovery.

- 2026-10-10: View Tab menu and actions implemented from behavior-only oracle review; 1332 units + 1 integration passed,8 ignored; build/clippy passed. G11 met; G7 remains open for broader parity. Inline crash/focus and macro context branches are ready for integration; CSV recovery is active.

- 2026-10-10: reviewed inline input focus/crash ec8869b delta integrated; 1339 units + 1 integration passed,8 ignored; build/clippy passed with no new touched-code warnings. G6 met for real headless routing; native checks remain G8. Macro context integration is next; bounded shaping, CSV recovery and File menu branches continue.

- 2026-10-10: reviewed macro context b57b386 integrated;1353 units+1 integration pass,8 ignored;build/clippy pass with no new touched warnings. G5 met. Macro playback Undo grouping remains under G7; bounded input shaping is ready; CSV recovery and File menu continue.

- 2026-10-10: reviewed bounded inline4ffcf61 integrated;1358 units+1 integration pass,8 ignored;build/clippy pass with warnings reduced199app/211test,195duplicates. Optimized11.2MB preparation improves14-16ms to6-14us; no native-font/paint claim. G12 File actions and G13 macro Undo leaves recorded; brief autocomplete semantics branch active.

- 2026-10-10: macro Undo batch implemented;1367 units+1 integration pass,8 ignored;build/clippy pass with no new touched warnings. G13 met for editing playback, with legacy persisted Undo/Redo timeline compatibility. CSV recovery, File menu and brief-completion leaves remain active.

- 2026-10-10: reviewed brief02ee135 integrated;1375 units+1 integration pass,8 ignored;build/clippy pass with no new touched warnings. Preferences now control row narrowing, retained nonmatches cannot overwrite new text, capped discovery and matching selection remain bounded. API registry/completion implementation begins separately; CSV recovery and File batches ready for review.
