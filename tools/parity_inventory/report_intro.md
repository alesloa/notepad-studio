# Notepad++ static menu parity inventory

This is a source-route audit, not a claim of complete behavior parity. The tables enumerate every static command appearance in `IDR_M30_MENU`, including both Language layouts and menu-bar controls. Generated metadata contains factual labels, command identifiers, chords and source references. No Notepad++ implementation source is included.

<!-- INVENTORY_COUNTS -->

## Verification limits

`routed` means the command has a declared action, an installed static menu item, a CATALOG entry and a registered production handler. It does not establish correct edge cases, native input delivery, Windows visuals or Mac behavior. `partial` records a source-confirmed behavioral or surface difference. `action-only` has a registered action but lacks the required menu/catalog surface. `alternate-only` has a related working caller outside the static main menu. `status-only` only explains an unsupported operation. `stub` has an actual caller reporting unimplemented behavior. `missing` means no mapped static main-menu/catalog route was found, not that every related behavior is absent everywhere.

Oracle sources are the local `notepad-plus-plus-master/PowerEditor/src` tree. `@` numbers in the oracle columns refer to `Notepad_plus.rc` and `Parameters.cpp`; dispatch references refer to `NppCommands.cpp`. All switch cases are reported, including later macro-recordability cases. A case reference alone does not prove its implementation. Local links identify the exact Notepad Studio declaration, menu, catalog, handler and registration line.

The complete resource has no preprocessor conditional directives inside it. Both Language menus are static appearances of the same command IDs. `Notepad_plus.cpp:217-220` removes one layout at runtime according to `_isLangMenuCompact`. The inventory retains both and reports unique IDs separately. Top-level `＋` and `✕` reuse File New/Close IDs and carry `HELP`; the `▼` submenu has its own disabled dynamic placeholder. `GRAYED` placeholders and updater entries remain in the static count even when runtime configuration changes them. MRU files, saved macros, saved Run commands, plugin commands and externally added languages are runtime entries outside this static resource.

The Rust scanner excludes test source files, `#[cfg(test)]` items and the undeclared `src/app/keymap/` files. It masks comments, ordinary/raw strings and character literals while preserving line offsets. Handler evidence must name a method actually registered by `Workspace::render` or a closure registered by `install_global_actions`; declarations or stale comments alone do not qualify.

Default chords come directly from live `winKeyDefs[]` and `scintKeyDefs[]`, preserving every assigned alternative and redirect. Unassigned means a zero/VK_NULL key; no-row means neither table provides the menu command. CATALOG Windows/Mac defaults are reported independently. Notepad Studio maps Scintilla Undo/Redo/Delete/Select All and zoom rows to their redirected menu IDs. Numpad representations can differ at the GPUI boundary; literal source comparison is not a keyboard-delivery test.

## Confirmed next work

Copy All Filenames and Copy All File Paths now have the required menu, CATALOG, registered handler and macro routes. Nine actual-caller tests cover visual order, independent duplicate names/paths, Unicode/untitled labels, CRLF aggregation, unchanged edit state, saved macro replay against current tabs and cancellation of an older bookmark copy. `Notepad_plus.rc:511-512`, `NppCommands.cpp:1529-1549,4590-4591` and `Common.cpp:1088-1106` supplied behavior only. Notepad Studio uses its current window's single tab strip; split views remain a separate infrastructure gap.

Copy Bookmarked Lines also has its menu, CATALOG, handler and persisted macro route. Tests cover loaded CRLF/CR/LF and BOM/UTF-16, unchanged document/history, empty output, background cancellation and stale focus/document replies. Large manual copies prepare in the background; large recording/playback remains unavailable. See [scope and limits](bookmark-copy.md). Its connected route is counted below without implying that these remaining macro limits match Notepad++.

1. Add the six missing sort variants. Case-insensitive lexical, locale and length ascending/descending commands are absent from the menu/action/catalog route while `src/edit_ops/line_ops.rs:459-555` implements related lexical/integer/decimal sorts. Locale collation needs an explicit cross-platform policy and oracle edge-case tests.
2. Add the remaining bulk bookmark operations. Cut/paste/remove require single-Undo, annotations, read-only and macro tests; large Copy Bookmarked Lines replay needs background macro continuation.
3. Wire File Rename and Move to Recycle Bin into actions, menu and CATALOG. Their tab-popup callers already call `open_tab_rename` and `move_tab_to_trash`. This is a confirmed missing route, not an absent underlying file operation. The deletion caller now confirms, rejects stale document/window replies and preserves the original tab on recovery failure; its thirteen caller tests use a temporary backend. Rename ownership/error behavior still needs review before reusing it.
4. Fix or remove the tab-popup Print stub; File Print and Print Now have no static main-menu route. Printing is a larger native-platform feature and must not be represented as shipped by adding labels.
5. Audit encoding semantics before adding character sets. `src/workspace/format.rs:71-88` only changes the save encoding. `NppCommands.cpp:2991-3096` includes system-code-page checks, distinct dirty-state rules and a reload branch when the original file used an explicit character set. CATALOG currently associates Latin-1 with the oracle ANSI command. Character sets and the five explicit conversion commands are also missing. Any reinterpretation must preserve recovery data and reject unrepresentable writes correctly.

Search selection/volatile navigation, result focus/navigation, brace actions, five independent token styles, change history, multi-select match commands, path completion, Column Editor, folding, split views, monitoring, extra encodings and grammars, user-defined languages, Tools hashes, Window sorting and several Settings/Help routes remain visible in the full tables. Product-specific Help links and updater policy need decisions rather than copying Notepad++ branding. New search macros capture parameters; legacy parameterless search/Save replay and native completion/proofreading QA remain outside this source-route inventory.

## Reproduce

Run `python tools/parity_inventory/generate.py --oracle <Notepad++ checkout>/PowerEditor/src` to regenerate the Markdown and factual JSON. Add `--check` to compare both outputs. Git revisions and documentation changes do not affect the generated source identity, so committing this report does not invalidate the check. Run `python -m unittest discover -s tools/parity_inventory -p 'test_*.py'` for parser/count checks. The generator fails on unparsed resource lines, unresolved command numbers, unparsed CATALOG rows, unclassified mapped routes, or a differing independent MENUITEM count. It uses only the Python standard library; no product dependency or build step is added.

<!-- INVENTORY_SOURCES -->

## Full static resource
