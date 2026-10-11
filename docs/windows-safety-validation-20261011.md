# Windows safety investigations, 2026-10-11

Base: `088e8d209a5dfc226ec215da1a13853a412b9dac`, after the tested
17-command checkpoint. These three isolated investigations established baseline
failures through real callers. The root reviewed the evidence and approved the
limited policies below. The reviewed save fix is applied to main; column and
physical-line integration and final combined checks are pending.

## Save rescue copy

The actual injectable writer ran in a temporary child process. Its write closure
wrote and synchronized a partial prefix, then signaled a token-checked barrier.
The parent waited with a timeout and stopped only its owned child.

With a rescue name available, the previous disk version remained byte-identical
in the rescue. With all 100 rescue names occupied by unrelated stale contents,
the writer used its in-memory fallback; interruption left a changed target and
zero discoverable copies of the prior version in the fixture. Every stale file
remained unchanged. An ordinary returned-error control restored from memory.
Three independent runs each passed the positive/error controls and failed the
recoverable-prior-version assertion.

Approved policy: an existing target requires a created and synchronized sibling
rescue before it is opened for writing. Rescue creation or sync failure refuses
the save with the existing `NoSafetyCopy` error. Keep in-place target semantics,
hard links, permissions, optional backup policy and actionable restoration paths.
New-file behavior remains separate. This evidence concerns process interruption;
it does not establish power-loss durability or every filesystem's behavior.

