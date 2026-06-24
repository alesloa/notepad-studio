// Auto-Completion runtime: word scanner + popup state + action handlers.
//
// Scope:
//   * Word Completion — buffer-scans for unique identifiers sharing the
//     prefix at the caret, opens a popup with the candidates.
//   * Function Completion — pulls from a per-language static keyword +
//     common-builtin list (`syntax::keywords`). NPP loads these from
//     per-language `apis/<lang>.xml` registries with full signatures;
//     Notepad Studio's v1 surfaces just the identifier — the full
//     signature variant lands when the XML loader does.
//   * Function and Word Completion — merges both candidate pools.
//   * Function Parameters Hint — toasts a status message. Real call-tip
//     popup needs signature data that the static lists don't carry.
//
// Settings wiring map (so future readers don't have to grep):
//   * `autocompletion_insert_with_tab` — gates Tab acceptance, lives in
//     `edits.rs::insert_tab` (the keystroke handler is in edits, the
//     popup state is here).
//   * `autocompletion_insert_with_enter` — same story in
//     `edits.rs::insert_newline`.
//   * `autocompletion_ignore_numbers` — read in the three `try_open_*`
//     candidate collectors below.
//   * `autocompletion_brief` — read in `try_open_function_popup` /
//     `try_open_combined_popup`. Currently a no-op because every
//     candidate is already a bare identifier (no signature column yet);
//     the read is in place so the brief filter activates automatically
//     when the per-language API XML registry ships signatures.

use std::collections::BTreeSet;

use gpui::{Context, Pixels, Point, Window, point};

use crate::actions::*;
use crate::settings;
use crate::syntax::{LanguageId, keywords};
use crate::tab::InputFocus;
use crate::workspace::Workspace;

/// Snapshot of the popup. Held on `Workspace` while the menu is on screen;
/// `None` when no popup is up. `anchor` is window-space pixels of the popup's
/// top-left corner (typically one line below the caret).
#[derive(Clone)]
pub(crate) struct AutocompleteState {
    pub(crate) candidates: Vec<String>,
    pub(crate) selected: usize,
    /// Byte offset where the matched prefix starts in the active buffer's
    /// rope. Replacement on accept covers `[replace_start, caret)`.
    pub(crate) replace_start: usize,
    /// Top-left of the popup body in window-space pixels.
    pub(crate) anchor: Point<Pixels>,
}

/// Maximum candidates surfaced. NPP caps the popup at this kind of size
/// implicitly via scroll; here we hard-cap to keep the scan + render cheap on
/// large buffers. 50 is well above what a user would scroll through anyway.
const MAX_CANDIDATES: usize = 50;

/// Minimum identifier length for a candidate to be considered. NPP also
/// filters single-char identifiers from its word scan — a 1-char popup would
/// be more noise than help.
const MIN_CANDIDATE_LEN: usize = 2;

/// True for chars that belong inside an identifier. Mirrors NPP's
/// `isCharWordChar` (`Notepad_plus::isCharWordChar`) which uses ASCII
/// alphanumerics + `_`, with Scintilla's allow-list opt-in for additional
/// chars deferred (NPP exposes it as a per-syntax setting).
pub(crate) fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Walk backwards from `caret` through the rope and return the byte offset
/// where the prefix starts. Empty prefix returns `caret` unchanged.
pub(crate) fn find_prefix_start(rope: &ropey::Rope, caret: usize) -> usize {
    if caret == 0 {
        return 0;
    }
    let mut idx = caret;
    let char_caret = rope.byte_to_char(caret);
    let mut char_idx = char_caret;
    while char_idx > 0 {
        let c = rope.char(char_idx - 1);
        if !is_word_char(c) {
            break;
        }
        char_idx -= 1;
        idx = rope.char_to_byte(char_idx);
    }
    idx
}

