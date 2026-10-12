# Notepad Studio handoff, 2026-10-11

Continue the user's goal of finishing Windows Notepad++ behavior and feature
parity. Mac testing is deferred. Full parity remains open; this is a tested
checkpoint for one completed batch.

## Current checkpoint, updated after deletion integration on 2026-10-12

Ordinary Source/Raw Backspace/Delete grouping is integrated after the `c5221e7`
documentation checkpoint. Required main build, 2,917 unit plus three integration
tests/32 ignored, Clippy with unchanged 332/193 located diagnostic counts,
inventory checks and all 12 parser tests pass. See
[deletion main validation](windows-deletion-main-validation-20261011.md) for the
exact source, logs, executable and scope. Twenty-four separate optimized model
controls and 48 empty inventories pass; added deletion costs and lower model
memory are recorded without a native speed claim. No push accompanies this batch.

The [2026-10-12 audit](windows-parity-audit-20261012.md) records the latest isolated
results. File's registered Save→Full readback passes, but independent controls
found shared-turn bookmark/CSV overruns and an admitted storage error hidden by
close. Corrected focused controls and actual unsaved peer preservation pass;
immediate explicit retry still refuses from the old completed error. Its deciding
correction, close/Exit and full coordinator composition remain open.
Sort candidate 11 passes corrected activity/IME/CSV stamps, coalesced deletion
notifications and continuation controls. Its full-run parser and inherited
getter fixture need correction before required broad gates can pass. Book's
keyboard baseline ran all 69 contexts across two frozen packets, with three
passing functions and eight failures. Implement exact Enter/Tab history, columns,
indentation and recording while continuing Advanced, rectangle and large-command
work. A temporary safe refusal is not completed keyboard parity.
Root owns final composition and commits. Use the actual Git log for the current
commit identity; native verification and the full inventory backlog remain open.

## Previous typing checkpoint

The latest product-source checkpoint is `ba9f296` (`fix: group adjacent text
input for undo`), after Hash Tools `f62f750`, legacy line operations `4a5dad8`
and explicit untitled labels `19e26d5`. Main source is clean; nothing was pushed.
The earlier sections below are historical evidence, including superseded worker
assignments. Use this section and the current work queue to choose new work.

Adjacent ordinary Source/Raw insertion grouping is integrated. Main build,
2,899 unit plus three integration tests/32 ignored, Clippy with zero new warning
pairs, inventory checks and 18 scoped optimized controls pass. The main Clippy
baseline has 332 located occurrences/193 distinct message/path pairs; the extra
occurrence is an inherited session warning. See
[typing main validation](windows-typing-main-validation-20261011.md) and
[typing performance](windows-typing-undo-performance-20261011.md). Optional
grouping refusal keeps a separate edit; essential text/history refusal precedes
mutation and recording. Captured Save fences and stronger history stamps are
part of this checkpoint. Deletion, Enter/Tab, completion and native boundaries
still need their own proof. The inventory remains 196 routed/271 missing unique
IDs; routing alone does not establish behavior parity.

Active isolated work, inspected during the continuation:

- **File** (`windows-normal-save-background`, base `6be0a4d`): normal Save and
  held Save/Save As/Save All prerequisites passed their scoped controls. G14
  protected recovery publication and complete orphan planning pass 37
  persistence functions/one inherited ignored test. Root verified 64 evidence
  entries and 52 source pairs. The deciding late-entry error now preserves all
  four original files, and actual retry succeeds. Planning uses the shared
  request budget; known Delete/stale-native cleanup and later removal errors
  have separate guarantees. These are Writer/helper prerequisites. Registered
  coherent Full capture, cache coordinator, App permits/tickets, Copy/Move and
  close/Exit settlement are still being implemented under boundary `4f43e770`.
  Preserve 64 KiB/128 items per UI turn, the 512 MiB cumulative budget and the
  complete current raw/native/Undo/CSV recovery schema. Essential error and
  actual cleanup acknowledgements govern close permission.