Local leaf: `../notepad-studio-agents/windows-save-durability`.
Milestone patch SHA-256:
`5135d0e065fee5aece70531e2c0fac0da9db8b72a89d82c95c78f5801ecde50c`.
The leaf's `gates/g1-first-interruption.log` and two repeats record the failures.
The approved implementation must include a write-capable, nontruncating rescue
sync handle and an injected sync-error refusal control. Microsoft's API requires
write access for [FlushFileBuffers](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers);
Rust documents [sync_all](https://doc.rust-lang.org/std/fs/struct.File.html#method.sync_all)
as the explicit operation for reporting otherwise hidden close errors.

The frozen candidate passes 2,586 unit and 3 integration tests, with 29 ignored,
plus build and Clippy. Its focused controls exercise actual Save, Save As, Save
All and Save Copy callers with copy/sync errors, native formats, encodings,
hard-link identity, read-only attributes and failed restoration. No new
changed-code diagnostics appeared. Main integration and its save-only required
checks pass; the later column/physical composition needs separate combined
gates. Source patch SHA-256:
`d850cb37ee91c4f85d39c15381f734a88c43fa6301d217b11e0425edea3c6aa6`.

A quiet standalone comparison uses byte-exact writer/error excerpts and the
candidate helper with the shipping optimization flags: level 3, thin LTO, one
codegen unit and aborting panics. Three paired overwrites of 18,874,368 bytes
preserved exact contents, hard-link identity and attributes. Baseline times were
171.918, 205.888 and 145.829 ms; candidate times were 376.451, 366.460 and
362.197 ms. Medians are 171.918 and 366.460 ms, a 194.542 ms difference. Compiler
inventories were empty before, between and after the runs. Root verified all
excerpts against the recorded source bytes and verified the binary/provenance
hashes. This measures the writer helper; it excludes encoding preparation,
fixture setup and UI latency. Moving normal saves off the UI thread requires a
separate caller/capture/publication audit.

A separate read-only restoration audit examined matching installed Rust 1.98
source. Its initial `fs::read` allocation is fallible: refusal becomes an I/O
`OutOfMemory` error, which the current writer reports with the retained rescue.
A conditional probe-growth branch uses infallible allocation, but no allocation
abort was reproduced. Do not report ordinary restore allocation as a proven
crash. A proposed 64 KiB restoration pump would bound scratch space while
preserving the target handle, exact errors and rescue retention; it has not been
implemented. Local preparation SHA-256:
`5639f2667ea71b9001e88e6fcf76a9179101a21340b8f36f7bfe26ec86c494fd`.

Main's tested save-only Rust inventory SHA-256 is
`8781129422befa76ef7db9f6cca68a4709693d6b4aace91a0b84d89ef30b23a8`,
covering 336 audited production source files. The six reviewed Rust files remain
equivalent after Git's new-file newline normalization. Main build passes in
25.473 s wall; full tests pass 2,586 unit plus 3 integration, with 29 ignored
(56.71 s unit, 135.062 s total wall); Clippy passes in 23.192 s wall. Concurrent
leaf compilation was active, so these are check times rather than isolated
performance controls. Canonical warning-message/path comparison has 334
occurrences and 195 unique pairs on both baseline and current logs, with zero
additions or removals. The inventory's 12 tests and `--check` also pass; route
counts stay 182 routed and 285 missing. Main logs are
`log/windows-save-main-{build,tests,clippy}.txt`, timing JSONs, diagnostic
comparison and inventory logs. No plugin ABI/host or vendor change occurred.

## Column geometry

Five registered-action tests failed before production changes: extreme virtual
columns caused immediate Copy/Paste capacity overflow and Delete arithmetic
overflow; a stale rectangle changed the clipboard to a truncated result;
read-only Cut retained text/history but changed clipboard and selection geometry.
These are synthetic hostile-state failures reachable at the caller, rather than
reproduced ordinary user/file-triggered crashes.

Approved policy: validate geometry and all row plans before changing clipboard,
selection, transaction or text. Derive tentative Delete/Backspace/line-part
rectangles locally. Reject invalid rows and overflow, use character boundaries
and fallible reservations, and preflight case expansion. Read-only/loading/view
guards precede planning and publication.

This safety leaf uses temporary foreground limits: 128 KiB of cumulative
document payload/clipboard/removed-history work, at most 4,096 rows, and a checked
1 MiB total plan/metadata allocation budget. Exceeding a limit must leave the
original document, raw rectangle, stream selection, clipboard and histories
intact with an actionable status. Large-column background/input/IME queuing and
mutating bookmark rectangles remain separate parity work.

Local leaf: `../notepad-studio-agents/windows-column-safety`;
baseline evidence: `gates/G1-red.log`, 0 passed / 5 failed, 0.10 s.
The first candidate rerun passes all five regression tests in 0.12 s. Two more
registered-action baseline assertions failed in 0.04 s: classic uppercase and
lowercase changed text outside the selected rectangle. These are behavior
failures, not crash reproductions. The root approved routing those two actions
and their existing saved events through the same bounded case planner. Linear
behavior and persisted event names remain unchanged; final controls are pending.
The unchanged word-delete branch also has two registered-action REDs: read-only
Left clears the rectangle; stale Right clears it and deletes stream text, with
history effects. Those tests ran after the other candidate guards were present,
so they prove the unchanged branch rather than a clean whole-app baseline.
Only guards and a checked bound on its existing stream fallback are approved.
The six extended case commands' missing macro recording remains separate work.
The frozen candidate's 32 focused controls pass in 2.40 s. Root review requested
one correction before final acceptance: ordinary macro Paste should reuse its
captured clipboard text rather than reading the OS clipboard twice. This is an
introduced code-path cost, not a reproduced native clipboard failure. The
correction now passes 35 column controls, 13 macro-context controls and nine
macro-Undo controls. The first full run's sole failure came from a test fixture
marking its workspace closed; scoped persistence sinks replace that artificial
state. The final frozen full checks remain pending.

## Physical lines

Twenty genuine output assertions failed across VT, FF, NEL, line separator and
paragraph separator, with four CRLF/LF/CR/empty/EOF positive controls passing.
MoveDown, BookmarkToggle, Delete Line and all six registered rectangle sorts
treated Unicode separators as row boundaries. The oracle defaults to physical
CR/LF; PowerEditor contains no call enabling Unicode line-end types. These are
behavior failures, not product crash reproductions.

Two additional baseline tests use the real Find in Files button, worker and
result-navigation handler against neutral temporary files. All five separators
split a match's displayed row/snippet incorrectly. A regex match on the literal
separator at EOF navigates to an empty range rather than the separator's exact
bytes. Both assertions failed in 14.01 s; log SHA-256:
`d24b4af2a1abefa6cbcce174cac886feacc21988682970700423bea97a110ad8`.
Earlier incorrect Source-action dispatch and whitespace-query fixtures are
excluded from defect evidence.

Approved policy: disable Ropey's defaults, retain `cr_lines` and `simd`, and keep
the locked version. This changes indexing without transforming stored bytes.
Align the two Find in Files break/trim helpers only after actual result-content
and navigation controls. Keep word-character classification unchanged. Retain
Unicode/hostile-offset coverage while replacing older Unicode-row expectations
with explicit physical-row contracts.

Local leaf: `../notepad-studio-agents/windows-physical-lines`.
Baseline log SHA-256:
`a239a3e2f3e8051e651366d6ea6e86da2a810349754f6ef5efc0945d98b2c0db`.
The local proposal pins studied oracle files and confirms that the app alone
enables Ropey features. Required compatibility controls include all encodings,
byte-identical saves, Undo/Redo, restored selection bytes and bookmark
normalization, saved macros, Reader/rich/CSV views and over-10-MB streaming.

The isolated candidate focus passes 36 controls with one optimized control
ignored in 6.68 s. Its separate real streaming prerequisite passes in 2.66 s,
opening exact 14,240,003-byte UTF8 BOM and 25,280,002-byte UTF16 BE files. Fixtures
install scoped recent-file state before window creation. The final candidate
patch SHA-256 is
`fa56bd046f6e6090ece6d3a810ed4de605ff0c0a90a2cc41f29d2da44a3f212a`.
Its debug build, 2,605 unit plus three integration tests (30 ignored), and Clippy
pass. Canonical diagnostics match the baseline's 334 occurrences and 195 unique
message/path pairs. The optimized streaming controls are still pending.
One saved-macro fixture incorrectly expected Undo to recreate a removed bookmark;
it is excluded from product-defect evidence. The oracle's
`Notepad_plus.cpp:3060-3068` deletes the marker before the row text, and Scintilla's
`CellBuffer.cxx:1159-1183` restores text without a marker snapshot. Re-establishing
the bookmark before replay tests the intended saved-event contract. The fixture
correction adds no product behavior change. A legacy case test also encoded
Unicode-row indexing; its expected case changes and final-row geometry now
express the approved physical-row contract. That fixture correction adds no
production behavior beyond the Ropey policy. Main integration is pending.

Each leaf must pass its required frozen checks before integration. Native
Windows verification remains pending, Mac tests are deferred, and the overall
Notepad++ parity goal remains active. Local gates/logs remain ignored.

Storage for checks: Cargo cleaned the verified inactive
`startup-background-reader/target/agent-startup-reader-debug` cache (24.9 GiB),
after confirming its absolute path, Cargo marker and lack of active process
references. Current caches, source, local evidence and release artifacts remain
available. C: had approximately 42 GB free afterward.