/// Collect every unique identifier in the buffer that starts with `prefix`,
/// excluding the exact prefix itself (the user already typed it — offering
/// it back would just dismiss the popup with no change). When
/// `ignore_numbers` is true, candidates whose first char is a digit are
/// skipped (NPP `Auto-Completion ▸ Insert Selection ▸ Ignore numbers`,
/// default on).
pub(crate) fn collect_word_candidates(
    rope: &ropey::Rope,
    prefix: &str,
    ignore_numbers: bool,
) -> Vec<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let text = rope.to_string();
    let mut current = String::new();
    let lower_prefix = prefix.to_ascii_lowercase();
    let push = |word: &str, seen: &mut BTreeSet<String>| {
        if word.len() < MIN_CANDIDATE_LEN {
            return;
        }
        if word.eq_ignore_ascii_case(prefix) {
            return;
        }
        if !prefix.is_empty()
            && !word.to_ascii_lowercase().starts_with(&lower_prefix)
        {
            return;
        }
        if ignore_numbers && word.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            return;
        }
        seen.insert(word.to_string());
    };

    for ch in text.chars() {
        if is_word_char(ch) {
            current.push(ch);
        } else {
            if !current.is_empty() {
                push(&current, &mut seen);
                current.clear();
            }
        }
        if seen.len() >= MAX_CANDIDATES * 4 {
            // Safety cap on extremely large buffers — once we have a healthy
            // pool we stop scanning. Sort + truncate below trims to MAX.
            break;
        }
    }
    if !current.is_empty() {
        push(&current, &mut seen);
    }

    let mut out: Vec<String> = seen.into_iter().collect();
    if out.len() > MAX_CANDIDATES {
        out.truncate(MAX_CANDIDATES);
    }
    out
}

/// Filter the language's static keyword list by `prefix`. Case-insensitive
/// prefix match mirrors `collect_word_candidates`. Returns at most
/// `MAX_CANDIDATES` entries, BTreeSet-sorted for stable ordering.
pub(crate) fn collect_keyword_candidates(
    prefix: &str,
    lang: LanguageId,
    ignore_numbers: bool,
) -> Vec<String> {
    let kws = keywords::keywords_for(lang);
    if kws.is_empty() {
        return Vec::new();
    }
    let lower_prefix = prefix.to_ascii_lowercase();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for word in kws {
        if word.len() < MIN_CANDIDATE_LEN {
            continue;
        }
        if word.eq_ignore_ascii_case(prefix) {
            continue;
        }
        if !prefix.is_empty()
            && !word.to_ascii_lowercase().starts_with(&lower_prefix)
        {
            continue;
        }
        if ignore_numbers && word.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            continue;
        }
        seen.insert((*word).to_string());
    }
    let mut out: Vec<String> = seen.into_iter().collect();
    if out.len() > MAX_CANDIDATES {
        out.truncate(MAX_CANDIDATES);
    }
    out
}

/// NPP `Auto-Completion ▸ Make auto-completion list brief`. When `brief`
/// is on, strip the signature column from each candidate label so the
/// popup shows only the identifier. The keyword pool today carries no
/// signatures (the per-language API XML registry isn't loaded yet), so
/// this is currently identity — kept as the single chokepoint so the
/// toggle starts taking effect the moment signature-bearing entries land.
fn apply_brief_filter(candidates: Vec<String>, brief: bool) -> Vec<String> {
    if !brief {
        return candidates;
    }
    candidates
        .into_iter()
        .map(|c| {
            // NPP brief format strips at the first `(` (function-style
            // signature) or whitespace following the identifier. If neither
            // appears the candidate is already brief.
            let cut = c.find('(').unwrap_or(c.len());
            let head = &c[..cut];
            let cut = head
                .find(|ch: char| ch.is_whitespace())
                .unwrap_or(head.len());
            head[..cut].to_string()
        })
        .collect()
}

/// Merge two pre-filtered candidate vectors into a deduped, sorted, capped
/// list. Used by `Function and Word Completion` to combine keyword
/// candidates with buffer-word candidates.
fn merge_candidates(a: Vec<String>, b: Vec<String>) -> Vec<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    seen.extend(a);
    seen.extend(b);
    let mut out: Vec<String> = seen.into_iter().collect();
    if out.len() > MAX_CANDIDATES {
        out.truncate(MAX_CANDIDATES);
    }
    out
}

