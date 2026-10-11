# Save rescue-copy investigation

The corrected writer refuses an existing-target save unless it has created and
synchronized a sibling rescue. The original writer's process-interruption
contract failed on Windows at base `088e8d209a5dfc226ec215da1a13853a412b9dac`.

The controlled child calls `io.rs::write_file_safely_with`, writes a fixed prefix
to an existing temporary target, syncs it, and signals a token-checked readiness
marker. The parent waits at most 10 seconds and terminates and waits only on its
owned `Child` handle. The child has its own 20-second fallback timeout. Fixtures
use only neutral temporary files, with no editor, settings, registry, or user-app
initialization.

With rescue names available, interruption changed the target but retained the
exact prior bytes at `target.txt.notepad-studio-save`. With all 100 possible names
occupied by unrelated stale bytes, interruption left a partially changed target
and no discoverable copy of the prior version. Every preseeded rescue remained
unchanged. An ordinary returned-error control with the same saturated names
restored the original through the current in-memory fallback.

The prior bytes had SHA256
`e0b3dea5355dc81872ffd720fbe7694faaaf8b602ab769987c73103dca603db6`.
Both interrupted targets had SHA256
`fea324c70464bd813929180aa9c65a5071a5d9a854d220d4313bf8db5cc595be`.
`gates/g1-first-interruption.log` records 3 passing controls and 1 failing
recoverable-prior-version assertion, exit 101. The failure establishes loss of
the previous disk version in this fixture after process termination. It does not
establish behavior after power loss or on every filesystem.

The implemented policy keeps the existing in-place target writer, including its
hard-link and file-permission behavior. Before opening an existing target for
writing, require a successfully created and synchronized sibling rescue. If
creation or synchronization fails, return the existing `NoSafetyCopy` refusal
with its underlying I/O error, leaving the target unchanged. The mandatory rescue
is separate from optional configured backup-on-save. New files have no prior
version to preserve. Restore failures must continue to report surviving rescue
paths, including cancellation or stale-caller cases.

