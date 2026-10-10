// Find-in-Files engine — recursive search across a directory tree.
//
// `run_search` is the blocking entry point — caller MUST run it off the gpui
// main thread (see `cx.background_executor().spawn(...)`). The function walks
// the directory with `walkdir`, skips common build/VCS dirs and dotfile dirs,
// caps per-file size, treats non-UTF-8 files as skipped, and returns at most
// `FIND_IN_FILES_RESULT_CAP` matches before truncating.

use std::ops::Range;
use std::path::{Path, PathBuf};

use regex::Regex;
use walkdir::WalkDir;

use crate::text::extended_escape::interpret_extended;

/// Cap on total matches per search. The UI footers a "(truncated...)" row
/// when this fires so the user knows to refine.
pub(crate) const FIND_IN_FILES_RESULT_CAP: usize = 1000;

/// Per-file size limit. Files above this are skipped silently — they're
/// almost certainly binary, generated, or vendored data and aren't useful for
/// a content search.
pub(crate) const FIND_IN_FILES_FILE_SIZE_LIMIT: u64 = 1_000_000;

/// Longest slice of a line a result row keeps, in bytes from the line
/// start. Notepad++ cuts result lines at `SC_SEARCHRESULT_LINEBUFFERMAXLENGTH
/// - 4` (scintilla/include/Scintilla.h:1358, PowerEditor/src/
/// ScintillaComponent/FindReplaceDlg.cpp:3581). Without a cap, 1000 hits on
/// a 1 MB minified line hold 1000 copies of it.
pub(crate) const RESULT_LINE_TEXT_MAX_BYTES: usize = 2048 - 4;

/// Subdirectory names skipped at any depth. A user can still search inside
/// one of these by setting it as the search root explicitly.
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    ".next",
    "dist",
    "build",
    ".venv",
    "venv",
    "__pycache__",
    ".cache",
];

/// Search inputs. `directory` must be an existing directory; `text` may be
/// any string (interpreted as literal, whole-word, or regex per the toggles).
#[derive(Clone)]
pub(crate) struct FindInFilesQuery {
    pub(crate) text: String,
    pub(crate) directory: PathBuf,
    pub(crate) filter: String,
    pub(crate) match_case: bool,
    pub(crate) whole_word: bool,
    pub(crate) regex: bool,
    /// Notepad++'s "Extended" escape mode. When on (and `regex` is off),
    /// `\n` `\r` `\t` `\0` `\\` in `text` are decoded into their literal byte
    /// equivalents before the regex builds. Mutually-exclusive with `regex`.
    pub(crate) extended: bool,
    pub(crate) dot_matches_newline: bool,
    /// Recurse into sub-folders. When false the walker stays at depth 0
    /// and only scans files directly inside `directory`.
    pub(crate) in_subfolders: bool,
    /// Walk into directories whose name starts with `.`. The hard-coded
    /// `SKIP_DIRS` list still applies regardless — `.git`, `node_modules`,
    /// etc. are always skipped because they're "obviously vendored", not
    /// because of dotfile filtering.
    pub(crate) in_hidden_folders: bool,
}

/// One match — `path` is the file's full path, `line` is the 0-based line
/// containing the hit, `line_text` is that line (no trailing newline), and
/// `match_in_line` is the byte span of the match within `line_text`.
/// Click-to-jump re-resolves the absolute byte offset against the loaded
/// rope so CRLF normalization stays correct on Windows.
///
/// The "Find All in All Opened Documents" path reuses this struct against
/// in-memory buffers (no on-disk read). When `tab_idx` is `Some`, click-to-
/// jump activates that tab directly instead of looking up `path` in the tab
/// list. `display_label` overrides the file-name + parent-dir rendering so
/// untitled buffers ("Untitled", "Untitled 2", …) render correctly even
/// though `path` is empty.
#[derive(Clone)]
pub(crate) struct FindResult {
    pub(crate) path: PathBuf,
    pub(crate) line: usize,
    pub(crate) line_text: String,
    pub(crate) match_in_line: Range<usize>,
    /// Set when the result came from an open buffer (Find All in All Opened
    /// Documents). Bypasses the path-based tab lookup on click.
    pub(crate) tab_idx: Option<usize>,
    /// Optional display override for the path column. Used by the all-opened
    /// flow so untitled buffers show "Untitled" instead of an empty cell.
    pub(crate) display_label: Option<String>,
}