impl Workspace {
    /// Compute the popup's window-space anchor by reading `last_lines` (the
    /// per-paint cache of visible-line geometry). Falls back to the editor
    /// pane's top-left when the caret's line isn't visible — drop-rare but
    /// possible right after a programmatic caret move before the next paint
    /// updates the cache.
    fn autocomplete_anchor(&self, caret: usize) -> Point<Pixels> {
        for line in &self.last_lines {
            let line_end = line.line_byte_start + line.line_byte_len;
            if caret >= line.line_byte_start && caret <= line_end {
                let local = caret - line.line_byte_start;
                let x = line.bounds.left() + line.shaped.x_for_index(local);
                let y = line.bounds.bottom();
                return point(x, y);
            }
        }
        // Fallback: position over the editor pane's origin so the popup is
        // at least visible. Better than rendering off-screen.
        let b = self.viewport.bounds;
        point(b.left(), b.top())
    }

    /// Compute prefix + open a popup for it. Empty candidate list = no
    /// popup, ephemeral status message instead (NPP also no-ops when the
    /// scan finds nothing). Returns true when a popup was opened.
    fn try_open_word_popup(&mut self, cx: &mut Context<Self>) -> bool {
        let caret = self.selection().head;
        let prefix_start;
        let prefix;
        let snap = settings::snapshot();
        let candidates;
        {
            let rope = &self.buffer().rope;
            prefix_start = find_prefix_start(rope, caret);
            prefix = rope.byte_slice(prefix_start..caret).to_string();
            candidates = collect_word_candidates(
                rope,
                &prefix,
                snap.autocompletion_ignore_numbers,
            );
        }
        if candidates.is_empty() {
            self.status_message = Some(if prefix.is_empty() {
                "Auto-completion: no candidates in buffer".to_string()
            } else {
                format!("Auto-completion: no match for \"{prefix}\"")
            });
            cx.notify();
            return false;
        }
        let anchor = self.autocomplete_anchor(prefix_start);
        self.autocomplete = Some(AutocompleteState {
            candidates,
            selected: 0,
            replace_start: prefix_start,
            anchor,
        });
        self.status_message = None;
        cx.notify();
        true
    }

    /// Open a popup using only the per-language keyword pool. Empty pool /
    /// no matches falls through to a status message.
    fn try_open_function_popup(
        &mut self,
        lang: LanguageId,
        cx: &mut Context<Self>,
    ) -> bool {
        let caret = self.selection().head;
        let snap = settings::snapshot();
        let prefix_start;
        let prefix;
        {
            let rope = &self.buffer().rope;
            prefix_start = find_prefix_start(rope, caret);
            prefix = rope.byte_slice(prefix_start..caret).to_string();
        }
        let candidates = collect_keyword_candidates(
            &prefix,
            lang,
            snap.autocompletion_ignore_numbers,
        );
        // Brief mode would strip the signature column from each candidate
        // label. The keyword pool today is identifiers only — no signature
        // is glued on — so the filter is identity. Keeping the read live so
        // the moment the API XML loader starts producing "name(args) →ret"
        // strings, the toggle takes effect with no extra wire-up.
        let candidates = apply_brief_filter(candidates, snap.autocompletion_brief);
        if candidates.is_empty() {
            self.status_message = Some(if prefix.is_empty() {
                format!("Function completion: no keywords for {}", lang.label())
            } else {
                format!("Function completion: no match for \"{prefix}\"")
            });
            cx.notify();
            return false;
        }
        let anchor = self.autocomplete_anchor(prefix_start);
        self.autocomplete = Some(AutocompleteState {
            candidates,
            selected: 0,
            replace_start: prefix_start,
            anchor,
        });
        self.status_message = None;
        cx.notify();
        true
    }

