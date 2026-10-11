// PowerEditor/src/NppCommands.cpp:4008-4017 — selecting a buffer's language.

use gpui::{AppContext, Context, Window};

use crate::actions::*;
use crate::buffer::Buffer;
use crate::syntax::{HighlightSpan, LanguageId};
use super::Workspace;

fn select_language(buffer: &mut Buffer, language: Option<LanguageId>) -> Result<bool, &'static str> {
    if buffer.loading.is_some() {
        return Err("Wait for the file to finish loading before changing its language");
    }
    if buffer.language == language {
        return Ok(false);
    }
    buffer.language = language;
    buffer.highlights.clear();
    Ok(true)
}

fn parse_language(rope: &ropey::Rope, language: LanguageId, settings: &crate::settings::Settings) -> Vec<HighlightSpan> {
    if crate::syntax::should_skip_syntax(rope.len_bytes() as u64, settings) {
        return Vec::new();
    }
    crate::syntax::highlight(language, &rope.to_string())
}

fn install_highlights(buffer: &mut Buffer, language: LanguageId, source: &ropey::Rope, highlights: Vec<HighlightSpan>) -> bool {
    if buffer.loading.is_some() || buffer.language != Some(language) || !buffer.rope.is_instance(source) {
        return false;
    }
    buffer.highlights = highlights;
    true
}

macro_rules! language_handlers {
    ($($method:ident, $action:ident, $language:expr;)*) => {
        impl Workspace {
            $(pub(crate) fn $method(&mut self, _: &$action, _window: &mut Window, cx: &mut Context<Self>) {
                self.set_language($language, cx);
            })*
        }
    };
}

language_handlers! {
    language_normal_text, LanguageNormalText, None;
    language_c, LanguageC, Some(LanguageId::C);
    language_csharp, LanguageCSharp, Some(LanguageId::CSharp);
    language_cpp, LanguageCpp, Some(LanguageId::Cpp);
    language_css, LanguageCss, Some(LanguageId::Css);
    language_go, LanguageGo, Some(LanguageId::Go);
    language_html, LanguageHtml, Some(LanguageId::Html);
    language_java, LanguageJava, Some(LanguageId::Java);
    language_javascript, LanguageJavaScript, Some(LanguageId::JavaScript);
    language_json, LanguageJson, Some(LanguageId::Json);
    language_lua, LanguageLua, Some(LanguageId::Lua);
    language_markdown, LanguageMarkdown, Some(LanguageId::Markdown);
    language_php, LanguagePhp, Some(LanguageId::Php);
    language_powershell, LanguagePowerShell, Some(LanguageId::PowerShell);
    language_python, LanguagePython, Some(LanguageId::Python);
    language_ruby, LanguageRuby, Some(LanguageId::Ruby);
    language_rust, LanguageRust, Some(LanguageId::Rust);
    language_scss, LanguageScss, Some(LanguageId::Scss);
    language_bash, LanguageShell, Some(LanguageId::Bash);
    language_sql, LanguageSql, Some(LanguageId::Sql);
    language_swift, LanguageSwift, Some(LanguageId::Swift);
    language_toml, LanguageToml, Some(LanguageId::Toml);
    language_tsx, LanguageTsx, Some(LanguageId::Tsx);
    language_typescript, LanguageTypeScript, Some(LanguageId::TypeScript);
    language_xml, LanguageXml, Some(LanguageId::Xml);
    language_yaml, LanguageYaml, Some(LanguageId::Yaml);
}

