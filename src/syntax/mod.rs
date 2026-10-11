// Tree-sitter syntax spans are sliced per visible line in editor_element.rs.
// Workspace schedules file-open, session and post-edit/language syntax jobs in background.

pub(crate) mod comments;
pub(crate) mod grammars;
pub(crate) mod keywords;
mod bounded_highlight;
pub(crate) use bounded_highlight::{HighlightError, highlight_cancellable};

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tree_sitter_highlight::{HighlightConfiguration, Highlighter};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LanguageId {
    Rust,
    Python,
    C,
    CSharp,
    JavaScript,
    TypeScript,
    Tsx,
    Json,
    Markdown,
    Bash,
    PowerShell,
    Go,
    Java,
    Swift,
    Ruby,
    Php,
    Cpp,
    Lua,
    Html,
    Css,
    Scss,
    Yaml,
    Toml,
    Xml,
    Sql,
}

impl LanguageId {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::Python => "Python",
            Self::C => "C",
            Self::CSharp => "C#",
            Self::JavaScript => "JavaScript",
            Self::TypeScript => "TypeScript",
            Self::Tsx => "TSX",
            Self::Json => "JSON",
            Self::Markdown => "Markdown",
            Self::Bash => "Shell",
            Self::PowerShell => "PowerShell",
            Self::Go => "Go",
            Self::Java => "Java",
            Self::Swift => "Swift",
            Self::Ruby => "Ruby",
            Self::Php => "PHP",
            Self::Cpp => "C++",
            Self::Lua => "Lua",
            Self::Html => "HTML",
            Self::Css => "CSS",
            Self::Scss => "SCSS",
            Self::Yaml => "YAML",
            Self::Toml => "TOML",
            Self::Xml => "XML",
            Self::Sql => "SQL",
        }
    }

    pub(crate) fn all() -> &'static [LanguageId] {
        &[
            Self::Bash,
            Self::C,
            Self::CSharp,
            Self::Cpp,
            Self::Css,
            Self::Go,
            Self::Html,
            Self::Java,
            Self::JavaScript,
            Self::Json,
            Self::Lua,
            Self::Markdown,
            Self::Php,
            Self::PowerShell,
            Self::Python,
            Self::Ruby,
            Self::Rust,
            Self::Scss,
            Self::Sql,
            Self::Swift,
            Self::Toml,
            Self::Tsx,
            Self::TypeScript,
            Self::Xml,
            Self::Yaml,
        ]
    }

    /// Stable snake_case identifier for serde-keyed maps (e.g.
    /// `Settings::language_indent_overrides`). Matches the enum's
    /// `rename_all = "snake_case"` serde representation so a value
    /// round-tripped through JSON/TOML uses this same string.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::C => "c",
            Self::CSharp => "c_sharp",
            Self::JavaScript => "java_script",
            Self::TypeScript => "type_script",
            Self::Tsx => "tsx",
            Self::Json => "json",
            Self::Markdown => "markdown",
            Self::Bash => "bash",
            Self::PowerShell => "power_shell",
            Self::Go => "go",
            Self::Java => "java",
            Self::Swift => "swift",
            Self::Ruby => "ruby",
            Self::Php => "php",
            Self::Cpp => "cpp",
            Self::Lua => "lua",
            Self::Html => "html",
            Self::Css => "css",
            Self::Scss => "scss",
            Self::Yaml => "yaml",
            Self::Toml => "toml",
            Self::Xml => "xml",
            Self::Sql => "sql",
        }
    }

    pub(crate) fn pref_id(self) -> &'static str {
        match self {
            Self::Rust => "prefs-language-rust",
            Self::Python => "prefs-language-python",
            Self::C => "prefs-language-c",
            Self::CSharp => "prefs-language-csharp",
            Self::JavaScript => "prefs-language-javascript",
            Self::TypeScript => "prefs-language-typescript",
            Self::Tsx => "prefs-language-tsx",
            Self::Json => "prefs-language-json",
            Self::Markdown => "prefs-language-markdown",
            Self::Bash => "prefs-language-bash",
            Self::PowerShell => "prefs-language-powershell",
            Self::Go => "prefs-language-go",
            Self::Java => "prefs-language-java",
            Self::Swift => "prefs-language-swift",
            Self::Ruby => "prefs-language-ruby",
            Self::Php => "prefs-language-php",
            Self::Cpp => "prefs-language-cpp",
            Self::Lua => "prefs-language-lua",
            Self::Html => "prefs-language-html",
            Self::Css => "prefs-language-css",
            Self::Scss => "prefs-language-scss",
            Self::Yaml => "prefs-language-yaml",
            Self::Toml => "prefs-language-toml",
            Self::Xml => "prefs-language-xml",
            Self::Sql => "prefs-language-sql",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum HighlightKind {
    Keyword,
    String,
    Comment,
    Number,
    Type,
    Function,
    Constant,
    Property,
    Attribute,
}

#[derive(Clone, Debug)]
pub(crate) struct HighlightSpan {
    pub(crate) range: Range<usize>,
    pub(crate) kind: HighlightKind,
}

pub(crate) fn language_for_path(path: &Path) -> Option<LanguageId> {
    // Filename-based detection first — covers extensionless files (Dockerfile,
    // Makefile) and special filenames where the basename matters.
    if let Some(stem) = path.file_name().and_then(|s| s.to_str()) {
        let lower = stem.to_ascii_lowercase();
        // (Dockerfile / Makefile etc. land here once their grammars are wired.)
        if lower == "rakefile" || lower == "gemfile" {
            return Some(LanguageId::Ruby);
        }
    }

    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "rs" => LanguageId::Rust,
        "py" | "pyi" | "pyw" => LanguageId::Python,
        "c" => LanguageId::C,
        "cs" => LanguageId::CSharp,
        "js" | "mjs" | "cjs" | "jsx" => LanguageId::JavaScript,
        "ts" => LanguageId::TypeScript,
        "tsx" => LanguageId::Tsx,
        "json" => LanguageId::Json,
        "md" | "markdown" => LanguageId::Markdown,
        "sh" | "bash" | "zsh" | "ksh" => LanguageId::Bash,
        "ps1" | "psm1" | "psd1" => LanguageId::PowerShell,
        "go" => LanguageId::Go,
        "java" => LanguageId::Java,
        "swift" => LanguageId::Swift,
        "rb" | "rake" | "gemspec" => LanguageId::Ruby,
        "php" | "phtml" => LanguageId::Php,
        "cpp" | "cxx" | "cc" | "h" | "hpp" | "hxx" | "hh" => LanguageId::Cpp,
        "lua" => LanguageId::Lua,
        "html" | "htm" => LanguageId::Html,
        "css" => LanguageId::Css,
        "scss" => LanguageId::Scss,
        "yaml" | "yml" => LanguageId::Yaml,
        "toml" => LanguageId::Toml,
        "xml" | "svg" | "xhtml" | "xsl" | "xslt" => LanguageId::Xml,
        "sql" => LanguageId::Sql,
        _ => return None,
    })
}

