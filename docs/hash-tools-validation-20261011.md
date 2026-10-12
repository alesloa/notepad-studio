# Windows Hash Tools validation

The twelve Tools commands, eight retained dialogs and shared clipboard
coordinator passed their isolated Windows gates. The final required build took
18.554 s; 2,718 unit tests and three integration tests passed, with 30 ignored.
The full command took 57.651 s, including 48.93 s in the unit harness. Clippy
took 8.902 s and introduced no warning message/path pairs. Its app-target JSON
comparison retained duplicate app/test emissions: 404 occurrences and 114 unique
message/primary-path pairs before and after. These counts are not Cargo warning
summaries or counts from a broader vendor-inclusive extraction.

The 63 Hash Tools and three coordinator controls use registered actions and
installed GPUI callbacks, with scoped settings, macros, recents, clipboard
ownership and persistence sinks. They cover the byte policy, read-only disk
selections, real separate Find-window ownership, output Copy with fresh
sentinels, completed replies held across Cancel/close/reopen, newer clipboard
requests, delayed Cut before any editor/history mutation, budget leases and
input/output callback lifetime. Earlier missing-route and absent-dialog tests,
weak/failed fixtures and candidate regressions remain separate evidence. A
painted GPUI test window does not establish native visual or IME parity.

## Optimized worker control

Exactly 45 sequential helper processes passed, three per case, during a quiet
slot on 2026-10-12 from 02:28:48.076 to 02:29:00.001 UTC. Every process had a
fresh before/after competitor check, yielding 90 empty inventories. There were
no rejected or restarted deciding runs. Each process exited zero and verified
an independent Python hashlib golden, complete/refusal progress totals and
relevant output/reference-copy counts.

The fixture has 12,288 UTF-8 rows and 12,582,912 bytes, including representable
é/€ and LF. Its SHA-256 is
`7203bd5a4fd599a1493d72b3a1c5d49c847d193383bb6debab9beee91c215646`.
The helper compiles the exact hash engine, file worker, original encoding/EOL
modules and two original byte-clamp functions. Its 29-package active dependency
closure matches the app's accepted locked versions/checksums. Shipping flags are
opt-level 3, thin LTO, one codegen unit, panic abort and symbol stripping. The
standalone release compile took 6.50 s. No full application release LTO ran.

These are three-run medians. Worker timing excludes fixture-to-Rope preparation
and the independent per-line output checksum. Process wall time and OS peak
working set include both. Raw/file worker timing includes actual reads. Peak
working set describes the whole helper process, not the worker's allocations.

| Case | Worker ms | Rope prep ms | Process wall ms | Whole-process peak MiB |
| --- | ---: | ---: | ---: | ---: |
| Raw stream MD5 | 15.877 | 0.002 | 44.201 | 4.39 |
| Raw stream SHA-1 | 6.270 | 0.001 | 29.631 | 4.39 |
| Raw stream SHA-256 | 6.568 | 0.001 | 30.761 | 4.40 |
| Raw stream SHA-512 | 19.641 | 0.001 | 43.415 | 4.40 |
| Raw flatten reference SHA-256 | 8.008 | 0.001 | 32.293 | 16.34 |
| Whole text SHA-256 | 7.863 | 8.461 | 40.365 | 30.30 |
| UTF-8 selection SHA-256 | 9.520 | 10.435 | 46.959 | 30.30 |
| Selection flatten reference SHA-256 | 10.399 | 10.123 | 47.663 | 35.61 |
| File batch SHA-256 | 6.501 | 0.000 | 30.124 | 4.42 |
| Per-line text SHA-256 | 55.528 | 8.620 | 90.083 | 30.30 |
| Restored CRLF selection SHA-256 | 62.894 | 8.969 | 97.662 | 30.31 |
| Strict Windows-1252/CRLF selection SHA-256 | 127.074 | 9.145 | 163.172 | 30.30 |
| Controlled Cancel after first chunk | 0.101 | 0.000 | 22.402 | 4.40 |
| Controlled read error after first chunk | 0.095 | 0.000 | 22.256 | 4.39 |
| Per-line output-cap refusal | 72.990 | 6.968 | 102.404 | 35.25 |

Complete cases accounted for all 12,582,912 source bytes. The per-line case
produced 811,008 bytes, independently checksummed in full. Cancel/read-error
controls stopped after 65,536 bytes. Output-cap refusal accounted for 8,388,609
source bytes while refusing output beyond 16 MiB. Flattening references copied
an extra 12,582,912-byte whole input; production streaming/Rope paths made no
such reference copy. They still allocate their bounded buffers and results.
The references are allocation controls, not previous shipping baselines: these
Hash commands were absent before this leaf.

