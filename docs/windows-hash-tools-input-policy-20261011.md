# Windows Hash Tools input and execution policy

Source/test preparation dated 2026-10-11. No hash implementation, Rust build,
application execution or native visual verification is claimed here. The initial
preparation base is `10d47236c088ea83309505c0398334af7ddd035e`; actual baseline
runs and implementation must use the reviewed Column/Physical checkpoint
`b42c889f8d37f09b244bab8037ad31a4e82caede` or its reviewed successor.

## Oracle trace

Paths in this section are relative to the local, unvendored Notepad++ checkout.
They establish behavior; no GPL algorithm or dialog source is copied or translated.

- `PowerEditor/src/Notepad_plus.rc:1281-1308`: Tools follows Settings and precedes
  Macro. MD5, SHA-1, SHA-256, SHA-512 each have Generate..., Generate from files...,
  Generate from selection into clipboard, in that order, without separators.
  `menuCmdID.h:622-633` names all twelve commands. `Parameters.cpp` has no
  `IDM_TOOL_` default key row. The explicit macro whitelist at
  `NppCommands.cpp:4412-4639` omits them. Hashing does not edit an editor document;
  no MacroEvent or saved schema addition is proposed.
- `NppCommands.cpp:3595-3617,3674-3729`: selection commands use `_pEditView`,
  require SCI_GETSELECTIONS equal to one and a nonzero physical byte span, then
  retrieve SCI_GETSELTEXT and hash its NUL-terminated prefix. SHA uses strlen at
  3698, 3705 and 3712; `MISC/md5/md5.h:381-384` uses strlen for MD5. There is no
  read-only or field-focus condition in these handlers. Targeting the active
  editor while a Find field has focus is a source-derived inference, not a
  reproduced native user interaction. No Find-field text is hashed.
- `scintilla/src/Editor.cxx:8972-8973` returns the selection range count.
  `570-595` builds one rectangle selection per physical row. `6919-6927` returns
  rectangular limits for selection start/end. `4480-4488` copies stored document
  bytes; `4505-4525` copies physical ranges without virtual-space padding and
  adds an EOL separator only for multiple ranges. `6460-6472` copies those bytes
  and a terminal NUL for SCI_GETSELTEXT. Thus a one-row rectangle is eligible,
  hashes only its physical selected bytes and appends no EOL. Multiple-row
  rectangles are ineligible. An all-virtual span has no physical bytes.
- `PowerEditor/src/Utf8_16.cpp:123-218` passes ordinary UTF-8/8-bit input through,
  strips the transport UTF-8 BOM, and converts UTF-16 bodies to UTF-8 after
  skipping their transport BOM. `ScintillaComponent/Buffer.cpp:2143-2155`
  converts explicitly detected non-UTF-8 codepages to UTF-8 before SCI_APPENDTEXT;
  `2161-2198` appends the Unicode converter result, with an additional current
  Windows UTF-8-system-codepage branch. `1148-1164` resolves explicit codepages
  to uniUTF8_NoBOM and lets default ASCII follow the open-ANSI-as-UTF-8 setting.
  `ScintillaComponent/ScintillaEditView.cpp:2476-2495` sets Unicode buffers to
  SC_CP_UTF8 and uni8Bit to ACP or a CJK codepage. Therefore UTF-16 selection
  hashing uses UTF-8 editor bytes, not UTF-16 disk bytes. Default ANSI bytes
  depend on system/editor codepage; explicit detected encodings may instead
  have UTF-8 editor bytes.
- `MISC/md5/md5Dlgs.cpp:359-420` retrieves Unicode text, converts it to UTF-8,
  hashes exact UTF-8 bytes and clears output for empty native input. `422-510`
  splits on LF, removes exactly one trailing CR per row, emits CRLF alone for
  an empty row and emits lowercase digest plus CRLF for each nonempty row.
  A final LF does not produce another row; Unicode separators remain body text.
  Retrieval/conversion is NUL-terminated. Synthetic embedded-NUL input uses the
  effective prefix before NUL before whole/per-line processing; an empty
  effective text input clears output. This makes synthetic state follow what a
  native text control can expose rather than treating unreachable suffix text
  as another row.
