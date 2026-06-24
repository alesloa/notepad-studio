// SPDX-License-Identifier: Apache-2.0
//
// Per-style record. Mirrors Notepad++'s `struct Style` (`Parameters.h:324`)
// but uses `Option<u32>` for unset values instead of NPP's `-1` / `0xFF000000`
// sentinels. Round-trip parity is preserved by `xml_load` / `xml_save`.

use std::fmt;

/// Bitmask flags for which colour channels are active on a `Style`.
/// Mirrors NPP's `COLORSTYLE_FOREGROUND` / `COLORSTYLE_BACKGROUND`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ColourFlags(u8);

impl ColourFlags {
    pub const FOREGROUND: u8 = 0x01;
    pub const BACKGROUND: u8 = 0x02;
    pub const ALL: u8 = 0x03;

    pub const fn new(bits: u8) -> Self {
        Self(bits & Self::ALL)
    }

    pub const fn all() -> Self {
        Self(Self::ALL)
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn has_fg(self) -> bool {
        (self.0 & Self::FOREGROUND) != 0
    }

    pub const fn has_bg(self) -> bool {
        (self.0 & Self::BACKGROUND) != 0
    }

    pub fn set_fg(&mut self, on: bool) {
        if on {
            self.0 |= Self::FOREGROUND;
        } else {
            self.0 &= !Self::FOREGROUND;
        }
    }

    pub fn set_bg(&mut self, on: bool) {
        if on {
            self.0 |= Self::BACKGROUND;
        } else {
            self.0 &= !Self::BACKGROUND;
        }
    }
}

impl Default for ColourFlags {
    fn default() -> Self {
        Self::all()
    }
}

impl fmt::Display for ColourFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Bitmask flags for font weight / style. NPP `FontStyle` enum.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FontFlags(u8);

impl FontFlags {
    pub const NONE: u8 = 0;
    pub const BOLD: u8 = 1;
    pub const ITALIC: u8 = 2;
    pub const UNDERLINE: u8 = 4;
    pub const ALL: u8 = Self::BOLD | Self::ITALIC | Self::UNDERLINE;

    pub const fn new(bits: u8) -> Self {
        Self(bits & Self::ALL)
    }

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn bold(self) -> bool {
        (self.0 & Self::BOLD) != 0
    }

    pub const fn italic(self) -> bool {
        (self.0 & Self::ITALIC) != 0
    }

    pub const fn underline(self) -> bool {
        (self.0 & Self::UNDERLINE) != 0
    }

    pub fn set_bold(&mut self, on: bool) {
        self.set_bit(Self::BOLD, on);
    }

    pub fn set_italic(&mut self, on: bool) {
        self.set_bit(Self::ITALIC, on);
    }

    pub fn set_underline(&mut self, on: bool) {
        self.set_bit(Self::UNDERLINE, on);
    }

    fn set_bit(&mut self, bit: u8, on: bool) {
        if on {
            self.0 |= bit;
        } else {
            self.0 &= !bit;
        }
    }
}

impl fmt::Display for FontFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Sentinel: NPP encodes "colour not set" as `0xFF000000` (alpha byte = 0xFF).
/// We expose the helper for `xml_load` / `xml_save` parity.
pub const COLOUR_UNSET_HIGH_BYTE: u32 = 0xFF00_0000;

/// One row of a theme — either a `<WordsStyle>` (per-language) or a
/// `<WidgetStyle>` (Global Styles).
///
/// All optional attributes use `Option<T>`. `None` round-trips as either an
/// omitted XML attribute or NPP's `0xFF000000` / `-1` sentinel, depending on
/// what the original document used (see `xml_save`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Style {
    /// Display label (e.g. `"COMMENT"`, `"Caret colour"`).
    pub name: String,
    /// Scintilla style ID (or 0 for non-Scintilla widgets identified by name).
    pub style_id: i32,
    /// Foreground RGB (low 24 bits). `None` = not set.
    pub fg_color: Option<u32>,
    /// Background RGB (low 24 bits). `None` = not set.
    pub bg_color: Option<u32>,
    /// Which channels are live; defaults to ALL (fg+bg) per NPP.
    pub colour_style: ColourFlags,
    /// Font family. Empty / `None` = inherit Default Style.
    pub font_name: Option<String>,
    /// Font weight bitmask. `None` = inherit.
    pub font_style: Option<FontFlags>,
    /// Font point size. `None` = inherit. `Some(0)` = explicit 0 (NPP writes "").
    pub font_size: Option<u32>,
    /// Extra-style bits applied when nested. Rarely set.
    pub nesting: u32,
    /// Keyword class index (`instre1`/`instre2`/`type1..7`/`substyle1..8`).
    /// `None` when the style does not declare a keyword class.
    pub keyword_class: Option<i32>,
    /// Custom keyword list (text-content child of the XML element).
    pub keywords: Option<String>,
}

