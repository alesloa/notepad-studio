# Windows File commands

This change completes six File menu routes against the local Notepad++ behavior
oracle. The Rust implementation is original. Oracle code was read for menus,
defaults, dialog choices, and file effects; it was not translated or copied.

| Action ID | Notepad++ ID | Menu route | Workspace handler |
| --- | --- | --- | --- |
| `FileOpenContainingFolderCmd` | `IDM_FILE_OPEN_CMD` (41020) | File / Open Containing Folder / cmd | `file_open_containing_folder_cmd` |
| `FileOpenContainingFolderPowerShell` | `IDM_FILE_OPEN_POWERSHELL` (41027) | File / Open Containing Folder / PowerShell | `file_open_containing_folder_powershell` |
| `FileContainingFolderAsWorkspace` | `IDM_FILE_CONTAININGFOLDERASWORKSPACE` (41025) | File / Open Containing Folder / Folder as Workspace | `file_containing_folder_as_workspace` |
| `FileSaveCopyAs` | `IDM_FILE_SAVECOPYAS` (41015) | File / Save a Copy As | `save_copy_as` |
| `FileRename` | `IDM_FILE_RENAME` (41017) | File / Rename | `file_rename` |
| `FileMoveToTrash` | `IDM_FILE_DELETE` (41016) | File / Move to Recycle Bin | `file_move_to_trash` |

All six actions are declared in `src/actions/file.rs`, present in the File menu,
registered in `src/workspace/render.rs`, and rebindable in `CATALOG`. They have no
default keyboard chords on Windows or macOS. Notepad++ does not record these file
commands in macros; no macro event or replay arm is added. Existing popup basename
Rename and guarded Move to Recycle Bin remain the alternate tab surfaces.

Behavior sources in `PowerEditor/src/`:

- `Notepad_plus.rc:449-475`: containing-folder submenu, item order and labels.
- `menuCmdID.h:20-51`: numeric command identifiers.
- `Parameters.cpp:110-128`: no-default shortcut entries; containing Folder as
  Workspace has no entry and is confirmed by `Notepad_plus.rc:455`.
- `NppCommands.cpp:241-273,399-405,453-454`: shell, workspace, Rename, recycle,
  and Save Copy dispatch. `NppCommands.cpp:4413-4430` excludes them from recording.
- `NppIO.cpp:1994-2094`: Save Copy preserves the original and optionally opens
  the result. `NppIO.cpp:2013` uses the current document name in the picker.
- `NppIO.cpp:2097-2265`: existing-file Rename uses a full destination picker;
  untitled or missing files use a document-label prompt. `NppConstants.h:358`
  and `NppIO.cpp:2296` limit that label to 63 UTF-16 units plus its terminator.
- `ScintillaComponent/Buffer.cpp:1201-1226`: destination replacement and
  cross-volume Rename. `RunDlg/RunDlg.cpp:207-251`: executable environment
  expansion and working directory.

## Ownership and disk effects

Existing-file Rename captures the tab UUID, its Arc incarnation, original path,
window, and request nonce before the picker. Editing or reordering the requester
does not invalidate the rename; closing/restoring it, changing its path, or issuing
a later picker does. Rename keeps dirty state, undo history, and recovery identity.
Starting a later native or popup Rename cancels an earlier background move before
its staging result can install files, including when the newer picker stays open.
Untitled Rename changes only the label and rejects an invalid or overlong name
without truncating Unicode. Save, Save Copy, and Save-close suggest that label;
recovered native documents retain their format extension, including `.docx` for
Save As from legacy `.doc`.

Save Copy captures the rope by shared ownership and scalar metadata. Native
original bytes remain in an Arc. Edited native content is captured in UI turns of
at most 64 KiB and 128 structural items, yielding between turns. The snapshot
omits undo/redo stacks, render state, and the original edit log; the existing
native writers need its emptiness and the edited AST. Container writers preserve
headers, footers, and other untouched parts through the original archive bytes.
All input-sized snapshot growth uses fallible reservations and a checked,
conservative 512 MiB cumulative allocation budget. Refusal leaves both files
unchanged and directs the user to Save As.

Encoding, line endings, native serialization, staging, and copy commit run on the
background executor. Progress and Cancel remain available while preparation is
running. The existing writer restores old destination bytes after a partial
write. If restoration also fails, the error names the surviving safety copy even
when cancellation or a later source edit happened. Partial snapshots and discarded
stages are disposed of on the background executor, including stale callbacks and
failed entity updates.

Before disk authorization, Save Copy rejects a changed source revision, edit/undo
ABA, path, encoding, EOL, read-only state, save point, incarnation, window, or
request. Both Rename and Copy reject a destination already open under another
path or hard link, including another workspace window. The currently updating
window is excluded before reading window roots to avoid a gpui double borrow.
The check is repeated after background staging.

