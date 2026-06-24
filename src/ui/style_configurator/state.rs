// SPDX-License-Identifier: Apache-2.0
//
// `DialogState` — the in-memory model the Style Configurator window edits.
// Owns the discovered theme list, the currently-loaded `ThemeDocument`, and
// the listbox selection cursors. Phase 3 wires loading + cascade behaviour;
// Phase 4 adds dirty tracking on per-style edits; Phase 5 adds Cancel-restore
// from the snapshot taken on open.

use std::path::PathBuf;

use crate::style_model::global_override::GlobalOverride;
use crate::style_model::lexer::LexerStyler;
use crate::style_model::style::Style;
use crate::style_model::theme_switcher::{self, ThemeEntry, ThemeSource};
use crate::style_model::xml_load::{self, ThemeDocument};

/// Names the seven `GlobalOverride` flags so the right-column checkbox
/// renderers can emit a single `toggle_override(field)` call without
/// reaching into the struct directly. Mirrors NPP's seven `WORD`-flag
/// fields on `GlobalOverride`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverrideField {
    Fg,
    Bg,
    Font,
    FontSize,
    Bold,
    Italic,
    Underline,
}

/// Owned by `StyleConfiguratorDialog`. Plain struct (no Entity wrapping) —
/// mutated through the parent entity's `update` like every other field on
/// the dialog. Visibility is set so sibling modules
/// (`header`, `left_column`, `footer`, `layout`) can both name and read
/// the type without leaking it out of the configurator.
pub(in crate::ui::style_configurator) struct DialogState {
    /// Theme rows in dropdown order (synthetic "Default" first, then sorted).
    pub themes: Vec<ThemeEntry>,
    /// Index into `themes` of the active selection. Always `< themes.len()`
    /// when `themes` is non-empty; `0` is a safe fallback.
    pub active_theme_idx: usize,
    /// Parsed XML for the active theme. On a parse failure the dialog falls
    /// back to a default-constructed document so the UI still renders.
    pub document: ThemeDocument,
    /// `0` selects the synthetic "Global Styles" row; `1..=lexers.len()`
    /// selects `document.lexers.lexers[idx - 1]`.
    pub selected_lang_idx: usize,
    /// Index into `current_styles()`; clamped on every refresh so a smaller
    /// list never produces an out-of-bounds read.
    pub selected_style_idx: usize,
    /// Marks the in-memory document as edited-but-unsaved. Phase 3 leaves
    /// this `false`; Phase 4 wires it from the right-column controls.
    pub dirty: bool,
    /// Cancel-restore baseline. Cloned on `switch_theme` and on `new` so the
    /// future Cancel button can revert in-memory edits to the snapshot.
    pub snapshot: Option<ThemeDocument>,
    /// Theme combo index pending confirmation when the user tries to switch
    /// themes with unsaved edits. `Some(idx)` means the dirty prompt is up
    /// and the user has not yet picked Save / Discard / Cancel-switch.
    pub switch_pending: Option<usize>,
    /// `true` when the inline dirty-prompt overlay should render. Mirrors
    /// `switch_pending.is_some()` but kept separate so future prompts (e.g.
    /// "delete user theme?") can reuse the same field without piggybacking.
    pub prompt_visible: bool,
    /// Last save error message, surfaced in the footer when `Some`. Cleared
    /// on the next successful save or on dialog close.
    pub last_save_error: Option<String>,
    /// "Global Styles ▸ Global override" checkbox flags. NOT persisted in
    /// the theme XML — these are user-level prefs stored in `Settings`.
    /// Loaded from `Settings::snapshot()` on dialog open; written back on
    /// Save & Close via `replace_and_save`.
    pub global_override: GlobalOverride,
    /// Snapshot of `global_override` taken on dialog open / after each save.
    /// Cancel restores from this. Mirrors `snapshot: Option<ThemeDocument>`.
    pub global_override_snapshot: Option<GlobalOverride>,
}

impl DialogState {
    /// Discover available themes and load the first one. Synchronous I/O —
    /// the dialog waits a beat on first open while a few KB of XML parses,
    /// matching NPP's behaviour.
    pub fn new() -> Self {
        let themes = theme_switcher::discover();
        let settings = crate::settings::snapshot();
        let stored_path = settings.theme_path.clone();
        let global_override = settings.global_override;
        let active_theme_idx = stored_path
            .as_deref()
            .and_then(|p| themes.iter().position(|t| theme_path_matches(t, p)))
            .unwrap_or(0);
        let document = themes
            .get(active_theme_idx)
            .map(load_document)
            .unwrap_or_default();
        let snapshot = Some(document.clone());
        Self {
            themes,
            active_theme_idx,
            document,
            selected_lang_idx: 0,
            selected_style_idx: 0,
            dirty: false,
            snapshot,
            switch_pending: None,
            prompt_visible: false,
            last_save_error: None,
            global_override,
            global_override_snapshot: Some(global_override),
        }
    }

