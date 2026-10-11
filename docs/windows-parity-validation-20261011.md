# Windows parity checkpoint, 2026-10-11

This batch adds 17 main-menu commands: six File routes, five bookmark commands,
and six line sorts. It also adds their guarded background preparation, clipboard
ownership, Undo/Redo and macro continuation. Full Notepad++ parity remains open.

The combined Rust source is identified by inventory SHA-256
`0b28fcc1c0ac5f2c2b5d4c6f005aa3857ffe8cf656c7cdc3b597b9d6d0dfd7d3`.
Root reviewed and integrated the isolated workers' patches before checking main.
The Rust source remained unchanged through the checks below.

## Combined Windows checks

Both `CC` and `CXX` pointed to `C:/Program Files/LLVM/bin/clang-cl.exe`.

| Check | Result |
| --- | --- |
| `cargo build` | Exit 0; 63.083 s wall. |
| `cargo test -p notepad-studio` | 2,574 unit and 3 integration tests passed; 29 ignored. Unit run 70.81 s; total 193.337 s wall. |
| `cargo clippy -p notepad-studio --all-targets` | Exit 0; 33.416 s wall. Inherited 195 app / 209 test warnings, including 191 duplicates. |
| Inventory generator tests | All 12 passed, including a regression for stale popup-only classifications of fully wired Rename/Delete routes. |
| Inventory `--check` | Passed on the combined source. |

The parent compared warning headings and normalized file paths against the
previous combined Clippy log: 334 diagnostic occurrences before and after, with
zero added pairs. This count includes dependencies. The repository is not
warning-free. No plugin ABI, host API or vendored code changed.

Local logs are `log/windows-parity-main-{build,tests,clippy}.txt`, the times file,
and `log/windows-parity-main-diagnostic-comparison.json`. These logs are ignored;
this document preserves their essential results.

## Default release controls

Each worker built its frozen source with the shipping profile: optimization
level 3, thin LTO and one codegen unit. Root's combined checks verify the shared
integration. The isolated release binaries verify their corresponding callers;
they are not a combined main release executable. Book's final common production
code matches the sort binary's consumed common code; two borrowed test-only file
differences are documented in [line sorts](windows-line-sorts.md).
The Rust test harness uses unwinding; these controls measure the shipping
optimizer settings and caller behavior, rather than process-crash recovery.

All three release builds completed before measurements. Workers measured File,
then Bookmarks, then Sort, with no concurrent compiler or linker activity. Every
deciding control ran three times. Earlier empty-output launch attempts were
discarded. These are harness measurements, not native input or painting latency,
and do not establish a whole-app speedup.

| Control | Input and accepted measurement |
| --- | --- |
| Save Copy | 13,631,488 input bytes, 14,680,064 faithful CRLF output bytes. Blocking writer median 21.204 ms; complete background caller median 47.446 ms. |
| Cross-volume Rename | 14,680,064 bytes. Worker preparation median 16.500 ms; preparation plus installation median 17.365 ms. |
| Edited native Copy cancellation | 13,631,488 text bytes. First bounded capture turn median 0.393 ms; request through visible cancellation median 0.708 ms. |
| Bookmark edits | 12,582,960-byte edits with Undo/Redo and cancellation/staleness controls; 12,096,000-byte Cut and 10,500,000-byte clipboard Paste also passed. Capture maximum 0.248 ms; publication maximum 12.758 ms. |
| Mixed bookmark macro history | Existing history plus ordinary and prepared edits retained one Undo. Capture turn maximum 0.042 ms; large histories yielded across 161/163 turns. |
| Case-insensitive sort | 12,599,999 bytes / 180,000 rows. Worker median 66.497 ms, capture 0.016 ms, publication 0.042 ms, complete caller 72.928 ms. Old synchronous lexical control median 41.848 ms. |
| Native locale mixed macro | Same large source. Worker median 165.480 ms, capture 0.006 ms, publication 0.046 ms, complete caller 188.160 ms; output and one Undo passed. |

Copy caller clocks combine capture, worker work and publication; they do not
separate those phases. Native cancellation's clock ends after caller-visible
state publication and excludes pending timer work and completed snapshot
disposal. Background disposal is supported by source review and cleanup controls,
with no independent disposal timing. Bookmark clipboard timings use GPUI's test
platform and do not measure the operating-system clipboard. Test dispatch can
eagerly run executor work; it is not native responsiveness evidence.

Per-caller ranges, hashes and exact local artifact paths are recorded in
[File validation](windows-file-routes-validation.md),
[bookmark editing](bookmark-edits.md), and [line sorts](windows-line-sorts.md).

## Remaining work

The regenerated inventory has 581 command appearances, 481 unique IDs, 45
separators and 275 catalog rows. It classifies 182 unique IDs as routed and 285
as missing, with 8 partial, 2 alternate-only, 2 action-only, 1 status-only and 1
stub. Routing is not proof of behavior parity. The separate
[behavior ledger](windows-parity-behavior.json) retains all native verification
requirements.

The supported native bridge again returned:

```text
Computer Use native pipe is unavailable: failed to connect native pipe: The system cannot find the file specified. (os error 2)
```

No native editor launch or input occurred. Completion placement, IME, clipboard,
dialogs and rendered behavior still need native Windows verification. Mac tests
are deferred by the user. Mutating bookmark commands still refuse rectangles;
basic mixed-EOL sorting follows the documented fidelity policy; macro grouping
can retain separate Undo steps when the full timeline exceeds 128 MiB.

Next safety investigations are interrupted-save recovery, invalid rectangle
geometry and Unicode versus physical line boundaries. Their suspected paths
have not yet been reproduced through deciding controls. See the
[work queue](windows-parity-work-queue.md). Local `gates/` remains ignored with
its files preserved. No push or release publication was performed.
