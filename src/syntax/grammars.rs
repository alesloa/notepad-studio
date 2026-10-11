// Per-language `HighlightConfiguration` builders. Each language's grammar
// crate exposes its compiled grammar (`LANGUAGE`) plus the highlight query
// strings (`HIGHLIGHTS_QUERY` or `HIGHLIGHT_QUERY`). Names diverge across
// crates — that's why this is one match per language rather than a generic
// loop. Markdown ships separate block + inline grammars; v1 only wires the
// block one (no injection callback yet, so fenced code blocks render as
// plain text inside the surrounding markdown styling).

use tree_sitter_highlight::HighlightConfiguration;

use super::LanguageId;

pub(super) fn build_config(lang: LanguageId) -> Option<HighlightConfiguration> {
    let result = match lang {
        LanguageId::Rust => HighlightConfiguration::new(
            tree_sitter_rust::LANGUAGE.into(),
            "rust",
            tree_sitter_rust::HIGHLIGHTS_QUERY,
            tree_sitter_rust::INJECTIONS_QUERY,
            "",
        ),
        LanguageId::Python => HighlightConfiguration::new(
            tree_sitter_python::LANGUAGE.into(),
            "python",
            tree_sitter_python::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        LanguageId::C => HighlightConfiguration::new(
            tree_sitter_c::LANGUAGE.into(),
            "c",
            tree_sitter_c::HIGHLIGHT_QUERY,
            "",
            "",
        ),
        LanguageId::CSharp => HighlightConfiguration::new(
            tree_sitter_c_sharp::LANGUAGE.into(),
            "c-sharp",
            tree_sitter_c_sharp::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        LanguageId::JavaScript => HighlightConfiguration::new(
            tree_sitter_javascript::LANGUAGE.into(),
            "javascript",
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::INJECTIONS_QUERY,
            tree_sitter_javascript::LOCALS_QUERY,
        ),
        LanguageId::TypeScript => {
            // The TypeScript query contains additions to JavaScript.
            let query = format!("{}\n{}", tree_sitter_javascript::HIGHLIGHT_QUERY, tree_sitter_typescript::HIGHLIGHTS_QUERY);
            HighlightConfiguration::new(
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                "typescript",
                &query,
                "",
                tree_sitter_typescript::LOCALS_QUERY,
            )
        }
        LanguageId::Tsx => {
            let query = format!("{}\n{}\n{}", tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY, tree_sitter_javascript::JSX_HIGHLIGHT_QUERY);
            HighlightConfiguration::new(
                tree_sitter_typescript::LANGUAGE_TSX.into(),
                "tsx",
                &query,
                "",
                tree_sitter_typescript::LOCALS_QUERY,
            )
        }
        LanguageId::Json => HighlightConfiguration::new(
            tree_sitter_json::LANGUAGE.into(),
            "json",
            tree_sitter_json::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        LanguageId::Markdown => HighlightConfiguration::new(
            tree_sitter_md::LANGUAGE.into(),
            "markdown",
            tree_sitter_md::HIGHLIGHT_QUERY_BLOCK,
            "",
            "",
        ),
        LanguageId::Bash => HighlightConfiguration::new(
            tree_sitter_bash::LANGUAGE.into(),
            "bash",
            tree_sitter_bash::HIGHLIGHT_QUERY,
            "",
            "",
        ),
        LanguageId::PowerShell => HighlightConfiguration::new(
            tree_sitter_powershell::LANGUAGE.into(),
            "powershell",
            tree_sitter_powershell::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        LanguageId::Go => HighlightConfiguration::new(
            tree_sitter_go::LANGUAGE.into(),
            "go",
            tree_sitter_go::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        LanguageId::Java => HighlightConfiguration::new(
            tree_sitter_java::LANGUAGE.into(),
            "java",
            tree_sitter_java::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        LanguageId::Swift => HighlightConfiguration::new(
            tree_sitter_swift::LANGUAGE.into(),
            "swift",
            tree_sitter_swift::HIGHLIGHTS_QUERY,
            tree_sitter_swift::INJECTIONS_QUERY,
            tree_sitter_swift::LOCALS_QUERY,
        ),
        LanguageId::Ruby => HighlightConfiguration::new(
            tree_sitter_ruby::LANGUAGE.into(),
            "ruby",
            tree_sitter_ruby::HIGHLIGHTS_QUERY,
            "",
            tree_sitter_ruby::LOCALS_QUERY,
        ),
        LanguageId::Php => HighlightConfiguration::new(
            tree_sitter_php::LANGUAGE_PHP.into(),
            "php",
            tree_sitter_php::HIGHLIGHTS_QUERY,
            tree_sitter_php::INJECTIONS_QUERY,
            "",
        ),
        LanguageId::Cpp => {
            // The C++ query contains additions to C.
            let query = format!("{}\n{}", tree_sitter_c::HIGHLIGHT_QUERY, tree_sitter_cpp::HIGHLIGHT_QUERY);
            HighlightConfiguration::new(
                tree_sitter_cpp::LANGUAGE.into(),
                "cpp",
                &query,
                "",
                "",
            )
        }
        LanguageId::Lua => HighlightConfiguration::new(
            tree_sitter_lua::LANGUAGE.into(),
            "lua",
            tree_sitter_lua::HIGHLIGHTS_QUERY,
            tree_sitter_lua::INJECTIONS_QUERY,
            tree_sitter_lua::LOCALS_QUERY,
        ),
        LanguageId::Html => HighlightConfiguration::new(
            tree_sitter_html::LANGUAGE.into(),
            "html",
            tree_sitter_html::HIGHLIGHTS_QUERY,
            tree_sitter_html::INJECTIONS_QUERY,
            "",
        ),
        LanguageId::Css => HighlightConfiguration::new(
            tree_sitter_css::LANGUAGE.into(),
            "css",
            tree_sitter_css::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        LanguageId::Scss => HighlightConfiguration::new(
            tree_sitter_scss::language(),
            "scss",
            tree_sitter_scss::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        LanguageId::Yaml => HighlightConfiguration::new(
            tree_sitter_yaml::LANGUAGE.into(),
            "yaml",
            tree_sitter_yaml::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        LanguageId::Toml => HighlightConfiguration::new(
            tree_sitter_toml_ng::LANGUAGE.into(),
            "toml",
            tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        LanguageId::Xml => HighlightConfiguration::new(
            tree_sitter_xml::LANGUAGE_XML.into(),
            "xml",
            tree_sitter_xml::XML_HIGHLIGHT_QUERY,
            "",
            "",
        ),
        LanguageId::Sql => HighlightConfiguration::new(
            tree_sitter_sequel::LANGUAGE.into(),
            "sql",
            tree_sitter_sequel::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
    };
    match result {
        Ok(cfg) => Some(cfg),
        Err(err) => {
            eprintln!("[syntax] failed to build {lang:?} highlight config: {err}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::{highlight, HighlightKind};

    fn has_token(language: LanguageId, source: &str, token: &str, kind: HighlightKind) -> bool {
        highlight(language, source).iter().any(|span| {
            span.kind == kind && source.get(span.range.clone()).is_some_and(|text| text.contains(token))
        })
    }

    #[test]
    fn cpp_inherited_tokens_keep_their_c_styles() {
        let source = "// note\nint main() { return 42; }\nclass Derived {};";
        assert!(has_token(LanguageId::Cpp, source, "note", HighlightKind::Comment));
        assert!(has_token(LanguageId::Cpp, source, "42", HighlightKind::Number));
        assert!(has_token(LanguageId::Cpp, source, "return", HighlightKind::Keyword));
        assert!(has_token(LanguageId::Cpp, source, "class", HighlightKind::Keyword));
    }

    #[test]
    fn typescript_inherited_tokens_keep_their_javascript_styles() {
        let source = "// note\nconst answer = 42;\ninterface Derived {}";
        for language in [LanguageId::TypeScript, LanguageId::Tsx] {
            assert!(has_token(language, source, "note", HighlightKind::Comment), "{language:?}");
            assert!(has_token(language, source, "42", HighlightKind::Number), "{language:?}");
            assert!(has_token(language, source, "const", HighlightKind::Keyword), "{language:?}");
            assert!(has_token(language, source, "interface", HighlightKind::Keyword), "{language:?}");
        }
    }

    #[test]
    fn markdown_headings_and_reference_links_use_the_theme() {
        let source = "# Heading\n\n[label]: https://example.test \"title\"\n";
        assert!(has_token(LanguageId::Markdown, source, "Heading", HighlightKind::Type));
        assert!(has_token(LanguageId::Markdown, source, "label", HighlightKind::Property));
        assert!(has_token(LanguageId::Markdown, source, "https://example.test", HighlightKind::String));
        assert!(has_token(LanguageId::Markdown, source, "title", HighlightKind::String));
    }

    #[test]
    fn tsx_inherited_tags_keep_their_jsx_styles() {
        let source = "const view = <div title=\"hello\"/>;";
        assert!(has_token(LanguageId::Tsx, source, "div", HighlightKind::Keyword));
        assert!(has_token(LanguageId::Tsx, source, "title", HighlightKind::Attribute));
        assert!(has_token(LanguageId::Tsx, source, "hello", HighlightKind::String));
    }
}
