# Windows parity audit, 2026-10-12

The integrated product source is `ec8ae9848e405902ba736dc20c18934628d17ab3`.
Its [deletion build and test results](windows-deletion-main-validation-20261011.md)
remain current. The following results concern isolated candidates. They do not
establish completed Windows parity or native UI verification. Nothing was pushed.

Root independently checked the archived source, compiled inputs, retained test
executables and logs described below. Detailed local packets stay in ignored
`gates/` and `log/`; this document preserves their essential findings.

The keyboard baseline ran ten registered GPUI controls across all 51 planned
Source/Raw contexts. Two functions passed and eight failed. Seven failures match
the local oracle's behavior: ordinary empty Enter should continue eligible
insertion Undo; Tab should use scalar columns and existing tab stops, replace
leading indentation correctly, and preserve multiline rows and selection
direction; Basic Enter should set suffix indentation without duplicating it;
recording/replay should suppress maintained indentation; refused read-only
Enter should record no successful newline, while accepted Tab intent remains
recordable. The eighth failure is a synthetic stale-rectangle caller control,
which must refuse safely while preserving both geometries and histories. It is
not evidence of a native or normal-user crash.

The two passing functions cover nine actual Save encoding/EOL cases, reversible
selected Enter, and old saved Enter/Tab unit names with explicit macro grouping.
The baseline receipt is `71e108462b56b5f330b6b0ab83e145a5be44eb5e4d568ab26ce51698ef9af4de`;
Root verified 39 primary source archives and all 762 compiled input entries.

A separate successful macro control ran nine source-to-target EOL cases across
18 mounted contexts with UTF8 BOM and UTF16 LE/BE. Actual Enter recording,
serialization/reload, replay into another EOL document and FileSave preserve
the existing `InsertNewline` unit. Expected transport bytes use the existing
tested encoder; this does not independently validate that encoder. The local
oracle's `PowerEditor/src/Parameters.cpp:4090–4163` converts saved single-EOL
replacement steps to `SCI_NEWLINE`, including CR/LF pairing. A new literal-EOL
persisted variant is therefore unnecessary. The suffix receipt is
`583a03947294fe3c4cb9ba5d49f03693d22841dd9166fff3038295a45d9ce1ee`.
Production keyboard corrections are underway in the isolated candidate. Valid
rectangles, Advanced lexer behavior and large commands remain part of the task;
temporary refusals cannot establish finished keyboard parity.

Automatic-title candidate 11 passes six deciding continuation functions plus
18 deletion and 24 typing controls. Root verified all 774 archived inputs,
13 evidence files and its 36-file owned patch against current product source;
all existing non-owned inputs match after newline normalization. The correction
observes final authoritative CSV Undo stamps, preserves each coalesced deletion's
notifications, and retains the earlier pristine-tab and IME no-op fixes. Its
source manifest is
`ef33a8a01558a10e65a97fb717052241b030b54778259c73b32628f3f4534989`.
Text, native AST, saved bytes, encoding, EOL, focus and history positives precede
the original CSV naming failure; the fix adds no extra naming notification.

The candidate's required build passed. The full-test runner then stopped on a
JSON parsing error while reading an inherited failing clipboard fixture.
That fixture enables the preference without an edit, then expects getters to
recompute a name. The verified oracle contract keeps the stored name until an
accepted notification. Child exit was not harvested, so this is no full-test,
integration-test or Clippy pass. The fixture and runner corrections require a
fresh frozen full run. Optimized controls and main integration remain pending.
Naming resource limits affect metadata only and must preserve document edits,
history, Save and recovery.

Registered Full recovery has two independently reproduced capture-budget
failures. A real Save preserved all 300 bookmarks, but the UI turn copied 128
bookmark values in addition to its guard, exceeding the 128-item allowance.
An equal live CSV cell of 155,648 bytes preserved popup Save, input and the
previous recovery, but requested 69,424 first-turn bytes and refused before
comparison. Both require using the shared turn's remaining allowance. The
later candidate passes these controls, including all bookmark values and
311,296 compared CSV bytes, but final coordinator migration is still open.

A separate typed-error control executed the actual protected Writer,
manifest/sidecar publication and matching cache installation. A scoped cleanup
fault produced `io::ErrorKind::Interrupted` after publication. Close ignored
that admitted error, published another generation and removed the window.
This is a genuine close-contract failure, not an OS interruption or power-loss
test. Root verified all 55 source archives and 64 evidence entries; the source
manifest is
`c07c4b002313652591fc230806599acea42d259841139fb2d59bf0c661c2d989`.
The earlier OS995 fixture never reached this path and is excluded: installed
Rust classifies that code as `TimedOut`.

The corrected isolated coordinator waits for disposal acknowledgements,
distinguishes explicit unadmitted cancellation from admitted Writer failures,
and refuses the failed close with its original actionable diagnostic. Focused
controls also pass queued cancellation, actual Save/autosave intake during held
close and later-edit refusal. A peer-window fixture needed painting before its
input could be accepted. The corrected setup now proves owner removal while
preserving the other window's actual unsaved text, revision and history.

That later ten-function run has nine passes and one genuine retry failure.
After the first storage error is reported and the fault removed, an immediate
second Close admits no fresh Full job. The old completed error is reported again;
the final retry Writer/removal assertions remain unreached. Its 55-source manifest
is `a20f1194f8eaf78f3d555e9daa3ebf683258e2d2d2f6d6367499ce89d9209003`.
The correction must acknowledge the reported error only after cleanup and actual
close refusal, then let a later explicit request capture a fresh Full point.
New Writer failures and repair fences must remain authoritative.
Ordinary later autosave may retry recovery while the refused window remains open;
it must not resume the failed close request implicitly.

Before integration, finish the retry control and preserve the current open/closed
custom-label metadata through the complete raw/native/Undo/CSV schema. Compose
naming settlement, document Save, final Full capture, cache publication and
window removal in one guarded close sequence. Startup, Exit, other windows,
request tickets and Copy/Move still need verification. Required combined build,
full tests, Clippy and optimized checks follow reviewed integration. Native
Windows tooling remains unavailable, Mac tests are deferred, and the inventory
still reports 196 routed and 271 missing unique IDs.