    /// Re-parse the XML for `idx` and reset the cascade selection. Out-of-
    /// range indices are ignored.
    ///
    /// Switching themes counts as a pending change — the user has to Save &
    /// Close to persist the new selection to `Settings.theme_path`. Without
    /// the dirty flip the Save button would stay greyed and the user would
    /// have no obvious way to commit their pick.
    pub fn switch_theme(&mut self, idx: usize) {
        if idx >= self.themes.len() || idx == self.active_theme_idx {
            return;
        }
        self.active_theme_idx = idx;
        self.document = load_document(&self.themes[idx]);
        self.snapshot = Some(self.document.clone());
        self.selected_lang_idx = 0;
        self.selected_style_idx = 0;
        self.dirty = true;
        self.last_save_error = None;
    }

    /// Caller-facing entry point for the theme combo: when dirty, show the
    /// confirm prompt; otherwise switch immediately. Phase 5 handlers route
    /// theme-combo clicks through here instead of `switch_theme` directly.
    pub fn request_theme_switch(&mut self, idx: usize) {
        if idx >= self.themes.len() || idx == self.active_theme_idx {
            return;
        }
        if self.dirty {
            self.switch_pending = Some(idx);
            self.prompt_visible = true;
        } else {
            self.switch_theme(idx);
        }
    }

    /// Discard a pending switch (user clicked "Cancel switch" in the prompt
    /// or closed the prompt). Leaves the dialog on the current theme.
    pub fn cancel_pending_switch(&mut self) {
        self.switch_pending = None;
        self.prompt_visible = false;
    }

    /// Take the pending-switch index without applying it. Used by the
    /// Save / Discard prompt buttons after they've completed their work.
    pub fn take_pending_switch(&mut self) -> Option<usize> {
        self.prompt_visible = false;
        self.switch_pending.take()
    }

    /// Re-take the snapshot from the current document. Called after a
    /// successful save so future Cancels revert to the post-save state.
    pub fn take_snapshot(&mut self) {
        self.snapshot = Some(self.document.clone());
        self.global_override_snapshot = Some(self.global_override);
        self.dirty = false;
    }

    /// Restore the in-memory document from the cancel-restore snapshot.
    /// Called by the Cancel button before closing the window. No-op when
    /// no snapshot exists (shouldn't happen — `new` always sets one).
    pub fn restore_from_snapshot(&mut self) {
        if let Some(snap) = self.snapshot.clone() {
            self.document = snap;
        }
        if let Some(ovr) = self.global_override_snapshot {
            self.global_override = ovr;
        }
        self.selected_lang_idx = 0;
        self.selected_style_idx = 0;
        self.dirty = false;
        self.last_save_error = None;
    }

    /// Toggle one `GlobalOverride` checkbox + mark dirty. Routed through
    /// here (instead of the right column writing to fields directly) so the
    /// dirty bit is never forgotten and the OverrideField enum is the
    /// single source of truth for the seven flags.
    pub fn toggle_override(&mut self, field: OverrideField) {
        let g = &mut self.global_override;
        match field {
            OverrideField::Fg => g.enable_fg = !g.enable_fg,
            OverrideField::Bg => g.enable_bg = !g.enable_bg,
            OverrideField::Font => g.enable_font = !g.enable_font,
            OverrideField::FontSize => g.enable_font_size = !g.enable_font_size,
            OverrideField::Bold => g.enable_bold = !g.enable_bold,
            OverrideField::Italic => g.enable_italic = !g.enable_italic,
            OverrideField::Underline => g.enable_underline = !g.enable_underline,
        }
        self.notify_modified();
    }

    /// Read a `GlobalOverride` flag by enum tag — convenience for the
    /// right-column renderer so it can map the seven checkboxes uniformly.
    pub fn override_flag(&self, field: OverrideField) -> bool {
        match field {
            OverrideField::Fg => self.global_override.enable_fg,
            OverrideField::Bg => self.global_override.enable_bg,
            OverrideField::Font => self.global_override.enable_font,
            OverrideField::FontSize => self.global_override.enable_font_size,
            OverrideField::Bold => self.global_override.enable_bold,
            OverrideField::Italic => self.global_override.enable_italic,
            OverrideField::Underline => self.global_override.enable_underline,
        }
    }