impl Style {
    /// Build a `Style` representing a "not used" sentinel. Equivalent to NPP's
    /// `Style{}` default-construction at `Parameters.h:324`.
    pub fn empty() -> Self {
        Self {
            name: String::new(),
            style_id: STYLE_NOT_USED,
            fg_color: None,
            bg_color: None,
            colour_style: ColourFlags::all(),
            font_name: None,
            font_style: None,
            font_size: None,
            nesting: 0,
            keyword_class: None,
            keywords: None,
        }
    }

    /// Convenience: a style with only a name + id, no colours/fonts.
    pub fn named(style_id: i32, name: impl Into<String>) -> Self {
        let mut s = Self::empty();
        s.style_id = style_id;
        s.name = name.into();
        s
    }
}

/// NPP's `STYLE_NOT_USED = -1` sentinel, kept for round-trip semantics.
pub const STYLE_NOT_USED: i32 = -1;

/// `0x80` is NPP's `L_USER` mask used in user-style sub-style IDs (high 16
/// bits). The parser preserves the flag and exposes it here.
pub const L_USER_FLAG: u32 = 0x0080_0000;

/// Range of user-keyword sub-style IDs (low 16 bits) per NPP.
pub const USER_SUBSTYLE_FIRST: i32 = 128;
pub const USER_SUBSTYLE_LAST: i32 = 135;

/// Parse a 6-hex-digit RGB string into an RGB integer. NPP also accepts the
/// `0xFF000000` "not set" sentinel — callers handle the mapping to `None`.
pub fn parse_hex_rgb(input: &str) -> Option<u32> {
    let s = input.trim();
    if s.is_empty() || s.len() != 6 {
        return None;
    }
    u32::from_str_radix(s, 16).ok().map(|n| n & 0x00FF_FFFF)
}

/// Format an RGB integer as a 6-digit upper-hex string, matching NPP's
/// `%.6lX` output style.
pub fn format_hex_rgb(rgb: u32) -> String {
    format!("{:06X}", rgb & 0x00FF_FFFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_flags_round_trip() {
        let mut f = FontFlags::empty();
        f.set_bold(true);
        f.set_italic(true);
        assert!(f.bold() && f.italic() && !f.underline());
        assert_eq!(f.bits(), 3);
        f.set_bold(false);
        assert!(!f.bold() && f.italic());
    }

    #[test]
    fn colour_flags_default_is_all() {
        assert_eq!(ColourFlags::default().bits(), ColourFlags::ALL);
    }

    #[test]
    fn parse_and_format_hex_rgb() {
        assert_eq!(parse_hex_rgb("FF8000"), Some(0x00FF_8000));
        assert_eq!(parse_hex_rgb("ff8000"), Some(0x00FF_8000));
        assert_eq!(parse_hex_rgb(""), None);
        assert_eq!(parse_hex_rgb("nothex"), None);
        assert_eq!(format_hex_rgb(0x00FF_8000), "FF8000");
        assert_eq!(format_hex_rgb(0xFFFF_8000), "FF8000"); // alpha stripped
    }

    #[test]
    fn empty_style_uses_sentinel() {
        let s = Style::empty();
        assert_eq!(s.style_id, STYLE_NOT_USED);
        assert!(s.fg_color.is_none());
        assert!(s.bg_color.is_none());
        assert_eq!(s.colour_style, ColourFlags::all());
    }
}
