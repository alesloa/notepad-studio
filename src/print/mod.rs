// NPP `Print ▸ Header/Footer` template variable substitution.
//
// Header / footer text supports a small token language that NPP expands at
// print time. The token set is documented in NPP's user manual under
// `Preferences ▸ Print`; the codes mirror NPP's `Printer.cpp::expandHeaderFooter`
// (the `${...}` syntax is NPP's; we use `$(...)` to match NPP's
// `Run ▸ Run...` token style which is the more familiar Notepad Studio convention).
//
// The print pipeline itself (NSPrintOperation on macOS, GDI on Windows) is
// not yet wired; the panel persists the templates and Cmd+P stays unwired
// until the pipeline lands. The substitution helper here is reusable from
// the print pipeline once it is implemented.

use chrono::{DateTime, Local};
use std::path::PathBuf;

/// Context for `substitute_template`. Each token expansion reads a single
/// field of this struct; missing context (e.g. printing an untitled buffer
/// has no `file_path`) substitutes an empty string for the path-derived
/// tokens, matching NPP's behaviour for unsaved documents.
//
// Allowed-dead until the print pipeline (NSPrintOperation on macOS) is
// wired and consumes this struct. Pipeline is DEFER_LONG; settings panel
// ships first so user templates persist round-trip.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct PrintVarContext {
    /// Absolute path to the source file. `None` for unsaved buffers.
    pub file_path: Option<PathBuf>,
    /// Word at the caret when print was invoked. Empty when no word.
    pub current_word: String,
    /// 1-based line number at the caret.
    pub current_line: u32,
    /// 1-based column number at the caret.
    pub current_column: u32,
    /// Wall-clock time at print invocation. Used for `$(SHORT_DATE)`,
    /// `$(LONG_DATE)`, `$(TIME)`. The caller picks the timezone — Notepad Studio
    /// uses `Local` to match NPP's "user-locale clock" expectation.
    pub now: DateTime<Local>,
    /// Notepad Studio's executable directory (for `$(NPP_DIRECTORY)` parity).
    /// Empty string disables the token; matches the unsaved-buffer
    /// behaviour for `$(NPP_FULL_FILE_PATH)` when no exe path is known.
    pub app_directory: PathBuf,
    /// Path to the running Notepad Studio executable itself (for
    /// `$(NPP_FULL_FILE_PATH)` parity).
    pub app_executable_path: PathBuf,
}

impl PrintVarContext {
    /// Convenience builder used by tests; production callers fill the
    /// struct field-by-field from the active workspace + buffer state.
    #[cfg(test)]
    fn empty(now: DateTime<Local>) -> Self {
        Self {
            file_path: None,
            current_word: String::new(),
            current_line: 1,
            current_column: 1,
            now,
            app_directory: PathBuf::new(),
            app_executable_path: PathBuf::new(),
        }
    }
}

/// Replace `$(TOKEN)` occurrences in `template` with values pulled from
/// `ctx`. Unknown tokens are left intact so a typo in the user's header
/// template doesn't silently disappear at print time. Performs a single
/// left-to-right scan; the loop body is O(n * t) where t is the token count
/// — fine for header/footer strings which are at most a handful of tokens.
//
// Allowed-dead until the print pipeline consumes it. The unit-test suite
// exercises every supported token so the helper stays correct while the
// pipeline is in development.
#[allow(dead_code)]
pub fn substitute_template(template: &str, ctx: &PrintVarContext) -> String {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'(' {
            if let Some(end) = bytes[i + 2..].iter().position(|&b| b == b')') {
                let token = &template[i + 2..i + 2 + end];
                if let Some(replacement) = expand_token(token, ctx) {
                    out.push_str(&replacement);
                    i += 2 + end + 1;
                    continue;
                }
            }
        }
        // Push the current byte boundary (UTF-8 safe — we only consume `$`
        // when followed by `(`, which are ASCII).
        let next = i + char_at_len(template, i);
        out.push_str(&template[i..next]);
        i = next;
    }
    out
}

/// Returns `Some(value)` when the token is recognized, `None` otherwise.
/// Unknown tokens are emitted verbatim by the caller so user typos don't
/// vanish silently.
fn expand_token(token: &str, ctx: &PrintVarContext) -> Option<String> {
    let path = ctx.file_path.as_deref();
    let value = match token {
        "FULL_CURRENT_PATH" => path
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        "CURRENT_DIRECTORY" => path
            .and_then(|p| p.parent())
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        "FILE_NAME" => path
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        "NAME_PART" => path
            .and_then(|p| p.file_stem())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        "EXT_PART" => path
            .and_then(|p| p.extension())
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_default(),
        "CURRENT_WORD" => ctx.current_word.clone(),
        "CURRENT_LINE" => ctx.current_line.to_string(),
        "CURRENT_COLUMN" => ctx.current_column.to_string(),
        "NPP_DIRECTORY" => ctx.app_directory.display().to_string(),
        "NPP_FULL_FILE_PATH" => ctx.app_executable_path.display().to_string(),
        "SHORT_DATE" => ctx.now.format("%Y-%m-%d").to_string(),
        "LONG_DATE" => ctx.now.format("%A, %B %e, %Y").to_string(),
        "TIME" => ctx.now.format("%H:%M:%S").to_string(),
        _ => return None,
    };
    Some(value)
}

