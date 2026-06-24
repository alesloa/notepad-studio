// SPDX-License-Identifier: Apache-2.0
//
// Style Configurator data model. Mirrors Notepad++'s `<NotepadPlus>` theme XML
// schema and the runtime structures it parses into. Pure data + I/O — no UI.

#![allow(dead_code, unused_imports)]

pub mod active_theme;
pub mod bundled;
pub mod global_override;
pub mod global_styles_list;
pub mod keyword_class;
pub mod lang_enum;
pub mod lexer;
pub mod paths;
pub mod style;
pub mod theme_switcher;
pub mod xml_load;
pub mod xml_save;

pub use global_override::GlobalOverride;
pub use global_styles_list::{GlobalStyleEntry, Role, GLOBAL_STYLE_ENTRIES};
pub use keyword_class::KeywordClass;
pub use lang_enum::{LangType, ALL as ALL_LANG_TYPES};
pub use lexer::{LexerStyler, LexerStylerArray};
pub use style::{ColourFlags, FontFlags, Style, STYLE_NOT_USED};
pub use theme_switcher::{ThemeEntry, ThemeSource};
pub use xml_load::{parse, ParseError, ThemeDocument};
pub use xml_save::write;
