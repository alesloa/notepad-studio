# Notepad Studio

A lightweight, cross-platform text editor inspired by Notepad++, written in Rust. Notepad++ feel, AI features optional, no bloat.

![Notepad Studio, a lightweight cross-platform text editor for macOS, Windows, and Linux](docs/screenshot.png)

**Status:** v0.0.6. The core editor works on macOS, Linux and Windows.
Currently in flight: making it behave like Notepad++ in the details. That means
checking input methods (IME) and auto-completion, fixing focus bugs, speeding up
very large edits, replaying old macros that save files, and filling the gaps in
the [menu inventory](docs/notepadpp-parity-inventory.md).

## What it does

- Tabs, sessions that come back after a restart, recent files, and drag a file onto the window to open it.
- Find and Replace with regex, Find in Files, Mark and bookmarks.
- Syntax highlighting for 25 languages, bracket matching, auto-indent and comment toggling.
- Word wrap, show whitespace, zoom and full screen (F11).
- Column (block) selection and multi-cursor editing.
- Macro record and replay, with saved macros in the Macro menu.
- A Shortcut Mapper, so you can change the shortcut of any built-in menu command.
- Opens big files without freezing. Files over 10 MB load in the background with a Cancel button.
- Native plugins (see [Plugins](#plugins)).
- English spell and grammar check in plain text and Markdown.
- MD5, SHA-1, SHA-256 and SHA-512 hashes of text, files or a selection.

Slow jobs like sorting lines, indenting big selections, reloading and Find All
run in the background, so the window keeps working while they run.

More detail on some features:

- [Regex replacement](docs/regex-replacement.md)
- [Search macros](docs/search-macros.md)
- [Copy Bookmarked Lines](docs/bookmark-copy.md) and [bookmark editing](docs/bookmark-edits.md)
- [Spell and grammar check](docs/proofreading.md)
- [Hash Tools](docs/hash-tools.md)
- [API completion definitions](docs/api-completion.md): function names and parameter hints per language
- [Save protection](docs/windows-save-durability.md): a backup copy exists before a file is overwritten

## Goals

- **Lightweight:** small binary, fast cold start, low memory.
- **Cross-platform:** macOS, Linux, Windows.
- **Familiar:** Notepad++-style tabs, find/replace, encoding and line-ending
  toggles, syntax highlighting, plugins.
- **Open source:** Apache-2.0.

## Built with

- [`gpui`](https://gpui.rs/), the GPU-accelerated UI framework from Zed Industries
  (Apache-2.0). Notepad Studio uses gpui as a UI library; all editor logic, layout,
  buffer handling, file I/O, and UX are original to this project.
- [`ropey`](https://crates.io/crates/ropey) for the text buffer.
- [`rfd`](https://crates.io/crates/rfd) for native file dialogs.
- [`regex`](https://crates.io/crates/regex) for find and replace.
- [`dirs`](https://crates.io/crates/dirs) for config and log paths on each OS.
- [`serde`](https://crates.io/crates/serde) and [`toml`](https://crates.io/crates/toml) to save settings.

## Platform support

| Platform | Status         | Notes |
| -------- | -------------- | ----- |
| macOS    | tested daily   | Native menu bar, dock-icon reopen. Default line ending: LF. |
| Linux    | builds         | X11 and Wayland. Default line ending: LF. Closing the last window quits the app. |
| Windows  | builds         | Default line ending: CRLF. Closing the last window quits the app. `File ▸ Exit` and `Alt+F4` both work. |

Shortcuts use `Ctrl` on Linux and Windows and `Cmd` on macOS.

If the app crashes, it writes a report to:

- macOS: `~/Library/Logs/Notepad Studio/panic.log`
- Linux: `$XDG_STATE_HOME/Notepad Studio/panic.log`
- Windows: `%LOCALAPPDATA%\Notepad Studio\Logs\panic.log`

## Installing a release

Download a build from the
[Releases page](https://github.com/alesloa/notepad-studio/releases):

| Platform | Artifact | Contents |
| -------- | -------- | -------- |
| macOS (Apple Silicon) | `Notepad-Studio-macos-arm64.zip`   | `Notepad Studio.app` |
| macOS (Intel)         | `Notepad-Studio-macos-x86_64.zip`  | `Notepad Studio.app` |
| Linux x86_64          | `Notepad-Studio-linux-x86_64.tar.gz` | `notepad-studio` binary |
| Windows x86_64        | `Notepad-Studio-windows-x86_64.zip`  | `notepad-studio.exe` |

Icons and themes are built into the binary. On Linux and Windows you only need
the one executable.

> **macOS:** the app is **not signed yet**, so macOS blocks the first launch.
> Right-click the app and choose **Open**, then confirm. Or move it to
> `/Applications` and run:
>
> ```sh
> xattr -dr com.apple.quarantine "/Applications/Notepad Studio.app"
> ```

## Build & run

```sh
cargo build            # debug build
cargo run              # build + run (debug)
cargo build --release  # optimised build
cargo run --release    # build + run (optimised)
cargo check            # type-check only, no binary
```

`gpui` is included in the repo under [`vendor/gpui`](vendor/gpui), so nothing
is downloaded for it. The first build compiles it from source and takes a few
minutes. Later builds are fast.

Release builds (`cargo bundle`) include the app icon. To make new icons, see
[`docs/icons.md`](docs/icons.md).

## Roadmap

### v0.0.1: scaffold ✅
- [x] Window, native menu bar, dark theme, status bar
- [x] File / Edit / View / Help menus
- [x] Read-only text view with line numbers

### v0.0.2: selection and themes ✅
- [x] Click and drag to select; shift-click to extend
- [x] `Cmd/Ctrl-A` select all, `Cmd/Ctrl-C` copy
- [x] Light and dark theme (`View ▸ Toggle Theme`)

### v0.0.3: editing ✅
- [x] Typing, including input methods (IME)
- [x] Backspace, Delete, Enter, Tab (4 spaces, configurable)
- [x] Arrow keys, Home/End, jump to start or end of the document
- [x] Jump by word with Alt or Ctrl + Left/Right
- [x] Shift + any movement key extends the selection
- [x] Up/Down remember the column you started in
- [x] Blinking caret
- [x] Cut and Paste
- [x] Undo and Redo (`Cmd+Z`, `Cmd+Shift+Z`, `Ctrl+Y`)
- [x] Unsaved changes show a ● in the title bar

### v0.0.3: cross-platform polish ✅
- [x] Drag-select keeps going when the mouse leaves the window
- [x] `File ▸ New Window` (`Cmd/Ctrl+Shift+N`) and macOS dock-icon reopen
- [x] `Ctrl` versions of every `Cmd` shortcut
- [x] macOS keeps running after the last window closes; Linux and Windows quit
- [x] `File ▸ Exit`
- [x] New files use CRLF on Windows and LF elsewhere
- [x] Crash reports saved to a log file
- [x] Window size and position remembered

### v0.0.4: files and tabs ✅
- [x] Tabs, with a tab strip at the top
- [x] `Cmd/Ctrl-N` new tab, `Cmd/Ctrl-W` close tab
- [x] `Ctrl-Tab` / `Ctrl-Shift-Tab` and `Ctrl-PageDown` / `Ctrl-PageUp` switch tabs
- [x] `Cmd/Ctrl-1` to `Cmd/Ctrl-9` jump to a tab
- [x] `Cmd/Ctrl-Shift-W` closes every tab
- [x] `Cmd/Ctrl-Shift-T` reopens a closed tab (last 20)
- [x] Closing the last tab closes the window
- [x] Open, with line-ending detection (LF / CRLF / CR)
- [x] Save (`Cmd/Ctrl-S`) and Save As (`Cmd-Shift-S` on mac, `Ctrl-Alt-S` on Linux/Windows)
- [x] Save All (`Cmd-Opt-S` on mac, `Ctrl-Shift-S` on Linux/Windows), including untitled tabs
- [x] Save a Copy As, Rename, Move to Trash / Recycle Bin, open the containing folder or a terminal there
- [x] Save / Don't Save / Cancel prompt when closing unsaved tabs
- [x] Title bar and tabs show the file path and unsaved state
- [x] Drag a file onto the window to open it
- [x] Recent Files (last 10)
- [x] Each window remembers its own size and position
- [x] Drag tabs to reorder them
- [x] View ▸ Tab: select, move and color tabs

### v0.0.5: Notepad++ basics ✅
- [x] Find bar that searches as you type (`Cmd/Ctrl-F`)
- [x] Find Next / Previous (`F3` / `Shift+F3`, or `Enter` / `Shift+Enter` in the find box)
- [x] Match case, whole word and regex options
- [x] Match counter ("X of Y")
- [x] All matches highlighted, current match brighter
- [x] Replace and Replace All (`Ctrl-H`, or `Cmd-Opt-F` on macOS); Replace All undoes in one step
- [x] Go to Line (`Ctrl-G`, or `Cmd-L` on macOS)
- [x] Find in Files (`Cmd/Ctrl+Shift+F`) with a results panel; click a result to open it
- [x] Extended search mode: `\n`, `\r`, `\t` and other escapes in normal find and replace
- [x] Bookmarks: `F2` toggle, `Ctrl+F2` next, `Shift+F2` previous, `Cmd/Ctrl+Shift+F2` clear all
- [x] [Copy Bookmarked Lines](docs/bookmark-copy.md)
- [x] [Regex replacement](docs/regex-replacement.md) with numbered and named groups
- [x] Click the line-ending label in the status bar to switch CRLF / LF / CR
- [x] Click the encoding label to switch UTF-8 / UTF-8 BOM / UTF-16 LE/BE / Latin-1

### v0.0.6: viewing comfort
- [x] Scrolling only draws the visible lines, so long files stay fast
- [x] Vertical and horizontal scrollbars
- [x] Mouse wheel and trackpad scrolling
- [x] PageUp / PageDown (Shift to select)
- [x] The view follows the cursor
- [x] Drag-select scrolls past the edge of the view
- [x] Fixed a crash when closing an unsaved tab
- [x] Word wrap (`View ▸ Word Wrap`)
- [x] Right-click menu: Undo/Redo, Cut/Copy/Paste/Delete, Select All, Toggle Bookmark, Go to Line, Find
- [x] Show whitespace (`View ▸ Show Whitespace`)
- [x] Zoom in and out with `Cmd/Ctrl +/-/0`, remembered between runs
- [x] Full screen (`F11`)

### v0.0.7: edit commands (Notepad++ parity)
- [x] Duplicate Line / Selection (`Cmd/Ctrl+D`)
- [x] Move Line Up / Down (`Ctrl+Shift+Up/Down` on Linux/Windows, `Alt+Shift+Up/Down` on mac)
- [x] Delete Line (`Cmd/Ctrl+Shift+L`)
- [x] UPPERCASE (`Cmd/Ctrl+Shift+U`) and lowercase (`Cmd/Ctrl+U`)
- [x] Toggle Block Comment (`Cmd+/` on mac, `Ctrl+Q` on Linux/Windows)
- [x] Toggle Stream Comment (`Cmd+Shift+/` on mac, `Ctrl+Shift+Q` on Linux/Windows)
- [x] Insert Blank Line Above / Below (`Cmd/Ctrl+Alt+Enter` / `Cmd/Ctrl+Shift+Enter`)
- [x] Trim Trailing Whitespace (Edit menu)
- [x] Comment toggles use the right comment style for each language

### v0.1.0: syntax and languages ✅
- [x] Syntax highlighting: Rust, Python, C, C#, JS/TS/TSX, JSON, Markdown, Bash/Sh/Zsh/Ksh, PowerShell, Go, Java, Swift, Ruby, PHP, C++, Lua, HTML, CSS, SCSS, YAML, TOML, XML, SQL. Also detects files like `Rakefile` and `Gemfile`, and scripts by their `#!` line. Kotlin and Dockerfile are not supported yet.
- [x] Language menu to pick the language for each tab
- [x] Bracket matching that skips brackets inside strings and comments
- [x] Auto-indent after Enter, with extra indent after `{`, `[`, `(`
- [x] Indent and unindent lines from the Edit menu; large selections run in the background

### Settings ✅
- [x] Shortcut Mapper (`Settings ▸ Shortcut Mapper`) for every menu command, with conflict warnings. Shortcuts for named Run commands, saved macros and plugin commands are still open.

### Polish ✅
- [x] Double-click selects a word, triple-click a line, four clicks select all
- [x] Notepad++-style toolbar above the tabs

### Beyond
- [x] Run dialog (`F5` or `Run ▸ Run...`), with saved commands in the Run menu. Shortcuts for saved commands are still open.
- [x] Column selection: Alt-drag a block, then type, delete, cut, copy or paste across every line at once. `Cmd/Ctrl+Alt+Shift+Arrow` extends the block. Block selection with word wrap on, and the Column Editor dialog, are still open.
- [x] Macros: `Cmd/Ctrl+Shift+R` to record, `Cmd/Ctrl+Shift+P` to play, `Cmd/Ctrl+Shift+M` to run N times. Saved macros appear in the Macro menu.
- [x] [Search macros](docs/search-macros.md) remember the search text and options
- [x] Edit ▸ Copy to Clipboard ▸ Copy All Filenames / Copy All File Paths
- [x] Plugins (see [Plugins](#plugins))
- [x] Large files: over 10 MB load in the background with progress and Cancel, over 2 GB ask first, and syntax highlighting turns off above 50 MB
- [x] Open tabs, scroll positions, selections and window positions come back after a restart

## Plugins

Notepad Studio loads native plugins from this folder:

| Platform | Path |
| -------- | ---- |
| macOS    | `~/Library/Application Support/Notepad Studio/plugins/` |
| Linux    | `$XDG_CONFIG_HOME/Notepad Studio/plugins/` (defaults to `~/.config/Notepad Studio/plugins/`) |
| Windows  | `%APPDATA%\Notepad Studio\plugins\` |

The folder is created on first launch. To install a plugin, put its `.dylib`
(mac), `.so` (Linux) or `.dll` (Windows) file there and restart Notepad Studio.
Each plugin gets its own submenu in the Plugins menu.
`Plugins ▸ Open Plugins Folder` opens the folder.

There is a sample plugin in
[`crates/sample-uppercase-plugin`](crates/sample-uppercase-plugin). Build it with
`cargo build -p sample-uppercase-plugin --release`, then copy
`target/release/libsample_uppercase_plugin.{dylib,so,dll}` into the plugins folder.

To write your own, use the [`notepad-studio-plugin`](crates/notepad-studio-plugin)
crate. Plugins can read and change the text and the selection, and show a
message in the status bar. Plugins built for a different plugin API version
(currently `PLUGIN_API_VERSION = 1`) are skipped.

> **Trust:** plugins run as native code with the same access as Notepad Studio.
> A bad plugin can read any file you can and run any code your account can. Only install
> plugins you trust. A sandbox for plugins is planned for v2.

## License

Apache-2.0. See [LICENSE](LICENSE).
