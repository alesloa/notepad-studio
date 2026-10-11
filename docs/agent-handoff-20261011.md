# Notepad Studio handoff, 2026-10-11

Continue the user's goal of finishing Windows Notepad++ behavior and feature
parity. Mac testing is deferred. Full parity remains open; this is a tested
checkpoint for one completed batch.

## Current checkpoint

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
- Completed save, column and physical-line worktrees preserve frozen evidence.
  File now implements bounded restoration in `windows-bounded-restore`; Book
  prepares hash-tool controls in `windows-hash-tools`. Sort prepares existing
  line-operation controls in `windows-legacy-lines` at `b42c889`. Caches have
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

Inventory: 581 appearances, 481 unique IDs, 45 separators, 275 catalog rows;
182 routed, 285 missing, 8 partial, 2 alternate-only, 2 action-only, 1 status-only
and 1 stub. Routing proves wiring only. Behavior records remain pending.

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
   normal Save background capture/publication is the next read-only contract.
   That local contract is
   `../notepad-studio-agents/windows-save-durability/gates/windows-normal-save-background-contract.md`,
   SHA-256 `a5a429745879784779e04abb34c09422e700c42b453b35b3e856d4320aafff4e`.
   It maps Save/Save As/Save All/popup/close and quit ownership. Capture must be
   bounded; success publishes the captured savepoint so later edits stay dirty.
   Preserve picker-time edits, UUID/Arc/path ownership, optional backup policy,
   mandatory rescue refusal and critical failed-restore diagnostics, including
   Cancel/stale replies. Settle destructive jobs before accepted close/Exit.
   Legacy `.doc` conversion requires separate scope. Bounded restoration is
   now a separate active leaf below. No normal-Save background implementation
   is started yet.
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

- `windows-bounded-restore`, detached at `10d4723`: reviewed G1 runs have one
  positive and one intentional contract failure. The old restoration callback
  receives 18,874,368 bytes against the proposed 65,536-byte budget. Injected
  read OOM preserves the exact rescue and original write/restore errors; no
  allocator abort or physical memory exhaustion was reproduced. Root verified
  all 13 evidence hashes and approved the fixed 64 KiB same-handle recovery
  pump, with checked counts, short-read/Interrupted/EOF/growth controls, final
  truncation/sync only after complete restoration, and rescue retention on
  failure. Implementation is active; nothing from this leaf is integrated.
- `windows-hash-tools`, detached at `10d4723`: source-only G1 preparation for
  twelve MD5/SHA1/SHA256/SHA512 text/file/selection commands. No Cargo or
  production implementation yet. Selection policy review covers UTF8 editor
  bytes, document EOL, no transport BOM, strict Windows-1252 and rectangle/focus
  behavior. Read [oracle preparation](windows-hash-tools-oracle-20261011.md).
  Rebase onto the latest combined checkpoint before production work.
- `windows-legacy-lines`, detached at `b42c889`: source-only G1 preparation
  for the eight older lexical/integer/decimal sort routes, Reverse, Randomize
  and both duplicate-removal commands. No Cargo or production fixes yet.
  Audit exact numeric/refusal, UTF16 lexical, rectangle and selection behavior
  before reusing the guarded prepared-line engine and saved macro pipeline.

Then continue older sort/reverse/shuffle/dedup performance/macro/fidelity repair,
hash tools, brace navigation/highlighting, session/pinning/printing, Search
results/mark styles, split views/folding/monitoring, encoding/languages and the
remaining ledger. Explicit renamed-label precedence over
`untitled_uses_first_line` is also open; the stored label already supplies
Save/Copy/Save-close suggestions.

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
