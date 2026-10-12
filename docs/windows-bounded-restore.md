# Bounded restoration after an unsuccessful save

The failed-save recovery path now restores opaque prior destination bytes
through a fixed 64 KiB buffer into the same open target handle. It opens and
reads the rescue metadata before target seek, handles short/Interrupted I/O
and guards reader/writer counts and checked u64 totals. Premature EOF or an
extra byte beyond the recorded length refuses restoration. It sets the final
length and syncs only after exact bytes and EOF are verified. No rename or
truncate-first replacement is introduced.

The existing save-error schema and original write/restore errors are preserved.
Every restoration failure retains the exact rescue path; cleanup follows only
complete successful restoration and sync. Cancel of the new write cannot stop
automatic recovery halfway. Mandatory rescue creation/sync, optional backup
policy, hardlinks, encoding/BOM/EOL/native handling and saved-state/ownership
publication remain unchanged. Normal Save background work is a separate leaf.
The scratch bound excludes path/error-reporting allocations, encoded/staged
new-content buffers and global allocator failure. It is not a blocking-I/O
deadline, physical power-loss or full ACL/owner guarantee.

This separate worktree began at
`10d47236c088ea83309505c0398334af7ddd035e`. Root accepted the G1 preparation
patch 0da346967ca94dece3eb0e6bcedc938c229c0df5a954cb927596a301feab6c96
and source manifest 58cab91955f402949422e842282647a861d42db84e1e858624f95e1f8f5ff05b.
After Physical's quiet measurements, the actual baseline ran with no source
drift or compile/fixture correction. One produced-error control passed and
one requested bounded-memory contract failed, 0.07 s unit time, Cargo exit 101.
The actual writer restored byte-correct 18,874,368-byte prior data and cleaned
its rescue before its callback payload failed the 65,536-byte limit. This is
an unmet bounded-memory contract, not a crash or allocator-abort reproduction.

Baseline raw `gates/g1-baseline.log` is 10,958 bytes, SHA256
`6b5e758ddf4f4e547e046c1d6bb62526097ab20b8da7fd35741be8d3e264e1e9`.
A byte-exact 153,164,800-byte test executable is retained locally at
`gates/g1-baseline-test.exe`, SHA256
`1088c2dd7f066ee658607669d062169a1a46f2fa3b3e3f9979a00013aff732e5`.
The 13-entry baseline evidence manifest has SHA256
`cf78b78bcba7a8d27cca3516aa7785f20d8bcab19e6e337069c8699f0b518a9f`.
These artifacts and the completed earlier durability worktree remain frozen.

The baseline cfg(test) produced-OutOfMemory seam failed before `fs::read` and
left the callback uninvoked, retaining exact save error 112 and rescue bytes.
The deliberate callback change now gives it the rescue path, so it owns open,
metadata and streaming read. The candidate error control honestly records
callback_invoked=true: the injected read refusal follows real open/metadata
and still precedes target seek or restoration byte mutation. It retains the
same exact errors and byte-correct rescue. No physical memory exhaustion is
used. The matching installed Rust 1.98.0 source audit establishes a fallible
initial full-file allocation and conditional infallible probe growth; it does
not supply an observed allocator abort.

Production changes are the new `src/io/restore_copy.rs` helper and io.rs
restoration call/signature. The two existing test callback callers in
`save_durability_tests.rs` and `copy_stage.rs` now invoke the helper. No
production full-file rescue-read compatibility wrapper remains. The pump
accepts short reads/writes, retries Interrupted, rejects zero progress or
oversized counts and requests at most 64 KiB, followed by one EOF byte.

Candidate controls include 12 helper/writer functions, 10 empty/shorter/longer/
multichunk size combinations and 7 separately injected outer phase failures.
They also cover actual missing-rescue refusal before seek, produced read OOM,
partial read/write errors, malicious counts, short/Interrupted I/O, empty
scratch and huge expected length without giant file allocation. Fixed scratch
is observed on an actual 18 MiB writer recovery with hard-link IDs and bytes.
Metadata assertions are scoped to those IDs/bytes and readonly attribute,
not every filesystem metadata field.

Existing registered Copy cancellation/stale-source critical presentation stays
covered. Two new registered controls exercise successful restoration after a
partial cancelled Copy write and failed-restoration logging after accepted
synthetic window close. A test-only post-result gate and isolated log sender
observe the same critical worker eprintln branch; production result handling
and logging are unchanged. This does not claim native dialog/input visual
verification, whose Windows bridge remains unavailable.

The focused candidate passed 17 test functions in six groups. The 10 size
combinations, 7 phase failures and source-unchanged/source-edited Copy cases
are nested cases, not extra function counts. The closed-window critical log
control uses the existing callback restore-error injection; the separate
core controls prove the streaming pump and phase-error behavior. One new
test initially expected different status wording. Its assertion was corrected
to the unchanged SaveError message; that fixture failure is retained locally
and excluded from product-defect evidence.

