# Notepad++ menu parity inventory

This is a source audit, dated 2026-10-10, of the entire `IDR_M30_MENU` resource against Notepad Studio. It does not declare 1:1 behavior or visual parity. Menu exposure, a declared action, a catalog row, a handler, a passing regression, and a native UI check are different evidence.

Notepad Studio is pinned to commit **`270735511b9fce0d703a33568f13519f183b74f7`**. The behavior oracle is the local, unversioned source archive `../notepad-plus-plus-master`, identified as **Notepad++ v8.9.8.1** by `PowerEditor/src/resource.h:22`. The archive has no Git commit metadata; the hashes below identify what was read. This is not a claim about the latest release. Labels, command identifiers and shortcut assignments are functional metadata; the descriptions and Rust-side assessment here are original. No GPL implementation or assets were copied or translated.

The oracle contains **716 ordered resource nodes: 581 command occurrences, 90 popups, and 45 separators; 481 distinct command IDs**. Both Language trees are included because both occur in the resource. `PowerEditor/src/Notepad_plus.cpp:218` removes one at startup according to compact-menu preference. There are 13 runtime top-level menus, plus the resource’s New, recent-window dropdown, and Close buttons. Studio installs 11 main menus; **Tools and Window are absent** (`src/app/menus.rs:38`). On macOS an additional application menu is installed (`src/app/menus.rs:32`).

Later integrations are deliberately separate from the pinned comparison: `dd23181` integrates guarded tab popups and Rename, including exclusive no-overwrite rename; `80d23fd` includes that integration plus Source/CSV IME fixes. Their line numbers and tests must be checked at those commits rather than attributed to this base. Separate leaves address Quit/session-failure handling, parameter hints, and distinct menu indentation/line-duplicate actions. Consult their integrated commits and gates for current status; they do not retroactively change this pinned comparison.

## Evidence and status

| Code | Meaning |
| --- | --- |
| M | No corresponding installed menubar action was found in this base. This does not prove there is no internal helper, context-menu route, or related capability. |
| R/U | Menu route and handler/wiring exist; behavior against the oracle remains unverified. |
| S | An exposed handler reports an unimplemented feature or uses an explicit fallback. |
| D | A source-visible behavior or dispatch difference is established by the cited sources; its full runtime extent still needs an oracle comparison. |
| Context only | Existing tab-context helper is not exposed as the corresponding File action. |
| No row | No `CATALOG` entry: the action is absent from the catalog-driven Shortcut Mapper. This does not imply that it has no internal binding. |

In the complete tree, **RC** is `PowerEditor/src/Notepad_plus.rc`, **P** is `PowerEditor/src/Parameters.cpp`, **Cmd** is `PowerEditor/src/NppCommands.cpp`, and **C** is `src/shortcuts.rs`. Paths and lines refer to the pinned sources. “None assigned” means an explicit zero-key entry; “Not listed” means the key tables contain no assignment found for that ID or its documented Scintilla redirect. Catalog chords are **Windows/Linux → macOS**. A Scintilla keyboard alias does not prove menu-command equivalence; indentation and duplication show why.

Missing-menu assessments compare the complete installer at `src/app/menus.rs:31` with the menu builders and `CATALOG` at `src/shortcuts.rs:122`. They are exposure findings, not speculative declarations that an entire engine is absent. Popup and separator rows preserve every resource position; they are layout references, not behavior checks.

## Source identities

| Oracle file | SHA-256 |
| --- | --- |
| `PowerEditor/src/Notepad_plus.rc` | `cb930d135a47c2f8da40ddfab8cacc9d399621dc7927f4fab29fe031d6c139e9` |
| `PowerEditor/src/Parameters.cpp` | `99506f082fe12e8e77731273daa144b5813ddbfc325d29bedfec91a08fab652a` |
| `PowerEditor/src/menuCmdID.h` | `46c6bee79ccd8742f3f7de04ff9e674ee3f4e9bdaf32c512f4868071fb62b700` |
| `PowerEditor/src/NppCommands.cpp` | `5fda9f9dbf31f0208f90c8ec571b51acb723297b5aa228dbf5b8a160c0c8e510` |
| `PowerEditor/src/resource.h` | `1b10b92466d5b4b5f26b5c28f6911ea3312ec0ebb9a807d3247765a7bbaf06f8` |
| `PowerEditor/src/Notepad_plus.cpp` | `1e65be82d8009e21edf5791d5213661d68b2062253c661e16175605d400f9877` |
| `PowerEditor/src/ScintillaComponent/FindReplaceDlg.cpp` | `e590ae9a0066267365c384b7feef27071c40aee7321048c32f9211ff84a25270` |
| `PowerEditor/src/ScintillaComponent/ScintillaEditView.cpp` | `1330d3e6f1824a681b66fb20235d2cf1ff4c22898f616b36f6e832571db14304` |
| `PowerEditor/src/WinControls/StaticDialog/RunDlg/RunDlg.cpp` | `d73bca7b44a2699f1117fed2f648c26d8bae3d8fb338eb682ad6e74f4b1b0241` |
| `scintilla/src/Editor.cxx` | `305561a566b373a6aa4b1431d3f87c63b82cd6be0513d56fbbc13c38e4fdef7b` |

| Studio file | SHA-256 |
| --- | --- |
| `src/app/menus.rs` | `cbaafe500a4fb2724680a2e2ce1a6f80a5d70caa24bc756e48d0a21cbbf7d202` |
| `src/shortcuts.rs` | `9c11c52e2b6a9e54fb476e781ba57a1c4c41f5e7e76551460a8f587e12d112bd` |
| `src/workspace/render.rs` | `153785e373a52710bcf30c13f305d5a9efd56d840757d937983d6727e34b7344` |
| `src/workspace/edits.rs` | `da245a41a4ae9caaa2f162e786cf005441cd6ec19a7e92a000837dc1414be027` |
| `src/workspace/format.rs` | `514dd0c98f07fd04f1dc25404474788d0bd574404929bb0751303e38b5337ad4` |
| `src/workspace/find.rs` | `9e62b26d560e51ba76766e67f812bffb5d67a2e50e7e154daf5a4156c4782268` |
| `src/workspace/autocomplete.rs` | `f88a1a68953ace35ee811239eeccc2771660b99ab252f2180ed4e840c3693768` |
| `src/workspace/completion_api.rs` | `dc12bdc5fed8c1e7ff42890ab5cf203de8da2696b05e5c1b8637abca545b0171` |
| `src/shell.rs` | `04f1f44c04af6cbabd9f02aae93a8b0642b3d8b392b2f78a1371433e17aea646` |

## Findings that should drive the next work

| Priority / finding | Behavior evidence | Studio evidence and remaining work |
| --- | --- | --- |
| Data-loss boundary: Quit at this base ignores a recovery refusal | File Exit RC:483. | `src/app/menus.rs:98` calls `cx.quit()` directly; `src/main.rs:139` saves on app quit but discards the `save_session_now()` result at `:143`. This is a pinned-base finding; use the separate Quit leaf’s integrated commit and failure regressions for current status. |
| External terminal path handling needs a safety regression | File cmd/PowerShell RC:452–453 are not exposed here; tab and selection terminal routes are already reachable. | `src/shell.rs:68` embeds a directory in CMD command text; `src/shell.rs:84` constructs xterm text. `src/edit_ops/on_selection.rs:329` also invokes CMD’s `start` with a path. Space, `%`, `!`, shell metacharacters, Unicode, launcher failures, and cwd need neutral-path tests. This audit did not run an exploit or launch a terminal. No default-viewer regression is inferred: `src/shell.rs:119` uses the native `open` helper. |
| Menu indent is not keyboard Tab | Cmd:2059–2082 adjusts line indentation for a single caret/single-line selection; multi-line/multiple selections use Scintilla Tab/Backtab. Menu defaults are unset at P:159–160. | `src/app/menus/edit.rs:79` dispatches `EditInsertTab`; `src/workspace/edits.rs:813` inserts a literal Tab or fixed-width spaces over a selection. Decrease uses `EditOutdent` at menu:80. Distinct menu actions are needed while keeping keyboard IDs and saved overrides compatible. |
| Menu line duplication is not keyboard selection duplication | Cmd:2085–2087 sends LineDuplicate. `scintilla/src/Editor.cxx:4168` selects line duplication; `:3298` uses the caret line for that mode. P:561 separately assigns Ctrl-D to SelectionDuplicate; P:642 leaves menu LineDuplicate without a default. | `src/app/menus/edit.rs:107` uses `EditDuplicateLine`; `src/workspace/edits.rs:3288` duplicates selected text when nonempty. Keep Ctrl-D semantics and the persisted action ID; add a distinct menu route. Runtime selection, caret and undo comparison remains to be done. |
| Shortcut rebindability is incomplete | Most defaultless oracle commands still have entries in P:103 onward. A zero default does not mean they are excluded from a mapper. | 63 distinct static Studio menu action types have no catalog row. `src/ui/shortcut_mapper.rs:194` reads the catalog and `:278` enumerates it; `src/shortcuts.rs:626` registers only catalog entries. The complete list below is a bounded follow-up. Do not rename action IDs or invent defaults. |
| Function parameter hints are a stub at this base; other completion commands are absent | RC:574–577. | `src/workspace/autocomplete.rs:450` reports not implemented. Previous/Next Hint and Path Completion have no menu/action counterpart here. Optional function-name API completion does exist; the Edit menu’s introductory comment predates that implementation. |
| Column-selection entry is a linear fallback | RC:497; Ctrl/Alt selection modes also appear in P. | `src/edit_ops/begin_end_select.rs:59` delegates to linear selection and reports column mode unsupported. Declared rectangular movement actions are not proof that Column Editor or all multi-select menu commands are implemented. |
| Regex behavior differs | `PowerEditor/src/ScintillaComponent/FindReplaceDlg.cpp:36` selects Boost search; `:3025`–`:3045` configures empty matches and CRLF handling. | Rust regex compilation at `src/workspace/find.rs:49` rejects lookaround/backreferences; `:72` skips all empty matches. Tests `:2114` and `:2142` establish that limitation safely. Zero-width replacement, backward searches, CRLF-as-one and Unicode word boundaries require a shared oracle matrix. |
| Regex replacement uses literal text | `PowerEditor/src/ScintillaComponent/FindReplaceDlg.cpp:3198` and `:3667` call regex replacement; `PowerEditor/src/ScintillaComponent/ScintillaEditView.cpp:2954`–`:2963` sends ReplaceTargetRE. | `src/workspace/find.rs:1307` clones replacement text in regex mode and `:1336` passes it directly to the edit; Replace All similarly inserts the same literal string at `:1440`. No capture-expansion step occurs in those handlers. Establish capture, escaping and conditional-replacement examples in actual handlers before choosing compatible syntax. |
| Encoding selection omits the oracle’s legacy reload path | Cmd:2991–3097 chooses encoding mode; Cmd:3073–3085 reloads when a legacy encoding is active, otherwise Unicode mode changes can just mark dirty. Cmd:3200 onward provides distinct Convert commands. OS ANSI is selected at Cmd:3078; UTF-8-system handling appears at Cmd:2999. RC:872–995 includes charset and conversion menus. | `src/workspace/format.rs:75` explicitly does not re-decode original bytes. `src/text/encoding.rs:125` decodes the Latin-1 label with Windows-1252. Five menu actions exist, without catalog entries; legacy charsets and the distinct Convert submenu are missing. Unicode-to-Unicode behavior must be checked separately from legacy reinterpretation; byte-identical save tests do not establish either menu’s complete parity. |
| Entire Tools and Window menus are absent | RC:1281–1307 and RC:1330–1348. | `src/app/menus.rs:38` installs neither. Hash libraries used for recovery do not establish MD5/SHA generation dialogs or clipboard actions. MRU tab switching does not establish Window sorting or the Windows dialog. |
| File helpers already exist behind context menus | Rename RC:464 and Recycle Bin RC:475. | Context rows at `src/ui/tab_context_menu.rs:203` and `:209`; helpers `src/workspace/tabs.rs:687` and `:745`. Missing File actions are low-hanging exposure work after their refusal, UUID, race and focus behavior is tested. Rename’s later exclusive-rename integration is noted above, not counted at this base. |
| Save Copy, pinned tabs, explicit session dialogs and printing are absent | RC:462, :470, :477–481. | File builder `src/app/menus/file.rs:16` contains no corresponding action. Automatic recovery is not Load/Save Session. The tab-context Print row is disabled; it is not a File Print implementation. |
| View long tail is mostly unexposed | RC:748–869. | `src/app/menus/view.rs:11` exposes zoom, the Tab submenu, Word Wrap, Whitespace, folder panel and fullscreen, plus a theme toggle. Split views, folds, symbol variants, project panels, document map/list, function list, direction and Monitoring still need actions or validated existing logic. |
| Named Run commands have no per-command shortcut field | `PowerEditor/src/WinControls/StaticDialog/RunDlg/RunDlg.cpp:413`–`:454` creates a named command through a shortcut dialog and refreshes accelerators. | `src/run_command.rs:470` stores only id/name/command; `src/actions/run.rs:5` defines a dynamic ID payload, without a catalog row per saved command. F5 and Validate shortcuts.xml are wired and tested, but named-command shortcut editing is missing. |

## Bounded behavior checks already present

These tests passed on the pinned base. They prove their assertions, not complete Notepad++ parity. Native application visuals, desktop focus behavior, OS IME, external viewers/terminals, and macOS/Linux execution were not exercised by this inventory.

| Area | Exact regression evidence | What remains unverified |
| --- | --- | --- |
| File/Reload | `src/workspace/file_tab_action_tests.rs:183`, `:403`, `:439`, `:526`, `:566`; `src/ui/tab_reload_tests.rs:80`, `:429` | Native prompts, filesystem permissions on all platforms, full oracle session/Reload behavior. Tests cover captured UUIDs, late source/rich/draft changes, refusals and focus ownership. |
| Close | `src/workspace/close_batch_tests.rs:55`, `:86`; `src/workspace/close_async_tests.rs:52`, `:79`, `:140`, `:314` | Native Save dialogs and complete closing policy. Tests prove nonempty rendered strips, late edits/drafts and save races in synthetic windows. |
| View Tab | `src/workspace/tab_actions_tests.rs:71`, `:95`, `:125`, `:150` | Native checkmarks/chord labels and full MRU conventions. Tests dispatch visible slot/cycle/move/color actions and preserve document identity/history. |
| Completion | `src/workspace/autocomplete_state_tests.rs:60`, `:84`, `:199`, `:270`, `:292`, `:339`; `src/workspace/autocomplete_api_tests.rs:60`, `:101` | Function parameter popup, path completion, full API XML conventions, native IME and placement. Word collection intentionally scans a bounded region around the caret rather than every word in arbitrarily large documents. |
| Language | `src/workspace/language.rs:122`, `:161`, `:176`, `:194`, `:247` | Lexer/token/style equivalence and the missing modes/UDL workflow. Tests preserve bytes, reject stale highlights and round-trip language choices. |
| Regex | `src/workspace/find.rs:2114`, `:2124`, `:2142` | Boost-compatible grammar, zero-width and replacement semantics; the tests intentionally confirm Rust limitations. |
| Macro playback/undo | `src/workspace/macro_context_tests.rs:129`, `:165`, `:318`; `src/workspace/macro_undo_tests.rs:151` | Every Edit submenu’s record/replay fidelity, saved-macro chords and cross-format support. Source-only playback guards are intentional and tested. |
| CSV recovery | `src/ui/csv_doc/recovery_tests.rs:54`, `:248`, `:349`, `:410`, `:482` | Native filesystem failures, all CSV dialects and desktop IME. These tests exercise drafts, source binding, focus and debounced persistence. |
| Run | `src/app/menus/run.rs:39`, `:71`; `src/run_command.rs:1177`, `:1236` | Named shortcuts and every platform launch convention. Tests cover menu/catalog F5, literal paths/argv and contextual expansion. |
| Shortcuts | `src/shortcuts.rs:738`, `:880`, `:903`, `:951` | Absent catalog rows and actual mapper interaction for new rows. Existing tests cover collisions and old override-file loading. |

Validation: `cargo build`, `cargo test -p notepad-studio` (**1513 unit tests + 1 integration test passed; 8 ignored**) and `cargo clippy -p notepad-studio --all-targets` passed on this base using LLVM clang-cl and a separate target directory. Clippy reports 196 existing app warnings and 210 in the test build (193 duplicates); this document changes no Rust code. Structural checks account for every RC node and command ID, catalog row, static menu action and cited source location.

## Language modes and API aliases

Studio exposes 26 modes: Normal Text plus 25 grammars (`src/app/menus/language.rs:15`, `src/workspace/language.rs:49`). Twenty-three menu modes have oracle built-in counterparts; Markdown, SCSS and TSX are Studio additions in this resource comparison. The oracle contains 94 built-in modes including Normal Text, plus User-Defined and its management commands. Both oracle arrangements below reference the same command IDs. Missing a menu mode does not establish whether a related grammar is bundled indirectly.

Optional API files are original/user-provided under the config API directory; they are not copied from the GPL oracle. `src/workspace/completion_api.rs:99` fixes the filename mapping. Function-name completion, a language grammar and function-parameter hints are separate features. The Shell menu uses the Bash grammar and `bash.xml`; C# uses `cs.xml`; C++ uses `cpp.xml`. `docs/api-completion.md:16` documents these conventions. Imported API compatibility beyond existing parser tests remains unverified.

| Menu mode / action | Oracle ID or Studio addition | API filename | Handler mapping |
| --- | --- | --- | --- |
| Language: None (Normal Text) / `LanguageNormalText` | `IDM_LANG_TEXT` | `normal.xml` | `src/workspace/language.rs:48` |
| Language: C / `LanguageC` | `IDM_LANG_C` | `c.xml` | `src/workspace/language.rs:49` |
| Language: C# / `LanguageCSharp` | `IDM_LANG_CS` | `cs.xml` | `src/workspace/language.rs:50` |
| Language: C++ / `LanguageCpp` | `IDM_LANG_CPP` | `cpp.xml` | `src/workspace/language.rs:51` |
| Language: CSS / `LanguageCss` | `IDM_LANG_CSS` | `css.xml` | `src/workspace/language.rs:52` |
| Language: Go / `LanguageGo` | `IDM_LANG_GOLANG` | `go.xml` | `src/workspace/language.rs:53` |
| Language: HTML / `LanguageHtml` | `IDM_LANG_HTML` | `html.xml` | `src/workspace/language.rs:54` |
| Language: Java / `LanguageJava` | `IDM_LANG_JAVA` | `java.xml` | `src/workspace/language.rs:55` |
| Language: JavaScript / `LanguageJavaScript` | `IDM_LANG_JS` | `javascript.xml` | `src/workspace/language.rs:56` |
| Language: JSON / `LanguageJson` | `IDM_LANG_JSON` | `json.xml` | `src/workspace/language.rs:57` |
| Language: Lua / `LanguageLua` | `IDM_LANG_LUA` | `lua.xml` | `src/workspace/language.rs:58` |
| Language: Markdown / `LanguageMarkdown` | Studio addition | `markdown.xml` | `src/workspace/language.rs:59` |
| Language: PHP / `LanguagePhp` | `IDM_LANG_PHP` | `php.xml` | `src/workspace/language.rs:60` |
| Language: PowerShell / `LanguagePowerShell` | `IDM_LANG_POWERSHELL` | `powershell.xml` | `src/workspace/language.rs:61` |
| Language: Python / `LanguagePython` | `IDM_LANG_PYTHON` | `python.xml` | `src/workspace/language.rs:62` |
| Language: Ruby / `LanguageRuby` | `IDM_LANG_RUBY` | `ruby.xml` | `src/workspace/language.rs:63` |
| Language: Rust / `LanguageRust` | `IDM_LANG_RUST` | `rust.xml` | `src/workspace/language.rs:64` |
| Language: SCSS / `LanguageScss` | Studio addition | `scss.xml` | `src/workspace/language.rs:65` |
| Language: Shell / `LanguageShell` | `IDM_LANG_BASH` | `bash.xml` | `src/workspace/language.rs:66` |
| Language: SQL / `LanguageSql` | `IDM_LANG_SQL` | `sql.xml` | `src/workspace/language.rs:67` |
| Language: Swift / `LanguageSwift` | `IDM_LANG_SWIFT` | `swift.xml` | `src/workspace/language.rs:68` |
| Language: TOML / `LanguageToml` | `IDM_LANG_TOML` | `toml.xml` | `src/workspace/language.rs:69` |
| Language: TSX / `LanguageTsx` | Studio addition | `tsx.xml` | `src/workspace/language.rs:70` |
| Language: TypeScript / `LanguageTypeScript` | `IDM_LANG_TYPESCRIPT` | `typescript.xml` | `src/workspace/language.rs:71` |
| Language: XML / `LanguageXml` | `IDM_LANG_XML` | `xml.xml` | `src/workspace/language.rs:72` |
| Language: YAML / `LanguageYaml` | `IDM_LANG_YAML` | `yaml.xml` | `src/workspace/language.rs:73` |

## Dynamic menus, platforms and other boundaries