    /// Refresh the theme list after a save (so a freshly-written user
    /// override appears in the combo on next open). Tries to keep the
    /// active selection on the same theme by display name; falls back to
    /// index 0 if the prior selection vanishes.
    pub fn refresh_themes(&mut self) {
        let prior_name = self
            .themes
            .get(self.active_theme_idx)
            .map(|t| t.display_name.clone());
        self.themes = theme_switcher::discover();
        if let Some(name) = prior_name {
            if let Some(i) = self.themes.iter().position(|t| t.display_name == name) {
                self.active_theme_idx = i;
                return;
            }
        }
        self.active_theme_idx = 0;
    }

    /// Set the language listbox selection and reset the style cursor.
    /// No-op if `idx` already matches `selected_lang_idx` so re-clicking the
    /// active row doesn't disturb the style cursor.
    pub fn switch_lang(&mut self, idx: usize) {
        let max = self.lang_count();
        if idx >= max || idx == self.selected_lang_idx {
            return;
        }
        self.selected_lang_idx = idx;
        self.selected_style_idx = 0;
    }

    /// Set the style listbox selection. Clamped — out-of-range is ignored.
    pub fn switch_style(&mut self, idx: usize) {
        if idx >= self.current_styles().len() {
            return;
        }
        self.selected_style_idx = idx;
    }

    /// Convenience: `1` for "Global Styles" + one row per lexer.
    pub fn lang_count(&self) -> usize {
        1 + self.document.lexers.len()
    }

    /// `["Global Styles", "<lexer 1 desc>", …]`. Stable index space — the
    /// caller may use the index returned here as a `selected_lang_idx`.
    pub fn lang_names(&self) -> Vec<String> {
        let mut out = Vec::with_capacity(self.lang_count());
        out.push("Global Styles".to_string());
        for lex in &self.document.lexers.lexers {
            out.push(lex.lexer_desc.clone());
        }
        out
    }

    /// Names of `current_styles()`. May be empty when the active lexer has
    /// zero `<WordsStyle>` rows in the loaded XML.
    pub fn style_names(&self) -> Vec<String> {
        self.current_styles().iter().map(|s| s.name.clone()).collect()
    }

    /// The slice the Style listbox should render. Either the global widget
    /// styles or the active lexer's per-style entries.
    pub fn current_styles(&self) -> &[Style] {
        if self.selected_lang_idx == 0 {
            &self.document.global_styles
        } else {
            self.document
                .lexers
                .lexers
                .get(self.selected_lang_idx - 1)
                .map(|l| l.styles.as_slice())
                .unwrap_or(&[])
        }
    }

    /// Selected `Style`, or `None` if the listbox is empty. Phase 4's right
    /// column reads this for the swatches / font controls.
    pub fn current_style(&self) -> Option<&Style> {
        self.current_styles().get(self.selected_style_idx)
    }

    /// `&mut` view of the selected `Style`, mirroring `current_style()`.
    /// Phase 4's right-column edits route writes through here so the
    /// in-memory document mutates in place. Returns `None` when the listbox
    /// is empty — callers must handle that (no panic on an unselectable
    /// state).
    pub fn current_style_mut(&mut self) -> Option<&mut Style> {
        let idx = self.selected_style_idx;
        if self.selected_lang_idx == 0 {
            self.document.global_styles.get_mut(idx)
        } else {
            self.document
                .lexers
                .lexers
                .get_mut(self.selected_lang_idx - 1)
                .and_then(|l| l.styles.get_mut(idx))
        }
    }

    /// Mark the in-memory document as edited. Phase 4 calls this after any
    /// right-column control mutates `current_style_mut()`. Phase 5 reads
    /// `dirty` to enable Save & Close and to gate Cancel-restore.
    pub fn notify_modified(&mut self) {
        self.dirty = true;
    }

    /// `None` when the language combo is on "Global Styles", otherwise the
    /// active lexer entry. Used by the right column for the description
    /// label and by the User-ext / Default-ext rows.
    pub fn current_lexer(&self) -> Option<&LexerStyler> {
        if self.selected_lang_idx == 0 {
            return None;
        }
        self.document.lexers.lexers.get(self.selected_lang_idx - 1)
    }

