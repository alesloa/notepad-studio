# Windows line sorts

Own implementation: src/edit_ops/line_sort.rs, src/edit_ops/line_sort_tests.rs, src/app/menus/edit_line_sort_tests.rs and src/locale_sort_windows.rs. The frozen sort-owned patch contains these untracked modules, six actions/menu/CATALOG/replay registrations, the line-span visibility change, cfg Windows module/dependency registration, Cargo.lock and README/change.log additions. It excludes every bookmark-owned prepared/history/clipboard change and all gates paths.

The first ownership package omitted the six npp_defaults no-shortcut fixture rows; root added them on main. gates/sort-default-fixture.patch records just those rows (SHA256 ee655d504efedf21ecbd17b866cf4e1e005a61bd2980bf97ee0365fb3447d3ba). gates/sort-owned-complete.patch includes the original frozen patch plus that fixture delta (SHA256 a708f0073f8f772c5abac8654afc0bb4c4fe4d001ad839f3fc5b0730bd24471a); reverse apply check passed. The original integrated patch remains a46a74cc9c08a1faaa5019e35f85c40144c846867cb9761881a799a3751d4a0e. Do not apply the fixture twice to root's existing integration.

| Action | Notepad++ ID | Numeric ID | Menu source |
|---|---|---:|---|
| EditSortLinesLexIgnoreCaseAsc | IDM_EDIT_SORTLINES_LEXICO_CASE_INSENS_ASCENDING | 42080 | Notepad_plus.rc:547 |
| EditSortLinesLexIgnoreCaseDesc | IDM_EDIT_SORTLINES_LEXICO_CASE_INSENS_DESCENDING | 42081 | Notepad_plus.rc:555 |
| EditSortLinesLocaleAsc | IDM_EDIT_SORTLINES_LOCALE_ASCENDING | 42100 | Notepad_plus.rc:548 |
| EditSortLinesLocaleDesc | IDM_EDIT_SORTLINES_LOCALE_DESCENDING | 42101 | Notepad_plus.rc:556 |
| EditSortLinesLengthAsc | IDM_EDIT_SORTLINES_LENGTH_ASCENDING | 42104 | Notepad_plus.rc:552 |
| EditSortLinesLengthDesc | IDM_EDIT_SORTLINES_LENGTH_DESCENDING | 42105 | Notepad_plus.rc:560 |

All six have no default shortcut, but their CATALOG rows allow rebinding. Their MacroEvent names are SortLinesLexIgnoreCaseAsc/Desc, SortLinesLocaleAsc/Desc and SortLinesLengthAsc/Desc; real actions are recorded, saved/reloaded and replayed in tests.

The four basic sorts compare UTF16 units. Case ignoring folds ASCII under the unchanged CRT C locale and stops comparison at NUL, while preserving all document bytes. Length counts UTF16 units. Rectangles select raw UTF16 key units using corner byte offsets plus virtual space: CP1252/Latin1 editing counts one byte per character; UTF8/BOM/UTF16 files use UTF8 editing offsets. Thin nonzero rectangles sort suffixes. Full lines move, and Studio preserves its rectangle geometry.

Microsoft's [CRT comparison documentation](https://learn.microsoft.com/en-us/cpp/c-runtime-library/reference/stricmp-wcsicmp-mbsicmp-stricmp-l-wcsicmp-l-mbsicmp-l) confirms the default C locale and explains that Latin1 case pairs require changing that locale. The oracle checkout has no such change; the pure comparator tests therefore distinguish accented case pairs while folding ASCII.

NppCommands.cpp:933–1063 and Scintilla Editor.cxx:6915–6923 reset basic stream selections forward to the selected bodies. Changed whole-document basic sorts move an empty caret to target start; unchanged text keeps its caret, following Selection.cxx:54–81,131–142. Locale preserves stream direction, selects complete lines including terminators, and carries the caret with its original line. SortLocale.cpp:28–332 supplies those separate rules and Windows user-default locale keys: case ignored, numbers ordered numerically, accents and symbols significant.

