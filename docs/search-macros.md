# Search macros

Start recording from the Macro menu, perform Find Next, Find Previous, Replace
or Replace All, then stop and save the recording. New search events keep the
query, replacement, Normal/Extended/Regular expression mode, case and whole-word
flags, wrap, direction, dot-matches-newline, In Selection and the Replace option
that keeps the current occurrence. Saved recordings use those options when
played in a fresh editor.

Find Next and Find Previous retain their named directions. The Find dialog's
primary button follows Backward direction; Shift reverses it once, and the
recording keeps the resulting operation. In Selection uses the selection in the
playback document, rather than offsets from the recording document. Rectangular
search scopes are refused.

Playback requires the active, focused Source or Raw View editor to be writable,
fully loaded and outside IME composition. Regex replacements prepare in the
background and continue the remaining macro events only after a valid result.
Escape cancels. Editing, changing selection/search, switching tabs or focus, or
closing/restoring the document stops the remaining events. Completed prefix edits
remain available to Undo. An uninterrupted replay, including repeated runs, has
one Undo group; real input during a worker wait keeps its own history.

Each recorded query and replacement is limited to 1 MiB. An oversized operation
still runs normally but reports that its search event was not recorded. Regex
replay uses the existing [replacement syntax and work limits](regex-replacement.md).
Invalid patterns or replacement templates stop the macro with a status message.

Older parameterless `FindNext`, `FindPrev`, `ReplaceCurrent` and `ReplaceAll`
events remain readable, but lack enough information to replay. A macro containing
one of those events, or the unsupported `Save` event, is refused before any
prefix edit. Record the search again to capture its parameters. All Opened,
Find/Replace in Files and CSV/Doc View search recordings remain outside this batch.

Behavior references: `PowerEditor/src/ScintillaComponent/FindReplaceDlg.cpp:4328–4389`
and `PowerEditor/src/NppCommands.cpp:83–149`. The implementation is original Rust.