- **Sort** (`windows-automatic-title`, base `19e26d5`, with later source
  additions): candidate 6's three behavior controls and one diagnostic pass;
  15 encoding/EOL cases are nested in one function. Two candidate-5 Save
  assertions wrongly expected an untitled label after successful Save; corrected
  tests preserve disk/document/history checks and the saved-path caption.
  The per-character macro setup exhausted naming metadata while preserving
  text/history; the intended single painted callback saved-macro control passes.
  Candidate 7 then reproduced a real UI-budget defect across 512 accepted Source
  callbacks: Latest-map growth exceeded 32 KiB. Bounded 16-slot pages, joint
  notice/input accounting and command/waiter limits are being corrected.
  Complete source/consumer/disposal review, broad gates and main integration
  remain open. Metadata limits retain accepted names and keep edits, Save and
  recovery available. The original mounted CRLF fixture's exact failure cause
  remains unproven; its diagnostic is not an additional parity result.
- **Book** (`windows-deletion-undo`, base `ba9f296`): test-only G1 ran all twelve
  registered Source/Raw parameters, with eight single-deletion positives and
  four repeated-deletion mismatches in two functions. The worker reports 17 deletion
  and 24 existing typing controls passing on its frozen correction; Root review,
  broad checks and main composition are pending. Preserve the oracle's
  editing-byte adjacency rule, every operation/revision/first caret and captured
  Save fences. Valid BOF/EOF/read-only macro intents remain replayable;
  essential refusal stops a saved macro before its later edit. Test-accessor
  and UTF-8 packaging failures were compile-only, not behavior results.

Root also ran a detached-Find brace probe. Actual Find/document/field controls
passed before two missing-GoTo observations. The oracle's separate Find
accelerator table excludes brace actions, so these assertions are not accepted
parity failures without native routing evidence. The test-only patch, binary
and logs are retained; main source was restored exactly. Exact lexer styles,
styled tails and native brace behavior remain open.

Local detailed receipts and review records stay under ignored `gates/` and
`log/`. Keep existing caches/artifacts and inspect live processes before reusing
them. Root's Cargo cache is `target`; workers retain their exclusive caches.
Recheck the actual process state before any optimized timing run. The native
bridge remains unavailable; no desktop UI/IME/clipboard verification is claimed.
The full Windows goal remains active. Mac tests are deferred; Linux/WSL/VM runs,
pushing and release publication are outside this pass.

## Earlier checkpoints and reference

Bounded failed-save restoration is committed as `6be0a4d`; its combined
Windows checks and exact optimized helper controls passed. Brace navigation is
committed as `45eeb7e`: both Search routes and the shared worker/cache candidate
are integrated. Main was clean after the commit.
The isolated candidate passed 18 focused tests and 2,684 unit plus three
integration tests (30 ignored). Main build (47.959 s), full tests (81.59 s unit / 211.665 s total),
Clippy (26.914 s, zero new diagnostics) and all 12 inventory tests/check pass.
See [brace validation](windows-brace-validation-20261011.md) for the deciding receipt.
Do not treat this as completion of the Windows parity goal.

The current main batch also fixes explicit untitled-label precedence and
persists its origin through open/closed recovery and window transfer. Main
build (62.729 s), 2,690 unit plus three integration tests/30 ignored
(51.12 s unit / 166.669 s total), Clippy (22.365 s, unchanged 331 occurrences /
193 pairs), 12 inventory tests and generation check pass on all 733 frozen
source/manifest entries. Seven source files are exact copies of the final
reviewed leaf. See [rename validation](windows-untitled-label-validation-20261011.md).
The label fix is committed as `19e26d5`. The legacy batch is integrated and checked; verify Git log/status for its resulting commit.

The twelve legacy line commands are integrated with the newer brace/macro
behavior preserved. Main build (44.379 s), 2,809 unit plus three integration
checks/32 ignored (52.94 s unit /138.404 s total), Clippy (23.510 s) with exact
331 occurrences/193 pairs, and twelve inventory tests/check pass. All 966
frozen entries stayed unchanged during the required gates. Eighteen leaf files
match after Git newline normalization; three macro files retain only main's
newer brace additions. See [legacy validation](windows-legacy-lines-validation-20261011.md).
Six optimized candidate test-harness controls pass on 12,059,999 bytes/90,000
rows, with 36 actions, 36 saved macros and 144 held refusals; harness unwind differs
from shipping abort. Root verified all77 final evidence and nine release
provenance entries. Native rectangle/input/visual proof remains open.