The basic mixed-EOL policy deliberately prioritizes text fidelity. ScintillaEditView.cpp:4474–4517 splits/joins only the configured EOL. Studio parses physical CR/LF/CRLF lines, retains per-position terminators, and refuses a permutation that would fuse CR and LF around an empty body. The focused tests prove this policy; mixed-EOL output is not claimed to equal the older oracle path. Whole-document basic equal-key order is stable in Studio; the oracle's std::sort does not specify that order.

Source text is capped at 32 MiB and 1,048,576 rows; aggregate locale keys at 64 MiB. The shared prepared engine caps history at 128 MiB and captured annotations at 65,536. Heavy work, Undo/Redo, annotation mapping, macro history merge and disposal use the background executor. Cancellation or a stale source/context returns before publication. The only new unsafe is the two audited LCMapStringEx calls approved by root after the user delegated the narrow exception decision.

Required shared seam is owned by the bookmarks worker: PreparedGroup::build_line_edits/set_after_selection/combine_history and the prepared edits/macro capture/continuation engine. Its sort call captures Encoding/EOL and passes SortFormat { eol, encoding } to prepare_sort. It records command.event(), stamps rectangle/context/encoding state, and resumes a mixed macro after completion. Use the owner’s frozen common files, not this leaf’s copied drafts. gates/sort-source.sha256 distinguishes owned files from the borrowed common snapshot; shared full-file hashes include both owners' registrations, while gates/sort-owned.patch includes only sort hunks.

Mixed macro grouping captures the full existing Undo timeline in bounded turns, including the preserved prefix, and builds the replacement history on a worker. Publication swaps the prepared Vec; old history disposal also runs on a worker. The full timeline has a cumulative 128 MiB capture limit. If it exceeds that limit after the macro's edits, grouping is refused with “The full Undo timeline exceeds the 128 MiB macro capture limit; Undo remains in separate steps”. The text and all original command-level Undo entries remain intact. This is an explicit limit on single-Undo grouping, not permission to discard history.

This leaf's deciding composition uses owner manifest 5d394dc2f0c6bbb2f39a4168989d64f00c1d59fd87b7a9b305379aadfc9e6951. The owner later appended only an ignored bookmark Paste benchmark body in src/edit_ops/bookmark_edits_tests.rs, producing manifest f1fd66578525f2fe11c528379c0bc35d4dff13f15b33c4a6c5e8b6f231d331d3. Every consumed production path and all owned sort caller/benchmark code remain identical. Root approved retaining the deciding release binary without restarting LTO for that unrelated ignored control; root's final combined checks use the owner's latest complete files. This leaf does not claim full 28-path identity with that later manifest.

The owner subsequently moved the unchanged clipboard_history.rs cfg(test) module to file end to fix items_after_test_module. That changes its raw source hash, but no production function body or test control. The final owner manifest is e522d0a87d3aca4332eb88d22396b6284c7d40eaac9535f22000fb1eec5ee014; comparison finds exactly those two borrowed test-only raw-file differences. The leaf's frozen Clippy log retains this shared placement warning; root's combined final checks verify the owner's placement correction. Owned sort source has no Clippy diagnostics. The production and sort benchmark paths measured here remain unchanged.

The deciding 5d394 composition passed cargo build (19.51s, 11 existing app warnings), cargo test -p notepad-studio (2527 unit + 3 integration passed, 27 ignored, 47.81s) and cargo clippy -p notepad-studio --all-targets (exit 0, 13.20s, 195 app/210 test warnings). No diagnostic names the owned sort, NLS or caller-test modules. The earlier two line_sort arity warnings were removed by SortFormat. The suite includes 16 transform/fidelity tests, two native NLS tests and nine actual action/menu/macro/Undo controls. The surface scanner reports zero bug findings; its cosmetic flags are the required action names, descriptive regression-test names and astral Unicode fixture.