/// Outcome surface for the panel — separates the success path from the two
/// expected error states (invalid regex, missing directory) so the UI can
/// show the right message without parsing strings.
pub(crate) enum FindInFilesOutcome {
    Ok {
        results: Vec<FindResult>,
        searched_files: usize,
        truncated: bool,
        elapsed_ms: u64,
    },
    InvalidRegex(String),
    DirectoryMissing,
}

pub(crate) fn build_pattern(q: &FindInFilesQuery) -> Result<Regex, regex::Error> {
    // Apply extended-escape decoding first when the user picked Extended
    // mode. Skipped when `regex` is on — Notepad++ treats the two as
    // mutually-exclusive and the regex engine already speaks `\n`, `\t`, etc.
    let raw_text = if q.extended && !q.regex {
        interpret_extended(&q.text)
    } else {
        q.text.clone()
    };
    let pattern = if q.regex {
        raw_text
    } else if q.whole_word {
        format!(r"\b{}\b", regex::escape(&raw_text))
    } else {
        regex::escape(&raw_text)
    };
    regex::RegexBuilder::new(&pattern)
        .case_insensitive(!q.match_case)
        .multi_line(true)
        .dot_matches_new_line(q.regex && q.dot_matches_newline)
        .build()
}

pub(crate) fn matches_filter(filename: &str, filter: &str) -> bool {
    let trimmed = filter.trim();
    if trimmed.is_empty() || trimmed == "*" || trimmed == "*.*" {
        return true;
    }
    for raw in trimmed.split(|c: char| c == ';' || c.is_whitespace()) {
        let p = raw.trim();
        if p.is_empty() {
            continue;
        }
        if glob_match(p, filename) {
            return true;
        }
    }
    false
}

/// Tiny glob matcher: `*` (any run, including empty) and `?` (single char).
/// Case-insensitive — matches Notepad++ filter behavior.
///
/// Iterative, remembering only the last `*`: O(pattern × name) time and no
/// recursion, so a long pasted filter can neither overflow the stack nor
/// backtrack exponentially.
fn glob_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().flat_map(|c| c.to_lowercase()).collect();
    let n: Vec<char> = name.chars().flat_map(|c| c.to_lowercase()).collect();
    let (mut pi, mut ni) = (0usize, 0usize);
    // Pattern index just past the last `*` seen, and the name index that
    // star currently stops before.
    let mut star: Option<(usize, usize)> = None;
    while let Some(&nc) = n.get(ni) {
        match p.get(pi) {
            Some('*') => {
                star = Some((pi + 1, ni));
                pi += 1;
            }
            Some(&pc) if pc == '?' || pc == nc => {
                pi += 1;
                ni += 1;
            }
            _ => match star {
                // Let the last star swallow one more name char and retry.
                Some((sp, sn)) => {
                    star = Some((sp, sn + 1));
                    pi = sp;
                    ni = sn + 1;
                }
                None => return false,
            },
        }
    }
    p.get(pi..).unwrap_or(&[]).iter().all(|&c| c == '*')
}

/// Walker filter that respects the "in sub-folders" / "in hidden folders"
/// flags. The hard-coded `SKIP_DIRS` list (`.git`, `node_modules`, etc.) is
/// always honoured — those are vendored by definition.
fn skip_entry(entry: &walkdir::DirEntry, in_subfolders: bool, in_hidden: bool) -> bool {
    if entry.depth() == 0 {
        return false;
    }
    // When sub-folder recursion is off, prune every directory below the root.
    // Files at depth 1 (direct children of `directory`) still pass — the
    // walker's filter_entry only blocks descent into pruned dirs, not the
    // sibling files at the same depth.
    if !in_subfolders && entry.file_type().is_dir() && entry.depth() >= 1 {
        return true;
    }
    let Some(name) = entry.file_name().to_str() else {
        return false;
    };
    if entry.file_type().is_dir() {
        if name.starts_with('.') && !in_hidden {
            return true;
        }
        if SKIP_DIRS.contains(&name) {
            return true;
        }
    }
    false
}

