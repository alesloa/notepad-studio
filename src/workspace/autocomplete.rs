// Auto-Completion runtime: word scanner + popup state + action handlers.
//
// Scope:
//   * Word Completion — buffer-scans for unique identifiers sharing the
//     prefix at the caret, opens a popup with the candidates.
//   * Function Completion — optional per-language API definitions, with
//     original static keywords and common builtins while a file loads or is absent.
//   * Function and Word Completion — merges both candidate pools.
//   * Function Parameters Hint — bounded background call scanning and a
//     themed signature popup live in `call_hint` and `ui::function_hint`.
//
// Settings wiring map (so future readers don't have to grep):
//   * `autocompletion_insert_with_tab` — gates Tab acceptance, lives in
//     `edits.rs::insert_tab` (the keystroke handler is in edits, the
//     popup state is here).
//   * `autocompletion_insert_with_enter` — same story in
//     `edits.rs::insert_newline`.
//   * `autocompletion_ignore_numbers` — captured by the candidate source.
//   * `autocompletion_brief` — captured on opening; controls whether rows
//     shrink or stay visible while the prefix narrows.

use std::collections::BTreeSet;

use gpui::{Context, Pixels, Window, point};

use crate::actions::*;
use crate::settings;
use crate::syntax::{LanguageId, keywords};
use crate::tab::InputFocus;
use crate::workspace::Workspace;
use super::completion_api::ApiSnapshot;

/// Current completion session. Placement resolves fresh editor geometry at prepaint.
#[derive(Clone)]
pub(crate) struct AutocompleteState {
    /// Identifies this opening even if another popup has identical candidates.
    pub(crate) session_id: uuid::Uuid,
    pub(crate) candidates: Vec<String>,
    /// Refilter until the prefix broadens or a capped pool needs new matches.
    pub(crate) candidate_pool: Vec<String>,
    pool_prefix: String,
    source: CandidateSource,
    /// Only an unchanged initial manual Function list may select another member.
    manual_prefix: Option<ManualFunctionPrefix>,
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

#[derive(Clone)]
struct ManualFunctionPrefix {
    prefix: String,
    range: std::ops::Range<usize>,
    revision: crate::buffer::ContentRevision,
    rope: ropey::Rope,
    generation: Option<uuid::Uuid>,
}

#[derive(Clone, Copy)]
pub(super) enum CandidateKind {
    Word,
    Function(Option<LanguageId>),
    Combined(Option<LanguageId>),
}

#[derive(Clone)]
struct CandidateSource {
    kind: CandidateKind,
    ignore_numbers: bool,
    brief: bool,
    api: Option<ApiSnapshot>,
}

impl CandidateSource {
    fn prefix_at(&self, rope: &ropey::Rope, caret: usize) -> Option<(usize, usize, String)> {
        let caret = super::edits::floor_rope_char_boundary(rope, caret);
        let start = find_prefix_start_with(rope, caret, |ch| {
            self.api.as_ref().map_or_else(|| is_word_char(ch), |api| api.registry.environment.is_word_char(ch))
        })?;
        Some((start, caret, rope.byte_slice(start..caret).to_string()))
    }

    fn prefix_key(&self, prefix: &str) -> String {
        if self.api.as_ref().is_some_and(|api| !api.registry.environment.ignore_case) {
            prefix.to_owned()
        } else { prefix.to_ascii_lowercase() }
    }

    fn matches_prefix(&self, name: &str, prefix: &str) -> bool {
        name.len() > prefix.len() && self.api.as_ref().map_or_else(
            || matches_prefix(name, prefix), |api| api.registry.environment.matches_prefix(name, prefix),
        )
    }

    fn sort_names(&self, names: &mut [String]) {
        if self.api.as_ref().is_some_and(|api| api.registry.environment.ignore_case) {
            names.sort_by(|a, b| a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase()).then_with(|| a.cmp(b)));
        } else { names.sort(); }
    }