The synchronization step reopens the already copied owned rescue
with write access and no truncation, then calls `sync_all` before permitting
the target write. Microsoft requires write access for `FlushFileBuffers`; Rust's
`File::open` is read-only. A separate injected sync-error control should prove
refusal before the data writer runs, while an actual Windows success control
proves that the chosen handle can sync. Sources: [Microsoft FlushFileBuffers](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers)
and [Rust File documentation](https://doc.rust-lang.org/std/fs/struct.File.html#method.sync_all).

Root approved that policy after the recorded reproduction. The final focused
run passes 12 tests. One is the inert child entry point during a normal suite
invocation; the other tests cover interruption, refusal, synchronization,
rollback, identity, caller state and byte fidelity. Nested combinations are
reported separately from the test count:

- Save, Save As, Save All and Save Copy each run with copy and sync failures on
  UTF-16 BE, RTF and DOCX sources, 24 combinations. They retain UUID, Arc
  incarnation, path, label, dirty state, revisions, saved marker, selection,
  undo/redo, native original-byte Arc, and recovery metadata.
- Separate actual callers cover untitled Save As refusal with UTF-8 BOM metadata
  and Save All continuing another target while the refused tab stays dirty.
- Five encodings crossed with LF, CRLF and CR give 15 byte-fidelity combinations.
  Unicode fixtures include accented, CJK and supplementary characters; Latin-1
  uses representable accented characters.
- Writer controls cover all 100 occupied rescue names, exact copy error 112 and
  sync error 5, successful write-capable sync, partial-write restoration,
  retained rescue after failed restoration, and hard-link identity.

The ordinary error controls inject Windows I/O codes. They do not fill the
physical volume or simulate power loss. The new identity control verifies both
hard-link file IDs and bytes and the writable file's read-only attribute value.
The existing read-only-file control still proves refusal with unchanged bytes;
the existing Windows hidden-file control still saves successfully. ACLs,
ownership and every metadata field are not independently asserted here. The
in-place target algorithm is retained rather than replaced by rename.

`g2-g4-caller-controls.log` records the 12 focused passes. The existing actual
Save Copy cancellation plus failed-restoration caller also passes on the new
writer, with and without a later source edit; `g3-critical-restore-caller.log`
reports both surviving rescue paths and verifies their previous destination
bytes. Test import and Save All status-expectation corrections are fixture
corrections, not additional product defects.

Windows checks on the exact frozen source used the assigned
`../rich-match-blocks/target/agent-rich-match-blocks` cache and
`C:/Program Files/LLVM/bin/clang-cl.exe` for CC and CXX:

| Check | Result |
| --- | --- |
| `cargo test -p notepad-studio` | 2,586 unit and 3 integration tests passed; 0 failed; 29 ignored. Unit run 46.26 s; test build 25.43 s. |
| `cargo clippy -p notepad-studio --all-targets` | Exit 0, 29.68 s; 195 app and 209 test warnings, 191 duplicates. No new touched diagnostics. |
| `cargo build` | Exit 0, 43.92 s; 11 existing warnings. |

The reviewed seven-file source patch is `gates/source-before-g5.patch`, 37,800
bytes, SHA256 `d850cb37ee91c4f85d39c15381f734a88c43fa6301d217b11e0425edea3c6aa6`.
`source-before-g5-sha256.json` matches every file after the debug checks. The Rust
source remains frozen; this final verification prose is a separate suffix.
Gate evidence remains local and gitignored. There is no manifest/dependency,
plugin ABI, action ID, shortcut, persisted schema, or production unsafe change.

The approved optimized comparison uses verbatim baseline and candidate writer
functions and `SaveError`, plus the complete candidate rescue helper. The
baseline comes from the recorded Git base; byte offsets and SHA256 checks prove
each excerpt matches its source. The small standalone Rust binary uses
`opt-level=3`, thin LTO, one codegen unit and `panic=abort`, with existing optimized
dependencies. It does not instantiate the app or a window.

The parent reserved a quiet measurement slot. Fresh Cargo, rustc, link, lld-link
and Clang process inventories were empty before, between and after six sequential
processes, alternating baseline and candidate. Each overwrote an identical
18,874,368-byte existing target with an equal-sized replacement, verified target
and hard-link bytes, preserved file IDs and the read-only attribute, and left no
rescue behind. The harness field `prior_permissions_preserved` refers to that
read-only attribute check, not an ACL/ownership assertion.

| Exact writer/helper scope | Three process measurements, ms | Median, ms |
| --- | --- | ---: |
| Baseline existing-target overwrite | 171.918, 205.888, 145.829 | 171.918 |
| Created and synced rescue, then existing-target overwrite | 376.451, 366.460, 362.197 | 366.460 |

The median difference is 194.542 ms. This scope includes mandatory rescue
preparation, target write/sync and rescue cleanup. It excludes fixture setup,
encoding/native serialization, optional configured backup, app scheduling and
caller publication. It does not establish whole-app latency or a UI speedup.
The additional synchronous checkpoint cost also makes the existing foreground
normal Save paths a follow-up performance concern; this leaf does not redesign
those callers. Save Copy already performs this writer work on its background
executor.

Input original SHA256 is
`d4d9b7aa1258ad4e53d4be1f77827f3df61cd62c656cdd36bb71aeb40f7a61e2`;
replacement SHA256 is
`b91e4df827c3972f23e0920e2f9dceb8b04c7e4cacfa4b3869c0d189988ac45a`.
The optimized artifact SHA256 is
`f34dcc7d335bbe505cb0571cdf9fcf873e9ef2ee94c7f2645079ddd907665b98`.
Under `gates/writer-benchmark/`, `excerpts-sha256.json` records exact source
excerpts, `timing-provenance.json` identifies source/harness/artifact/profile,
`input-sha256.json` identifies the data, `idle-inventories.json` records quiet
checks, and `{baseline,candidate}-{1,2,3}.log` contains each deciding result.

The guarantee is a successful rescue-creation and sync checkpoint before the
existing target write begins, plus the tested refusal/restoration behavior.
Process termination is tested on Windows temporary fixtures. Power loss,
network filesystem durability, global allocator failure and the existing
whole-file restore-read allocation remain outside that evidence. No additional
claim about those cases is made. Both assigned caches are idle and retained.
