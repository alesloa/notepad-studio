// Per-language comment delimiters used by `Edit > Block Comment` and
// `Edit > Stream Comment`. NPP picks comment chars per-language; we mirror
// that here keyed off `LanguageId`. Buffers without a detected language
// fall back to C-style (`//` line, `/* */` block) — same default the
// hardcoded toggle used to use.

use crate::syntax::LanguageId;

/// Comment delimiters for a single language. `line` is the prefix used
/// for line-comments (`//`, `#`, `--`, etc.). `block` is the
/// open / close pair for stream comments (`/* */`, `<!-- -->`, etc.).
/// Either field may be `None` when the language doesn't support that
/// comment style (e.g. YAML has no block comment, HTML has no line
/// comment, JSON has neither).
#[derive(Clone, Copy, Debug)]
pub(crate) struct CommentSyntax {
    pub(crate) line: Option<&'static str>,
    pub(crate) block: Option<(&'static str, &'static str)>,
}

impl CommentSyntax {
    const fn line_only(line: &'static str) -> Self {
        Self { line: Some(line), block: None }
    }
    const fn block_only(open: &'static str, close: &'static str) -> Self {
        Self { line: None, block: Some((open, close)) }
    }
    const fn both(line: &'static str, open: &'static str, close: &'static str) -> Self {
        Self { line: Some(line), block: Some((open, close)) }
    }
    const fn none() -> Self {
        Self { line: None, block: None }
    }
}

/// Default for buffers with no detected language. C-style is the
/// most common form across "unknown" code so it's the safest fallback.
pub(crate) const DEFAULT_COMMENT_SYNTAX: CommentSyntax =
    CommentSyntax::both("//", "/*", "*/");

/// Look up comment delimiters for `lang`. Returns `DEFAULT_COMMENT_SYNTAX`
/// when `lang` is `None` (no language detected — typical for unsaved
/// buffers and unsupported extensions).
pub(crate) fn comment_syntax(lang: Option<LanguageId>) -> CommentSyntax {
    let Some(lang) = lang else {
        return DEFAULT_COMMENT_SYNTAX;
    };
    match lang {
        // C-style line + block.
        LanguageId::Rust
        | LanguageId::C
        | LanguageId::Cpp
        | LanguageId::CSharp
        | LanguageId::Java
        | LanguageId::JavaScript
        | LanguageId::TypeScript
        | LanguageId::Tsx
        | LanguageId::Go
        | LanguageId::Swift
        | LanguageId::Php
        | LanguageId::Css
        | LanguageId::Scss => CommentSyntax::both("//", "/*", "*/"),

        // Hash-line + triple-quote block (NPP treats `"""` as Python's
        // block comment via docstring convention).
        LanguageId::Python => CommentSyntax::both("#", "\"\"\"", "\"\"\""),

        // Hash-line, no block comment.
        LanguageId::Bash | LanguageId::Toml | LanguageId::Yaml | LanguageId::Ruby => {
            CommentSyntax::line_only("#")
        }

        // Lua: `--` line, `--[[ ]]` block.
        LanguageId::Lua => CommentSyntax::both("--", "--[[", "]]"),

        // SQL: `--` line, `/* */` block.
        LanguageId::Sql => CommentSyntax::both("--", "/*", "*/"),

        // Markup: no line comment, `<!-- -->` block.
        LanguageId::Html | LanguageId::Xml | LanguageId::Markdown => {
            CommentSyntax::block_only("<!--", "-->")
        }

        // PowerShell: `#` line, `<# #>` block.
        LanguageId::PowerShell => CommentSyntax::both("#", "<#", "#>"),

        // JSON has no comments.
        LanguageId::Json => CommentSyntax::none(),
    }
}
