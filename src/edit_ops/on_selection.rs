// On Selection ▶ submenu — Open File, Open in New Instance, Open Containing
// Folder (Explorer / Terminal), Search on Internet, Send via Email.
//
// Notepad++ source:
//   PowerEditor/src/NppCommands.cpp:721  — IDM_EDIT_OPENINFOLDER /
//                                          IDM_EDIT_OPENASFILE
//   PowerEditor/src/NppCommands.cpp:794  — IDM_EDIT_SEARCHONINTERNET
//
// Selection-or-word semantics match Notepad++: when the selection is empty
// the action operates on the word under the caret (`word_range_at` — the
// same classifier used for double-click word select).

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::Command;

use gpui::{Context, Window};
use ropey::Rope;

use crate::actions::*;
use crate::edit_ops::util::clamp_byte_range;
use crate::settings::{self, SearchEngineChoice};
use crate::workspace::Workspace;

impl Workspace {
    pub(crate) fn open_selected_file(
        &mut self,
        _: &EditOpenSelectedFile,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = self.selection_or_word_text() else {
            self.set_status("No selection");
            cx.notify();
            return;
        };
        let Some(path) = self.resolve_selected_path(&text) else {
            self.set_status(format!("Path not found: {text}"));
            cx.notify();
            return;
        };
        if !path.exists() {
            self.set_status(format!("Path not found: {}", path.display()));
            cx.notify();
            return;
        }
        if path.is_dir() {
            // Directory — let the OS file manager show it.
            launch_file_manager(&path);
            self.set_status(format!("Opened folder {}", path.display()));
            cx.notify();
            return;
        }
        self.open_path(path, cx);
    }

    pub(crate) fn open_selected_in_new_instance(
        &mut self,
        _: &EditOpenSelectedInNewInstance,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = self.selection_or_word_text() else {
            self.set_status("No selection");
            cx.notify();
            return;
        };
        let Some(path) = self.resolve_selected_path(&text) else {
            self.set_status(format!("Open in New Instance: file not found: {text}"));
            cx.notify();
            return;
        };
        if !path.is_file() {
            self.set_status(format!(
                "Open in New Instance: file not found: {}",
                path.display()
            ));
            cx.notify();
            return;
        }
        if let Err(error) = crate::app::window::open_editor_window_with_initial_path(path, cx) {
            self.set_status(format!("Could not open a destination window: {error}. Try Open in New Instance again."));
        }
        cx.notify();
    }

    pub(crate) fn open_containing_folder(
        &mut self,
        _: &EditOpenContainingFolder,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = self.selection_or_word_text() else {
            self.set_status("No selection");
            cx.notify();
            return;
        };
        let Some(path) = self.resolve_selected_path(&text) else {
            self.set_status(format!("Path not found: {text}"));
            cx.notify();
            return;
        };
        if !path.exists() {
            self.set_status(format!("Path not found: {}", path.display()));
            cx.notify();
            return;
        }
        reveal_in_file_manager(&path);
        self.set_status(format!("Revealed {}", path.display()));
        cx.notify();
    }

    pub(crate) fn open_containing_folder_in_terminal(
        &mut self,
        _: &EditOpenContainingFolderInTerminal,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = self.selection_or_word_text() else {
            self.set_status("No selection");
            cx.notify();
            return;
        };
        let Some(path) = self.resolve_selected_path(&text) else {
            self.set_status(format!("Path not found: {text}"));
            cx.notify();
            return;
        };
        let dir = if path.is_dir() {
            path.clone()
        } else if let Some(parent) = path.parent() {
            parent.to_path_buf()
        } else {
            self.set_status(format!("No containing folder for {}", path.display()));
            cx.notify();
            return;
        };
        if !dir.exists() {
            self.set_status(format!("Folder not found: {}", dir.display()));
            cx.notify();
            return;
        }
        launch_terminal_in(&dir);
        self.set_status(format!("Opened terminal in {}", dir.display()));
        cx.notify();
    }

    pub(crate) fn search_selection_on_internet(
        &mut self,
        _: &EditSearchSelectionOnInternet,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = self.selection_or_word_text() else {
            self.set_status("No selection");
            cx.notify();
            return;
        };
        let snap = settings::snapshot();
        let url = url_for(snap.search_engine, &snap.search_engine_custom_url, &text);
        match crate::shell::open_url(&url) {
            Ok(()) => self.set_status(format!("Searching: {text}")),
            Err(error) => self.set_status(format!("Could not open link: {error}")),
        }
        cx.notify();
    }

    pub(crate) fn send_selection_via_email(
        &mut self,
        _: &EditSendSelectionViaEmail,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = self.selection_or_word_text() else {
            self.set_status("No selection");
            cx.notify();
            return;
        };
        let body = encode_uri_component(&text);
        let url = format!("mailto:?body={body}");
        match crate::shell::open_url(&url) {
            Ok(()) => self.set_status("Composing email"),
            Err(error) => self.set_status(format!("Could not open link: {error}")),
        }
        cx.notify();
    }