/// Markdown fenced-code info-string → `LanguageId`. The info-string is
/// freeform user input (e.g. ```` ```rust ````, ```` ```py ````,
/// ```` ```sh ````) so we accept the common aliases for every grammar we
/// support. Case-insensitive; the leading word before any space is what
/// counts (pulldown-cmark also strips trailing classes like
/// `rust,no_run` — we mirror that by splitting on whitespace and comma).
pub(crate) fn language_for_info_string(info: &str) -> Option<LanguageId> {
    let head = info
        .split(|c: char| c.is_whitespace() || c == ',')
        .next()?;
    if head.is_empty() {
        return None;
    }
    Some(match head.to_ascii_lowercase().as_str() {
        "rust" | "rs" => LanguageId::Rust,
        "python" | "py" | "python3" => LanguageId::Python,
        "c" => LanguageId::C,
        "csharp" | "cs" | "c#" => LanguageId::CSharp,
        "javascript" | "js" | "mjs" | "cjs" | "node" | "jsx" => LanguageId::JavaScript,
        "typescript" | "ts" => LanguageId::TypeScript,
        "tsx" => LanguageId::Tsx,
        "json" | "json5" | "jsonc" => LanguageId::Json,
        "markdown" | "md" => LanguageId::Markdown,
        "bash" | "sh" | "shell" | "zsh" | "ksh" => LanguageId::Bash,
        "powershell" | "ps1" | "pwsh" => LanguageId::PowerShell,
        "go" | "golang" => LanguageId::Go,
        "java" => LanguageId::Java,
        "swift" => LanguageId::Swift,
        "ruby" | "rb" => LanguageId::Ruby,
        "php" | "phtml" => LanguageId::Php,
        "cpp" | "c++" | "cxx" | "cc" | "hpp" | "hxx" => LanguageId::Cpp,
        "lua" => LanguageId::Lua,
        "html" | "htm" => LanguageId::Html,
        "css" => LanguageId::Css,
        "scss" | "sass" => LanguageId::Scss,
        "yaml" | "yml" => LanguageId::Yaml,
        "toml" => LanguageId::Toml,
        "xml" | "svg" | "xhtml" => LanguageId::Xml,
        "sql" => LanguageId::Sql,
        _ => return None,
    })
}