impl Workspace {
    fn set_language(&mut self, language: Option<LanguageId>, cx: &mut Context<Self>) {
        match select_language(&mut self.active_tab_mut().buffer, language) {
            Ok(false) => return,
            Err(message) => {
                self.set_status(message);
                cx.notify();
                return;
            }
            Ok(true) => {}
        }
        self.close_autocomplete(cx);
        self.close_function_hint(cx);
        self.refresh_tag_match();
        self.last_lines.clear();
        self.set_status(format!("Language: {}", language.map(LanguageId::label).unwrap_or("None (Normal Text)")));
        self.save_session(cx);
        cx.notify();

        let Some(language) = language else { return; };
        let tab = self.active_tab_mut();
        let id = tab.session_id;
        let rope = tab.buffer.rope.clone();
        let settings = crate::settings::snapshot();
        let task = cx.background_spawn(async move {
            let highlights = parse_language(&rope, language, &settings);
            (rope, highlights)
        });
        cx.spawn(async move |this, cx| {
            let (source, highlights) = task.await;
            let _ = this.update(cx, |workspace, cx| {
                let Some(tab) = workspace.tabs.iter_mut().find(|tab| tab.session_id == id) else { return; };
                if !install_highlights(&mut tab.buffer, language, &source, highlights) { return; }
                workspace.last_lines.clear();
                cx.notify();
            });
        }).detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::encoding::{self, Encoding};
    use crate::text::line_ending::{split_line_endings, LineEnding};

    #[test]
    fn language_changes_preserve_file_bytes_and_edit_state() {
        let source = "# café 中😀\r\nprint(42)\r\n";
        for encoding in [Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be] {
            let bytes = encoding::encode_checked(source, encoding).unwrap();
            let decoded = encoding::decode_bytes(&bytes);
            let (eol, text) = split_line_endings(decoded.text);
            let mut buffer = Buffer::empty();
            buffer.rope = ropey::Rope::from_str(&text);
            buffer.encoding = decoded.encoding;
            buffer.line_ending = eol;
            buffer.file_path = Some("sample.txt".into());
            buffer.read_only = true;
            buffer.dirty = false;
            buffer.bookmarks.insert(0);
            buffer.marks.push(0..1);
            assert!(select_language(&mut buffer, Some(LanguageId::Python)).unwrap());
            let settings = crate::settings::Settings::default();
            buffer.highlights = parse_language(&buffer.rope, LanguageId::Python, &settings);
            assert!(!buffer.highlights.is_empty());
            assert_eq!(buffer.language, Some(LanguageId::Python));
            assert!(!buffer.dirty);
            assert!(buffer.read_only);
            assert_eq!(buffer.file_path, Some("sample.txt".into()));
            assert_eq!(buffer.bookmarks.iter().copied().collect::<Vec<_>>(), vec![0]);
            assert_eq!(buffer.marks, vec![0..1]);
            assert_eq!(buffer.encoding, encoding);
            assert_eq!(buffer.line_ending, LineEnding::Crlf);
            let disk_text = buffer.rope.to_string().replace('\n', "\r\n");
            assert_eq!(encoding::encode_checked(&disk_text, buffer.encoding).unwrap(), bytes);
            assert!(select_language(&mut buffer, None).unwrap());
            assert!(buffer.highlights.is_empty());
            assert!(!select_language(&mut buffer, None).unwrap());
            buffer.dirty = true;
            assert!(select_language(&mut buffer, Some(LanguageId::Rust)).unwrap());
            assert!(buffer.dirty);
        }
    }

    #[test]
    fn loading_buffers_keep_their_detected_language_and_highlights() {
        let mut buffer = Buffer::empty();
        buffer.language = Some(LanguageId::Rust);
        buffer.highlights = vec![HighlightSpan { range: 0..2, kind: crate::syntax::HighlightKind::Keyword }];
        buffer.loading = Some(crate::buffer::LoadProgress {
            bytes_read: 0,
            total_bytes: 100,
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        });
        assert!(select_language(&mut buffer, Some(LanguageId::Python)).is_err());
        assert_eq!(buffer.language, Some(LanguageId::Rust));
        assert_eq!(buffer.highlights.len(), 1);
    }

    #[test]
    fn stale_highlights_do_not_replace_a_new_language_or_edited_text() {
        let mut buffer = Buffer::empty();
        buffer.rope = ropey::Rope::from_str("let x = 1;");
        select_language(&mut buffer, Some(LanguageId::Rust)).unwrap();
        let source = buffer.rope.clone();
        let spans = parse_language(&source, LanguageId::Rust, &crate::settings::Settings::default());
        assert!(!spans.is_empty());
        assert!(install_highlights(&mut buffer, LanguageId::Rust, &source, spans.clone()));
        select_language(&mut buffer, Some(LanguageId::Python)).unwrap();
        assert!(!install_highlights(&mut buffer, LanguageId::Rust, &source, spans.clone()));
        assert!(buffer.highlights.is_empty());
        select_language(&mut buffer, Some(LanguageId::Rust)).unwrap();
        buffer.rope.insert(0, "// draft\n");
        assert!(!install_highlights(&mut buffer, LanguageId::Rust, &source, spans));
        assert!(buffer.highlights.is_empty());
    }

    #[test]
    fn large_file_restrictions_apply_when_selecting_a_language() {
        let rope = ropey::Rope::from_str("let x = 1;");
        let settings = crate::settings::Settings {
            large_file_restriction_enabled: true,
            large_file_disable_syntax: true,
            large_file_threshold_bytes: rope.len_bytes() as u64,
            ..crate::settings::Settings::default()
        };
        assert!(parse_language(&rope, LanguageId::Rust, &settings).is_empty());
        let unrestricted = crate::settings::Settings {
            large_file_restriction_enabled: false,
            ..settings
        };
        assert!(!parse_language(&rope, LanguageId::Rust, &unrestricted).is_empty());
    }

    #[test]
    fn every_selectable_grammar_highlights_a_small_document() {
        let settings = crate::settings::Settings::default();
        for &language in LanguageId::all() {
            let source = match language {
                LanguageId::Rust => "fn main() { let answer = 42; }",
                LanguageId::Python => "def answer():\n    return 42\n",
                LanguageId::C => "int main(void) { return 42; }",
                LanguageId::CSharp => "class Sample { int answer = 42; }",
                LanguageId::JavaScript => "const answer = 42;",
                LanguageId::TypeScript => "const answer: number = 42;",
                LanguageId::Tsx => "const view = <div className=\"x\">hi</div>;",
                LanguageId::Json => "{\"answer\": 42}",
                LanguageId::Markdown => "# title\n**bold**\n",
                LanguageId::Bash => "if true; then echo \"$HOME\"; fi\n",
                LanguageId::PowerShell => "function Read-Value { $answer = 42; Write-Output $answer }",
                LanguageId::Go => "package main\nfunc main() { println(\"hi\") }",
                LanguageId::Java => "class Sample { public static void main(String[] args) {} }",
                LanguageId::Swift => "let message = \"hi\"",
                LanguageId::Ruby => "def answer\n  42\nend\n",
                LanguageId::Php => "<?php $answer = 42;",
                LanguageId::Cpp => "int main() { return 42; }",
                LanguageId::Lua => "local x = 42; function answer() return x end",
                LanguageId::Html => "<div class=\"x\">hi</div>",
                LanguageId::Css => "body { color: red; font-size: 12px; }",
                LanguageId::Scss => "$primary: red;\nbody { color: $primary; }",
                LanguageId::Yaml => "answer: 42\n",
                LanguageId::Toml => "[project]\nname = \"sample\"\n",
                LanguageId::Xml => "<root attr=\"x\">value</root>",
                LanguageId::Sql => "SELECT name FROM users WHERE id = 42;",
            };
            let rope = ropey::Rope::from_str(source);
            assert!(!parse_language(&rope, language, &settings).is_empty(), "{language:?}");
        }
    }

    #[test]
    fn selected_languages_round_trip_through_the_session_manifest() {
        let mut tab = crate::tab::Tab::empty();
        for language in std::iter::once(None).chain(LanguageId::all().iter().copied().map(Some)) {
            select_language(&mut tab.buffer, language).unwrap();
            let entry = crate::session::snapshot_tab_with_policy(&tab, crate::session::BakPolicy::Full);
            let encoded = toml::to_string(&entry).unwrap();
            let restored: crate::session::SessionTab = toml::from_str(&encoded).unwrap();
            assert_eq!(restored.language, language);
        }
    }
}
