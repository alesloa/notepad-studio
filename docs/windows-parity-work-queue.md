# Windows parity work queue

The user's goal is to finish Windows Notepad++ parity. Mac testing is deferred. The static inventory and behavior ledger cover different questions: a connected menu item still needs a behavior audit.

Current integrated batch, with combined Windows build/tests/Clippy and default-release large-file controls passing. The [checkpoint record](windows-parity-validation-20261011.md) gives the results and their limits:

- File routes: Rename, Delete, cmd, PowerShell, folder as workspace and Save a Copy As, including optional opening of the copy.
- Bookmark edits: Cut, Paste, Remove Bookmarked Lines, Remove Non-Bookmarked Lines and Inverse Bookmark. This leaf owns arbitrary prepared line edits and mixed prepared/legacy macro history.
- Line sorts: ascending/descending case-insensitive lexical, UTF-16 length and Windows locale sorting, rectangle keys, worker preparation and macro continuation. The user delegated the Windows NLS seam decision; the parent approved the bounded two-call bridge described in `windows-nls-seam-proposal.md`.

Next leaves:

- Save durability audit: reproduce or disprove interrupted writes after the existing rescue-copy failure falls back to memory. Local checks are in ignored `gates/windows-save-durability.md`. Existing error-rollback tests do not prove interruption recovery.
- Column safety: reproduce or disprove oversized virtual-space and stale rectangle-row panic paths, then refuse invalid plans before any partial edit. Extend the mutating bookmark commands with exact rectangle remapping; the current leaf refuses that context while allowing metadata-only Inverse. Local checks are in `gates/windows-column-safety.md`.
- Physical lines: establish whether Ropey's Unicode line splitting differs from the oracle's default CR/LF policy in real callers. Local checks are in `gates/windows-physical-lines.md`. This remains an investigation, not a verified product defect.
- Tools: all twelve hash commands. Contract and gates are in `gates/windows-hash-tools.md`.
- Brace navigation and bracket highlighting: the two missing Search commands share a cancellable matcher and cached paint result. Baseline painting scans the whole Rope for a partner. Contract and gates are in `gates/windows-brace-navigation.md`; exact lexer-style distinctions remain part of the language audit.
- Existing sort/reverse/shuffle/dedup callers: migrate their synchronous preparation to the guarded worker pipeline; establish UTF-16 lexical and numeric error behavior, selection retention, rectangle keys and macro recording. At baseline, `src/edit_ops/line_ops.rs:406-438` copies and transforms the block on the UI thread, then collapses the selection. The baseline `MacroEvent` enum has no sort/reverse/shuffle/dedup events.
- Search: independent mark styles, brace/change navigation, result navigation, incremental/volatile find and character-range search.
- File/session/tab state: pinned tabs, session exchange, printing, and close rules. Audit explicit renamed-label precedence over the use-first-line title preference; current save-picker suggestions honor the stored label but the title preference can mask it.
- View: split panes, folding, character/EOL controls, monitoring and document panels.
- Encoding/languages: reinterpretation versus conversion, Windows ANSI/code pages, and missing languages.
- Remaining Settings, Macro, Run, Plugins, Window and Help behavior from the ledger.

Constraints still open:

- Native Windows UI verification: `sky.list_apps` again failed on 2026-10-11 with native pipe unavailable, os error 2. Source work continues; no visual pass is claimed.
- Exact locale sorting: the bounded Windows NLS bridge and caller tests are integrated; combined automated checks and optimized controls pass. Basic mixed-EOL sorting deliberately preserves physical endings rather than following the oracle's configured-EOL splitting. Mixed macro merging refuses full legacy timelines above 128 MiB while retaining separate Undo entries. See `windows-line-sorts.md` and `bookmark-edits.md`.

No push, release publication, Linux/WSL/VM execution or Mac test run is authorized for this pass. Keep existing platform source compiling as far as the available Windows build can establish.

Acceptance ledgers under `gates/` are local ignored work files at the user's request. Keep essential behavior and verification evidence in committed documents.
