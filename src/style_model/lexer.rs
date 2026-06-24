// SPDX-License-Identifier: Apache-2.0
//
// `LexerStyler` and `LexerStylerArray` — per-language groupings of `Style`
// entries. Mirror NPP `Parameters.h` `struct LexerStyler` /
// `struct LexerStylerArray`.

use crate::style_model::style::Style;

/// Per-language style group: lexer name + its `Vec<Style>`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LexerStyler {
    /// Lookup key (e.g. `"cpp"`). Matches `<LexerType name="...">`.
    pub lexer_name: String,
    /// Display label (e.g. `"C++"`). Matches `<LexerType desc="...">`.
    pub lexer_desc: String,
    /// User-defined extension override (`<LexerType ext="...">`). May be empty.
    pub lexer_user_ext: String,
    /// Whether the lexer is hidden from menus (`<LexerType excluded="yes">`).
    pub excluded: bool,
    /// Per-style entries from `<WordsStyle>` children.
    pub styles: Vec<Style>,
}

impl LexerStyler {
    pub fn new(name: impl Into<String>, desc: impl Into<String>) -> Self {
        Self {
            lexer_name: name.into(),
            lexer_desc: desc.into(),
            lexer_user_ext: String::new(),
            excluded: false,
            styles: Vec::new(),
        }
    }

    pub fn find_by_id(&self, style_id: i32) -> Option<&Style> {
        self.styles.iter().find(|s| s.style_id == style_id)
    }

    pub fn find_by_id_mut(&mut self, style_id: i32) -> Option<&mut Style> {
        self.styles.iter_mut().find(|s| s.style_id == style_id)
    }

    pub fn find_by_name(&self, name: &str) -> Option<&Style> {
        self.styles.iter().find(|s| s.name == name)
    }

    pub fn find_by_name_mut(&mut self, name: &str) -> Option<&mut Style> {
        self.styles.iter_mut().find(|s| s.name == name)
    }
}

/// Top-level container for every `<LexerType>` block in the loaded theme XML.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LexerStylerArray {
    pub lexers: Vec<LexerStyler>,
}

impl LexerStylerArray {
    pub fn new() -> Self {
        Self { lexers: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.lexers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lexers.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&LexerStyler> {
        self.lexers.get(index)
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut LexerStyler> {
        self.lexers.get_mut(index)
    }

    pub fn push(&mut self, lexer: LexerStyler) {
        self.lexers.push(lexer);
    }

    pub fn find(&self, lexer_name: &str) -> Option<&LexerStyler> {
        self.lexers.iter().find(|l| l.lexer_name == lexer_name)
    }

    pub fn find_mut(&mut self, lexer_name: &str) -> Option<&mut LexerStyler> {
        self.lexers.iter_mut().find(|l| l.lexer_name == lexer_name)
    }

    /// Sort lexers alphabetically by `lexer_desc`, case-insensitive,
    /// pinning `"Search result"` to the bottom (NPP parity).
    pub fn sort(&mut self) {
        self.lexers.sort_by(|a, b| {
            let a_search = a.lexer_desc.eq_ignore_ascii_case("Search result");
            let b_search = b.lexer_desc.eq_ignore_ascii_case("Search result");
            match (a_search, b_search) {
                (true, false) => std::cmp::Ordering::Greater,
                (false, true) => std::cmp::Ordering::Less,
                _ => a.lexer_desc.to_lowercase().cmp(&b.lexer_desc.to_lowercase()),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_pins_search_result_last() {
        let mut arr = LexerStylerArray::new();
        arr.push(LexerStyler::new("xml", "XML"));
        arr.push(LexerStyler::new("searchResult", "Search result"));
        arr.push(LexerStyler::new("cpp", "C++"));
        arr.sort();
        assert_eq!(arr.lexers[0].lexer_name, "cpp");
        assert_eq!(arr.lexers[1].lexer_name, "xml");
        assert_eq!(arr.lexers[2].lexer_name, "searchResult");
    }

    #[test]
    fn lookup_by_name() {
        let mut arr = LexerStylerArray::new();
        arr.push(LexerStyler::new("cpp", "C++"));
        assert!(arr.find("cpp").is_some());
        assert!(arr.find("none").is_none());
    }

    #[test]
    fn style_lookup_by_name_and_id() {
        let mut lex = LexerStyler::new("cpp", "C++");
        lex.styles.push(Style::named(1, "COMMENT"));
        lex.styles.push(Style::named(5, "KEYWORD"));
        assert!(lex.find_by_name("COMMENT").is_some());
        assert!(lex.find_by_id(5).is_some());
        assert!(lex.find_by_name("nope").is_none());
        assert!(lex.find_by_id(99).is_none());
    }
}