All twelve Hash Tools commands and the clipboard coordinator are now integrated
with brace/legacy behavior preserved. Main build (41.150 s), 2,875 unit plus
three integration tests/32 ignored (74.59 s unit / 190.338 s total), Clippy
(24.562 s, exactly unchanged 331 occurrences/193 pairs), twelve inventory tests
and generation check pass. All 993 frozen entries stayed unchanged during gates;
32 of 38 Rust files match the final leaf after newline normalization, six retain
only newer main additions. Root verified the final 55-file delivery, helper
compile, all 45 optimized standalone results/90 empty inventories, independent
hashlib vectors, every output/progress/refusal value and medians. See
[Hash validation](hash-tools-validation-20261011.md). The 353 production Rust
files hash to `2f32e1e531367e6f6f706967db2c0cb8d55039a83691d32b00b8a58031098cb9`;
static inventory is 196 routed/271 missing unique IDs and 289 CATALOG rows.
The earlier isolated app-target JSON warning counts (404/114) use a different
extraction from main's rendered vendor-inclusive comparison. Native UI/IME/
paste/ACP/external clipboard remain open; the byte/caption/budget limits are
explicit. Source/leaf evidence stays frozen; inspect Git log for this batch's
resulting commit. The legacy checkpoint is committed as `4a5dad8`.

Book now owns test-only `windows-typing-undo`, based at `4a5dad8`. Five actual
painted installed-source callback controls give three PASS/two genuine RED:
separate a/b/c and Unicode commits require separate Undo slots rather than the
oracle's coalesced insertion. Single-callback, explicit model-savepoint and
registered Paste boundaries pass. Registered Redo and unchanged metadata were
checked before the deciding assertions. First compilation/fixtures succeeded;
no native proof or production typing change exists. Receipt `7753d2f3` and
retained EXE `c8582597` await full root review/proposal. Preserve IME's dedicated
slot, explicit completion/macro/prepared groups and captured asynchronous Save
boundaries; no timer/movement/Undo heuristic or global replace auto-coalescing
is approved. Source-count-only delayed ownership guards need revision/activity
checks when a Group grows without changing stack length.

File's `windows-normal-save-background` leaf is based at `6be0a4d`. Root
verified separate G3 Save All/popup: 42 positive functions plus the remaining Full
foreground-clone RED, the G4 native helper (five new positives; all26 source
archives/103 evidence entries), and G5 held normal Save (three positive
functions/one genuine close-settlement RED; all104 evidence entries). Counts
are not additive across packets. The registered close callback accepts while
an authorized Save remains held; the test releases/settles it before failing.
The bounded native cursor/Builder is reviewed, but not registered in Full yet.
File also reports G6 Rename/close settlement fixes passing eight registered
controls (normal jobs only; synchronous final recovery still remains), and G7
bounded raw Undo helper passing four controls with actual Writer/codec equality.
Their final packets await root review; do not aggregate counts or claim Full ready.
Actual Writer/readback, raw Undo/CSV/metadata/cache/tickets/aliases and close/Exit
settlement still require integration under the unchanged Full contract.
App-owned gates share destructive admission across windows without blocking
UI; admitted operations keep permits until actual cleanup. Root reviewed the
refined 64 KiB/128-item shared-turn, 512 MiB budget and ticket/retirement barrier
boundary `4f43e770`. Preserve mandatory rescue/errors and old recovery/cache on
refusal; metadata identities cover changes not scheduling autosave.

Sort now owns `windows-automatic-title`, detached at `19e26d5`. Corrected G1
has three PASS/ten RED: nine real naming-contract failures and one getter-work
failure. Compile-only and welcome-text/EOL fixture runs remain excluded and
preserved. Sort reports phase 1 with 25 naming/parser/summary controls plus one actual
typed-source control green; all thirteen original G1 controls pass. Root has
not reviewed the final phase-1 packet yet. Long-prefix/prepared/history/consumer
integration is still in progress. The approved
fixed-collision summary preserves admitted intermediate names. Capacity
exhaustion may suspend title metadata visibly, retain the accepted name and
leave text/IME/history/Save/recovery available; never refuse a document edit
for a derived title. Surrogate-boundary U+FFFD representation is an explicit
tested parity limit with document bytes unchanged. Root owns final composition.

Root also verified the separate publication packet: one intentional bounded-
capture RED after real Full recovery byte/state positives. A controlled model
hook inserts a later native edit after synchronous Save; it does not establish
interactive input during that write. The actual isolated Writer preserves that
edit, a separate dirty native tab and its raw undo bytes. Its current capture
still clones file-sized payloads on the UI thread. All 19 evidence entries and
seven consumed sources were verified. Receipt SHA256:
`1e37326f0e2081e3b5a60104e3a2dc5dc6111f5ad52286c870a92819d249c8ec`.

