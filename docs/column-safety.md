# Column operation limits

Column Copy, Cut, Delete, Backspace, line-part deletion, case conversion,
text input and same-session column Paste validate the whole request before
changing text, selections, clipboard or history. Stale row coordinates are
refused instead of truncating the rectangle. Read-only buffers allow Copy.
A refusal keeps the original stream selection, raw rectangle and Undo/Redo
entries and reports a reason in the status bar.

These paths currently run in the foreground. Each request has temporary
limits of 128 KiB of cumulative text work, 4,096 rows and 1 MiB of plan/history
metadata allocation. Text work includes virtual-space padding, clipboard
copies and removed/inserted history strings. Case conversion charges a
conservative expansion allowance before running any transform. Existing open
edit-group metadata is included when its history vector must grow. A smaller
rectangle or a finished edit group can bring a request within the limits.
These limits intentionally refuse some large column operations that Notepad++
accepts. Background column planning remains unfinished.

Normal edits retain character-based columns, virtual-space padding and row
direction. Each accepted multi-row edit creates one Undo step. Saved
`InsertText`, `Uppercase` and `Lowercase` events now use the same column path
when the current document has a rectangle; their persisted names are unchanged.
Linear playback keeps its existing selection behavior. A refused column event
stops playback before its remaining events. Mixed prepared and legacy macros
retain their existing history merging rules and limits.

IME composition with an active rectangle remains unavailable. Committed native
text input supports rectangles; ordinary stream composition still commits as
one Undo step. Six extended case commands use the same safety preflight, but
their separate macro-recording gap remains open. Cross-application column
clipboard formats, ragged word deletion and broader column keyboard parity
also remain unfinished. Ctrl+Word Delete retains its existing stream fallback:
it validates the rectangle and bounds the word range before clearing column
mode. Mutating bookmark commands still refuse rectangles pending a separate
prepared edit/remapping design.

Plans use fallible reservations for their vectors and payloads and reserve
history slots before the first row changes. The legacy buffer editor still
allocates removed/inserted history strings and Rope nodes through infallible
allocation. Their payload sizes are bounded here; this change does not provide
a global allocator-failure guarantee. Existing post-edit highlighting and
annotation maintenance, OS clipboard reads/writes and the saved-macro store's
ownership copies are outside the plan budget.

## Evidence and behavior sources

The safety regressions inject synthetic hostile coordinates through registered
editor actions. On baseline `088e8d2`, five controls failed: Copy/Paste virtual
padding overflowed capacity, thin Delete overflowed `usize`, stale-row Copy
changed the clipboard, and read-only Cut changed both selection geometries.
No ordinary user gesture or file-triggered crash was reproduced. The Delete
arithmetic failure was observed in a debug build; release overflow can wrap.
The checked preflight rejects both cases without relying on panic handling.

Two further controls reproduced classic case conversion acting on the stream
instead of its rectangle. Two Ctrl+Word Delete controls reproduced lost geometry
on read-only refusal and text/history changes with stale geometry. These were
run before changing those branches, with the other column safety fixes already
present. Rectangle `InsertText` replay is a consistency contract verified through
saved/reloaded events; it is not reported as a reproduced baseline crash.

Behavior was studied in the local Notepad++ checkout, especially Scintilla
`Editor.cxx:2336–2347` (Cut/Paste read-only guards), `2344–2408` (rectangle Paste)
and `4505–4525` (rectangle Copy), and
`ScintillaEditView.cpp:3835` (per-selection case conversion). The Rust planner
and tests are original Apache-2.0 code. Notepad++'s rectangle Paste can extend
the document; the existing Notepad Studio path still truncates clipboard rows
past EOF. This leaf preserves that difference.

Windows handler tests use neutral in-memory windows and scoped settings,
macro and recent-file overrides plus a session snapshot sink. No user
configuration is written. Native interactive UI verification, macOS and Linux
execution remain outstanding.