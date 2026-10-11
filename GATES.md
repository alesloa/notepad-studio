# Gates: Notepad Studio correctness and parity audit

Scope: Continue the requested cross-platform Notepad++ parity audit and fix the reported completion behavior. These gates distinguish tested code from native runtime evidence; unresolved rows remain open.

- [x] G1: Completion placement uses the current visible caret and stays off its text line, with bounded matches and filtering after edits.
  EVIDENCE: c88a594, 4cae206 and 66c8a9d; actual deferred GPUI bounds/click/wheel tests and bounded candidate/lifecycle regressions. Native appearance is tracked in G8.

- [x] G2: Real key routing, preview transitions, deactivation and stale painted callbacks preserve the current completion session and visible editor context.
  EVIDENCE: reviewed 181087cf source integration; 10 new real-Workspace dispatch/session/focus regressions plus existing bounds/lifecycle tests pass in the 1312-unit main gate. Native validation remains G8.

- [ ] G3: Pending CSV cell text survives successful Save/close/tab switch and recovery; refused commits preserve editable text and the source.
  EVIDENCE: reviewed fec4d0c/e257957 Save/close/navigation batch; 13 new handler regressions pass in the 1325-unit main gate. Recovery leaf remains open.

- [x] G4: Rich typing, Enter and Tab preserve Unicode and undo selected replacements in one action, with no-op/read-only Redo preserved.
  EVIDENCE: 78af329 plus reviewed 003146fc source integration; selected-Tab actual handler, nested/no-op/read-only tests pass in the 1302-unit main gate.

- [x] G5: Macro playback uses the visible editable source context and preserves input fields, preview source, inactive windows and read-only documents.
  EVIDENCE: reviewed b57b386; 13 context/caller/error regressions plus one isolated macro fixture test pass in the 1353-unit main gate. Preview, CSV draft/grid, visible fields, inactive/unfocused/loading/read-only/missing-tab contexts refuse replay; completion is closed first. Partial completion failure stops further events/repetitions and retains the reason. Native evidence remains G8; playback Undo grouping is covered in G13.

- [x] G6: Rendered inline text fields have a real focus/action route for keyboard editing, Enter and Escape.
  EVIDENCE: reviewed 766b763..ec8869b source delta; reproduced Go to Line Enter crash, seven actual-frame focus/edit/clipboard/Enter/Escape regressions, and multiline raw-value rendering tests pass in the 1339-unit main gate. Bounded shaping and catalog-derived Mac fixture chords were reviewed in 4ffcf61 and integrated. Native evidence remains G8.

- [ ] G7: A complete menu/behavior parity inventory names and closes remaining Notepad++ gaps, including command shortcuts, lexer/API completion and platform behavior.
  EVIDENCE: pending

- [ ] G8: Native Windows, macOS and Linux builds and exercised UI flows confirm placement, focus, typing and save behavior.
  EVIDENCE: pending; current native computer-use transport is unavailable and CUA native surfaces are disabled. Headless GPUI tests are not native screenshots.

- [x] G9: The integrated main tree builds and passes its required unit/integration/clippy checks with no new warnings on touched code.
  CHECK: powershell -NoProfile -Command "Get-Content log/macro-undo-integration-tests.txt -Tail 16"
  EXPECT: 1367 passed; 0 failed
  EVIDENCE: macro Undo batch; cargo build and clippy all-targets passed; 1367 units + 1 integration passed, 8 ignored. Refresh for each source batch.

- [x] G10: Completed source batches have separate conventional commits and current change.log/devlog entries; the main working tree is clean before beginning a new batch.
  CHECK: git status --porcelain
  EXPECT: /^$/
  EVIDENCE: main8106a10 was clean before this macro Undo batch; 63 current-version release notes and a devlog entry accompany it. Refresh after each source batch.

- [x] G11: View > Tab exposes every oracle Tab command in order with rebindable shortcuts; selection/reordering/color commands preserve document UUID, text, encoding/EOL, saved state and history, with edge and pending-cell refusal coverage.
  EVIDENCE: oracle RC789-817 / Commands1304-1412,3532-3537; red missing-menu contract then 7 new menu/actual-dispatch/refusal/edge tests pass. 1332-unit main gate; existing shortcut parity/collision tests cover added rows. Native appearance remains G8.

- [ ] G12: File menu tab commands use current UUID targets, expose oracle order and collision-free defaults, preserve pending cell drafts on refusal and launch literal external paths safely.
  EVIDENCE: active agent branch reproduced missing menu/catalog, read-only draft loss on Reload and premature Close Unchanged; reviewed source integration and main gate pending.

- [x] G13: Macro editing playback is a single Undo operation across repetitions and nested command groups; no-op/refused replay preserves Redo, failures close their group and saved-state/Unicode/EOL/caret history remains correct.
  EVIDENCE: behavior-only oracle NppCommands.cpp83-149 and shortcut.cpp925-1054; six red failures then nine new handler/history/recording regressions pass in the 1367-unit gate. Balanced nested source transactions defer to the outer group; failed replay closes its group before restoring its reason. New recordings omit Undo/Redo; older persisted history events remain readable and replay on their legacy timeline.