/// Walk `query.directory` and collect matches. Blocking — caller's job to
/// keep this off the UI thread.
pub(crate) fn run_search(query: &FindInFilesQuery) -> FindInFilesOutcome {
    if !query.directory.is_dir() {
        return FindInFilesOutcome::DirectoryMissing;
    }
    let re = match build_pattern(query) {
        Ok(r) => r,
        Err(e) => return FindInFilesOutcome::InvalidRegex(format!("Invalid regex: {e}")),
    };

    let started = std::time::Instant::now();
    let mut results: Vec<FindResult> = Vec::new();
    let mut searched_files: usize = 0;
    let mut truncated = false;

    let in_subfolders = query.in_subfolders;
    let in_hidden = query.in_hidden_folders;
    let walker = WalkDir::new(&query.directory)
        .follow_links(false)
        .into_iter()
        .filter_entry(move |e| !skip_entry(e, in_subfolders, in_hidden));

    for entry in walker.flatten() {
        if results.len() >= FIND_IN_FILES_RESULT_CAP {
            truncated = true;
            break;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let Some(filename) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !matches_filter(filename, &query.filter) {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if metadata.len() > FIND_IN_FILES_FILE_SIZE_LIMIT {
            continue;
        }
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let content = match std::str::from_utf8(&bytes) {
            Ok(s) => s,
            Err(_) => continue,
        };
        searched_files += 1;
        match_file(path, content, &re, &mut results);
        if results.len() >= FIND_IN_FILES_RESULT_CAP {
            results.truncate(FIND_IN_FILES_RESULT_CAP);
            truncated = true;
            break;
        }
    }

    let elapsed_ms = started.elapsed().as_millis() as u64;
    FindInFilesOutcome::Ok {
        results,
        searched_files,
        truncated,
        elapsed_ms,
    }
}

fn match_file(path: &Path, content: &str, re: &Regex, out: &mut Vec<FindResult>) {
    collect_results(content, re, path, None, None, out);
}

/// Append a `FindResult` for every non-empty match of `re` in `text`,
/// stopping once `out` holds `FIND_IN_FILES_RESULT_CAP` results. Shared by
/// Find in Files and the Find All in Current / All Opened Documents
/// commands. Returns how many results it added.
///
/// `line` and `match_in_line` are relative to the whole line, so a click
/// lands on the match even when `line_text` is cut at
/// `RESULT_LINE_TEXT_MAX_BYTES`. A match running past its line is clipped
/// to the line end.
pub(crate) fn collect_results(
    text: &str,
    re: &Regex,
    path: &Path,
    tab_idx: Option<usize>,
    display_label: Option<&str>,
    out: &mut Vec<FindResult>,
) -> usize {
    let mut lines = LineWalker::new(text);
    let mut added = 0usize;
    for m in re.find_iter(text) {
        if out.len() >= FIND_IN_FILES_RESULT_CAP {
            break;
        }
        // Skip zero-length matches (e.g. `^`, `\b`) — they'd otherwise spam
        // a result on every line for a pattern like `\b`.
        if m.start() == m.end() {
            continue;
        }
        let (line, line_start) = lines.seek(m.start());
        let content_len = lines.content_end().saturating_sub(line_start);
        let shown_end = text.floor_char_boundary(
            line_start.saturating_add(content_len.min(RESULT_LINE_TEXT_MAX_BYTES)),
        );
        let line_text = text.get(line_start..shown_end).unwrap_or_default().to_string();
        let local_start = m.start().saturating_sub(line_start).min(content_len);
        let local_end = m.end().saturating_sub(line_start).min(content_len).max(local_start);
        out.push(FindResult {
            path: path.to_path_buf(),
            line,
            line_text,
            match_in_line: local_start..local_end,
            tab_idx,
            display_label: display_label.map(str::to_string),
        });
        added += 1;
    }
    added
}

/// Length of the line break starting at `bytes[i]`, or 0. The breaks are
/// the ones ropey counts: LF, VT, FF, CR, CRLF (one break), NEL, LS, PS.
fn line_break_len(bytes: &[u8], i: usize) -> usize {
    match bytes.get(i..).unwrap_or_default() {
        [b'\r', b'\n', ..] => 2,
        [b'\n' | 0x0B | 0x0C | b'\r', ..] => 1,
        [0xC2, 0x85, ..] => 2,
        [0xE2, 0x80, 0xA8 | 0xA9, ..] => 3,
        _ => 0,
    }
}

/// Walks a text forward, tracking the line a byte offset sits on and where
/// that line's content ends, with lines split where ropey splits them so a
/// result's line number is the rope line it lands on once the file is
/// open. Offsets must come in non-decreasing order, as `find_iter` yields
/// them; the whole walk is O(text) with no per-line allocation.
struct LineWalker<'a> {
    bytes: &'a [u8],
    line: usize,
    start: usize,
    pos: usize,
    end: Option<usize>,
}

impl<'a> LineWalker<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            bytes: text.as_bytes(),
            line: 0,
            start: 0,
            pos: 0,
            end: None,
        }
    }

    /// Line index and line start byte of the line holding byte `to`. A byte
    /// inside a line break belongs to the line the break ends.
    fn seek(&mut self, to: usize) -> (usize, usize) {
        let to = to.min(self.bytes.len());
        while self.pos < to {
            let brk = line_break_len(self.bytes, self.pos);
            if brk == 0 {
                self.pos += 1;
                continue;
            }
            if self.pos + brk > to {
                break;
            }
            self.pos += brk;
            self.line += 1;
            self.start = self.pos;
            self.end = None;
        }
        (self.line, self.start)
    }

    /// Byte where the current line's content ends (its break excluded).
    fn content_end(&mut self) -> usize {
        if let Some(end) = self.end {
            return end;
        }
        let mut i = self.pos;
        while i < self.bytes.len() && line_break_len(self.bytes, i) == 0 {
            i += 1;
        }
        self.end = Some(i);
        i
    }
}