The surgical bounded Full session capture proposal is authorized in File's
leaf; it is not integrated. Proposal SHA256:
`57566cd6924b72ca1c86fb21f6b4d9fd2728afc9b8bab47d179321d67aa82606`.
Use 64 KiB/128 newly captured structural items per UI turn and a 512 MiB
cumulative new-allocation budget. Preserve complete current AST/edit/raw undo,
CSV/metadata, schema, Keep/UUID/MRU ownership and prior recovery on refusal.
Move reserve/join/last-reference cache disposal to workers. Request tickets
and existing generation retirement must reject captures admitted before
accepted close/Exit, which awaits actual writer/restoration/cleanup and a fresh
Full recovery point. The candidate must observe the new capture work directly;
removing the old whole-clone observer does not prove bounded copying.
Synchronous compatibility and forced-platform fallback remain separately
inventoried gaps. No new unsafe/dependencies, main edits or release claims are
authorized in this leaf.

Current source and generator outputs include 340 production Rust files,
SHA256 `032a8ba5223f22e50801f0f4794d280dda7e591cb99212d898d6ee046849b586`.
Cargo retains `fb072656`; lockfile remains `9ea6f022`. All 733 frozen
source/manifest hashes stayed unchanged during main checks. Native Windows verification remains unavailable; exact brace lexer
styles and styled-tail behavior remain open. Overall goal active. Verify Git
status/log and actual owner state before resuming.

- Repository: `C:/Users/Ale/Documents/GitHub/notepad-studio`, branch `main`.
- Feature checkpoint: `c077e2dca48b6c35c457691c84f4b9c2c843b935`, adding 17 File,
  bookmark and line-sort commands plus the shared background engine. Save
  protection is committed as `10d47236c088ea83309505c0398334af7ddd035e`.
  Column/physical-line composition is committed as
  `b42c889f8d37f09b244bab8037ad31a4e82caede`, with the combined gates below
  passing. Verify actual Git status and log before editing or applying patches.
- Gitignore: `1b1a11a` adds `/gates/` and removes eight old gate files from the
  index. All eight remain locally. Essential evidence is in committed documents.
- Previous baseline: `d01099f01dd464607cdbe62736789192ba8787eb`. Earlier resumed
  source `fdf45a0` and completion fixture `72dcf44` are already incorporated.
  Do not reapply old worker patches.
- Oracle: `C:/Users/Ale/Documents/GitHub/notepad-plus-plus-master/PowerEditor/src`.
  Study behavior and write original Rust; preserve the Apache-2.0 boundary.
- Windows PowerShell; set both `CC` and `CXX` to
  `C:/Program Files/LLVM/bin/clang-cl.exe` for Cargo.
- Completed save, column, physical-line and bounded-restoration worktrees
  preserve frozen evidence. File implements the fresh normal-Save background
  leaf. Book implements hash tools in `windows-hash-tools`; Sort implements the
  reviewed legacy-line phases in `windows-legacy-lines`. Both are based at
  `b42c889`. Caches have
  exclusive owners; inspect actual agent/process state before using one.
- No push, release publication, Linux/WSL/VM work or Mac testing occurred.

Read `AGENTS.md` and `README.md` first. Main has `.codegraph/`: use CodeGraph
before locating or understanding source. The [inventory](notepadpp-parity-inventory.md),
[behavior ledger](windows-parity-behavior.json) and
[work queue](windows-parity-work-queue.md) identify remaining work.

## Authorization and working rules

The user authorized autonomous work, isolated parallel agents, reviewed
integration and commits through `$commit`, without pushing. Reproduced
crash/data-loss fixes take priority. Root owns integration, manifest decisions,
combined checks and records. Workers own distinct worktrees/Cargo caches,
return frozen patches/hashes, and do not write main or commit.

The user delegated the Windows locale exception after stressing that people
rely on their saved and unsaved work. Root approved exactly two audited
`LCMapStringEx` calls in the bounded safe `Result` wrapper. `AGENTS.md` records
that narrow seam; no general unsafe permission exists.

Keep native Rust/GPUI, Markdown Reader, rich documents, CSV views and local
English proofreading. Preserve user documents and shared config/session state.
Single-instance forwarding can direct a dev launch to an older installed copy.
Use neutral fixtures and scoped persistence sinks; do not fake `window_closed`
merely to suppress persistence. Format only code you write.

## What landed