| Surface | Current evidence | Difference / unverified behavior |
| --- | --- | --- |
| Recent files | `src/app/menus/file.rs:30` inserts a snapshot list; global `FileOpenRecent` route in `src/app/menus.rs:143`. | Recent Window resource IDs are dynamic window lists, not the same as file MRU. Grouping, placement, missing-file behavior and count preferences need oracle runtime comparison. |
| Saved macros | `src/app/menus/macros.rs:16` appends escaped saved names as `MacroRunSaved`; `src/workspace/macros.rs:300` resolves the name. | Static recording is a combined toggle instead of separate Start and Stop rows; Save and Run Multiple order differs. Saved-macro-specific shortcut editing is not established by the static catalog. |
| Named Run | `src/app/menus/run.rs:13` appends literal escaped names as `RunSavedCommand`. | Per-command chords missing; shell/file-association semantics differ from native argv execution. `src/run_command.rs:927` intentionally rejects Windows .cmd/.bat executables instead of silently invoking a shell. |
| Plugins | `src/app/menus/plugins.rs:21` reads a startup plugin snapshot; `crates/notepad-studio-plugin/src/ffi.rs` defines Studio’s C ABI. | Studio plugins target its own cross-platform ABI, not binary-compatible Notepad++ Windows DLLs. Hot reload is not supported (`src/app/menus/plugins.rs:8`). No plugins installed is an extra submenu; native plugin behavior remains unverified. |
| File shell routes | `src/shell.rs:17`, `:106`; File wrappers `src/workspace/file_tab_actions.rs:136`, `:152`. | Native default-viewer launch accepts literal path data on Windows. macOS reveals with Finder; Linux opens the parent directory without an equivalent file-selection operation. Launcher success is not completion or error feedback from the external application. |
| Mac conventions | `src/shortcuts.rs:124`–`:141`, `:251`; `src/app/menus.rs:32`. | Cmd-Q, Cmd-Shift-S Save As, Cmd-Alt-S Save All, Cmd-L Go to Line, and application-menu placement are deliberate platform adjustments. The full table records actual mirrors, not an assumption that every Ctrl chord becomes Cmd. |
| Fixed menu rows beyond this oracle resource | New Window, Restore Closed Tab, Save As .docx, Text Tools, Find All Current, Count, Find in Projects, theme toggle, combined function/word completion, Markdown/SCSS/TSX, selection New Instance/Terminal/Email. | An extra fixed row is not necessarily a Studio-exclusive behavior: Restore Closed Tab has an oracle key-table entry, for example. These rows do not close unrelated resource IDs. Several also lack a catalog row. Plain text, CSV preview, Markdown and rich Doc View have different edit contexts that need explicit refusal/undo tests. |
| Themes/preferences/dialogs | Style Configurator and Preferences have global handlers at `src/app/menus.rs:115`–`:121`. | Handler existence does not prove all controls, theme XML or preference defaults match. A complete dialog, toolbar, editor/tab-context, theme, language and file-fidelity audit remains outside this menu inventory. |

## Existing static menu actions missing from CATALOG

These 63 action types can be reached through a static menu but are absent from the mapper catalog at this base. Multiple menu occurrences of one action count once. The computed platform file-manager label is accounted for and already has a catalog row. Payload actions generated from recent files, saved macros, named Run commands and plugins are separate dynamic contracts, not part of these 63 rows.

| Existing action | Menu evidence | Oracle counterpart, if any | Handler / wiring |
| --- | --- | --- | --- |
| `EditClearReadOnlyFlag` | `src/app/menus/edit.rs:48` | Studio addition / no IDR_M30_MENU counterpart mapped | `src/edit_ops/readonly.rs:38` |
| `EditCopyDirPath` | `src/app/menus/edit.rs:70` | `IDM_EDIT_CURRENTDIRTOCLIP` | `src/edit_ops/clipboard_ops.rs:54` |
| `EditCopyFilename` | `src/app/menus/edit.rs:69` | `IDM_EDIT_FILENAMETOCLIP` | `src/edit_ops/clipboard_ops.rs:35` |
| `EditCopyFullPath` | `src/app/menus/edit.rs:68` | `IDM_EDIT_FULLPATHTOCLIP` | `src/edit_ops/clipboard_ops.rs:20` |
| `EditEolMacintosh` | `src/app/menus/edit.rs:189` | `IDM_FORMAT_TOMAC` | `src/workspace/format.rs:51` |
| `EditEolToSpace` | `src/app/menus/edit.rs:204` | `IDM_EDIT_EOL2WS` | `src/edit_ops/blank_ops.rs:50` |
| `EditEolUnix` | `src/app/menus/edit.rs:188` | `IDM_FORMAT_TOUNIX` | `src/workspace/format.rs:47` |
| `EditEolWindows` | `src/app/menus/edit.rs:187` | `IDM_FORMAT_TODOS` | `src/workspace/format.rs:38` |
| `EditFindAllCurrent` | `src/app/menus/search.rs:15` | Studio addition / no IDR_M30_MENU counterpart mapped | `src/workspace/find.rs:1204` |
| `EditFindCount` | `src/app/menus/search.rs:16` | Studio addition / no IDR_M30_MENU counterpart mapped | `src/workspace/find.rs:1175` |
| `EditInsertDateTimeCustomized` | `src/app/menus/edit.rs:59` | `IDM_EDIT_INSERT_DATETIME_CUSTOMIZED` | `src/edit_ops/insert.rs:61` |
| `EditInsertDateTimeLong` | `src/app/menus/edit.rs:58` | `IDM_EDIT_INSERT_DATETIME_LONG` | `src/edit_ops/insert.rs:45` |
| `EditInsertDateTimeShort` | `src/app/menus/edit.rs:57` | `IDM_EDIT_INSERT_DATETIME_SHORT` | `src/edit_ops/insert.rs:26` |
| `EditInvertCase` | `src/app/menus/edit.rs:97` | `IDM_EDIT_INVERTCASE` | `src/edit_ops/case_ops.rs:80` |
| `EditOpenCharacterPanel` | `src/app/menus/edit.rs:45` | `IDM_EDIT_CHAR_PANEL` | `src/app/menus.rs:127` |
| `EditOpenClipboardHistory` | `src/app/menus/edit.rs:44` | `IDM_EDIT_CLIPBOARDHISTORY_PANEL` | `src/app/menus.rs:124` |
| `EditOpenContainingFolder` | `src/app/menus/edit.rs:226` | `IDM_EDIT_OPENSELECTEDFILEFOLDERINEXPLORER` | `src/edit_ops/on_selection.rs:85` |
| `EditOpenContainingFolderInTerminal` | `src/app/menus/edit.rs:227` | Studio addition / no IDR_M30_MENU counterpart mapped | `src/edit_ops/on_selection.rs:111` |
| `EditOpenSelectedFile` | `src/app/menus/edit.rs:224` | `IDM_EDIT_OPENSELECTEDFILETOEDIT` | `src/edit_ops/on_selection.rs:26` |
| `EditOpenSelectedInNewInstance` | `src/app/menus/edit.rs:225` | Studio addition / no IDR_M30_MENU counterpart mapped | `src/edit_ops/on_selection.rs:57` |
| `EditOpenTextTools` | `src/app/menus/edit.rs:43` | Studio addition / no IDR_M30_MENU counterpart mapped | `src/app/menus.rs:121` |
| `EditRandomCase` | `src/app/menus/edit.rs:98` | `IDM_EDIT_RANDOMCASE` | `src/edit_ops/case_ops.rs:89` |
| `EditRandomizeLineOrder` | `src/app/menus/edit.rs:151` | `IDM_EDIT_SORTLINES_RANDOMLY` | `src/edit_ops/line_ops.rs:578` |
| `EditRemoveConsecutiveDuplicateLines` | `src/app/menus/edit.rs:109` | `IDM_EDIT_REMOVE_CONSECUTIVE_DUP_LINES` | `src/edit_ops/line_ops.rs:610` |
| `EditRemoveDuplicateLines` | `src/app/menus/edit.rs:108` | `IDM_EDIT_REMOVE_ANY_DUP_LINES` | `src/edit_ops/line_ops.rs:592` |
| `EditRemoveEmptyLines` | `src/app/menus/edit.rs:117` | `IDM_EDIT_REMOVEEMPTYLINES` | `src/edit_ops/line_ops.rs:670` |
| `EditRemoveEmptyLinesContainingBlank` | `src/app/menus/edit.rs:118` | `IDM_EDIT_REMOVEEMPTYLINESWITHBLANK` | `src/edit_ops/line_ops.rs:682` |
| `EditRemoveUnnecessaryBlankAndEol` | `src/app/menus/edit.rs:205` | `IDM_EDIT_TRIMALL` | `src/edit_ops/blank_ops.rs:59` |
| `EditReverseLineOrder` | `src/app/menus/edit.rs:150` | `IDM_EDIT_SORTLINES_REVERSE_ORDER` | `src/edit_ops/line_ops.rs:566` |
| `EditSearchSelectionOnInternet` | `src/app/menus/edit.rs:231` | `IDM_EDIT_SEARCHONINTERNET` | `src/edit_ops/on_selection.rs:146` |
| `EditSendSelectionViaEmail` | `src/app/menus/edit.rs:232` | Studio addition / no IDR_M30_MENU counterpart mapped | `src/edit_ops/on_selection.rs:164` |
| `EditSortLinesDecCommaAsc` | `src/app/menus/edit.rs:133` | `IDM_EDIT_SORTLINES_DECIMALCOMMA_ASCENDING` | `src/edit_ops/line_ops.rs:507` |
| `EditSortLinesDecCommaDesc` | `src/app/menus/edit.rs:137` | `IDM_EDIT_SORTLINES_DECIMALCOMMA_DESCENDING` | `src/edit_ops/line_ops.rs:519` |
| `EditSortLinesDecDotAsc` | `src/app/menus/edit.rs:141` | `IDM_EDIT_SORTLINES_DECIMALDOT_ASCENDING` | `src/edit_ops/line_ops.rs:531` |
| `EditSortLinesDecDotDesc` | `src/app/menus/edit.rs:145` | `IDM_EDIT_SORTLINES_DECIMALDOT_DESCENDING` | `src/edit_ops/line_ops.rs:543` |
| `EditSortLinesIntAsc` | `src/app/menus/edit.rs:131` | `IDM_EDIT_SORTLINES_INTEGER_ASCENDING` | `src/edit_ops/line_ops.rs:483` |
| `EditSortLinesIntDesc` | `src/app/menus/edit.rs:132` | `IDM_EDIT_SORTLINES_INTEGER_DESCENDING` | `src/edit_ops/line_ops.rs:495` |
| `EditSortLinesLexAsc` | `src/app/menus/edit.rs:123` | `IDM_EDIT_SORTLINES_LEXICOGRAPHIC_ASCENDING` | `src/edit_ops/line_ops.rs:459` |
| `EditSortLinesLexDesc` | `src/app/menus/edit.rs:127` | `IDM_EDIT_SORTLINES_LEXICOGRAPHIC_DESCENDING` | `src/edit_ops/line_ops.rs:471` |
| `EditSpaceToTabAll` | `src/app/menus/edit.rs:211` | `IDM_EDIT_SW2TAB_ALL` | `src/edit_ops/blank_ops.rs:87` |
| `EditSpaceToTabLeading` | `src/app/menus/edit.rs:212` | `IDM_EDIT_SW2TAB_LEADING` | `src/edit_ops/blank_ops.rs:97` |
| `EditStreamUncomment` | `src/app/menus/edit.rs:165` | `IDM_EDIT_STREAM_UNCOMMENT` | `src/edit_ops/comment_ops.rs:45` |
| `EditTabToSpace` | `src/app/menus/edit.rs:210` | `IDM_EDIT_TAB2SW` | `src/edit_ops/blank_ops.rs:73` |
| `EditToggleReadOnly` | `src/app/menus/edit.rs:47` | `IDM_EDIT_TOGGLEREADONLY` | `src/edit_ops/readonly.rs:20` |
| `EditTrimLeadingAndTrailingWs` | `src/app/menus/edit.rs:200` | `IDM_EDIT_TRIM_BOTH` | `src/edit_ops/blank_ops.rs:41` |
| `EditTrimLeadingWs` | `src/app/menus/edit.rs:199` | `IDM_EDIT_TRIMLINEHEAD` | `src/edit_ops/blank_ops.rs:32` |
| `EditTrimTrailingWs` | `src/app/menus/edit.rs:198` | `IDM_EDIT_TRIMTRAILING` | `src/workspace/edits.rs:3932` |
| `EncodingLatin1` | `src/app/menus/encoding.rs:16` | `IDM_FORMAT_ANSI` | `src/workspace/format.rs:121` |
| `EncodingUtf16Be` | `src/app/menus/encoding.rs:15` | `IDM_FORMAT_UTF_16BE` | `src/workspace/format.rs:112` |
| `EncodingUtf16Le` | `src/app/menus/encoding.rs:14` | `IDM_FORMAT_UTF_16LE` | `src/workspace/format.rs:103` |
| `EncodingUtf8` | `src/app/menus/encoding.rs:12` | `IDM_FORMAT_AS_UTF_8` | `src/workspace/format.rs:85` |
| `EncodingUtf8Bom` | `src/app/menus/encoding.rs:13` | `IDM_FORMAT_UTF_8` | `src/workspace/format.rs:94` |
| `FileClearRecent` | `src/app/menus/file.rs:40` | Studio addition / no IDR_M30_MENU counterpart mapped | `src/app/menus.rs:171` |
| `MarkClearAll` | `src/app/menus/search.rs:26` | Studio addition / no IDR_M30_MENU counterpart mapped | `src/workspace/find.rs:1966` |
| `MarkCopy` | `src/app/menus/search.rs:27` | `IDM_SEARCH_MARKEDTOCLIP` | `src/workspace/find.rs:1988` |
| `OpenShortcutMapper` | `src/app/menus/settings.rs:16` | `IDM_SETTING_SHORTCUT_MAPPER` | `src/app/menus.rs:112` |
| `PluginsOpenFolder` | `src/app/menus/plugins.rs:21` | `IDM_SETTING_OPENPLUGINSDIR` | `src/app/menus.rs:176` |
| `RichDocSaveAsDocx` | `src/app/menus/file.rs:60` | Studio addition / no IDR_M30_MENU counterpart mapped | `src/workspace/edits.rs:3079` |
| `SearchFindInProjects` | `src/app/menus/search.rs:23` | Studio addition / no IDR_M30_MENU counterpart mapped | `src/workspace/find_in_files.rs:558` |
| `ViewShowWhitespace` | `src/app/menus/view.rs:20` | `IDM_VIEW_TAB_SPACE` | `src/workspace/edits.rs:1161` |
| `ViewToggleFolderPanel` | `src/app/menus/view.rs:22` | `IDM_VIEW_FILEBROWSER` | `src/workspace/edits.rs:3160` |
| `ViewToggleTheme` | `src/app/menus/view.rs:12` | Studio addition / no IDR_M30_MENU counterpart mapped | `src/workspace/edits.rs:1128` |
| `ViewWordWrap` | `src/app/menus/view.rs:19` | `IDM_VIEW_WRAP` | `src/workspace/edits.rs:3171` |

## Complete ordered IDR_M30_MENU comparison

Every node below appears once in resource order, including separators, both alternative Language trees and the three root buttons. The row marker `RC:n` uniquely identifies a node. For a missing item, the installer/builders/catalog cited above establish the exposure gap; a listed Cmd location points to the oracle dispatch for a subsequent behavior audit. It is not a claim that that handler has already been exercised.

### File

| Resource node / hierarchy | Oracle ID / dispatch | Oracle default | Studio route | Catalog Windows/Linux → Mac | Assessment |
| --- | --- | --- | --- | --- | --- |
| RC:445 File | popup | — | See children | — | Layout reference |
| RC:447 File > New | `IDM_FILE_NEW`; Cmd:162 | `ctrl-n` (P:108) | `FileNew`; `src/app/menus/file.rs:20`; `src/io.rs:29` | `ctrl-n` → `cmd-n`; C:126 | R/U |
| RC:448 File > Open... | `IDM_FILE_OPEN`; Cmd:222 | `ctrl-o` (P:109) | `FileOpen`; `src/app/menus/file.rs:21`; `src/io.rs:224` | `ctrl-o` → `cmd-o`; C:127 | R/U |
| RC:449 File > Open Containing Folder | popup | — | See children | — | Layout reference |
| RC:451 File > Open Containing Folder > Explorer | `IDM_FILE_OPEN_FOLDER`; Cmd:228 | None assigned (P:110) | `FileOpenContainingFolder`; `src/app/menus/file.rs:24`; `src/workspace/file_tab_actions.rs:136` | none → none; C:128 | R/U |
| RC:452 File > Open Containing Folder > cmd | `IDM_FILE_OPEN_CMD`; Cmd:241 | None assigned (P:111) | No corresponding installed menubar action located | — | M |
| RC:453 File > Open Containing Folder > PowerShell | `IDM_FILE_OPEN_POWERSHELL`; Cmd:248 | None assigned (P:112) | No corresponding installed menubar action located | — | M |
| RC:454 File > Open Containing Folder > separator | separator | — | — | — | Layout reference |
| RC:455 File > Open Containing Folder > Folder as Workspace | `IDM_FILE_CONTAININGFOLDERASWORKSPACE`; Cmd:273 | Not listed | No corresponding installed menubar action located | — | M |
| RC:457 File > Open in Default Viewer | `IDM_FILE_OPEN_DEFAULT_VIEWER`; Cmd:297 | None assigned (P:113) | `FileOpenDefaultViewer`; `src/app/menus/file.rs:26`; `src/workspace/file_tab_actions.rs:152` | none → none; C:129 | R/U |
| RC:458 File > Open Folder as Workspace... | `IDM_FILE_OPENFOLDERASWORKSPACE`; Cmd:325 | None assigned (P:114) | `FileOpenFolderAsWorkspace`; `src/app/menus/file.rs:27`; `src/io.rs:256` | none → none; C:131 | R/U |
| RC:459 File > Reload from Disk | `IDM_FILE_RELOAD`; Cmd:362 | `ctrl-r` (P:115) | `FileReloadFromDisk`; `src/app/menus/file.rs:28`; `src/workspace/file_tab_actions.rs:168` | `ctrl-r` → `cmd-r`; C:130 | R/U |
| RC:460 File > Save | `IDM_FILE_SAVE`; Cmd:441 | `ctrl-s` (P:116) | `FileSave`; `src/app/menus/file.rs:58`; `src/io.rs:439` | `ctrl-s` → `cmd-s`; C:132 | R/U |
| RC:461 File > Save As... | `IDM_FILE_SAVEAS`; Cmd:449 | `ctrl-alt-s` (P:117) | `FileSaveAs`; `src/app/menus/file.rs:59`; `src/io.rs:470` | `ctrl-alt-s` → `cmd-shift-s`; C:133 | R/U |
| RC:462 File > Save a Copy As... | `IDM_FILE_SAVECOPYAS`; Cmd:453 | None assigned (P:118) | No corresponding installed menubar action located | — | M |
| RC:463 File > Save All | `IDM_FILE_SAVEALL`; Cmd:445 | `ctrl-shift-s` (P:119) | `FileSaveAll`; `src/app/menus/file.rs:61`; `src/io.rs:554` | `ctrl-shift-s` → `cmd-alt-s`; C:134 | R/U |
| RC:464 File > Rename... | `IDM_FILE_RENAME`; Cmd:404 | None assigned (P:120) | Rename tab-context row; `src/ui/tab_context_menu.rs:203`; `src/workspace/tabs.rs:687` | No File action | Context only; File exposure missing; behavior U |
| RC:465 File > Close | `IDM_FILE_CLOSE`; Cmd:394 | `ctrl-w` (P:121) | `FileCloseTab`; `src/app/menus/file.rs:63`; `src/workspace/tabs.rs:269` | `ctrl-w` → `cmd-w`; C:135 | R/U |
| RC:466 File > Close All | `IDM_FILE_CLOSEALL`; Cmd:408 | `ctrl-shift-w` (P:122) | `FileCloseAllTabs`; `src/app/menus/file.rs:64`; `src/workspace/tabs.rs:284` | `ctrl-shift-w` → `cmd-shift-w`; C:136 | R/U |
| RC:467 File > Close Multiple Documents | popup | — | See children | — | Layout reference |
| RC:469 File > Close Multiple Documents > Close All but Active Document | `IDM_FILE_CLOSEALL_BUT_CURRENT`; Cmd:416 | None assigned (P:123) | `FileCloseAllButActive`; `src/app/menus/file.rs:68`; `src/workspace/file_tab_actions.rs:180` | none → none; C:137 | R/U |
| RC:470 File > Close Multiple Documents > Close All but Pinned Documents | `IDM_FILE_CLOSEALL_BUT_PINNED`; Cmd:421 | None assigned (P:124) | No corresponding installed menubar action located | — | M |
| RC:471 File > Close Multiple Documents > Close All to the Left | `IDM_FILE_CLOSEALL_TOLEFT`; Cmd:426 | None assigned (P:125) | `FileCloseAllToLeft`; `src/app/menus/file.rs:69`; `src/workspace/file_tab_actions.rs:193` | none → none; C:138 | R/U |
| RC:472 File > Close Multiple Documents > Close All to the Right | `IDM_FILE_CLOSEALL_TORIGHT`; Cmd:431 | None assigned (P:126) | `FileCloseAllToRight`; `src/app/menus/file.rs:70`; `src/workspace/file_tab_actions.rs:206` | none → none; C:139 | R/U |
| RC:473 File > Close Multiple Documents > Close All Unchanged | `IDM_FILE_CLOSEALL_UNCHANGED`; Cmd:436 | None assigned (P:127) | `FileCloseAllUnchanged`; `src/app/menus/file.rs:71`; `src/workspace/file_tab_actions.rs:219` | none → none; C:140 | R/U |
| RC:475 File > Move to Recycle Bin | `IDM_FILE_DELETE`; Cmd:399 | None assigned (P:128) | Move to Recycle Bin tab-context row; `src/ui/tab_context_menu.rs:209`; `src/workspace/tabs.rs:745` | No File action | Context only; File exposure missing; behavior U |
| RC:476 File > separator | separator | — | — | — | Layout reference |
| RC:477 File > Load Session... | `IDM_FILE_LOADSESSION`; Cmd:457 | None assigned (P:129) | No corresponding installed menubar action located | — | M |
| RC:478 File > Save Session... | `IDM_FILE_SAVESESSION`; Cmd:461 | None assigned (P:130) | No corresponding installed menubar action located | — | M |
| RC:479 File > separator | separator | — | — | — | Layout reference |
| RC:480 File > Print... | `IDM_FILE_PRINT`; Cmd:469 | `ctrl-p` (P:131) | No corresponding installed menubar action located | — | M |
| RC:481 File > Print Now | `IDM_FILE_PRINTNOW`; Cmd:465 | None assigned (P:132) | No corresponding installed menubar action located | — | M |
| RC:482 File > separator | separator | — | — | — | Layout reference |
| RC:483 File > Exit | `IDM_FILE_EXIT`; Cmd:473 | `alt-f4` (P:134) | `Quit`; `src/app/menus/app_menu.rs:16`; `src/app/menus.rs:98` | `alt-f4` → `cmd-q`; C:124 | D at pinned base: direct quit; later update noted separately |
### Edit

