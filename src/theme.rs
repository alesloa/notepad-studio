use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(crate) enum ThemeMode {
    #[serde(rename = "dark")]
    Dark,
    #[serde(rename = "light")]
    Light,
    /// Mirrors the OS appearance — Light when the system is in Light Mode,
    /// Dark when the system is in Dark Mode (or any vibrant variant). The
    /// resolution happens in `Workspace::theme()` against the cached
    /// `WindowAppearance` updated by `cx.observe_window_appearance`.
    #[serde(rename = "follow_system")]
    FollowSystem,
}

impl ThemeMode {
    /// Cycle Light→Dark→FollowSystem→Light. Used by the `Cmd/Ctrl+Shift+T`
    /// toggle so power users can rotate through every option without the
    /// dialog.
    pub(crate) fn flipped(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::FollowSystem,
            Self::FollowSystem => Self::Dark,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::FollowSystem => "Follow System",
        }
    }

    /// Resolve a `FollowSystem` mode against the cached `WindowAppearance`.
    /// Light/Dark pass through unchanged. Vibrant variants of the system
    /// appearance map to their non-vibrant counterparts.
    pub(crate) fn resolved(self, appearance: gpui::WindowAppearance) -> ResolvedMode {
        match self {
            Self::Dark => ResolvedMode::Dark,
            Self::Light => ResolvedMode::Light,
            Self::FollowSystem => match appearance {
                gpui::WindowAppearance::Light | gpui::WindowAppearance::VibrantLight => {
                    ResolvedMode::Light
                }
                gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark => {
                    ResolvedMode::Dark
                }
            },
        }
    }
}

/// Effective Light/Dark choice after resolving a `FollowSystem` mode.
/// Distinct type so call sites can't accidentally pass an unresolved
/// `ThemeMode` into `Theme::for_resolved`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ResolvedMode {
    Light,
    Dark,
}