The reviewed candidate patch is 33,588 bytes, SHA256
`3ab007b8fc929a1df8a547879f440611efedc0beb1dbdaa5a5954f1551cd8ff5`.
Its eight-file source manifest has SHA256
`565106dde3b186d6740368cc6c4092eac76d4791562a76c2d3924ec4233f270e`.
The 153,375,232-byte focused test binary is retained at `gates/focused-test.exe`,
SHA256 `de79afc2b981f5925c2f94150bc216a11979aba057a6b59a58497961a47e7e6b`.
The 14-entry focused evidence manifest is
`gates/focused-evidence-sha256.json`, SHA256
`cfab52c7def085b7f66003375ea494353da16605b4c19c7c1e805b692309165d`.

The worktree then advanced without source drift to Column/Physical commit
`b42c889f8d37f09b244bab8037ad31a4e82caede`. The composition record is
`gates/composition.json`, SHA256
`cd0b63167ada5bcdf81725eeb161f7a334914f317fba22b0d70d18d488dcbe02`.
All final debug checks ran against that composed base and reviewed candidate.
The temporary save-durability fidelity group passed 12 functions, including
15 encoding/EOL and 24 registered-action/format/fault combinations as nested
cases. These overlap the full suite and are not added to its total.

Normal build passed in 43.113 s wall time. Full tests passed 2,666 unit and
3 integration tests with 30 ignored, 54.701 s wall time and 39.98 s unit time.
Clippy passed in 30.429 s. The parent's canonical message/normalized-path
comparison includes dependency diagnostics and reports 334 occurrences,
195 unique pairs, zero added and zero removed. The reviewed eight-file
source manifest was unchanged before and after these checks.

Raw logs are `gates/g4-composed-fidelity.log`, `gates/g5-composed-build.log`,
`gates/g5-composed-fulltest.log` and `gates/g5-composed-clippy.log`. Exact
commands, UTC bounds, wall times and log hashes are in
`gates/composed-gate-results.json`. The receipt is
`gates/composed-gate-receipt.json`, SHA256
`ec309a3cd77b2f2d8e71604b6c45109f351d469c7c8455ecac73a5fdc427cc47`.
The 13-entry composed evidence manifest is
`gates/composed-gate-evidence-sha256.json`, SHA256
`44d428e4c924917cbbae472efaa54adc714119d115c08d6646f14b621f48ff36`.
Gate files are local ignored evidence; this document retains the essential
results. No dependency, unsafe, schema/action change or app release LTO was
introduced.

The optimized control uses seven verbatim source excerpts: the b42 baseline
and frozen candidate SaveError/writer, each unchanged safety-copy module,
and the candidate's complete restore helper. Byte ranges/full-source hashes
are in `gates/restore-benchmark/excerpts-sha256.json`, SHA256
`e5fcbfb7a97e3dd732ab0cab0efa47c20879fbd31d7a9fab7fd375b5c327b1bc`.
The safety modules differ only by checked CRLF/LF serialization. The harness
is SHA256 `ac4e3d0c5d860f2e53911ff85a4d2fa7e7529e5620367c20eb5b05b363d41a96`.
It built with installed Rust 1.98.0, shipping opt-level 3, thin LTO, one
codegen unit, panic abort and symbol stripping, using cached optimized
dependencies. Exact command/dependency/import-library hashes are in
`gates/restore-benchmark/build-provenance.json`.

The 223,744-byte executable has SHA256
`86d173d92f2111ca24e1998354db7382f366a504abaa231312801c308d954c11`.
Its neutral temporary prior version contains 18,874,368 UTF-8 bytes with
Unicode and CRLF, SHA256
`d4d9b7aa1258ad4e53d4be1f77827f3df61cd62c656cdd36bb71aeb40f7a61e2`.
Each process performs a controlled 65,536-byte partial save followed by
error 112, then checks exact recovery bytes, target/hardlink identities,
readonly attribute and rescue cleanup. These are ordinary controlled
failures, not a physical disk-full test.

After parent review and quiet clearance, three alternating baseline/candidate
process pairs passed. The timer surrounds only the actual failed-save writer
call, including mandatory rescue creation/sync, target open/partial write,
automatic recovery/sync and rescue disposal. Fixture/input allocation,
validation, optional backup, encoding preparation and app/UI work are outside
the timer. All seven compiler inventories before/between/after were empty;
the seven candidate Rust files remained unchanged.

Baseline times were 66.446, 35.556 and 37.901 ms, median 37.901 ms and range
35.556–66.446 ms. Candidate times were 52.444, 34.291 and 35.685 ms, median
35.685 ms and range 34.291–52.444 ms. Median difference was -2.216 ms. These
small helper controls do not establish an app/UI speedup. The functional
result is bounded restoration scratch while preserving exact recovery/error
contracts. The baseline's file-sized payload and candidate's 64 KiB scratch
are established by their separate actual-writer controls, not inferred from
timing or from a reproduced allocator abort.

Six per-process stdout/stderr logs, `timing-provenance.json` and
`idle-inventories.json` are retained under `gates/restore-benchmark/`. G1,
focused and composed receipts keep their original hashes and retained binaries.
The later prose suffix changes this document only. Normal Save foreground
capture/write/publication, global/path/reporting allocations, hostile blocking
device latency, external rescue modification, full metadata coverage, >2 GiB
policy, power-loss behavior and native visual verification remain outside the
fixed-scratch claim. No independent current source work or Cargo is pending
in this leaf; parent owns final main integration and records.
