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

use gpui::{Context, Pixels, Window, point};

use crate::actions::*;
use crate::settings;
use crate::syntax::{LanguageId, keywords};
use crate::tab::InputFocus;
use crate::workspace::Workspace;

/// Current completion session. Placement resolves fresh editor geometry at prepaint.
#[derive(Clone)]
pub(crate) struct AutocompleteState {
    pub(crate) candidates: Vec<String>,
    /// Refilter until the prefix broadens or a capped pool needs new matches.
    pub(crate) candidate_pool: Vec<String>,
    pool_prefix: String,
    source: CandidateSource,
    pub(crate) caret: usize,
    pub(crate) allow_empty_prefix: bool,
    pub(crate) selected: usize,
    /// Byte offset where the matched prefix starts in the active buffer's
    /// rope. Replacement on accept covers `[replace_start, caret)`.
    pub(crate) replace_start: usize,
    /// Session id of the tab the popup was opened on; `replace_start` means
    /// nothing in any other buffer.
    pub(crate) tab_id: uuid::Uuid,
}

#[derive(Clone, Copy)]
enum CandidateKind {
    Word,
    Function(LanguageId),
    Combined(LanguageId),
}

#[derive(Clone, Copy)]
struct CandidateSource {
    kind: CandidateKind,
    ignore_numbers: bool,
    brief: bool,
}

impl CandidateSource {
    fn collect(&self, rope: &ropey::Rope, prefix: &str, caret: usize) -> Vec<String> {
        match self.kind {
            CandidateKind::Word => collect_word_candidates(rope, prefix, self.ignore_numbers, caret),
            CandidateKind::Function(lang) => apply_brief_filter(
                collect_keyword_candidates(prefix, lang, self.ignore_numbers), self.brief,
            ),
            CandidateKind::Combined(lang) => merge_candidates(
                apply_brief_filter(collect_keyword_candidates(prefix, lang, self.ignore_numbers), self.brief),
                collect_word_candidates(rope, prefix, self.ignore_numbers, caret),
            ),
        }
    }
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

/// Find the identifier start with at most 256 bytes of reverse traversal.
/// Oversized prefixes refuse completion rather than replacing a token suffix.
/// Empty prefixes return the caret, clamped and floored to a char start.
pub(crate) fn find_prefix_start(rope: &ropey::Rope, caret: usize) -> Option<usize> {
    let caret = super::edits::floor_rope_char_boundary(rope, caret);
    if caret == 0 {
        return Some(0);
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
        if caret.saturating_sub(idx) > candidates::MAX_CANDIDATE_BYTES {
            return None;
        }
    }
    Some(idx)
}

/// The identifier prefix ending at `caret`: its start byte, the caret byte
/// the prefix ends at, and the prefix text.
fn prefix_at(rope: &ropey::Rope, caret: usize) -> Option<(usize, usize, String)> {
    let caret = super::edits::floor_rope_char_boundary(rope, caret);
    let start = find_prefix_start(rope, caret)?;
    Some((start, caret, rope.byte_slice(start..caret).to_string()))
}

fn prefix_reaches_threshold(prefix: &str, configured: u32) -> bool {
    prefix.chars().count() >= configured.max(1) as usize
}

/// Byte range an accepted candidate replaces: the identifier prefix ending
/// at the caret, provided it still starts where the popup opened. `None`
/// once the caret has left that prefix (moved away, typed a separator,
/// deleted past its start), so accept cancels instead of replacing
/// whatever lies between the old start and the new caret.
fn accept_range(
    rope: &ropey::Rope,
    replace_start: usize,
    caret: usize,
) -> Option<std::ops::Range<usize>> {
    let (start, caret, _) = prefix_at(rope, caret)?;
    (start == replace_start).then_some(start..caret)
}

#[path = "autocomplete_candidates.rs"]
mod candidates;
pub(crate) use candidates::collect_word_candidates;

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
    /// Resolved after the editor prepaints this frame. Missing or offscreen
    /// geometry hides the list rather than covering the editor's first row.
    pub(crate) fn autocomplete_caret_bounds(&self) -> Option<gpui::Bounds<Pixels>> {
        let tab = self.tabs.get(self.active)?;
        if tab.view_mode != crate::tab::TabViewMode::Edit {
            return None;
        }
        let caret = tab.selection.clamped(&tab.buffer.rope).head;
        // A wrap boundary belongs to the next visual row, just as the caret
        // painter resolves it; the later row wins when both include it.
        let line = self.last_lines.iter().rev().find(|line| {
            caret >= line.line_byte_start && caret <= line.line_byte_start.saturating_add(line.line_byte_len)
        })?;
        let x = line.bounds.left() + line.shaped.x_for_index(caret.saturating_sub(line.line_byte_start));
        Some(gpui::Bounds::new(point(x, line.bounds.top()), gpui::size(gpui::px(1.), line.bounds.size.height)))
    }

