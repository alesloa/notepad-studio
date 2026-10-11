# Gates: completion and proofreading verification

Scope: verify completion placement/input and proofreading guards, fixing only reproduced defects.

- [x] G1: available native tooling inspected and native verification completed or exact prerequisite recorded
  EVIDENCE: Root independently imported @oai/sky; list_apps returned Computer Use native pipe is unavailable: failed to connect native pipe: The system cannot find the file specified. (os error 2). No native input/launch performed.
- [x] G2: actual editor action/layout regression coverage reviewed for zoom, pane edges, navigation, IME and correction safety
  EVIDENCE: Main actual Workspace sweep passes 32 layouts, 24 placed clear of the caret and 8 dismissed; CATALOG arrow/page navigation preserves text, caret and Undo. Leaf autocomplete 99, ime_ 29, proofreading 31 filtered groups pass; overlapping groups are not summed.
- [x] G3: any reproduced defects fixed with baseline-failing regression tests
  EVIDENCE: No product defect reproduced. Initial missing-caret assertion came from the zero-height test pane and was repaired in the fixture; no production change or crash fix is claimed.
- [x] G4: focused checks pass and reviewed patch or evidence report is ready
  EVIDENCE: Root reviewed/applied patch SHA256 6b59603e2139ddb557552b96a5bf9d737cd3945ec2c2a6d15f761286e8fe76fe; integrated source hash 419c3c557bf3f82ac895f4e21d571c7c062c9594c355c2987cb09d27fb2faa6b. Main test passed 1.02s, log/resumed-completion-main-focused.txt. Main full gates passed: build7.88s, 2419 unit plus3 integration/22 ignored, unit59.14s, Clippy9.01s retaining baseline195 app/209 test warnings,191 duplicates. Logs log/resumed-qa-main-{build,tests,clippy}.txt; no touched-code diagnostics.