- `md5Dlgs.cpp:122-222` retains chosen file order and formats successful full
  paths with CRLF, and digest + two spaces + basename + CRLF. SHA passes explicit
  content length at 167, 173 and 179; file NUL bytes are included. MD5's ANSI path
  conversion and SHA's whole-file allocation/failure early return are limitations
  to avoid. `md5Dlgs.rc:23-46` pins the text/file controls. `md5Dlgs.cpp:584-628`
  recomputes on input/checkbox changes, copies only nonempty output and hides on
  Close. `650` onward and `NppCommands.cpp:3577-3593,3620-3672` reuse retained
  per-algorithm dialog instances.

## Supported selection bytes

The reviewed policy applies to Notepad Studio's five supported encodings.
Unicode selections (UTF-8, UTF-8 BOM, UTF-16 LE and BE) use UTF-8 editing bytes
with the current document EOL reconstructed. No transport BOM is added. A literal
U+FEFF in the selected body remains data. Latin1 uses strict Windows-1252;
unmappable text before the first NUL refuses with a status, retaining clipboard,
draft and both histories. Arbitrary Windows ACP, CJK and detected codepage parity
is outside this supported encoding policy.

The existing decoder removes one transport BOM and preserves a doubled BOM as
text. Uniform CRLF/CR loading stores normalized LF plus the EOL mode. Mixed EOL
that cannot round-trip normalization stays raw in LF mode. A worker must restore
editing EOL bytes while streaming, carrying the preceding CR across Rope chunks
and from just before the selected range. Raw mixed CR/LF bytes remain intact.
Do not use the save encoder for Unicode selections: it writes disk BOM/UTF-16.
Do not use the permissive Latin1 encoder: it can write numeric references.

Truncate the selected source at its first U+0000 before conversion. A nonempty
selection starting at NUL copies the standard digest of zero bytes. An empty
physical selection leaves the clipboard unchanged. Latin1 `abc\0中` hashes abc;
`中\0abc` refuses. A literal selected U+FEFF remains bytes EF BB BF.

Stream endpoints are locally clamped/floored to valid UTF-8 boundaries without
changing the raw selection. A one-row rectangle uses the reviewed checked row
mapper to clamp columns to the physical row; it never synthesizes spaces. A
stale row refuses. Multiple rows or a physically empty range leave the clipboard
unchanged. Capture uses the checked RopeSlice row/character index helper; it does not
clone or scan a whole line. Encoding/EOL conversion and digest work belong on
the worker. Thus even a long row has bounded dispatch work. The app cannot express selection of
half an originally normalized CRLF pair: selecting its LF editing character
selects the restored pair. This representation limit is explicit.

Source/Raw editor views are eligible; CSV, Markdown and rendered rich views
must refuse instead of hashing a different projection. Loading buffers and an
active source IME composition must refuse without settling composition or
changing focus/selection. Read-only is allowed. Source selection remains the
target when a Find input has focus, as inferred above. Capture that input context
and reject a later context change; do not move focus to make a job eligible.

File digests always consume raw disk bytes, including BOM, NUL and EOL. Text
input always uses its Unicode-to-UTF-8 content, independent of any source tab's
save encoding. Text and file inputs must not reuse selection conversion.

## Proposed Rust API and ownership

These are review interfaces, not declarations or completed behavior:

```rust
pub(crate) enum Algorithm { Md5, Sha1, Sha256, Sha512 }
pub(crate) enum TextMode { Whole, PerLine }
pub(crate) struct EditorFormat {
    pub(crate) encoding: Encoding,
    pub(crate) ending: LineEnding,
}
pub(crate) fn digest_reader(
    algorithm: Algorithm, reader: &mut impl Read, cancel: &AtomicBool,
) -> Result<String, HashError>;
pub(crate) fn text_result(
    algorithm: Algorithm, input: &Rope, mode: TextMode, cancel: &AtomicBool,
) -> Result<String, HashError>;
pub(crate) fn selection_digest(
    algorithm: Algorithm, source: &Rope, range: Range<usize>,
    format: EditorFormat, cancel: &AtomicBool,
) -> Result<Option<String>, HashError>;
```

