# Windows brace navigation validation

The integrated candidate adds both Search routes and replaces per-paint scanning
with a shared cancellable matcher/cache. Main build passed in 47.959 s; 2,684
unit and three integration tests passed, with 30 ignored (81.59 s unit /
211.665 s total). Clippy passed in 26.914 s. Its canonical diagnostic count
fell from 334 to 331 occurrences and 195 to 193 unique message/path pairs,
with zero new pairs. All 12 inventory tests and regeneration checks pass.

All 733 source/manifest entries remained unchanged during the deciding main
checks. The 29 integrated files are byte-identical to the final leaf patch:
`ee8728f0ca9408e08ea9fbe0c01c75972f83424493b819771cec8413674bdb00`.
Production source SHA256, covering 340 Rust files:
`1282a5ac2d774561b51d5ee06cbba1b0a2a4b2343f0639dbc6d3e35d6c83e78d`.
Source snapshot SHA256: `8f77cda978c3d447217e9dc0471b491f17ad729a32d3690e8ae138808c3319c2`.
Cargo retains SHA256 `fb072656`; lockfile SHA256 is unchanged at `9ea6f022`.
Retained main test binary: `c5e4f47585a0ff09c3e5e4e50fa146d54e2871772fc8d11b516d7514276a16e2`
(158,401,024 bytes). Logs, receipt and binary remain in ignored `log/`.

The isolated candidate passed 18 focused tests, then build, 2,684 unit plus three
integration tests (30 ignored), and Clippy. Its first broad run found a new
`is_multiple_of` style diagnostic. The final candidate uses the recommended
method; deciding main checks confirm zero new diagnostics. The earlier
12- and 18-test packets, compiler corrections and exact binaries remain local.
The final test fixtures render neutral Markdown, CSV and RTF buffers and settle
the views; a plain unpathed preview flag does not establish a rendered CSV view.

Focused controls cover actual registered dispatch, read-only Unicode positions,
15 encoding/EOL combinations, caret/rectangle retention, style/tab/content/focus
ABA rejection, Escape, saved TOML events, macro prefix/tail ordering and history,
paint-worker promotion, and refusal during loading, composition and active
transactions or in preview/field contexts. Actual unchanged paints of matched
and unmatched 12 MiB samples visit zero matcher bytes on the foreground thread
and start no additional worker after the cached result arrives.

An exact-excerpt optimized helper control used shipping flags: opt-level=3,
thin LTO, one codegen unit, panic=abort and stripped symbols. Twelve processes
ran sequentially in three alternating baseline/candidate repetitions per input,
with thirteen empty compiler/test process inventories. Every result matched the
expected pair or miss. Inputs were 12,582,913 and 12,582,914 bytes.

| Input | Old helper, two scans (median) | New helper, initial match (median) |
| --- | ---: | ---: |
| Unmatched | 25.660 ms | 10.698 ms |
| Matched | 28.019 ms | 14.467 ms |

The two pair-copy reads in the wrapper were below 0.001 ms. That wrapper excludes
controller stamp validation, painting, shaping and native frame latency. These
numbers show the measured helper work; they are not a whole-app speedup claim.
The real paint-work assertion is a separate TestWindow control.

The optimized binary is SHA256
`de71ff119a5d81a62207f185ab0e463b2c0f7c298a35a81a8ea2f84caf0c0eb9`.
Baseline source comes from `b42c889:src/editor_element.rs`; candidate matcher,
style definitions and char-boundary helper are exact source excerpts. Inputs,
excerpts, compiler command/toolchain, dependencies and results are hash-pinned
under the isolated leaf's ignored `gates/brace-performance/`.

Exact per-lexer style parity, styled-tail behavior, Find-focused navigation,
native Windows menus/shortcuts/toolbar/outlines and new Mac changes remain
unverified. No unsafe, dependency, plugin ABI or persisted-field rename changed.
