# Physical line boundaries

Source and Raw View use CRLF, LF and CR as row boundaries. Vertical tab, form feed, NEL (U+0085), line separator (U+2028) and paragraph separator (U+2029) remain body characters. Movement, bookmarks, line editing, rectangle coordinates and search results now use the same row indices. The change affects indexing; it does not remove or replace characters.

This matches Notepad++'s default policy. Scintilla Document.cxx:169 initializes allowed line-end flags to Default(0);263–287 intersects them with lexer support. LexCPP.cxx:638–640 advertises Unicode support(1), which cannot enable the zero allowed flags. CellBuffer.cxx:578–593 treats only CR/LF as breaks when those flags remain zero. No enabling SetLineEndTypesAllowed call was found in PowerEditor. A plugin explicitly enabling Unicode line endings in Notepad++ is outside this default-policy contract.

Ropey remains at version1.6.1. The app disables its default Unicode-row feature and enables cr_lines and simd. The locked feature tree has no other Ropey consumer or feature enabler. Find in Files' forward row scanner and result-navigation terminator trimming follow the same physical policy. Word-character classification is unchanged. Existing physical-span widening stays in place.

The baseline caller tests produced20 genuine expected-output failures across the five separators: registered MoveDown stopped on the Unicode subrow; BookmarkToggle stored its start; EditDeleteLine left the prefix of the physical row; and all six rectangle sorts omitted a selected physical row. Four CRLF/LF/CR/empty/EOF control groups passed. Evidence: gates/G1-physical-lines-red.log, SHA256 a239a3e2f3e8051e651366d6ea6e86da2a810349754f6ef5efc0945d98b2c0db. Final fixtures scope settings, macros and recents before window creation and retain the session snapshot sink; recent.rs:152–164 asserts against reaching its process-global loader under that guard. The missing recent guard in the first candidate was corrected before full gates; it is a fixture-isolation correction, not a production defect.

The real Find-in-Files button→worker→result-navigation controls separately reproduced wrong row/snippet coordinates and collapsed Unicode matches at EOF for all five separators. The corrected regex-query log is gates/G1-find-results-regex-red.log, SHA256 d24b4af2a1abefa6cbcce174cac886feacc21988682970700423bea97a110ad8. An earlier Source action dispatch did not reach the dialog callback; a bare whitespace query was refused. Those invalid fixtures are excluded from defect evidence. Failed test assertions are not native application crashes.

After the policy change, the focused run passed36 controls with one ignored release control. It includes the original caller REDs, all six rectangle modes, correct Find-in-Files content/EOF navigation,15 encoding×EOL open/save/duplicate/Undo/Redo byte checks, old session selection/history, saved bookmark macro playback, and Reader/rich/CSV controls. Existing Cut, Find-in-Files and rectangle-case unit fixtures encoded the prior Unicode-row policy; their expectations now explicitly retain Unicode body text and physical row offsets. Their hostile-byte-boundary coverage remains. The case fixture selects both physical rows (head row1) and retains its separate stale rows10–12 refusal. This pure-helper fixture does not prove registered stale-geometry acceptance. The first full run had2604 passes and only that prior-policy case expectation failed; its captured actual text already had the required uppercase C/E. That run is retained as G4-cargo-test-prior-case-fixture.log, SHA25669ded96204e27505236fd5eae7b6baa79f5eeae236018a30a06e9e6602da2537.

Old session selections keep their byte offsets, clamped only at EOF or a UTF8 character boundary. Old bookmarks at Unicode subrow starts floor to the containing physical row and coalesce; other physical-row bookmarks remain. The session schema and Undo codec are unchanged. A serialized legacy Undo entry restores its original selection bytes and text through actual Undo/Redo actions.

Removing a bookmarked row deletes its marker. Undo restores its text without resurrecting that marker, so saved macro replay is tested after re-toggling the restored row. This is existing behavior, not a policy fix: Notepad_plus.cpp:3060–3068 explicitly deletes the marker before replacing the row; Scintilla CellBuffer.cxx:1159–1183 reinserts text during Undo, and PerLine.cxx:98–119 inserts fresh empty marker state. The invalid same-marker replay expectation is excluded.

