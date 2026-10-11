# Notepad Studio correctness and parity audit

Continue the authorized clean-room Rust/gpui work. The primary acceptance ledger is GATES.md. Prior completed batches and measured results are in log/devlog.md; change.log records user-visible changes.

## Current ownership

- Root: review branch commits, integrate exact source hunks, run the full main-tree build/test/clippy gate, update records and commit each batch. Pure API registry and call-context helpers remain root-owned.
- language_menu: completion and inline input routing. Brief-list behavior integrated; API loading/manual Function lists integrated; parameter hints integrated; regex capture replacement integrated; separate FIF read-error report fix ready; reviewing background syntax reparse and replacement context follow-ups. Root owns the pure registry parser and completion/IME follow-ups.
- doc_parser: CSV draft recovery integrated; clean-grid focus integrated and first-character input integrated; Source/CSV IME composition integrated; asynchronous Exit recovery integrated; next leaf: large manual indentation with atomic application and fast history, preserving recording/replay refusal until async macro ordering is implemented.
- undo_readonly: File menu commands integrated; context Reload integrated; submenu/Rename guards integrated; complete inventory integrated; static-menu catalog completeness and actual Shortcut Mapper routing integrated; captured Source origin guards integrated; malformed XIM reply crash fix in validation, followed by separately reproduced Japanese decoder and synchronous commit issues. Fresh-slot OSIME retirement remains open.

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

- 2026-10-10: reviewed submenu/Rename8935351 integrated;1532units+1integration/8ignored,build/clippy pass196app210test193duplicates,no new touched warnings.14 actual painted/native-file tests prove popup/UUID/fresh focus guards and atomic no-replace rename; renamore0.3.2 pinned, permissive license/NOTICE included. Root Close/futures/Quit preserved. IME next; hints, inventory and async Exit continue.

- 2026-10-10: reviewed IMEd46b446 integrated;1548units+1integration/8ignored,build/clippy pass196app210test193duplicates,no new touched warnings.16 actual protocol/history/draft/context cases; Source marked preedit aggregates into one exact Undo, cancellation restores prior Redo/revision, committed macro event emits once. Generic field/DocView IME and lost-native-finalization macro parity remain open. Parameter hints/inventory/async Exit/current Linux continue.

- 2026-10-10: reviewed adccf497 parity inventory:716 ordered resource nodes,481 command IDs,63 exposed actions missing CATALOG and behavior/stub/native gaps recorded in docs/notepadpp-parity.md. Inventory is complete; feature parity remains open. Linux80d23fd build+1547 units+1integration pass/8ignored; Clippy1.99 inherited vendor golden-ratio deny lint needs a separate portable fix.

- 2026-10-10: Edit menu line indentation and duplication separated from keyboardTab/CtrlD.19 new tests;1567units+1integration/8ignored,build/clippy pass. Foreground indentation4096-row cap refuses atomically; larger background batches and block-selection parity remain open. Release12MiB core3.5us indent/1.3us duplicate;4096-row plan+Rope median6.886ms/p9518.421ms, no native latency claim.

- 2026-10-10: portable GPUI golden ratio preserves exact float bits/const/pixel/rem behavior. Reviewed3254c04 integrated;1567units+3integration/8ignored,Windowsbuild/clippy pass. Linux1.99 leaf Clippy and geometry tests pass; current combined Linux/native/macOS checks remain open.

- 2026-10-10: original parameter hints integrated;32 new tests,1599units+3integration/8ignored,Windowsbuild/clippy pass,no new warnings. Fresh bounded background scan/API replies, active argument/overload/caret+list placement and IME/tab/frame lifecycle covered. Ordinary manual-hint dismissal status, IME-final completion lists, native visual/currentMac checks remain follow-ups. Regex, catalog and asyncExit branches continue.

- 2026-10-10: quiet successful-manual-hint dismissal fixed after real warm-shortcut/input red regression;1600units+3integration/8ignored,Windowsbuild/clippy pass. Initial manual errors remain actionable. Immutablef4d1a4f current Linux build/test/clippy1598units+3integration/8ignored pass; nativeMac/UI and laterLinux remain open. AsyncExit leafbf246db reviewed next; catalog/regex continue.

- 2026-10-10: reviewedbf246db asyncExit capture/write/freshness and distinct final writer retirement integrated.16 new tests;1616units+3integration/8ignored,Windowsbuild/clippy pass,no touched warnings. Release16MiB primitive28.836/62.905ms sync vs0.124/0.164ms stamp+snapshot, excludes registry/native frames; final barrier may wait for current writer. Native/currentMac/laterLinux and large edit/history/macro parity remain open; catalog/regex leaves continue.

- 2026-10-10: final Source IME commits/unmarking now resume automatic completion, with configured character threshold and no preedit/cancel/nonword/selected-replacement popup. Five new actual protocol/paint/history/save regressions;1621units+3integration/8ignored,build/clippy green,no owned warnings. Generic native callback origins/fields/DocView composition and native OSIME remain open.

- 2026-10-10: reviewedaa0e3aa adds63 fixed-menu Shortcut Mapper rows;252 total. Production menu-builder completeness and real painted row/Modify/capture/binder/Workspace dispatch, conflict Cancel/Clear/Reset and old/new/unknown persisted names verified. Seven new tests;1628units+3integration/8ignored,build/clippy green,no owned warnings. Named payload shortcuts remain open; inventory baseline tables remain pinned.