The helper measures worker behavior. It excludes application dispatch, GPUI
publication/painting and native clipboard costs. Its injected Cancel/read error
does not measure native Cancel-button latency. Actual caller controls establish
the application ownership/lifetime behavior separately.

## Frozen identities and remaining work

The final application patch is
`11a0b92b0e9a27b879971b57d36de9f9a2672c947be8e2b33e144f8866a519e5`;
its 54-file manifest is
`4397b195245b1f77a992adab32b11ea7b47aa317b6da3f493c1aaa22ab92cc13`.
The callback/gate receipt is
`05029b8fd410e44d274a43e69d4d7101cd8a3d129e4de426b81104cfc5452f3e`.
The 17-input helper manifest is
`6e2ad6b0874622cb911daa7dc9a709035807f47d95ccb9a64b021685e37f047b`;
the retained helper EXE is
`343367c694b24d7fd4b4f796c8fd44171d5561060843430fec7ba0555c00a823`.
The 45-run result hash is
`5f88ced27511dbd218343db9209968f0b93b18769ead6badf0039a2268d7233c`;
the 90-inventory hash is
`2405223aca76398af1edf71922a7e8be879682a2e07845ad4496d12822808853`.
The performance receipt is
`7f36076561cbfd9eaa476ae2e26705dd29355287bb90ae01bcfc040967210aa2`.
Logs, exact consumed-source archives and verified app/test/helper artifacts
remain in the isolated worktree's ignored `gates/`; they are excluded from the
source patch. This evidence paragraph is a separate docs-only suffix.

Native visual/IME/picker interaction, exact Win32 mixed-EOL paste conversion,
arbitrary Windows ACP/detected codepages and external OS clipboard ordering
remain open. The supported Unicode/strict Windows-1252 policy and temporary
input/output/queue/budget limits are in [Hash Tools](hash-tools.md). The SHA-512
file title deliberately corrects upstream's misleading default-English title;
its source trace is in [the exception record](hash-tools-oracle-exceptions-20261011.md).
Main composition and its combined Windows checks also pass. No macOS/Linux
or native GUI performance result is claimed.

## Combined main verification

The final 55-file delivery was reviewed against committed `4a5dad8`, retaining
the newer brace actions and all twelve prepared legacy line commands. Thirty-
two of its thirty-eight Rust source files match after Git newline normalization;
six shared files contain only retained brace/legacy additions. Clipboard Cut
checks ownership before editor/history publication. Explicit untitled-label
origin and existing session/recovery behavior remain intact.

Main build took 41.150 s. All 2,875 unit plus three integration tests passed,
32 ignored (74.59 s unit / 190.338 s total). Clippy took 24.562 s; the rendered,
vendor-inclusive canonical comparison equals prior main exactly at 331
occurrences / 193 message-path pairs, with zero additions or removals. This
extraction differs from the isolated app-target JSON counts above. All 993
frozen tracked/additional entries stayed unchanged during these three gates.
The inventory's twelve tests and generation check pass: 196 routed and 271
missing unique IDs, 289 CATALOG rows. Behavior remains pending native checks.

The two pinned direct dependencies use packages/versions/checksums already in
the lockfile. Only the app dependency list changes; licences and NOTICE entries
are retained. No new application unsafe block or plugin ABI/host change exists.
Root separately verified all 55 source archives, the 17-input helper compile
packet, every one of the 45 results and all 90 inventories. Independent Python
hashlib recomputation matches all four algorithm vector groups and every
digest, progress, output, reference-copy and refusal result; medians match.

Deciding main identities:

- 353 production Rust files: `2f32e1e531367e6f6f706967db2c0cb8d55039a83691d32b00b8a58031098cb9`.
- Gate snapshot: `3ea0ff835eaa135263d8430a5bdc948bc86ac41201894fdd15ba56097481bda3`.
- Retained main test EXE (168,529,920 bytes): `f958443aeadad77563ce274f4c52dfcbb7b1700d16e43278ecb2d80da22b21f6`.
- Final 55-file leaf manifest: `67ef100676c99d66766329bd0ef902834c41af3d806f40a980c1587322e8bdde`.
- Final delivery patch: `fb3e769df9b25c64aaea89c19a90b35c2885eb261ff85a70734aa5d4ba8aa834`.

The Windows goal remains active. Documented byte policies and temporary limits,
native visual/input/paste/ACP/clipboard verification, save/close settlement and
the broader behavior backlog remain open. Nothing was pushed or published.
