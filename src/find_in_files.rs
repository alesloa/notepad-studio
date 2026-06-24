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

fn matches_filter(filename: &str, filter: &str) -> bool {
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
fn glob_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().flat_map(|c| c.to_lowercase()).collect();
    let n: Vec<char> = name.chars().flat_map(|c| c.to_lowercase()).collect();
    glob_recurse(&p, &n)
}

fn glob_recurse(p: &[char], n: &[char]) -> bool {
    match (p.first(), n.first()) {
        (None, None) => true,
        (None, Some(_)) => false,
        (Some('*'), _) => {
            if glob_recurse(&p[1..], n) {
                return true;
            }
            !n.is_empty() && glob_recurse(p, &n[1..])
        }
        (Some('?'), Some(_)) => glob_recurse(&p[1..], &n[1..]),
        (Some(pc), Some(nc)) if pc == nc => glob_recurse(&p[1..], &n[1..]),
        _ => false,
    }
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
    // Pre-compute line-start byte offsets so each match resolves to its
    // (line index, line start) in O(log n) — beats re-scanning per match.
    let mut line_starts: Vec<usize> = vec![0];
    for (i, b) in content.bytes().enumerate() {
        if b == b'\n' {
            line_starts.push(i + 1);
        }
    }

    for m in re.find_iter(content) {
        // Skip zero-length matches (e.g. `^`, `\b`) — they'd otherwise spam
        // a result on every line for a pattern like `\b`.
        if m.start() == m.end() {
            continue;
        }
        let byte = m.start();
        let line = match line_starts.binary_search(&byte) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let line_start = line_starts[line];
        let line_end = line_starts
            .get(line + 1)
            .map(|p| p.saturating_sub(1))
            .unwrap_or(content.len());
        let line_text = content[line_start..line_end].to_string();
        let local_start = byte - line_start;
        let local_end = (m.end() - line_start).min(line_text.len());
        out.push(FindResult {
            path: path.to_path_buf(),
            line,
            line_text,
            match_in_line: local_start..local_end,
            tab_idx: None,
            display_label: None,
        });
        if out.len() >= FIND_IN_FILES_RESULT_CAP {
            break;
        }
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