UTF8, UTF8 BOM, UTF16LE/BE and ANSI (the app's Latin1/Windows-1252 encoding) controls preserve exact file bytes with LF/CRLF/CR, before edits and after Undo. All five Unicode separators are exercised in the Unicode encodings; ANSI exercises representable VT/FF and multibyte internal UTF8 characters. Unrepresentable NEL/LS/PS are explicitly rejected for ANSI. Reader paragraphs retain body characters and source ranges; rich-document paragraphs and CSV cells retain their separate structural boundaries, including quoted CSV newlines.

The ignored default-release caller control opens14,240,003-byte UTF8-BOM and25,280,002-byte UTF16BE files through the actual streaming path, verifies14,080,000 rope bytes and160,001 physical rows, dispatches movement/bookmark/save, and checks exact encoded bytes after both Saves. Its additional Duplicate→Undo sequence asserts restored source without asserting the intermediate duplicate; it is a restoration smoke check, not standalone large-edit or one-Undo proof. The15 small encoding/EOL controls separately prove changed text, one Undo entry and Undo/Redo bytes. TestAppContext does not establish native UI latency or replace visual verification. Mac/Linux execution and native visual verification remain separate gates.

The scoped pre-policy optimized comparison uses the prior frozen sort test binary only for the unchanged io_stream big-files control. Its consumed io_stream.rs, encoding.rs and line_ending.rs source hashes match this leaf exactly; it is not claimed to be the complete current source. The old binary lacks the later injectable restoration helper in io.rs. Whole/stream load timings precede that writer and are the comparable phases; Save timings are excluded from attribution to the line-policy change. This control constructs no Workspace/window and reaches no recents/config API. No total-time speedup is assumed.

The oracle directory has no Git metadata, so studied source is pinned by SHA256:

| File | SHA256 |
|---|---|
| scintilla/src/Document.cxx | b67676dbe5f7ba357d3258582b127302d5e4b6b35de959c4e4124f2e2fec857a |
| scintilla/src/CellBuffer.cxx | d51afe836fdd95c77d6dc2b379ef28def01bcbe2927e86d4516eb0fcf5ad3ec4 |
| scintilla/src/Editor.cxx | 305561a566b373a6aa4b1431d3f87c63b82cd6be0513d56fbbc13c38e4fdef7b |
| lexilla/lexers/LexCPP.cxx | 337fe39c40788f5795e522113583a9f9510b4c7875da0c9d537e1d41e2a4fde1 |
| PowerEditor/src/NppCommands.cpp | 5fda9f9dbf31f0208f90c8ec571b51acb723297b5aa228dbf5b8a160c0c8e510 |
| PowerEditor/src/Notepad_plus.cpp | 1e65be82d8009e21edf5791d5213661d68b2062253c661e16175605d400f9877 |
| PowerEditor/src/ScintillaComponent/FindReplaceDlg.cpp | e590ae9a0066267365c384b7feef27071c40aee7321048c32f9211ff84a25270 |

All Rust changes and tests are original. No new unsafe, dependency, vendor change, plugin ABI or persisted field was introduced. Local gates/logs remain ignored artifacts; this document retains the behavioral evidence and limits.

Final verification used detached leaf 088e8d209a5dfc226ec215da1a13853a412b9dac plus compile patch fa56bd046f6e6090ece6d3a810ed4de605ff0c0a90a2cc41f29d2da44a3f212a. The nine compiled runtime/config source hashes stayed unchanged through all measurements; subsequent edits update only this evidence prose. Required Windows checks passed: build 10.713s; 2605 unit and 3 integration tests, 30 ignored (unit 271.63s, total 289.405s); Clippy 38.741s. Canonical warning-message/normalized-path comparison retained 334 occurrences and 195 unique pairs with zero added or removed pairs; inherited 195 app/209 test warnings (191 duplicates) remain. The single default-release test build passed 1132.077s, using opt-level 3, thin LTO and codegen-units 1, with no profile overrides.

The quiet deciding slot ran nine sequential test processes: three baseline load controls, three candidate load controls and three Unicode caller controls. Every process exited 0 and ran exactly one deciding test. Compiler/link/clang/test inventories were empty before, between and after them; all other build owners held compilation and heavy work. The baseline executable SHA256 is ea62f158c42dd982d035b59c17ee99a21a3cf41dd6f8075ee3fb1f48ce7ac1d4 (source common manifest 5d394dc2f0c6bbb2f39a4168989d64f00c1d59fd87b7a9b305379aadfc9e6951). The candidate executable SHA256 is 0d96670712915029c6fe2a58b3ffc07f20e50162fef0d2729513f2859dad8974. Exact runtime/binary provenance is G4-source-provenance.json and G4-release-measurement-receipt.json; all three raw runs and samples are retained in ignored local gates.

Median whole-file/streamed load milliseconds, baseline→candidate, across the three runs:

- Single line, about 50MiB: 81/55→89/59.
- 5,000,000 CRLF rows: 139/149→132/171.
- Binary, 64MiB: 436/465→381/378.
- 64KiB rows with tabs, about 64MiB: 127/57→117/54.
- 64MiB CRLF with a late lone LF: 133/191→130/190.

These are loader-phase measurements with unchanged helper source, not a combined-app speedup claim. The fixtures above 50MiB still compare whole/streamed ropes and exact encoded output, including the two-pass fallback cases. Helper SHA256: io_stream.rs 450a533562310b8c50d18cb4b2a425be85bd830eac2ce321537ef20ed1126f61; encoding.rs e3246b131d86f48e680e9e7ed80d8618202eb98a4976d46ef0db3a6905168a01; line_ending.rs 2663cdf88df4e7fb066c618fe7db5d48ae6f4c5f0dc3485a65494ef7deaf4906.

The Unicode caller's median open/movement/Save milliseconds were 35.745/0.944/22.535 for UTF8 BOM and 83.878/0.861/47.998 for UTF16BE. All three runs retain the exact disk bytes and 160,001 physical rows. These Save timings describe this leaf's 088e8d2 writer; they do not measure the later main 10d47236c088ea83309505c0398334af7ddd035e Save-protection changes. The control's Duplicate→Undo result remains restoration smoke only, as described above. Additive integration with the newer Column/Save work, combined-source checks and native visual verification remain separate; this leaf does not establish those results.
