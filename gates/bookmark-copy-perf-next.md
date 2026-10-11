# Gates: repeated bookmark line lookup

Scope: avoid repeated backward scans for bookmarks within a physical line already copied.

- [x] G1: deterministic regression demonstrates repeated lookups through the real extraction helper
  EVIDENCE: Perf leaf log/bookmark-dense-red.txt observed4096 physical-line lookups where1 was expected. Thread-local test instrumentation counts the real extraction helper, with no production counter.
- [x] G2: covered offsets skip lookup while cancellation, EOF, CRLF, invalid offsets and caller output remain correct
  EVIDENCE: Parent log/bookmark-dense-main-focused.txt passes14 tests/1 manual timing ignored in4.64s. Actual read-only registered action also looks up one line and preserves text, UTF-16/EOL, selection, bookmarks, dirty state and Undo/Redo. Edge tests retain unsorted helper input behavior and check cancellation before covered-offset skipping.
- [x] G3: optimized before/after dense-case timing is reproducible from the exact pure source
  EVIDENCE: Parent log/build_bookmark_dense_bench.py extracts exact frozen-before/current-main bodies; log/bookmark_dense_bench_source.json matches the leaf hashes. rustc uses opt-level=3,lto=thin,codegen-units=1,panic=abort,strip=symbols and cached optimized Ropey. Parent log/bookmark-dense-main-optimized.txt:11,062,145 source bytes/4096 marks/262,145 output bytes,1.6592213s before vs425.6us after, output equal. Leaf independent1.8072563s vs610.2us. This pure helper comparison links no GPUI; no whole-app claim.
- [x] G4: reviewed additive patch integrated; parent focused checks pass with no new diagnostics
  EVIDENCE: Parent reviewed/applied additive patch53D28032418EA94136C6914C3A99D251C02A2778649A31A17FB9F122F4B6641E. Main source hashes3F080CC46F887479738CDAD6492F2B7E61949DBE40AC7849FF88C519CE3BBE7E/AE1EB78267F81001B06823EB39A48D94BDAB4FE83A10D1E6CDE7A434E67F25A1 exactly match corrected leaf. Parent focused14 pass; final combined build2471 unit+3 integration/Clippy pass, no new diagnostics. Leaf12 focused and Clippy pass independently. No source edits during parent final gates.
