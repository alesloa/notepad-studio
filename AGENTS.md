# AGENTS.md: guide for AI agents and contributors

Notepad Studio is an open-source, native, cross-platform text editor modelled on Notepad++, written in **Rust only** (no JavaScript or TypeScript). The UI is [gpui](https://gpui.rs/), Zed's GPU UI framework, vendored under `vendor/gpui`. **No Tauri, Electron or webview shells:** never add a webview or JS UI framework as a dependency, build step or packaging target. The product name is always written **Notepad Studio** (two words, both capitalised; `crate::consts::APP_NAME`) in user-facing text: UI, window titles, About, dialogs, release names, docs prose. Machine names stay lowercase and hyphenated: crates (`notepad-studio-*`), the binary, file names, the bundle id `io.alesloa.notepad-studio`. gpui actions live in the `notepad_studio` namespace. The goal is 1:1 Notepad++ parity (same menus, shortcuts, behaviour, file and theme fidelity) on macOS, Linux and Windows, with better performance. Read this file first, then `README.md`.

## 1. Orientation (5 minutes)

| Read | Why |
|---|---|
| `README.md` | What has shipped, platform notes, plugin install, build. Its **Status** line names the work in flight |
| `change.log` | User-facing release notes; the app opens it on first launch after every version bump (`src/changelog.rs`) |
| `CONTRIBUTING.md` | PR rules, licence of contributions |
| `NOTICE` | Where vendored gpui came from and every change made to it |
| `crates/notepad-studio-plugin/src/ffi.rs` | The plugin C ABI |
| `docs/icons.md` | Regenerating the app icons |

**The Notepad++ source is the behaviour oracle.** Keep a checkout of https://github.com/notepad-plus-plus/notepad-plus-plus beside this repo at `../notepad-plus-plus` (`tools/gen_themes/gen_themes.py` looks there; `NPP_STYLERS` overrides). It is never vendored or committed. Under `PowerEditor/src/`:

| File | Answers |
|---|---|
| `Notepad_plus.rc` (`IDR_M30_MENU`) | Menu tree: item order, labels, separators |
| `Parameters.cpp` (`winKeyDefs[]`) | Default shortcuts |
| `menuCmdID.h` | `IDM_*` command ids |
| `NppCommands.cpp` | What each command does |
| `stylers.model.xml`, `langs.model.xml` | Theme and language schema |
| `WinControls/Preference/preference.rc`, `WinControls/ColourPicker/WordStyleDlg.rc`, `ScintillaComponent/FindReplaceDlg.rc` | Dialog layouts: Preferences, Style Configurator, Find |

## 2. Workspace map

```text
src/
  main.rs              startup: panic hook, single-instance gate, plugins, actions, keymap, menus, session restore
  actions/             gpui `actions!` blocks, one file per menu area
  app/menus/           one file per menubar menu; item order mirrors Notepad++
  app/window.rs        open, restore and raise editor windows
  shortcuts.rs         CATALOG: every rebindable action and its default chords (feeds the Shortcut Mapper)
  workspace/           `Workspace`, the per-window entity: tabs, edits, movement, find, macros.
                       `render.rs` wires every editor action with `.on_action(cx.listener(..))`
  edit_ops/            Notepad++ Edit-submenu commands: `impl Workspace` handlers + pure transforms + tests
  buffer.rs tab.rs     ropey buffer and tab model
  text/                encoding, EOL, extended escapes, wrap, UTF-16 (pure, heavily tested)
  syntax/              tree-sitter grammars (24 languages), keywords, comment tokens
  style_model/         Notepad++ theme XML: load, save, lexer styles, bundled themes
  theme.rs             runtime UI colours (`Theme`)
  ui/                  dialogs and panels: find, preferences, style configurator, shortcut mapper,
                       toolbar, context menus, CSV grid, markdown and rich-doc views, text tools
  rich_doc/            .docx / .odt / .rtf import, edit, export
  io.rs io_stream.rs   file load and save; streaming reads for large files
  settings.rs session.rs config.rs recent.rs macros.rs
                       state persisted as TOML under `<config>/Notepad Studio/` (section 8)
  plugins/             cdylib loader and host API
  panic_log.rs         panic hook, writes panic.log
crates/
  notepad-studio-plugin    plugin C ABI, safe `Plugin` trait, `declare_plugin!`
  sample-uppercase-plugin  example plugin
vendor/gpui/           vendored Zed gpui and support crates (Apache-2.0)
assets/                icons and themes, embedded in the binary at compile time
tools/gen_themes/      generates assets/themes/*.xml
tests/                 integration tests
```

`src/app/keymap/*.rs` are not declared as modules, so the compiler never sees them. User-facing chords go in `CATALOG`; chords internal to the Find dialog go in `register_dialog_only` (`src/app/keymap.rs`).

## 3. Golden rules

### Never crash (outranks feature work)

People keep unsaved work in Notepad Studio, and the release profile sets `panic = "abort"`: one panic closes every window at once, and the debounced session autosave loses the latest edits with it. A malformed file, an odd encoding, a corrupt settings or session file, a broken theme XML, a bad plugin call, an odd keystroke or a full disk must produce an error the user can act on (`Workspace::set_status`, or an `rfd` dialog), never a panic. Fix a crash before building on top of it.

- **Non-test code never panics.** No `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!` or `unimplemented!`. Return `Result` and propagate with `?` (`anyhow` in the app, `thiserror` for typed errors); use `let .. else`, `if let`, `ok_or(..)?`, or `unwrap_or*` where a fallback is truly correct (never one that silently corrupts a document). Older code still has some panic sites: add none, and convert the ones on lines you touch.
- **`unsafe` lives only at FFI seams:** the plugin ABI (`crates/notepad-studio-plugin`, `src/plugins/`), the macOS pasteboard bridge (`src/clipboard_rich.rs`), Linux `nl_langinfo` (`src/locale_date.rs`) and the Windows locale-sort bridge (`src/locale_sort_windows.rs`, exactly two audited `LCMapStringEx` calls behind a safe, bounded `Result` interface). The user delegated this narrow Windows exception on 2026-10-11. Every new block carries a `// SAFETY:` comment. Everything else is safe Rust.
- **File contents are hostile.** Buffer positions are byte offsets into the rope (`replace_byte_range`, `selection().head`): clamp them to `len_bytes()` and land them on a char boundary. Use `get()` rather than `[i]`/`[a..b]` for indices derived from files, params or arithmetic on them; use `checked_*`/`saturating_*` for lengths and offsets; guard division by zero; cap allocations sized by input.
- **Bound recursion** with depth limits: markdown, XML and DOCX/ODT/RTF can nest arbitrarily deep.
- **Don't cascade.** Handle a poisoned mutex (`let Ok(g) = m.lock() else { .. }`, as the existing code does) and treat thread joins as `Result`s.
- **Prove it.** Every crash fix comes with a small synthetic regression test that panicked before the fix.

### Rules

1. **Everything is an action, wired where Notepad++ wires it.** New user-visible behaviour is a gpui action that lands in every place Notepad++ exposes it:
   1. Declare it in `src/actions/<area>.rs` (`gpui::actions!(notepad_studio, [..])`), named after the Notepad++ menu item.
   2. Handle it. An editor action is a `Workspace` method registered in `src/workspace/render.rs` with `.on_action(cx.listener(Self::method))`; a window-less action goes in `install_global_actions` (`src/app/menus.rs`). A Notepad++ Edit-submenu command gets its own `src/edit_ops/<op>.rs`.
   3. Add the menu item at Notepad++'s position, with its label, in `src/app/menus/<menu>.rs`.
   4. Add a `CATALOG` row in `src/shortcuts.rs` with Notepad++'s default chord, its `cmd-` mirror on the same row. That makes it rebindable in the Shortcut Mapper.
   5. Add it to the toolbar (`src/ui/toolbar.rs`) and context menus (`src/ui/context_menu.rs`, `src/ui/tab_context_menu.rs`) where Notepad++ has it.
   6. Make keyboard-driven editor edits macro-recordable: a `MacroEvent` variant (`src/macros.rs`), `crate::macros::record(..)` in the handler, and a replay arm in `replay_event` (`src/workspace/macros.rs`).
   7. Head the module with the Notepad++ source it mirrors, as the existing modules do: `// PowerEditor/src/NppCommands.cpp:2149 — IDM_EDIT_BLOCK_COMMENT (toggle)`.

   An item whose behaviour isn't built yet says so through `set_status`, so the user sees why nothing happened.
2. **Cross-platform from one source.** Notepad++ is Windows-only; Notepad Studio ships on macOS, Linux and Windows. Windows and Linux get Notepad++'s chord; macOS gets the `cmd-` mirror, or the Mac convention where Notepad++'s chord collides with one (Go to Line is `Cmd-L` because `Cmd-G` is Find Next). Paths come from `dirs`. OS-specific code sits behind `#[cfg(target_os = ..)]` with an arm for each of the three. CI builds all three (`.github/workflows/ci.yml`), so a `cfg` arm you can't compile locally still has to be right.
3. **Text fidelity.** Encoding (UTF-8, UTF-8 BOM, UTF-16 LE/BE, Latin-1) and line ending (CRLF, LF, CR) are per-buffer data. A file opened and saved without edits comes back byte-identical, BOM included. Test new text paths with CRLF, a BOM, UTF-16 and multi-byte characters.
4. **Clean-room: Notepad++ is GPL-3.0, Notepad Studio is Apache-2.0.** Read Notepad++ for *behaviour*: menu order, labels, default chords, dialog layout, edge cases. Write the Rust from scratch and cite what you studied by file and line. Copying Notepad++ code, or translating it line by line, puts GPL code into an Apache-2.0 product; treat its bundled themes, icons and language files the same way. `tools/gen_themes/gen_themes.py` shows the boundary: it reads `stylers.model.xml` only for lexer and style identifiers, and every palette is original. Third-party code and assets need a permissive, Apache-2.0-compatible licence, keep their licence file beside them, and get an entry in `NOTICE` in the same change. Every change under `vendor/gpui` is listed in `NOTICE` (Apache-2.0 §4(b)), and the vendored tree stays free of GPL and AGPL code.
5. **Tests are the gate.** Every change comes with tests: a `#[cfg(test)] mod tests` beside the code, or an integration test in `tests/`. Keep logic testable without a window: pure functions over `&str` and plain data (the `src/edit_ops/` transforms, `src/text/`, `src/style_model/`), with the `Workspace` handler a thin call into them.
6. **The UI is thin and themed.** UI colours come from `crate::theme::Theme` and syntax colours from `style_model`, never from hex literals in UI code. Prefer `rems(..)` for UI sizes: zoom (`Cmd/Ctrl +/-/0`) scales rems, not `px`. Labels, grouping and dialog layout follow Notepad++.
7. **Verify UI changes visually.** There is no headless snapshot harness: build, run, exercise the change and look at it. `notepad-studio <path>` opens a file on start. Two traps:
   - **Single instance.** While any Notepad Studio is running, `cargo run` hands your launch to that copy and exits 0 (`src/single_instance.rs`). Quit the other copy first.
   - **Shared state.** Dev builds read and write the same `<config>/Notepad Studio/` (settings, session, shortcuts, macros) as an installed copy.

   Screenshots and sample files show no personal data. The title bar and tabs carry full paths, so open samples from a neutral directory.
8. **Performance is a feature.** Only visible lines shape and paint; keep it that way. Long work (Find in Files, large reads, parsing) runs on the background executor (`cx.background_spawn`) with progress and Cancel, never on the UI thread. Files over 10 MB stream in 1 MB chunks, over 2 GB ask first, and highlighting stops above 50 MB (constants in `src/io_stream.rs`). Time heavy changes in a `--release` build on a file above the streaming threshold, and record before/after numbers in the dev log.
9. **The plugin ABI is a contract.** Any layout change in `crates/notepad-studio-plugin/src/ffi.rs` bumps `PLUGIN_API_VERSION`; the host skips plugins built against another version. Calls arriving from a plugin are hostile input under *Never crash*.

## 4. Picking work

Priorities: crashes and data loss first, then low-hanging Notepad++ parity, then the long tail.

1. `README.md`: the **Status** line ("Currently in flight") and unchecked roadmap boxes.
2. `log/devlog.md`: the "Still open" bullets of recent entries.
3. Parity gaps: walk Notepad++'s menu tree (`Notepad_plus.rc`, `IDR_M30_MENU`) against `src/app/menus/`, and find items that only report "not yet implemented" (`rg "not yet implemented" src`). Low-hanging fruit is usually a missing item whose logic already exists in `edit_ops`, `text` or `workspace`.

## 5. Before you finish a task

```sh
cargo build
cargo test -p notepad-studio
cargo clippy -p notepad-studio --all-targets   # no new warnings in code you touched
cargo build -p sample-uppercase-plugin         # if you touched the plugin ABI or host API
```

**Windows needs `clang-cl` (LLVM).** The tree-sitter grammar build scripts pass `-Wno-unused-parameter`, which MSVC's `cl.exe` rejects (`D8021`). Point the C compiler at clang-cl for the cargo command, as CI does: `$env:CC='clang-cl'; $env:CXX='clang-cl'` in PowerShell, `CC=clang-cl CXX=clang-cl cargo …` in Git Bash.

**Format only what you write.** Most files are not rustfmt-clean, so match the surrounding style by hand; running `cargo fmt` on the crate buries your change under reformatting.

Then update the record:

- A user-visible change gets a line in `change.log` under the current version.
- A shipped roadmap item gets its `README.md` box ticked and the Status line updated.
- Append a terse entry to `log/devlog.md`: what landed, numbers, what's still open. Sessions can end abruptly (crashes, context limits), so the dev log plus a green tree is how the next agent picks up. Keep the tree building at every step.

Commit messages follow Conventional Commits (`feat:`, `fix:`, `docs:`). Commits and PRs carry no AI-tool attribution or co-author trailers.

## 6. Parallel agents

- Use your own target dir (`CARGO_TARGET_DIR=target/agent-<name>`) to avoid the Cargo build lock. A fresh dir compiles vendored gpui from scratch (a few minutes). Delete the `target/agent-*` dirs of finished agents.
- Edit only the files you own. Shared files get small, surgical edits; re-read before editing: `src/workspace/render.rs` (the `on_action` list), `src/shortcuts.rs` (`CATALOG`), `src/app/menus/*.rs`, `src/actions/*.rs`, `src/macros.rs` and `src/workspace/macros.rs`, `src/workspace/mod.rs`, `src/main.rs`.
- Put a new command in a **new module** (`src/edit_ops/<op>.rs` holding its `impl Workspace` block) rather than growing a shared file.
- If someone else's in-progress edit breaks the build, wait and retry; leave their files to them.
- Keep `Cargo.toml` valid at all times: one broken manifest stops every agent's build. Rewrite it atomically: write a temp file outside the repo, then move it into place.

## 7. Where things are tracked

- `README.md`: status, shipped features, roadmap checkboxes.
- `change.log`: user-facing notes per version, shown in the app after an update.
- `Cargo.toml` `version`: the app version (`consts::VERSION`). Bumping it re-opens `change.log` for every user on next launch.
- `.github/workflows/release.yml`: pushing a `v*` tag builds macOS (arm64, x86_64), Linux and Windows artifacts and publishes the GitHub Release. macOS builds are unsigned.
- `NOTICE`: vendored gpui provenance and modifications.
- `log/` (local, gitignored): the dev log.

## 8. Keeping persisted state loadable

Settings, session, window placement, recent files, shortcuts and macros persist as TOML under `<config>/Notepad Studio/`. Users upgrade with these files in place, so every release must read what the previous one wrote:

- A new field on a persisted struct gets `#[serde(default)]`, so older files still load.
- A file that fails to parse loads as defaults and startup continues, as the existing loaders do.
- Renaming or removing a field keeps the old name readable (`#[serde(alias = "..")]`).
- `shortcuts.toml` keys overrides by action type name (`ActionEntry.id`), and `macros.toml` stores `MacroEvent` variant names. Renaming either orphans what users saved; keep the old name readable.
- Themes stay in Notepad++'s stylers XML schema, so `style_model` reads a Notepad++ theme and writes one Notepad++ can load.