| Resource node / hierarchy | Oracle ID / dispatch | Oracle default | Studio route | Catalog Windows/Linux → Mac | Assessment |
| --- | --- | --- | --- | --- | --- |
| RC:486 Edit | popup | — | See children | — | Layout reference |
| RC:488 Edit > Undo | `IDM_EDIT_UNDO`; Cmd:477 | `ctrl-z` (P:549), `alt-backspace` (P:550) | `EditUndo`; `src/app/menus/edit.rs:20`; `src/workspace/edits.rs:428` | `ctrl-z`, `alt-backspace` → `cmd-z`; C:144 | R/U |
| RC:489 Edit > Redo | `IDM_EDIT_REDO`; Cmd:486 | `ctrl-y` (P:551), `ctrl-shift-z` (P:552) | `EditRedo`; `src/app/menus/edit.rs:21`; `src/workspace/edits.rs:460` | `ctrl-y`, `ctrl-shift-z` → `cmd-shift-z`; C:145 | R/U |
| RC:490 Edit > separator | separator | — | — | — | Layout reference |
| RC:491 Edit > Cut | `IDM_EDIT_CUT`; Cmd:495 | `shift-delete` (P:139), `ctrl-x` (P:140) | `EditCut`; `src/app/menus/edit.rs:23`; `src/workspace/edits.rs:273` | `ctrl-x`, `shift-delete` → `cmd-x`; C:146 | R/U |
| RC:492 Edit > Copy | `IDM_EDIT_COPY`; Cmd:517 | `ctrl-insert` (P:142), `ctrl-c` (P:143) | `EditCopy`; `src/app/menus/edit.rs:24`; `src/workspace/edits.rs:219` | `ctrl-c`, `ctrl-insert` → `cmd-c`; C:147 | R/U |
| RC:493 Edit > Paste | `IDM_EDIT_PASTE`; Cmd:639 | `shift-insert` (P:145), `ctrl-v` (P:146) | `EditPaste`; `src/app/menus/edit.rs:25`; `src/workspace/edits.rs:329` | `ctrl-v`, `shift-insert` → `cmd-v`; C:148 | R/U |
| RC:494 Edit > Delete | `IDM_EDIT_DELETE`; Cmd:1437 | `delete` (P:547) | `EditDelete`; `src/app/menus/edit.rs:26`; `src/workspace/edits.rs:585` | `delete` → `delete`; C:151 | R/U |
| RC:495 Edit > Select All | `IDM_EDIT_SELECTALL`; Cmd:2054 | `ctrl-a` (P:546) | `EditSelectAll`; `src/app/menus/edit.rs:28`; `src/workspace/edits.rs:193` | `ctrl-a` → `cmd-a`; C:149 | R/U |
| RC:496 Edit > Begin/End Select | `IDM_EDIT_BEGINENDSELECT`; Cmd:922 | `ctrl-shift-b` (P:150) | `EditBeginEndSelect`; `src/app/menus/edit.rs:29`; `src/edit_ops/begin_end_select.rs:28` | `ctrl-shift-b` → `cmd-shift-b`; C:197 | R/U |
| RC:497 Edit > Begin/End Select in Column Mode | `IDM_EDIT_BEGINENDSELECT_COLUMNMODE`; Cmd:923 | `alt-shift-b` (P:151) | `EditBeginEndSelectColumnMode`; `src/app/menus/edit.rs:30`; `src/edit_ops/begin_end_select.rs:37` | `alt-shift-b` → none; C:198 | S/D: falls back to linear selection at src/edit_ops/begin_end_select.rs:59 |
| RC:498 Edit > separator | separator | — | — | — | Layout reference |
| RC:499 Edit > Insert | popup | — | See children | — | Layout reference |
| RC:501 Edit > Insert > Date Time (short) | `IDM_EDIT_INSERT_DATETIME_SHORT`; Cmd:168 | None assigned (P:206) | `EditInsertDateTimeShort`; `src/app/menus/edit.rs:57`; `src/edit_ops/insert.rs:26` | **No row** | R/U; no catalog row |
| RC:502 Edit > Insert > Date Time (long) | `IDM_EDIT_INSERT_DATETIME_LONG`; Cmd:169 | None assigned (P:207) | `EditInsertDateTimeLong`; `src/app/menus/edit.rs:58`; `src/edit_ops/insert.rs:45` | **No row** | R/U; no catalog row |
| RC:503 Edit > Insert > Date Time (customized) | `IDM_EDIT_INSERT_DATETIME_CUSTOMIZED`; Cmd:205 | None assigned (P:208) | `EditInsertDateTimeCustomized`; `src/app/menus/edit.rs:59`; `src/edit_ops/insert.rs:61` | **No row** | R/U; no catalog row |
| RC:505 Edit > Copy to Clipboard | popup | — | See children | — | Layout reference |
| RC:507 Edit > Copy to Clipboard > Copy Current Full File path | `IDM_EDIT_FULLPATHTOCLIP`; Cmd:1507 | None assigned (P:153) | `EditCopyFullPath`; `src/app/menus/edit.rs:68`; `src/edit_ops/clipboard_ops.rs:20` | **No row** | R/U; no catalog row |
| RC:508 Edit > Copy to Clipboard > Copy Current Filename | `IDM_EDIT_FILENAMETOCLIP`; Cmd:1509 | None assigned (P:154) | `EditCopyFilename`; `src/app/menus/edit.rs:69`; `src/edit_ops/clipboard_ops.rs:35` | **No row** | R/U; no catalog row |
| RC:509 Edit > Copy to Clipboard > Copy Current Dir. Path | `IDM_EDIT_CURRENTDIRTOCLIP`; Cmd:1508 | None assigned (P:155) | `EditCopyDirPath`; `src/app/menus/edit.rs:70`; `src/edit_ops/clipboard_ops.rs:54` | **No row** | R/U; no catalog row |
| RC:510 Edit > Copy to Clipboard > separator | separator | — | — | — | Layout reference |
| RC:511 Edit > Copy to Clipboard > Copy All Filenames | `IDM_EDIT_COPY_ALL_NAMES`; Cmd:1529 | None assigned (P:156) | No corresponding installed menubar action located | — | M |
| RC:512 Edit > Copy to Clipboard > Copy All File Paths | `IDM_EDIT_COPY_ALL_PATHS`; Cmd:1530 | None assigned (P:157) | No corresponding installed menubar action located | — | M |
| RC:514 Edit > Indent | popup | — | See children | — | Layout reference |
| RC:516 Edit > Indent > Increase Line Indent | `IDM_EDIT_INS_TAB`; Cmd:2059 | None assigned (P:159) | `EditInsertTab`; `src/app/menus/edit.rs:79`; `src/workspace/edits.rs:813` | `tab` → `tab`; C:166 | D: menu indent aliases keyboard Tab; see behavior findings |
| RC:517 Edit > Indent > Decrease Line Indent | `IDM_EDIT_RMV_TAB`; Cmd:2060 | None assigned (P:160) | `EditOutdent`; `src/app/menus/edit.rs:80`; `src/edit_ops/indent_ops.rs:23` | `shift-tab` → `shift-tab`; C:167 | D/U: menu decrease indent aliases keyboard Backtab; compare single-line selection |
| RC:519 Edit > Convert Case to | popup | — | See children | — | Layout reference |
| RC:521 Edit > Convert Case to > UPPERCASE | `IDM_EDIT_UPPERCASE`; Cmd:2166 | `ctrl-shift-u` (P:161) | `EditUppercase`; `src/app/menus/edit.rs:89`; `src/workspace/edits.rs:3531` | `ctrl-shift-u` → `cmd-shift-u`; C:179 | R/U |
| RC:522 Edit > Convert Case to > lowercase | `IDM_EDIT_LOWERCASE`; Cmd:2170 | `ctrl-u` (P:162) | `EditLowercase`; `src/app/menus/edit.rs:90`; `src/workspace/edits.rs:3544` | `ctrl-u` → `cmd-u`; C:180 | R/U |
| RC:523 Edit > Convert Case to > Proper Case | `IDM_EDIT_PROPERCASE_FORCE`; Cmd:2174 | `alt-u` (P:163) | `EditProperCaseForce`; `src/app/menus/edit.rs:92`; `src/edit_ops/case_ops.rs:44` | `alt-u` → none; C:183 | R/U |
| RC:524 Edit > Convert Case to > Proper Case (blend) | `IDM_EDIT_PROPERCASE_BLEND`; Cmd:2178 | `alt-shift-u` (P:164) | `EditProperCaseBlend`; `src/app/menus/edit.rs:93`; `src/edit_ops/case_ops.rs:53` | `alt-shift-u` → none; C:184 | R/U |
| RC:525 Edit > Convert Case to > Sentence case | `IDM_EDIT_SENTENCECASE_FORCE`; Cmd:2182 | `ctrl-alt-u` (P:165) | `EditSentenceCaseForce`; `src/app/menus/edit.rs:94`; `src/edit_ops/case_ops.rs:62` | `ctrl-alt-u` → `cmd-alt-u`; C:185 | R/U |
| RC:526 Edit > Convert Case to > Sentence case (blend) | `IDM_EDIT_SENTENCECASE_BLEND`; Cmd:2186 | `ctrl-alt-shift-u` (P:166) | `EditSentenceCaseBlend`; `src/app/menus/edit.rs:95`; `src/edit_ops/case_ops.rs:71` | `ctrl-alt-shift-u` → `cmd-alt-shift-u`; C:186 | R/U |
| RC:527 Edit > Convert Case to > iNVERT cASE | `IDM_EDIT_INVERTCASE`; Cmd:2190 | None assigned (P:167) | `EditInvertCase`; `src/app/menus/edit.rs:97`; `src/edit_ops/case_ops.rs:80` | **No row** | R/U; no catalog row |
| RC:528 Edit > Convert Case to > ranDOm CasE | `IDM_EDIT_RANDOMCASE`; Cmd:2194 | None assigned (P:168) | `EditRandomCase`; `src/app/menus/edit.rs:98`; `src/edit_ops/case_ops.rs:89` | **No row** | R/U; no catalog row |
| RC:530 Edit > Line Operations | popup | — | See children | — | Layout reference |
| RC:532 Edit > Line Operations > Duplicate Current Line | `IDM_EDIT_DUP_LINE`; Cmd:2085 | None assigned (P:642) | `EditDuplicateLine`; `src/app/menus/edit.rs:107`; `src/workspace/edits.rs:3288` | `ctrl-d` → `cmd-d`; C:168 | D: menu line duplicate aliases keyboard selection duplicate |
| RC:533 Edit > Line Operations > Remove Duplicate Lines | `IDM_EDIT_REMOVE_ANY_DUP_LINES`; Cmd:2095 | None assigned (P:170) | `EditRemoveDuplicateLines`; `src/app/menus/edit.rs:108`; `src/edit_ops/line_ops.rs:592` | **No row** | R/U; no catalog row |
| RC:534 Edit > Line Operations > Remove Consecutive Duplicate Lines | `IDM_EDIT_REMOVE_CONSECUTIVE_DUP_LINES`; Cmd:2089 | None assigned (P:169) | `EditRemoveConsecutiveDuplicateLines`; `src/app/menus/edit.rs:109`; `src/edit_ops/line_ops.rs:610` | **No row** | R/U; no catalog row |
| RC:535 Edit > Line Operations > Split Lines | `IDM_EDIT_SPLIT_LINES`; Cmd:2101 | `ctrl-i` (P:171) | `EditSplitLines`; `src/app/menus/edit.rs:113`; `src/edit_ops/line_ops.rs:629` | `ctrl-i` → `ctrl-i`; C:189 | R/U |
| RC:536 Edit > Line Operations > Join Lines | `IDM_EDIT_JOIN_LINES`; Cmd:2132 | `ctrl-j` (P:172) | `EditJoinLines`; `src/app/menus/edit.rs:114`; `src/edit_ops/line_ops.rs:646` | `ctrl-j` → `cmd-j`; C:190 | R/U |
| RC:537 Edit > Line Operations > Move Up Current Line | `IDM_EDIT_LINE_UP`; Cmd:2146 | `ctrl-shift-up` (P:173) | `EditMoveLineUp`; `src/app/menus/edit.rs:115`; `src/workspace/edits.rs:3379` | `ctrl-shift-up` → `alt-shift-up`; C:176 | R/U |
| RC:538 Edit > Line Operations > Move Down Current Line | `IDM_EDIT_LINE_DOWN`; Cmd:2150 | `ctrl-shift-down` (P:174) | `EditMoveLineDown`; `src/app/menus/edit.rs:116`; `src/workspace/edits.rs:3392` | `ctrl-shift-down` → `alt-shift-down`; C:177 | R/U |
| RC:539 Edit > Line Operations > Remove Empty Lines | `IDM_EDIT_REMOVEEMPTYLINES`; Cmd:2154 | None assigned (P:175) | `EditRemoveEmptyLines`; `src/app/menus/edit.rs:117`; `src/edit_ops/line_ops.rs:670` | **No row** | R/U; no catalog row |
| RC:540 Edit > Line Operations > Remove Empty Lines (Containing Blank characters) | `IDM_EDIT_REMOVEEMPTYLINESWITHBLANK`; Cmd:2160 | None assigned (P:176) | `EditRemoveEmptyLinesContainingBlank`; `src/app/menus/edit.rs:118`; `src/edit_ops/line_ops.rs:682` | **No row** | R/U; no catalog row |
| RC:541 Edit > Line Operations > Insert Blank Line Above Current | `IDM_EDIT_BLANKLINEABOVECURRENT`; Cmd:1081 | `ctrl-alt-enter` (P:177) | `EditInsertBlankLineAbove`; `src/app/menus/edit.rs:214`; `src/workspace/edits.rs:3971` | `ctrl-alt-enter` → `cmd-alt-enter`; C:195 | R/U |
| RC:542 Edit > Line Operations > Insert Blank Line Below Current | `IDM_EDIT_BLANKLINEBELOWCURRENT`; Cmd:1087 | `ctrl-alt-shift-enter` (P:178) | `EditInsertBlankLineBelow`; `src/app/menus/edit.rs:215`; `src/workspace/edits.rs:3993` | `ctrl-alt-shift-enter` → `cmd-shift-enter`; C:196 | R/U |
| RC:543 Edit > Line Operations > Reverse Line Order | `IDM_EDIT_SORTLINES_REVERSE_ORDER`; Cmd:943 | None assigned (P:193) | `EditReverseLineOrder`; `src/app/menus/edit.rs:150`; `src/edit_ops/line_ops.rs:566` | **No row** | R/U; no catalog row |
| RC:544 Edit > Line Operations > Randomize Line Order | `IDM_EDIT_SORTLINES_RANDOMLY`; Cmd:944 | None assigned (P:194) | `EditRandomizeLineOrder`; `src/app/menus/edit.rs:151`; `src/edit_ops/line_ops.rs:578` | **No row** | R/U; no catalog row |
| RC:545 Edit > Line Operations > separator | separator | — | — | — | Layout reference |
| RC:546 Edit > Line Operations > Sort Lines Lexicographically Ascending | `IDM_EDIT_SORTLINES_LEXICOGRAPHIC_ASCENDING`; Cmd:933 | None assigned (P:181) | `EditSortLinesLexAsc`; `src/app/menus/edit.rs:123`; `src/edit_ops/line_ops.rs:459` | **No row** | R/U; no catalog row |
| RC:547 Edit > Line Operations > Sort Lines Lex. Ascending Ignoring Case | `IDM_EDIT_SORTLINES_LEXICO_CASE_INSENS_ASCENDING`; Cmd:935 | None assigned (P:183) | No corresponding installed menubar action located | — | M |
| RC:548 Edit > Line Operations > Sort Lines In Locale Order Ascending | `IDM_EDIT_SORTLINES_LOCALE_ASCENDING`; Cmd:1066 | None assigned (P:185) | No corresponding installed menubar action located | — | M |
| RC:549 Edit > Line Operations > Sort Lines As Integers Ascending | `IDM_EDIT_SORTLINES_INTEGER_ASCENDING`; Cmd:937 | None assigned (P:187) | `EditSortLinesIntAsc`; `src/app/menus/edit.rs:131`; `src/edit_ops/line_ops.rs:483` | **No row** | R/U; no catalog row |
| RC:550 Edit > Line Operations > Sort Lines As Decimals (Comma) Ascending | `IDM_EDIT_SORTLINES_DECIMALCOMMA_ASCENDING`; Cmd:939 | None assigned (P:189) | `EditSortLinesDecCommaAsc`; `src/app/menus/edit.rs:133`; `src/edit_ops/line_ops.rs:507` | **No row** | R/U; no catalog row |
| RC:551 Edit > Line Operations > Sort Lines As Decimals (Dot) Ascending | `IDM_EDIT_SORTLINES_DECIMALDOT_ASCENDING`; Cmd:941 | None assigned (P:191) | `EditSortLinesDecDotAsc`; `src/app/menus/edit.rs:141`; `src/edit_ops/line_ops.rs:531` | **No row** | R/U; no catalog row |
| RC:552 Edit > Line Operations > Sort Lines By Length Ascending | `IDM_EDIT_SORTLINES_LENGTH_ASCENDING`; Cmd:945 | None assigned (P:179) | No corresponding installed menubar action located | — | M |
| RC:553 Edit > Line Operations > separator | separator | — | — | — | Layout reference |
| RC:554 Edit > Line Operations > Sort Lines Lexicographically Descending | `IDM_EDIT_SORTLINES_LEXICOGRAPHIC_DESCENDING`; Cmd:934 | None assigned (P:182) | `EditSortLinesLexDesc`; `src/app/menus/edit.rs:127`; `src/edit_ops/line_ops.rs:471` | **No row** | R/U; no catalog row |
| RC:555 Edit > Line Operations > Sort Lines Lex. Descending Ignoring Case | `IDM_EDIT_SORTLINES_LEXICO_CASE_INSENS_DESCENDING`; Cmd:936 | None assigned (P:184) | No corresponding installed menubar action located | — | M |
| RC:556 Edit > Line Operations > Sort Lines In Locale Order Descending | `IDM_EDIT_SORTLINES_LOCALE_DESCENDING`; Cmd:1067 | None assigned (P:186) | No corresponding installed menubar action located | — | M |
| RC:557 Edit > Line Operations > Sort Lines As Integers Descending | `IDM_EDIT_SORTLINES_INTEGER_DESCENDING`; Cmd:938 | None assigned (P:188) | `EditSortLinesIntDesc`; `src/app/menus/edit.rs:132`; `src/edit_ops/line_ops.rs:495` | **No row** | R/U; no catalog row |
| RC:558 Edit > Line Operations > Sort Lines As Decimals (Comma) Descending | `IDM_EDIT_SORTLINES_DECIMALCOMMA_DESCENDING`; Cmd:940 | None assigned (P:190) | `EditSortLinesDecCommaDesc`; `src/app/menus/edit.rs:137`; `src/edit_ops/line_ops.rs:519` | **No row** | R/U; no catalog row |
| RC:559 Edit > Line Operations > Sort Lines As Decimals (Dot) Descending | `IDM_EDIT_SORTLINES_DECIMALDOT_DESCENDING`; Cmd:942 | None assigned (P:192) | `EditSortLinesDecDotDesc`; `src/app/menus/edit.rs:145`; `src/edit_ops/line_ops.rs:543` | **No row** | R/U; no catalog row |
| RC:560 Edit > Line Operations > Sort Lines By Length Descending | `IDM_EDIT_SORTLINES_LENGTH_DESCENDING`; Cmd:946 | None assigned (P:180) | No corresponding installed menubar action located | — | M |
| RC:562 Edit > Comment/Uncomment | popup | — | See children | — | Layout reference |
| RC:564 Edit > Comment/Uncomment > Toggle Single Line Comment | `IDM_EDIT_BLOCK_COMMENT`; Cmd:2198 | `ctrl-q` (P:195) | `EditBlockComment`; `src/app/menus/edit.rs:160`; `src/workspace/edits.rs:3602` | `ctrl-q` → `cmd-/`; C:191 | R/U: menu calls this Toggle Block Comment; oracle Single Line |
| RC:565 Edit > Comment/Uncomment > Single Line Comment | `IDM_EDIT_BLOCK_COMMENT_SET`; Cmd:2202 | `ctrl-k` (P:196) | `EditBlockCommentSet`; `src/app/menus/edit.rs:161`; `src/edit_ops/comment_ops.rs:21` | `ctrl-k` → `ctrl-k`; C:192 | R/U: menu calls this Block Comment; oracle Single Line |
| RC:566 Edit > Comment/Uncomment > Single Line Uncomment | `IDM_EDIT_BLOCK_UNCOMMENT`; Cmd:2206 | `ctrl-shift-k` (P:197) | `EditBlockUncomment`; `src/app/menus/edit.rs:162`; `src/edit_ops/comment_ops.rs:33` | `ctrl-shift-k` → `cmd-shift-k`; C:193 | R/U: menu calls this Block Uncomment; oracle Single Line |
| RC:567 Edit > Comment/Uncomment > Block Comment | `IDM_EDIT_STREAM_COMMENT`; Cmd:2210 | `ctrl-shift-q` (P:198) | `EditStreamComment`; `src/app/menus/edit.rs:164`; `src/workspace/edits.rs:3856` | `ctrl-shift-q` → `cmd-shift-/`; C:194 | R/U: menu calls this Toggle Stream Comment; oracle Block |
| RC:568 Edit > Comment/Uncomment > Block Uncomment | `IDM_EDIT_STREAM_UNCOMMENT`; Cmd:2214 | None assigned (P:199) | `EditStreamUncomment`; `src/app/menus/edit.rs:165`; `src/edit_ops/comment_ops.rs:45` | **No row** | R/U: menu calls this Stream Uncomment; oracle Block; no catalog row |
| RC:570 Edit > Auto-Completion | popup | — | See children | — | Layout reference |
| RC:572 Edit > Auto-Completion > Function Completion | `IDM_EDIT_AUTOCOMPLETE`; Cmd:3871 | `ctrl-space` (P:200) | `EditAutocompleteFunc`; `src/app/menus/edit.rs:174`; `src/workspace/autocomplete.rs:401` | `ctrl-space` → `ctrl-space`; C:204 | R/U |
| RC:573 Edit > Auto-Completion > Word Completion | `IDM_EDIT_AUTOCOMPLETE_CURRENTFILE`; Cmd:3875 | `ctrl-enter` (P:202) | `EditAutocompleteWord`; `src/app/menus/edit.rs:175`; `src/workspace/autocomplete.rs:381` | `ctrl-enter` → `ctrl-enter`; C:205 | R/U |
| RC:574 Edit > Auto-Completion > Function Parameters Hint | `IDM_EDIT_FUNCCALLTIP`; Cmd:3883 | `ctrl-shift-space` (P:203) | `EditAutocompleteFuncParams`; `src/app/menus/edit.rs:178`; `src/workspace/autocomplete.rs:439` | `ctrl-shift-space` → `ctrl-shift-space`; C:207 | S: reports not yet implemented at src/workspace/autocomplete.rs:450 |
| RC:575 Edit > Auto-Completion > Function Parameters Previous Hint | `IDM_EDIT_FUNCCALLTIP_PREVIOUS`; Cmd:3887 | `alt-up` (P:204) | No corresponding installed menubar action located | — | M |
| RC:576 Edit > Auto-Completion > Function Parameters Next Hint | `IDM_EDIT_FUNCCALLTIP_NEXT`; Cmd:3891 | `alt-down` (P:205) | No corresponding installed menubar action located | — | M |
| RC:577 Edit > Auto-Completion > Path Completion | `IDM_EDIT_AUTOCOMPLETE_PATH`; Cmd:3879 | `ctrl-alt-space` (P:201) | No corresponding installed menubar action located | — | M |
| RC:579 Edit > EOL Conversion | popup | — | See children | — | Layout reference |
| RC:581 Edit > EOL Conversion > Windows (CR LF) | `IDM_FORMAT_TODOS`; Cmd:2971 | None assigned (P:209) | `EditEolWindows`; `src/app/menus/edit.rs:187`; `src/workspace/format.rs:38` | **No row** | R/U; no catalog row |
| RC:582 Edit > EOL Conversion > Unix (LF) | `IDM_FORMAT_TOUNIX`; Cmd:2972 | None assigned (P:210) | `EditEolUnix`; `src/app/menus/edit.rs:188`; `src/workspace/format.rs:47` | **No row** | R/U; no catalog row |
| RC:583 Edit > EOL Conversion > Macintosh (CR) | `IDM_FORMAT_TOMAC`; Cmd:2973 | None assigned (P:211) | `EditEolMacintosh`; `src/app/menus/edit.rs:189`; `src/workspace/format.rs:51` | **No row** | R/U; no catalog row |
| RC:585 Edit > Blank Operations | popup | — | See children | — | Layout reference |
| RC:587 Edit > Blank Operations > Trim Trailing Space | `IDM_EDIT_TRIMTRAILING`; Cmd:2218 | None assigned (P:212) | `EditTrimTrailingWs`; `src/app/menus/edit.rs:198`; `src/workspace/edits.rs:3932` | **No row** | R/U; no catalog row |
| RC:588 Edit > Blank Operations > Trim Leading Space | `IDM_EDIT_TRIMLINEHEAD`; Cmd:2228 | None assigned (P:213) | `EditTrimLeadingWs`; `src/app/menus/edit.rs:199`; `src/edit_ops/blank_ops.rs:32` | **No row** | R/U; no catalog row |
| RC:589 Edit > Blank Operations > Trim Leading and Trailing Space | `IDM_EDIT_TRIM_BOTH`; Cmd:2238 | None assigned (P:214) | `EditTrimLeadingAndTrailingWs`; `src/app/menus/edit.rs:200`; `src/edit_ops/blank_ops.rs:41` | **No row** | R/U; no catalog row |
| RC:590 Edit > Blank Operations > EOL to Space | `IDM_EDIT_EOL2WS`; Cmd:2248 | None assigned (P:215) | `EditEolToSpace`; `src/app/menus/edit.rs:204`; `src/edit_ops/blank_ops.rs:50` | **No row** | R/U; no catalog row |
| RC:591 Edit > Blank Operations > Trim both and EOL to Space | `IDM_EDIT_TRIMALL`; Cmd:2254 | None assigned (P:216) | `EditRemoveUnnecessaryBlankAndEol`; `src/app/menus/edit.rs:205`; `src/edit_ops/blank_ops.rs:59` | **No row** | R/U; no catalog row |
| RC:592 Edit > Blank Operations > separator | separator | — | — | — | Layout reference |
| RC:593 Edit > Blank Operations > TAB to Space | `IDM_EDIT_TAB2SW`; Cmd:2267 | None assigned (P:217) | `EditTabToSpace`; `src/app/menus/edit.rs:210`; `src/edit_ops/blank_ops.rs:73` | **No row** | R/U; no catalog row |
| RC:594 Edit > Blank Operations > Space to TAB (All) | `IDM_EDIT_SW2TAB_ALL`; Cmd:2275 | None assigned (P:218) | `EditSpaceToTabAll`; `src/app/menus/edit.rs:211`; `src/edit_ops/blank_ops.rs:87` | **No row** | R/U; no catalog row |
| RC:595 Edit > Blank Operations > Space to TAB (Leading) | `IDM_EDIT_SW2TAB_LEADING`; Cmd:2271 | None assigned (P:219) | `EditSpaceToTabLeading`; `src/app/menus/edit.rs:212`; `src/edit_ops/blank_ops.rs:97` | **No row** | R/U; no catalog row |
| RC:597 Edit > Paste Special | popup | — | See children | — | Layout reference |
| RC:599 Edit > Paste Special > Paste HTML Content | `IDM_EDIT_PASTE_AS_HTML`; Cmd:893 | None assigned (P:220) | No corresponding installed menubar action located | — | M |
| RC:600 Edit > Paste Special > Paste RTF Content | `IDM_EDIT_PASTE_AS_RTF`; Cmd:892 | None assigned (P:221) | No corresponding installed menubar action located | — | M |
| RC:601 Edit > Paste Special > separator | separator | — | — | — | Layout reference |
| RC:602 Edit > Paste Special > Copy Binary Content | `IDM_EDIT_COPY_BINARY`; Cmd:555 | None assigned (P:222) | No corresponding installed menubar action located | — | M |
| RC:603 Edit > Paste Special > Cut Binary Content | `IDM_EDIT_CUT_BINARY`; Cmd:556 | None assigned (P:223) | No corresponding installed menubar action located | — | M |
| RC:604 Edit > Paste Special > Paste Binary Content | `IDM_EDIT_PASTE_BINARY`; Cmd:665 | None assigned (P:224) | No corresponding installed menubar action located | — | M |
| RC:606 Edit > On Selection | popup | — | See children | — | Layout reference |
| RC:608 Edit > On Selection > Open File | `IDM_EDIT_OPENSELECTEDFILETOEDIT`; Cmd:759 | None assigned (P:225) | `EditOpenSelectedFile`; `src/app/menus/edit.rs:224`; `src/edit_ops/on_selection.rs:26` | **No row** | R/U; no catalog row |
| RC:609 Edit > On Selection > Open Containing Folder in Explorer | `IDM_EDIT_OPENSELECTEDFILEFOLDERINEXPLORER`; Cmd:758 | None assigned (P:226) | `EditOpenContainingFolder`; `src/app/menus/edit.rs:226`; `src/edit_ops/on_selection.rs:85` | **No row** | R/U; no catalog row |
| RC:610 Edit > On Selection > separator | separator | — | — | — | Layout reference |
| RC:611 Edit > On Selection > Redact Selection █ (Shift: ●) | `IDM_EDIT_REDACT_SELECTION`; Cmd:708 | Not listed | No corresponding installed menubar action located | — | M |
| RC:612 Edit > On Selection > separator | separator | — | — | — | Layout reference |
| RC:613 Edit > On Selection > Search on Internet | `IDM_EDIT_SEARCHONINTERNET`; Cmd:842 | None assigned (P:227) | `EditSearchSelectionOnInternet`; `src/app/menus/edit.rs:231`; `src/edit_ops/on_selection.rs:146` | **No row** | R/U; no catalog row |
| RC:614 Edit > On Selection > Change Search Engine... | `IDM_EDIT_CHANGESEARCHENGINE`; Cmd:885 | None assigned (P:228) | No corresponding installed menubar action located | — | M |
| RC:616 Edit > separator | separator | — | — | — | Layout reference |
| RC:617 Edit > Multi-select All | popup | — | See children | — | Layout reference |
| RC:619 Edit > Multi-select All > Ignore Case & Whole Word | `IDM_EDIT_MULTISELECTALL`; Cmd:2325 | None assigned (P:229) | No corresponding installed menubar action located | — | M |
| RC:620 Edit > Multi-select All > Match Case Only | `IDM_EDIT_MULTISELECTALLMATCHCASE`; Cmd:2326 | None assigned (P:230) | No corresponding installed menubar action located | — | M |
| RC:621 Edit > Multi-select All > Match Whole Word Only | `IDM_EDIT_MULTISELECTALLWHOLEWORD`; Cmd:2327 | None assigned (P:231) | No corresponding installed menubar action located | — | M |
| RC:622 Edit > Multi-select All > Match Case & Whole Word | `IDM_EDIT_MULTISELECTALLMATCHCASEWHOLEWORD`; Cmd:2328 | None assigned (P:232) | No corresponding installed menubar action located | — | M |
| RC:624 Edit > Multi-select Next | popup | — | See children | — | Layout reference |
| RC:626 Edit > Multi-select Next > Ignore Case & Whole Word | `IDM_EDIT_MULTISELECTNEXT`; Cmd:2349 | None assigned (P:233) | No corresponding installed menubar action located | — | M |
| RC:627 Edit > Multi-select Next > Match Case Only | `IDM_EDIT_MULTISELECTNEXTMATCHCASE`; Cmd:2350 | None assigned (P:234) | No corresponding installed menubar action located | — | M |
| RC:628 Edit > Multi-select Next > Match Whole Word Only | `IDM_EDIT_MULTISELECTNEXTWHOLEWORD`; Cmd:2351 | None assigned (P:235) | No corresponding installed menubar action located | — | M |
| RC:629 Edit > Multi-select Next > Match Case & Whole Word | `IDM_EDIT_MULTISELECTNEXTMATCHCASEWHOLEWORD`; Cmd:2352 | None assigned (P:236) | No corresponding installed menubar action located | — | M |
| RC:631 Edit > Undo the Latest Added Multi-Select | `IDM_EDIT_MULTISELECTUNDO`; Cmd:2364 | None assigned (P:237) | No corresponding installed menubar action located | — | M |
| RC:632 Edit > Skip Current & Go to Next Multi-select | `IDM_EDIT_MULTISELECTSSKIP`; Cmd:2372 | None assigned (P:238) | No corresponding installed menubar action located | — | M |
| RC:633 Edit > separator | separator | — | — | — | Layout reference |
| RC:634 Edit > Column Mode... | `IDM_EDIT_COLUMNMODETIP`; Cmd:1914 | Not listed | No corresponding installed menubar action located | — | M |
| RC:635 Edit > Column Editor... | `IDM_EDIT_COLUMNMODE`; Cmd:1931 | `alt-c` (P:240) | No corresponding installed menubar action located | — | M |
| RC:636 Edit > Character Panel | `IDM_EDIT_CHAR_PANEL`; Cmd:1093 | None assigned (P:241) | `EditOpenCharacterPanel`; `src/app/menus/edit.rs:45`; `src/app/menus.rs:127` | **No row** | R/U; no catalog row |
| RC:637 Edit > Clipboard History | `IDM_EDIT_CLIPBOARDHISTORY_PANEL`; Cmd:1110 | None assigned (P:242) | `EditOpenClipboardHistory`; `src/app/menus/edit.rs:44`; `src/app/menus.rs:124` | **No row** | R/U; no catalog row |
| RC:638 Edit > separator | separator | — | — | — | Layout reference |
| RC:639 Edit > Read-Only in Notepad++ | popup | — | See children | — | Layout reference |
| RC:641 Edit > Read-Only in Notepad++ > Read-Only on Current Document | `IDM_EDIT_TOGGLEREADONLY`; Cmd:2279 | None assigned (P:243) | `EditToggleReadOnly`; `src/app/menus/edit.rs:47`; `src/edit_ops/readonly.rs:20` | **No row** | R/U; no catalog row |
| RC:642 Edit > Read-Only in Notepad++ > Read-Only for All Documents | `IDM_EDIT_SETREADONLYFORALLDOCS`; Cmd:2286 | None assigned (P:244) | No corresponding installed menubar action located | — | M |
| RC:643 Edit > Read-Only in Notepad++ > Clear Read-Only for All Documents | `IDM_EDIT_CLEARREADONLYFORALLDOCS`; Cmd:2292 | None assigned (P:245) | No corresponding installed menubar action located | — | M |
| RC:645 Edit > Read-Only Attribute in Windows | `IDM_EDIT_TOGGLESYSTEMREADONLY`; Cmd:2298 | None assigned (P:246) | No corresponding installed menubar action located | — | M |
### Search