- Six File routes: Rename, Move to Recycle Bin, cmd, PowerShell, Folder as
  Workspace and Save a Copy As. Pickers/moves bind UUID/Arc/path/window/request
  ownership; newer native or popup Rename cancels an earlier held stage. Copy
  preserves original path/dirty/history and can open its result. Cross-volume
  moves stream in 1 MiB chunks; copy capture/encoding/writing uses background
  work. Failed restoration reports the surviving original's path even after
  Cancel or later edits. See [File validation](windows-file-routes-validation.md).
- Five bookmark commands: Cut, Paste, Remove Bookmarked Lines, Remove
  Non-Bookmarked Lines and Inverse, with saved macro routes. Physical endings
  survive. Nonempty identical Paste is an edit; empty-to-empty is a no-op.
  Inverse permits read-only rectangles without changing text/history.
  Text-mutating commands currently refuse rectangles. See [bookmark editing](bookmark-edits.md).
- Six sorts: ascending/descending case-insensitive lexical, UTF-16 length and
  Windows locale sorts, with rectangle keys, background planning and macro
  continuation. Windows uses user-default NLS keys. Basic mixed-EOL sorting
  preserves physical endings rather than claiming the oracle's configured-EOL
  output. See [line sorts](windows-line-sorts.md).
- Prepared Undo/Redo retains histories until validated publication. Mixed macro
  capture yields at 64 KiB/256 operations per turn, assembles the full timeline
  on a worker and swaps the completed vector. Refusing a timeline above 128 MiB
  keeps text and every separate command Undo entry. Clipboard normalization and
  large result/history disposal run on workers. Source/context stamps reject
  stale results before publishing.
- Inventory scanner regression: complete Rename/Delete main routes now
  supersede obsolete popup-only review notes. The regenerated two-case test
  failed before the fix; all 12 generator tests now pass.

Limits remain explicit: native copy snapshots have a conservative 512 MiB
cumulative allocation budget; bookmark output/history are capped at 128 MiB,
annotations at 65,536, and targets at 262,144. Sort source is capped at 32 MiB /
1,048,576 rows and aggregate locale keys at 64 MiB. These do not prove every
older input path is bounded.

## Verification

Frozen combined Windows checks passed:

- Build: exit 0, 63.083 s wall.
- Full tests: 2,574 unit and 3 integration passed, 29 ignored; unit runtime
  70.81 s, total 193.337 s wall.
- Clippy: exit 0, 33.416 s wall. Inherited 195 app / 209 test warnings,
  including 191 duplicates. Normalized message/path comparison: 334 diagnostic
  occurrences before and after, zero added pairs, including dependencies.
  The tree is not warning-free.
- Inventory tests: 12 passed; `--check` passed. Rust source SHA-256:
  `0b28fcc1c0ac5f2c2b5d4c6f005aa3857ffe8cf656c7cdc3b597b9d6d0dfd7d3`.
- No plugin ABI/host API or vendor change; `git diff --check` passed.

Default-release warmups used optimization level 3, thin LTO and one codegen
unit. Once all compilers/linkers were idle, File → Book → Sort ran 12, 6 and 3
deciding invocations respectively, all passing. Controls cover over-10-MB
copies, native cancellation, cross-volume moves, bookmark edits/Undo/Redo,
large clipboard/history capture and native locale mixed macros. The test
harness uses unwinding. Measurements do not establish native latency, OS
clipboard cost or a whole-app speedup. Native cancellation measures visible
publication and excludes completed snapshot disposal.

The [combined validation record](windows-parity-validation-20261011.md) and
caller documents retain sizes, times, ranges, hashes and local artifact paths.
Main logs: `log/windows-parity-main-{build,tests,clippy}.txt`, with timing,
diagnostic comparison and inventory logs alongside. Gates/logs are ignored.

## Next work

Current inventory: 581 appearances, 481 unique IDs, 45 separators, 277 catalog
rows; 184 routed, 283 missing, 8 partial, 2 alternate-only, 2 action-only,
1 status-only and 1 stub. Routing proves wiring only. Behavior records remain
pending. The following save/column/physical-line receipts are historical;
the current combined brace checks are recorded above.

The save, column and physical-line safety batch is integrated. Its baseline
reproductions, approved policies and combined checks are recorded in
[the safety validation record](windows-safety-validation-20261011.md). Do not
reapply any of these completed patches.