    /// Open a popup with keyword pool + buffer-word pool merged. Used by
    /// `Function and Word Completion`. Empty merged set surfaces the same
    /// "no match" status as the individual paths.
    fn try_open_combined_popup(
        &mut self,
        lang: LanguageId,
        cx: &mut Context<Self>,
    ) -> bool {
        let caret = self.selection().head;
        let snap = settings::snapshot();
        let prefix_start;
        let prefix;
        let words;
        {
            let rope = &self.buffer().rope;
            prefix_start = find_prefix_start(rope, caret);
            prefix = rope.byte_slice(prefix_start..caret).to_string();
            words = collect_word_candidates(
                rope,
                &prefix,
                snap.autocompletion_ignore_numbers,
            );
        }
        let kws = collect_keyword_candidates(
            &prefix,
            lang,
            snap.autocompletion_ignore_numbers,
        );
        // Brief filter only matters to the keyword pool (buffer words have
        // no signatures by definition). Apply before merging so a future
        // signature-bearing keyword doesn't slip past the toggle.
        let kws = apply_brief_filter(kws, snap.autocompletion_brief);
        let candidates = merge_candidates(kws, words);
        if candidates.is_empty() {
            self.status_message = Some(if prefix.is_empty() {
                "Auto-completion: no candidates available".to_string()
            } else {
                format!("Auto-completion: no match for \"{prefix}\"")
            });
            cx.notify();
            return false;
        }
        let anchor = self.autocomplete_anchor(prefix_start);
        self.autocomplete = Some(AutocompleteState {
            candidates,
            selected: 0,
            replace_start: prefix_start,
            anchor,
        });
        self.status_message = None;
        cx.notify();
        true
    }

    /// Edit ▸ Auto-Completion ▸ Word Completion (NPP `IDM_EDIT_AUTOCOMPLETE`).
    /// Buffer-scan word completion — always runs against the active buffer
    /// regardless of language.
    pub(crate) fn autocomplete_word(
        &mut self,
        _: &EditAutocompleteWord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        self.try_open_word_popup(cx);
    }

    /// Edit ▸ Auto-Completion ▸ Function Completion (NPP
    /// `IDM_EDIT_FUNCCOMPLETION`). Surfaces the active buffer's per-language
    /// static keyword + builtin list. NPP behaviour: Function alone does NOT
    /// include buffer words — those are reached via Word Completion (or the
    /// combined Function-and-Word path).
    pub(crate) fn autocomplete_function(
        &mut self,
        _: &EditAutocompleteFunc,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        let Some(lang) = self.buffer().language else {
            self.status_message = Some(
                "Function completion: no language detected for this buffer".to_string(),
            );
            cx.notify();
            return;
        };
        self.try_open_function_popup(lang, cx);
    }

    /// Edit ▸ Auto-Completion ▸ Function and Word Completion (NPP
    /// `IDM_EDIT_AUTOCOMPLETE_CURRENTFILE`). Merges the per-language keyword
    /// pool with the buffer-word scan. When the buffer has no detected
    /// language, falls through to pure Word Completion.
    pub(crate) fn autocomplete_function_and_word(
        &mut self,
        _: &EditAutocompleteFuncAndWord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        if let Some(lang) = self.buffer().language {
            self.try_open_combined_popup(lang, cx);
        } else {
            self.try_open_word_popup(cx);
        }
    }

    /// Edit ▸ Auto-Completion ▸ Function Parameters Hint (NPP
    /// `IDM_EDIT_FUNCCALLTIP`). Gated on the same language API registry as
    /// Function Completion — surface a status notice so the menu entry
    /// doesn't appear broken.
    pub(crate) fn autocomplete_function_params(
        &mut self,
        _: &EditAutocompleteFuncParams,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_focus(window) != InputFocus::Editor {
            return;
        }
        self.status_message =
            Some("Function parameter hints: language API registry not loaded".to_string());
        cx.notify();
    }