    /// Selection text, falling back to the word under the caret when the
    /// selection is empty. Returns `None` only when the buffer is empty or
    /// the resolved range is whitespace-only (NPP treats that as "no
    /// target"). Trims whitespace and surrounding `< >` / `" "` so things
    /// like `<https://example.com>` and `"path/to/file"` work.
    fn selection_or_word_text(&self) -> Option<String> {
        let sel = self.selection();
        let range = if sel.is_empty() {
            self.word_range_at(sel.head)
        } else {
            sel.ordered()
        };
        target_text(&self.buffer().rope, range)
    }

    /// Turn the selection text into a real path. Absolute paths pass through.
    /// Relative paths resolve against the active tab's directory; for
    /// untitled tabs, against the process CWD. Tilde-prefixed paths
    /// (`~/foo`) expand to `$HOME/foo`.
    fn resolve_selected_path(&self, text: &str) -> Option<PathBuf> {
        let raw = text;
        let expanded = if let Some(rest) = raw.strip_prefix("~/") {
            dirs::home_dir().map(|h| h.join(rest))
        } else if raw == "~" {
            dirs::home_dir()
        } else {
            None
        };
        let candidate = expanded.unwrap_or_else(|| PathBuf::from(raw));
        if candidate.is_absolute() {
            return Some(candidate);
        }
        let base = self
            .buffer()
            .file_path
            .as_ref()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .or_else(|| std::env::current_dir().ok())?;
        Some(base.join(candidate))
    }
}

/// Longest target Notepad++ hands on, in UTF-16 units: `CURRENTWORD_MAXLENGTH`
/// (RunDlg.h:25) is 2048 wide chars including the terminator, and the
/// selected text is cut to fit (`wcsncpy_s(.., _TRUNCATE)`,
/// NppBigSwitch.cpp:1065).
const TARGET_MAX_UTF16: usize = 2047;

/// The text in `range`, cut to `TARGET_MAX_UTF16` and trimmed of
/// whitespace and surrounding brackets and quotes. The range comes from the
/// selection, so it is clamped and floored to char boundaries before
/// slicing; only the kept prefix is copied, never the whole selection.
fn target_text(rope: &Rope, range: Range<usize>) -> Option<String> {
    let range = clamp_byte_range(rope, range);
    if range.is_empty() {
        return None;
    }
    let slice = rope.get_byte_slice(range)?;
    let mut raw = String::new();
    let mut units = 0usize;
    for ch in slice.chars() {
        units = units.saturating_add(ch.len_utf16());
        if units > TARGET_MAX_UTF16 {
            break;
        }
        raw.push(ch);
    }
    let trimmed = raw
        .trim()
        .trim_matches(|c: char| matches!(c, '<' | '>' | '"' | '\'' | '`' | '(' | ')'))
        .trim()
        .to_string();
    (!trimmed.is_empty()).then_some(trimmed)
}

// --- Per-platform launchers ---------------------------------------------

#[cfg(target_os = "macos")]
fn launch_file_manager(path: &Path) {
    let _ = Command::new("open").arg(path).spawn();
}

#[cfg(target_os = "linux")]
fn launch_file_manager(path: &Path) {
    let _ = Command::new("xdg-open").arg(path).spawn();
}

#[cfg(target_os = "windows")]
fn launch_file_manager(path: &Path) {
    let _ = Command::new("explorer.exe").arg(path).spawn();
}

#[cfg(target_os = "macos")]
fn reveal_in_file_manager(path: &Path) {
    // `open -R <file>` reveals the file selected in Finder.
    let _ = Command::new("open").arg("-R").arg(path).spawn();
}

#[cfg(target_os = "linux")]
fn reveal_in_file_manager(path: &Path) {
    // No portable "select this file" — best we can do is open the parent.
    let dir = path.parent().unwrap_or(path);
    let _ = Command::new("xdg-open").arg(dir).spawn();
}

#[cfg(target_os = "windows")]
fn reveal_in_file_manager(path: &Path) {
    // `explorer.exe /select,<path>` opens the parent and highlights the file.
    let arg = format!("/select,{}", path.display());
    let _ = Command::new("explorer.exe").arg(arg).spawn();
}

#[cfg(target_os = "macos")]
fn launch_terminal_in(dir: &Path) {
    let _ = Command::new("open")
        .arg("-a")
        .arg("Terminal")
        .arg(dir)
        .spawn();
}

#[cfg(target_os = "linux")]
fn launch_terminal_in(dir: &Path) {
    // Honour $TERMINAL when set; otherwise probe the common emulators in
    // the order most distros install them.
    if let Ok(term) = std::env::var("TERMINAL") {
        if Command::new(&term).current_dir(dir).spawn().is_ok() {
            return;
        }
    }
    for term in [
        "x-terminal-emulator",
        "gnome-terminal",
        "konsole",
        "xfce4-terminal",
        "alacritty",
        "kitty",
        "wezterm",
        "xterm",
    ] {
        if Command::new(term).current_dir(dir).spawn().is_ok() {
            return;
        }
    }
}

