# Notepad Studio handoff, 2026-10-11

Resume the user's ongoing Notepad++ parity and correctness work. This handoff covers the resumed session as well as the earlier save/proofreading batches. Full parity remains open; use the generated inventory to select confirmed gaps.

## Starting state

- Repository: `C:\Users\Ale\Documents\GitHub\notepad-studio`, branch `main`.
- Resumed implementation checkpoint: `fdf45a0fea522dc05af743722e8f224b9872225c`. Completion test commit: `72dcf44`. A records commit follows this implementation checkpoint. This session started clean at `beb5679`; the previous implementation checkpoint was `647b0ff`. Check current Git state before working.
- Notepad++ behavior oracle: `C:\Users\Ale\Documents\GitHub\notepad-plus-plus-master`. Read behavior, menu order and shortcuts; write Rust independently. Apache-2.0 rules and licensing are in `AGENTS.md`.
- Windows PowerShell. The latest checks used `CC` and `CXX` set to `C:\Program Files\LLVM\bin\clang-cl.exe`.
- Reviewed worker patches are integrated on main and committed. All workers completed and their borrowed Cargo caches are idle. Old worktrees under `..\notepad-studio-agents` remain for local evidence; reapplying them would duplicate shipped changes.
- No Linux/WSL/VM work was started. User currently wants Windows and Mac only; preserve existing platform code and CI configuration.
- The reported application crash was **T3 Code / the agent session**, not a reproduced Notepad Studio crash. No Notepad Studio panic log was found during recovery.

Read `AGENTS.md` and `README.md` first. `.codegraph/` exists in main, so use CodeGraph before locating or understanding indexed code. Read the latest entries in `GATES.md`, `PLAN.md`, and the local ignored `log/devlog.md` for detailed evidence only when needed.

## User preferences to preserve

Work autonomously on actual bugs and missing Notepad++ behavior, with parallel agents on separate worktrees and separate Cargo target directories. Review and integrate completed work back into main. Commit each completed feature/fix through the local `$commit` skill; no push was authorized.

The app stays native Rust/GPUI. Keep Markdown Reader, rich documents and CSV views. English spelling and grammar checking stays local and ignores URLs, email, paths, identifiers and code. The original completion complaint was a blue suggestion box covering the typed words; native placement, sizing and interaction still need direct verification despite many earlier completion fixes.

Use concise progress updates. Ask only for missing information that matters or destructive actions without authorization. Preserve unsaved application documents and user files. Dev and installed builds share config/session state; single-instance forwarding can make a new build appear to launch while an older copy handles it.

## Completed work

| Commit | Result |
| --- | --- |
| `131da7c` | Trim Leading Space respects selected physical lines. |
| `c54108d` | Offline English spelling/grammar, wavy underlines, suggestions, ignore and macro-recordable corrections. |
| `63c6160` | Trim Leading and Trailing Space respects selected physical lines. |
| `61b33b4` | Save As rejects old replies after close/restore or accepted window closure. |
| `fbbe548` | Trim Trailing Space respects selection, handles CR-only lines and refuses rectangles. |
| `0dc03cb` | Explicit Find Next/Previous retains its named direction; dialog operations honor checkbox and Shift reversal once. |
| `03bca4a` | Seven missing Blank Operations record stable macro events and replay with one Undo. |
| `c42ee98` | Spelling/grammar toggles immediately republish menu checkmarks. |
| `b313975` | Save-before-close and dirty confirmations bind replies to the original tab incarnation and live window. |
| `647b0ff` | Save All includes dirty untitled files, sequential pickers, per-file Cancel continuation and Always Yes confirmation. |

Other recent completion work includes `5629a11` for faithful recorded completion replay, `e36cad0` for dismissing menus whose labels cannot fit, and `36e042d` for actual page navigation across zoom levels. Inspect history and `docs/api-completion.md` before revisiting that area.

## Resumed work

