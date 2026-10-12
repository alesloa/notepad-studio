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

- Normal Save/Save As/Save All: Root accepted isolated ordinary-job Rename/close and held Save/Save As/Save All controls, plus bounded native/raw Undo/path and checked-cache prerequisites. Protected predecessor publication/rollback and complete fallible orphan-cleanup planning are verified prerequisites. The first registered Save→Full slice passed real native/raw Undo readback; shared capture-budget and notification controls passed separately. Its held admitted-Full close control reproduced premature close acceptance, so close/Exit and final Full/cache/ticket/Copy/Move composition remain open. Bounded recovery is integrated, with [combined Windows checks](windows-bounded-restore-validation-20261011.md). Preserve rescue/error behavior while moving foreground work off the UI thread.
- Tools: all twelve hash commands and shared clipboard ordering are integrated. Main build, 2,875 unit plus three integration tests/32 ignored, unchanged Clippy (331/193) and inventory checks pass. Root verified all 55 delivery entries and 45 exact standalone engine/file-worker controls with 90 empty competitor checks; independent Python vectors/results/medians match. See [Hash validation](hash-tools-validation-20261011.md). Native UI/IME/picker/paste/ACP/external clipboard remain open, with explicit UTF8/strict Windows-1252, caption and resource-limit policies.
- Existing sort/reverse/shuffle/dedup callers: all twelve are integrated after green combined main build, 2,809 unit plus three integration tests/32 ignored, unchanged Clippy (331 occurrences/193 pairs) and inventory checks. Six optimized candidate test-harness controls on >10 MiB pass; they use unwind and do not claim shipping-abort/native timing. See [legacy validation](windows-legacy-lines-validation-20261011.md). Native verification and the explicit UTF16/numeric/rectangle/EOL/order limits remain open; no native CRT conversion probe is claimed.
- Brace navigation and bracket highlighting: both Search routes, saved macro continuation and shared cancellable matching/cache are integrated. Eighteen focused tests, main build, 2,684 unit plus three integration tests/30 ignored, Clippy with zero new diagnostics and inventory checks pass. The [brace validation](windows-brace-validation-20261011.md) records unchanged-paint work and scoped optimized helper timing. Exact lexer-style distinctions, styled-tail handling, Find-focused behavior and native visual/input checks remain open.
- Source/Raw deletion Undo: ordinary empty-selection Backspace/Delete grouping is integrated. [Main checks](windows-deletion-main-validation-20261011.md) pass: build, 2,917 unit plus three integration tests/32 ignored, unchanged Clippy (332/193), inventory and parser tests. Twenty-four scoped optimized model controls retain every operation and document both deletion overhead and lower peak working memory. Held asynchronous deletion Save composition, native/ACP verification and Enter/Tab/completion boundaries remain open.

Then continue:

- Column parity: temporary safety caps are integrated. Large column preparation, ragged word deletion, external clipboard semantics, active rectangle IME and mutating bookmark rectangles remain open. Metadata-only Inverse already works in rectangles. See [column limits](column-safety.md).
- Search: independent mark styles, brace/change navigation, result navigation, incremental/volatile find and character-range search.
- File/session/tab state: pinned tabs, session exchange, printing, and close rules. Explicit renamed-label precedence is committed as `19e26d5`; see [validation](windows-untitled-label-validation-20261011.md). Automatic-title parsing and ordered scalar/prepared naming remain under review. The fixed-page candidate passed wide input, joint admission, yielded freshness, bounded commands, disposal and transfer controls; two deciding failures exposed pristine-tab activity initialization and IME no-op final revision stamps. Narrow corrections are underway. Naming capacity must affect metadata only, never refuse text/IME/history edits or prevent Save/recovery. Adjacent Source/Raw insertion grouping is integrated with [combined main checks](windows-typing-main-validation-20261011.md); deletion is recorded above. Final asynchronous Save/recovery composition and Enter/Tab/completion/native boundaries remain open.
- View: split panes, folding, character/EOL controls, monitoring and document panels.
- Encoding/languages: reinterpretation versus conversion, Windows ANSI/code pages, and missing languages.
- Remaining Settings, Macro, Run, Plugins, Window and Help behavior from the ledger.

Constraints still open:

- Native Windows UI verification: `sky.list_apps` again failed on 2026-10-11 with native pipe unavailable, os error 2. Source work continues; no visual pass is claimed.
- Exact locale sorting: the bounded Windows NLS bridge and caller tests are integrated; combined automated checks and optimized controls pass. Basic mixed-EOL sorting deliberately preserves physical endings rather than following the oracle's configured-EOL splitting. Mixed macro merging refuses full legacy timelines above 128 MiB while retaining separate Undo entries. See `windows-line-sorts.md` and `bookmark-edits.md`.

No push, release publication, Linux/WSL/VM execution or Mac test run is authorized for this pass. Keep existing platform source compiling as far as the available Windows build can establish.

Acceptance ledgers under `gates/` are local ignored work files at the user's request. Keep essential behavior and verification evidence in committed documents.
