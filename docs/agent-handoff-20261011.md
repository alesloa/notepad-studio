# Notepad Studio handoff, 2026-10-11

Resume the user's ongoing Notepad++ parity and correctness work. This is a tested checkpoint, not a claim of complete parity. The previous session finished its assigned batches and stopped for a fresh agent session at the user's request.

## Starting state

- Repository: `C:\Users\Ale\Documents\GitHub\notepad-studio`, branch `main`.
- Implementation checkpoint: `647b0ff6edb082770533144f918972573db7483b`. A documentation commit containing this handoff follows it. Check current Git state before working.
- Notepad++ behavior oracle: `C:\Users\Ale\Documents\GitHub\notepad-plus-plus-master`. Read behavior, menu order and shortcuts; write Rust independently. Apache-2.0 rules and licensing are in `AGENTS.md`.
- Windows PowerShell. The latest checks used `CC` and `CXX` set to `C:\Program Files\LLVM\bin\clang-cl.exe`.
- The three active workers completed. Their reviewed patches were personally integrated on main and committed. Old worktrees under `..\notepad-studio-agents` remain for local evidence; reapplying them would duplicate shipped changes.
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

## Latest validation

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
2. Search macro parameter capture/replay. `src/workspace/macros.rs::unsupported_macro_operation` still refuses legacy parameterless FindNext, FindPrev, ReplaceCurrent, ReplaceAll and Save events because required parameters were not recorded. Introduce recordable behavior with backward-compatible persisted names and actual save/reload/replay tests. Do not silently skip unsupported events or change search semantics.
3. Build a concrete menu/action parity inventory against `PowerEditor/src/Notepad_plus.rc`, `Parameters.cpp`, `NppCommands.cpp` and current menus/catalog. An earlier limited scan found a tab-context-menu Print stub at `src/ui/tab_context_menu.rs` and an optional cloud-sync stub in `src/ui/preferences/cloud_link.rs`. Audit the real routes before calling these whole-product feature gaps; the inventory is not exhaustive.
4. Remaining completion/IME/focus and large-edit performance work is listed in README status and recent records. Full-buffer Blank Operations allocation/performance has no new optimized measurement. Existing recorded optimized campaigns cover specific rich search/drawing/startup workloads only; do not generalize their numbers to whole-app performance.
5. Follow-up save policy audits may include cross-window duplicate targets and concurrent batches. Local duplicate-target checks differ from standalone Save As's cross-window check; this was noted from source, not reproduced as a defect. Existing legacy `.doc` Save All limitations were not redesigned.

For each chosen batch, establish intended behavior, reproduce any defect through an actual caller, make the smallest original Rust fix, and run required gates before committing. Use a live-window persistence sink in UI test fixtures rather than setting `window_closed` unless accepted closure is the behavior under test. Reuse available idle build caches deliberately, keep source stable while compiling, and format only code you write.

Checkpoint after integration with concrete commits, test evidence, idle workers and an updated handoff. Full parity requires a verified inventory; the successful checks above do not establish it.
