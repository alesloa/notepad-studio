// SPDX-License-Identifier: Apache-2.0
//
// The 53-row Global Styles table — the exact strings shown in the Style
// listbox when "Global Styles" is selected in the Language combo.
//
// Each entry has:
//   - the display name (stored in `<WidgetStyle name="...">`),
//   - the Scintilla style ID (or 0 for non-Scintilla widgets identified by name),
//   - a `Role` enum tagging where the value is consumed in the editor.

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// `STYLE_DEFAULT` (32) — base for inheritance.
    DefaultStyle,
    /// `STYLE_INDENTGUIDE` (37).
    IndentGuide,
    /// `STYLE_BRACELIGHT` (34).
    BraceLight,
    /// `STYLE_BRACEBAD` (35).
    BraceBad,
    /// `STYLE_LINENUMBER` (33).
    LineNumber,
    /// Non-Scintilla widget — caret colour.
    Caret,
    /// Caret line background.
    CurrentLine,
    /// Selection foreground / background.
    Selection,
    MultiSelection,
    MultiCaret,
    /// Edge column (long-line indicator).
    Edge,
    BookmarkMargin,
    /// Change-history sub-roles.
    ChangeHistoryMargin,
    ChangeHistoryModified,
    ChangeHistoryRevertModified,
    ChangeHistoryRevertOrigin,
    ChangeHistorySaved,
    Fold,
    FoldActive,
    FoldMargin,
    Whitespace,
    /// Smart Highlighting indicator (id 29).
    SmartHighlight,
    /// Find Mark Style (id 31).
    FindMark,
    FindStatusNotFound,
    FindStatusMessage,
    FindStatusSearchEnd,
    /// Mark indicator slots 1..5 (ids 25, 24, 23, 22, 21 respectively).
    MarkStyle(u8),
    /// Incremental highlight indicator (id 28).
    IncrementalHighlight,
    /// Tag matching indicators.
    TagsMatch,
    TagsAttribute,
    /// Tab-bar widgets.
    ActiveTabFocused,
    ActiveTabUnfocused,
    ActiveTabText,
    InactiveTabs,
    /// Per-tab palette slots (light / dark). `1..=5`.
    TabColor(u8),
    TabColorDark(u8),
    /// Hovered URL fg.
    UrlHovered,
    /// Document map (minimap) fg / bg.
    DocumentMap,
    EolCustom,
    NonPrintingCustom,
    /// Special: drives the seven `Enable global *` checkboxes.
    GlobalOverride,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlobalStyleEntry {
    pub name: &'static str,
    pub style_id: i32,
    pub role: Role,
}