/// Live state for the Find-in-Files dialog tab and its inline results panel.
/// One per `Workspace`, preserved across dialog close/reopen so the user's
/// last query restores.
///
/// `open` controls visibility of the *results panel* below the editor; the
/// search dialog tab is governed separately by the dialog's own visibility
/// state. Kicking off a search opens the panel; closing the dialog leaves
/// the panel up so the user can keep clicking results.
pub(crate) struct FindInFilesState {
    pub(crate) open: bool,
    pub(crate) query: String,
    pub(crate) query_cursor: usize,
    pub(crate) query_anchor: usize,
    pub(crate) replacement: String,
    pub(crate) replacement_cursor: usize,
    pub(crate) replacement_anchor: usize,
    pub(crate) filter: String,
    pub(crate) filter_cursor: usize,
    pub(crate) filter_anchor: usize,
    pub(crate) directory: Option<PathBuf>,
    /// Editable string view of the directory — kept separate so the user can
    /// type a path in the dialog without losing the validated `directory`
    /// until they commit. Mirrors `directory.display()` whenever a Browse
    /// pick lands.
    pub(crate) directory_text: String,
    pub(crate) directory_text_cursor: usize,
    pub(crate) directory_text_anchor: usize,
    /// Re-anchor the search root to the active buffer's parent directory on
    /// every dialog open. Notepad++'s "Follow current doc" toggle.
    pub(crate) follow_current_doc: bool,
    /// Recurse into sub-folders. On by default.
    pub(crate) in_subfolders: bool,
    /// Walk into hidden / dotfile directories. Off by default.
    pub(crate) in_hidden_folders: bool,
    pub(crate) match_case: bool,
    pub(crate) whole_word: bool,
    pub(crate) regex: bool,
    /// Notepad++'s "Extended" mode for the FIF search box. `\n` `\r` `\t`
    /// `\0` `\\` literal escapes in the query are decoded before the regex
    /// builds. Mutually-exclusive with `regex`.
    pub(crate) extended: bool,
    /// `. matches newline` companion to the Regular expression radio.
    /// Threaded into `build_pattern`'s `RegexBuilder::dot_matches_new_line`
    /// when `regex` is on.
    pub(crate) dot_matches_newline: bool,
    pub(crate) results: Vec<FindResult>,
    pub(crate) truncated: bool,
    /// "Searched 234 files in 122ms" — set on a successful run; replaced on
    /// each subsequent run.
    pub(crate) status: Option<String>,
    /// Invalid regex / missing directory — presented in the panel header in
    /// the error tint.
    pub(crate) error: Option<String>,
    /// True while a search is in flight.
    pub(crate) searching: bool,
    /// "Find in Projects" — each panel maps to one folder-panel root
    /// (sidebar). NPP allows up to 3; we cap at 3 too. Default: panel 1
    /// on so the search runs immediately if a single root is open. The
    /// Find/Replace buttons stay disabled when no panel is checked.
    pub(crate) project_panel_1: bool,
    pub(crate) project_panel_2: bool,
    pub(crate) project_panel_3: bool,
    /// Dialog transparency group (NPP's "Transparency" box on the FIF tab).
    /// When `transparency_enabled` is on, the dialog window's background
    /// fades to `transparency_level/100` alpha. `transparency_always` picks
    /// "Always" (faded even when focused) vs "On losing focus" (only when
    /// the window is inactive). Default `enabled=true, always=false,
    /// level=80` matches the previously-hardcoded UNFOCUSED_ALPHA=0.80
    /// on-focus-loss behaviour.
    pub(crate) transparency_enabled: bool,
    pub(crate) transparency_always: bool,
    pub(crate) transparency_level: u8,
    /// Header metadata for the NPP-style results panel — frozen at the
    /// moment a search completed so the header text stays consistent even
    /// if the user types a new query into the dialog afterwards.
    pub(crate) last_search_query: String,
    /// `"Normal"`, `"Extended"`, or `"Regular expression"` — formatted to
    /// match NPP's panel header `[Mode]` suffix.
    pub(crate) last_search_mode: String,
    /// Files actually scanned by the last search (for `(N hits in M files
    /// of K searched)`).
    pub(crate) last_search_files_searched: usize,
}