    /// Close the popup. Idempotent — safe to call when no popup is up.
    pub(crate) fn close_autocomplete(&mut self, cx: &mut Context<Self>) {
        if self.autocomplete.take().is_some() {
            cx.notify();
        }
    }

    /// Step the selected row by `delta` (positive = down, negative = up).
    /// Wraps at both ends so a held Up at the top jumps to the bottom (and
    /// vice versa), matching NPP's native popup feel.
    pub(crate) fn autocomplete_nav(&mut self, delta: i32, cx: &mut Context<Self>) {
        let Some(state) = self.autocomplete.as_mut() else {
            return;
        };
        let len = state.candidates.len() as i32;
        if len == 0 {
            return;
        }
        let next = (state.selected as i32 + delta).rem_euclid(len);
        state.selected = next as usize;
        cx.notify();
    }

    /// Apply the currently-selected candidate at the caret, then dismiss the
    /// popup. Replaces `[replace_start, caret)` with the candidate string.
    pub(crate) fn accept_autocomplete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.autocomplete.as_ref() else {
            return;
        };
        let Some(pick) = state.candidates.get(state.selected).cloned() else {
            self.close_autocomplete(cx);
            return;
        };
        self.accept_autocomplete_with(&pick, window, cx);
    }

    /// Variant called by the popup's click handler — uses the explicitly-
    /// picked candidate string instead of the keyboard-selected row so a
    /// click on an unselected row still inserts the right word.
    pub(crate) fn accept_autocomplete_with(
        &mut self,
        pick: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.autocomplete.take() else {
            return;
        };
        let caret = self.selection().head;
        let start = state.replace_start.min(caret);
        let end = caret.max(state.replace_start);
        crate::macros::record(crate::macros::MacroEvent::InsertText(pick.to_string()));
        self.replace_byte_range(start..end, pick, cx);
        cx.notify();
    }

    /// NPP `Auto-Completion ▸ From N-th character` runtime hook. Called from
    /// the keystroke insertion path after a single identifier char has
    /// landed in the buffer. Opens the popup matching the active
    /// `autocompletion_mode` once the prefix at the caret reaches the
    /// configured threshold. No-op when:
    ///   * a popup is already up (let the existing lifecycle run; typing
    ///     more chars doesn't re-trigger),
    ///   * mode is `None` (Disabled),
    ///   * the prefix at the caret is shorter than `autocompletion_from_chars`
    ///     (clamped to a minimum of 1 — the preferences UI exposes 1-9).
    /// Auto-trigger fires per keystroke, so a status toast on every miss
    /// would be obnoxious. Stash + restore `status_message` so the manual
    /// path's "no candidates" / "no match" notices stay silent when the
    /// trigger comes from typing instead of an explicit Ctrl+Enter /
    /// Ctrl+Space.
    pub(crate) fn maybe_auto_trigger_autocomplete(&mut self, cx: &mut Context<Self>) {
        if self.autocomplete.is_some() {
            return;
        }
        let snap = settings::snapshot();
        if matches!(snap.autocompletion_mode, settings::AutocompletionMode::None) {
            return;
        }
        let caret = self.selection().head;
        let prefix_start = find_prefix_start(&self.buffer().rope, caret);
        let prefix_len = caret.saturating_sub(prefix_start);
        let needed = snap.autocompletion_from_chars.max(1) as usize;
        if prefix_len < needed {
            return;
        }
        let prev_status = self.status_message.clone();
        match snap.autocompletion_mode {
            settings::AutocompletionMode::Word => {
                self.try_open_word_popup(cx);
            }
            settings::AutocompletionMode::Function => {
                if let Some(lang) = self.buffer().language {
                    self.try_open_function_popup(lang, cx);
                }
            }
            settings::AutocompletionMode::FunctionAndWord => {
                if let Some(lang) = self.buffer().language {
                    self.try_open_combined_popup(lang, cx);
                } else {
                    self.try_open_word_popup(cx);
                }
            }
            settings::AutocompletionMode::None => {}
        }
        if self.autocomplete.is_none() {
            self.status_message = prev_status;
        }
    }
}