/// The full 53-row table, in the order the listbox shows.
pub const GLOBAL_STYLE_ENTRIES: &[GlobalStyleEntry] = &[
    GlobalStyleEntry { name: "Default Style",                       style_id: 32, role: Role::DefaultStyle },
    GlobalStyleEntry { name: "Indent guideline style",              style_id: 37, role: Role::IndentGuide },
    GlobalStyleEntry { name: "Brace highlight style",               style_id: 34, role: Role::BraceLight },
    GlobalStyleEntry { name: "Bad brace colour",                    style_id: 35, role: Role::BraceBad },
    GlobalStyleEntry { name: "Current line background colour",      style_id: 0,  role: Role::CurrentLine },
    GlobalStyleEntry { name: "Selected text colour",                style_id: 0,  role: Role::Selection },
    GlobalStyleEntry { name: "Multi-selected text color",           style_id: 0,  role: Role::MultiSelection },
    GlobalStyleEntry { name: "Caret colour",                        style_id: 0,  role: Role::Caret },
    GlobalStyleEntry { name: "Multi-edit carets color",             style_id: 0,  role: Role::MultiCaret },
    GlobalStyleEntry { name: "Edge colour",                         style_id: 0,  role: Role::Edge },
    GlobalStyleEntry { name: "Line number margin",                  style_id: 33, role: Role::LineNumber },
    GlobalStyleEntry { name: "Bookmark margin",                     style_id: 0,  role: Role::BookmarkMargin },
    GlobalStyleEntry { name: "Change History margin",               style_id: 0,  role: Role::ChangeHistoryMargin },
    GlobalStyleEntry { name: "Change History modified",             style_id: 0,  role: Role::ChangeHistoryModified },
    GlobalStyleEntry { name: "Change History revert modified",      style_id: 0,  role: Role::ChangeHistoryRevertModified },
    GlobalStyleEntry { name: "Change History revert origin",        style_id: 0,  role: Role::ChangeHistoryRevertOrigin },
    GlobalStyleEntry { name: "Change History saved",                style_id: 0,  role: Role::ChangeHistorySaved },
    GlobalStyleEntry { name: "Fold",                                style_id: 0,  role: Role::Fold },
    GlobalStyleEntry { name: "Fold active",                         style_id: 0,  role: Role::FoldActive },
    GlobalStyleEntry { name: "Fold margin",                         style_id: 0,  role: Role::FoldMargin },
    GlobalStyleEntry { name: "White space symbol",                  style_id: 0,  role: Role::Whitespace },
    GlobalStyleEntry { name: "Smart Highlighting",                  style_id: 29, role: Role::SmartHighlight },
    GlobalStyleEntry { name: "Find Mark Style",                     style_id: 31, role: Role::FindMark },
    GlobalStyleEntry { name: "Find status: Not found",              style_id: 0,  role: Role::FindStatusNotFound },
    GlobalStyleEntry { name: "Find status: Message",                style_id: 0,  role: Role::FindStatusMessage },
    GlobalStyleEntry { name: "Find status: Search end reached",     style_id: 0,  role: Role::FindStatusSearchEnd },
    GlobalStyleEntry { name: "Mark Style 1",                        style_id: 25, role: Role::MarkStyle(1) },
    GlobalStyleEntry { name: "Mark Style 2",                        style_id: 24, role: Role::MarkStyle(2) },
    GlobalStyleEntry { name: "Mark Style 3",                        style_id: 23, role: Role::MarkStyle(3) },
    GlobalStyleEntry { name: "Mark Style 4",                        style_id: 22, role: Role::MarkStyle(4) },
    GlobalStyleEntry { name: "Mark Style 5",                        style_id: 21, role: Role::MarkStyle(5) },
    GlobalStyleEntry { name: "Incremental highlight all",           style_id: 28, role: Role::IncrementalHighlight },
    GlobalStyleEntry { name: "Tags match highlighting",             style_id: 27, role: Role::TagsMatch },
    GlobalStyleEntry { name: "Tags attribute",                      style_id: 26, role: Role::TagsAttribute },
    GlobalStyleEntry { name: "Active tab focused indicator",        style_id: 0,  role: Role::ActiveTabFocused },
    GlobalStyleEntry { name: "Active tab unfocused indicator",      style_id: 0,  role: Role::ActiveTabUnfocused },
    GlobalStyleEntry { name: "Active tab text",                     style_id: 0,  role: Role::ActiveTabText },
    GlobalStyleEntry { name: "Inactive tabs",                       style_id: 0,  role: Role::InactiveTabs },
    GlobalStyleEntry { name: "Tab color 1",                         style_id: 0,  role: Role::TabColor(1) },
    GlobalStyleEntry { name: "Tab color 2",                         style_id: 0,  role: Role::TabColor(2) },
    GlobalStyleEntry { name: "Tab color 3",                         style_id: 0,  role: Role::TabColor(3) },
    GlobalStyleEntry { name: "Tab color 4",                         style_id: 0,  role: Role::TabColor(4) },
    GlobalStyleEntry { name: "Tab color 5",                         style_id: 0,  role: Role::TabColor(5) },
    GlobalStyleEntry { name: "Tab color dark mode 1",               style_id: 0,  role: Role::TabColorDark(1) },
    GlobalStyleEntry { name: "Tab color dark mode 2",               style_id: 0,  role: Role::TabColorDark(2) },
    GlobalStyleEntry { name: "Tab color dark mode 3",               style_id: 0,  role: Role::TabColorDark(3) },
    GlobalStyleEntry { name: "Tab color dark mode 4",               style_id: 0,  role: Role::TabColorDark(4) },
    GlobalStyleEntry { name: "Tab color dark mode 5",               style_id: 0,  role: Role::TabColorDark(5) },
    GlobalStyleEntry { name: "URL hovered",                         style_id: 0,  role: Role::UrlHovered },
    GlobalStyleEntry { name: "Document map",                        style_id: 0,  role: Role::DocumentMap },
    GlobalStyleEntry { name: "EOL custom color",                    style_id: 0,  role: Role::EolCustom },
    GlobalStyleEntry { name: "Non-printing characters custom color", style_id: 0, role: Role::NonPrintingCustom },
    GlobalStyleEntry { name: "Global override",                     style_id: 0,  role: Role::GlobalOverride },
];

/// Look up a `Role` for a given Global Styles display name. Used by the
/// dialog when wiring a `<WidgetStyle>` to its renderer hook.
pub fn role_for(name: &str) -> Option<Role> {
    GLOBAL_STYLE_ENTRIES
        .iter()
        .find(|e| e.name == name)
        .map(|e| e.role)
}

/// Find the canonical entry for a name (preserves case-sensitive XML key).
pub fn entry_for(name: &str) -> Option<GlobalStyleEntry> {
    GLOBAL_STYLE_ENTRIES.iter().copied().find(|e| e.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_table_size() {
        assert_eq!(GLOBAL_STYLE_ENTRIES.len(), 53);
    }

    #[test]
    fn names_unique() {
        let mut names: Vec<&str> = GLOBAL_STYLE_ENTRIES.iter().map(|e| e.name).collect();
        names.sort();
        let count_before = names.len();
        names.dedup();
        assert_eq!(names.len(), count_before, "names must be unique");
    }

    #[test]
    fn role_lookup() {
        assert_eq!(role_for("Default Style"), Some(Role::DefaultStyle));
        assert_eq!(role_for("Caret colour"), Some(Role::Caret));
        assert_eq!(role_for("Tab color 3"), Some(Role::TabColor(3)));
        assert_eq!(role_for("Global override"), Some(Role::GlobalOverride));
        assert_eq!(role_for("nonexistent"), None);
    }

    #[test]
    fn ordered_known_landmarks() {
        // Sanity-check a few absolute positions to catch table re-ordering bugs.
        assert_eq!(GLOBAL_STYLE_ENTRIES[0].name, "Default Style");
        assert_eq!(GLOBAL_STYLE_ENTRIES[7].name, "Caret colour");
        assert_eq!(
            GLOBAL_STYLE_ENTRIES.last().map(|e| e.name),
            Some("Global override")
        );
    }
}
