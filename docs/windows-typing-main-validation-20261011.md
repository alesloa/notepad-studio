# Windows insertion Undo — combined main checks

Adjacent committed text input in Source and Raw View now undoes as one run,
retaining every original edit, revision and first caret. Required Undo text and
history storage is reserved before editing. Growing runs move operations into
an existing Group and append without copying earlier strings. Every accepted
callback still flushes separately.

The exact 15-file delivery is based on `f62f750991bb6a76194a35e7d7aa1dda064e8955`.
It preserves the integrated Hash, clipboard, brace, legacy-line and Source
context behavior. The final delivery manifest is `95d36f3412c9273d...`; its
receipt is `d0116ff7420811ac...`. Root verified every live/archive file, reviewed
the core edit/input/history changes and repeated the required Windows gates on
main. No dependency, plugin ABI or persisted history schema changed.

Combined checks used clang-cl for CC/CXX and the existing main target cache:

- Build: exit zero, 32.790 seconds.
- Tests: 2,899 unit and three integration tests passed; 32 ignored. Unit
  execution took 51.64 seconds; the complete Cargo command took 150.397 seconds.
- Clippy, all targets: exit zero, 20.924 seconds. The canonical rendered
  comparison has 332 located warning occurrences and 193 message/path pairs
  versus 331/193 at the parent. The sole extra occurrence is the inherited
  `session.rs` large-enum warning after app/test Tab sizes change. There are no
  new warning pairs or added warnings in the changed files.
- Inventory regeneration/check and all 12 parser tests passed. The 354-file
  Rust source identity is `e9328ac02a9aff05b8e7bd515c8a8874e9ec1d1cd4a4e68548e533ce979f94c2`.
  Static route counts remain 196 routed, 271 missing and 289 CATALOG entries.

All three Cargo commands preserved the frozen checked inputs: 995 files,
combined SHA-256 `25752679e1d61779aeed107c098f00ca3650b4404b2e3aec06d0ab806efe2d2c`.
Local ignored records retain the logs, exact test EXE, input hashes and Root
review. Generated inventory and this document were recorded after the gates;
the accepted implementation files remain byte-identical to the delivery.

Root also verified all 118 optimized evidence files, 18 successful terminal
results and 36 empty before/after process inventories. Independent Python
fixture/checksum values and six recomputed median groups match the packet.
The [optimized storage report](windows-typing-undo-performance-20261011.md)
records the constructed Singles reference, timing/peak scope and small
short/fenced insertion overhead. These measurements exclude native dispatch,
GPUI painting, annotations, macro/persistence work and actual history validation.

The [behavior notes](windows-typing-undo-validation-20261011.md) describe the
installed callback regressions, physical encoding/EOL cases, unchanged legacy
Group codec, Raw/native history separation, macro recording and same-depth
worker guards. This main gate covers the current synchronous Save path and
captured-save API/model. The File leaf's held asynchronous Save/Save As/Save All
checks still need final composite verification with coherent Full recovery and
close/Exit settlement. Automatic-title integration remains separate. Ordinary
deletion, keyboard Enter/Tab and completion coalescence require their own oracle
checks. Native Windows input/IME/visual observation and Mac testing remain open.
Existing Rope, annotation/general-command and recorder allocation limits remain;
checked typing/history storage is not a global OOM guarantee.
