# Windows legacy line operations, 2026-10-11

The twelve existing lexical, integer, decimal, reverse, randomize and duplicate-
removal commands now use background preparation, cancellation and guarded
publication. Completed changes add one Undo entry and work in saved mixed
macros. The prior callbacks copied/transformed the selected text in foreground,
collapsed selection and did not record these commands.

The original Rust implementation studies `NppCommands.cpp:933–1063`,
`ScintillaEditView.cpp:4474–4517,4806–4871`, `Sorters.h:37–668`,
`Common.cpp:926–963` and `Notepad_plus.cpp:1759–1842`. No upstream implementation,
dependency, unsafe code or persisted-schema change was added. The newer six
sort commands keep their existing behavior. Split, Join and Remove Empty
remain outside this migration.

Lexical keys use UTF16 units. Integer comparison handles signed ASCII digit
runs without machine-integer overflow. Decimal conversion supports valid
prefixes, subnormal values and signed zero under default nearest-even rounding;
invalid conversions, infinity and nonzero underflow-to-zero refuse the whole
request. The original 768-significant-digit scanner is checked against fourteen
independent exact-rational CRT boundary vectors. Numeric refusal preserves text,
format, geometry, annotations, clipboard, savepoint and history.

Duplicate removal keeps the first equal body. Consecutive removal uses body and
terminator equality and preserves a matching final unterminated copy. Runtime
prepared history tracks rectangle geometry through Undo/Redo and mixed macros.
The existing persisted history codec is unchanged.

Explicit limits and differences remain:

- Selected spans are capped at 32 MiB, rows at 1,048,576, annotations at 65,536
  and prepared/full macro history at 128 MiB. Refused macro grouping leaves
  completed text and separate Undo entries intact.
- Physical CR/LF/CRLF endings and embedded NUL survive. Mixed endings follow
  the documented fidelity policy rather than the older configured-EOL oracle
  splitter. A permutation that could fuse CR and LF around an empty row is
  refused.
- Stable cancellable natural sorting defines pathological non-total comparison
  results. Exact native `std::sort` order and random generator permutations are
  not claimed. Changed floating-point rounding modes remain outside scope.
- Numeric refusal retains geometry; upstream resets stream selection after its
  error dialog. Rectangle replacement behavior is source-inferred and still
  needs native observation.

Main composition preserves the newer brace actions, read-only macro allowance,
replay state and pending-worker continuation. Eighteen source files match the
reviewed leaf after Git newline normalization; three macro files contain only
those retained main additions. Cargo and its lockfile are unchanged.

Combined Windows checks pass on 966 frozen tracked/additional entries, with no
drift during any gate: build 44.379 s; 2,809 unit plus three integration tests,
32 ignored (52.94 s unit / 138.404 s total); Clippy 23.510 s. Canonical rendered
warnings equal the prior main exactly: 331 occurrences / 193 message-path
pairs, zero added or removed. The inventory's twelve tests and generation check
pass. Its 343 production Rust files have SHA256
`6286ae5df89da0b2fb13d767939948d65210c9c2695ff5f7dd36ba8ee08dc08d`.
Static counts remain 184 routed and 283 missing unique IDs; routing does not
prove behavioral parity.

The installed leaf controls cover 111 caller tests, including 180 independent
disk-byte cases across five encodings and three EOLs: open bytes, unchanged
Save, sorted Save, Undo Save and Redo Save. Read-only/view/focus/IME/loading/
transaction refusals, both held completion boundaries, typed-Undo ABA, numeric
errors, seeded histories and saved rectangle/scalar continuation are covered.

Six sequential optimized candidate controls pass on 12,059,999 bytes / 90,000
rows: three caller and three refusal processes, with thirteen empty competitor
inventories. They exercise 36 actions, 36 saved mixed macros and 144 held
cancellation/typed-Undo refusals. Initial capture ranged 0.007–0.035 ms and
publication 0.032–0.097 ms. Worker medians ranged 17.490–92.086 ms. These are
candidate-only GPUI test-executor measurements; no old-source speedup, native
latency or Save timing is claimed. The default opt3/Thin-LTO/codegen-units1 test
harness uses unwind, while the shipping app uses abort. The preserved optimized
binary was built once in 1,197.173 s; no profile override was used.

Local evidence remains ignored under `gates/` and `log/`. Deciding hashes:

- Leaf source manifest: `caebb0f7d988427b3cccd03ced0f0d87b11e1c6e36cb941809866ce667a2f6e3`.
- Reviewed source patch: `734a0a555ff1dfa9e284ba37c8cd7c42c4f92b2221eac143d13363e814588968`.
- Final 77-entry leaf evidence: `ea9dd3f76f6734b95cb9b8d614b9d023037e8ef6a4389429b47031d8b9f917e2`.
- Optimized binary: `447bec658a8ae0a0a0ab560b7076809cda494661661b9c95f83232e5dfd74a79`.
- Optimized results: `e40c84cf6c3121f9fa0dcbcbc9b1b610650de9dc46bc32a9bb1d5eca8c06e178`.
- Main gate snapshot: `93755b1b9d7b02d80fbf32e3c9dad1a9dd185bf61611a1f508d81b38efe203a8`.
- Main test binary: `8bde216135fbd850cda1da77cebbeb18d840afe43a97c7706502d2aebbd3a2ec`.

Earlier compile/fixture errors and the first warning-bearing candidate are
preserved separately and excluded from the deciding result. Native Windows
visual/input/rectangle verification remains open; the unavailable native bridge
has not changed. Mac testing is deferred. The full Windows parity goal remains
active.