| Resource node / hierarchy | Oracle ID / dispatch | Oracle default | Studio route | Catalog Windows/Linux → Mac | Assessment |
| --- | --- | --- | --- | --- | --- |
| RC:648 Search | popup | — | See children | — | Layout reference |
| RC:650 Search > Find... | `IDM_SEARCH_FIND`; Cmd:1553 | `ctrl-f` (P:247) | `EditFind`; `src/app/menus/search.rs:12`; `src/workspace/find.rs:418` | `ctrl-f` → `cmd-f`; C:246 | R/U |
| RC:651 Search > Find in Files... | `IDM_SEARCH_FINDINFILES`; Cmd:1575 | `ctrl-shift-f` (P:248) | `SearchFindInFiles`; `src/app/menus/search.rs:22`; `src/workspace/find_in_files.rs:29` | `ctrl-shift-f` → `cmd-shift-f`; C:252 | R/U |
| RC:652 Search > Find Next | `IDM_SEARCH_FINDNEXT`; Cmd:1598 | `f3` (P:249) | `EditFindNext`; `src/app/menus/search.rs:13`; `src/workspace/find.rs:961` | `f3` → `f3`, `cmd-g`; C:248 | R/U |
| RC:653 Search > Find Previous | `IDM_SEARCH_FINDPREV`; Cmd:1599 | `shift-f3` (P:250) | `EditFindPrev`; `src/app/menus/search.rs:14`; `src/workspace/find.rs:972` | `shift-f3` → `shift-f3`, `cmd-shift-g`; C:249 | R/U |
| RC:654 Search > Select and Find Next | `IDM_SEARCH_SETANDFINDNEXT`; Cmd:1633 | `ctrl-f3` (P:251) | No corresponding installed menubar action located | — | M |
| RC:655 Search > Select and Find Previous | `IDM_SEARCH_SETANDFINDPREV`; Cmd:1634 | `ctrl-shift-f3` (P:252) | No corresponding installed menubar action located | — | M |
| RC:656 Search > Find (Volatile) Next | `IDM_SEARCH_VOLATILE_FINDNEXT`; Cmd:1690 | `ctrl-alt-f3` (P:253) | No corresponding installed menubar action located | — | M |
| RC:657 Search > Find (Volatile) Previous | `IDM_SEARCH_VOLATILE_FINDPREV`; Cmd:1691 | `ctrl-alt-shift-f3` (P:254) | No corresponding installed menubar action located | — | M |
| RC:658 Search > Replace... | `IDM_SEARCH_REPLACE`; Cmd:1554 | `ctrl-h` (P:255) | `EditReplace`; `src/app/menus/search.rs:18`; `src/workspace/find.rs:581` | `ctrl-h` → `cmd-alt-f`; C:250 | R/U |
| RC:659 Search > Incremental Search | `IDM_SEARCH_FINDINCREMENT`; Cmd:1581 | `ctrl-alt-i` (P:256) | No corresponding installed menubar action located | — | M |
| RC:660 Search > Search Results Window | `IDM_FOCUS_ON_FOUND_RESULTS`; Cmd:1680 | `f7` (P:257) | No corresponding installed menubar action located | — | M |
| RC:661 Search > Next Search Result | `IDM_SEARCH_GOTONEXTFOUND`; Cmd:1668 | `f4` (P:259) | No corresponding installed menubar action located | — | M |
| RC:662 Search > Previous Search Result | `IDM_SEARCH_GOTOPREVFOUND`; Cmd:1674 | `shift-f4` (P:258) | No corresponding installed menubar action located | — | M |
| RC:663 Search > Go to... | `IDM_SEARCH_GOTOLINE`; Cmd:1896 | `ctrl-g` (P:260) | `EditGotoLine`; `src/app/menus/search.rs:20`; `src/workspace/find.rs:733` | `ctrl-g` → `cmd-l`; C:251 | R/U |
| RC:664 Search > Go to Matching Brace | `IDM_SEARCH_GOTOMATCHINGBRACE`; Cmd:1940 | `ctrl-b` (P:261) | No corresponding installed menubar action located | — | M |
| RC:665 Search > Select All In-between {} [] or () | `IDM_SEARCH_SELECTMATCHINGBRACES`; Cmd:1941 | `ctrl-alt-b` (P:262) | No corresponding installed menubar action located | — | M |
| RC:666 Search > Mark... | `IDM_SEARCH_MARK`; Cmd:1555 | `ctrl-m` (P:266) | `SearchMark`; `src/app/menus/search.rs:25`; `src/workspace/find.rs:1767` | `ctrl-m` → `cmd-m`; C:253 | R/U |
| RC:667 Search > separator | separator | — | — | — | Layout reference |
| RC:668 Search > Change History | popup | — | See children | — | Layout reference |
| RC:670 Search > Change History > Go to Next Change | `IDM_SEARCH_CHANGED_NEXT`; Cmd:1979 | None assigned (P:264) | No corresponding installed menubar action located | — | M |
| RC:671 Search > Change History > Go to Previous Change | `IDM_SEARCH_CHANGED_PREV`; Cmd:1978 | None assigned (P:263) | No corresponding installed menubar action located | — | M |
| RC:672 Search > Change History > Clear Change History | `IDM_SEARCH_CLEAR_CHANGE_HISTORY`; Cmd:1983 | None assigned (P:265) | No corresponding installed menubar action located | — | M |
| RC:674 Search > separator | separator | — | — | — | Layout reference |
| RC:675 Search > Style All Occurrences of Token | popup | — | See children | — | Layout reference |
| RC:677 Search > Style All Occurrences of Token > Using 1st Style | `IDM_SEARCH_MARKALLEXT1`; Cmd:1718 | None assigned (P:267) | No corresponding installed menubar action located | — | M |
| RC:678 Search > Style All Occurrences of Token > Using 2nd Style | `IDM_SEARCH_MARKALLEXT2`; Cmd:1719 | None assigned (P:268) | No corresponding installed menubar action located | — | M |
| RC:679 Search > Style All Occurrences of Token > Using 3rd Style | `IDM_SEARCH_MARKALLEXT3`; Cmd:1720 | None assigned (P:269) | No corresponding installed menubar action located | — | M |
| RC:680 Search > Style All Occurrences of Token > Using 4th Style | `IDM_SEARCH_MARKALLEXT4`; Cmd:1721 | None assigned (P:270) | No corresponding installed menubar action located | — | M |
| RC:681 Search > Style All Occurrences of Token > Using 5th Style | `IDM_SEARCH_MARKALLEXT5`; Cmd:1722 | None assigned (P:271) | No corresponding installed menubar action located | — | M |
| RC:683 Search > Style One Token | popup | — | See children | — | Layout reference |
| RC:685 Search > Style One Token > Using 1st Style | `IDM_SEARCH_MARKONEEXT1`; Cmd:1745 | None assigned (P:272) | No corresponding installed menubar action located | — | M |
| RC:686 Search > Style One Token > Using 2nd Style | `IDM_SEARCH_MARKONEEXT2`; Cmd:1746 | None assigned (P:273) | No corresponding installed menubar action located | — | M |
| RC:687 Search > Style One Token > Using 3rd Style | `IDM_SEARCH_MARKONEEXT3`; Cmd:1747 | None assigned (P:274) | No corresponding installed menubar action located | — | M |
| RC:688 Search > Style One Token > Using 4th Style | `IDM_SEARCH_MARKONEEXT4`; Cmd:1748 | None assigned (P:275) | No corresponding installed menubar action located | — | M |
| RC:689 Search > Style One Token > Using 5th Style | `IDM_SEARCH_MARKONEEXT5`; Cmd:1749 | None assigned (P:276) | No corresponding installed menubar action located | — | M |
| RC:691 Search > Clear Style | popup | — | See children | — | Layout reference |
| RC:693 Search > Clear Style > Clear 1st Style | `IDM_SEARCH_UNMARKALLEXT1`; Cmd:1778 | None assigned (P:277) | No corresponding installed menubar action located | — | M |
| RC:694 Search > Clear Style > Clear 2nd Style | `IDM_SEARCH_UNMARKALLEXT2`; Cmd:1779 | None assigned (P:278) | No corresponding installed menubar action located | — | M |
| RC:695 Search > Clear Style > Clear 3rd Style | `IDM_SEARCH_UNMARKALLEXT3`; Cmd:1780 | None assigned (P:279) | No corresponding installed menubar action located | — | M |
| RC:696 Search > Clear Style > Clear 4th Style | `IDM_SEARCH_UNMARKALLEXT4`; Cmd:1781 | None assigned (P:280) | No corresponding installed menubar action located | — | M |
| RC:697 Search > Clear Style > Clear 5th Style | `IDM_SEARCH_UNMARKALLEXT5`; Cmd:1782 | None assigned (P:281) | No corresponding installed menubar action located | — | M |
| RC:698 Search > Clear Style > Clear all Styles | `IDM_SEARCH_CLEARALLMARKS`; Cmd:1850 | None assigned (P:282) | No corresponding installed menubar action located | — | M |
| RC:700 Search > Jump Up | popup | — | See children | — | Layout reference |
| RC:702 Search > Jump Up > 1st Style | `IDM_SEARCH_GOPREVMARKER1`; Cmd:1825 | `ctrl-shift-1` (P:283) | No corresponding installed menubar action located | — | M |
| RC:703 Search > Jump Up > 2nd Style | `IDM_SEARCH_GOPREVMARKER2`; Cmd:1826 | `ctrl-shift-2` (P:284) | No corresponding installed menubar action located | — | M |
| RC:704 Search > Jump Up > 3rd Style | `IDM_SEARCH_GOPREVMARKER3`; Cmd:1827 | `ctrl-shift-3` (P:285) | No corresponding installed menubar action located | — | M |
| RC:705 Search > Jump Up > 4th Style | `IDM_SEARCH_GOPREVMARKER4`; Cmd:1828 | `ctrl-shift-4` (P:286) | No corresponding installed menubar action located | — | M |
| RC:706 Search > Jump Up > 5th Style | `IDM_SEARCH_GOPREVMARKER5`; Cmd:1829 | `ctrl-shift-5` (P:287) | No corresponding installed menubar action located | — | M |
| RC:707 Search > Jump Up > Find Mark Style | `IDM_SEARCH_GOPREVMARKER_DEF`; Cmd:1830 | `ctrl-shift-0` (P:288) | No corresponding installed menubar action located | — | M |
| RC:709 Search > Jump Down | popup | — | See children | — | Layout reference |
| RC:711 Search > Jump Down > 1st Style | `IDM_SEARCH_GONEXTMARKER1`; Cmd:1800 | `ctrl-1` (P:289) | No corresponding installed menubar action located | — | M |
| RC:712 Search > Jump Down > 2nd Style | `IDM_SEARCH_GONEXTMARKER2`; Cmd:1801 | `ctrl-2` (P:290) | No corresponding installed menubar action located | — | M |
| RC:713 Search > Jump Down > 3rd Style | `IDM_SEARCH_GONEXTMARKER3`; Cmd:1802 | `ctrl-3` (P:291) | No corresponding installed menubar action located | — | M |
| RC:714 Search > Jump Down > 4th Style | `IDM_SEARCH_GONEXTMARKER4`; Cmd:1803 | `ctrl-4` (P:292) | No corresponding installed menubar action located | — | M |
| RC:715 Search > Jump Down > 5th Style | `IDM_SEARCH_GONEXTMARKER5`; Cmd:1804 | `ctrl-5` (P:293) | No corresponding installed menubar action located | — | M |
| RC:716 Search > Jump Down > Find Mark Style | `IDM_SEARCH_GONEXTMARKER_DEF`; Cmd:1805 | `ctrl-0` (P:294) | No corresponding installed menubar action located | — | M |
| RC:718 Search > Copy Styled Text | popup | — | See children | — | Layout reference |
| RC:720 Search > Copy Styled Text > 1st Style | `IDM_SEARCH_STYLE1TOCLIP`; Cmd:1860 | None assigned (P:295) | No corresponding installed menubar action located | — | M |
| RC:721 Search > Copy Styled Text > 2nd Style | `IDM_SEARCH_STYLE2TOCLIP`; Cmd:1865 | None assigned (P:296) | No corresponding installed menubar action located | — | M |
| RC:722 Search > Copy Styled Text > 3rd Style | `IDM_SEARCH_STYLE3TOCLIP`; Cmd:1870 | None assigned (P:297) | No corresponding installed menubar action located | — | M |
| RC:723 Search > Copy Styled Text > 4th Style | `IDM_SEARCH_STYLE4TOCLIP`; Cmd:1875 | None assigned (P:298) | No corresponding installed menubar action located | — | M |
| RC:724 Search > Copy Styled Text > 5th Style | `IDM_SEARCH_STYLE5TOCLIP`; Cmd:1880 | None assigned (P:299) | No corresponding installed menubar action located | — | M |
| RC:725 Search > Copy Styled Text > All Styles | `IDM_SEARCH_ALLSTYLESTOCLIP`; Cmd:1885 | None assigned (P:300) | No corresponding installed menubar action located | — | M |
| RC:726 Search > Copy Styled Text > Find Mark Style | `IDM_SEARCH_MARKEDTOCLIP`; Cmd:1890 | None assigned (P:301) | `MarkCopy`; `src/app/menus/search.rs:27`; `src/workspace/find.rs:1988` | **No row** | R/U; no catalog row |
| RC:728 Search > separator | separator | — | — | — | Layout reference |
| RC:729 Search > Bookmark | popup | — | See children | — | Layout reference |
| RC:731 Search > Bookmark > Toggle Bookmark | `IDM_SEARCH_TOGGLE_BOOKMARK`; Cmd:1962 | `ctrl-f2` (P:302) | `BookmarkToggle`; `src/app/menus/bookmarks.rs:11`; `src/workspace/movement.rs:525` | `ctrl-f2` → `cmd-f2`; C:337 | R/U |
| RC:732 Search > Bookmark > Next Bookmark | `IDM_SEARCH_NEXT_BOOKMARK`; Cmd:1966 | `f2` (P:303) | `BookmarkNext`; `src/app/menus/bookmarks.rs:12`; `src/workspace/movement.rs:540` | `f2` → `f2`; C:338 | R/U |
| RC:733 Search > Bookmark > Previous Bookmark | `IDM_SEARCH_PREV_BOOKMARK`; Cmd:1970 | `shift-f2` (P:304) | `BookmarkPrev`; `src/app/menus/bookmarks.rs:13`; `src/workspace/movement.rs:567` | `shift-f2` → `shift-f2`; C:339 | R/U |
| RC:734 Search > Bookmark > Clear All Bookmarks | `IDM_SEARCH_CLEAR_BOOKMARKS`; Cmd:1974 | None assigned (P:305) | `BookmarkClearAll`; `src/app/menus/bookmarks.rs:15`; `src/workspace/movement.rs:587` | none → `cmd-shift-f2`; C:340 | R/U |
| RC:735 Search > Bookmark > Cut Bookmarked Lines | `IDM_SEARCH_CUTMARKEDLINES`; Cmd:2385 | None assigned (P:306) | No corresponding installed menubar action located | — | M |
| RC:736 Search > Bookmark > Copy Bookmarked Lines | `IDM_SEARCH_COPYMARKEDLINES`; Cmd:2389 | None assigned (P:307) | No corresponding installed menubar action located | — | M |
| RC:737 Search > Bookmark > Paste to (Replace) Bookmarked Lines | `IDM_SEARCH_PASTEMARKEDLINES`; Cmd:2393 | None assigned (P:308) | No corresponding installed menubar action located | — | M |
| RC:738 Search > Bookmark > Remove Bookmarked Lines | `IDM_SEARCH_DELETEMARKEDLINES`; Cmd:2397 | None assigned (P:309) | No corresponding installed menubar action located | — | M |
| RC:739 Search > Bookmark > Remove Non-Bookmarked Lines | `IDM_SEARCH_DELETEUNMARKEDLINES`; Cmd:2401 | None assigned (P:310) | No corresponding installed menubar action located | — | M |
| RC:740 Search > Bookmark > Inverse Bookmarks | `IDM_SEARCH_INVERSEMARKS`; Cmd:2405 | None assigned (P:311) | No corresponding installed menubar action located | — | M |
| RC:742 Search > separator | separator | — | — | — | Layout reference |
| RC:743 Search > Find characters in range... | `IDM_SEARCH_FINDCHARINRANGE`; Cmd:1905 | None assigned (P:312) | No corresponding installed menubar action located | — | M |
### View

