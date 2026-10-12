# Source and Raw deletion Undo checks

Ordinary Backspace and Delete with an empty stream selection now coalesce in
Source and Raw View. Each deletion keeps its own `EditOp`, revision and removed
text; the shared Undo entry restores the first caret. The saved history and
macro event names are unchanged.

The behavior oracle is the local Notepad++ checkout, especially
`scintilla/src/UndoHistory.cxx:253–350,565–622`. A new removal of one or two editing
bytes can join when it starts at the previous removal's position or ends there.
The previous removal can be wider: a three- or four-byte scalar starts a run,
and a following short removal can join it. Direction changes, elapsed time,
caret movement and Copy do not independently seal a run. The new position must
still satisfy the adjacency rule.

The current raw savepoint, an exact captured-save seal, explicit edit groups
and a change between insertion and removal separate runs. A failed or cancelled
captured Save retains its sealed boundary. Publishing an older saved revision
does not split newer adjacent edits. Undo/Redo retains the eligible endpoint,
while content activity and history endpoint stamps reject delayed replies after
same-depth extension or an Undo/Redo round trip.

`Editor.cxx:2415–2443,2496–2536` distinguishes ordinary deletion from selected,
multiple and virtual-space deletion. This change leaves selected, rectangular,
word, helper and IME edits on their existing separate paths. The default
unindent-on-Backspace setting is off (`PowerEditor/src/Parameters.h:737`);
this is not a new unindent implementation.

UTF-8 scalars and complete CRLF pairs follow
`Document.cxx:756–780,1739–1753`. Notepad++ loads Unicode disk encodings into UTF-8
editing bytes (`PowerEditor/src/Utf8_16.cpp:123–206` and
`ScintillaComponent/ScintillaEditView.cpp:2476–2495`). Tests distinguish those
widths from UTF-16 disk size and preserve CRLF/BOM/UTF-16 transport bytes. They
do not establish arbitrary Windows ACP or DBCS parity.

Required removed/inserted history strings and the ordinary history slot are
reserved before mutation. Optional coalescing reservation failure falls back to
a separate entry. Essential storage refusal preserves the document, selection,
history, savepoint and input generations and records no unapplied event. Saved
macro playback stops at that refusal. Valid BOF/EOF/read-only key intents remain
recordable, as Scintilla records accepted key commands before execution
(`Editor.cxx:6300–6302,3018,3076`); those saved commands execute in a fresh editor.

The initial installed-protocol baseline had two actual failing grouping
functions and one passing single-deletion function. Both Source and Raw cases
executed before the failing assertions. The first candidate phase passed all
17 deletion functions and all 24 existing insertion functions. Its compiled
source manifest was `ecddd866c51eb9d48369d8bdf048276284bf578ed3ba095f70a8c30e1522c578`.
The final phase adds a legacy-codec round trip checking every retained removal
and revision chain. Its full frozen suite passes all 18 deletion functions,
2,917 unit tests and 3 integration tests, with 32 intentionally ignored. Required
build, full tests and all-target Clippy passed in 53.644, 111.376 and 36.384 seconds
respectively; these are debug check durations, not performance measurements.
Rendered Clippy diagnostics match the accepted base exactly: 332 located
occurrences and 193 distinct message/first-path pairs, with no additions or
removals. Compiled source drift was zero. The consumed source manifest is
`b85a8b55ca230ee587585f760c65e9e119442c80a534d872af0b73e7caec6cf2`.

Two setup/compile corrections were not behavioral failures: a test used a
nonexistent Buffer accessor, and a freeze script initially decoded Japanese
fixture text using the process ANSI default. Neither failing attempt executed
controls. After the successful broad commands, artifact retention guessed the
app PDB's hyphenated name; a separate retention step used the actual underscore
name and verified both app and test EXE/PDB identities. No successful check was
repeated for that packaging correction. A later patch transport check found
ANSI decoding of Git's UTF-8 output had corrupted two existing context lines;
the source archives were correct. A separate UTF-8 patch passed the reverse
application check, with all nine source hashes unchanged. This was a transport
artifact correction, not a source or compilation failure.

The controls use painted GPUI windows, installed input callbacks, registered
catalog-bound keys, real Undo/Redo and saved macro serialization/reload. Scoped
settings, recent files, macro execution, clipboard ownership and session sinks
isolate user state. They cover Unicode widths, CRLF/lone endings, direction
switches, movement/Copy, insertion/selection/word/IME boundaries, Save models,
registered synchronous Save, allocation refusal and held sort/bookmark-copy
workers. Native Windows input and macOS observation remain pending. Actual held
asynchronous deletion Save success/error/cancel remains a separate composed
File-caller check.

Rope allocation, annotation updates and legacy general-command/explicit-group
publication still use existing allocation paths. This change provides no global
out-of-memory guarantee. Native UI performance remains unmeasured. The separate
[optimized history model](windows-deletion-undo-performance-20261011.md) passed
24 processes on 12.6 MB with no source drift; it reports added deletion costs
and lower peak memory within its stated scope.
