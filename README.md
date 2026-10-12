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