#[derive(Clone, Copy)]
pub(crate) struct Theme {
    pub(crate) bg: u32,
    pub(crate) chrome: u32,
    pub(crate) chrome_text: u32,
    pub(crate) text: u32,
    pub(crate) border: u32,
    pub(crate) gutter_text: u32,
    pub(crate) gutter_bg: u32,
    pub(crate) selection: u32,
    /// Tint painted under every match of the active find query.
    pub(crate) match_highlight: u32,
    /// Stronger tint for the match the cursor is currently on.
    pub(crate) match_highlight_current: u32,
    /// Persistent tint painted under every "Mark All" hit. Distinct from
    /// `match_highlight` (the live find tint) so the two highlight layers can
    /// coexist on screen without colliding. Uses a slightly cooler alpha-tinted
    /// shade of the existing `match_highlight` value: same hue family for
    /// palette harmony, lower alpha so a hit currently under the cursor still
    /// reads through. Dark mode reuses `match_highlight_current`'s base hue
    /// (0xb0309a) at 0x46 alpha; light mode reuses `match_highlight_current`'s
    /// base hue (0x8f1faa) at 0x55 alpha — both sourced from existing slots.
    pub(crate) mark_highlight: u32,
    /// Tint painted under every other occurrence of the word at the cursor
    /// when Smart Highlighting is enabled. Distinct hue from match/mark so
    /// the three layers can coexist. Reuses `tab_color_2` (green, already in
    /// both palettes) at low alpha so the editor text stays readable.
    pub(crate) smart_highlight: u32,
    /// Toggle button background when the option is on (match case, etc.).
    pub(crate) toggle_active: u32,
    /// Border tint for inputs that hold an invalid regex.
    pub(crate) error: u32,
    /// Border color used for "group box" frames in the Find dialog (the
    /// labelled rectangles around `Search Mode`, `In selection`, etc.). One
    /// step softer than `border` so multiple nested frames don't get noisy.
    pub(crate) group_box_border: u32,
    /// Fill color of a checkbox / radio when checked or selected. Notepad++
    /// uses the system accent on Windows; we mirror with `toggle_active` on
    /// dark and a slightly punchier value on light to stay legible against
    /// the lighter dialog background.
    pub(crate) checkbox_fill: u32,
    /// Resting background of a Notepad++-style action button (rectangular,
    /// 1px border). One shade above `bg` so the buttons read as raised
    /// against the dialog body.
    pub(crate) button_bg: u32,
    /// Hover background for an action button — slightly lighter than the
    /// resting `button_bg` so the user gets a clear "you can click me" cue
    /// before mouse-down.
    pub(crate) button_hover_bg: u32,
    /// Pressed-state background for an action button — one step *darker*
    /// than the resting `button_bg`, to read as "inset" while the mouse is
    /// held down. Released back to `button_bg` (or `button_hover_bg` if the
    /// pointer is still over the button) when the click ends.
    pub(crate) button_pressed_bg: u32,
    /// Foreground color for disabled controls.
    pub(crate) disabled_text: u32,
    /// Resting fill of the scrollbar thumb.
    pub(crate) scrollbar: u32,
    /// Brighter fill of the scrollbar thumb on hover or while dragging.
    pub(crate) scrollbar_hover: u32,
    /// Foreground color for the "Show Whitespace" overlay glyphs (centered
    /// dot for spaces, arrow for tabs). Faint by design — visible enough to
    /// scan but quiet enough not to fight the actual text. Mirrors the tone
    /// NPP uses on each theme: a low-contrast mid-gray on dark, a
    /// low-contrast light-gray on light.
    pub(crate) whitespace_glyph: u32,
    /// Background tint painted across the full editor width on the line
    /// containing the caret (NPP `Style Configurator ▸ Current line background
    /// colour`). Reads as a subtle band that lets the user track which line
    /// the cursor sits on, including in the gutter. Painted *under* find /
    /// mark / smart / selection highlights so any of those overlays still
    /// read on top. Gated on the `highlight_current_line` preference.
    pub(crate) current_line_bg: u32,
    /// 1px outline color drawn around the bracket under the caret AND its
    /// matching pair. Reuses `toggle_active` — same accent that lights up
    /// match-case / regex / etc. — so the bracket frame reads as a deliberate
    /// "this is the editor's active accent" cue without inventing a fresh
    /// palette slot. Notepad++ uses a similar accent-blue frame in its
    /// default style.
    pub(crate) bracket_match: u32,
    /// Background color of the icon toolbar that sits above the tab strip.
    /// One step lighter than `chrome` on dark / one step darker on light so
    /// the toolbar reads as a distinct band without contrasting with the
    /// title bar above it.
    pub(crate) toolbar_bg: u32,
    /// Foreground tint for toolbar icons in their resting state.
    pub(crate) toolbar_icon: u32,
    /// Foreground tint for toolbar icons whose action is unavailable
    /// (no selection for Cut, no undo history, etc.).
    pub(crate) toolbar_icon_disabled: u32,
    /// Hover background for a toolbar icon button.
    pub(crate) toolbar_hover_bg: u32,
    /// Vertical separator line drawn between toolbar groups.
    pub(crate) toolbar_separator: u32,
    /// Notepad++ "Apply Color to Tab" — five canonical tab tints painted as
    /// a thin top stripe on the tab. Indexed 1..=5; index 0 unused. Same
    /// hex values across dark/light so tabs read consistently when the user
    /// flips theme.
    pub(crate) tab_color_1: u32,
    pub(crate) tab_color_2: u32,
    pub(crate) tab_color_3: u32,
    pub(crate) tab_color_4: u32,
    pub(crate) tab_color_5: u32,
    /// Foreground / underline tint for clickable URL ranges in the editor
    /// and any other "this is a link" surface. NPP's `Tones` palette exposes
    /// a "Link" slot (`SLOT_LINK`) — `dark_with_palette` plumbs the resolved
    /// palette value through here so per-tone customizations apply.
    pub(crate) link: u32,
    /// Editor font family from the theme XML's Default Style `fontName`, set
    /// only when that family is installed (`crate::fonts::installed`). `None`
    /// means the platform monospace (`crate::fonts::mono`).
    pub(crate) editor_font: Option<&'static str>,
    pub(crate) syntax: SyntaxTheme,
}

/// Per-capture token colors used by the tree-sitter highlighter. Captures
/// not listed here (variable, operator, punctuation, …) fall back to the
/// theme's default `text` color.
#[derive(Clone, Copy)]
pub(crate) struct SyntaxTheme {
    pub(crate) keyword: u32,
    pub(crate) string: u32,
    pub(crate) comment: u32,
    pub(crate) number: u32,
    pub(crate) type_: u32,
    pub(crate) function: u32,
    pub(crate) constant: u32,
    pub(crate) property: u32,
    pub(crate) attribute: u32,
}

