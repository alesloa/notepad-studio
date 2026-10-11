# Hash Tools oracle notes

Preparation for the missing Tools menu; no implementation or behavior completion
is claimed. Study the local Notepad++ checkout for behavior only and write the
Rust/GPUI independently.

`Notepad_plus.rc:1281-1308` puts Tools between Settings and Macro, with MD5,
SHA-1, SHA-256 and SHA-512 in that order. Each submenu has Generate..., Generate
from files..., and Generate from selection into clipboard. The command IDs are
in `menuCmdID.h:622-633`. No hash command appears in the default shortcut table
or the explicit macro-recording whitelist in `NppCommands.cpp:4412-4639`.

The text dialog has a per-line checkbox, editable multiline input, read-only
multiline output, Copy to Clipboard and Close. Input changes and checkbox changes
recompute the result. Closing hides the existing dialog; reopening retains its
state. File dialogs have Choose files, read-only path/result areas, Copy to
Clipboard and Close. Layout/control labels are in `MISC/md5/md5Dlgs.rc:23-48`;
callbacks are in `md5Dlgs.cpp:122-222,359-510,584-650`.

Text input becomes UTF-8. Whole-text mode hashes its exact input, including line
endings, and produces lowercase hex. Empty text clears output rather than
displaying the digest of zero bytes. Native text retrieval is null-terminated;
synthetic embedded-NUL inputs need an explicit tested contract.

Per-line mode splits on LF and removes one trailing CR from each extracted row.
An empty row contributes only CRLF. Other rows contribute their lowercase digest
and CRLF. A final LF does not add another empty row; a nonempty final row still
gets CRLF. Lone CR inside a row remains part of that row, except the one trailing
CR removed by this rule. Unicode separators stay in the row body.

File digests consume raw file bytes, including BOM, NUL and original line
endings. Successful results retain chosen order, list full paths with trailing
CRLF, and format each result as digest, two spaces, basename, CRLF. The oracle's
MD5 legacy path conversion and SHA whole-file allocation are implementation
limitations, not code to reproduce; use safe Unicode paths and streamed reads.
Report failures clearly and retain completed results through a tested policy.

Selection commands in `NppCommands.cpp:3595-3617,3674-3729` require exactly one
nonempty selection. They hash selected editor bytes up to the first NUL and copy
lowercase hex. Empty or multiple selections leave the clipboard unchanged.
The app stores Unicode in its Rope; mapping ANSI/editor-codepage selections to
the oracle still needs a specific fidelity audit. File bytes and displayed text
must not be substituted for one another silently.

Use existing locked digest versions (`md-5 0.10.6`, `sha1 0.10.6`, `sha2 0.10.9`,
`digest 0.10.7`) if adding direct dependencies, with retained compatible licenses
and NOTICE entries. Actual registered caller failures, independent vectors,
bounded asynchronous capture/results, stale reply/clipboard supersession and
default-release over-10-MB controls remain required in the local
`gates/windows-hash-tools.md` contract. Native visuals remain pending.

The checkout has no Git metadata; identities below pin the studied files:

- `NppCommands.cpp`: `5fda9f9dbf31f0208f90c8ec571b51acb723297b5aa228dbf5b8a160c0c8e510`
- `Notepad_plus.rc`: `cb930d135a47c2f8da40ddfab8cacc9d399621dc7927f4fab29fe031d6c139e9`
- `MISC/md5/md5Dlgs.cpp`: `6d7e6a48ca540e602c149d231fe6e2bf2962bedf99dd3166ca5fff5f9694e1ea`
- `MISC/md5/md5Dlgs.rc`: `215039609c40b43eade96974e3ad38fb73dadcc09c2a2bf53880727396d31e1c`