| Resource node / hierarchy | Oracle ID / dispatch | Oracle default | Studio route | Catalog Windows/Linux → Mac | Assessment |
| --- | --- | --- | --- | --- | --- |
| RC:746 View | popup | — | See children | — | Layout reference |
| RC:748 View > Always on Top | `IDM_VIEW_ALWAYSONTOP`; Cmd:2409 | None assigned (P:314) | No corresponding installed menubar action located | — | M |
| RC:749 View > Toggle Full Screen Mode | `IDM_VIEW_FULLSCREENTOGGLE`; Cmd:2474 | `f11` (P:315) | `ViewToggleFullScreen`; `src/app/menus/view.rs:24`; `src/workspace/edits.rs:1149` | `f11` → `f11`; C:292 | R/U |
| RC:750 View > Post-It | `IDM_VIEW_POSTIT`; Cmd:2481 | `f12` (P:316) | No corresponding installed menubar action located | — | M |
| RC:751 View > Distraction Free Mode | `IDM_VIEW_DISTRACTIONFREE`; Cmd:2488 | None assigned (P:317) | No corresponding installed menubar action located | — | M |
| RC:753 View > separator | separator | — | — | — | Layout reference |
| RC:754 View > View Current File in | popup | — | See children | — | Layout reference |
| RC:756 View > View Current File in > Firefox | `IDM_VIEW_IN_FIREFOX`; Cmd:2496 | None assigned (P:319) | No corresponding installed menubar action located | — | M |
| RC:757 View > View Current File in > Chrome | `IDM_VIEW_IN_CHROME`; Cmd:2497 | None assigned (P:320) | No corresponding installed menubar action located | — | M |
| RC:758 View > View Current File in > Edge | `IDM_VIEW_IN_EDGE`; Cmd:2498 | None assigned (P:322) | No corresponding installed menubar action located | — | M |
| RC:759 View > View Current File in > IE | `IDM_VIEW_IN_IE`; Cmd:2499 | None assigned (P:321) | No corresponding installed menubar action located | — | M |
| RC:761 View > separator | separator | — | — | — | Layout reference |
| RC:762 View > Show Symbol | popup | — | See children | — | Layout reference |
| RC:764 View > Show Symbol > Show Space and Tab | `IDM_VIEW_TAB_SPACE`; Cmd:2565 | None assigned (P:324) | `ViewShowWhitespace`; `src/app/menus/view.rs:20`; `src/workspace/edits.rs:1161` | **No row** | R/U: flat Whitespace item, no Show Symbol submenu; no catalog row |
| RC:765 View > Show Symbol > Show End of Line | `IDM_VIEW_EOL`; Cmd:2584 | None assigned (P:325) | No corresponding installed menubar action located | — | M |
| RC:766 View > Show Symbol > Show Non-Printing Characters | `IDM_VIEW_NPC`; Cmd:2603 | None assigned (P:327) | No corresponding installed menubar action located | — | M |
| RC:767 View > Show Symbol > Show Control Characters & Unicode EOL | `IDM_VIEW_NPC_CCUNIEOL`; Cmd:2625 | None assigned (P:328) | No corresponding installed menubar action located | — | M |
| RC:768 View > Show Symbol > Show All Characters | `IDM_VIEW_ALL_CHARACTERS`; Cmd:2663 | None assigned (P:326) | No corresponding installed menubar action located | — | M |
| RC:769 View > Show Symbol > separator | separator | — | — | — | Layout reference |
| RC:770 View > Show Symbol > Show Indent Guide | `IDM_VIEW_INDENT_GUIDE`; Cmd:2706 | None assigned (P:329) | No corresponding installed menubar action located | — | M |
| RC:771 View > Show Symbol > Show Wrap Symbol | `IDM_VIEW_WRAP_SYMBOL`; Cmd:2744 | None assigned (P:330) | No corresponding installed menubar action located | — | M |
| RC:773 View > Zoom | popup | — | See children | — | Layout reference |
| RC:775 View > Zoom > Zoom In (Ctrl+Mouse Wheel Up) | `IDM_VIEW_ZOOMIN`; Cmd:2761 | `ctrl-numpadadd` (P:558) | `ViewZoomIn`; `src/app/menus/view.rs:14`; `src/workspace/mod.rs:1326` | `ctrl-+`, `ctrl-add` → `cmd-=`, `cmd-+`; C:289 | R/U |
| RC:776 View > Zoom > Zoom Out (Ctrl+Mouse Wheel Down) | `IDM_VIEW_ZOOMOUT`; Cmd:2767 | `ctrl-numpadsubtract` (P:559) | `ViewZoomOut`; `src/app/menus/view.rs:15`; `src/workspace/mod.rs:1335` | `ctrl--`, `ctrl-subtract` → `cmd--`; C:290 | R/U |
| RC:777 View > Zoom > Restore Default Zoom | `IDM_VIEW_ZOOMRESTORE`; Cmd:2772 | `ctrl-numpaddivide` (P:560) | `ViewZoomReset`; `src/app/menus/view.rs:16`; `src/workspace/mod.rs:1344` | `ctrl-/`, `ctrl-divide` → `cmd-0`; C:291 | R/U |
| RC:778 View > Zoom > separator | separator | — | — | — | Layout reference |
| RC:779 View > Zoom > Synchronize Across Views | `IDM_VIEW_ZOOM_SYNC`; Cmd:2778 | Not listed | No corresponding installed menubar action located | — | M |
| RC:781 View > Move/Clone Current Document | popup | — | See children | — | Layout reference |
| RC:783 View > Move/Clone Current Document > Move to Other View | `IDM_VIEW_GOTO_ANOTHER_VIEW`; Cmd:3540 | None assigned (P:336) | No corresponding installed menubar action located | — | M |
| RC:784 View > Move/Clone Current Document > Clone to Other View | `IDM_VIEW_CLONE_TO_ANOTHER_VIEW`; Cmd:3546 | None assigned (P:337) | No corresponding installed menubar action located | — | M |
| RC:785 View > Move/Clone Current Document > Move to New Instance | `IDM_VIEW_GOTO_NEW_INSTANCE`; Cmd:3552 | None assigned (P:338) | No corresponding installed menubar action located | — | M |
| RC:786 View > Move/Clone Current Document > Open in New Instance | `IDM_VIEW_LOAD_IN_NEW_INSTANCE`; Cmd:3556 | None assigned (P:339) | No corresponding installed menubar action located | — | M |
| RC:789 View > Tab | popup | — | See children | — | Layout reference |
| RC:791 View > Tab > 1st Tab | `IDM_VIEW_TAB1`; Cmd:1324 | `ctrl-numpad1` (P:341) | `TabGoto1`; `src/app/menus/view.rs:34`; `src/workspace/tabs.rs:961` | `ctrl-1` → `cmd-1`; C:262 | R/U |
| RC:792 View > Tab > 2nd Tab | `IDM_VIEW_TAB2`; Cmd:1325 | `ctrl-numpad2` (P:342) | `TabGoto2`; `src/app/menus/view.rs:35`; `src/workspace/tabs.rs:964` | `ctrl-2` → `cmd-2`; C:263 | R/U |
| RC:793 View > Tab > 3rd Tab | `IDM_VIEW_TAB3`; Cmd:1326 | `ctrl-numpad3` (P:343) | `TabGoto3`; `src/app/menus/view.rs:36`; `src/workspace/tabs.rs:967` | `ctrl-3` → `cmd-3`; C:264 | R/U |
| RC:794 View > Tab > 4th Tab | `IDM_VIEW_TAB4`; Cmd:1327 | `ctrl-numpad4` (P:344) | `TabGoto4`; `src/app/menus/view.rs:37`; `src/workspace/tabs.rs:970` | `ctrl-4` → `cmd-4`; C:265 | R/U |
| RC:795 View > Tab > 5th Tab | `IDM_VIEW_TAB5`; Cmd:1328 | `ctrl-numpad5` (P:345) | `TabGoto5`; `src/app/menus/view.rs:38`; `src/workspace/tabs.rs:973` | `ctrl-5` → `cmd-5`; C:266 | R/U |
| RC:796 View > Tab > 6th Tab | `IDM_VIEW_TAB6`; Cmd:1329 | `ctrl-numpad6` (P:346) | `TabGoto6`; `src/app/menus/view.rs:39`; `src/workspace/tabs.rs:976` | `ctrl-6` → `cmd-6`; C:267 | R/U |
| RC:797 View > Tab > 7th Tab | `IDM_VIEW_TAB7`; Cmd:1330 | `ctrl-numpad7` (P:347) | `TabGoto7`; `src/app/menus/view.rs:40`; `src/workspace/tabs.rs:979` | `ctrl-7` → `cmd-7`; C:268 | R/U |
| RC:798 View > Tab > 8th Tab | `IDM_VIEW_TAB8`; Cmd:1331 | `ctrl-numpad8` (P:348) | `TabGoto8`; `src/app/menus/view.rs:41`; `src/workspace/tabs.rs:982` | `ctrl-8` → `cmd-8`; C:269 | R/U |
| RC:799 View > Tab > 9th Tab | `IDM_VIEW_TAB9`; Cmd:1332 | `ctrl-numpad9` (P:349) | `TabGoto9`; `src/app/menus/view.rs:42`; `src/workspace/tabs.rs:985` | `ctrl-9` → `cmd-9`; C:270 | R/U |
| RC:800 View > Tab > separator | separator | — | — | — | Layout reference |
| RC:801 View > Tab > First Tab | `IDM_VIEW_TAB_START`; Cmd:1352 | None assigned (P:350) | `TabFirst`; `src/app/menus/view.rs:44`; `src/workspace/tab_actions.rs:11` | none → none; C:272 | R/U |
| RC:802 View > Tab > Last Tab | `IDM_VIEW_TAB_END`; Cmd:1353 | None assigned (P:351) | `TabLast`; `src/app/menus/view.rs:45`; `src/workspace/tab_actions.rs:15` | none → none; C:273 | R/U |
| RC:803 View > Tab > Next Tab | `IDM_VIEW_TAB_NEXT`; Cmd:1360 | `ctrl-pagedown` (P:352) | `TabNextSequential`; `src/app/menus/view.rs:46`; `src/workspace/tabs.rs:934` | `ctrl-pagedown` → `ctrl-pagedown`; C:260 | R/U |
| RC:804 View > Tab > Previous Tab | `IDM_VIEW_TAB_PREV`; Cmd:1375 | `ctrl-pageup` (P:353) | `TabPrevSequential`; `src/app/menus/view.rs:47`; `src/workspace/tabs.rs:943` | `ctrl-pageup` → `ctrl-pageup`; C:261 | R/U |
| RC:805 View > Tab > separator | separator | — | — | — | Layout reference |
| RC:806 View > Tab > Move to Start | `IDM_VIEW_GOTO_START`; Cmd:3532 | None assigned (P:334) | `TabMoveToStart`; `src/app/menus/view.rs:49`; `src/workspace/tab_actions.rs:21` | none → none; C:274 | R/U |
| RC:807 View > Tab > Move to End | `IDM_VIEW_GOTO_END`; Cmd:3536 | None assigned (P:335) | `TabMoveToEnd`; `src/app/menus/view.rs:50`; `src/workspace/tab_actions.rs:25` | none → none; C:275 | R/U |
| RC:808 View > Tab > Move Tab Forward | `IDM_VIEW_TAB_MOVEFORWARD`; Cmd:1390 | `ctrl-shift-pagedown` (P:354) | `TabMoveForward`; `src/app/menus/view.rs:51`; `src/workspace/tab_actions.rs:29` | `ctrl-shift-pagedown` → `cmd-shift-pagedown`; C:276 | R/U |
| RC:809 View > Tab > Move Tab Backward | `IDM_VIEW_TAB_MOVEBACKWARD`; Cmd:1391 | `ctrl-shift-pageup` (P:355) | `TabMoveBackward`; `src/app/menus/view.rs:52`; `src/workspace/tab_actions.rs:34` | `ctrl-shift-pageup` → `cmd-shift-pageup`; C:277 | R/U |
| RC:810 View > Tab > separator | separator | — | — | — | Layout reference |
| RC:811 View > Tab > Apply Color 1 | `IDM_VIEW_TAB_COLOUR_1`; Cmd:1305 | None assigned (P:398) | `TabColor1`; `src/app/menus/view.rs:54`; `src/workspace/tab_actions.rs:40` | none → none; C:278 | R/U |
| RC:812 View > Tab > Apply Color 2 | `IDM_VIEW_TAB_COLOUR_2`; Cmd:1306 | None assigned (P:399) | `TabColor2`; `src/app/menus/view.rs:55`; `src/workspace/tab_actions.rs:44` | none → none; C:279 | R/U |
| RC:813 View > Tab > Apply Color 3 | `IDM_VIEW_TAB_COLOUR_3`; Cmd:1307 | None assigned (P:400) | `TabColor3`; `src/app/menus/view.rs:56`; `src/workspace/tab_actions.rs:48` | none → none; C:280 | R/U |
| RC:814 View > Tab > Apply Color 4 | `IDM_VIEW_TAB_COLOUR_4`; Cmd:1308 | None assigned (P:401) | `TabColor4`; `src/app/menus/view.rs:57`; `src/workspace/tab_actions.rs:52` | none → none; C:281 | R/U |
| RC:815 View > Tab > Apply Color 5 | `IDM_VIEW_TAB_COLOUR_5`; Cmd:1309 | None assigned (P:402) | `TabColor5`; `src/app/menus/view.rs:58`; `src/workspace/tab_actions.rs:56` | none → none; C:282 | R/U |
| RC:816 View > Tab > Remove Color | `IDM_VIEW_TAB_COLOUR_NONE`; Cmd:1304 | None assigned (P:397) | `TabClearColor`; `src/app/menus/view.rs:59`; `src/workspace/tab_actions.rs:60` | none → none; C:283 | R/U |
| RC:818 View > Word wrap | `IDM_VIEW_WRAP`; Cmd:2718 | None assigned (P:358) | `ViewWordWrap`; `src/app/menus/view.rs:19`; `src/workspace/edits.rs:3171` | **No row** | R/U; no catalog row |
| RC:819 View > Focus on Another View | `IDM_VIEW_SWITCHTO_OTHER_VIEW`; Cmd:3560 | `f8` (P:360) | No corresponding installed menubar action located | — | M |
| RC:820 View > Hide Lines | `IDM_VIEW_HIDELINES`; Cmd:2755 | `alt-h` (P:359) | No corresponding installed menubar action located | — | M |
| RC:821 View > separator | separator | — | — | — | Layout reference |
| RC:822 View > Fold All | `IDM_VIEW_FOLDALL`; Cmd:2434 | `alt-0` (P:362) | No corresponding installed menubar action located | — | M |
| RC:823 View > Unfold All | `IDM_VIEW_UNFOLDALL`; Cmd:2435 | `alt-shift-0` (P:363) | No corresponding installed menubar action located | — | M |
| RC:824 View > Fold Current Level | `IDM_VIEW_FOLD_CURRENT`; Cmd:2418 | `ctrl-alt-f` (P:364) | No corresponding installed menubar action located | — | M |
| RC:825 View > Unfold Current Level | `IDM_VIEW_UNFOLD_CURRENT`; Cmd:2419 | `ctrl-alt-shift-f` (P:365) | No corresponding installed menubar action located | — | M |
| RC:826 View > Fold Level | popup | — | See children | — | Layout reference |
| RC:828 View > Fold Level > 1 | `IDM_VIEW_FOLD_1`; Cmd:2448 | `alt-1` (P:366) | No corresponding installed menubar action located | — | M |
| RC:829 View > Fold Level > 2 | `IDM_VIEW_FOLD_2`; Cmd:2449 | `alt-2` (P:367) | No corresponding installed menubar action located | — | M |
| RC:830 View > Fold Level > 3 | `IDM_VIEW_FOLD_3`; Cmd:2450 | `alt-3` (P:368) | No corresponding installed menubar action located | — | M |
| RC:831 View > Fold Level > 4 | `IDM_VIEW_FOLD_4`; Cmd:2451 | `alt-4` (P:369) | No corresponding installed menubar action located | — | M |
| RC:832 View > Fold Level > 5 | `IDM_VIEW_FOLD_5`; Cmd:2452 | `alt-5` (P:370) | No corresponding installed menubar action located | — | M |
| RC:833 View > Fold Level > 6 | `IDM_VIEW_FOLD_6`; Cmd:2453 | `alt-6` (P:371) | No corresponding installed menubar action located | — | M |
| RC:834 View > Fold Level > 7 | `IDM_VIEW_FOLD_7`; Cmd:2454 | `alt-7` (P:372) | No corresponding installed menubar action located | — | M |
| RC:835 View > Fold Level > 8 | `IDM_VIEW_FOLD_8`; Cmd:2455 | `alt-8` (P:373) | No corresponding installed menubar action located | — | M |
| RC:837 View > Unfold Level | popup | — | See children | — | Layout reference |
| RC:839 View > Unfold Level > 1 | `IDM_VIEW_UNFOLD_1`; Cmd:2461 | `alt-shift-1` (P:375) | No corresponding installed menubar action located | — | M |
| RC:840 View > Unfold Level > 2 | `IDM_VIEW_UNFOLD_2`; Cmd:2462 | `alt-shift-2` (P:376) | No corresponding installed menubar action located | — | M |
| RC:841 View > Unfold Level > 3 | `IDM_VIEW_UNFOLD_3`; Cmd:2463 | `alt-shift-3` (P:377) | No corresponding installed menubar action located | — | M |
| RC:842 View > Unfold Level > 4 | `IDM_VIEW_UNFOLD_4`; Cmd:2464 | `alt-shift-4` (P:378) | No corresponding installed menubar action located | — | M |
| RC:843 View > Unfold Level > 5 | `IDM_VIEW_UNFOLD_5`; Cmd:2465 | `alt-shift-5` (P:379) | No corresponding installed menubar action located | — | M |
| RC:844 View > Unfold Level > 6 | `IDM_VIEW_UNFOLD_6`; Cmd:2466 | `alt-shift-6` (P:380) | No corresponding installed menubar action located | — | M |
| RC:845 View > Unfold Level > 7 | `IDM_VIEW_UNFOLD_7`; Cmd:2467 | `alt-shift-7` (P:381) | No corresponding installed menubar action located | — | M |
| RC:846 View > Unfold Level > 8 | `IDM_VIEW_UNFOLD_8`; Cmd:2468 | `alt-shift-8` (P:382) | No corresponding installed menubar action located | — | M |
| RC:849 View > separator | separator | — | — | — | Layout reference |
| RC:850 View > Summary... | `IDM_VIEW_SUMMARY`; Cmd:2828 | None assigned (P:383) | No corresponding installed menubar action located | — | M |
| RC:851 View > separator | separator | — | — | — | Layout reference |
| RC:852 View > Project Panels | popup | — | See children | — | Layout reference |
| RC:854 View > Project Panels > Project Panel 1 | `IDM_VIEW_PROJECT_PANEL_1`; Cmd:1165 | None assigned (P:384) | No corresponding installed menubar action located | — | M |
| RC:855 View > Project Panels > Project Panel 2 | `IDM_VIEW_PROJECT_PANEL_2`; Cmd:1166 | None assigned (P:385) | No corresponding installed menubar action located | — | M |
| RC:856 View > Project Panels > Project Panel 3 | `IDM_VIEW_PROJECT_PANEL_3`; Cmd:1167 | None assigned (P:386) | No corresponding installed menubar action located | — | M |
| RC:858 View > Folder as Workspace | `IDM_VIEW_FILEBROWSER`; Cmd:1208 | None assigned (P:387) | `ViewToggleFolderPanel`; `src/app/menus/view.rs:22`; `src/workspace/edits.rs:3160` | **No row** | R/U; no catalog row |
| RC:859 View > Document Map | `IDM_VIEW_DOC_MAP`; Cmd:1246 | None assigned (P:388) | No corresponding installed menubar action located | — | M |
| RC:860 View > Document List | `IDM_VIEW_DOCLIST`; Cmd:1143 | None assigned (P:389) | No corresponding installed menubar action located | — | M |
| RC:861 View > Function List | `IDM_VIEW_FUNC_LIST`; Cmd:1285 | None assigned (P:390) | No corresponding installed menubar action located | — | M |
| RC:862 View > separator | separator | — | — | — | Layout reference |
| RC:863 View > Synchronize Vertical Scrolling | `IDM_VIEW_SYNSCROLLV`; Cmd:2789 | None assigned (P:403) | No corresponding installed menubar action located | — | M |
| RC:864 View > Synchronize Horizontal Scrolling | `IDM_VIEW_SYNSCROLLH`; Cmd:2807 | None assigned (P:404) | No corresponding installed menubar action located | — | M |
| RC:865 View > separator | separator | — | — | — | Layout reference |
| RC:866 View > Text Direction RTL | `IDM_EDIT_RTL`; Cmd:4088 | `ctrl-alt-r` (P:405) | No corresponding installed menubar action located | — | M |
| RC:867 View > Text Direction LTR | `IDM_EDIT_LTR`; Cmd:4089 | `ctrl-alt-l` (P:406) | No corresponding installed menubar action located | — | M |
| RC:868 View > separator | separator | — | — | — | Layout reference |
| RC:869 View > Monitoring (tail -f) | `IDM_VIEW_MONITORING`; Cmd:2905 | None assigned (P:407) | No corresponding installed menubar action located | — | M |
### Encoding