`None` means no physical selection. A nonempty source beginning with NUL returns
Some(empty-byte digest). HashError distinguishes cancellation, stale/invalid
geometry, unmappable text, allocation budget and I/O failure. A small internal
engine enum uses the existing locked RustCrypto versions and a reusable 64 KiB
scratch buffer; cancellation and progress are polled at that bound. Reader calls
retry Interrupted, propagate other failures, and never flatten files or ropes.
Output hex lengths are 32/40/64/128 and lowercase. Text row output is built with
checked cumulative lengths and fallible reservation before appending.

Production modules are `src/hash_tools.rs` for algorithms/conversion and
`src/ui/hash_tools/` for dialog state, editor-selection jobs and thin themed GPUI
controls. Add the twelve actions under Tools, their menu in the oracle position,
CATALOG rows without default chords, global text/file open-or-focus handlers,
and Workspace selection listeners. No toolbar/context additions are present in
this oracle scope. Keep dependency additions atomic and include retained
compatible md-5/sha1 licences and NOTICE entries with their exact locked versions.
sha2 is already direct and retains its licence.

Each algorithm has independent retained text and file state. Close cancels that
window generation while retaining its input, checkbox and completed output;
reopen restores state without borrowing or editing a source tab. One latest job
per state replaces its predecessor. Proposed limits for review: 32 MiB text input,
16 MiB cumulative text output, 4096 selected file paths, 16 MiB path/result
metadata, and a 128 MiB cumulative retained/pending state budget. These are
explicit temporary dialog limits, not full unbounded parity. File data itself
streams without a whole-file memory limit. Files over 2 GiB use the project's
confirmation convention before reading. Selection streams an already-owned
Rope snapshot rather than applying a document-size cap. Global active worker
count is bounded to two; queued work retains O(1) Rope snapshots, not copied text.

A text input owns a Rope, stream selection and its own composition state.
Snapshots are O(1). Ordinary keyboard/IME edits apply small checked changes;
large Paste constructs a candidate Rope on a worker and validates the input
revision before swapping it. Editing/cancellation supersedes that candidate;
there is no queued typing or forced IME commit. Rendering shapes visible rows
and a bounded horizontal segment of a long row, rather than the full 32 MiB
input. Existing Text Tools' flattened String/whole-line widget cannot be reused
unchanged for this path. Its GPUI interaction conventions can guide original
controls. Input mutation, rendering and native protocol tests are required gates.

Dialog result jobs capture state identity, generation, input revision and mode.
Copy is unavailable while a replacement result is pending. Worker-built output
publishes by owned swap after validation; replaced/stale/cancelled large values
are disposed on a worker. No deep source/history copy is allowed on dispatch or
publication. Native clipboard retrieval/write has a platform cost to measure
separately. Any required history copy is prepared on a worker.

File chooser cancellation leaves the previous completed panels intact. A new
selection invalidates older chooser/job replies. Workers open PathBuf directly,
stream each file in chosen order and retain successful rows through an error or
Cancel. Proposed policy: stop on the first error, show the path/reason, publish
the completed successful prefix, and retain old panels if the prefix is empty.
Paths/results stay aligned and have oracle formatting. Metadata before/after
reading can detect common length/mtime changes; it is not an atomic filesystem
snapshot. A changed file reports an error instead of publishing its digest.

An editor-selection job captures tab session/instance, Rope revision and content
activity, raw stream and rectangle geometry, encoding/EOL, window input-context
ownership and clipboard token. No text/geometry/history is edited. Before
publication validate every captured owner, including edit/Undo ABA, focus/context
ABA, tab reopen and clipboard supersession. Escape/cancel drops the result.

## Clipboard seam requiring coordinated review

There is no existing app-wide clipboard generation observer. Bookmarked-copy
cancellation hooks protect some editor Copy/Cut ordering; clipboard history
alone cannot observe direct path/dialog/rich clipboard writes. Proposed shared
seam: every actual application clipboard write advances an opaque epoch, and an
eligible new asynchronous clipboard-producing request claims a token and retires
older pending jobs. A hash result may publish only while its token and source
stamp remain current. New Copy/Cut, path/dialog Copy and another eligible Hash
must win over an older hash result. An empty Hash does not claim ownership.
Existing empty Paste behavior and Copy/Cut cancellation sites remain unchanged.

