// SPDX-License-Identifier: Apache-2.0
//
// Per-checkbox flags shown when "Global Styles ▸ Global override" is selected.
// When `enable_*` is true, the named widget's value overrides the matching
// attribute across every style. Mirrors NPP's `struct GlobalOverride`
// (`Parameters.h:345`).

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GlobalOverride {
    #[serde(default)]
    pub enable_fg: bool,
    #[serde(default)]
    pub enable_bg: bool,
    #[serde(default)]
    pub enable_font: bool,
    #[serde(default)]
    pub enable_font_size: bool,
    #[serde(default)]
    pub enable_bold: bool,
    #[serde(default)]
    pub enable_italic: bool,
    #[serde(default)]
    pub enable_underline: bool,
}

impl GlobalOverride {
    pub const fn new() -> Self {
        Self {
            enable_fg: false,
            enable_bg: false,
            enable_font: false,
            enable_font_size: false,
            enable_bold: false,
            enable_italic: false,
            enable_underline: false,
        }
    }

    /// True when at least one override flag is set.
    pub const fn is_enabled(&self) -> bool {
        self.enable_fg
            || self.enable_bg
            || self.enable_font
            || self.enable_font_size
            || self.enable_bold
            || self.enable_italic
            || self.enable_underline
    }

    /// Compact bitmask (purely for debug / testing, not persisted).
    pub const fn bits(&self) -> u8 {
        (self.enable_fg as u8)
            | ((self.enable_bg as u8) << 1)
            | ((self.enable_font as u8) << 2)
            | ((self.enable_font_size as u8) << 3)
            | ((self.enable_bold as u8) << 4)
            | ((self.enable_italic as u8) << 5)
            | ((self.enable_underline as u8) << 6)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_all_off() {
        let g = GlobalOverride::default();
        assert!(!g.is_enabled());
        assert_eq!(g.bits(), 0);
    }

    #[test]
    fn bits_layout() {
        let mut g = GlobalOverride::default();
        g.enable_fg = true;
        g.enable_bold = true;
        assert_eq!(g.bits(), 0b0001_0001);
        assert!(g.is_enabled());
    }
}