/// Last-resort detection from the file's first line. Called by `io::open_path`
/// when both extension and filename matching come up empty. Reads the shebang
/// line (`#!...`) and maps common interpreters to languages.
pub(crate) fn language_for_shebang(content: &str) -> Option<LanguageId> {
    let first_line = content.lines().next()?;
    if !first_line.starts_with("#!") {
        return None;
    }
    let line = first_line.to_ascii_lowercase();
    if line.contains("bash") || line.contains("/sh") || line.contains("zsh") || line.contains("ksh") {
        Some(LanguageId::Bash)
    } else if line.contains("python") {
        Some(LanguageId::Python)
    } else if line.contains("node") {
        Some(LanguageId::JavaScript)
    } else if line.contains("ruby") {
        Some(LanguageId::Ruby)
    } else if line.contains("php") {
        Some(LanguageId::Php)
    } else if line.contains("pwsh") || line.contains("powershell") {
        Some(LanguageId::PowerShell)
    } else if line.contains("lua") {
        Some(LanguageId::Lua)
    } else {
        None
    }
}

/// Index → tree-sitter highlight name. Order matters — every grammar's
/// `HighlightConfiguration::configure` is called with this slice, so the
/// numeric index that the highlighter reports is a position into this list.
pub(crate) const HIGHLIGHT_NAMES: &[&str] = &[
    "attribute",
    "comment",
    "constant",
    "constant.builtin",
    "constructor",
    "function",
    "function.builtin",
    "function.macro",
    "function.method",
    "keyword",
    "label",
    "number",
    "operator",
    "property",
    "punctuation",
    "punctuation.bracket",
    "punctuation.delimiter",
    "punctuation.special",
    "string",
    "string.special",
    "tag",
    "type",
    "type.builtin",
    "variable",
    "variable.builtin",
    "variable.parameter",
    "text.title",
    "text.literal",
    "text.uri",
    "text.reference",
];

fn index_to_kind(idx: usize) -> Option<HighlightKind> {
    let name = *HIGHLIGHT_NAMES.get(idx)?;
    Some(match name {
        "attribute" => HighlightKind::Attribute,
        "comment" => HighlightKind::Comment,
        "constant" | "constant.builtin" => HighlightKind::Constant,
        "constructor"
        | "function"
        | "function.builtin"
        | "function.macro"
        | "function.method"
        | "label" => HighlightKind::Function,
        "keyword" | "tag" => HighlightKind::Keyword,
        "number" => HighlightKind::Number,
        "property" | "text.reference" => HighlightKind::Property,
        "string" | "string.special" | "text.literal" | "text.uri" => HighlightKind::String,
        "type" | "type.builtin" | "text.title" => HighlightKind::Type,
        // operator / punctuation / variable.* fall through to default text color.
        _ => return None,
    })
}

thread_local! {
    static CONFIGS: RefCell<HashMap<LanguageId, HighlightConfiguration>> = RefCell::new(HashMap::new());
    static HIGHLIGHTER: RefCell<Highlighter> = RefCell::new(Highlighter::new());
}

/// Run the highlighter on `source` against `lang`'s grammar and return a
/// flat list of (byte-range, kind) spans in document order. Returns empty
/// vec if the grammar fails to build or the highlighter errors — callers
/// just paint plain text in that case.
/// Returns true when the buffer should bypass tree-sitter parsing because
/// it exceeds the user-configured large-file threshold. Mirrors NPP's
/// `Buffer::isLargeFile` gate (Buffer.cpp:942) — when the restriction is
/// enabled and the file size is at or above
/// `large_file_threshold_bytes`, the styling pass is skipped.
/// The 50 MiB safety ceiling applies even when user restrictions are disabled.
pub(crate) fn should_skip_syntax(buffer_size_bytes: u64, settings: &crate::settings::Settings) -> bool {
    buffer_size_bytes > crate::io_stream::HIGHLIGHT_SIZE_LIMIT || (settings.large_file_restriction_enabled
        && settings.large_file_disable_syntax
        && buffer_size_bytes >= settings.large_file_threshold_bytes)
}

pub(crate) fn highlight(lang: LanguageId, source: &str) -> Vec<HighlightSpan> {
    highlight_cancellable(lang, source, None).unwrap_or_default()
}

/// Map a `HighlightKind` to a syntax-theme color. `None` means "use default
/// text color" — variable / operator / punctuation already return `None`
/// from `index_to_kind`, so they never reach here, but kinds that don't
/// have a slot in `SyntaxTheme` would land here in the future.
pub(crate) fn kind_color(kind: HighlightKind, syntax: &crate::theme::SyntaxTheme) -> u32 {
    match kind {
        HighlightKind::Keyword => syntax.keyword,
        HighlightKind::String => syntax.string,
        HighlightKind::Comment => syntax.comment,
        HighlightKind::Number => syntax.number,
        HighlightKind::Type => syntax.type_,
        HighlightKind::Function => syntax.function,
        HighlightKind::Constant => syntax.constant,
        HighlightKind::Property => syntax.property,
        HighlightKind::Attribute => syntax.attribute,
    }
}