// --- tone palette (NPP `Dark Mode ▸ Tones`) ------------------------------

/// One of NPP's eight tone presets in `Dark Mode ▸ Tones`. Light mode is
/// unaffected by tone — only the resolved Dark theme reads the palette.
/// Source: `notepad-plus-plus/PowerEditor/src/NppDarkMode.h:65-76`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(crate) enum TonePalette {
    #[serde(rename = "black")]
    Black,
    #[serde(rename = "red")]
    Red,
    #[serde(rename = "green")]
    Green,
    #[serde(rename = "blue")]
    Blue,
    #[serde(rename = "purple")]
    Purple,
    #[serde(rename = "cyan")]
    Cyan,
    #[serde(rename = "olive")]
    Olive,
    #[serde(rename = "customized")]
    Customized,
}

impl Default for TonePalette {
    fn default() -> Self {
        Self::Black
    }
}

impl TonePalette {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Black => "Black",
            Self::Red => "Red",
            Self::Green => "Green",
            Self::Blue => "Blue",
            Self::Purple => "Purple",
            Self::Cyan => "Cyan",
            Self::Olive => "Olive",
            Self::Customized => "Customized",
        }
    }

}

/// Twelve-slot palette mirroring NPP's `Colors` struct
/// (`NppDarkMode.h:Colors`). Slot order matches NPP's customized-color UI:
/// Top, Menu hot track, Active, Main, Error, Text, Darker text, Disabled
/// text, Link, Edge, Edge highlight, Edge disabled.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PaletteColors {
    pub(crate) top: u32,
    pub(crate) menu_hot_track: u32,
    pub(crate) active: u32,
    pub(crate) main: u32,
    pub(crate) error: u32,
    pub(crate) text: u32,
    pub(crate) darker_text: u32,
    pub(crate) disabled_text: u32,
    pub(crate) link: u32,
    pub(crate) edge: u32,
    pub(crate) edge_highlight: u32,
    pub(crate) edge_disabled: u32,
}

impl PaletteColors {
    /// NPP `darkColors` (NppDarkMode.cpp:192-205). Forms the base for every
    /// tone — non-Black tones add their per-slot offset on top of this.
    pub(crate) const BLACK: Self = Self {
        top: 0x202020,
        menu_hot_track: 0x383838,
        active: 0x454545,
        main: 0x202020,
        error: 0xB00000,
        text: 0xE0E0E0,
        darker_text: 0xC0C0C0,
        disabled_text: 0x808080,
        link: 0xFFFF00,
        edge: 0x646464,
        edge_highlight: 0x9B9B9B,
        edge_disabled: 0x484848,
    };
}

/// NPP `offsetEdge` (NppDarkMode.cpp:207). Added to `edge` on every
/// non-Black tone to keep the edge legible against the tinted background.
const OFFSET_EDGE: u32 = 0x1C1C1C;

/// Slot indices used by the customized-slot override array. Order matches
/// NPP's `getDarkModeDefaultColors` field order so a user-provided override
/// at index N aligns with NPP's slot N.
pub(crate) const SLOT_TOP: usize = 0;
pub(crate) const SLOT_MENU_HOT_TRACK: usize = 1;
pub(crate) const SLOT_ACTIVE: usize = 2;
pub(crate) const SLOT_MAIN: usize = 3;
pub(crate) const SLOT_ERROR: usize = 4;
pub(crate) const SLOT_TEXT: usize = 5;
pub(crate) const SLOT_DARKER_TEXT: usize = 6;
pub(crate) const SLOT_DISABLED_TEXT: usize = 7;
pub(crate) const SLOT_LINK: usize = 8;
pub(crate) const SLOT_EDGE: usize = 9;
pub(crate) const SLOT_EDGE_HIGHLIGHT: usize = 10;
pub(crate) const SLOT_EDGE_DISABLED: usize = 11;
pub(crate) const SLOT_COUNT: usize = 12;