1. Save durability: `windows-save-durability` passes its required leaf checks
   (2,586 unit plus 3 integration tests, build and Clippy). Main already contains
   reviewed source patch `d850cb37ee91c4f85d39c15381f734a88c43fa6301d217b11e0425edea3c6aa6`
   and doc suffix `12e637049ed60aca02e5c7d0dacb22de19014f77045fb543a903bda0d39d2917`;
   new-file line endings were normalized by Git, with equivalent bytes verified.
   The writer requires a created and synced sibling rescue before touching an
   existing target. Three actual child-interruption baseline runs showed why
   the memory fallback was insufficient. Main save-only build, 2,586 unit plus
   three integration tests (29 ignored), Clippy and inventory checks pass.
   Canonical diagnostics have zero additions: 334 occurrences/195 unique pairs.
   Tested Rust inventory SHA-256 is
   `8781129422befa76ef7db9f6cca68a4709693d6b4aace91a0b84d89ef30b23a8`.
   The later column/physical composition passes the combined checks below.
   The quiet 18 MiB writer-helper comparison adds 194.542 ms at the medians;
   normal Save background capture/publication is now an authorized active leaf.
   That local contract is
   `../notepad-studio-agents/windows-save-durability/gates/windows-normal-save-background-contract.md`,
   SHA-256 `a5a429745879784779e04abb34c09422e700c42b453b35b3e856d4320aafff4e`.
   It maps Save/Save As/Save All/popup/close and quit ownership. Capture must be
   bounded; success publishes the captured savepoint so later edits stay dirty.
   Preserve picker-time edits, UUID/Arc/path ownership, optional backup policy,
   mandatory rescue refusal and critical failed-restore diagnostics, including
   Cancel/stale replies. Settle destructive jobs before accepted close/Exit.
   Legacy `.doc` conversion requires separate scope. Bounded restoration is
   integrated as `6be0a4d`. Normal-Save background implementation is active in
   its fresh leaf under this frozen contract; it is not integrated yet.
2. Column safety: `windows-column-safety` has five actual hostile-state REDs
   and two classic-case rectangle behavior REDs. Its first five regression
   reruns pass. The candidate validates geometry, payload and history plans
   before publication, with temporary 128 KiB work, 4,096-row and 1 MiB
   metadata limits. The corrected candidate passes 35 column controls, 13
   macro-context controls and nine macro-Undo controls. Frozen build, 2,609
   unit plus three integration tests (29 ignored), and Clippy pass. Reviewed
   patch `03e1ebb1cacec736a19315356a59befac6048df28fc825e1ba576f4066d64874`
   is integrated. Macro Paste reads the clipboard once, and fixture persistence
   uses scoped sinks instead of an artificial closed-workspace state.
   No asynchronous input queue or new macro schema is authorized in
   this safety batch; mutating bookmark rectangles remain separate work.
3. Physical lines: `windows-physical-lines` has 20 caller behavior REDs and
   two real Find in Files result/navigation REDs. The corrected candidate
   focus passes 36 controls, with one optimized control ignored. Production
   scope is Ropey's explicit `cr_lines`/`simd` feature policy and two Find in
   Files helpers; bytes, version and word classification stay unchanged.
   The final frozen candidate passes build, 2,605 unit plus three integration
   tests (30 ignored), and Clippy with zero new canonical diagnostics. Its patch
   SHA-256 is `fa56bd046f6e6090ece6d3a810ed4de605ff0c0a90a2cc41f29d2da44a3f212a`.
   Recent-file isolation and legacy case/marker fixture corrections are
   included. Main integration and all nine quiet default-release processes
   pass. The release comparison covers loader phases only; Unicode Save
   timings describe this leaf's older writer. Duplicate→Undo is restoration
   smoke, without an intermediate edit assertion. Final 54-entry evidence
   manifest SHA-256 is
   `22a4735c2f85404ff2cba9393d35a166a7f6bac4ea8ff33d324c75244dbae2d1`.

Combined main checks pass without source changes during verification: build
36.338 s; 2,652 unit plus three integration tests, 30 ignored (unit 40.56 s,
total 157.024 s); Clippy 24.340 s. Canonical diagnostics remain 334 occurrences
and 195 unique message/path pairs, with zero additions or removals. Inventory
tests (12) and `--check` pass. Current 337-production-file Rust inventory SHA:
`c71335ee3eaa3d371bef0f956e95ea302ca23a719329a7c54e15832b3c26c339`.
Cargo manifest SHA:
`fb072656ef80cb3fdf6d5fb858003562f941c031ea5ab9dcd802aa294991e67a`;
Cargo.lock is unchanged. Logs are `log/windows-safety-main-*`.

