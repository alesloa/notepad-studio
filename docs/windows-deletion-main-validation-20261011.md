# Windows deletion Undo integration

Ordinary Backspace and Delete in Source and Raw View now share adjacent Undo
entries using Notepad++'s editing-byte rules. Every removal, revision and first
caret remains in history. Required storage is reserved before deletion; an
essential reservation refusal leaves the document intact and stops saved macro
playback before later events. The existing history codec and macro event names
remain readable.

The [behavior checks](windows-deletion-undo-validation-20261011.md) describe the
oracle rules, Unicode/CRLF cases, Save boundaries, refusal controls and input
routes. The nine-file worker delivery was composed on `c5221e7`, whose product
source matches the previously verified `ba9f296` typing batch. Root verified all
46 primary archived inputs, 743 compiled application inputs, 16 oracle inputs,
four retained executable/debug artifacts and the exact delivered files.

The original review patch decoded Git's UTF-8 output through an ANSI code page,
damaging two context headers. Its apply check failed without changing main.
The archived Rust and document bytes were correct and were integrated after a
clean-tree/base check. A separate UTF-8 patch passes its reverse apply check;
the faulty transport remains retained for diagnosis.

Main required Windows checks passed with LLVM `clang-cl` and the locked app
dependencies:

- `cargo build --locked`: exit 0, 33.396 seconds.
- `cargo test --locked -p notepad-studio`: exit 0, 2,917 unit plus 3 integration
  tests passed; 32 intentionally ignored. Whole-command duration: 232.306 seconds.
- `cargo clippy --locked -p notepad-studio --all-targets`: exit 0, 22.285 seconds.
  Its 332 located warnings and 193 distinct message/first-path pairs match the
  preceding typing batch exactly. Seven summaries without locations are excluded;
  there are no added pairs, occurrences or new touched-source warnings.

All 998 frozen inputs were unchanged throughout those commands. Root separately
checked the 850 Rust/manifest/embedded-asset inputs after documentation
generation. Check durations are debug validation evidence. They do not measure
native input latency.

The parity inventory regenerated successfully; its check and all 12 parser
tests pass. It covers 354 production Rust files with combined SHA-256
`c396575e9168704bacf008f97f2b56705527662e118212a4cd1d70fe4d90d80a`.
Static routing remains 196 routed and 271 missing command IDs. This change fixes
existing deletion behavior and does not add a menu route.

Root verified the separate optimized history/storage control's 24 processes,
48 empty successful competitor inventories, 150 retained evidence files and
22 final source identities. Independent complete byte vectors and all eight
three-run medians match. Coalescing preserves every operation and reduces the
large model's working set; it adds deletion work, and some Redo costs rise.
The constructed model excludes GPUI, annotations, macros, persistence and
native keyboard costs. See the [performance record](windows-deletion-undo-performance-20261011.md).

Retained main identities:

- Source freeze: `b44dd3436a77f1594454bc8c17087be1c58e44201956c5128fa1f6e87205c5ee`.
- Build log: `d434a41ad86aa0d130c38aa813e0b98968ab67167de2fe78e30b206e2d4ce05b`.
- Test log: `2c944b8ae4058c3b2fa0e6b48c31407938ff3e8b34795e4d9babc1f16df00b63`.
- Clippy log: `c684afb81c5bb0e94a3b318d99fdd40446a4723dda1f829a658430957a9553f2`.
- Retained test EXE: `a160ffff8f7a899c95d10adcf8c83feae66f2c6702df16636ffa4aedf68e1dcb`.
- Optimized evidence manifest: `ac3e030d06eac3f0ce055d69799e34797375446b3132ca2564b811373d3c89a7`.

Native Windows visual/input/IME verification, arbitrary ACP/DBCS behavior and
actual held asynchronous deletion Save success/error/cancel remain open. Enter,
Tab and completion boundaries need their own oracle checks. Mac testing is
deferred. No push or release publication accompanies this integration.
