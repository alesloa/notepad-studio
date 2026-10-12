# Windows matching-brace work

Both Search commands are now wired through actions, menus, Shortcut Mapper and
saved macro playback. Go to Matching Brace also has a toolbar button. The
painter and commands share a cancellable background matcher and cache both
matches and misses. Navigation changes no document bytes or text history.
Main build, full tests, Clippy and inventory checks pass; the
[validation record](windows-brace-validation-20261011.md) records their exact source
and remaining limits.

The local Notepad++ oracle specifies before-caret brace priority, with an
at-caret fallback only when there is no brace immediately before the caret
(`Notepad_plus.cpp:3107–3169`). An unmatched before-caret brace does not fall
back. A linear selection does not disable matching. Matching compares the
origin's lexer style (`scintilla/src/Document.cxx:3034–3070`), including matching
within a string or comment. Notepad Studio's HighlightKind currently combines
several lexer styles; matching those broad classes cannot establish exact
per-lexer parity.

Go to Matching Brace moves to the partner's byte position. Select All In-between
{} [] or () includes both delimiters and orders the earlier endpoint as anchor
and the later endpoint plus one as head. Both update the preferred caret column
(`NppCommands.cpp:1940–1958`). Their default Windows shortcuts are Ctrl-B and
Ctrl-Alt-B (`Parameters.cpp:261–262`); On macOS, Control-B preserves the existing Cmd-B Markdown Bold binding; Select uses Cmd-Alt-B. Mac testing is deferred.
The menu positions and labels are at `Notepad_plus.rc:664–665`. Go to Matching
Brace has a toolbar route; Select does not. The macro whitelist contains both
commands at `NppCommands.cpp:4434–4435`. Implementation must be original Rust;
the oracle is read for behavior only.

Root ran a frozen test-only baseline against main commit
`da0ea709631d13e2ce10e5c5a239a185ad04ed05`, whose product sources equal
`b42c889f8d37f09b244bab8037ad31a4e82caede`. Four helper assertions failed:
at-caret fallback, matching inside one string style, matching inside one comment
style and matching with an existing linear selection. Three controls passed:
before-caret priority, ordinary nested/Unicode matching and empty/stale/non-brace
carets. An actual EditorElement prepaint probe also failed its foreground-work
contract: the 12,582,913-byte sample visited 12,582,912 bytes on each of two
unchanged paints. This is a deterministic work count, not a native responsiveness
measurement. The run took 51.162 s wall/0.40 s unit and exited 101.

The baseline patch is SHA256
`b1ec3d299dd22595173e293ac377af1dd84b06c7264ebf3608257a625dd1668d`;
its retained test binary is
`bf51dcb72606fcee74236857c0a6acb5dc945b71a0e2b899e568548ba53732f3`.
The eight-entry evidence manifest is
`b305e00745f96ccdc42015d8b2988aa9cfb811ae12164d3f568bced0a67ac1a9`.

Two additional actual-window dispatch probes failed to move/select on the
baseline. Their action declarations are test-only absence probes, not product
wiring. Text, empty history, saved state and read-only state passed their checks
before the endpoint assertions failed. The run exited 101 in 47.057 s wall/
0.02 s unit. Its patch is
`bdde9e4185e62f1aa220e1ea6a7f817d8efadd821f3a112dbd6c2f35b673bccc`,
retained binary
`8524bc9163b951c0c0aad8cd1e29a67c03f1c2df6acc8b77a2c864e0851a57a8`
and evidence manifest
`cdc30f47b89b6ef373f571437673ed78ce641ab7310c2604637d0691d9c4cb81`.
No fixture or compile correction followed either source freeze. After each
run, root removed only its temporary test patch and verified all 446 original
source/manifest hashes and a clean main tree. Local evidence remains in ignored
`gates/` under the isolated Windows brace worktree.

The implementation captures Rope and `Arc<Vec<HighlightSpan>>` snapshots without
copying full text or the style vector on the UI thread. The worker validates up
to one million ordered, non-overlapping syntax spans and uses checked iterative
nesting and byte offsets. Cancellation is checked at most every 64 KiB of scan.
Replies are rejected after source, caret/rectangle, tab incarnation, syntax
publication or focus/window changes. Snapshot disposal runs on a worker.

Source or Raw View must be focused, loaded and free of composition or an active
edit transaction. Read-only navigation is allowed. A successful Go to command
collapses to the partner's byte position; Select includes both delimiters and
clears rectangle geometry. Accepted misses preserve selection and rectangle
geometry. Refused commands preserve the document and are not recorded.

Saved macros wait for matching, keep their completed prefix on cancellation and
stop the remaining tail after stale results. Completed prefix/navigation/tail
edits share one Undo without merging earlier user history. A manual brace action
cannot take a waiting macro's worker. A paint-only worker can be promoted to a
navigation request without starting another scan.

The matcher compares broad `HighlightKind` values, including string/comment
origins. Exact Notepad++ lexer styles and its styled-tail relaxation remain
open. Find-focused navigation is not established. Native Windows visual/input
verification remains unavailable; TestWindow dispatch and paint probes do not
establish it. See the validation record for scoped optimized helper timings.
Mac testing is deferred.
