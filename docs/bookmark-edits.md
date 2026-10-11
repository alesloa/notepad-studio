# Bookmark editing

Search > Bookmark includes Cut Bookmarked Lines, Paste to (Replace) Bookmarked
Lines, Remove Bookmarked Lines, Remove Non-Bookmarked Lines, and Inverse
Bookmarks. They have no default keyboard shortcut and can be rebound in the
Shortcut Mapper.

Cut copies the bookmarked physical lines in document order, including their
terminators, then removes those complete lines. With no bookmarks it clears the
clipboard. Remove performs the same deletion without changing the clipboard.
Remove Non-Bookmarked Lines retains the complete bookmarked lines. Removing an
unterminated last line leaves the preceding line ending in place. An empty final
line can have its own bookmark.

Paste replaces each bookmarked line's body with the entire clipboard text and
retains that line's original terminator. Clipboard line breaks are inserted as
supplied. Empty text removes the bodies; a clipboard without a text entry does
nothing. Text stops at its first NUL, as in Notepad++'s Unicode text clipboard.
Inverse Bookmarks exchanges bookmarked and unbookmarked physical lines,
including the empty final line. It works in read-only documents and creates no
text Undo entry.

Commands require the focused Source or Raw View editor, completed loading, and
no active IME composition or edit transaction. Editing commands also require a
writable document and a linear selection. Inverse Bookmarks also permits a
rectangle and preserves both selection geometries. Text-editing bookmark
commands currently refuse rectangle selections; their column remapping remains
part of the column editing audit. Planning runs in the background with
Escape cancellation. A changed document, tab incarnation, selection, focus,
annotations, encoding, line ending, savepoint, or history discards the candidate.
Cut and Copy retire older pending clipboard commands.

Each completed editing command creates one Undo entry. Undo and Redo prepare in
the background and leave their stacks intact until a validated result publishes.
Surviving annotations move with their text; annotations whose text was removed
or replaced are dropped. Removed bookmarks are not recreated by Undo.
Replacing a nonempty body with identical clipboard text still creates an edit,
leaves the savepoint and clears Redo, as Scintilla's basic target replacement
deletes and reinserts the text. Replacing an empty body with empty text is a no-op.

All five commands record and replay through saved macros. Playback waits for
workers and stops on a refused or cancelled event. Mixed ordinary edits and
prepared edits combine into one macro Undo. Legacy history capture copies at
most 64 KiB and 256 operations per UI turn; sealed history shares its immutable
body. The full Undo timeline is captured under a cumulative 128 MiB limit;
the worker retains its prefix and combines the macro suffix. Publication swaps
the completed timeline and releases the old one on a worker. A capture refusal
leaves the existing Undo and Redo stacks intact.

The foreground annotation snapshot is limited to 65,536 bookmarks and marks
combined. Inverse Bookmarks cannot create more than 65,536 bookmarks. Clipboard
output and edit history are each capped at 128 MiB, clipboard input at 4,096
entries, and plans at 262,144 separate
targets. Limit or memory failures report a status message before publication.

Behavior was studied in `PowerEditor/src/Notepad_plus.cpp:2979-3091`,
`Notepad_plus.rc:735-740`, `Parameters.cpp:306-311`, and
`NppCommands.cpp:2385-2407,4440,4562-4566`. Selection movement follows
`scintilla/src/Selection.cxx:54-80,131-143`. The Rust implementation is original.

Windows automated validation passed 2,527 unit tests and three integration
tests. The bookmark-focused run passed 24 tests covering actual menu and
catalog callers, saved/reloaded macros, mixed history, cancellation, stale
ownership, read-only rectangles, disk-loaded encodings, and identical-paste
savepoints. The normal build and Clippy passed with no new warnings in the
changed code. Tests use scoped settings, macro state, and persistence sinks.
Native GPU interaction and operating-system clipboard timing remain unverified
because native computer control is unavailable. macOS and Linux were not run.

Three quiet Windows release runs used the frozen test executable built with
`opt-level=3`, thin LTO and one codegen unit, on Rust 1.98.0
(`x86_64-pc-windows-msvc`). No Cargo, rustc or linker was active during the
measurements. The exact caller controls passed edit, Undo and Redo on a
12,582,960-byte Unicode CRLF document, plus cancellation and stale-result
controls for all four text commands. Median timings in milliseconds were:

| Command | Capture | Worker | Publication |
| --- | ---: | ---: | ---: |
| Cut Bookmarked Lines | 0.082 | 48.296 | 0.247 |
| Paste to Bookmarked Lines | 0.136 | 47.510 | 0.168 |
| Remove Bookmarked Lines | 0.070 | 43.925 | 0.197 |
| Remove Non-Bookmarked Lines | 0.182 | 48.850 | 11.540 |

Across edit, Undo and Redo, capture peaked at 0.248 ms and publication at
12.758 ms. Publication is not claimed constant time. Cutting all bookmarked
lines copied 12,096,000 bytes, with worker time 34.614-37.608 ms and publication
0.139-0.240 ms. The separately timed TestAppContext clipboard write took
0.002-0.003 ms; this does not measure the Windows clipboard. Pasting a
10,500,000-byte clipboard payload into the same source took 21.460-32.755 ms
on the worker and 0.029-0.046 ms to publish. Test action dispatch, including
clipboard ownership and executor work, took 24.205-36.386 ms.

Macro controls with 10,500,000-byte Single, Group and existing-prefix
legacy history passed in all three runs, including the combined macro Undo
and the retained prefix Undo. Capture took 161-163 turns, each limited to
64 KiB and 256 operations; the slowest measured capture turn was 0.042 ms.
Test macro dispatch took 20.583-47.769 ms including existing InsertText
playback and executor work. These totals do not isolate legacy edit cost or
native frame latency. The commands were absent in the baseline caller, so
there is no runnable command timing baseline.

The measured executable SHA256 is
`df439c85343bd52dc7adfbf21473e1c9bb54b2836b7e509c5944d247e2350cbc`;
the frozen composed source manifest SHA256 is
`37c1df80f5f6aa42121a0b4153a95c7ab5de01e63868c015bcc6195684c98f11`.
Local logs are `G5-optimized-bookmark-callers-run1.log` through `run3.log`
and the corresponding `G5-optimized-bookmark-macros` logs. The two exact
ignored controls live in `bookmark_edits_tests.rs` and
`bookmark_edits_macro_tests.rs`.
