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

- Normal Save/Save As/Save All: bounded recovery is integrated, with [combined Windows checks](windows-bounded-restore-validation-20261011.md) and exact optimized helper controls passing. The next task is the separate frozen background capture/publication and close/Exit settlement contract. Preserve the rescue/error behavior while moving foreground work off the UI thread.
- Tools: all twelve hash commands are implemented in the isolated candidate. Root verified the final 54-file freeze and 60 hash plus three clipboard ownership controls; required build/full tests/Clippy are authorized and pending. Optimized/native checks and main composition remain open. Selection uses UTF8 editor bytes without BOM and strict Windows-1252; raw LF/mixed native Paste remains an audit gap. The SHA-512 file title deliberately follows the accurate English localization schema rather than the misleading default SHA-1 caption. [Oracle notes](windows-hash-tools-oracle-20261011.md) record the audit boundary; earlier fixture failures are preserved separately.
- Existing sort/reverse/shuffle/dedup callers: the isolated candidate passes build, 2,771 unit plus three integration tests/32 ignored and Clippy with no new diagnostics. Both ignored >10 MiB registered-caller controls also pass in debug. Root verified the 23-file freeze; default release test compilation is authorized, with optimized runs held for a quiet slot and effective profile provenance required. Main composition and native verification remain pending. Preserve UTF16 keys, whole-request numeric refusal, rectangle/EOL and macro policies; no native CRT conversion probe is claimed.
- Brace navigation and bracket highlighting: both Search routes, saved macro continuation and shared cancellable matching/cache are integrated. Eighteen focused tests, main build, 2,684 unit plus three integration tests/30 ignored, Clippy with zero new diagnostics and inventory checks pass. The [brace validation](windows-brace-validation-20261011.md) records unchanged-paint work and scoped optimized helper timing. Exact lexer-style distinctions, styled-tail handling, Find-focused behavior and native visual/input checks remain open.

Then continue:

- Column parity: temporary safety caps are integrated. Large column preparation, ragged word deletion, external clipboard semantics, active rectangle IME and mutating bookmark rectangles remain open. Metadata-only Inverse already works in rectangles. See [column limits](column-safety.md).
- Search: independent mark styles, brace/change navigation, result navigation, incremental/volatile find and character-range search.
- File/session/tab state: pinned tabs, session exchange, printing, and close rules. Explicit renamed-label precedence is integrated after green main gates; see [validation](windows-untitled-label-validation-20261011.md). Automatic titles still require an oracle audit for first physical line, whitespace normalization, empty-line/name retention and bounded UI work. The current helper materializes entire lines and can scan the document during title calculation (source finding without a native latency/OOM reproduction). Typing Undo grouping also needs a separate oracle audit.
- View: split panes, folding, character/EOL controls, monitoring and document panels.
- Encoding/languages: reinterpretation versus conversion, Windows ANSI/code pages, and missing languages.
- Remaining Settings, Macro, Run, Plugins, Window and Help behavior from the ledger.

Constraints still open:

- Native Windows UI verification: `sky.list_apps` again failed on 2026-10-11 with native pipe unavailable, os error 2. Source work continues; no visual pass is claimed.
- Exact locale sorting: the bounded Windows NLS bridge and caller tests are integrated; combined automated checks and optimized controls pass. Basic mixed-EOL sorting deliberately preserves physical endings rather than following the oracle's configured-EOL splitting. Mixed macro merging refuses full legacy timelines above 128 MiB while retaining separate Undo entries. See `windows-line-sorts.md` and `bookmark-edits.md`.

No push, release publication, Linux/WSL/VM execution or Mac test run is authorized for this pass. Keep existing platform source compiling as far as the available Windows build can establish.

Acceptance ledgers under `gates/` are local ignored work files at the user's request. Keep essential behavior and verification evidence in committed documents.