- New search macro events persist query/replacement/options and replay through the existing handlers. Regex playback waits for background replacement, guards source ownership and groups uninterrupted edits into one Undo. Old parameterless search and Save events remain readable but refuse the whole macro before edits. Query and replacement recording each cap at 1 MiB before cloning; oversized live operations still work. See [scope](search-macros.md).
- Copy All Filenames / Copy All File Paths now have Edit menu, action, CATALOG and stable macro routes. Current-window visual tab order, duplicates, Unicode and untitled names are retained; CRLF rows match the oracle. Empty tab lists leave the clipboard unchanged.
- Copy Bookmarked Lines now has Search menu, action, CATALOG and saved macro routes. It copies each physical line once, preserving per-buffer EOL and unterminated EOF. Empty output clears the clipboard. Large manual work is cancellable in background; large recording/playback is explicitly refused. Source/focus/activity/incarnation guards reject stale results. Editor Copy, Copy All, MarkCopy and all three painted tab-popup copy rows supersede pending work. Other modal/folder/external clipboard writers were not audited. See [limits](bookmark-copy.md).
- Dense bookmarks in a single physical line previously repeated backward scanning. A real helper counter failed at 4,096 lookups versus one; the corrected caller scans once. Cancellation, EOF, CRLF, invalid offsets and unsorted helper inputs retain their behavior.
- The existing tab-popup Move to Trash / Recycle Bin caller deleted without confirmation. It now shows a parented OK/Cancel dialog with path and unsaved warning, binds replies to the original document/window and refuses stale edits, Edit/Undo ABA, restore, path/metadata changes and unfinished drafts. Last-tab deletion first verifies recovery; if the subsequent recovery write fails, the exact original Tab/history/native payload remains open as dirty untitled for Save As. Tests rename temporary fixture files through a controlled backend and never use the system trash.
- Completion adds an actual Workspace layout sweep: 32 combinations of rem/window/caret position, 24 placed and eight dismissed. Roomy positive cases prevent a trivial always-dismiss pass. This still does not resolve the original native visual complaint.
- The exhaustive static inventory contains 581 appearances, 481 unique IDs and 45 separators, including both Language layouts. Current classifications: 165 routed, eight partial, two action-only, four alternate-only, one status-only, one stub and 300 missing mapped main-menu routes. `routed` proves wiring only. Read [inventory and next work](notepadpp-parity-inventory.md); regenerate with `tools/parity_inventory/generate.py` and run its eleven tests.

## Latest validation

Final combined Windows checks passed with clang-cl, without source changes during the checks:

- `cargo build`: exit 0, 28.99 s wall.
- `cargo test -p notepad-studio`: 2,471 unit plus three integration passed; 24 ignored. Compile 8.19 s, unit runtime 53.16 s, total 61.52 s wall.
- `cargo clippy -p notepad-studio --all-targets`: exit 0, 23.44 s Cargo / 23.56 s wall. Inherited 195 app / 209 test warnings, 191 duplicates; no new message/file diagnostics or new-file warnings. The tree is not warning-free.

Logs: `log/resumed-final-main-{build,tests,clippy}.txt`, times and diagnostic comparison beside them. No plugin ABI/host or vendored code changed. `git diff --check` passes. The root ledger records seven met gates and one abandoned native-verification gate; leaf ledgers record 29 met gates.

Focused parent evidence: 17 search macro tests, 14 bookmark/clipboard tests and 13 trash tests; the first two groups each exclude one manual timing fixture. Test names overlap groups, so counts are not additive. The inventory generator `--check` and eleven parser/count/identity tests pass on the final source. Parent logs use `log/search-macros-main-focused-v2.txt`, `log/bookmark-dense-main-focused.txt`, `log/trash-main-focused.txt` and `log/resumed-inventory-final-{check,tests}-v2.txt`.

Final report review exposed a scanner defect: `fn build() -> Menu {` was treated as a Menu literal, attaching unrelated submenu names to sibling items. Root added a genuine failing nested/sibling/computed-name regression, excluded return-type blocks, read only direct Menu name fields, and regenerated both outputs. Eleven tests pass in 0.022 s; File New/Close and Close Multiple plus Edit clipboard paths were spotchecked. Trailing generated whitespace was also corrected. Rust source and its successful checks are unchanged.

Optimized synthetic evidence is scoped to the measured callers/helpers:

- Search recording/replay used a 12,582,973-byte source with the default release profile. Direct dispatch/capture/apply/wall: 0.053/0.011/0.124/11.664 ms; macro: 0.054/0.002/0.066/10.457 ms. Escape took 0.008 ms and left no transaction open.
- The frozen pre-dense bookmark caller used the default release profile: 12,000,600 bytes/600 bookmarks, completion dispatch/worker/total 4.0024/5.6144/15.0824 ms. Escape, stale-revision and ordinary-Copy controls passed. A 12,584,724-byte Unicode physical row took 24.7862 ms total; TestWindow dispatch included completion. Compilation took 17m42s. Evidence is `../notepad-studio-agents/bookmark-copy-next/log/bookmark-release-default.txt`, SHA256 `1CA672EED41A1BA2B596510FC669DD0C81BDBBB096704C1EBED7659B2CF825F0`.
- Final main dense helper, 11,062,145 bytes/4,096 marks: 1.6592213 s before versus 425.6 microseconds after, identical 262,145-byte output. Parent `log/build_bookmark_dense_bench.py` extracts unchanged helper bodies; source hashes are in `log/bookmark_dense_bench_source.json`. Compile `log/bookmark_dense_bench.rs` with `rustc --edition=2021 -C opt-level=3 -C lto=thin -C codegen-units=1 -C panic=abort -C strip=symbols`, dependency/extern paths pointing at the optimized Ropey cache in `../notepad-studio-agents/regex-replacement/target/agent-regex-replacement/release/deps`. This standalone fixture links no GPUI and is not a whole-app speedup measurement.

