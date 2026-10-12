# Bounded failed-save recovery: integrated Windows checks

Root integrated the reviewed fixed-buffer restoration into the tested
Column/Physical checkpoint. Existing-file writes still require a synchronized
sibling rescue before overwrite. A failed write restores through the same target
handle using 64 KiB scratch, without loading the whole rescue into memory.
Incomplete restoration retains the rescue and reports both errors. This change
does not move normal Save preparation or writing off the UI thread.

The reviewed candidate patch is
`3ab007b8fc929a1df8a547879f440611efedc0beb1dbdaa5a5954f1551cd8ff5`.
The final eight-file source/prose patch is
`0c08c9c3566fbb0bb8689be066e02cdf942aae7099021668c1fbc9c245e87fc5`.
Root verified all 90 entries of the leaf's final evidence manifest
`838ca847bf02b4ee8326584f9094f731b057e20216bdaf1981409995886a1002`,
the eight final source files and exact integration after Git newline
normalization. Seven Rust files retain their reviewed candidate hashes.
The [implementation record](windows-bounded-restore.md) gives the deciding
baseline, phase failures, registered callers and retained artifact identities.

Main required checks passed on frozen source:

| Check | Result | Wall time |
| --- | --- | --- |
| Normal Windows build | Passed | 22.216 s |
| Full tests | 2,666 unit + 3 integration passed; 30 ignored | 148.148 s |
| Clippy, all app targets | Passed; zero new diagnostics | 22.440 s |
| Inventory tests and regeneration check | 12 passed; generated output current | 0.036 s unit |

Unit execution took 64.86 s. Other owners were compiling functional baseline
tests in separate caches, so these wall times are gate evidence rather than
performance measurements. Canonical Clippy comparison with the previous tested
main includes dependency diagnostics: 334 occurrences, 195 unique
message/normalized-path pairs, zero added and zero removed.

The 448-file source/manifest snapshot remained unchanged from before the main
checks through their completion. Its SHA256 is
`d3868def425e5b7bd452657e8f084faf1a897f0feca32f26c80c00ba999750ba`.
The 338-file production Rust inventory has SHA256
`a54ce8dd308d461367f743437d9216a897d89a48cf932cc8b5217b8adfa785f5`.
Cargo.toml retains SHA256
`fb072656ef80cb3fdf6d5fb858003562f941c031ea5ab9dcd802aa294991e67a`;
Cargo.lock is unchanged. The retained 157,100,544-byte main test binary has
SHA256 `f92cb4396f525a7bd17bc7ebb4d1283537aa422101763f8fd940b71a0d7a95a3`.
The inventory still records 182 routed and 285 missing unique IDs; routing is
separate from behavior parity. No dependency, plugin ABI, vendor, unsafe or
persisted-schema change was introduced.

The optimized control uses exact baseline/candidate writer and recovery excerpts,
with the shipping opt-level 3, thin LTO, one codegen unit, panic abort and symbol
stripping. Root independently verified all seven excerpts, input hashes, three
dependency/import hashes and the compiled artifact. Six alternating processes
restored an 18,874,368-byte original after a controlled 65,536-byte partial write
and error 112. Target and hardlink bytes/identity, prior read-only attribute and
rescue cleanup passed in every run. Seven deciding compiler inventories were
empty. Baseline median was 37.901 ms and candidate median 35.685 ms, including
rescue creation/sync, partial write, recovery/sync and cleanup. These small helper
controls establish the functional recovery contract; they do not establish an
app or native UI speedup. Bounded scratch is established by the source and
actual-writer controls, not inferred from timings.

Local main logs are `log/windows-bounded-main-{build,tests,clippy}.txt`, with
matching timing files. Source snapshots, canonical diagnostics, the retained
main test binary and reviewed integration receipt are also under ignored
`log/`. Leaf logs and receipts remain under ignored `gates/` in the bounded
restore worktree. Essential results are retained in these committed documents.
Normal Save background capture/publication and close/Exit settlement, global
allocations, power-loss behavior, full metadata coverage, native Windows visuals
and Mac testing remain open. The overall Windows parity goal remains active.
