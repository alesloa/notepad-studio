# Windows column safety verification

This leaf is based on `088e8d209a5dfc226ec215da1a13853a412b9dac`.
Its frozen source patch is `03e1ebb1cacec736a19315356a59befac6048df28fc825e1ba576f4066d64874`. The 17 owned-file hashes
were checked after the final gates; the source stayed unchanged through them.
Root integrated the source with its save checkpoint separately. Physical-line
policy changes and their test-only case fixture correction are separate work.

- Registered-action regressions: five original synthetic geometry/refusal
  controls failed before the safety fix. Two classic-case consistency controls
  and two word-delete refusal controls failed before changing those branches.
  The latter runs already contained the other safety fixes. No ordinary user
  or file-triggered crash was reproduced; Delete overflow was observed in debug.
- Focused final run: 35 passed, 0 failed in 2.61 s. Controls include both
  rectangle directions, virtual-space limits, 4,096/4,097 rows, later-row
  refusal, existing transaction metadata, clipboard and history preservation,
  Unicode expansion, native text-input protocol and stream IME, persisted
  macro replay, ordinary multiline Paste, read-only/loading/view/focus guards,
  and UTF-8/BOM/UTF-16 CRLF file Save/Undo/Redo.
- Macro context: 13 passed in 0.08 s. Macro Undo: 9 passed in 0.04 s. A full-run
  failure exposed an old fixture marked closed while exercising a live editor.
  It now uses a session sink and scoped execution/recents. Production guards
  remain intact. The corrected control passed both its scope and the full suite.
- Windows build: exit 0, 13.890 s. Full suite: 2,609 unit
  tests plus 3 integration tests passed; 29 ignored. Unit execution took
  280.96 s; total command time was 290.368 s.
- Clippy: exit 0, 11.398 s; 404 inherited application
  warning messages (195 ordinary / 209 test), with no new message/path pairs.
  Fourteen source scans found no bug-class or high-severity findings. Remaining
  scanner findings are cosmetic names and existing fixtures, not a lint waiver.

The review found an added second clipboard read in ordinary macro Paste.
Action and replay now share the editor tail and reuse one captured plain value;
non-Editor replay delegates before reading. Empty/missing Paste behavior and
existing Copy/Cut cancellation are retained. No runtime read-count observer was
available; no vendor seam was added solely to count calls.

The current [column limits](column-safety.md) are deliberate foreground
refusals: 128 KiB cumulative text work, 4,096 rows and 1 MiB plan/history
metadata. This is not a completed large-column implementation or a shipping
release latency result. No additional full-app release LTO was required for
this bounded safety leaf. Legacy infallible history/Rope allocation, saved-macro
ownership copies, post-edit maintenance and native clipboard costs retain their
stated limits. Background column edits, bookmark rectangle remapping, six
extended case macro events and broader rectangle parity remain unfinished.

Tests use scoped settings, recorder/playback and recent-file data plus session
sinks and neutral temporary files. Native interactive UI, macOS and Linux
execution remain unverified.

The Windows debug test executable SHA256 was `27794b16cdbe932ce9db58366efa4fde7c032fc526f334c16440091fc317fc9f`.
Local ignored `gates/` files hold the RED logs, final focused/full logs,
diagnostics and complete SHA256 provenance; they are not committed source.
