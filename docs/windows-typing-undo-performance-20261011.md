# Windows insertion Undo validation results

Adjacent ordinary Source/Raw insertions now share a logical Undo entry while
retaining every original operation, revision and first caret. Each callback
still flushes independently. The implementation and oracle contract are in
[the insertion validation notes](windows-typing-undo-validation-20261011.md).
Deletion, keyboard Enter/Tab, completion and IME coalescing remain separate work.

The frozen leaf is based on `f62f750991bb6a76194a35e7d7aa1dda064e8955`.
Its final 24 focused controls passed. Required build passed in 9.097 seconds;
full tests passed with 2,899 unit and three integration tests, with 32 ignored,
in 58.339 seconds total. Unit execution took 49.70 seconds. Clippy passed in
38.291 seconds with zero new message/path pairs or touched-path warnings.
The rendered comparison is 332 occurrences/193 pairs against 331/193. The
extra inherited `session.rs` occurrence reflects different app/test Tab sizes.
No production padding or boxing change was made to alter that diagnostic.

The installed GPUI tests exercise painted handlers and registered Undo/Redo,
recording, saved playback, source stamps and held worker publication. Fifteen
disk cases cover UTF-8, BOM, UTF-16 LE/BE and Windows-1252 with LF/CRLF/CR.
These are installed protocol controls, not native Windows character/IME proof.
The leaf tests the captured-save seal and settlement model. The File leaf
separately owns held registered asynchronous Save/Save As/Save All controls;
their final combined-main verification remains an integration requirement.

One standalone helper compiled with the unchanged production
`workspace/typing_undo.rs`, model Tab/Buffer types and Rope operations. The
constructed Singles reference uses the same fallible text preparation and Rope
edits with a fresh history entry per callback. It is not an old shipping or
native baseline. The locked closure contains the app's exact Ropey 1.6.1,
smallvec 1.15.1 and str_indices 0.4.4 versions/checksums. The release profile is
opt3, thin LTO, one codegen unit, panic abort and stripping. The build took
1.664 seconds and produced no warnings.

The source fixture is 12,600,000 UTF-8 bytes. Exactly 18 deciding processes ran
sequentially during a Root-coordinated quiet window. All exited zero, all 36
fresh before/after inventories were empty, and no source drift or restarted run
occurred. Independent Python checksums agreed with full before/after/Undo/Redo
content and the captured fence endpoint. Each mode/case has three repetitions.

Median insertion/Undo/Redo times in milliseconds are:

- 4,096 short Unicode callbacks: Singles 1.081/1.278/1.085;
  grouped 1.153/1.110/1.048. Logical entries fall from 4,096 to one.
- 256 callbacks of 64 KiB: Singles 11.617/3.325/8.267;
  grouped 10.140/2.379/7.439. Logical entries fall from 256 to one.
- 4,096 short callbacks with a midpoint seal/savepoint: Singles
  1.039/1.214/1.034; grouped 1.232/1.146/1.040. The grouped version retains
  two entries; Undo reaches the exact clean captured revision before original
  text. Both versions retain all 4,096 operations.

Whole-process peak working-set/pagefile medians in MiB are 31.742/28.246 versus
31.395/27.910 for short callbacks, 66.863/64.336 versus 65.344/63.086 for 64 KiB
callbacks, and 31.742/28.254 versus 31.309/27.836 for the fenced case. These
peaks include file/Rope/input preparation and all checksums. Insertion timers
include text reservation, history planning/storage, Rope edits and revision
identities. Inverse/forward timers cover the same physical operations in both
representations. Checksums and initial preparation are outside those timers.
GPUI, annotations, title notifications, source guards, macro/persistence work,
actual history validation and native paint/dispatch are outside this helper.
Short/fenced insertion has a small additional cost; these controls do not show
a universal speed improvement.

The immutable final source manifest is `8052747d...`, broad receipt `f8eea12f...`,
corrected helper manifest `62dc8048...`, and measured helper EXE SHA-256 is
`6a6044b89cf6138fd97bef34ce2c1c29c2ee4cb7bd9a1c184579207bd832c7eb`.
Local ignored evidence retains the original REDs/fixture corrections, exact
source archives, logs, binaries, all 18 terminal receipts and 36 inventories.
The source remains frozen and both borrowed caches are idle. Native Windows,
Mac and composite async-save verification remain open. Existing Rope,
annotation/general-command and recorder allocators retain their limitations;
fallible required typing/history preparation is not a global OOM guarantee.