    /// `&mut` view of `current_lexer()`. The user-ext input edits the
    /// active lexer's extension list through this — Save & Close serialises
    /// it back into the theme XML's `userExt` attribute.
    pub fn current_lexer_mut(&mut self) -> Option<&mut LexerStyler> {
        if self.selected_lang_idx == 0 {
            return None;
        }
        self.document.lexers.lexers.get_mut(self.selected_lang_idx - 1)
    }

    /// `None` for Global Styles. For real lexers, returns the lexer's
    /// declared default extension list (the `userExt` / `ext` attribute the
    /// theme XML carries — Phase 1's parser populates `lexer_user_ext`).
    pub fn current_default_ext(&self) -> Option<&str> {
        self.current_lexer()
            .map(|l| l.lexer_user_ext.as_str())
    }

    /// Overwrite the active lexer's user-ext string and mark the document
    /// dirty. No-op on Global Styles (the user-ext row only renders for
    /// real lexers, but guard anyway). Called from the dialog's
    /// `on_user_ext_buffer_changed` after every keystroke / paste / drag.
    pub fn set_current_user_ext(&mut self, value: String) {
        let Some(lexer) = self.current_lexer_mut() else {
            return;
        };
        if lexer.lexer_user_ext == value {
            return;
        }
        lexer.lexer_user_ext = value;
        self.dirty = true;
    }
}

/// Resolve the on-disk path Settings stored for a theme entry. Bundled
/// themes match against their save-redirect path (the override location
/// where edits land); user-file entries match their direct path. Used at
/// dialog open to restore the previously-selected theme across launches.
fn theme_path_matches(entry: &ThemeEntry, target: &std::path::Path) -> bool {
    match &entry.source {
        ThemeSource::UserFile(p) => p == target,
        ThemeSource::Bundled(_) => entry
            .save_redirect
            .as_deref()
            .map(|p| p == target)
            .unwrap_or(false),
    }
}

/// Resolve the path Save & Close should write to for the active theme.
/// `None` means the theme cannot be saved (bundled with no redirect — not
/// expected in practice because `theme_switcher` always sets one).
pub fn save_target(state: &DialogState) -> Option<PathBuf> {
    state
        .themes
        .get(state.active_theme_idx)
        .and_then(theme_switcher::save_path_for)
}