| Resource node / hierarchy | Oracle ID / dispatch | Oracle default | Studio route | Catalog Windows/Linux → Mac | Assessment |
| --- | --- | --- | --- | --- | --- |
| RC:872 Encoding | popup | — | See children | — | Layout reference |
| RC:874 Encoding > ANSI | `IDM_FORMAT_ANSI`; Cmd:2991 | None assigned (P:409) | `EncodingLatin1`; `src/app/menus/encoding.rs:16`; `src/workspace/format.rs:121` | **No row** | D: CP1252 save target, not OS ANSI interpretation; no catalog row |
| RC:875 Encoding > UTF-8 | `IDM_FORMAT_AS_UTF_8`; Cmd:2995 | None assigned (P:410) | `EncodingUtf8`; `src/app/menus/encoding.rs:12`; `src/workspace/format.rs:85` | **No row** | D/U: save target only; omits oracle legacy-encoding reload path; no catalog row |
| RC:876 Encoding > UTF-8-BOM | `IDM_FORMAT_UTF_8`; Cmd:2992 | None assigned (P:411) | `EncodingUtf8Bom`; `src/app/menus/encoding.rs:13`; `src/workspace/format.rs:94` | **No row** | D/U: save target only; omits oracle legacy-encoding reload path; no catalog row |
| RC:877 Encoding > UTF-16 BE BOM | `IDM_FORMAT_UTF_16BE`; Cmd:2993 | None assigned (P:412) | `EncodingUtf16Be`; `src/app/menus/encoding.rs:15`; `src/workspace/format.rs:112` | **No row** | D/U: save target only; omits oracle legacy-encoding reload path; no catalog row |
| RC:878 Encoding > UTF-16 LE BOM | `IDM_FORMAT_UTF_16LE`; Cmd:2994 | None assigned (P:413) | `EncodingUtf16Le`; `src/app/menus/encoding.rs:14`; `src/workspace/format.rs:103` | **No row** | D/U: save target only; omits oracle legacy-encoding reload path; no catalog row |
| RC:879 Encoding > Character sets | popup | — | See children | — | Layout reference |
| RC:881 Encoding > Character sets > Arabic | popup | — | See children | — | Layout reference |
| RC:883 Encoding > Character sets > Arabic > ISO 8859-6 | `IDM_FORMAT_ISO_8859_6`; Cmd:3114 | None assigned (P:415) | No corresponding installed menubar action located | — | M |
| RC:884 Encoding > Character sets > Arabic > OEM 720 | `IDM_FORMAT_DOS_720`; Cmd:3122 | None assigned (P:427) | No corresponding installed menubar action located | — | M |
| RC:885 Encoding > Character sets > Arabic > Windows-1256 | `IDM_FORMAT_WIN_1256`; Cmd:3106 | None assigned (P:416) | No corresponding installed menubar action located | — | M |
| RC:888 Encoding > Character sets > Baltic | popup | — | See children | — | Layout reference |
| RC:890 Encoding > Character sets > Baltic > ISO 8859-4 | `IDM_FORMAT_ISO_8859_4`; Cmd:3112 | None assigned (P:453) | No corresponding installed menubar action located | — | M |
| RC:891 Encoding > Character sets > Baltic > ISO 8859-13 | `IDM_FORMAT_ISO_8859_13`; Cmd:3118 | None assigned (P:417) | No corresponding installed menubar action located | — | M |
| RC:892 Encoding > Character sets > Baltic > OEM 775 | `IDM_FORMAT_DOS_775`; Cmd:3124 | None assigned (P:429) | No corresponding installed menubar action located | — | M |
| RC:893 Encoding > Character sets > Baltic > Windows-1257 | `IDM_FORMAT_WIN_1257`; Cmd:3107 | None assigned (P:418) | No corresponding installed menubar action located | — | M |
| RC:895 Encoding > Character sets > Celtic | popup | — | See children | — | Layout reference |
| RC:897 Encoding > Character sets > Celtic > ISO 8859-14 | `IDM_FORMAT_ISO_8859_14`; Cmd:3119 | None assigned (P:419) | No corresponding installed menubar action located | — | M |
| RC:899 Encoding > Character sets > Cyrillic | popup | — | See children | — | Layout reference |
| RC:901 Encoding > Character sets > Cyrillic > ISO 8859-5 | `IDM_FORMAT_ISO_8859_5`; Cmd:3113 | None assigned (P:420) | No corresponding installed menubar action located | — | M |
| RC:902 Encoding > Character sets > Cyrillic > KOI8-R | `IDM_FORMAT_KOI8R_CYRILLIC`; Cmd:3145 | None assigned (P:422) | No corresponding installed menubar action located | — | M |
| RC:903 Encoding > Character sets > Cyrillic > KOI8-U | `IDM_FORMAT_KOI8U_CYRILLIC`; Cmd:3144 | None assigned (P:423) | No corresponding installed menubar action located | — | M |
| RC:904 Encoding > Character sets > Cyrillic > Macintosh | `IDM_FORMAT_MAC_CYRILLIC`; Cmd:3143 | None assigned (P:421) | No corresponding installed menubar action located | — | M |
| RC:905 Encoding > Character sets > Cyrillic > OEM 855 | `IDM_FORMAT_DOS_855`; Cmd:3127 | None assigned (P:432) | No corresponding installed menubar action located | — | M |
| RC:906 Encoding > Character sets > Cyrillic > OEM 866 | `IDM_FORMAT_DOS_866`; Cmd:3135 | None assigned (P:440) | No corresponding installed menubar action located | — | M |
| RC:907 Encoding > Character sets > Cyrillic > Windows-1251 | `IDM_FORMAT_WIN_1251`; Cmd:3101 | None assigned (P:424) | No corresponding installed menubar action located | — | M |
| RC:910 Encoding > Character sets > Central European | popup | — | See children | — | Layout reference |
| RC:912 Encoding > Character sets > Central European > OEM 852 | `IDM_FORMAT_DOS_852`; Cmd:3126 | None assigned (P:431) | No corresponding installed menubar action located | — | M |
| RC:913 Encoding > Character sets > Central European > Windows-1250 | `IDM_FORMAT_WIN_1250`; Cmd:3100 | None assigned (P:425) | No corresponding installed menubar action located | — | M |
| RC:916 Encoding > Character sets > Chinese | popup | — | See children | — | Layout reference |
| RC:918 Encoding > Character sets > Chinese > Big5 (Traditional) | `IDM_FORMAT_BIG5`; Cmd:3137 | None assigned (P:442) | No corresponding installed menubar action located | — | M |
| RC:919 Encoding > Character sets > Chinese > GB2312 (Simplified) | `IDM_FORMAT_GB2312`; Cmd:3138 | None assigned (P:443) | No corresponding installed menubar action located | — | M |
| RC:922 Encoding > Character sets > Eastern European | popup | — | See children | — | Layout reference |
| RC:924 Encoding > Character sets > Eastern European > ISO 8859-2 | `IDM_FORMAT_ISO_8859_2`; Cmd:3110 | None assigned (P:444) | No corresponding installed menubar action located | — | M |
| RC:927 Encoding > Character sets > Greek | popup | — | See children | — | Layout reference |
| RC:929 Encoding > Character sets > Greek > ISO 8859-7 | `IDM_FORMAT_ISO_8859_7`; Cmd:3115 | None assigned (P:445) | No corresponding installed menubar action located | — | M |
| RC:930 Encoding > Character sets > Greek > OEM 737 | `IDM_FORMAT_DOS_737`; Cmd:3123 | None assigned (P:428) | No corresponding installed menubar action located | — | M |
| RC:931 Encoding > Character sets > Greek > OEM 869 | `IDM_FORMAT_DOS_869`; Cmd:3136 | None assigned (P:441) | No corresponding installed menubar action located | — | M |
| RC:932 Encoding > Character sets > Greek > Windows-1253 | `IDM_FORMAT_WIN_1253`; Cmd:3103 | None assigned (P:446) | No corresponding installed menubar action located | — | M |
| RC:935 Encoding > Character sets > Hebrew | popup | — | See children | — | Layout reference |
| RC:937 Encoding > Character sets > Hebrew > ISO 8859-8 | `IDM_FORMAT_ISO_8859_8`; Cmd:3116 | None assigned (P:447) | No corresponding installed menubar action located | — | M |
| RC:938 Encoding > Character sets > Hebrew > OEM 862 | `IDM_FORMAT_DOS_862`; Cmd:3132 | None assigned (P:437) | No corresponding installed menubar action located | — | M |
| RC:939 Encoding > Character sets > Hebrew > Windows-1255 | `IDM_FORMAT_WIN_1255`; Cmd:3105 | None assigned (P:448) | No corresponding installed menubar action located | — | M |
| RC:942 Encoding > Character sets > Japanese | popup | — | See children | — | Layout reference |
| RC:944 Encoding > Character sets > Japanese > Shift-JIS | `IDM_FORMAT_SHIFT_JIS`; Cmd:3139 | None assigned (P:449) | No corresponding installed menubar action located | — | M |
| RC:947 Encoding > Character sets > Korean | popup | — | See children | — | Layout reference |
| RC:949 Encoding > Character sets > Korean > Windows 949 | `IDM_FORMAT_KOREAN_WIN`; Cmd:3140 | Not listed | No corresponding installed menubar action located | — | M |
| RC:950 Encoding > Character sets > Korean > EUC-KR | `IDM_FORMAT_EUC_KR`; Cmd:3141 | None assigned (P:450) | No corresponding installed menubar action located | — | M |
| RC:953 Encoding > Character sets > North European | popup | — | See children | — | Layout reference |
| RC:955 Encoding > Character sets > North European > OEM 861 : Icelandic | `IDM_FORMAT_DOS_861`; Cmd:3131 | None assigned (P:436) | No corresponding installed menubar action located | — | M |
| RC:956 Encoding > Character sets > North European > OEM 865 : Nordic | `IDM_FORMAT_DOS_865`; Cmd:3134 | None assigned (P:439) | No corresponding installed menubar action located | — | M |
| RC:959 Encoding > Character sets > Thai | popup | — | See children | — | Layout reference |
| RC:961 Encoding > Character sets > Thai > TIS-620 | `IDM_FORMAT_TIS_620`; Cmd:3142 | None assigned (P:457) | No corresponding installed menubar action located | — | M |
| RC:964 Encoding > Character sets > Turkish | popup | — | See children | — | Layout reference |
| RC:966 Encoding > Character sets > Turkish > ISO 8859-3 | `IDM_FORMAT_ISO_8859_3`; Cmd:3111 | None assigned (P:455) | No corresponding installed menubar action located | — | M |
| RC:967 Encoding > Character sets > Turkish > ISO 8859-9 | `IDM_FORMAT_ISO_8859_9`; Cmd:3117 | None assigned (P:458) | No corresponding installed menubar action located | — | M |
| RC:968 Encoding > Character sets > Turkish > OEM 857 | `IDM_FORMAT_DOS_857`; Cmd:3128 | None assigned (P:433) | No corresponding installed menubar action located | — | M |
| RC:969 Encoding > Character sets > Turkish > Windows-1254 | `IDM_FORMAT_WIN_1254`; Cmd:3104 | None assigned (P:459) | No corresponding installed menubar action located | — | M |
| RC:972 Encoding > Character sets > Western European | popup | — | See children | — | Layout reference |
| RC:974 Encoding > Character sets > Western European > ISO 8859-1 | `IDM_FORMAT_ISO_8859_1`; Cmd:3109 | None assigned (P:461) | No corresponding installed menubar action located | — | M |
| RC:976 Encoding > Character sets > Western European > ISO 8859-15 | `IDM_FORMAT_ISO_8859_15`; Cmd:3120 | None assigned (P:452) | No corresponding installed menubar action located | — | M |
| RC:977 Encoding > Character sets > Western European > OEM 850 | `IDM_FORMAT_DOS_850`; Cmd:3125 | None assigned (P:430) | No corresponding installed menubar action located | — | M |
| RC:978 Encoding > Character sets > Western European > OEM 858 | `IDM_FORMAT_DOS_858`; Cmd:3129 | None assigned (P:434) | No corresponding installed menubar action located | — | M |
| RC:979 Encoding > Character sets > Western European > OEM 860 : Portuguese | `IDM_FORMAT_DOS_860`; Cmd:3130 | None assigned (P:435) | No corresponding installed menubar action located | — | M |
| RC:980 Encoding > Character sets > Western European > OEM 863 : French | `IDM_FORMAT_DOS_863`; Cmd:3133 | None assigned (P:438) | No corresponding installed menubar action located | — | M |
| RC:981 Encoding > Character sets > Western European > OEM-US : CP437 | `IDM_FORMAT_DOS_437`; Cmd:3121 | None assigned (P:426) | No corresponding installed menubar action located | — | M |
| RC:982 Encoding > Character sets > Western European > Windows-1252 | `IDM_FORMAT_WIN_1252`; Cmd:3102 | None assigned (P:460) | No corresponding installed menubar action located | — | M |
| RC:985 Encoding > Character sets > Vietnamese | popup | — | See children | — | Layout reference |
| RC:987 Encoding > Character sets > Vietnamese > Windows-1258 | `IDM_FORMAT_WIN_1258`; Cmd:3108 | None assigned (P:462) | No corresponding installed menubar action located | — | M |
| RC:990 Encoding > separator | separator | — | — | — | Layout reference |
| RC:991 Encoding > Convert to ANSI | `IDM_FORMAT_CONV2_ANSI`; Cmd:3200 | None assigned (P:463) | No corresponding installed menubar action located | — | M |
| RC:992 Encoding > Convert to UTF-8 | `IDM_FORMAT_CONV2_AS_UTF_8`; Cmd:3201 | None assigned (P:464) | No corresponding installed menubar action located | — | M |
| RC:993 Encoding > Convert to UTF-8-BOM | `IDM_FORMAT_CONV2_UTF_8`; Cmd:3202 | None assigned (P:465) | No corresponding installed menubar action located | — | M |
| RC:994 Encoding > Convert to UTF-16 BE BOM | `IDM_FORMAT_CONV2_UTF_16BE`; Cmd:3203 | None assigned (P:466) | No corresponding installed menubar action located | — | M |
| RC:995 Encoding > Convert to UTF-16 LE BOM | `IDM_FORMAT_CONV2_UTF_16LE`; Cmd:3204 | None assigned (P:467) | No corresponding installed menubar action located | — | M |
### Language [flat]