- 2026-10-10: immutable12c3fe0 Linux current full gates passed on Ubuntu24.04/WSL Rust1.99:1627units+3integration/8ignored;build1m19s,test27.08s,Clippy27.62s,195app209test192duplicate inherited warnings,no owned warnings. Native Windows/Linux/macOS UI/OSIME and currentMac build remain G8.

- 2026-10-10: actual native-input/helper regressions reproduce bookmark/mark and Begin-End signed-offset overflows; checked unsigned shifts now repair/discard malformed metadata and enforce mark/marker character boundaries. Six new tests preserve valid affinities, Unicode/CRLF text and saved Undo/Redo identity.1634units+3integration/8ignored,build/clippy green,no owned warnings. Bulk editing and captured Source origin leaves remain separate.

- 2026-10-11: immutable e6f5cab Linux full gates passed on Ubuntu24.04/WSL Rust1.99:1633units+3integration/8ignored;build51.59s,test compile1m57s/unit runtime5.36s,Clippy41.34s,195app209test192duplicate inherited warnings,no owned warnings. Captured Source origin, regex and large indentation leaves remain active. CurrentMac and native UI/OSIME remain G8.

- 2026-10-11: smart-highlight buffer cutoff now runs before physical-line copying and cached-word acceptance. One stale cached-band RED regression and one exact-limit Unicode/CRLF source/history positive control;1636units+3integration/8ignored,build/clippy green,no owned warnings. Bulk leaf will measure large-line publication; native/currentMac/laterLinux remain open.

- 2026-10-11: reviewed7b20f48 captured Source callback protection integrated. UI-owned nonwrapping document and Window focus tokens, Rope/revision/linear/block selection proof and quiet stale queries/mutations; current native protocol typing/UTF16 composition keeps one handler usable across repaint.28 tests include first welcome paint, Source/caret/focus/tab/history ABA, CSV SaveAs Source mode repair and pending live/recovery draft retention.1664units+3integration/8ignored,build/clippy green,no owned warnings; NOTICE records GPUI token/test seam. Late OSIME callbacks fetched from a new slot, native visual/currentMac/laterLinux and root Hint/API focus ABA remain open.

- 2026-10-11: Hint/API requests and cold manual hint intents pin the synchronous Window input context. Three actual same-frame focus-return REDs now pass; an immediate fresh manual invocation first retires the old intent, with no-op focus/activation and real repaint positive controls. Five new tests;1669units+3integration/8ignored,59focus checks,build/clippy green,no owned warnings. Immutable1097ce6 Linux full gates1663units+3integration/8ignored passed; currentMac/native and laterLinux remain open.

- 2026-10-11: reviewed b24780c exact owned delta from8a2e5bf integrated without prerequisites; root Source retirement/smart-highlight/focus fixes preserved. Regex numbered/named/whole/optional/zero-length capture replacement, bounded Source worker planning and Rich AST preparation, one Undo/document and atomic preflight refusals. Two actual tab-return REDs now pass through nine lifecycle hooks, including Source MRU epoch preservation.48 new tests;1717 units+3 integration/9ignored,build/clippy green,196app210test193duplicate inherited warnings,no owned warnings. Logs log/regex-tabs-main-red.txt and log/regex-main-{build,tests,clippy}.txt. Immutableb247 release single-run Source12.6MB/4096 edits Normal invocation13.431ms/total13.561ms vsRegex2.250ms/24.456ms,capture20.1us/apply7.979ms; Rich1MiB/7 edits invocation14.186ms,total28.236ms,capture426.8us/apply144.1us. No native font/paint or total-latency improvement claim; measurements use pre-e6 annotation helpers. Full Boost/search macro/FIF cancellation and encoding/native/currentMac/laterLinux remain open. Separate23dc read-error reporting fix is next.

- 2026-10-11: reviewed23dc7d1 narrow FIF read-error reporting delta fromb247 integrated. Actual missing-after-discovery and Windows exclusive-lock REDs now report exact path/OS error, zero failed writes/recreation, BOM/CRLF/Unicode bytes preserved.1719units+3integration/9ignored,build/all-targetClippy green,196app210test193duplicate inherited warnings,no owned warnings. Logs log/fif-read-main-{build,tests,clippy}.txt plus leaf read-failure-{red,green}.log; no heavy-path performance change. Native/currentMac/laterLinux and FIF cancellation/encoding remain open.

- 2026-10-11: actual stale manual API reply left Loading status indefinitely (RED1/positive2). Request now records whether it created that message; stale current-id replies clear only owned unchanged Loading after caching. Three tests cover loaded/missing/malformed replies, newer command status, nonloading reload with similar text and older/newer requests; Source/history unchanged.1722units+3integration/9ignored,build/all-targetClippy green,196app210test193duplicate inherited warnings,no owned warnings. Logs log/api-loading-status-{red,focused,build,tests,clippy}.txt. Immutable950a5ee Linux full gates1716units+3integration/9ignored passed; build1m57s,test compile1m08s/runtime4.18s,Clippy47.87s,195app209test192duplicate inherited warnings,no owned warnings. CurrentMac/native/laterLinux remain open.
