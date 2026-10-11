// PowerEditor/src/Notepad_plus.rc:998-1094 — Language menu labels and order.
// Only installed grammars are listed; Markdown, SCSS and TSX are Studio modes.

use gpui::{Menu, MenuItem};

use crate::actions::*;
use crate::syntax::LanguageId;
use super::MenuState;

pub(crate) fn build(state: &MenuState) -> Menu {
    let selected = |language| state.has_document && state.language == language;
    Menu {
        name: "&Language".into(),
        items: vec![
            MenuItem::action("None (Normal Text)", LanguageNormalText).checked(selected(None)),
            MenuItem::separator(),
            MenuItem::action("C", LanguageC).checked(selected(Some(LanguageId::C))),
            MenuItem::action("C#", LanguageCSharp).checked(selected(Some(LanguageId::CSharp))),
            MenuItem::action("C++", LanguageCpp).checked(selected(Some(LanguageId::Cpp))),
            MenuItem::action("CSS", LanguageCss).checked(selected(Some(LanguageId::Css))),
            MenuItem::action("Go", LanguageGo).checked(selected(Some(LanguageId::Go))),
            MenuItem::action("HTML", LanguageHtml).checked(selected(Some(LanguageId::Html))),
            MenuItem::action("Java", LanguageJava).checked(selected(Some(LanguageId::Java))),
            MenuItem::action("JavaScript", LanguageJavaScript).checked(selected(Some(LanguageId::JavaScript))),
            MenuItem::action("JSON", LanguageJson).checked(selected(Some(LanguageId::Json))),
            MenuItem::action("Lua", LanguageLua).checked(selected(Some(LanguageId::Lua))),
            MenuItem::action("Markdown", LanguageMarkdown).checked(selected(Some(LanguageId::Markdown))),
            MenuItem::action("PHP", LanguagePhp).checked(selected(Some(LanguageId::Php))),
            MenuItem::action("PowerShell", LanguagePowerShell).checked(selected(Some(LanguageId::PowerShell))),
            MenuItem::action("Python", LanguagePython).checked(selected(Some(LanguageId::Python))),
            MenuItem::action("Ruby", LanguageRuby).checked(selected(Some(LanguageId::Ruby))),
            MenuItem::action("Rust", LanguageRust).checked(selected(Some(LanguageId::Rust))),
            MenuItem::action("SCSS", LanguageScss).checked(selected(Some(LanguageId::Scss))),
            MenuItem::action("Shell", LanguageShell).checked(selected(Some(LanguageId::Bash))),
            MenuItem::action("SQL", LanguageSql).checked(selected(Some(LanguageId::Sql))),
            MenuItem::action("Swift", LanguageSwift).checked(selected(Some(LanguageId::Swift))),
            MenuItem::action("TOML", LanguageToml).checked(selected(Some(LanguageId::Toml))),
            MenuItem::action("TSX", LanguageTsx).checked(selected(Some(LanguageId::Tsx))),
            MenuItem::action("TypeScript", LanguageTypeScript).checked(selected(Some(LanguageId::TypeScript))),
            MenuItem::action("XML", LanguageXml).checked(selected(Some(LanguageId::Xml))),
            MenuItem::action("YAML", LanguageYaml).checked(selected(Some(LanguageId::Yaml))),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checked_labels(menu: &Menu) -> Vec<&str> {
        menu.items.iter().filter_map(|item| match item {
            MenuItem::Action { name, checked: true, .. } => Some(name.as_ref()),
            _ => None,
        }).collect()
    }

    #[test]
    fn only_the_active_language_is_checked() {
        for &language in LanguageId::all() {
            let menu = build(&MenuState {
                has_document: true,
                language: Some(language),
                ..MenuState::default()
            });
            assert_eq!(checked_labels(&menu), vec![language.label()]);
        }
        let menu = build(&MenuState { has_document: true, ..MenuState::default() });
        assert_eq!(checked_labels(&menu), vec!["None (Normal Text)"]);
        assert!(checked_labels(&build(&MenuState::default())).is_empty());
    }

    #[test]
    fn supported_modes_follow_the_menu_order_and_are_rebindable() {
        let menu = build(&MenuState::default());
        let names: Vec<&str> = menu.items.iter().filter_map(|item| match item {
            MenuItem::Action { name, .. } => Some(name.as_ref()),
            _ => None,
        }).collect();
        assert_eq!(names, vec![
            "None (Normal Text)", "C", "C#", "C++", "CSS", "Go", "HTML", "Java",
            "JavaScript", "JSON", "Lua", "Markdown", "PHP", "PowerShell", "Python",
            "Ruby", "Rust", "SCSS", "Shell", "SQL", "Swift", "TOML", "TSX",
            "TypeScript", "XML", "YAML",
        ]);
        assert!(matches!(menu.items.get(1), Some(MenuItem::Separator)));
        for item in &menu.items {
            let MenuItem::Action { action, .. } = item else { continue; };
            let entry = crate::shortcuts::catalog().iter()
                .find(|entry| (entry.action)().name() == action.name())
                .expect("Language menu action missing from Shortcut Mapper");
            assert!(entry.keys.is_empty());
            assert!(entry.mac_keys.is_empty());
        }
    }
}