| Resource node / hierarchy | Oracle ID / dispatch | Oracle default | Studio route | Catalog Windows/Linux → Mac | Assessment |
| --- | --- | --- | --- | --- | --- |
| RC:998 Language [flat] | popup | — | See children | — | Layout reference |
| RC:1000 Language [flat] > None (Normal Text) | `IDM_LANG_TEXT`; Cmd:3933 | Not listed | `LanguageNormalText`; `src/app/menus/language.rs:15`; `src/workspace/language.rs:48` | none → none; C:298 | R/U |
| RC:1001 Language [flat] > separator | separator | — | — | — | Layout reference |
| RC:1002 Language [flat] > ActionScript | `IDM_LANG_FLASH`; Cmd:3941 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1003 Language [flat] > Ada | `IDM_LANG_ADA`; Cmd:3955 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1004 Language [flat] > ASN.1 | `IDM_LANG_ASN1`; Cmd:3975 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1005 Language [flat] > ASP | `IDM_LANG_ASP`; Cmd:3921 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1006 Language [flat] > Assembly | `IDM_LANG_ASM`; Cmd:3946 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1007 Language [flat] > AutoIt | `IDM_LANG_AU3`; Cmd:3959 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1008 Language [flat] > AviSynth | `IDM_LANG_AVS`; Cmd:3976 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1009 Language [flat] > BaanC | `IDM_LANG_BAANC`; Cmd:3970 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1010 Language [flat] > Batch | `IDM_LANG_BATCH`; Cmd:3927 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1011 Language [flat] > Blitzbasic | `IDM_LANG_BLITZBASIC`; Cmd:3977 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1012 Language [flat] > C | `IDM_LANG_C`; Cmd:3911 | Not listed | `LanguageC`; `src/app/menus/language.rs:17`; `src/workspace/language.rs:49` | none → none; C:299 | R/U |
| RC:1013 Language [flat] > C# | `IDM_LANG_CS`; Cmd:3914 | Not listed | `LanguageCSharp`; `src/app/menus/language.rs:18`; `src/workspace/language.rs:50` | none → none; C:300 | R/U |
| RC:1014 Language [flat] > C++ | `IDM_LANG_CPP`; Cmd:3912 | Not listed | `LanguageCpp`; `src/app/menus/language.rs:19`; `src/workspace/language.rs:51` | none → none; C:301 | R/U |
| RC:1015 Language [flat] > Caml | `IDM_LANG_CAML`; Cmd:3954 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1016 Language [flat] > CMake | `IDM_LANG_CMAKE`; Cmd:3961 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1017 Language [flat] > COBOL | `IDM_LANG_COBOL`; Cmd:3963 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1018 Language [flat] > CSound | `IDM_LANG_CSOUND`; Cmd:3980 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1019 Language [flat] > CoffeeScript | `IDM_LANG_COFFEESCRIPT`; Cmd:3969 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1020 Language [flat] > CSS | `IDM_LANG_CSS`; Cmd:3922 | Not listed | `LanguageCss`; `src/app/menus/language.rs:20`; `src/workspace/language.rs:52` | none → none; C:302 | R/U |
| RC:1021 Language [flat] > D | `IDM_LANG_D`; Cmd:3964 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1022 Language [flat] > Diff | `IDM_LANG_DIFF`; Cmd:3947 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1023 Language [flat] > Erlang | `IDM_LANG_ERLANG`; Cmd:3981 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1024 Language [flat] > ErrorList | `IDM_LANG_ERRORLIST`; Cmd:4002 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1025 Language [flat] > Escape Sequence (ANSI) | `IDM_LANG_ESCSEQ`; Cmd:4003 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1026 Language [flat] > ESCRIPT | `IDM_LANG_ESCRIPT`; Cmd:3982 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1027 Language [flat] > Forth | `IDM_LANG_FORTH`; Cmd:3983 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1028 Language [flat] > Fortran (free form) | `IDM_LANG_FORTRAN`; Cmd:3938 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1029 Language [flat] > Fortran (fixed form) | `IDM_LANG_FORTRAN_77`; Cmd:3939 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1030 Language [flat] > Freebasic | `IDM_LANG_FREEBASIC`; Cmd:3979 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1031 Language [flat] > GDScript | `IDM_LANG_GDSCRIPT`; Cmd:3996 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1032 Language [flat] > Go | `IDM_LANG_GOLANG`; Cmd:3998 | Not listed | `LanguageGo`; `src/app/menus/language.rs:21`; `src/workspace/language.rs:53` | none → none; C:303 | R/U |
| RC:1033 Language [flat] > Gui4Cli | `IDM_LANG_GUI4CLI`; Cmd:3965 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1034 Language [flat] > Haskell | `IDM_LANG_HASKELL`; Cmd:3958 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1035 Language [flat] > Hollywood | `IDM_LANG_HOLLYWOOD`; Cmd:3997 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1036 Language [flat] > HTML | `IDM_LANG_HTML`; Cmd:3915 | Not listed | `LanguageHtml`; `src/app/menus/language.rs:22`; `src/workspace/language.rs:54` | none → none; C:304 | R/U |
| RC:1037 Language [flat] > INI file | `IDM_LANG_INI`; Cmd:3936 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1038 Language [flat] > Inno Setup | `IDM_LANG_INNO`; Cmd:3960 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1039 Language [flat] > Intel HEX | `IDM_LANG_IHEX`; Cmd:3972 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1040 Language [flat] > Java | `IDM_LANG_JAVA`; Cmd:3913 | Not listed | `LanguageJava`; `src/app/menus/language.rs:23`; `src/workspace/language.rs:55` | none → none; C:305 | R/U |
| RC:1041 Language [flat] > JavaScript | `IDM_LANG_JS`; Cmd:3917 | Not listed | `LanguageJavaScript`; `src/app/menus/language.rs:24`; `src/workspace/language.rs:56` | none → none; C:306 | R/U |
| RC:1042 Language [flat] > JSON | `IDM_LANG_JSON`; Cmd:3918 | Not listed | `LanguageJson`; `src/app/menus/language.rs:25`; `src/workspace/language.rs:57` | none → none; C:307 | R/U |
| RC:1043 Language [flat] > JSON5 | `IDM_LANG_JSON5`; Cmd:3919 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1044 Language [flat] > JSP | `IDM_LANG_JSP`; Cmd:3968 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1045 Language [flat] > KIXtart | `IDM_LANG_KIX`; Cmd:3953 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1046 Language [flat] > LISP | `IDM_LANG_LISP`; Cmd:3944 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1047 Language [flat] > LaTeX | `IDM_LANG_LATEX`; Cmd:3984 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1048 Language [flat] > Lua | `IDM_LANG_LUA`; Cmd:3923 | Not listed | `LanguageLua`; `src/app/menus/language.rs:26`; `src/workspace/language.rs:58` | none → none; C:308 | R/U |
| RC:1049 Language [flat] > Makefile | `IDM_LANG_MAKEFILE`; Cmd:3935 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1050 Language [flat] > Matlab | `IDM_LANG_MATLAB`; Cmd:3957 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1051 Language [flat] > Microsoft Transact-SQL | `IDM_LANG_MSSQL`; Cmd:3931 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1052 Language [flat] > MMIXAL | `IDM_LANG_MMIXAL`; Cmd:3985 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1053 Language [flat] > MS-DOS Style | `IDM_LANG_ASCII`; Cmd:3932 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1054 Language [flat] > Nim | `IDM_LANG_NIM`; Cmd:3986 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1055 Language [flat] > Nncrontab | `IDM_LANG_NNCRONTAB`; Cmd:3987 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1056 Language [flat] > NSIS | `IDM_LANG_NSIS`; Cmd:3942 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1057 Language [flat] > Objective-C | `IDM_LANG_OBJC`; Cmd:3928 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1058 Language [flat] > OScript | `IDM_LANG_OSCRIPT`; Cmd:3988 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1059 Language [flat] > Pascal | `IDM_LANG_PASCAL`; Cmd:3926 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1060 Language [flat] > Perl | `IDM_LANG_PERL`; Cmd:3924 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1061 Language [flat] > PHP | `IDM_LANG_PHP`; Cmd:3920 | Not listed | `LanguagePhp`; `src/app/menus/language.rs:28`; `src/workspace/language.rs:60` | none → none; C:310 | R/U |
| RC:1062 Language [flat] > PostScript | `IDM_LANG_PS`; Cmd:3949 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1063 Language [flat] > PowerShell | `IDM_LANG_POWERSHELL`; Cmd:3966 | Not listed | `LanguagePowerShell`; `src/app/menus/language.rs:29`; `src/workspace/language.rs:61` | none → none; C:311 | R/U |
| RC:1064 Language [flat] > Properties | `IDM_LANG_PROPS`; Cmd:3948 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1065 Language [flat] > Purebasic | `IDM_LANG_PUREBASIC`; Cmd:3978 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1066 Language [flat] > Python | `IDM_LANG_PYTHON`; Cmd:3925 | Not listed | `LanguagePython`; `src/app/menus/language.rs:30`; `src/workspace/language.rs:62` | none → none; C:312 | R/U |
| RC:1067 Language [flat] > R | `IDM_LANG_R`; Cmd:3967 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1068 Language [flat] > Raku | `IDM_LANG_RAKU`; Cmd:3999 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1069 Language [flat] > REBOL | `IDM_LANG_REBOL`; Cmd:3989 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1070 Language [flat] > Registry | `IDM_LANG_REGISTRY`; Cmd:3990 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1071 Language [flat] > Resource file | `IDM_LANG_RC`; Cmd:3934 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1072 Language [flat] > Ruby | `IDM_LANG_RUBY`; Cmd:3950 | Not listed | `LanguageRuby`; `src/app/menus/language.rs:31`; `src/workspace/language.rs:63` | none → none; C:313 | R/U |
| RC:1073 Language [flat] > Rust | `IDM_LANG_RUST`; Cmd:3991 | Not listed | `LanguageRust`; `src/app/menus/language.rs:32`; `src/workspace/language.rs:64` | none → none; C:314 | R/U |
| RC:1074 Language [flat] > SAS | `IDM_LANG_SAS`; Cmd:4001 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1075 Language [flat] > Shell | `IDM_LANG_BASH`; Cmd:3940 | Not listed | `LanguageShell`; `src/app/menus/language.rs:34`; `src/workspace/language.rs:66` | none → none; C:316 | R/U |
| RC:1076 Language [flat] > Scheme | `IDM_LANG_SCHEME`; Cmd:3945 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1077 Language [flat] > Smalltalk | `IDM_LANG_SMALLTALK`; Cmd:3951 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1078 Language [flat] > Spice | `IDM_LANG_SPICE`; Cmd:3992 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1079 Language [flat] > SQL | `IDM_LANG_SQL`; Cmd:3930 | Not listed | `LanguageSql`; `src/app/menus/language.rs:35`; `src/workspace/language.rs:67` | none → none; C:317 | R/U |
| RC:1080 Language [flat] > Structured Text | `IDM_LANG_STTXT`; Cmd:4004 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1081 Language [flat] > Swift | `IDM_LANG_SWIFT`; Cmd:3974 | Not listed | `LanguageSwift`; `src/app/menus/language.rs:36`; `src/workspace/language.rs:68` | none → none; C:318 | R/U |
| RC:1082 Language [flat] > S-Record | `IDM_LANG_SREC`; Cmd:3971 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1083 Language [flat] > TCL | `IDM_LANG_TCL`; Cmd:3943 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1084 Language [flat] > Tektronix extended HEX | `IDM_LANG_TEHEX`; Cmd:3973 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1085 Language [flat] > TeX | `IDM_LANG_TEX`; Cmd:3937 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1086 Language [flat] > TOML | `IDM_LANG_TOML`; Cmd:4000 | Not listed | `LanguageToml`; `src/app/menus/language.rs:37`; `src/workspace/language.rs:69` | none → none; C:319 | R/U |
| RC:1087 Language [flat] > txt2tags | `IDM_LANG_TXT2TAGS`; Cmd:3993 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1088 Language [flat] > TypeScript | `IDM_LANG_TYPESCRIPT`; Cmd:3995 | Not listed | `LanguageTypeScript`; `src/app/menus/language.rs:39`; `src/workspace/language.rs:71` | none → none; C:321 | R/U |
| RC:1089 Language [flat] > Verilog | `IDM_LANG_VERILOG`; Cmd:3956 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1090 Language [flat] > VHDL | `IDM_LANG_VHDL`; Cmd:3952 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1091 Language [flat] > Visual Basic | `IDM_LANG_VB`; Cmd:3929 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1092 Language [flat] > Visual Prolog | `IDM_LANG_VISUALPROLOG`; Cmd:3994 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1093 Language [flat] > XML | `IDM_LANG_XML`; Cmd:3916 | Not listed | `LanguageXml`; `src/app/menus/language.rs:40`; `src/workspace/language.rs:72` | none → none; C:322 | R/U |
| RC:1094 Language [flat] > YAML | `IDM_LANG_YAML`; Cmd:3962 | Not listed | `LanguageYaml`; `src/app/menus/language.rs:41`; `src/workspace/language.rs:73` | none → none; C:323 | R/U |
| RC:1095 Language [flat] > separator | separator | — | — | — | Layout reference |
| RC:1096 Language [flat] > Define your language... | `IDM_LANG_USER_DLG`; Cmd:1987 | None assigned (P:469) | No corresponding installed menubar action located | — | M |
| RC:1097 Language [flat] > Open User Defined Language folder... | `IDM_LANG_OPENUDLDIR`; Cmd:4021 | None assigned (P:471) | No corresponding installed menubar action located | — | M |
| RC:1098 Language [flat] > Notepad++ User Defined Languages Collection | `IDM_LANG_UDLCOLLECTION_PROJECT_SITE`; Cmd:4028 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1099 Language [flat] > User-Defined | `IDM_LANG_USER`; Cmd:4005 | None assigned (P:470) | No corresponding installed menubar action located | — | M |
### Language [grouped]

| Resource node / hierarchy | Oracle ID / dispatch | Oracle default | Studio route | Catalog Windows/Linux → Mac | Assessment |
| --- | --- | --- | --- | --- | --- |
| RC:1102 Language [grouped] | popup | — | See children | — | Layout reference |
| RC:1104 Language [grouped] > None (Normal Text) | `IDM_LANG_TEXT`; Cmd:3933 | Not listed | `LanguageNormalText`; `src/app/menus/language.rs:15`; `src/workspace/language.rs:48` | none → none; C:298 | R/U |
| RC:1105 Language [grouped] > separator | separator | — | — | — | Layout reference |
| RC:1106 Language [grouped] > A | popup | — | See children | — | Layout reference |
| RC:1108 Language [grouped] > A > ActionScript | `IDM_LANG_FLASH`; Cmd:3941 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1109 Language [grouped] > A > Ada | `IDM_LANG_ADA`; Cmd:3955 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1110 Language [grouped] > A > ASN.1 | `IDM_LANG_ASN1`; Cmd:3975 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1111 Language [grouped] > A > ASP | `IDM_LANG_ASP`; Cmd:3921 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1112 Language [grouped] > A > Assembly | `IDM_LANG_ASM`; Cmd:3946 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1113 Language [grouped] > A > AutoIt | `IDM_LANG_AU3`; Cmd:3959 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1114 Language [grouped] > A > AviSynth | `IDM_LANG_AVS`; Cmd:3976 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1116 Language [grouped] > B | popup | — | See children | — | Layout reference |
| RC:1118 Language [grouped] > B > BaanC | `IDM_LANG_BAANC`; Cmd:3970 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1119 Language [grouped] > B > Batch | `IDM_LANG_BATCH`; Cmd:3927 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1120 Language [grouped] > B > Blitzbasic | `IDM_LANG_BLITZBASIC`; Cmd:3977 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1122 Language [grouped] > C | popup | — | See children | — | Layout reference |
| RC:1124 Language [grouped] > C > C | `IDM_LANG_C`; Cmd:3911 | Not listed | `LanguageC`; `src/app/menus/language.rs:17`; `src/workspace/language.rs:49` | none → none; C:299 | R/U |
| RC:1125 Language [grouped] > C > C# | `IDM_LANG_CS`; Cmd:3914 | Not listed | `LanguageCSharp`; `src/app/menus/language.rs:18`; `src/workspace/language.rs:50` | none → none; C:300 | R/U |
| RC:1126 Language [grouped] > C > C++ | `IDM_LANG_CPP`; Cmd:3912 | Not listed | `LanguageCpp`; `src/app/menus/language.rs:19`; `src/workspace/language.rs:51` | none → none; C:301 | R/U |
| RC:1127 Language [grouped] > C > Caml | `IDM_LANG_CAML`; Cmd:3954 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1128 Language [grouped] > C > CMake | `IDM_LANG_CMAKE`; Cmd:3961 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1129 Language [grouped] > C > COBOL | `IDM_LANG_COBOL`; Cmd:3963 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1130 Language [grouped] > C > CSound | `IDM_LANG_CSOUND`; Cmd:3980 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1131 Language [grouped] > C > CoffeeScript | `IDM_LANG_COFFEESCRIPT`; Cmd:3969 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1132 Language [grouped] > C > CSS | `IDM_LANG_CSS`; Cmd:3922 | Not listed | `LanguageCss`; `src/app/menus/language.rs:20`; `src/workspace/language.rs:52` | none → none; C:302 | R/U |
| RC:1134 Language [grouped] > D | popup | — | See children | — | Layout reference |
| RC:1136 Language [grouped] > D > D | `IDM_LANG_D`; Cmd:3964 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1137 Language [grouped] > D > Diff | `IDM_LANG_DIFF`; Cmd:3947 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1139 Language [grouped] > E | popup | — | See children | — | Layout reference |
| RC:1141 Language [grouped] > E > Erlang | `IDM_LANG_ERLANG`; Cmd:3981 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1142 Language [grouped] > E > ErrorList | `IDM_LANG_ERRORLIST`; Cmd:4002 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1143 Language [grouped] > E > Escape Sequence (ANSI) | `IDM_LANG_ESCSEQ`; Cmd:4003 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1144 Language [grouped] > E > ESCRIPT | `IDM_LANG_ESCRIPT`; Cmd:3982 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1146 Language [grouped] > F | popup | — | See children | — | Layout reference |
| RC:1148 Language [grouped] > F > Forth | `IDM_LANG_FORTH`; Cmd:3983 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1149 Language [grouped] > F > Fortran (free form) | `IDM_LANG_FORTRAN`; Cmd:3938 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1150 Language [grouped] > F > Fortran (fixed form) | `IDM_LANG_FORTRAN_77`; Cmd:3939 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1151 Language [grouped] > F > Freebasic | `IDM_LANG_FREEBASIC`; Cmd:3979 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1153 Language [grouped] > G | popup | — | See children | — | Layout reference |
| RC:1155 Language [grouped] > G > GDScript | `IDM_LANG_GDSCRIPT`; Cmd:3996 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1156 Language [grouped] > G > Go | `IDM_LANG_GOLANG`; Cmd:3998 | Not listed | `LanguageGo`; `src/app/menus/language.rs:21`; `src/workspace/language.rs:53` | none → none; C:303 | R/U |
| RC:1157 Language [grouped] > G > Gui4Cli | `IDM_LANG_GUI4CLI`; Cmd:3965 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1159 Language [grouped] > H | popup | — | See children | — | Layout reference |
| RC:1161 Language [grouped] > H > Haskell | `IDM_LANG_HASKELL`; Cmd:3958 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1162 Language [grouped] > H > Hollywood | `IDM_LANG_HOLLYWOOD`; Cmd:3997 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1163 Language [grouped] > H > HTML | `IDM_LANG_HTML`; Cmd:3915 | Not listed | `LanguageHtml`; `src/app/menus/language.rs:22`; `src/workspace/language.rs:54` | none → none; C:304 | R/U |
| RC:1165 Language [grouped] > I | popup | — | See children | — | Layout reference |
| RC:1167 Language [grouped] > I > INI file | `IDM_LANG_INI`; Cmd:3936 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1168 Language [grouped] > I > Inno Setup | `IDM_LANG_INNO`; Cmd:3960 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1169 Language [grouped] > I > Intel HEX | `IDM_LANG_IHEX`; Cmd:3972 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1171 Language [grouped] > J | popup | — | See children | — | Layout reference |
| RC:1173 Language [grouped] > J > Java | `IDM_LANG_JAVA`; Cmd:3913 | Not listed | `LanguageJava`; `src/app/menus/language.rs:23`; `src/workspace/language.rs:55` | none → none; C:305 | R/U |
| RC:1174 Language [grouped] > J > JavaScript | `IDM_LANG_JS`; Cmd:3917 | Not listed | `LanguageJavaScript`; `src/app/menus/language.rs:24`; `src/workspace/language.rs:56` | none → none; C:306 | R/U |
| RC:1175 Language [grouped] > J > JSON | `IDM_LANG_JSON`; Cmd:3918 | Not listed | `LanguageJson`; `src/app/menus/language.rs:25`; `src/workspace/language.rs:57` | none → none; C:307 | R/U |
| RC:1176 Language [grouped] > J > JSON5 | `IDM_LANG_JSON5`; Cmd:3919 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1177 Language [grouped] > J > JSP | `IDM_LANG_JSP`; Cmd:3968 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1179 Language [grouped] > KIXtart | `IDM_LANG_KIX`; Cmd:3953 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1180 Language [grouped] > L | popup | — | See children | — | Layout reference |
| RC:1182 Language [grouped] > L > LaTeX | `IDM_LANG_LATEX`; Cmd:3984 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1183 Language [grouped] > L > LISP | `IDM_LANG_LISP`; Cmd:3944 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1184 Language [grouped] > L > Lua | `IDM_LANG_LUA`; Cmd:3923 | Not listed | `LanguageLua`; `src/app/menus/language.rs:26`; `src/workspace/language.rs:58` | none → none; C:308 | R/U |
| RC:1186 Language [grouped] > M | popup | — | See children | — | Layout reference |
| RC:1188 Language [grouped] > M > Makefile | `IDM_LANG_MAKEFILE`; Cmd:3935 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1189 Language [grouped] > M > Matlab | `IDM_LANG_MATLAB`; Cmd:3957 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1190 Language [grouped] > M > Microsoft Transact-SQL | `IDM_LANG_MSSQL`; Cmd:3931 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1191 Language [grouped] > M > MMIXAL | `IDM_LANG_MMIXAL`; Cmd:3985 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1192 Language [grouped] > M > MS-DOS Style | `IDM_LANG_ASCII`; Cmd:3932 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1194 Language [grouped] > N | popup | — | See children | — | Layout reference |
| RC:1196 Language [grouped] > N > Nim | `IDM_LANG_NIM`; Cmd:3986 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1197 Language [grouped] > N > Nncrontab | `IDM_LANG_NNCRONTAB`; Cmd:3987 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1198 Language [grouped] > N > NSIS | `IDM_LANG_NSIS`; Cmd:3942 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1200 Language [grouped] > O | popup | — | See children | — | Layout reference |
| RC:1202 Language [grouped] > O > Objective-C | `IDM_LANG_OBJC`; Cmd:3928 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1203 Language [grouped] > O > OScript | `IDM_LANG_OSCRIPT`; Cmd:3988 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1205 Language [grouped] > P | popup | — | See children | — | Layout reference |
| RC:1207 Language [grouped] > P > Pascal | `IDM_LANG_PASCAL`; Cmd:3926 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1208 Language [grouped] > P > Perl | `IDM_LANG_PERL`; Cmd:3924 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1209 Language [grouped] > P > PHP | `IDM_LANG_PHP`; Cmd:3920 | Not listed | `LanguagePhp`; `src/app/menus/language.rs:28`; `src/workspace/language.rs:60` | none → none; C:310 | R/U |
| RC:1210 Language [grouped] > P > PostScript | `IDM_LANG_PS`; Cmd:3949 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1211 Language [grouped] > P > PowerShell | `IDM_LANG_POWERSHELL`; Cmd:3966 | Not listed | `LanguagePowerShell`; `src/app/menus/language.rs:29`; `src/workspace/language.rs:61` | none → none; C:311 | R/U |
| RC:1212 Language [grouped] > P > Properties | `IDM_LANG_PROPS`; Cmd:3948 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1213 Language [grouped] > P > Purebasic | `IDM_LANG_PUREBASIC`; Cmd:3978 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1214 Language [grouped] > P > Python | `IDM_LANG_PYTHON`; Cmd:3925 | Not listed | `LanguagePython`; `src/app/menus/language.rs:30`; `src/workspace/language.rs:62` | none → none; C:312 | R/U |
| RC:1216 Language [grouped] > R | popup | — | See children | — | Layout reference |
| RC:1218 Language [grouped] > R > R | `IDM_LANG_R`; Cmd:3967 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1219 Language [grouped] > R > Raku | `IDM_LANG_RAKU`; Cmd:3999 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1220 Language [grouped] > R > REBOL | `IDM_LANG_REBOL`; Cmd:3989 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1221 Language [grouped] > R > Registry | `IDM_LANG_REGISTRY`; Cmd:3990 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1222 Language [grouped] > R > Resource file | `IDM_LANG_RC`; Cmd:3934 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1223 Language [grouped] > R > Ruby | `IDM_LANG_RUBY`; Cmd:3950 | Not listed | `LanguageRuby`; `src/app/menus/language.rs:31`; `src/workspace/language.rs:63` | none → none; C:313 | R/U |
| RC:1224 Language [grouped] > R > Rust | `IDM_LANG_RUST`; Cmd:3991 | Not listed | `LanguageRust`; `src/app/menus/language.rs:32`; `src/workspace/language.rs:64` | none → none; C:314 | R/U |
| RC:1226 Language [grouped] > S | popup | — | See children | — | Layout reference |
| RC:1228 Language [grouped] > S > SAS | `IDM_LANG_SAS`; Cmd:4001 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1229 Language [grouped] > S > Shell | `IDM_LANG_BASH`; Cmd:3940 | Not listed | `LanguageShell`; `src/app/menus/language.rs:34`; `src/workspace/language.rs:66` | none → none; C:316 | R/U |
| RC:1230 Language [grouped] > S > Scheme | `IDM_LANG_SCHEME`; Cmd:3945 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1231 Language [grouped] > S > Smalltalk | `IDM_LANG_SMALLTALK`; Cmd:3951 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1232 Language [grouped] > S > Spice | `IDM_LANG_SPICE`; Cmd:3992 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1233 Language [grouped] > S > SQL | `IDM_LANG_SQL`; Cmd:3930 | Not listed | `LanguageSql`; `src/app/menus/language.rs:35`; `src/workspace/language.rs:67` | none → none; C:317 | R/U |
| RC:1234 Language [grouped] > S > Structured Text | `IDM_LANG_STTXT`; Cmd:4004 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1235 Language [grouped] > S > Swift | `IDM_LANG_SWIFT`; Cmd:3974 | Not listed | `LanguageSwift`; `src/app/menus/language.rs:36`; `src/workspace/language.rs:68` | none → none; C:318 | R/U |
| RC:1236 Language [grouped] > S > S-Record | `IDM_LANG_SREC`; Cmd:3971 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1238 Language [grouped] > T | popup | — | See children | — | Layout reference |
| RC:1240 Language [grouped] > T > TCL | `IDM_LANG_TCL`; Cmd:3943 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1241 Language [grouped] > T > Tektronix extended HEX | `IDM_LANG_TEHEX`; Cmd:3973 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1242 Language [grouped] > T > TeX | `IDM_LANG_TEX`; Cmd:3937 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1243 Language [grouped] > T > TOML | `IDM_LANG_TOML`; Cmd:4000 | Not listed | `LanguageToml`; `src/app/menus/language.rs:37`; `src/workspace/language.rs:69` | none → none; C:319 | R/U |
| RC:1244 Language [grouped] > T > txt2tags | `IDM_LANG_TXT2TAGS`; Cmd:3993 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1245 Language [grouped] > T > TypeScript | `IDM_LANG_TYPESCRIPT`; Cmd:3995 | Not listed | `LanguageTypeScript`; `src/app/menus/language.rs:39`; `src/workspace/language.rs:71` | none → none; C:321 | R/U |
| RC:1247 Language [grouped] > V | popup | — | See children | — | Layout reference |
| RC:1249 Language [grouped] > V > Visual Basic | `IDM_LANG_VB`; Cmd:3929 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1250 Language [grouped] > V > Visual Prolog | `IDM_LANG_VISUALPROLOG`; Cmd:3994 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1251 Language [grouped] > V > VHDL | `IDM_LANG_VHDL`; Cmd:3952 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1252 Language [grouped] > V > Verilog | `IDM_LANG_VERILOG`; Cmd:3956 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1254 Language [grouped] > XML | `IDM_LANG_XML`; Cmd:3916 | Not listed | `LanguageXml`; `src/app/menus/language.rs:40`; `src/workspace/language.rs:72` | none → none; C:322 | R/U |
| RC:1255 Language [grouped] > YAML | `IDM_LANG_YAML`; Cmd:3962 | Not listed | `LanguageYaml`; `src/app/menus/language.rs:41`; `src/workspace/language.rs:73` | none → none; C:323 | R/U |
| RC:1256 Language [grouped] > separator | separator | — | — | — | Layout reference |
| RC:1257 Language [grouped] > User Defined Language | popup | — | See children | — | Layout reference |
| RC:1259 Language [grouped] > User Defined Language > Define your language... | `IDM_LANG_USER_DLG`; Cmd:1987 | None assigned (P:469) | No corresponding installed menubar action located | — | M |
| RC:1260 Language [grouped] > User Defined Language > Open User Defined Language folder... | `IDM_LANG_OPENUDLDIR`; Cmd:4021 | None assigned (P:471) | No corresponding installed menubar action located | — | M |
| RC:1261 Language [grouped] > User Defined Language > Notepad++ User Defined Languages Collection | `IDM_LANG_UDLCOLLECTION_PROJECT_SITE`; Cmd:4028 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1263 Language [grouped] > User-Defined | `IDM_LANG_USER`; Cmd:4005 | None assigned (P:470) | No corresponding installed menubar action located | — | M |
### Settings

