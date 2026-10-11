# Gates: Notepad++ menu parity inventory

Scope: enumerate the full static main menu oracle and map current menu/action/catalog routes with evidence and prioritized gaps.

- [x] G1: every static oracle command enumerated with ID, menu path and source line
  EVIDENCE: Full IDR_M30_MENU has 581 appearances, 481 unique IDs and 45 separators; independent MENUITEM count agrees. Every literal RC/Parameters line is checked; duplicate100 equals second Language98 plus New/Close2.
- [x] G2: current routes classified with implementation/shortcut evidence and explicit verification limits
  EVIDENCE: Baseline255 CATALOG rows; unique classes routed162, partial8, action-only2, alternate-only4, status-only1, stub1, missing303. Doc explicitly separates connected source routes from verified behavior/native visuals; production scanner excludes tests/dead keymaps.
- [x] G3: confirmed missing or stub routes prioritized and checked against actual callers
  EVIDENCE: docs/notepadpp-parity-inventory.md documents clipboard, sort, bookmark, File route and encoding gaps with callers; Print is an actual tab-popup stub. Clipboard and bookmark leaves dispatched; save-policy distinction from old handoff corrected separately.
- [x] G4: inventory counts validated and standalone artifact ready for review
  EVIDENCE: Root reviewed/copied six artifacts, regenerated on main, ran --check and ten unittests successfully0.012s. Metadata pins deterministic audited Rust and six oracle hashes; commit-independent source identity regression passes. Logs log/resumed-inventory-main-{generate,check}.txt.

Final source: 324 Rust files, source identity103a595416cfc74a02d01036d85a612c3f4bc572dab13307b8e8357de6d2b948. CATALOG258; unique classes routed165/partial8/action-only2/alternate-only4/status-only1/stub1/missing300. Final generator --check and eleven tests pass0.022s, log/resumed-inventory-final-{check,tests}-v2.txt. Clipboard and bookmark commands integrated; deletion confirmation fixes an existing popup, leaving its missing main-menu route explicit. Root final review found return-type Menu blocks falsely reparenting siblings: log/inventory-menu-path-red.txt is genuine RED. Direct-field names and arrow-block exclusion now preserve nested/sibling/computed-name cases; generated EOF whitespace corrected. File and Edit paths independently spotchecked.