The proposed seam is `src/clipboard_ownership.rs`, with an opaque request marker
and the job's existing Arc<AtomicBool> cancellation flag. `claim(cancel)` replaces
the UI-thread current marker and cancels its predecessor; `is_current(&request)`
compares ownership; `write_now(&mut App, ClipboardItem)` claims then writes;
`publish(&request, &mut App, ClipboardItem)` verifies the marker before writing
and consuming ownership. No counter overflow, history clone or native read is
needed. Current state is thread-local to the application's UI thread. A scoped
test override restores the prior state after each synthetic application shuts
down, so parallel GPUI tests have independent ownership. Workers only read their
cancellation flag; comparisons/publication happen on the UI thread.

Eligibility/no-op policy must be explicit: a new Hash request claims only after
source/view/loading/IME and geometry checks establish one nonempty physical
selection. Empty/all-virtual/multiple-row/stale/refused requests do not retire
prior work. A later worker conversion/allocation/I/O error leaves the clipboard
unchanged but cannot resurrect the older retired request. Existing editor
Copy/Cut and bookmark/path Copy cancellation sites keep their current entry-time
semantics, including requests that later refuse or find no payload. Add ownership
retirement at those existing entry points, without moving their guards or changing
recording. Other synchronous UI Copy/Cut sites claim only when they actually have
a payload to write. No Paste request claims or cancels ownership. Remove, Paste
To Bookmarked Lines and Inverse do not become clipboard-producing requests;
their current local bookmark cancellation behavior is retained.

Existing delayed clipboard-producing requests are Copy Bookmarked Lines and
Cut Bookmarked Lines. They must claim at request start and carry the token into
the worker and finish guard. Cut validates ownership before *any* prepared text,
metadata or history publication; loss of ownership cancels the entire Cut,
preserving the draft and both histories. It cannot delete rows and merely skip
the clipboard write. The no-text/no-bookmark Cut completion path also validates
ownership. New selection hashes and Hash dialog Copy jobs use the same rule.
Each normal finisher may cancel its worker flag while disposing a completed job;
publication ownership is the marker, not that cleanup flag. A pending job id and
source/dialog stamp are still required, including when no newer request exists.

The complete direct-writer inventory at b42c889 has 39 production calls in 14
files (test fixtures and comments excluded):

- `src/workspace/edits.rs:235,272,290,2369`: focused input Copy/Cut, source
  clipboard funnel and rich-document selection Copy. Source line Copy/Cut and
  macro replay reach this funnel. Rich Cut delegates to its selection Copy.
- `src/workspace/block.rs:245`: prepared column Copy/Cut, including sentinel.
- `src/ui/csv_doc/actions.rs:1369,1395`: grid Copy/Cut.
- `src/edit_ops/bookmark_clipboard.rs:174,223`: small and delayed bookmark Copy.
- `src/workspace/prepared_edits.rs:415,475`: no-change and changed bookmark Cut
  completion; other prepared operations do not produce clipboard text.
- `src/edit_ops/clipboard_ops.rs:32,52,70`: full path, filename and directory.
- `src/edit_ops/clipboard_all.rs:71`: all filename/path labels and saved replay.
- `src/workspace/find.rs:2233`: Mark Copy.
- `src/ui/tab_context_menu.rs:630,658,687`: tab full path/filename/directory Copy.
- `src/ui/folder_panel/context_menu.rs:207,227`: folder full path/filename Copy.
- `src/ui/run_dialog/input.rs:145`: Run input Copy/Cut keyboard callback.
- `src/ui/preferences/mod.rs:1664,1672,1768,1776,1861,1869`: three preference
  field Copy/Cut pairs.
- `src/ui/style_configurator/mod.rs:803,812,924,934`: two field Copy/Cut pairs.
- `src/ui/text_tools/mod.rs:727,755,768,1064,1070,1137,1143`: result Copy, focused
  input Copy/Cut and two input-widget Copy/Cut pairs.

No plugin host clipboard writer was found. `clipboard_rich.rs` reads platform
flavors; it is not a writer. Clipboard History pastes an entry without changing
the OS clipboard. All direct calls above need narrow owned-write wrapping;
text construction, mutation, history order, macro events and surrounding guards
stay in their existing callers. A final source scan must find direct writes only
inside the coordinator and test fixtures. No clipboard-history refactor or new
rich flavor behavior is part of this seam.