| Resource node / hierarchy | Oracle ID / dispatch | Oracle default | Studio route | Catalog Windows/Linux → Mac | Assessment |
| --- | --- | --- | --- | --- | --- |
| RC:1266 Settings | popup | — | See children | — | Layout reference |
| RC:1268 Settings > Preferences... | `IDM_SETTING_PREFERENCE`; Cmd:3506 | None assigned (P:473) | `OpenPreferences`; `src/app/menus/settings.rs:14`; `src/app/menus.rs:115` | none → `cmd-,`; C:346 | R/U |
| RC:1269 Settings > Style Configurator... | `IDM_LANGSTYLE_CONFIG_DLG`; Cmd:3895 | None assigned (P:474) | `OpenStyleConfigurator`; `src/app/menus/settings.rs:15`; `src/app/menus.rs:118` | none → `cmd-shift-,`; C:347 | R/U |
| RC:1270 Settings > Shortcut Mapper... | `IDM_SETTING_SHORTCUT_MAPPER`; Cmd:3495 | None assigned (P:475) | `OpenShortcutMapper`; `src/app/menus/settings.rs:16`; `src/app/menus.rs:112` | **No row** | R/U; no catalog row |
| RC:1271 Settings > separator | separator | — | — | — | Layout reference |
| RC:1272 Settings > Import | popup | — | See children | — | Layout reference |
| RC:1274 Settings > Import > Import plugin(s)... | `IDM_SETTING_IMPORTPLUGIN`; Cmd:3426 | None assigned (P:476) | No corresponding installed menubar action located | — | M |
| RC:1275 Settings > Import > Import style theme(s)... | `IDM_SETTING_IMPORTSTYLETHEMES`; Cmd:3446 | None assigned (P:477) | No corresponding installed menubar action located | — | M |
| RC:1277 Settings > separator | separator | — | — | — | Layout reference |
| RC:1278 Settings > Edit Popup ContextMenu | `IDM_SETTING_EDITCONTEXTMENU`; Cmd:3518 | None assigned (P:478) | No corresponding installed menubar action located | — | M |
### Tools

| Resource node / hierarchy | Oracle ID / dispatch | Oracle default | Studio route | Catalog Windows/Linux → Mac | Assessment |
| --- | --- | --- | --- | --- | --- |
| RC:1281 Tools | popup | — | See children | — | Layout reference |
| RC:1283 Tools > MD5 | popup | — | See children | — | Layout reference |
| RC:1285 Tools > MD5 > Generate... | `IDM_TOOL_MD5_GENERATE`; Cmd:3577 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1286 Tools > MD5 > Generate from files... | `IDM_TOOL_MD5_GENERATEFROMFILE`; Cmd:3586 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1287 Tools > MD5 > Generate from selection into clipboard | `IDM_TOOL_MD5_GENERATEINTOCLIPBOARD`; Cmd:3595 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1289 Tools > SHA-1 | popup | — | See children | — | Layout reference |
| RC:1291 Tools > SHA-1 > Generate... | `IDM_TOOL_SHA1_GENERATE`; Cmd:3620 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1292 Tools > SHA-1 > Generate from files... | `IDM_TOOL_SHA1_GENERATEFROMFILE`; Cmd:3629 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1293 Tools > SHA-1 > Generate from selection into clipboard | `IDM_TOOL_SHA1_GENERATEINTOCLIPBOARD`; Cmd:3674 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1295 Tools > SHA-256 | popup | — | See children | — | Layout reference |
| RC:1297 Tools > SHA-256 > Generate... | `IDM_TOOL_SHA256_GENERATE`; Cmd:3638 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1298 Tools > SHA-256 > Generate from files... | `IDM_TOOL_SHA256_GENERATEFROMFILE`; Cmd:3647 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1299 Tools > SHA-256 > Generate from selection into clipboard | `IDM_TOOL_SHA256_GENERATEINTOCLIPBOARD`; Cmd:3675 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1301 Tools > SHA-512 | popup | — | See children | — | Layout reference |
| RC:1303 Tools > SHA-512 > Generate... | `IDM_TOOL_SHA512_GENERATE`; Cmd:3656 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1304 Tools > SHA-512 > Generate from files... | `IDM_TOOL_SHA512_GENERATEFROMFILE`; Cmd:3665 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1305 Tools > SHA-512 > Generate from selection into clipboard | `IDM_TOOL_SHA512_GENERATEINTOCLIPBOARD`; Cmd:3676 | Not listed | No corresponding installed menubar action located | — | M |
### Macro

| Resource node / hierarchy | Oracle ID / dispatch | Oracle default | Studio route | Catalog Windows/Linux → Mac | Assessment |
| --- | --- | --- | --- | --- | --- |
| RC:1309 Macro | popup | — | See children | — | Layout reference |
| RC:1311 Macro > Start Recording | `IDM_MACRO_STARTRECORDINGMACRO`; Cmd:1441 | None assigned (P:481) | `MacroStartStopRecording`; `src/app/menus/macros.rs:17`; `src/workspace/macros.rs:196` | `ctrl-shift-r` → `cmd-shift-r`; C:326 | D/U: combined Start/Stop toggle row |
| RC:1312 Macro > Stop Recording | `IDM_MACRO_STOPRECORDINGMACRO`; Cmd:1442 | None assigned (P:482) | `MacroStartStopRecording`; `src/app/menus/macros.rs:17`; `src/workspace/macros.rs:196` | `ctrl-shift-r` → `cmd-shift-r`; C:326 | D/U: same combined Start/Stop toggle row |
| RC:1313 Macro > Playback | `IDM_MACRO_PLAYBACKRECORDEDMACRO`; Cmd:1473 | `ctrl-shift-p` (P:483) | `MacroPlayback`; `src/app/menus/macros.rs:18`; `src/workspace/macros.rs:248` | `ctrl-shift-p` → `cmd-shift-p`; C:327 | R/U |
| RC:1314 Macro > Save Current Recorded Macro... | `IDM_MACRO_SAVECURRENTMACRO`; Cmd:1497 | None assigned (P:484) | `MacroSaveCurrent`; `src/app/menus/macros.rs:20`; `src/workspace/macros.rs:281` | none → none; C:329 | R/U |
| RC:1315 Macro > Run a Macro Multiple Times... | `IDM_MACRO_RUNMULTIMACRODLG`; Cmd:1480 | None assigned (P:485) | `MacroRunMultipleTimes`; `src/app/menus/macros.rs:19`; `src/workspace/macros.rs:266` | none → `cmd-shift-m`; C:328 | R/U |
### Run

| Resource node / hierarchy | Oracle ID / dispatch | Oracle default | Studio route | Catalog Windows/Linux → Mac | Assessment |
| --- | --- | --- | --- | --- | --- |
| RC:1318 Run | popup | — | See children | — | Layout reference |
| RC:1320 Run > Run... | `IDM_EXECUTE`; Cmd:2945 | `f5` (P:487) | `RunCommand`; `src/app/menus/run.rs:14`; `src/workspace/run.rs:71` | `f5` → `f5`; C:332 | R/U |
| RC:1321 Run > separator | separator | — | — | — | Layout reference |
| RC:1322 Run > Validate shortcuts.xml | `IDM_EXECUTE_VALIDATE_SHORTCUTSXML`; Cmd:2955 | None assigned (P:488) | `RunValidateShortcuts`; `src/app/menus/run.rs:16`; `src/workspace/run.rs:89` | none → none; C:333 | R/U |
### Plugins

| Resource node / hierarchy | Oracle ID / dispatch | Oracle default | Studio route | Catalog Windows/Linux → Mac | Assessment |
| --- | --- | --- | --- | --- | --- |
| RC:1325 Plugins | popup | — | See children | — | Layout reference |
| RC:1327 Plugins > Open Plugins Folder... | `IDM_SETTING_OPENPLUGINSDIR`; Cmd:3485 | Not listed | `PluginsOpenFolder`; `src/app/menus/plugins.rs:21`; `src/app/menus.rs:176` | **No row** | R/U; no catalog row |
### Window

| Resource node / hierarchy | Oracle ID / dispatch | Oracle default | Studio route | Catalog Windows/Linux → Mac | Assessment |
| --- | --- | --- | --- | --- | --- |
| RC:1330 Window | popup | — | See children | — | Layout reference |
| RC:1332 Window > Sort By | popup | — | See children | — | Layout reference |
| RC:1334 Window > Sort By > Name A to Z | `IDM_WINDOW_SORT_FN_ASC`; Cmd:4125 | None assigned (P:491) | No corresponding installed menubar action located | — | M |
| RC:1335 Window > Sort By > Name Z to A | `IDM_WINDOW_SORT_FN_DSC`; Cmd:4134 | None assigned (P:492) | No corresponding installed menubar action located | — | M |
| RC:1336 Window > Sort By > Path A to Z | `IDM_WINDOW_SORT_FP_ASC`; Cmd:4143 | None assigned (P:493) | No corresponding installed menubar action located | — | M |
| RC:1337 Window > Sort By > Path Z to A | `IDM_WINDOW_SORT_FP_DSC`; Cmd:4152 | None assigned (P:494) | No corresponding installed menubar action located | — | M |
| RC:1338 Window > Sort By > Type A to Z | `IDM_WINDOW_SORT_FT_ASC`; Cmd:4161 | None assigned (P:495) | No corresponding installed menubar action located | — | M |
| RC:1339 Window > Sort By > Type Z to A | `IDM_WINDOW_SORT_FT_DSC`; Cmd:4170 | None assigned (P:496) | No corresponding installed menubar action located | — | M |
| RC:1340 Window > Sort By > Content Length Ascending | `IDM_WINDOW_SORT_FS_ASC`; Cmd:4179 | None assigned (P:497) | No corresponding installed menubar action located | — | M |
| RC:1341 Window > Sort By > Content Length Descending | `IDM_WINDOW_SORT_FS_DSC`; Cmd:4188 | None assigned (P:498) | No corresponding installed menubar action located | — | M |
| RC:1342 Window > Sort By > Modified Time Ascending | `IDM_WINDOW_SORT_FD_ASC`; Cmd:4197 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1343 Window > Sort By > Modified Time Descending | `IDM_WINDOW_SORT_FD_DSC`; Cmd:4206 | Not listed | No corresponding installed menubar action located | — | M |
| RC:1345 Window > Windows... | `IDM_WINDOW_WINDOWS`; Cmd:4117 | None assigned (P:490) | No corresponding installed menubar action located | — | M |
| RC:1346 Window > separator | separator | — | — | — | Layout reference |
| RC:1347 Window > Recent Window | `IDM_WINDOW_MRU_FIRST`; non-switch/dynamic route | Not listed | No equivalent dynamic window menu located | — | M; resource starts GRAYED, runtime population U; RC flags GRAYED |
### ?

| Resource node / hierarchy | Oracle ID / dispatch | Oracle default | Studio route | Catalog Windows/Linux → Mac | Assessment |
| --- | --- | --- | --- | --- | --- |
| RC:1350 ? | popup | — | See children | — | Layout reference |
| RC:1352 ? > Command Line Arguments... | `IDM_CMDLINEARGUMENTS`; Cmd:3801 | None assigned (P:500) | No corresponding installed menubar action located | — | M |
| RC:1353 ? > separator | separator | — | — | — | Layout reference |
| RC:1354 ? > Notepad++ Home | `IDM_HOMESWEETHOME`; Cmd:3784 | None assigned (P:501) | No corresponding installed menubar action located | — | M |
| RC:1355 ? > Notepad++ Project Page | `IDM_PROJECTPAGE`; Cmd:3789 | None assigned (P:502) | No corresponding installed menubar action located | — | M |
| RC:1356 ? > Notepad++ Online User Manual | `IDM_ONLINEDOCUMENT`; Cmd:3795 | None assigned (P:503) | No corresponding installed menubar action located | — | M |
| RC:1357 ? > Notepad++ Community (Forum) | `IDM_FORUM`; Cmd:3807 | None assigned (P:504) | No corresponding installed menubar action located | — | M |
| RC:1358 ? > separator | separator | — | — | — | Layout reference |
| RC:1359 ? > Update Notepad++ | `IDM_UPDATE_NPP`; Cmd:3813 | None assigned (P:511) | No corresponding installed menubar action located | — | M |
| RC:1360 ? > Set Updater Proxy... | `IDM_CONFUPDATERPROXY`; Cmd:3814 | None assigned (P:512) | No corresponding installed menubar action located | — | M |
| RC:1361 ? > separator | separator | — | — | — | Layout reference |
| RC:1362 ? > Debug Info... | `IDM_DEBUGINFO`; Cmd:3732 | None assigned (P:513) | No corresponding installed menubar action located | — | M |
| RC:1363 ? > About Notepad++ | `IDM_ABOUT`; Cmd:3741 | `f1` (P:514) | `HelpAbout`; `src/app/menus/help.rs:13`; `src/app/menus.rs:104` | `f1` → none; C:350 | R/U |
### Root buttons

| Resource node / hierarchy | Oracle ID / dispatch | Oracle default | Studio route | Catalog Windows/Linux → Mac | Assessment |
| --- | --- | --- | --- | --- | --- |
| RC:1366 ＋ | `IDM_FILE_NEW`; Cmd:162 | `ctrl-n` (P:108) | `FileNew`; `src/app/menus/file.rs:20`; `src/io.rs:29` | `ctrl-n` → `cmd-n`; C:126 | R/U; RC flags HELP |
### ▼

| Resource node / hierarchy | Oracle ID / dispatch | Oracle default | Studio route | Catalog Windows/Linux → Mac | Assessment |
| --- | --- | --- | --- | --- | --- |
| RC:1367 ▼ | popup | — | See children | — | Layout reference |
| RC:1369 ▼ > Recent Window | `IDM_DROPLIST_LIST`; non-switch/dynamic route | Not listed | No equivalent dynamic window menu located | — | M; resource starts GRAYED, runtime population U; RC flags GRAYED |
### Root buttons

| Resource node / hierarchy | Oracle ID / dispatch | Oracle default | Studio route | Catalog Windows/Linux → Mac | Assessment |
| --- | --- | --- | --- | --- | --- |
| RC:1371 ✕ | `IDM_FILE_CLOSE`; Cmd:394 | `ctrl-w` (P:121) | `FileCloseTab`; `src/app/menus/file.rs:63`; `src/workspace/tabs.rs:269` | `ctrl-w` → `cmd-w`; C:135 | R/U; RC flags HELP |

## Follow-up acceptance criteria

First complete the tracked safety leaves and retain their narrow refusal/race regressions. Then add existing-menu catalog rows using stable action type names, behavior-oracle defaults, no invented shortcut where the oracle has none, and deliberate macOS mirrors. Prove that every static installed menu action is either cataloged or explicitly documented as a dynamic payload action; exercise actual mapper visibility and custom binding dispatch without writing user settings.

For new parity commands, establish the exact oracle behavior before implementation: single-line and multi-line selections, Unicode/CRLF/BOM/UTF-16, read-only and loading buffers, focus, UUID and asynchronous races, undo grouping, recovery and macro replay. A menu row alone must not turn this inventory’s U into a behavior pass. Native UI comparison and the three OS build/runtime gates remain required to claim complete parity.
