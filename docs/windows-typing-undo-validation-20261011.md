# Windows insertion Undo checks — 2026-10-11

Adjacent plain text commits in Source/Raw View share a logical Undo entry.
The entry retains each original edit, its revision pair and the first caret.
Every accepted callback still updates content activity and flushes independently.
Growing runs append operations without cloning or concatenating the old text.
No transaction stays open between callbacks.

The behavioral oracle is the local Notepad++ checkout, with exact source bytes
retained in the review packet. `scintilla/src/UndoHistory.cxx:253–350` defines
adjacent insert coalescence and explicit group boundaries. Lines281–282 compare
the current action with the savepoint. Lines565–622 retain coalescing flags during
Undo/Redo. `scintilla/src/Editor.cxx:2087–2094` leaves a plain point insertion
outside an explicit group. `scintilla/win32/ScintillaWin.cxx:2025–2037` routes
Windows character input through `AddWString` at1522–1533. The Rust implementation
is original; oracle code was used to establish behavior.

The baseline at4a5dad80 was exercised through a painted, registered GPUI Source
input handler, with ASCII a/b/c and Unicode é/中/emoji sent in separate turns.
Actual registered Undo left BASEab and BASEé中 respectively. Registered Redo,
revision and annotation assertions passed before those failing Undo assertions.
One-callback abc, model savepoint and registered Paste controls passed. The
receipt7753d2f3 pins the26 app/oracle source identities, two test-only overrides,
baseline archive, log and deciding EXE/PDB. Production starts from committed
f62f750, which adds the reviewed Hash context without changing the typing path.

Required inserted and removed Undo strings are reserved and copied fallibly
before any edit. Removed text is copied from RopeSlice chunks. History storage
is also reserved first. An optional grouping allocation failure can use a
separate existing history entry; essential storage failure leaves text, both
histories, geometry, annotations, savepoint, source generations and macro
recording intact, with an actionable status. Rope and existing metadata/revision
allocators retain their existing limitations; this is no global OOM guarantee.

The runtime run descriptor survives Undo/Redo of the same entry. Copy, elapsed
time and a caret move away and back do not create arbitrary boundaries. A
nonadjacent insertion, established current raw savepoint or explicit edit group
starts another entry. Empty explicit groups also close eligibility. Restored
legacy histories retain their Group contents and begin with eligibility closed;
the existing codec does not store Scintilla coalescing flags. Saved macro variant
names and payloads remain unchanged.

`Tab::seal_typing_undo_revision` closes eligibility at an exact current raw
revision without allocation or stored-history changes. A failed/cancelled
captured Save retains this boundary, giving a conservative extra Undo separation.
History is never retroactively joined. Settlement of an earlier captured marker
does not split a newer typing run. These are API/settlement model controls in
this leaf. Actual held asynchronous Save success/error/late-typing reachability
is owned by the File leaf and remains an integration gate.

Delayed macro history stamps compare revision, activity, depths, operation count,
first/last revision endpoints, first caret and Prepared Arc identity. Existing
prepared/edit/reload/quit guards continue to use their stronger source identities.
Registered held sort controls cover same-depth extension, Undo/Redo ABA and
Cancel; a held background bookmarked Copy must preserve its clipboard sentinel
after same-depth typing. All test preferences, recents, macros, session and
clipboard ownership are scoped before mounting a live synthetic window.

The fidelity controls open real neutral temporary files for UTF-8, UTF-8 BOM,
UTF-16 LE/BE and strict Windows-1252 with LF/CRLF/CR. Undo restores exact encoded
bytes and the clean revision, and Redo restores the edited bytes. Other controls
cover the existing synchronous registered Save, Raw View/native history
separation, recording/playback, original operation ordering and the Group codec.

Deletion coalescing, keyboard Enter/Tab helper boundaries, completion and IME
coalescing rules require their own oracle audits. Their existing paths remain
unchanged. Native Windows character/IME observation and Mac checks are pending.
The installed GPUI protocol controls do not establish those native results.
Required build/full tests/Clippy and the reviewed optimized storage scope are
recorded in the final evidence suffix after completion.