Root's final composition, including the latest bookmark and file-operation owner patches, also passed: build 63.083s; 2574 unit + 3 integration tests, 29 ignored (unit 70.81s, total 193.337s); Clippy 33.416s with 195 app/209 test inherited warnings and zero new message/path diagnostics against the baseline. The inventory's 12 tests and --check passed, with 182 routed commands and 285 missing commands.

The frozen release test binary finished its default opt3/thinLTO/codegen-units1 build in 15m02s (SHA256 ea62f158c42dd982d035b59c17ee99a21a3cf41dd6f8075ee3fb1f48ce7ac1d4). After all three leaf warmups and root's combined checks finished, the compiler/link inventory was empty. File and bookmark timing windows completed first; sorting then ran three isolated processes on 12,599,999 bytes and 180,000 descending rows, with en-US user/system locale. All three controls passed, including equal output against the old lexical baseline, exact Undo/Redo restoration, native locale sorting followed by movement/insertion, and one Undo for that mixed macro.

| Operation | Capture median | Worker median | Publish median | Overall median (range) |
|---|---:|---:|---:|---:|
| Old synchronous lexical action | — | — | — | 41.848 ms (41.041–41.940) |
| Prepared case-ignoring sort | 0.016 ms | 66.497 ms | 0.042 ms | 72.928 ms (71.633–79.667) |
| Prepared Undo | 0.019 ms | 0.002 ms | 0.014 ms | 6.473 ms (6.378–6.610) |
| Prepared Redo | 0.016 ms | 0.002 ms | 0.014 ms | 6.157 ms (6.130–6.441) |
| Native locale sort + mixed macro | 0.006 ms | 165.480 ms | 0.046 ms | 188.160 ms (186.436–194.043) |

Each phase median is calculated independently across the three runs. Locale capture/worker/publish values describe the sort phase; overall time includes following macro edits and history merge. The baseline dispatch and prepared overall columns measure different boundaries: the old action is synchronous, while the new operation includes worker completion and fidelity checks. No total-time speedup is claimed. TestAppContext may execute a background task eagerly during dispatch, so its dispatch time does not establish native UI latency. Capture/publication costs and guarded worker controls demonstrate the prepared phases; native visual verification remains a root-owned residual.

Deciding logs are gates/release-sort-final-run1.log (SHA256 17317aeb6ff2b01ed336817626450bf300574f7bf54099beb4dfea863910072d), run2 (2aed0dd3b82e1bcba97f63c0a6589098780d42e96918ef079f10de064fefdd14) and run3 (e82a22799de378b3a396f20f963557aba17bb3c999acef6c535418e1c54a6eba). gates/release-sort-final-receipt.json records the quiet window and waited, hidden, redirected process runner; gates/sort-final-evidence.json stores all samples and summaries. An initial PowerShell GUI launch returned no output or exit status and was excluded. gates/sort-final-evidence.sha256 hashes the deciding artifacts; gates/sort-source.sha256 remains 0ad5dc4081c40dda75aaba5bf492d9a8868996376413d46602ec3a24fb9759bf. Gates remain local ignored artifacts. Both borrowed debug/release caches are idle and retained. Mac/Linux locale collation remains a separate gate: those builds return a clear unavailable status without editing.

The twelve older lexical/integer/decimal/reverse/shuffle/dedup callers now use
the prepared engine, guarded publication and saved mixed macros. UTF16 keys,
whole-request numeric refusal, dedupe EOF behavior and runtime rectangle
history have installed-caller controls. Combined main Windows checks and six
optimized candidate test-harness controls pass; native verification remains
open. See [legacy validation and explicit limits](windows-legacy-lines-validation-20261011.md).
The preceding six-sort evidence is historical and remains qualified as recorded.