#[cfg(target_os = "windows")]
fn launch_terminal_in(dir: &Path) {
    let _ = Command::new("cmd")
        .args(["/C", "start", "cmd", "/K", "cd", "/D"])
        .arg(dir)
        .spawn();
}

// --- search-engine URL templates ---------------------------------------

/// Build the search URL the browser is opened with. Mirrors NPP
/// `Notepad_plus::launchSearchOnInternet` (NppCommands.cpp:794): each engine
/// has a fixed query template; `Custom` uses `search_engine_custom_url` with
/// `$(CURRENT_WORD)` as the term placeholder. `Custom` with an empty / missing
/// template falls back to Google so the menu item still works after the user
/// selects Custom but hasn't yet entered a URL.
pub(crate) fn url_for(choice: SearchEngineChoice, custom: &str, query: &str) -> String {
    let q = encode_query_form(query);
    match choice {
        SearchEngineChoice::DuckDuckGo => format!("https://duckduckgo.com/?q={q}"),
        SearchEngineChoice::Google => format!("https://www.google.com/search?q={q}"),
        SearchEngineChoice::Bing => format!("https://www.bing.com/search?q={q}"),
        SearchEngineChoice::Yahoo => format!("https://search.yahoo.com/search?p={q}"),
        SearchEngineChoice::StackOverflow => {
            format!("https://stackoverflow.com/search?q={q}")
        }
        SearchEngineChoice::Custom => {
            let trimmed = custom.trim();
            if trimmed.is_empty() {
                format!("https://www.google.com/search?q={q}")
            } else if trimmed.contains("$(CURRENT_WORD)") {
                trimmed.replace("$(CURRENT_WORD)", &q)
            } else {
                // Template lacks the placeholder — append the query as a
                // best-effort fallback rather than silently dropping it.
                if trimmed.contains('?') {
                    format!("{trimmed}&q={q}")
                } else {
                    format!("{trimmed}?q={q}")
                }
            }
        }
    }
}

// --- URL encoding -------------------------------------------------------

/// RFC 3986 unreserved-set guard. Everything else gets percent-escaped.
fn is_unreserved(c: u8) -> bool {
    matches!(c, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~')
}

/// `application/x-www-form-urlencoded` style — used for query strings (e.g.
/// Google search). Spaces become `+`, everything else outside the unreserved
/// set is `%XX`.
fn encode_query_form(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b == b' ' {
            out.push('+');
        } else if is_unreserved(b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

/// Plain RFC 3986 percent-encoding — used inside `mailto:` URIs where
/// `+` would be interpreted literally rather than as a space.
fn encode_uri_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if is_unreserved(b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str, range: Range<usize>) -> Option<String> {
        target_text(&Rope::from_str(s), range)
    }

    #[test]
    fn target_text_clamps_stale_and_mid_char_ranges() {
        // a=0, é=1..3, 中=3..6
        assert_eq!(text("aé中", 2..5).as_deref(), Some("é"));
        assert_eq!(text("aé中", 4..99).as_deref(), Some("中"));
        assert_eq!(text("aé中", Range { start: 5, end: 2 }).as_deref(), Some("é"));
        assert_eq!(text("", 0..4), None);
        assert_eq!(text("ab", 7..9), None);
    }

    #[test]
    fn target_text_trims_space_line_ends_and_brackets() {
        let cases: &[(&str, Option<&str>)] = &[
            ("", None),
            ("   ", None),
            ("\r\n", None),
            ("  \t\r", None),
            ("<https://example.com>", Some("https://example.com")),
            ("\"C:\\dir\\é.txt\"\r\n", Some("C:\\dir\\é.txt")),
            ("  ~/notes/中.md\r", Some("~/notes/中.md")),
            ("(👨‍👩‍👧 🇯🇵)\n", Some("👨‍👩‍👧 🇯🇵")),
            ("a\r\nb\rc\n", Some("a\r\nb\rc")),
        ];
        for (input, want) in cases {
            let got = text(input, 0..input.len());
            assert_eq!(got.as_deref(), *want, "input {input:?}");
        }
    }

    #[test]
    fn target_text_is_cut_to_the_notepad_plus_plus_word_length() {
        let long = "a".repeat(3000);
        assert_eq!(text(&long, 0..long.len()).map(|s| s.len()), Some(2047));
        // 😀 is two UTF-16 units: 1023 of them fill 2046, a 1024th would
        // pass 2047.
        let wide = "😀".repeat(1500);
        let got = text(&wide, 0..wide.len()).unwrap_or_default();
        assert_eq!(got.chars().count(), 1023);
        let short = "é".repeat(10);
        assert_eq!(text(&short, 0..short.len()), Some(short));
    }
}
