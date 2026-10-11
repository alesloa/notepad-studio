# Gates: Notepad Studio correctness and parity audit

Scope: Continue the requested cross-platform Notepad++ parity audit and fix the reported completion behavior. These gates distinguish tested code from native runtime evidence; unresolved rows remain open.

- [x] G1: Completion placement uses the current visible caret and stays off its text line, with bounded matches and filtering after edits.
  EVIDENCE: c88a594, 4cae206 and 66c8a9d; actual deferred GPUI bounds/click/wheel tests and bounded candidate/lifecycle regressions. Native appearance is tracked in G8.

- [x] G2: Real key routing, preview transitions, deactivation and stale painted callbacks preserve the current completion session and visible editor context.
  EVIDENCE: reviewed 181087cf source integration; 10 new real-Workspace dispatch/session/focus regressions plus existing bounds/lifecycle tests pass in the 1312-unit main gate. Native validation remains G8.

- [ ] G3: Pending CSV cell text survives successful Save/close/tab switch and recovery; refused commits preserve editable text and the source.
  EVIDENCE: pending

- [x] G4: Rich typing, Enter and Tab preserve Unicode and undo selected replacements in one action, with no-op/read-only Redo preserved.
  EVIDENCE: 78af329 plus reviewed 003146fc source integration; selected-Tab actual handler, nested/no-op/read-only tests pass in the 1302-unit main gate.

- [ ] G5: Macro playback uses the visible editable source context and preserves input fields, preview source, inactive windows and read-only documents.
  EVIDENCE: ec0c73d covers CompleteWord only; general InsertText/entry routing remains under audit.

- [ ] G6: Rendered inline text fields have a real focus/action route for keyboard editing, Enter and Escape.
  EVIDENCE: pending

- [ ] G7: A complete menu/behavior parity inventory names and closes remaining Notepad++ gaps, including command shortcuts, lexer/API completion and platform behavior.
  EVIDENCE: pending

- [ ] G8: Native Windows, macOS and Linux builds and exercised UI flows confirm placement, focus, typing and save behavior.
  EVIDENCE: pending; current native computer-use transport is unavailable and CUA native surfaces are disabled. Headless GPUI tests are not native screenshots.

- [x] G9: The integrated main tree builds and passes its required unit/integration/clippy checks with no new warnings on touched code.
  CHECK: powershell -NoProfile -Command "Get-Content log/completion-routing-integration-tests.txt -Tail 16"
  EXPECT: 1312 passed; 0 failed
  EVIDENCE: completion routing integration; cargo build and clippy all-targets passed; 1312 units + 1 integration passed, 8 ignored. Refresh for each source batch.

- [x] G10: Completed source batches have separate conventional commits and current change.log/devlog entries; the main working tree is clean before beginning a new batch.
  CHECK: git status --porcelain
  EXPECT: /^$/
  EVIDENCE: main 71d33f5 was clean before this ledger was created; refresh after committing the ledger and every source batch.
