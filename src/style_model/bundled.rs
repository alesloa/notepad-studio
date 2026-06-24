// SPDX-License-Identifier: Apache-2.0
//
// Bundled themes — embedded into the binary via `include_str!`. These XML
// strings are the source of truth on first launch; user edits are saved as
// XML files under `<userdata>/themes/<name>.xml` (the save-redirect mirrors
// NPP's `ThemeSwitcher::addThemeStylerSavePath`).

/// `(display_name, xml_string)` pairs. Order is preservation order in the
/// theme list; alphabetisation happens in `theme_switcher`.
pub const BUNDLED_THEMES: &[(&str, &str)] = &[
    (
        "NotepadStudioLight",
        include_str!("../../assets/themes/NotepadStudioLight.xml"),
    ),
    (
        "NotepadStudioDark",
        include_str!("../../assets/themes/NotepadStudioDark.xml"),
    ),
    (
        "Solarized",
        include_str!("../../assets/themes/Solarized.xml"),
    ),
    (
        "Dracula",
        include_str!("../../assets/themes/Dracula.xml"),
    ),
    (
        "Nord",
        include_str!("../../assets/themes/Nord.xml"),
    ),
    (
        "Gruvbox",
        include_str!("../../assets/themes/Gruvbox.xml"),
    ),
    (
        "Monokai",
        include_str!("../../assets/themes/Monokai.xml"),
    ),
];

/// Look up a bundled theme by name.
pub fn get(name: &str) -> Option<&'static str> {
    BUNDLED_THEMES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, xml)| *xml)
}

/// Return every bundled theme name (display order).
pub fn names() -> impl Iterator<Item = &'static str> {
    BUNDLED_THEMES.iter().map(|(n, _)| *n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_and_dark_exist() {
        assert!(get("NotepadStudioLight").is_some());
        assert!(get("NotepadStudioDark").is_some());
        assert!(get("Solarized").is_some());
        assert!(get("Dracula").is_some());
        assert!(get("Nord").is_some());
        assert!(get("Gruvbox").is_some());
        assert!(get("Monokai").is_some());
        assert!(get("Nonexistent").is_none());
    }

    #[test]
    fn names_match_table() {
        let collected: Vec<&str> = names().collect();
        assert_eq!(
            collected,
            vec![
                "NotepadStudioLight",
                "NotepadStudioDark",
                "Solarized",
                "Dracula",
                "Nord",
                "Gruvbox",
                "Monokai",
            ]
        );
    }

    #[test]
    fn bundled_themes_parse_cleanly() {
        for (name, xml) in BUNDLED_THEMES {
            let parsed = crate::style_model::xml_load::parse(xml)
                .unwrap_or_else(|e| panic!("bundled theme {name} failed to parse: {e:?}"));
            assert!(
                !parsed.global_styles.is_empty(),
                "{name} must have global styles"
            );
        }
    }

    #[test]
    fn bundled_round_trip() {
        // Load → serialize → load again → semantic equality on the second
        // round (exact byte-equality is not required; we want stable model).
        for (name, xml) in BUNDLED_THEMES {
            let first =
                crate::style_model::xml_load::parse(xml).expect("first parse");
            let written = crate::style_model::xml_save::write(&first);
            let second = crate::style_model::xml_load::parse(&written)
                .expect("second parse");
            assert_eq!(
                first, second,
                "{name} did not survive load → save → load round-trip"
            );
        }
    }
}