A successful authorized copy commit is a disk result. A source change during
that commit prevents optional opening and later MRU/directory effects; it does
not undo the completed copy. The original path, dirty state, undo history, and
recovery ownership remain unchanged. An app-owned Yes/No/Cancel prompt supplies
the open-copy choice before the native picker because rfd cannot embed the
Notepad++ checkbox.

Cross-volume Rename streams in 1 MiB chunks into an exclusively created sibling
scratch file. Cancel or stale ownership before installation leaves source and
destination unchanged. Failed source removal restores the prior destination;
failed restoration retains and names its previous copy. No user trash, settings,
registry, session, or file data is mutated by the tests: all fixtures are temporary,
external command launches and trash use controlled seams, and persisted-state
sinks are scoped.

Windows cmd uses COMSPEC with no arguments and the containing folder as cwd.
PowerShell uses the registered HKLM `SOFTWARE\Microsoft\PowerShell\1\ShellIds\Microsoft.PowerShell`
`Path` in the default process registry view, read-only, with no shell interpolation
or reg.exe parsing. Missing, invalid, or oversized registration yields an
actionable error. The pinned Windows-only `winreg = "=0.55.0"` dependency was
already in the lockfile; only the root dependency list changes. Its MIT license is
retained under `licenses/winreg-0.55.0/` and attributed in NOTICE. The windows-rs
license/NOTICE entry accompanies the parallel sort implementation as integration
paperwork; this leaf does not change that dependency.

## Verification record

Local command logs and the leaf ledger live under `gates/` and are excluded from
the source patch. `g1-red.log` records the original 2 passing / 2 failing menu
contract tests: Save Copy was missing and the containing submenu had one item
instead of five. `g3-current-window-red.log` records the real-caller gpui failure
before the current-window exclusion: "attempted to read a window that is already
on the stack." Final positive evidence is recorded below after the frozen checks.

The four passes covered route implementation, oracle and ownership reread,
background I/O and rollback defect tests, then aliases, registry configuration,
bounded capture, and renamed filename suggestions. Native Windows dialog layout,
interactive terminal launching, and rendered input inspection remain pending
because this session has no enabled native control bridge. GPUI action tests prove
the caller and state behavior; they do not replace that visual inspection.

One shared title policy remains for the session/label audit: when
`untitled_uses_first_line` is enabled, `Buffer::display_short_name` currently lets
the first line take precedence over a manually renamed `new_label`. Notepad++'s
explicit renamed name should win. This leaf leaves that shared preference policy
unchanged; the stored name still drives Save, Save Copy, and Save-close suggestions.
Default-preference label display and the three actual picker callers are covered
by `actual_untitled_and_missing_path_rename_changes_only_label_and_keeps_recovery_and_history`,
`actual_save_picker_suggests_current_untitled_label`,
`actual_copy_picker_suggests_current_name_and_cancel_preserves_original`, and
`close_save_picker_suggests_current_untitled_label_and_cancel_keeps_dirty_tab`.

Frozen Windows checks on 2026-10-11 used detached base
`d01099f01dd464607cdbe62736789192ba8787eb`, `clang-cl` from
`C:/Program Files/LLVM/bin/clang-cl.exe` for both CC and CXX, and the assigned
debug cache `../rich-match-blocks/target/agent-rich-match-blocks`:

| Check | Result |
| --- | --- |
| `cargo test -p notepad-studio` | 2,518 unit tests and 3 integration tests passed; 0 failed; 26 ignored. Unit run: 46.92 s. |
| `cargo clippy -p notepad-studio --all-targets` | Exit 0, 7.45 s; 195 app warnings and 209 test warnings, including 191 duplicates. No diagnostics in the seven new production modules. |
| `cargo build` | Exit 0, 13.08 s; 11 pre-existing warnings. |
| Final source scan | No high-severity or bug-class findings in the seven new production modules. |

The earlier Clippy review reported 197 app and 211 test warnings. Its two new
sites were corrected before freezing; existing diagnostics remain. The final
logs are `g5-final-fulltest.log`, `g5-final-clippy.log`, `g5-final-build.log`, and
`unslop-source-before-checks.log` under the local `gates/` directory.

The exact source patch is `gates/source-before-checks.patch`, 181,673 bytes,
SHA256 `c70f9cc3a9bc86752fed1b07e29fb87aedea940f7ce439c203e2fb4366924541`.
`source-before-checks-sha256.json` identifies all 30 changed files, including new
source and both license files. `final-debug-provenance.json` records zero hash
mismatches after the checks. Later changes to this verification document are a
prose-only follow-up; the checked Rust files remain frozen. The source patch
excludes local `gates/` and `log/` evidence and does not stage or commit anything.

