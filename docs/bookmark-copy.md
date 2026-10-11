# Copy Bookmarked Lines

Choose Search > Bookmark > Copy Bookmarked Lines to copy marked physical lines
in document order. Each line appears once, with its existing line ending; a final
line without a terminator gains none. Loaded CRLF and CR documents keep their
line endings on the clipboard. No bookmarks, or only an empty final line, clears
the clipboard, matching Notepad++.

The command works in the focused Source or Raw View editor, including read-only
documents. It preserves text, selection, encoding, bookmarks and Undo history.
Wait for loading or IME composition to finish before copying.

Large copies prepare in the background. Escape cancels. Changes to the text,
bookmarks, line endings, selection, tab or focus invalidate the pending result.
Editor Copy, Copy All, Copy Marked Text and file/path copies from the tab popup
take precedence. Output is limited to 128 MiB and the document may have at most
1,000,000 bookmarks.

Copies can be recorded and replayed through the Macro menu. Replay requires a
writable editor and uses its current document and bookmarks. Playback waits for
a background copy before executing the following event. Cut also takes
precedence over a pending copy. Text after the first NUL in each bookmarked line
is omitted from the clipboard, matching Notepad++'s text clipboard conversion.

Behavior references: `PowerEditor/src/Notepad_plus.cpp:2962–2976,3094–3106`,
`NppCommands.cpp:2389–2391,4563` and `MISC/Common/Common.cpp:976–1025`.
The implementation is original Rust.