    fn functions(&self, prefix: &str, language: Option<LanguageId>) -> Vec<String> {
        self.function_matches(prefix, language, false)
    }

    fn function_matches(&self, prefix: &str, language: Option<LanguageId>, include_exact: bool) -> Vec<String> {
        if let Some(api) = &self.api {
            if api.registry.entries().is_empty() { return Vec::new(); }
            api.registry.matching(prefix).filter(|entry| {
                (include_exact || self.matches_prefix(&entry.name, prefix))
                    && !(self.ignore_numbers && entry.name.chars().next().is_some_and(|ch| ch.is_ascii_digit()))
            }).take(MAX_CANDIDATES).map(|entry| entry.name.clone()).collect()
        } else {
            language.map(|language| {
                if include_exact { keyword_candidates(prefix, language, self.ignore_numbers, true) }
                else { collect_keyword_candidates(prefix, language, self.ignore_numbers) }
            }).unwrap_or_default()
        }
    }

    fn collect(&self, rope: &ropey::Rope, prefix: &str, caret: usize) -> Vec<String> {
        let words = || self.api.as_ref().map_or_else(
            || collect_word_candidates(rope, prefix, self.ignore_numbers, caret),
            |api| candidates::collect_word_candidates_with_environment(
                rope, prefix, self.ignore_numbers, caret, Some(&api.registry.environment),
            ),
        );
        match self.kind {
            CandidateKind::Word => words(),
            CandidateKind::Function(lang) => self.functions(prefix, lang),
            CandidateKind::Combined(lang) => merge_candidates(self.functions(prefix, lang), words()),
        }
    }
}

impl AutocompleteState {
    pub(super) fn api_generation(&self) -> Option<uuid::Uuid> {
        self.source.api.as_ref().map(|api| api.generation)
    }

    pub(super) fn matches_live_prefix(&self, rope: &ropey::Rope, caret: usize, name: &str) -> bool {
        self.source.prefix_at(rope, caret).is_some_and(|(_, _, prefix)| self.source.matches_prefix(name, &prefix))
    }
}

/// Bound collection and rendering while retaining a matching selected row.
const MAX_CANDIDATES: usize = 50;

/// Minimum identifier length for a candidate to be considered. NPP also
/// filters single-char identifiers from its word scan — a 1-char popup would
/// be more noise than help.
const MIN_CANDIDATE_LEN: usize = 2;

/// Default identifier chars. Loaded API environments can add word characters.
pub(crate) fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Find the identifier start with at most 256 bytes of reverse traversal.
/// Oversized prefixes refuse completion rather than replacing a token suffix.
/// Empty prefixes return the caret, clamped and floored to a char start.
pub(crate) fn find_prefix_start(rope: &ropey::Rope, caret: usize) -> Option<usize> {
    find_prefix_start_with(rope, caret, is_word_char)
}