Native Computer Use still fails with `Computer Use native pipe is unavailable: failed to connect native pipe: The system cannot find the file specified. (os error 2)`, independently reproduced by root and completion worker. Native CUA APIs are disabled. No native app launch/input occurred during this session. GPUI TestWindow tests do not prove native visuals/input delivery. Existing successful macos-14 CI run [38163317470](https://github.com/alesloa/notepad-studio/actions/runs/38163317470) covers baseline `beb5679` only; new changes lack Mac compilation/runtime evidence. No push was authorized or performed. Root gate G7 records the missing prerequisites; full parity/native verification remains open.

## Earlier validation

Final combined Windows checks passed on the source now committed in main:

- `cargo build`: 8.13s, after the initial combined build passed in 32.75s.
- `cargo test -p notepad-studio`: 2,418 unit plus 3 integration passed, 22 ignored; compile 0.49s, unit runtime 52.68s. The preceding focused test build took 52.50s.
- `cargo clippy -p notepad-studio --all-targets`: exit 0 in 25.69s. Inherited counts were 195 app / 209 test warnings, 191 duplicates. There were no new diagnostics in changed code; do not describe the repository as warning-free.
- Actual main action controls: 7 close/save tests passed in 0.06s, 18 Save All tests in 0.08s, and 14 tab-menu tests in 0.15s. Spelling menu tests previously passed 3 groups / 8 toggles, followed by full gates.

Local logs are `log/save-final-main-{build,tests,clippy}-v2.txt`, `log/close-save-main-focused.txt`, `log/save-all-main-focused.txt`, and `log/save-final-main-popup-fixture-v2.txt`. Logs are ignored and available only in this workspace.

The first combined full run had one existing tab-menu fixture failure. It falsely marked a live window closed to suppress persistence; the new guard correctly refused Close Multiple. Root replaced that flag with the existing session snapshot sink. The final full run passed. This was fixture repair, not a new product defect or crash reproduction.

Native Computer Use failed after restart with: `Computer Use native pipe is unavailable: failed to connect native pipe: The system cannot find the file specified. (os error 2)`. GPUI TestWindow tests exercise real registered actions, layout and clicks, but do not prove native Windows visuals/input delivery. Mac compilation and runtime were not checked. Retry available native tooling in the fresh session; keep verification claims explicit if it is still unavailable.

## Details to load for the relevant task

**Proofreading.** Start with `docs/proofreading.md`, `src/proofreading.rs`, `src/editor_proofreading.rs`, and `src/workspace/proofreading.rs`. Exact Apache-2.0 `harper-core = 0.33.0`, curated American English, no document network calls. Normal Text and Markdown Source View only; Reader/rich/CSV are preserved and excluded. Source cap 1 MiB, chunks 2,048 scalars/8 KiB, 512 engine-emitted diagnostics and three suggestions. The cap does not guarantee the earliest 512 diagnostics by offset. Grammar uses complete bounded paragraphs. Background scheduling debounces 250ms, allows one running job/latest pending source, and rejects stale incarnation/revision/activity/policy results. Preserve correction read-only, IME and transaction guards. Licenses and dependency provenance are in `NOTICE`; dictionary historical origin was not exhaustively attributed beyond upstream's published Apache license.

**Save ownership.** `src/io.rs`, `src/io/save_as_dialog.rs`, and `src/workspace/tabs.rs` now use UUID plus `Arc` incarnation. Restored tabs retain UUID and may retain content revision but are different runtime documents. Original live edits remain saveable. Five genuine close/save baseline failures were fixed; the leaf's 99 close compatibility tests passed. Late Don't Save had mutated retained documents, while the existing closed-window persistence guard prevented more recovery writes. Do not claim demonstrated recovery loss. A late failed helper can still set status on a retained closed entity; file/document/recovery effects are guarded.

**Save All.** `src/io/save_all.rs` captures dirty document UUID/incarnation/path in tab order. Cancel/error continues to later documents; clean bytes/mtime stay untouched. CSV refusal stops the batch without losing the draft. Always Yes disables later confirmations through existing settings persistence. `rfd`'s existing `common-controls-v6` feature is enabled; no version/default-feature or lockfile change. The compiled Windows leaf PE contained the Common Controls v6 manifest and `TaskDialogIndirect` import. That is static evidence, not native appearance verification. Scope remains per-window, existing unsupported native-format writer limitations remain, and independent concurrent Save All invocations are identity-safe but not serialized.

Frozen local manifests are `log/close-save-identity-final-manifest.json`, `log/save-all-candidate-manifest.json`, and `log/proofreading-menu-candidate-final.json`. They identify original leaf source/evidence hashes. Main preserves later integration changes: the picker lint annotation, combined IO module registrations and Root's extra tab-popup fixture repair.

## Work still open

Prioritize crash/data-loss reproductions over parity additions. Assign independent tasks to real agents when slots are available; do not interpret old displayed agent names as running workers.

1. Native Windows QA of completion near the caret, zoom/window edges, key navigation, IME, proofreading squiggles/right-click corrections, and custom Save All/close dialog buttons. Check original UX complaint directly. Verify Mac builds/native menus when a Mac or existing CI evidence is available. Do not substitute a browser for the native app.
2. Extend macro support deliberately: legacy Save still lacks a recorded target, and old parameterless search events cannot recover missing parameters. Large Copy Bookmarked Lines recording/playback needs background continuation; preserve the current refusal before clipboard edits or following Paste. New search parameter capture is complete within its documented Source/Raw View scope.
3. Select confirmed gaps from the exhaustive static inventory: six sort variants, remaining bulk bookmark edits, missing main-menu File Rename/Delete routes, printing, encoding semantics and split views. The existing deletion popup is now guarded; rename still needs an ownership audit before exposing another route. Printing is an actual popup stub and absent main-menu route. Avoid adding labels that imply an implementation.
4. Remaining completion/IME/focus and large-edit performance work is listed in README status and recent records. Full-buffer Blank Operations allocation/performance has no new optimized measurement. Existing recorded optimized campaigns cover specific rich search/drawing/startup workloads only; do not generalize their numbers to whole-app performance.
5. Follow-up save policy audits may include concurrent batches and unsupported legacy `.doc` writes. A resumed source audit corrected the previous claim: `save_target_open_elsewhere` in `src/workspace/open_existing.rs:43` scans the current workspace. Save All and standalone Save As both use per-window checks; no cross-window defect was reproduced. Notepad++ checks both views within one instance in `NppIO.cpp:2079`.

For each chosen batch, establish intended behavior, reproduce any defect through an actual caller, make the smallest original Rust fix, and run required gates before committing. Use a live-window persistence sink in UI test fixtures rather than setting `window_closed` unless accepted closure is the behavior under test. Reuse available idle build caches deliberately, keep source stable while compiling, and format only code you write.

Checkpoint after integration with concrete commits, test evidence, idle workers and an updated handoff. Full parity requires a verified inventory; the successful checks above do not establish it.