/// Resolve a tone palette from NPP's offset model. Black returns the base
/// directly; every other preset adds the per-tone offset to background-,
/// edge-, and active- family slots while leaving text/error/link untouched.
///
/// NPP-parity values (NppDarkMode.cpp:207) sit at +0x10/+0x20 per channel,
/// which on Win32's anti-aliased panel chrome reads as a subtle but
/// recognizable tint. On gpui's flat-fill rendering the same values are
/// almost imperceptible against a 0x202020 base — switching tones produces
/// no visible change, which the user reports as broken. We push the
/// offsets to roughly 4–6× NPP's values so each tone is unmistakable on
/// every chrome surface (toolbar, tab strip, status bar, group-box border,
/// dialog body) without losing the dark-mode character: a Red base.top of
/// 0x502020 reads as a clear dark crimson rather than the original 0x302020
/// which was within rounding distance of plain Black 0x202020.
pub(crate) fn palette_for_tone(tone: TonePalette) -> PaletteColors {
    let base = PaletteColors::BLACK;
    let bg_offset: u32 = match tone {
        TonePalette::Black | TonePalette::Customized => 0x000000,
        TonePalette::Red => 0x300000,
        TonePalette::Green => 0x002800,
        TonePalette::Blue => 0x000060,
        TonePalette::Purple => 0x280050,
        TonePalette::Cyan => 0x003050,
        TonePalette::Olive => 0x282800,
    };
    let edge_offset = if matches!(tone, TonePalette::Black | TonePalette::Customized) {
        0
    } else {
        OFFSET_EDGE + bg_offset
    };
    PaletteColors {
        top: saturating_add_rgb(base.top, bg_offset),
        menu_hot_track: saturating_add_rgb(base.menu_hot_track, bg_offset),
        active: saturating_add_rgb(base.active, bg_offset),
        main: saturating_add_rgb(base.main, bg_offset),
        error: base.error,
        text: base.text,
        darker_text: base.darker_text,
        disabled_text: base.disabled_text,
        link: base.link,
        edge: saturating_add_rgb(base.edge, edge_offset),
        edge_highlight: saturating_add_rgb(base.edge_highlight, bg_offset),
        edge_disabled: saturating_add_rgb(base.edge_disabled, bg_offset),
    }
}

/// Per-channel saturating add for 0xRRGGBB values. NPP relies on natural
/// COLORREF wraparound never tripping because every offset is small; we
/// saturate explicitly so a hand-edited customized override pinned at
/// 0xFF in a channel doesn't roll back to 0 when a tone offset is applied.
fn saturating_add_rgb(rgb: u32, offset: u32) -> u32 {
    let r = ((rgb >> 16) & 0xFF) as u16 + ((offset >> 16) & 0xFF) as u16;
    let g = ((rgb >> 8) & 0xFF) as u16 + ((offset >> 8) & 0xFF) as u16;
    let b = (rgb & 0xFF) as u16 + (offset & 0xFF) as u16;
    let r = r.min(0xFF) as u32;
    let g = g.min(0xFF) as u32;
    let b = b.min(0xFF) as u32;
    (r << 16) | (g << 8) | b
}

/// Apply the user's customized-slot overrides on top of the Customized
/// tone's base palette. NPP-parity: overrides only meaningful under the
/// Customized tone preset — every other tone paints its preset palette
/// untouched. Slots with `None` fall through to the base value, slots
/// with `Some` paint the user-chosen color.
pub(crate) fn apply_customized_overrides(
    base: PaletteColors,
    overrides: &[Option<u32>; SLOT_COUNT],
) -> PaletteColors {
    let pick = |slot: usize, default: u32| -> u32 { overrides[slot].unwrap_or(default) };
    PaletteColors {
        top: pick(SLOT_TOP, base.top),
        menu_hot_track: pick(SLOT_MENU_HOT_TRACK, base.menu_hot_track),
        active: pick(SLOT_ACTIVE, base.active),
        main: pick(SLOT_MAIN, base.main),
        error: pick(SLOT_ERROR, base.error),
        text: pick(SLOT_TEXT, base.text),
        darker_text: pick(SLOT_DARKER_TEXT, base.darker_text),
        disabled_text: pick(SLOT_DISABLED_TEXT, base.disabled_text),
        link: pick(SLOT_LINK, base.link),
        edge: pick(SLOT_EDGE, base.edge),
        edge_highlight: pick(SLOT_EDGE_HIGHLIGHT, base.edge_highlight),
        edge_disabled: pick(SLOT_EDGE_DISABLED, base.edge_disabled),
    }
}

