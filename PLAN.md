# Notepad Studio correctness and parity audit

Continue the authorized clean-room Rust/gpui work. The primary acceptance ledger is GATES.md. Prior completed batches and measured results are in log/devlog.md; change.log records user-visible changes.

## Current ownership

- Root: review branch commits, integrate exact source hunks, run the full main-tree build/test/clippy gate, update records and commit each batch. Completion macro helper/tests remain root-owned.
- language_menu: completion popup session/view/activation routing and source focus restoration. Next leaf: actual InputFieldElement action-dispatch audit, with reproductions before edits.
- doc_parser: CSV pending-cell Save/close/tab-switch/recovery correctness. Own CSV model/actions and surgical save/close hooks; retain tab UUID across asynchronous operations.
- undo_readonly: selected rich Tab history, then macro playback context audit. Own macro replay entry and new focused tests; report reproduced scope before changing behavior.

Shared-file edits are surgical, re-read before integration. Agent branches are applied by commit, excluding conflicting changelog numbering; root gates the combined tree before committing. Rust text changes preserve encoding/EOL, Unicode and read-only behavior. Native runtime evidence is recorded separately from headless TestWindow evidence.

## Continuing branches

1. Completion: placement -> live filtering -> mouse/keyboard lifecycle -> replay context -> native visual validation.
2. Documents: saved-state history -> rich input history -> CSV exact history -> pending cell persistence -> macro/view boundaries.
3. Parity: enumerate Notepad++ menu behavior against implemented actions -> close recorded gaps -> platform build/runtime checks.

## Status

- 2026-10-10: main 71d33f5; 1301 unit tests + 1 integration passed, 8 ignored. Build and clippy passed with no new touched-code warnings. Run, composite savepoints, rich Unicode/input history and CSV exact-source history integrated. Completion routing, selected rich Tab and pending-cell persistence remain active.

- 2026-10-10: reviewed selected rich Tab 003146fc integrated; 1302 units + 1 integration passed, 8 ignored; build/clippy passed with no new warnings. G4 now met.

- 2026-10-10: reviewed completion routing 181087cf integrated; 1312 units + 1 integration passed, 8 ignored; build/clippy passed with no new warnings. G2 met. Inline input dispatch and macro/CSV persistence leaves continue.