fn find_prefix_start_with(rope: &ropey::Rope, caret: usize, is_word: impl Fn(char) -> bool) -> Option<usize> {
    let caret = super::edits::floor_rope_char_boundary(rope, caret);
    if caret == 0 {
        return Some(0);
    }
    let mut idx = caret;
    let char_caret = rope.byte_to_char(caret);
    let mut char_idx = char_caret;
    while char_idx > 0 {
        let c = rope.char(char_idx - 1);
        if !is_word(c) {
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
    keyword_candidates(prefix, lang, ignore_numbers, false)
}

fn keyword_candidates(prefix: &str, lang: LanguageId, ignore_numbers: bool, include_exact: bool) -> Vec<String> {
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
        if !include_exact && word.eq_ignore_ascii_case(prefix) {
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

fn matches_prefix(candidate: &str, prefix: &str) -> bool {
    candidate.len() > prefix.len()
        && candidate.get(..prefix.len()).is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

/// PowerEditor/src/ScintillaComponent/AutoCompletion.cpp:1065–1083 —
/// the ordinary list keeps its rows. Prefer live matches if the scan cap
/// discovers new ones, then retain older visible rows within the same cap.
fn retain_visible_rows(matches: &[String], previous: &[String]) -> Vec<String> {
    let mut rows: BTreeSet<String> = matches.iter().take(MAX_CANDIDATES).cloned().collect();
    for candidate in previous {
        if rows.len() >= MAX_CANDIDATES { break; }
        rows.insert(candidate.clone());
    }
    rows.into_iter().collect()
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

    /// Lists pin one immutable registry generation for their whole opening.
    pub(super) fn open_completion_kind(
        &mut self, kind: CandidateKind, manual: bool, cx: &mut Context<Self>,
    ) -> bool {
        self.close_autocomplete(cx);
        if !self.autocomplete_editable() { return false; }
        let caret = self.selection().head;
        let snap = settings::snapshot();
        let source = CandidateSource {
            kind, ignore_numbers: snap.autocompletion_ignore_numbers, brief: snap.autocompletion_brief,
            api: self.loaded_api_snapshot(self.buffer().language),
        };
        let Some((prefix_start, _, prefix)) = source.prefix_at(&self.buffer().rope, caret) else { return false; };
        if !manual && !prefix_reaches_threshold(&prefix, snap.autocompletion_from_chars) { return false; }
        let initial_function = manual && matches!(kind, CandidateKind::Function(_));
        let matching = match kind {
            CandidateKind::Function(language) if initial_function => source.function_matches(&prefix, language, true),
            _ => source.collect(&self.buffer().rope, &prefix, caret),
        };
        let selected_name = matching.first().cloned();
        // PowerEditor/src/ScintillaComponent/AutoCompletion.cpp:451-454 —
        // manual Function Completion starts with names outside the prefix too.
        // ScintillaBase.cxx:369-372 / AutoComplete.cxx:249-300 — cancel if no
        // prefix match exists; an explicitly selected member can complete it.
        let mut candidates = if let CandidateKind::Function(language) = kind {
            if initial_function && !matching.is_empty() {
                retain_visible_rows(&matching, &source.functions("", language))
            } else { matching }
        } else { matching };
        source.sort_names(&mut candidates);
        let selected = selected_name.and_then(|name| candidates.iter().position(|candidate| *candidate == name)).unwrap_or(0);
        if candidates.is_empty() {
            self.status_message = Some(match kind {
                CandidateKind::Function(language) if prefix.is_empty() => {
                    format!("Function completion: no API definitions or original keywords for {}", language.map(LanguageId::label).unwrap_or("Normal Text"))
                }
                _ if prefix.is_empty() => "Auto-completion: no candidates available".to_string(),
                _ => format!("Auto-completion: no match for \"{prefix}\""),
            });
            cx.notify();
            return false;
        }
        let pool_prefix = source.prefix_key(&prefix);
        let manual_prefix = initial_function.then(|| ManualFunctionPrefix {
            prefix, range: prefix_start..caret, revision: self.buffer().raw_revision.clone(),
            rope: self.buffer().rope.clone(), generation: source.api.as_ref().map(|api| api.generation),
        });
        self.autocomplete = Some(AutocompleteState {
            session_id: uuid::Uuid::new_v4(),
            candidate_pool: candidates.clone(),
            pool_prefix,
            source,
            manual_prefix,
            candidates,
            caret,
            allow_empty_prefix: prefix_start == caret,
            selected,
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
        if !window.is_window_active() || !self.focus_handle.is_focused(window) || self.input_focus(window) != InputFocus::Editor {
            self.close_autocomplete(cx);
            return;
        }
        self.open_completion_kind(CandidateKind::Word, true, cx);
        self.watch_autocomplete_focus(window, cx);
        self.request_completion_api(CandidateKind::Word, true, window, cx);
    }

    /// Edit ▸ Auto-Completion ▸ Function Completion (NPP
    /// `IDM_EDIT_FUNCCOMPLETION`). Surfaces the active buffer's per-language
    /// API names or original static keywords. NPP behaviour: Function alone does NOT
    /// include buffer words — those are reached via Word Completion (or the
    /// combined Function-and-Word path).
    pub(crate) fn autocomplete_function(
        &mut self,
        _: &EditAutocompleteFunc,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !window.is_window_active() || !self.focus_handle.is_focused(window) || self.input_focus(window) != InputFocus::Editor {
            self.close_autocomplete(cx);
            return;
        }
        let kind = CandidateKind::Function(self.buffer().language);
        self.open_completion_kind(kind, true, cx);
        self.watch_autocomplete_focus(window, cx);
        self.request_completion_api(kind, true, window, cx);
    }

    /// Edit ▸ Auto-Completion ▸ Function and Word Completion (NPP
    /// `IDM_EDIT_AUTOCOMPLETE_CURRENTFILE`). Merges the per-language keyword
    /// pool with the buffer-word scan. Normal Text can load normal.xml.
    pub(crate) fn autocomplete_function_and_word(
        &mut self,
        _: &EditAutocompleteFuncAndWord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !window.is_window_active() || !self.focus_handle.is_focused(window) || self.input_focus(window) != InputFocus::Editor {
            self.close_autocomplete(cx);
            return;
        }
        let kind = CandidateKind::Combined(self.buffer().language);
        self.open_completion_kind(kind, true, cx);
        self.watch_autocomplete_focus(window, cx);
        self.request_completion_api(kind, true, window, cx);
    }

    /// Edit ▸ Auto-Completion ▸ Function Parameters Hint (NPP
    /// `IDM_EDIT_FUNCCALLTIP`). Load optional signatures without opening
    /// a completion list, then scan the current call in the background.
    pub(crate) fn autocomplete_function_params(
        &mut self,
        _: &EditAutocompleteFuncParams,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_function_hint(window, cx);
    }

    /// Close the popup. Idempotent — safe to call when no popup is up.
    pub(crate) fn close_autocomplete(&mut self, cx: &mut Context<Self>) {
        self.completion_api.epoch = uuid::Uuid::new_v4();
        self.autocomplete_blur_subscription = None;
        self.autocomplete_activation_subscription = None;
        if self.autocomplete.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn watch_autocomplete_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session_id) = self.autocomplete.as_ref().map(|state| state.session_id) else { return; };
        if self.autocomplete_blur_subscription.is_none() {
            let focus = self.focus_handle.clone();
            self.autocomplete_blur_subscription = Some(cx.on_blur(&focus, window, move |w, _, cx| {
                w.dismiss_autocomplete_session(session_id, cx);
            }));
        }
        // FocusHandle retains its id in an inactive window, and rendered blur
        // events wait for a frame. Cancel on the platform notification itself.
        if self.autocomplete_activation_subscription.is_none() {
            self.autocomplete_activation_subscription = Some(cx.observe_window_activation(window, move |w, window, cx| {
                if !window.is_window_active() {
                    w.dismiss_autocomplete_session(session_id, cx);
                }
            }));
        }
    }

    pub(super) fn autocomplete_editable(&self) -> bool {
        self.tabs.get(self.active).is_some_and(|tab| {
            tab.view_mode == crate::tab::TabViewMode::Edit && !tab.buffer.read_only
                && tab.selection.is_empty() && tab.block_selection.is_none()
        })
    }

    pub(crate) fn autocomplete_context_valid(&self, window: &Window) -> bool {
        let Some(state) = self.autocomplete.as_ref() else { return false; };
        window.is_window_active() && self.focus_handle.is_focused(window)
            && self.input_focus(window) == InputFocus::Editor
            && self.autocomplete_editable()
            && self.tabs.get(self.active).is_some_and(|tab| tab.session_id == state.tab_id)
            && self.selection().head == state.caret
    }

    /// Painted callbacks belong to one opening. Ignore stale callbacks without
    /// accepting, selecting or dismissing a more recent completion session.
    pub(crate) fn dismiss_autocomplete_session(&mut self, session_id: uuid::Uuid, cx: &mut Context<Self>) {
        if self.autocomplete.as_ref().is_some_and(|state| state.session_id == session_id) {
            self.close_autocomplete(cx);
        }
    }

    pub(crate) fn accept_autocomplete_session(
        &mut self, session_id: uuid::Uuid, pick: &str, window: &mut Window, cx: &mut Context<Self>,
    ) -> bool {
        if !self.autocomplete.as_ref().is_some_and(|state| state.session_id == session_id) {
            return false;
        }
        self.accept_autocomplete_with(pick, window, cx)
    }

    pub(crate) fn scroll_autocomplete_session(
        &mut self, session_id: uuid::Uuid, delta: i32, window: &Window, cx: &mut Context<Self>,
    ) {
        if !self.autocomplete.as_ref().is_some_and(|state| state.session_id == session_id) {
            return;
        }
        if !self.autocomplete_context_valid(window) {
            self.dismiss_autocomplete_session(session_id, cx);
            return;
        }
        if let Some(state) = self.autocomplete.as_mut() {
            let previous = state.selected;
            if delta < 0 {
                state.selected = state.selected.saturating_sub(1);
            } else if delta > 0 {
                state.selected = state.selected.saturating_add(1).min(state.candidates.len().saturating_sub(1));
            }
            if state.selected != previous { cx.notify(); }
        }
    }

    // NppNotification.cpp:200-217 and ScintillaBase.cxx:415-434 — keep an
    // active list aligned with inserted/deleted characters, cancel on stop chars.
    pub(crate) fn refresh_autocomplete_after_edit(&mut self, cx: &mut Context<Self>) {
        // Revoke permanently, even if a later Undo restores the same revision.
        if let Some(state) = self.autocomplete.as_mut() { state.manual_prefix = None; }
        let Some(state) = self.autocomplete.as_ref() else { return; };
        if !self.autocomplete_editable()
            || !self.tabs.get(self.active).is_some_and(|tab| tab.session_id == state.tab_id)
        {
            self.close_autocomplete(cx);
            return;
        }
        let Some((start, caret, prefix)) = state.source.prefix_at(&self.buffer().rope, self.selection().head) else {
            self.close_autocomplete(cx);
            return;
        };
        if start != state.replace_start || (prefix.is_empty() && !state.allow_empty_prefix) {
            self.close_autocomplete(cx);
            return;
        }
        let previous = state.candidates.get(state.selected).cloned();
        let lower = state.source.prefix_key(&prefix);
        // Requery only when cached matches cannot represent the new prefix.
        // Word collection still visits at most 128 KiB around the caret.
        let broadening = !lower.starts_with(&state.pool_prefix);
        let refill = broadening
            || (state.candidate_pool.len() == MAX_CANDIDATES && lower != state.pool_prefix);
        let pool = refill.then(|| state.source.collect(&self.buffer().rope, &prefix, caret));
        let mut filtered: Vec<String> = pool.as_ref().unwrap_or(&state.candidate_pool).iter()
            .filter(|word| state.source.matches_prefix(word, &prefix)).cloned().collect();
        // A broader query may cap out before a still-matching selected row.
        // Retain that row without increasing the existing candidate bound.
        if let Some(pick) = previous.as_ref().filter(|pick| state.source.matches_prefix(pick, &prefix)) {
            if !filtered.contains(pick) {
                filtered.truncate(MAX_CANDIDATES.saturating_sub(1));
                filtered.push(pick.clone());
                state.source.sort_names(&mut filtered);
            }
        }
        if filtered.is_empty() {
            self.close_autocomplete(cx);
            return;
        }
        let selected = previous.filter(|pick| state.source.matches_prefix(pick, &prefix))
            .unwrap_or_else(|| filtered.first().cloned().unwrap_or_default());
        let mut visible = if state.source.brief || broadening {
            filtered
        } else {
            retain_visible_rows(&filtered, &state.candidates)
        };
        state.source.sort_names(&mut visible);
        if let Some(state) = self.autocomplete.as_mut() {
            if let Some(pool) = pool {
                state.candidate_pool = pool;
                state.pool_prefix = lower;
            }
            state.selected = visible.iter().position(|word| *word == selected).unwrap_or(0);
            state.candidates = visible;
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
    /// Returns true for a handled exact-name no-op too. A missing or stale
    /// popup returns false so Enter / Tab can run their normal actions.
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
        self.autocomplete_activation_subscription = None;
        cx.notify();
        if !state.candidates.iter().any(|candidate| candidate == pick) { return false; }
        if matches!(state.source.kind, CandidateKind::Function(_))
            && state.source.api.as_ref().is_some_and(|api| api.registry.find(pick).is_none())
        { return false; }
        let caret = self.selection().head;
        let Some((start, caret, prefix)) = state.source.prefix_at(&self.buffer().rope, caret) else { return false; };
        let range = if state.source.api.is_some() {
            (start == state.replace_start).then_some(start..caret)
        } else { accept_range(&self.buffer().rope, state.replace_start, caret) };
        let Some(range) = range else { return false; };
        let initial_member = state.manual_prefix.as_ref().is_some_and(|pin| {
            matches!(state.source.kind, CandidateKind::Function(_))
                && pin.range == range && pin.prefix == prefix
                && pin.revision == self.buffer().raw_revision && pin.rope.is_instance(&self.buffer().rope)
                && pin.generation == state.api_generation()
        });
        if (prefix.is_empty() && !state.allow_empty_prefix)
            || (!initial_member && !state.source.matches_prefix(pick, &prefix))
        { return false; }
        if prefix == pick { return true; }
        crate::macros::record(crate::macros::completion_event(&prefix, pick));
        self.replace_byte_range(range, pick, cx);
        true
    }

    /// Auto-open after committed identifier input reaches the configured
    /// character threshold. Existing lists filter through after_edit_flush;
    /// composition updates suppress opening and automatic misses stay silent.
    pub(crate) fn maybe_auto_trigger_autocomplete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.autocomplete_auto_trigger_suppressed || !window.is_window_active() || !self.focus_handle.is_focused(window)
            || self.input_focus(window) != InputFocus::Editor || !self.autocomplete_editable()
        { return; }
        if self.autocomplete.is_some() {
            return;
        }
        let snap = settings::snapshot();
        if matches!(snap.autocompletion_mode, settings::AutocompletionMode::None) {
            return;
        }
        let kind = match snap.autocompletion_mode {
            settings::AutocompletionMode::None => return,
            settings::AutocompletionMode::Word => CandidateKind::Word,
            settings::AutocompletionMode::Function => CandidateKind::Function(self.buffer().language),
            settings::AutocompletionMode::FunctionAndWord => CandidateKind::Combined(self.buffer().language),
        };
        let source = CandidateSource {
            kind, ignore_numbers: snap.autocompletion_ignore_numbers, brief: snap.autocompletion_brief,
            api: self.loaded_api_snapshot(self.buffer().language),
        };
        let caret = self.selection().head;
        let Some((_, _, prefix)) = source.prefix_at(&self.buffer().rope, caret) else { return; };
        if !prefix_reaches_threshold(&prefix, snap.autocompletion_from_chars) {
            return;
        }
        let prev_status = self.status_message.clone();
        self.open_completion_kind(kind, false, cx);
        if self.autocomplete.is_none() {
            self.status_message = prev_status;
        }
        self.watch_autocomplete_focus(window, cx);
        self.request_completion_api(kind, false, window, cx);
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

#[cfg(test)]
#[path = "autocomplete_routing_tests.rs"]
mod completion_routing_tests;

#[cfg(test)]
#[path = "autocomplete_brief_tests.rs"]
mod completion_brief_tests;

#[cfg(test)]
#[path = "autocomplete_api_tests.rs"]
pub(super) mod completion_api_tests;