An additional debug functional run of the ignored streaming caller passes
(one test, 2.20 s), composing the newer save protection with physical rows:
14,240,003-byte UTF8-BOM and 25,280,002-byte UTF16BE files retain exact bytes,
movement and bookmarks. This is functional evidence, not optimized/native
latency. The historical feature and save-only source identities above remain
separate checkpoints.

Active next leaves:

- `windows-normal-save-background`, detached at `6be0a4d`: root verified the
  frozen G1 packet and authorized bounded capture, worker write/publication,
  captured savepoints and close/Exit settlement. The separate real recovery
  session-capture control and surgical bounded Full capture design above are
  reviewed; implementation is authorized in this leaf, with a first coherent
  capture phase required before broad integration. Bounded failed-save
  restoration is already integrated. Do not reapply its older patch. Root
  accepted the first coherent manual Save/Save As phase: seven new controls
  and 29 existing ownership/close controls pass; four expected REDs remain
  (three Save All callers and Full recovery foreground capture). This is
  partial proof under receipt ec7727e8, not full background-save acceptance.
  Save All live CSV fixtures and bounded Full capture/settlement remain active.
  Include the new defaulted new_label_is_custom scalar when composing capture.
- `windows-hash-tools`, detached at `b42c889`: all twelve MD5/SHA1/SHA256/SHA512
  text/file/selection tools are implemented in the isolated candidate. The
  engine passes 11 tests and final GPUI controls pass 60 plus three clipboard
  ownership controls. Root verified receipt e1e42d4f / manifest 13bf9974 and
  authorized broad debug gates. Optimized/native checks and main composition
  remain pending. Selection policy covers UTF8 editor bytes, document EOL, no transport BOM, strict
  Windows-1252 and rectangle/focus behavior. Read
  [oracle preparation](windows-hash-tools-oracle-20261011.md). The bound Find
  fallback must retain the exact owning Workspace and origin focus.
- `windows-legacy-lines`, detached at `b42c889`: the eight older lexical,
  integer and decimal sort routes, Reverse, Randomize and both duplicate
  commands use the guarded prepared-line engine and saved macro pipeline.
  Final required gates pass on manifest caebb0f7 / patch 734a0a55:
  2,771 unit plus three integration tests/32 ignored and unchanged Clippy.
  Both ignored large caller controls pass in debug; source/plan/evidence
  reviewed. Default release test compilation is authorized; optimized runs,
  native verification and main composition remain pending.
  Preserve the reviewed UTF16 lexical, numeric refusal, rectangle and EOL
  policies, including whole-draft refusal on invalid decimal rows.

Brace navigation/highlighting is integrated with green main gates. Exact
lexer styles, styled-tail relaxation, Find-focused brace behavior and native
UI verification remain open. Continue the active save/hash/legacy-line leaves,
then session/pinning/printing, Search results/mark styles, split views/folding/
monitoring, encoding/languages and the remaining ledger. Explicit renamed-label
precedence over `untitled_uses_first_line` now passes main gates. Automatic-title
first-physical-line/whitespace/empty-name semantics and bounded UI work remain
open; first_nonempty_line currently materializes entire lines and can scan all
lines during title calculation (source finding, no native latency/OOM claim).
Separately audit typing Undo grouping; final rename tests use one registered
Paste operation for exact Undo/Redo and do not establish typing coalescing.

Native verification is unavailable. The supported bridge returned:

```text
Computer Use native pipe is unavailable: failed to connect native pipe: The system cannot find the file specified. (os error 2)
```

No native app launch/input occurred. Retry supported tooling when its state
changes; GPUI TestWindow and browser tests do not replace desktop QA.
Completion at window edges/zoom, IME, clipboard, proofreading and dialogs still
need verification. Mac tests are deferred; older baseline CI does not cover
these local changes.

The earlier reported crash concerned T3 Code/agent state, not a reproduced
editor crash. Compile-storage recovery cleaned only verified inactive Cargo
output; source/current caches/gates/documents were preserved. Neither incident
establishes editor data loss.

Keep the overall goal active across tested batches. Commit reviewed work without
pushing; checkpoint with current evidence and idle workers. Mark the goal
complete only after the full behavior backlog and required verification pass.
