# Hash Tools

Tools provides MD5, SHA-1, SHA-256 and SHA-512. Each algorithm has **Generate...**,
**Generate from files...** and **Generate from selection into clipboard**.
The commands are rebindable in the Shortcut Mapper and have no default chords.
Hash commands do not record editor macro events.

Text dialogs hash UTF-8. Typed Enter inserts CRLF. Whole-text mode produces one
lowercase digest; empty effective input produces blank output. Per-line mode
splits on LF, removes one trailing CR from each row and emits CRLF after each
digest. Empty rows emit CRLF, and a final LF adds no extra row. Hashing stops at
the first embedded NUL. Input and results remain available after Close/reopen.
The dialog has local Undo/Redo; editor documents and histories stay untouched.
Tab moves through its fields and buttons. Output fields support selection and
keyboard Copy; Enter or Space activates a focused button. Stale native input
callbacks cannot edit another field or a reopened dialog.

File dialogs stream raw bytes, including BOMs, NULs and line endings. Chosen
order is preserved. The Files panel lists full paths; each hash row contains
the digest, two spaces, the basename and CRLF. The first file read/change error
stops the batch and publishes its completed success prefix. An error before any
success, a budget refusal or Cancel retains the preceding panels. Files above
2 GiB ask before hashing. Metadata checks catch common changes during hashing;
this is not an atomic filesystem snapshot.

Selection commands use the active Source/Raw editor, including read-only
documents. Unicode encodings hash UTF-8 editor bytes with the document's line
ending restored and no transport BOM. Latin1 uses strict Windows-1252 and
refuses unmappable characters. It does not claim arbitrary Windows ACP or
detected-codepage fidelity. A nonempty span stops at its first NUL; a leading
NUL copies the standard empty-byte digest. Empty selections, multiple-row
rectangles and all-virtual spans do nothing. A one-row rectangle hashes only
its selected physical text. A bound Find dialog can request the owning
editor's selection without changing its focus or query.

Large Paste, hashing and result Copy run in background with progress and Cancel.
Source selection jobs cancel with Escape. Replies check the live source/dialog,
selection, format and input context before publication. Every application
clipboard writer also retires earlier pending clipboard requests. A newer
writer prevents an older delayed Cut from changing the document or history.
Paste does not claim clipboard ownership. External OS clipboard writers are
outside this application-side ordering guarantee.

Temporary limits are 32 MiB per text input, 16 MiB per text output, 4,096 files,
16 MiB of path/result metadata and 128 MiB of retained/pending Hash Tools work.
There are two active work slots and at most 64 queued jobs. Native text commits
above 64 KiB are refused; Paste handles larger input in background. Queries from
the native input protocol return at most 64 KiB per request. Rendering shapes
at most 64 KiB of visible field text, without flattening a large Rope or line.
Existing editor buffers, OS clipboard storage and the existing clipboard-history
ring are separate from the Hash Tools budget; owned copy duplicates remain
charged through publication or worker disposal. Rope/GPUI/platform allocations
still use their existing allocators, so these limits are not a global OOM guarantee.

Raw LF/mixed Paste is preserved by GPUI. Exact Win32 EDIT paste conversion,
native visual/IME interaction and external clipboard ownership remain native
audit gaps. Preparation and oracle identities are in
[the byte policy](windows-hash-tools-input-policy-20261011.md) and
[the Enter audit](windows-hash-tools-enter-policy-20261011.md). The
[caption exception](hash-tools-oracle-exceptions-20261011.md) records the corrected
upstream SHA-512 file title. The frozen Windows build, full tests and Clippy gates
passed, followed by 45 optimized worker controls on a 12 MiB fixture. Their
[validation record](hash-tools-validation-20261011.md) gives the exact scope,
source identities, timings and remaining native audit work.