impl Theme {
    const DARK: Self = Self {
        bg: 0x1e1f22,
        chrome: 0x2b2d31,
        chrome_text: 0xb5bac1,
        text: 0xdbdee1,
        border: 0x1a1b1e,
        gutter_text: 0x6b6f76,
        gutter_bg: 0x000000,
        selection: 0x4e7ddb55,
        match_highlight: 0xd9aa3340,
        match_highlight_current: 0xf0b0309a,
        // Reuses the same magenta base as match_highlight_current (0xb0309a)
        // at lower alpha (0x46) so marks read as a softer, persistent
        // companion to the live find highlight.
        mark_highlight: 0xb0309a46,
        // Reuses `tab_color_2` (0x7bc36b — green) at 0x3a alpha. Distinct hue
        // from find (yellow) and mark (magenta) so the three bands stack.
        smart_highlight: 0x7bc36b3a,
        toggle_active: 0x4e7ddb,
        error: 0xe06060,
        group_box_border: 0x3a3c41,
        checkbox_fill: 0x4e7ddb,
        button_bg: 0x2b2d31,
        button_hover_bg: 0x3a3c41,
        // Reuses `border` (0x1a1b1e) — that's the darkest existing inset
        // tone in the dark palette. Reads as the button being pushed below
        // the dialog surface.
        button_pressed_bg: 0x1a1b1e,
        disabled_text: 0x6b6f76,
        scrollbar: 0x55585f80,
        scrollbar_hover: 0x8a8e96cc,
        whitespace_glyph: 0x404040,
        // NPP DarkModeDefault `Current line background colour` = 0x222233.
        // Slightly bluer-tinted lift over the editor `bg` (0x1e1f22) so the
        // caret line reads as a quiet band without fighting syntax colours.
        current_line_bg: 0x282a35,
        // Reuses `toggle_active` (0x4e7ddb) — accent blue, already in the
        // dark palette for "this option is on" states.
        bracket_match: 0x4e7ddb,
        toolbar_bg: 0x2b2d31,
        toolbar_icon: 0xb5bac1,
        toolbar_icon_disabled: 0x4f5358,
        toolbar_hover_bg: 0x3a3c41,
        toolbar_separator: 0x1a1b1e,
        tab_color_1: 0xf7c844,
        tab_color_2: 0x7bc36b,
        tab_color_3: 0x5b9bd5,
        tab_color_4: 0xe8943c,
        tab_color_5: 0xd96aac,
        // NPP `SLOT_LINK` default for Black tone — bright yellow on dark.
        link: 0xFFFF00,
        editor_font: None,
        syntax: SyntaxTheme {
            keyword: 0xcf8e6d,
            string: 0xa5d6a7,
            comment: 0x7f848e,
            number: 0xd19a66,
            type_: 0xe5c07b,
            function: 0x61afef,
            constant: 0xd19a66,
            property: 0xe06c75,
            attribute: 0xd19a66,
        },
    };

    const LIGHT: Self = Self {
        bg: 0xfdfdfd,
        chrome: 0xf2f3f5,
        chrome_text: 0x4f545c,
        text: 0x1e1f22,
        border: 0xdadde1,
        gutter_text: 0x9ba0a8,
        gutter_bg: 0xfdfdfd,
        selection: 0x3b82f655,
        match_highlight: 0xffe79a55,
        match_highlight_current: 0xff8f1faa,
        // Reuses the same magenta base as match_highlight_current (0x8f1faa)
        // at 0x55 alpha — matches the alpha used by `selection` in this
        // palette so marks layer well against the lighter background.
        mark_highlight: 0x8f1faa55,
        // Reuses `tab_color_2` (0x7bc36b — green) at 0x55 alpha to match the
        // alpha pattern used by `mark_highlight` and `selection` in this
        // lighter palette.
        smart_highlight: 0x7bc36b55,
        toggle_active: 0x3b82f6,
        error: 0xc02424,
        group_box_border: 0xc8ccd2,
        checkbox_fill: 0x3b82f6,
        button_bg: 0xf2f3f5,
        button_hover_bg: 0xe5e7eb,
        // Reuses `group_box_border` (0xc8ccd2) — same logic as dark mode:
        // the existing inset tone reads as a pressed-in button without
        // adding a fresh palette slot.
        button_pressed_bg: 0xc8ccd2,
        disabled_text: 0x9ba0a8,
        scrollbar: 0xc0c4ca99,
        scrollbar_hover: 0x808488cc,
        whitespace_glyph: 0xc0c0c0,
        // NPP default `Current line background colour` = 0xE8E8FF — pale
        // lavender. Reads as a quiet band against the near-white editor bg.
        current_line_bg: 0xe8e8ff,
        // Reuses `toggle_active` (0x3b82f6) — accent blue, already in the
        // light palette for "this option is on" states.
        bracket_match: 0x3b82f6,
        toolbar_bg: 0xf2f3f5,
        toolbar_icon: 0x4f545c,
        toolbar_icon_disabled: 0xb8bcc3,
        toolbar_hover_bg: 0xe1e3e7,
        toolbar_separator: 0xc8ccd2,
        tab_color_1: 0xf7c844,
        tab_color_2: 0x7bc36b,
        tab_color_3: 0x5b9bd5,
        tab_color_4: 0xe8943c,
        tab_color_5: 0xd96aac,
        // Light-mode link tint — standard browser blue so URLs stay legible
        // against the lighter editor background. NPP doesn't expose a Tones
        // palette in light mode; this constant is the only source.
        link: 0x1a73e8,
        editor_font: None,
        syntax: SyntaxTheme {
            keyword: 0x0033b3,
            string: 0x067d17,
            comment: 0x8c8c8c,
            number: 0x1750eb,
            type_: 0x0a004e,
            function: 0x00627a,
            constant: 0x1750eb,
            property: 0x871094,
            attribute: 0x0033b3,
        },
    };