/// Load the XML for a `ThemeEntry`. On any failure return an empty document
/// rather than panicking — the dialog still needs to render so the user can
/// pick a different theme.
fn load_document(entry: &ThemeEntry) -> ThemeDocument {
    let xml = match &entry.source {
        ThemeSource::UserFile(path) => match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(err) => {
                eprintln!(
                    "style_configurator: failed to read theme {:?}: {err}",
                    path
                );
                return ThemeDocument::default();
            }
        },
        ThemeSource::Bundled(s) => (*s).to_string(),
    };

    match xml_load::parse(&xml) {
        Ok(doc) => doc,
        Err(err) => {
            eprintln!(
                "style_configurator: failed to parse theme {:?}: {err}",
                entry.display_name
            );
            ThemeDocument::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style_model::lexer::{LexerStyler, LexerStylerArray};
    use crate::style_model::style::Style;

    fn make_state(doc: ThemeDocument) -> DialogState {
        DialogState {
            themes: Vec::new(),
            active_theme_idx: 0,
            document: doc,
            selected_lang_idx: 0,
            selected_style_idx: 0,
            dirty: false,
            snapshot: None,
            switch_pending: None,
            prompt_visible: false,
            last_save_error: None,
            global_override: GlobalOverride::default(),
            global_override_snapshot: None,
        }
    }

    #[test]
    fn restore_from_snapshot_reverts_document() {
        let mut doc = ThemeDocument::default();
        doc.global_styles.push(Style::named(32, "Default Style"));
        let mut state = make_state(doc);
        state.snapshot = Some(state.document.clone());
        state.document.global_styles[0].name = "edited".to_string();
        state.dirty = true;
        state.restore_from_snapshot();
        assert_eq!(state.document.global_styles[0].name, "Default Style");
        assert!(!state.dirty);
    }

    #[test]
    fn request_theme_switch_when_clean_switches_immediately() {
        let mut state = make_state(ThemeDocument::default());
        // No themes pushed; idx 0 is current. request switching to 99
        // should be ignored (out of range), not pend.
        state.request_theme_switch(99);
        assert!(state.switch_pending.is_none());
        assert!(!state.prompt_visible);
    }

    #[test]
    fn request_theme_switch_when_dirty_pends() {
        let doc = ThemeDocument::default();
        let mut state = make_state(doc);
        // Synthesise two phantom themes so request can pend a switch.
        use crate::style_model::theme_switcher::{ThemeEntry, ThemeSource};
        state.themes = vec![
            ThemeEntry {
                display_name: "A".into(),
                source: ThemeSource::Bundled("<NotepadPlus></NotepadPlus>"),
                save_redirect: None,
            },
            ThemeEntry {
                display_name: "B".into(),
                source: ThemeSource::Bundled("<NotepadPlus></NotepadPlus>"),
                save_redirect: None,
            },
        ];
        state.dirty = true;
        state.request_theme_switch(1);
        assert_eq!(state.switch_pending, Some(1));
        assert!(state.prompt_visible);
        // Cancel the pending switch.
        state.cancel_pending_switch();
        assert!(state.switch_pending.is_none());
        assert!(!state.prompt_visible);
    }

    #[test]
    fn lang_zero_means_global_styles() {
        let mut doc = ThemeDocument::default();
        doc.global_styles.push(Style::named(32, "Default Style"));
        let state = make_state(doc);
        assert_eq!(state.lang_count(), 1);
        assert_eq!(state.lang_names(), vec!["Global Styles".to_string()]);
        assert_eq!(state.current_styles().len(), 1);
        assert_eq!(state.current_style().map(|s| s.name.as_str()), Some("Default Style"));
        assert!(state.current_lexer().is_none());
    }

    #[test]
    fn lang_one_selects_first_lexer() {
        let mut lex = LexerStyler::new("cpp", "C++");
        lex.styles.push(Style::named(11, "DEFAULT"));
        lex.styles.push(Style::named(1, "COMMENT"));
        let mut lexers = LexerStylerArray::new();
        lexers.push(lex);
        let doc = ThemeDocument {
            lexers,
            global_styles: vec![Style::named(32, "Default Style")],
        };
        let mut state = make_state(doc);
        state.switch_lang(1);
        assert_eq!(state.selected_lang_idx, 1);
        assert_eq!(state.selected_style_idx, 0);
        assert_eq!(state.style_names(), vec!["DEFAULT".to_string(), "COMMENT".to_string()]);
        assert!(state.current_lexer().is_some());
        assert_eq!(state.current_lexer().unwrap().lexer_name, "cpp");
    }

    #[test]
    fn switch_lang_resets_style_cursor() {
        let mut lex = LexerStyler::new("cpp", "C++");
        lex.styles.push(Style::named(11, "DEFAULT"));
        lex.styles.push(Style::named(1, "COMMENT"));
        let mut lexers = LexerStylerArray::new();
        lexers.push(lex);
        let doc = ThemeDocument {
            lexers,
            global_styles: Vec::new(),
        };
        let mut state = make_state(doc);
        state.switch_lang(1);
        state.switch_style(1);
        assert_eq!(state.selected_style_idx, 1);
        state.switch_lang(1); // no-op, but must not move the style cursor
        assert_eq!(state.selected_style_idx, 1);
    }

    #[test]
    fn out_of_range_indices_are_ignored() {
        let state_doc = ThemeDocument::default();
        let mut state = make_state(state_doc);
        state.switch_lang(99);
        assert_eq!(state.selected_lang_idx, 0);
        state.switch_style(99);
        assert_eq!(state.selected_style_idx, 0);
    }

    #[test]
    fn toggle_override_flips_flag_and_dirties() {
        let mut state = make_state(ThemeDocument::default());
        assert!(!state.override_flag(OverrideField::Bold));
        assert!(!state.dirty);
        state.toggle_override(OverrideField::Bold);
        assert!(state.override_flag(OverrideField::Bold));
        assert!(state.dirty);
    }

    #[test]
    fn restore_reverts_global_override() {
        let mut state = make_state(ThemeDocument::default());
        state.global_override_snapshot = Some(state.global_override);
        state.toggle_override(OverrideField::Fg);
        state.toggle_override(OverrideField::Italic);
        assert!(state.override_flag(OverrideField::Fg));
        assert!(state.override_flag(OverrideField::Italic));
        state.restore_from_snapshot();
        assert!(!state.override_flag(OverrideField::Fg));
        assert!(!state.override_flag(OverrideField::Italic));
    }

    #[test]
    fn take_snapshot_captures_override() {
        let mut state = make_state(ThemeDocument::default());
        state.toggle_override(OverrideField::Underline);
        state.take_snapshot();
        // Subsequent toggle + restore must revert to underline=true.
        state.toggle_override(OverrideField::Underline);
        assert!(!state.override_flag(OverrideField::Underline));
        state.restore_from_snapshot();
        assert!(state.override_flag(OverrideField::Underline));
    }
}