Focused checks used the same final test executable and temporary fixtures:

| Evidence log | Deciding result |
| --- | --- |
| `g1-final-contracts.log` | 4 menu/catalog/default contracts passed. |
| `g3-final-file-callers.log` | 16 File action callers passed, including held-stage supersession by both native and popup Rename. |
| `g2-g4-final-copy-callers.log` | 21 Save Copy callers passed; 1 optimized timing test remained ignored. |
| `g3-final-move-errors.log` | 5 staged Rename error/control tests passed; 1 optimized timing test remained ignored. |
| `g2-final-restore-error.log` | Cancellation plus failed restoration reported the retained original's path and verified its contents, with and without a later source edit. |
| `g4-final-snapshot.log` | Bounded native capture preserved Unicode links, raw text, and structure, retained the original byte Arc, and omitted histories. |
| `g4-final-staged-writer.log` | Interrupted partial writes restored an existing destination and removed a newly created destination. |
| `g4-final-save-names.log`, `g4-final-close-name.log` | Label/native extension policy and the actual Save-close filename suggestion passed. |

The assigned optimized cache was
`../quit-async/target/agent-quit-async-release`. `cargo test -p notepad-studio
--release --no-run` passed in 14m58s with the shipping `opt-level=3`, thin LTO,
and one codegen unit. The frozen test executable SHA256 is
`9cdd493cf301cb207e626d97aad858beba15cf66d9912eec0e7cba8e765298c6`.
The source audit still found zero drift in all 22 Rust files. No source or build
flags changed for measurement.

The parent reserved a quiet File, Bookmark, Sort sequence. Fresh process
inventories contained no Cargo, rustc, clang-cl, or linker activity before,
between, or after the File controls. Three sequential processes ran each exact
control against temporary files, with captured timing markers and one-test pass
summaries. All 12 invocations passed. Plain Copy read 13,631,488 UTF-8 bytes and
wrote 14,680,064 faithful CRLF bytes; the edited-native fixture had a 13,631,488-byte
AST run. The figures below are milliseconds:

| Measured scope | Median | Range |
| --- | ---: | ---: |
| Existing synchronous Copy writer baseline | 21.204 | 18.862–24.621 |
| Plain Copy actual caller, from picker reply through completion | 47.446 | 39.758–49.026 |
| Cross-volume 14,680,064-byte staged stream preparation | 16.500 | 15.585–17.074 |
| Cross-volume preparation plus destination installation/source removal | 17.365 | 17.052–18.062 |
| Edited-native first bounded caller capture turn | 0.393 | 0.383–0.467 |
| Edited-native request through caller-visible Escape cancellation | 0.708 | 0.661–0.865 |

The plain baseline measures the existing writer, including flattening, encoding,
and disk writing. The background caller total includes immutable capture, worker
preparation and commit, authorization, and caller publication. Those latter
phases have no independent clocks here; the comparison demonstrates relocation
of work and correct bytes, not an end-to-end speedup. The cross-volume timing
calls the same staged stream and installation functions directly; actual action
dispatch, progress, cancellation, and stale ownership have separate caller tests.
There was no prior implemented cross-volume File route to time as a baseline.

The native first turn includes the picker reply and up to 64 KiB of scalar
capture in the GPUI harness; it excludes full snapshot joining and native
serialization. The cancellation number ends after Escape and runnable state
publication. GPUI's deterministic clock does not advance the pending 1 ms yield
timer there, so this number does not measure completed partial-snapshot disposal.
`BackgroundDrop` dispatches that disposal to the background executor when the
task resumes or ends. Disposal has no separate measured duration; stage cleanup
is checked by the controlled cancellation/stale/error tests. These harness
figures do not measure native dialog rendering or live desktop input latency.

The large plain caller control separately passed Cancel-chip, Escape, edit, and
close/restore cases in all three optimized processes. Each case left both disk
files unchanged, removed owned staging files, and left recent files untouched.
The native control preserved the original dirty state and edit history and
created no destination or stage. Logs are `g5-release-{copy,move,native-cancel,
plain-controls}-{1,2,3}.log`; `g5-release-timing-provenance.json` records binary,
source identity, profile, and idle inventories. Initial empty output from direct
PowerShell redirection was discarded; the deciding invocations use a captured
pipeline and require a pass summary plus each applicable timing marker.

Both assigned caches are idle and retained. The local five-gate ledger records
the completed code and performance checks; the native visual checks and shared
title-preference policy above remain explicit follow-up work.