    /// Resolve a `ThemeMode` against the cached `WindowAppearance` and the
    /// user's tone preferences. Light mode is unaffected by tone — the
    /// palette is only layered on top of Dark, and only when the user
    /// explicitly chose `FollowSystem`. Plain `Dark` always renders the
    /// vanilla Dark theme regardless of persisted palette/slot state, so
    /// the user can experiment with tones via Follow Windows without it
    /// silently affecting plain Dark.
    pub(crate) fn resolve(
        mode: ThemeMode,
        appearance: gpui::WindowAppearance,
        palette: TonePalette,
        customized_slots: &[Option<u32>; SLOT_COUNT],
    ) -> Self {
        match mode.resolved(appearance) {
            ResolvedMode::Light => Self::LIGHT,
            ResolvedMode::Dark => match mode {
                ThemeMode::FollowSystem => Self::dark_with_palette(palette, customized_slots),
                _ => Self::DARK,
            },
        }
    }

    /// Layer a tone palette + customized overrides on top of `Theme::DARK`.
    /// Public so the Preferences dialog can compute its own theme from the
    /// drafted palette without going through Workspace state.
    pub(crate) fn dark_with_palette(
        palette: TonePalette,
        customized_slots: &[Option<u32>; SLOT_COUNT],
    ) -> Self {
        let resolved = palette_for_tone(palette);
        let resolved = if matches!(palette, TonePalette::Customized) {
            apply_customized_overrides(resolved, customized_slots)
        } else {
            resolved
        };
        let mut t = Self::DARK;
        // Background family — `main` is the editor body, `top` covers the
        // chrome surfaces (toolbar / tab strip / status bar / dialog body).
        t.bg = resolved.main;
        t.chrome = resolved.top;
        t.toolbar_bg = resolved.top;
        t.button_bg = resolved.top;
        // Hot-track family — hover/active surfaces.
        t.button_hover_bg = resolved.menu_hot_track;
        t.toolbar_hover_bg = resolved.menu_hot_track;
        // Pressed = `active` (NPP's hottest background tier).
        t.button_pressed_bg = resolved.active;
        // Border / edge family.
        t.border = resolved.edge;
        t.toolbar_separator = resolved.edge_disabled;
        t.group_box_border = resolved.edge_highlight;
        // Text family.
        t.text = resolved.text;
        t.chrome_text = resolved.darker_text;
        t.toolbar_icon = resolved.darker_text;
        t.gutter_text = resolved.darker_text;
        t.disabled_text = resolved.disabled_text;
        t.toolbar_icon_disabled = resolved.disabled_text;
        // Error tint.
        t.error = resolved.error;
        // Clickable-URL tint — drives editor-text underline + any other
        // "link" surface. Customized slot overrides reach here naturally
        // because `apply_customized_overrides` already filled `resolved.link`.
        t.link = resolved.link;
        t
    }

