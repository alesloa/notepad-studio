# Notepad Studio correctness and parity audit

Continue the authorized clean-room Rust/gpui work. The primary acceptance ledger is GATES.md. Prior completed batches and measured results are in log/devlog.md; change.log records user-visible changes.

## Current ownership

- Root: review branch commits, integrate exact source hunks, run the full main-tree build/test/clippy gate, update records and commit each batch. Pure API registry and call-context helpers remain root-owned.
- language_menu: completion and inline input routing. Brief-list behavior integrated; API loading/manual Function lists integrated; next leaf: parameter hint UI. Root owns the pure registry parser.
- doc_parser: CSV draft recovery integrated; clean-grid focus integrated and first-character input integrated; next leaf: Source/CSV IME composition. Own CSV helper/test seams, preserve tabUUID and newer focus.
- undo_readonly: File menu commands integrated; context Reload integrated; next leaf: submenu/hover/Rename UUID targets and collision-safe rename.

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

- 2026-10-10: reviewed File5e6efe0 integrated;1397 units+1 integration pass,8ignored;build/clippy pass,no new touched warnings. G12met for File entrypoints/menu/defaults/UUID/draft/focus/literal paths. Shared catalog preserved root folder row/defaultless114 and View rows; tabs fixture depth1 retained. Legacy/context Reload remains active separateleaf. CSV recovery next.

- 2026-10-10: reviewed recoverybab3c30 integrated;1422 units+1 integration pass,8ignored;build/clippy pass,196app210test193duplicates,no new touchedwarnings. G3met. Immutablebasis/hash/publication-failure/detacheddraft and boundedUTF8focus/debounce coverage complete. CleanCSVgrid and legacycontextReload leaves continue; APIcompletion wiring underway. NativeG8 and broadinventoryG7 remainopen.

- 2026-10-10: atomic close batch implemented;1426 units+1 integration pass,8 ignored;build/clippy pass with no new touched-code warnings. Four actual rendered regressions prove final-tab Close/Close All/Close Unchanged cannot publish an empty strip, and Close All preserves documents opened after dispatch. CSV grid input/focus, context Reload and API completion continue.

- 2026-10-10: reviewed CSV grid focus ed51dbb integrated;1448 units+1 integration pass,8ignored;build/clippy pass with no new touched warnings.22 rendered cases include same-UUID nonCSV SaveAs, inactive/newer-field handoff and refused drafts. Ubuntu24.04/WSL8a1a353 build and1425units+1integration pass separately; native GUI/macOS and later Linux batches remain G8. CSV first-character bridge, context Reload and API completion active.

- 2026-10-10: reviewed context Reload c615c929 integrated;1466 units+1 integration pass,8ignored;build/clippy pass with no new touched warnings.18 actual-handler cases prove recovered/late CSV and rich/source/metadata changes cannot be discarded under an earlier Reload confirmation, and main tab-menu callbacks resolve original tab and popup UUID. Submenu/Rename, CSV grid input, API and async Close guards continue.

- 2026-10-10: async Close batch;1476 units+1 integration pass,8ignored;build/clippy pass with no new touched warnings. Six red data-loss cases now preserve late source/rich/live/lazy/detached CSV and metadata/loading changes; post-save dirty input refuses close. Recovery drafts refuse Save/Close before model construction while Preview can expose them. API/cache and grid-input leaves ready, IME and parameter hints underway; G7/G8 remain open.

- 2026-10-10: reviewed grid input1b6005baa integrated;1485units+1integration/8ignored,build/clippy pass,no new touched warnings. Nine real-frame/input-protocol cases prove first body/header character autosave and Unicode/UTF16 cell input routes without hidden source mutation. IME None-range composition continues separately; optional API completion next.

- 2026-10-10: reviewed API completionf592c22 + rootc1 parser integrated;1513units+1integration/8ignored,build/clippy pass,no new warnings.28 new pure/actual-frame lifecycle/manual/cache/encoding/case tests; optional background definitions and original static fallback shipped. Parameter hints+IME composition remain active; G14 met,G7/G8 open.

- 2026-10-10: Exit recovery preflight;1518units+1integration/8ignored,build/clippy pass,no new warnings. Two red failures fixed; five actual dispatch/close-hook tests cover final recovery failure, CSV drafts, all-window retry, empty Exit and failed last-tab strip recovery. Synchronous recovery error dialog removed; actionable status preserves focus. G15met; Rename and IME ready, hints/inventory active, asynchronous Exit review continues.