/// Width of the UTF-8 character starting at `byte_idx` in `s`. Returns 1
/// for the trailing `'$'` of a malformed `$(...)` so we make forward
/// progress on every iteration even when no token matches.
fn char_at_len(s: &str, byte_idx: usize) -> usize {
    s[byte_idx..]
        .chars()
        .next()
        .map(|c| c.len_utf8())
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fixed_now() -> DateTime<Local> {
        // 2026-05-08 13:45:22 local. The exact weekday rendered by
        // `LONG_DATE` depends on the platform calendar, so the tests below
        // only assert prefixes/components that don't drift across locales.
        Local.with_ymd_and_hms(2026, 5, 8, 13, 45, 22).unwrap()
    }

    #[test]
    fn substitutes_full_current_path() {
        let mut ctx = PrintVarContext::empty(fixed_now());
        ctx.file_path = Some(PathBuf::from("/tmp/notes.txt"));
        assert_eq!(
            substitute_template("File: $(FULL_CURRENT_PATH)", &ctx),
            "File: /tmp/notes.txt"
        );
    }

    #[test]
    fn substitutes_current_directory() {
        let mut ctx = PrintVarContext::empty(fixed_now());
        ctx.file_path = Some(PathBuf::from("/tmp/sub/notes.txt"));
        assert_eq!(
            substitute_template("$(CURRENT_DIRECTORY)", &ctx),
            "/tmp/sub"
        );
    }

    #[test]
    fn substitutes_file_name() {
        let mut ctx = PrintVarContext::empty(fixed_now());
        ctx.file_path = Some(PathBuf::from("/tmp/notes.txt"));
        assert_eq!(substitute_template("$(FILE_NAME)", &ctx), "notes.txt");
    }

    #[test]
    fn substitutes_name_part() {
        let mut ctx = PrintVarContext::empty(fixed_now());
        ctx.file_path = Some(PathBuf::from("/tmp/notes.txt"));
        assert_eq!(substitute_template("$(NAME_PART)", &ctx), "notes");
    }

    #[test]
    fn substitutes_ext_part() {
        let mut ctx = PrintVarContext::empty(fixed_now());
        ctx.file_path = Some(PathBuf::from("/tmp/notes.txt"));
        assert_eq!(substitute_template("$(EXT_PART)", &ctx), "txt");
    }

    #[test]
    fn substitutes_current_word() {
        let mut ctx = PrintVarContext::empty(fixed_now());
        ctx.current_word = "hello".to_string();
        assert_eq!(
            substitute_template("word=$(CURRENT_WORD)", &ctx),
            "word=hello"
        );
    }

    #[test]
    fn substitutes_current_line_and_column() {
        let mut ctx = PrintVarContext::empty(fixed_now());
        ctx.current_line = 42;
        ctx.current_column = 7;
        assert_eq!(
            substitute_template("$(CURRENT_LINE):$(CURRENT_COLUMN)", &ctx),
            "42:7"
        );
    }

    #[test]
    fn substitutes_npp_paths() {
        let mut ctx = PrintVarContext::empty(fixed_now());
        ctx.app_directory = PathBuf::from("/Applications/Notepad Studio");
        ctx.app_executable_path = PathBuf::from("/Applications/Notepad Studio/notepad-studio");
        assert_eq!(
            substitute_template("$(NPP_DIRECTORY)|$(NPP_FULL_FILE_PATH)", &ctx),
            "/Applications/Notepad Studio|/Applications/Notepad Studio/notepad-studio"
        );
    }

    #[test]
    fn substitutes_short_date() {
        let ctx = PrintVarContext::empty(fixed_now());
        assert_eq!(substitute_template("$(SHORT_DATE)", &ctx), "2026-05-08");
    }

    #[test]
    fn substitutes_long_date_components() {
        let ctx = PrintVarContext::empty(fixed_now());
        let out = substitute_template("$(LONG_DATE)", &ctx);
        // Format string `%A, %B %e, %Y` produces e.g. "Friday, May  8, 2026".
        // Assert the year + month name + day to stay locale-independent for
        // the day-of-week noise in CI.
        assert!(out.contains("2026"));
        assert!(out.contains("May"));
        assert!(out.contains("8"));
    }

    #[test]
    fn substitutes_time() {
        let ctx = PrintVarContext::empty(fixed_now());
        assert_eq!(substitute_template("$(TIME)", &ctx), "13:45:22");
    }

    #[test]
    fn unknown_token_left_intact() {
        let ctx = PrintVarContext::empty(fixed_now());
        assert_eq!(
            substitute_template("$(NOT_A_REAL_TOKEN)", &ctx),
            "$(NOT_A_REAL_TOKEN)"
        );
    }

    #[test]
    fn malformed_dollar_paren_left_intact() {
        let ctx = PrintVarContext::empty(fixed_now());
        // No closing paren — emit the dollar verbatim and keep going.
        assert_eq!(substitute_template("$(unterminated", &ctx), "$(unterminated");
    }

    #[test]
    fn missing_path_renders_empty() {
        let ctx = PrintVarContext::empty(fixed_now());
        assert_eq!(
            substitute_template("[$(FULL_CURRENT_PATH)]", &ctx),
            "[]"
        );
    }

    #[test]
    fn empty_template_returns_empty() {
        let ctx = PrintVarContext::empty(fixed_now());
        assert_eq!(substitute_template("", &ctx), "");
    }

    #[test]
    fn multibyte_text_around_tokens_preserved() {
        let mut ctx = PrintVarContext::empty(fixed_now());
        ctx.current_word = "résumé".to_string();
        assert_eq!(
            substitute_template("café — $(CURRENT_WORD)", &ctx),
            "café — résumé"
        );
    }
}