    /// Backwards-compatible accessor. Equivalent to `resolve` with
    /// `WindowAppearance::Dark` + Black tone + no overrides — preserves the
    /// pre-tone behaviour for call sites that haven't been threaded through
    /// `Workspace::theme()` yet (currently only `apply_session` reads here
    /// indirectly via `theme_mode` round-tripping).
    pub(crate) fn for_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark | ThemeMode::FollowSystem => Self::DARK,
            ThemeMode::Light => Self::LIGHT,
        }
    }

    /// Layer the active Style Configurator theme XML on top of the resolved
    /// base theme. Maps the Notepad++ schema's GlobalStyles + cpp lexer
    /// entries onto Notepad Studio's `Theme` slots:
    ///
    /// * `Default Style`           → `bg`, `gutter_bg`, `text`, `editor_font`
    /// * `Selected text colour`    → `selection` (RGB only — alpha kept
    ///                                from base so the existing translucency
    ///                                survives a theme swap)
    /// * `Indent guideline style`  → `whitespace_glyph`
    /// * `Brace highlight style`   → `bracket_match`
    /// * `cpp ▸ INSTRUCTION WORD`  → `syntax.keyword`
    /// * `cpp ▸ TYPE WORD`         → `syntax.type_`
    /// * `cpp ▸ NUMBER`            → `syntax.number`
    /// * `cpp ▸ STRING`            → `syntax.string`, `syntax.constant`
    /// * `cpp ▸ COMMENT`           → `syntax.comment`
    /// * `cpp ▸ PREPROCESSOR`      → `syntax.attribute`
    ///
    /// `cpp` is the only lexer mined for syntax colours. The Scintilla
    /// style-id space is per-lexer, but `cpp` is the most general C-family
    /// mapping NPP exposes — every theme bundled with Notepad Studio has a full
    /// `cpp` block, and tree-sitter's HighlightKind set lines up cleanly
    /// with its style names. Per-language overrides (a Rust file picking up
    /// the Rust lexer's COMMENT colour) are out of scope for v1.
    pub(crate) fn with_xml_overrides(
        mut self,
        doc: &crate::style_model::ThemeDocument,
    ) -> Self {
        let global = |name: &str| -> Option<&crate::style_model::Style> {
            doc.global_styles.iter().find(|s| s.name == name)
        };
        if let Some(default) = global("Default Style") {
            if let Some(bg) = default.bg_color {
                self.bg = bg;
                self.gutter_bg = bg;
            }
            if let Some(fg) = default.fg_color {
                self.text = fg;
            }
            self.editor_font = default.font_name.as_deref().and_then(crate::fonts::installed);
        }
        if let Some(sel) = global("Selected text colour") {
            if let Some(bg) = sel.bg_color {
                // Preserve the existing alpha byte so highlight translucency
                // doesn't change when a theme without an explicit alpha
                // gets layered on.
                let alpha = self.selection & 0xFF000000;
                self.selection = alpha | (bg & 0x00FFFFFF);
            }
        }
        if let Some(g) = global("Indent guideline style") {
            if let Some(fg) = g.fg_color {
                self.whitespace_glyph = fg;
            }
        }
        if let Some(b) = global("Brace highlight style") {
            if let Some(fg) = b.fg_color {
                self.bracket_match = fg;
            }
        }
        if let Some(cl) = global("Current line background colour") {
            if let Some(bg) = cl.bg_color {
                self.current_line_bg = bg;
            }
        }

        if let Some(cpp) = doc
            .lexers
            .lexers
            .iter()
            .find(|l| l.lexer_name == "cpp")
        {
            let pick = |name: &str| -> Option<u32> {
                cpp.styles
                    .iter()
                    .find(|s| s.name == name)
                    .and_then(|s| s.fg_color)
            };
            if let Some(c) = pick("INSTRUCTION WORD") {
                self.syntax.keyword = c;
            }
            if let Some(c) = pick("TYPE WORD") {
                self.syntax.type_ = c;
            }
            if let Some(c) = pick("NUMBER") {
                self.syntax.number = c;
            }
            if let Some(c) = pick("STRING") {
                self.syntax.string = c;
                self.syntax.constant = c;
            }
            if let Some(c) = pick("COMMENT").or_else(|| pick("COMMENT LINE")) {
                self.syntax.comment = c;
            }
            if let Some(c) = pick("PREPROCESSOR") {
                self.syntax.attribute = c;
            }
        }

        self
    }
}
