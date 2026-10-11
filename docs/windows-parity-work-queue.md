# Windows parity work queue

The user's goal is to finish Windows Notepad++ parity. Mac testing is deferred. The static inventory and behavior ledger cover different questions: a connected menu item still needs a behavior audit.

Current integrated batch, with combined Windows build/tests/Clippy and default-release large-file controls passing. The [checkpoint record](windows-parity-validation-20261011.md) gives the results and their limits:

- File routes: Rename, Delete, cmd, PowerShell, folder as workspace and Save a Copy As, including optional opening of the copy.
- Bookmark edits: Cut, Paste, Remove Bookmarked Lines, Remove Non-Bookmarked Lines and Inverse Bookmark. This leaf owns arbitrary prepared line edits and mixed prepared/legacy macro history.
- Line sorts: ascending/descending case-insensitive lexical, UTF-16 length and Windows locale sorting, rectangle keys, worker preparation and macro continuation. The user delegated the Windows NLS seam decision; the parent approved the bounded two-call bridge described in `windows-nls-seam-proposal.md`.

The safety follow-up is committed as `10d4723` (synced save rescue) and `b42c889`
(column preflight and physical line boundaries). Combined build, 2,652 unit plus
three integration tests, Clippy with zero new diagnostics, inventory checks and
scoped release controls pass. See [safety validation](windows-safety-validation-20261011.md)
for exact proof and limits. Gates/logs remain local and ignored.

Active next leaves:

- Bounded save restoration: an actual 18,874,368-byte restoration callback violates the proposed 65,536-byte budget; injected read OOM retains exact rescue/errors. The reviewed fixed 64 KiB same-handle pump is in implementation, with no allocator-abort claim. Local checks are in `gates/windows-bounded-restore.md`. Normal Save/Save As/Save All foreground work has a separate frozen background-ownership contract; no implementation yet.
- Tools: all twelve hash commands. Source-only G1 preparation audits editor-selection bytes, EOL/BOM, strict Windows-1252, rectangles, focus and clipboard supersession. Contract and gates are in `gates/windows-hash-tools.md`; [oracle notes](windows-hash-tools-oracle-20261011.md) preserve the behavior audit.
- Existing sort/reverse/shuffle/dedup callers: source-only G1 preparation at `b42c889` for guarded worker preparation, UTF-16 lexical and numeric error behavior, selection retention, rectangle keys and macro recording. Existing `src/edit_ops/line_ops.rs` flattens/transforms on the UI thread and collapses selection; existing macro events omit these commands. Actual defects require caller evidence before a policy/fix. Local checks are in `gates/windows-legacy-lines.md`.

Then continue:

- Column parity: temporary safety caps are integrated. Large column preparation, ragged word deletion, external clipboard semantics, active rectangle IME and mutating bookmark rectangles remain open. Metadata-only Inverse already works in rectangles. See [column limits](column-safety.md).
- Brace navigation and bracket highlighting: the two missing Search commands share a cancellable matcher and cached paint result. Baseline painting scans the whole Rope for a partner. Contract and gates are in `gates/windows-brace-navigation.md`; exact lexer-style distinctions remain part of the language audit.
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