This needs coordinated narrow write-site wrapping and pending Book/prepared-copy
retirement, beyond the new modules' registrations. It is not implemented by this
packet. The available GPUI API cannot prove atomic supersession by arbitrary
external OS clipboard writers; that residual needs either an explicit accepted
limit or a separately reviewed native sequence seam. No new unsafe/vendor patch
is proposed. Tests must hold workers and prove ordering on actual app writers,
not merely change a test token.

## Prepared controls and evidence limits

`hash_tools_tests.rs` captures actual installed menu inputs and checks all twelve
routes/order, tests CATALOG rebindability and empty defaults, dispatches all eight
text/file probes, and dispatches four selection probes on a read-only source with
real Undo/Redo, marks and a reversed selection. Missing types are declared only
in this test module: dispatch uses real GPUI routing but has no production action
or handler. Window count proves dialog creation only, not layout/input/results.
The existing registered EditCopy positive control checks the fixture's routing.

`hash_tools_policy_cases.rs` adds current decoder/EOL/strict1252 facts, supported
encoding dispatch, mixed EOL, NUL/leading NUL/ANSI ignored suffix, Find focus,
one-row virtual rectangle, empty/all-virtual/multiple-row preservation cases.
A missing handler also passes no-op cases; only pairing with real nonempty
behavior and later held-worker tests can prove the guards. These cases are
prepared and unrun. They are missing-feature tests, not crash reproductions.

`tests/data/hash_tools_vectors.json` contains 11 raw-byte and 12 whole/per-line
input cases for all four algorithms. Expected digests come from Python 3.12.8
hashlib/OpenSSL 3.0.15, independent of app/oracle implementations. Raw UTF-8 BOM,
UTF-16 LE/BE and embedded NUL distinguish file bytes from editor/text bytes.
Whole/per-line expected formatting follows the behavior specification above.
The fixture is generated, not an application test result; the future pure engine
must consume it. G1 remains unchecked until the authorized baseline controls
actually run. Native visual, default-release >10 MiB responsiveness, errors,
Cancel/stale/clipboard ordering and final full gates remain outstanding.

## Studied checkout identities

The local checkout has no Git metadata. SHA-256 pins the exact studied files:

- `PowerEditor/src/NppCommands.cpp`: `5fda9f9dbf31f0208f90c8ec571b51acb723297b5aa228dbf5b8a160c0c8e510`
- `PowerEditor/src/Notepad_plus.rc`: `cb930d135a47c2f8da40ddfab8cacc9d399621dc7927f4fab29fe031d6c139e9`
- `PowerEditor/src/menuCmdID.h`: `46c6bee79ccd8742f3f7de04ff9e674ee3f4e9bdaf32c512f4868071fb62b700`
- `PowerEditor/src/Parameters.cpp`: `99506f082fe12e8e77731273daa144b5813ddbfc325d29bedfec91a08fab652a`
- `PowerEditor/src/MISC/md5/md5Dlgs.cpp`: `6d7e6a48ca540e602c149d231fe6e2bf2962bedf99dd3166ca5fff5f9694e1ea`
- `PowerEditor/src/MISC/md5/md5Dlgs.rc`: `215039609c40b43eade96974e3ad38fb73dadcc09c2a2bf53880727396d31e1c`
- `PowerEditor/src/MISC/md5/md5.h`: `438ae5cde1852ad7bf860bcea156ed7c0bbb27267aa217765b7e41f992333894`
- `PowerEditor/src/Utf8_16.cpp`: `e7a7ca448c236ed8f5abc63fc21ea3604f2975f13adcf68fac543bf01ce38b4d`
- `PowerEditor/src/ScintillaComponent/Buffer.cpp`: `cd62291db1fa71070f4aa80c0c089167a34da5e6deda05f625d7a38c0b8444a9`
- `PowerEditor/src/ScintillaComponent/ScintillaEditView.cpp`: `1330d3e6f1824a681b66fb20235d2cf1ff4c22898f616b36f6e832571db14304`
- `scintilla/src/Editor.cxx`: `305561a566b373a6aa4b1431d3f87c63b82cd6be0513d56fbbc13c38e4fdef7b`