    /// Compute prefix + open a popup for it. Empty candidate list = no
    /// popup, ephemeral status message instead (NPP also no-ops when the
    /// scan finds nothing). Returns true when a popup was opened.
    fn try_open_word_popup(&mut self, cx: &mut Context<Self>) -> bool {
        self.close_autocomplete(cx);
        if !self.autocomplete_editable() { return false; }
        let caret = self.selection().head;
        let prefix_start;
        let prefix;
        let snap = settings::snapshot();
        let candidates;
        {
            let rope = &self.buffer().rope;
            let Some(found) = prefix_at(rope, caret) else { return false; };
            (prefix_start, _, prefix) = found;
            candidates = collect_word_candidates(
                rope,
                &prefix,
                snap.autocompletion_ignore_numbers,
                caret,
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
        self.autocomplete = Some(AutocompleteState {
            candidate_pool: candidates.clone(),
            pool_prefix: prefix.to_ascii_lowercase(),
            source: CandidateSource {
                kind: CandidateKind::Word,
                ignore_numbers: snap.autocompletion_ignore_numbers,
                brief: snap.autocompletion_brief,
            },
            candidates,
            caret,
            allow_empty_prefix: prefix.is_empty(),
            selected: 0,
            replace_start: prefix_start,
            tab_id: self.tabs[self.active].session_id,
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
        self.close_autocomplete(cx);
        if !self.autocomplete_editable() { return false; }
        let caret = self.selection().head;
        let snap = settings::snapshot();
        let Some((prefix_start, _, prefix)) = prefix_at(&self.buffer().rope, caret) else { return false; };
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
        self.autocomplete = Some(AutocompleteState {
            candidate_pool: candidates.clone(),
            pool_prefix: prefix.to_ascii_lowercase(),
            source: CandidateSource {
                kind: CandidateKind::Function(lang),
                ignore_numbers: snap.autocompletion_ignore_numbers,
                brief: snap.autocompletion_brief,
            },
            candidates,
            caret,
            allow_empty_prefix: prefix.is_empty(),
            selected: 0,
            replace_start: prefix_start,
            tab_id: self.tabs[self.active].session_id,
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
        self.close_autocomplete(cx);
        if !self.autocomplete_editable() { return false; }
        let caret = self.selection().head;
        let snap = settings::snapshot();
        let prefix_start;
        let prefix;
        let words;
        {
            let rope = &self.buffer().rope;
            let Some(found) = prefix_at(rope, caret) else { return false; };
            (prefix_start, _, prefix) = found;
            words = collect_word_candidates(
                rope,
                &prefix,
                snap.autocompletion_ignore_numbers,
                caret,
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
        self.autocomplete = Some(AutocompleteState {
            candidate_pool: candidates.clone(),
            pool_prefix: prefix.to_ascii_lowercase(),
            source: CandidateSource {
                kind: CandidateKind::Combined(lang),
                ignore_numbers: snap.autocompletion_ignore_numbers,
                brief: snap.autocompletion_brief,
            },
            candidates,
            caret,
            allow_empty_prefix: prefix.is_empty(),
            selected: 0,
            replace_start: prefix_start,
            tab_id: self.tabs[self.active].session_id,
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
        if !self.focus_handle.is_focused(window) || self.input_focus(window) != InputFocus::Editor {
            self.close_autocomplete(cx);
            return;
        }
        self.try_open_word_popup(cx);
        self.watch_autocomplete_focus(window, cx);
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
        if !self.focus_handle.is_focused(window) || self.input_focus(window) != InputFocus::Editor {
            self.close_autocomplete(cx);
            return;
        }
        self.close_autocomplete(cx);
        let Some(lang) = self.buffer().language else {
            self.status_message = Some(
                "Function completion: no language detected for this buffer".to_string(),
            );
            cx.notify();
            return;
        };
        self.try_open_function_popup(lang, cx);
        self.watch_autocomplete_focus(window, cx);
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
        if !self.focus_handle.is_focused(window) || self.input_focus(window) != InputFocus::Editor {
            self.close_autocomplete(cx);
            return;
        }
        if let Some(lang) = self.buffer().language {
            self.try_open_combined_popup(lang, cx);
        } else {
            self.try_open_word_popup(cx);
        }
        self.watch_autocomplete_focus(window, cx);
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
        if !self.focus_handle.is_focused(window) || self.input_focus(window) != InputFocus::Editor {
            self.close_autocomplete(cx);
            return;
        }
        self.status_message =
            Some("Function parameter hints: language API registry not loaded".to_string());
        cx.notify();
    }

    /// Close the popup. Idempotent — safe to call when no popup is up.
    pub(crate) fn close_autocomplete(&mut self, cx: &mut Context<Self>) {
        self.autocomplete_blur_subscription = None;
        if self.autocomplete.take().is_some() {
            cx.notify();
        }
    }

    fn watch_autocomplete_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.autocomplete.is_some() && self.autocomplete_blur_subscription.is_none() {
            let focus = self.focus_handle.clone();
            self.autocomplete_blur_subscription = Some(cx.on_blur(&focus, window, |w, _, cx| {
                w.close_autocomplete(cx);
            }));
        }
    }

    fn autocomplete_editable(&self) -> bool {
        self.tabs.get(self.active).is_some_and(|tab| {
            !tab.buffer.read_only && tab.selection.is_empty() && tab.block_selection.is_none()
        })
    }

    pub(crate) fn autocomplete_context_valid(&self, window: &Window) -> bool {
        let Some(state) = self.autocomplete.as_ref() else { return false; };
        self.focus_handle.is_focused(window)
            && self.input_focus(window) == InputFocus::Editor
            && self.autocomplete_editable()
            && self.tabs.get(self.active).is_some_and(|tab| tab.session_id == state.tab_id)
            && self.selection().head == state.caret
    }

    // NppNotification.cpp:200-217 and ScintillaBase.cxx:415-434 — keep an
    // active list aligned with inserted/deleted characters, cancel on stop chars.
    pub(crate) fn refresh_autocomplete_after_edit(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.autocomplete.as_ref() else { return; };
        if !self.autocomplete_editable()
            || !self.tabs.get(self.active).is_some_and(|tab| tab.session_id == state.tab_id)
        {
            self.close_autocomplete(cx);
            return;
        }
        let Some((start, caret, prefix)) = prefix_at(&self.buffer().rope, self.selection().head) else {
            self.close_autocomplete(cx);
            return;
        };
        if start != state.replace_start || (prefix.is_empty() && !state.allow_empty_prefix) {
            self.close_autocomplete(cx);
            return;
        }
        let previous = state.candidates.get(state.selected).cloned();
        let lower = prefix.to_ascii_lowercase();
        // Requery only when cached matches cannot represent the new prefix.
        // Word collection still visits at most 128 KiB around the caret.
        let refill = !lower.starts_with(&state.pool_prefix)
            || (state.candidate_pool.len() == MAX_CANDIDATES && lower != state.pool_prefix);
        let pool = refill.then(|| state.source.collect(&self.buffer().rope, &prefix, caret));
        let filtered: Vec<String> = pool.as_ref().unwrap_or(&state.candidate_pool).iter().filter(|word| {
            !word.eq_ignore_ascii_case(&prefix) && word.to_ascii_lowercase().starts_with(&lower)
        }).cloned().collect();
        if filtered.is_empty() {
            self.close_autocomplete(cx);
            return;
        }
        if let Some(state) = self.autocomplete.as_mut() {
            if let Some(pool) = pool {
                state.candidate_pool = pool;
                state.pool_prefix = lower;
            }
            state.selected = previous.and_then(|pick| filtered.iter().position(|word| *word == pick)).unwrap_or(0);
            state.candidates = filtered;
            state.caret = caret;
            state.allow_empty_prefix &= prefix.is_empty();
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
    /// Returns false when nothing was inserted (no popup, or the popup went
    /// stale and was closed), so the Enter / Tab caller can run the key's
    /// normal action instead.
    pub(crate) fn accept_autocomplete(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(state) = self.autocomplete.as_ref() else {
            return false;
        };
        let Some(pick) = state.candidates.get(state.selected).cloned() else {
            self.close_autocomplete(cx);
            return false;
        };
        self.accept_autocomplete_with(&pick, window, cx)
    }

    /// Variant called by the popup's click handler — uses the explicitly-
    /// picked candidate string instead of the keyboard-selected row so a
    /// click on an unselected row still inserts the right word.
    ///
    /// Revalidate even after dismissal hooks: a queued row click must not
    /// replace changed text, another tab, a selection, or a read-only buffer.
    pub(crate) fn accept_autocomplete_with(
        &mut self,
        pick: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.autocomplete_context_valid(window) {
            self.close_autocomplete(cx);
            return false;
        }
        let Some(state) = self.autocomplete.take() else {
            return false;
        };
        self.autocomplete_blur_subscription = None;
        cx.notify();
        if !state.candidates.iter().any(|candidate| candidate == pick) { return false; }
        let caret = self.selection().head;
        let Some(range) = accept_range(&self.buffer().rope, state.replace_start, caret) else {
            return false;
        };
        let Some((_, _, prefix)) = prefix_at(&self.buffer().rope, caret) else { return false; };
        if (prefix.is_empty() && !state.allow_empty_prefix)
            || !pick.to_ascii_lowercase().starts_with(&prefix.to_ascii_lowercase())
        { return false; }
        crate::macros::record(crate::macros::MacroEvent::InsertText(pick.to_string()));
        self.replace_byte_range(range, pick, cx);
        true
    }

    /// Auto-open after committed identifier input reaches the configured
    /// character threshold. Existing lists filter through after_edit_flush;
    /// composition updates suppress opening and automatic misses stay silent.
    pub(crate) fn maybe_auto_trigger_autocomplete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.autocomplete_auto_trigger_suppressed || !self.focus_handle.is_focused(window)
            || self.input_focus(window) != InputFocus::Editor || !self.autocomplete_editable()
        { return; }
        if self.autocomplete.is_some() {
            return;
        }
        let snap = settings::snapshot();
        if matches!(snap.autocompletion_mode, settings::AutocompletionMode::None) {
            return;
        }
        let caret = self.selection().head;
        let Some((_, _, prefix)) = prefix_at(&self.buffer().rope, caret) else { return; };
        if !prefix_reaches_threshold(&prefix, snap.autocompletion_from_chars) {
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
        self.watch_autocomplete_focus(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{accept_range, collect_word_candidates, find_prefix_start, prefix_at};
    use ropey::Rope;

    #[test]
    fn prefix_start_walks_back_over_word_chars() {
        let rope = Rope::from_str("let foo_bar = fo");
        assert_eq!(find_prefix_start(&rope, rope.len_bytes()), Some(14));
        assert_eq!(find_prefix_start(&rope, 11), Some(4));
        assert_eq!(find_prefix_start(&rope, 0), Some(0));
        let rope = Rope::from_str("x = café");
        assert_eq!(prefix_at(&rope, rope.len_bytes()), Some((4, 9, "café".to_string())));
        let rope = Rope::from_str("a\r\n中文");
        assert_eq!(prefix_at(&rope, rope.len_bytes()), Some((3, 9, "中文".to_string())));
    }

    #[test]
    fn prefix_with_caret_past_end_or_mid_char_does_not_panic() {
        // A caret past EOF hit `byte_to_char` out of bounds; one inside a
        // multi-byte char with no word char before it came back unchanged
        // and `byte_slice` panicked on it.
        let rope = Rope::from_str("ab 👍");
        assert_eq!(prefix_at(&rope, 99), Some((7, 7, String::new())));
        assert_eq!(prefix_at(&rope, 5), Some((3, 3, String::new())));
        for text in ["", "é", "中文 x", "👨‍👩‍👧ab", "🇫🇷\r\nzz", "a\rb"] {
            let rope = Rope::from_str(text);
            for caret in 0..=text.len() + 2 {
                let (start, end, prefix) = prefix_at(&rope, caret).expect("small prefix");
                assert!(start <= end && end <= text.len(), "{text:?} @ {caret}");
                assert_eq!(&text[start..end], prefix);
            }
        }
    }

    #[test]
    fn accept_replaces_only_the_prefix_the_popup_opened_on() {
        let rope = Rope::from_str("fo bar");
        assert_eq!(accept_range(&rope, 0, 2), Some(0..2));
        // Typing more word chars or backspacing inside the prefix still
        // accepts against the same start.
        assert_eq!(accept_range(&rope, 0, 1), Some(0..1));
        assert_eq!(accept_range(&rope, 0, 0), Some(0..0));
    }

    #[test]
    fn accept_after_caret_left_the_prefix_is_rejected() {
        // Popup opened on "fo" (start 0). Ctrl+End moved the caret to EOF:
        // the old min/max range replaced everything from 0 to EOF.
        let rope = Rope::from_str("fo bar baz\nmore text");
        assert_eq!(accept_range(&rope, 0, rope.len_bytes()), None);
        // Caret moved before the prefix start.
        let rope = Rope::from_str("abc fo");
        assert_eq!(accept_range(&rope, 4, 2), None);
        // Caret past EOF or mid-char never panics.
        assert_eq!(accept_range(&rope, 4, 99), Some(4..6));
        let rope = Rope::from_str("x é");
        assert_eq!(accept_range(&rope, 2, 3), Some(2..2));
    }

    #[test]
    fn word_candidates_on_edge_buffers() {
        assert!(collect_word_candidates(&Rope::new(), "", true, 0).is_empty());
        let rope = Rope::from_str("foo\r\nfood\rfool 12ab café_x 👍fob");
        assert_eq!(collect_word_candidates(&rope, "fo", true, rope.len_bytes()), ["fob", "foo", "food", "fool"]);
        assert_eq!(collect_word_candidates(&rope, "caf", true, rope.len_bytes()), vec!["café_x".to_string()]);
        assert!(collect_word_candidates(&rope, "1", true, rope.len_bytes()).is_empty());
        assert_eq!(collect_word_candidates(&rope, "1", false, rope.len_bytes()), vec!["12ab".to_string()]);
    }
}
#[cfg(test)]
#[path = "autocomplete_state_tests.rs"]
mod completion_state_tests;
