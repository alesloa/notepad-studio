# Gates: search macro replay

Scope: capture search parameters for new recordings, preserve persisted legacy names, and replay searches through their actual handlers.

- [x] G1: behavior established from Notepad++ and real recording failure reproduced
  EVIDENCE: FindReplaceDlg.cpp:4328-4389 records strings/flags and selection intent; NppCommands.cpp:83-149 groups playback Undo. Actual baseline four find/replace handlers recorded zero events, expected four. Root reviewed original implementation patchB3F12A4D566A0195CCB04E029CE9F8F2BAFBC3F4F70BB5078DDA3ECBF1BA00DA.
- [x] G2: recorded searches survive save/reload and replay with original parameters and directions
  EVIDENCE: Root main search_macro group15 passed/1 timing ignored0.41s, log/search-macros-main-focused.txt. Actual registered record/save/TOML reload/fresh-editor replay covers four events, Normal/Extended/regex, explicit and dialog directions, captures, named groups, zero-length replacement and repeated playback. Old names remain readable.
- [x] G3: replacement replay preserves text fidelity, read-only guards and one Undo; unsupported legacy events report clearly
  EVIDENCE: Same main group checks Unicode/CRLF/BOM/UTF16 metadata, playback selection scope, read-only/IME/rectangle refusal,12 held-worker cancellation/stale controls, independent user history, failed-prefix grouping and Undo/Redo. Legacy parameterless search and Save refuse the complete macro before edits. Direct tabs.clear fixture was repaired to restore neutral tab before paint; valid closure paths already settle tabs, no product crash claimed.
- [x] G4: focused checks pass and reviewed patch is ready for integration
  EVIDENCE: Parent reviewed/applied cap addendum D48F0C152A314EF16F7BED927DC4DF570E8C2D447FBBF7FC99C75518B7767EC4. Main log/search-macros-main-focused-v2.txt passes 17 tests/1 timing ignored in 7.37s, including two genuine cap REDs and exact 1 MiB/live-operation controls. Final leaf build and Clippy pass with no new changed-line diagnostics. Default optimized 12,582,973-byte synthetic caller log/search-macro-release-bench.txt passes: direct dispatch/capture/apply/wall 0.053/0.011/0.124/11.664ms, macro 0.054/0.002/0.066/10.457ms, Escape0.008ms/no open transaction. No native responsiveness or speedup claim. Root combined gates remain separate.