impl FindInFilesState {
    pub(crate) fn new() -> Self {
        Self {
            open: false,
            query: String::new(),
            query_cursor: 0,
            query_anchor: 0,
            replacement: String::new(),
            replacement_cursor: 0,
            replacement_anchor: 0,
            filter: "*".to_string(),
            filter_cursor: 1,
            filter_anchor: 1,
            directory: None,
            directory_text: String::new(),
            directory_text_cursor: 0,
            directory_text_anchor: 0,
            follow_current_doc: false,
            in_subfolders: true,
            in_hidden_folders: false,
            match_case: false,
            whole_word: false,
            regex: false,
            extended: false,
            dot_matches_newline: false,
            results: Vec::new(),
            truncated: false,
            status: None,
            error: None,
            searching: false,
            project_panel_1: true,
            project_panel_2: false,
            project_panel_3: false,
            transparency_enabled: true,
            transparency_always: false,
            transparency_level: 80,
            last_search_query: String::new(),
            last_search_mode: String::new(),
            last_search_files_searched: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run `f` on its own thread and fail if it takes longer than `secs`,
    /// so a matcher that hangs fails the test instead of stalling the run.
    fn within<T: Send + 'static>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(std::time::Duration::from_secs(secs))
            .expect("timed out")
    }

    #[test]
    fn glob_basics() {
        assert!(matches_filter("main.rs", "*.rs"));
        assert!(matches_filter("MAIN.RS", "*.rs"));
        assert!(matches_filter("a.txt", "*.rs; *.txt"));
        assert!(matches_filter("ab.c", "a?.c"));
        assert!(!matches_filter("abc.c", "a?.c"));
        assert!(matches_filter("anything", ""));
        assert!(matches_filter("anything", "*.*"));
        assert!(!matches_filter("main.rs", "*.txt"));
        assert!(matches_filter("é中.md", "?中.*"));
        assert!(matches_filter("x", "**x**"));
    }

    /// Every string over `alphabet` up to `max` chars long.
    fn all_strings(alphabet: &[char], max: usize) -> Vec<String> {
        let mut out = vec![String::new()];
        let mut layer = vec![String::new()];
        for _ in 0..max {
            layer = layer
                .iter()
                .flat_map(|s| alphabet.iter().map(move |c| format!("{s}{c}")))
                .collect();
            out.extend(layer.iter().cloned());
        }
        out
    }

    /// Textbook dynamic-programming glob, the reference for `glob_match`.
    fn glob_reference(p: &str, n: &str) -> bool {
        let p: Vec<char> = p.chars().collect();
        let n: Vec<char> = n.chars().collect();
        let mut dp = vec![vec![false; n.len() + 1]; p.len() + 1];
        dp[0][0] = true;
        for i in 1..=p.len() {
            for j in 0..=n.len() {
                dp[i][j] = match p[i - 1] {
                    '*' => dp[i - 1][j] || (j > 0 && dp[i][j - 1]),
                    '?' => j > 0 && dp[i - 1][j - 1],
                    c => j > 0 && n[j - 1] == c && dp[i - 1][j - 1],
                };
            }
        }
        dp[p.len()][n.len()]
    }

    #[test]
    fn glob_agrees_with_reference_on_small_inputs() {
        let patterns = all_strings(&['a', 'b', '*', '?'], 5);
        let names = all_strings(&['a', 'b'], 5);
        for p in &patterns {
            for n in &names {
                assert_eq!(glob_match(p, n), glob_reference(p, n), "pattern {p:?} name {n:?}");
            }
        }
    }

    #[test]
    fn glob_many_stars_does_not_hang() {
        // "*a*a*a…b" against "aaaa…" backtracks exponentially in a naive
        // recursive matcher.
        let pattern = format!("{}b", "*a".repeat(40));
        let name = "a".repeat(80);
        assert!(!within(10, move || glob_match(&pattern, &name)));
    }

    fn results_for(text: &str, pattern: &str) -> Vec<FindResult> {
        let re = Regex::new(pattern).unwrap();
        let mut out = Vec::new();
        match_file(Path::new("x.txt"), text, &re, &mut out);
        out
    }

    #[test]
    fn result_lines_follow_rope_line_breaks() {
        // Every break ropey counts: LF, VT, FF, CR, CRLF, NEL, LS, PS.
        let text = "x0\nx1\u{0B}x2\u{0C}x3\rx4\r\nx5\u{85}x6\u{2028}x7\u{2029}x8 中x9";
        let rope = ropey::Rope::from_str(text);
        let results = results_for(text, "x[0-9]");
        assert_eq!(results.len(), 10);
        for (r, m) in results.iter().zip(Regex::new("x[0-9]").unwrap().find_iter(text)) {
            let line = rope.byte_to_line(m.start());
            assert_eq!(r.line, line, "match {:?}", m.as_str());
            assert_eq!(rope.line_to_byte(line) + r.match_in_line.start, m.start());
            assert!(!r.line_text.contains(['\r', '\n', '\u{0B}', '\u{0C}', '\u{85}', '\u{2028}', '\u{2029}']));
        }
    }

    #[test]
    fn line_walker_agrees_with_rope_at_every_offset() {
        let texts = [
            "",
            "\n",
            "\r\n\r\n",
            "a\r\rb\n\nc",
            "é\u{2028}中\u{2029}😀\u{85}👨‍👩‍👧\u{0B}x\u{0C}",
            "tail without newline",
            "\u{FEFF}bom\r\nline",
        ];
        for text in texts {
            let rope = ropey::Rope::from_str(text);
            let mut walker = LineWalker::new(text);
            for (b, _) in text.char_indices().chain(std::iter::once((text.len(), ' '))) {
                let (line, start) = walker.seek(b);
                assert_eq!(line, rope.byte_to_line(b), "{text:?} @ {b}");
                assert_eq!(start, rope.line_to_byte(line), "{text:?} @ {b}");
                let slice = rope.line(line);
                let content = slice.to_string();
                let trimmed = content
                    .strip_suffix("\r\n")
                    .or_else(|| content.strip_suffix(['\n', '\r', '\u{0B}', '\u{0C}', '\u{85}', '\u{2028}', '\u{2029}']))
                    .unwrap_or(&content);
                assert_eq!(walker.content_end(), start + trimmed.len(), "{text:?} @ {b}");
            }
        }
    }

    #[test]
    fn result_line_text_is_capped_on_long_lines() {
        // 1000 hits on a 1 MB line kept 1000 copies of the line (1 GB).
        let text = format!("{}{}", "中".repeat(200_000), "hit ".repeat(1000));
        let results = results_for(&text, "hit");
        assert_eq!(results.len(), FIND_IN_FILES_RESULT_CAP);
        for r in &results {
            assert!(r.line_text.len() <= RESULT_LINE_TEXT_MAX_BYTES);
            assert!(text.is_char_boundary(r.match_in_line.start));
            assert_eq!(&text[r.match_in_line.clone()], "hit");
        }
    }

    #[test]
    fn glob_long_pattern_does_not_overflow_stack() {
        // A pasted filter tens of thousands of chars long recursed once per
        // char and overflowed the stack.
        let pattern = "?".repeat(300_000);
        let name = "a".repeat(300_000);
        assert!(within(10, move || glob_match(&pattern, &name)));
    }
}
