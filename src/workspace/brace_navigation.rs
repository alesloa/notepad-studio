// PowerEditor/src/NppCommands.cpp:1940-1958,4434-4435 - brace navigation/macros.
use super::{brace_match::{self, Pair}, Workspace};
use crate::{actions::{SearchGoToMatchingBrace, SearchSelectInBetween}, buffer::ContentRevision,
    macros::{self, MacroEvent}, syntax::{HighlightSpan, LanguageId}, tab::{InputFocus, TabViewMode}};
use gpui::{AppContext, Context, Window};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use uuid::Uuid;

const PREPARING: &str = "Finding matching brace; Escape cancels";

#[derive(Clone, Copy)]
pub(super) enum Navigation { GoTo, Select }

struct Source {
    rope: ropey::Rope,
    styles: Arc<Vec<HighlightSpan>>,
}

struct Stamp {
    tab: Uuid,
    instance: Arc<()>,
    activity: ContentRevision,
    revision: ContentRevision,
    language: Option<LanguageId>,
    selection: (usize, usize, bool),
    block: Option<(usize, usize, usize, usize, bool)>,
    epoch: super::source_input::SourceInputToken,
    input: gpui::InputContextToken,
    source: Option<Source>,
    executor: gpui::BackgroundExecutor,
}

impl Drop for Stamp {
    fn drop(&mut self) {
        if let Some(source) = self.source.take() {
            self.executor.spawn(async move { drop(source); }).detach();
        }
    }
}

fn available(w: &Workspace, window: &Window, navigation: bool) -> bool {
    !w.window_closed && window.is_window_active() && !w.is_composing_text(window)
        && w.tabs.get(w.active).is_some_and(|tab| tab.view_mode == TabViewMode::Edit
            && tab.buffer.loading.is_none() && tab.edit_transaction.is_none())
        && (!navigation || (w.focus_handle.is_focused(window) && w.input_focus(window) == InputFocus::Editor))
}

fn block(w: &Workspace) -> Option<(usize, usize, usize, usize, bool)> {
    w.tabs.get(w.active)?.block_selection.map(|b| (b.anchor_line, b.head_line, b.anchor_col, b.head_col, b.is_dragging))
}

impl Stamp {
    fn capture(w: &Workspace, window: &Window, cx: &Context<Workspace>) -> Option<Self> {
        if !available(w, window, false) { return None; }
        let tab = w.tabs.get(w.active)?;
        Some(Self { tab: tab.session_id, instance: tab.instance.clone(), activity: tab.content_activity.clone(),
            revision: tab.buffer.raw_revision.clone(), language: tab.buffer.language,
            selection: (tab.selection.anchor, tab.selection.head, tab.selection.is_dragging), block: block(w),
            epoch: w.source_input_token(), input: window.input_context_token(),
            source: Some(Source { rope: tab.buffer.rope.clone(), styles: tab.buffer.highlights.clone() }),
            executor: cx.background_executor().clone() })
    }

    fn valid(&self, w: &Workspace, window: &Window) -> bool {
        let Some(tab) = w.tabs.get(w.active) else { return false; };
        let Some(source) = &self.source else { return false; };
        available(w, window, false) && self.tab == tab.session_id && Arc::ptr_eq(&self.instance, &tab.instance)
            && self.activity == tab.content_activity && self.revision == tab.buffer.raw_revision
            && self.language == tab.buffer.language && source.rope.is_instance(&tab.buffer.rope)
            && Arc::ptr_eq(&source.styles, &tab.buffer.highlights)
            && self.selection == (tab.selection.anchor, tab.selection.head, tab.selection.is_dragging)
            && self.block == block(w) && self.epoch == w.source_input_token() && self.input == window.input_context_token()
    }
}

struct Cached { stamp: Stamp, result: Result<Option<Pair>, &'static str> }

struct Pending {
    id: Uuid,
    stamp: Stamp,
    cancel: Arc<AtomicBool>,
    navigation: Option<Navigation>,
    playback: Option<Uuid>,
}

impl Drop for Pending {
    fn drop(&mut self) { self.cancel.store(true, Ordering::Relaxed); }
}

#[derive(Default)]
pub(crate) struct State {
    cache: Option<Cached>,
    pending: Option<Pending>,
    #[cfg(test)] pub(super) publish_gate: Option<futures::channel::oneshot::Receiver<()>>,
    #[cfg(test)] pub(super) workers_started: usize,
}

impl Workspace {
    pub(crate) fn brace_navigation_pending(&self) -> bool {
        self.brace_navigation.pending.as_ref().is_some_and(|pending| pending.navigation.is_some())
    }

    pub(crate) fn cached_brace_pair(&self, window: &Window) -> Option<(usize, usize)> {
        let cache = self.brace_navigation.cache.as_ref()?;
        if !cache.stamp.valid(self, window) { return None; }
        cache.result.ok().flatten().map(Pair::ordered)
    }

    pub(crate) fn cancel_brace_navigation(&mut self) -> bool {
        let Some(pending) = self.brace_navigation.pending.take() else { return false; };
        let navigation = pending.navigation.is_some();
        if navigation {
            if pending.playback.is_some() && pending.playback == self.active_macro_playback_id() {
                self.stop_prepared_macro("brace navigation cancelled");
            } else if self.status_message.as_deref() == Some(PREPARING) {
                self.set_status("Brace navigation cancelled");
            }
        }
        drop(pending);
        navigation
    }

    pub(crate) fn refresh_brace_match(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_source_input_tab();
        if self.brace_navigation.pending.as_ref().is_some_and(|pending| !pending.stamp.valid(self, window)) {
            self.cancel_brace_navigation();
        }
        if self.brace_navigation.cache.as_ref().is_some_and(|cache| !cache.stamp.valid(self, window)) {
            self.brace_navigation.cache = None;
        }
        if self.brace_navigation.cache.is_some() || self.brace_navigation.pending.is_some() { return; }
        self.start_brace_match(None, window, cx);
    }

    fn start_brace_match(&mut self, navigation: Option<Navigation>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(stamp) = Stamp::capture(self, window, cx) else { return; };
        let Some(source) = &stamp.source else { return; };
        let head = stamp.selection.1;
        if brace_match::origin(&source.rope, head).is_none() {
            self.brace_navigation.cache = Some(Cached { stamp, result: Ok(None) });
            return;
        }
        let rope = source.rope.clone();
        let styles = source.styles.clone();
        let id = Uuid::new_v4();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let playback = navigation.and_then(|_| self.active_macro_playback_id());
        #[cfg(test)] let publish_gate = self.brace_navigation.publish_gate.take();
        #[cfg(test)] { self.brace_navigation.workers_started = self.brace_navigation.workers_started.saturating_add(1); }
        let worker = cx.background_spawn(async move { brace_match::locate(&rope, head, &styles, &worker_cancel) });
        self.brace_navigation.pending = Some(Pending { id, stamp, cancel, navigation, playback });
        cx.spawn_in(window, async move |this, cx| {
            let result = worker.await;
            #[cfg(test)] if let Some(gate) = publish_gate { let _ = gate.await; }
            let updated = this.update_in(cx, |w, window, cx| w.finish_brace_match(id, result, window, cx));
            if updated.is_err() {
                let _ = this.update(cx, |w, _| {
                    if w.brace_navigation.pending.as_ref().is_some_and(|pending| pending.id == id) { w.cancel_brace_navigation(); }
                });
            }
        }).detach();
    }

    fn finish_brace_match(&mut self, id: Uuid, result: Result<Option<Pair>, &'static str>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.brace_navigation.pending.as_ref().is_some_and(|pending| pending.id == id) { return; }
        let Some(mut pending) = self.brace_navigation.pending.take() else { return; };
        let valid = pending.stamp.valid(self, window)
            && (pending.navigation.is_none() || available(self, window, true))
            && pending.playback.is_none_or(|id| self.regex_replacement.macro_playback.as_ref().is_some_and(|playback| playback.id == id && playback.valid(self, window)));
        if !valid {
            if pending.playback.is_some() && pending.playback == self.active_macro_playback_id() { self.stop_prepared_macro("the editor changed before brace navigation"); }
            else if pending.navigation.is_some() && self.status_message.as_deref() == Some(PREPARING) { self.set_status("Brace navigation cancelled because the editor changed"); }
            cx.notify(); return;
        }
        let navigation = pending.navigation;
        let playback = pending.playback;
        // Move the stamp without dropping its worker-owned source on the UI thread.
        let Some(source) = pending.stamp.source.take() else { return; };
        let stamp = Stamp { tab: pending.stamp.tab, instance: pending.stamp.instance.clone(), activity: pending.stamp.activity.clone(),
            revision: pending.stamp.revision.clone(), language: pending.stamp.language, selection: pending.stamp.selection,
            block: pending.stamp.block, epoch: pending.stamp.epoch.clone(), input: pending.stamp.input.clone(),
            source: Some(source), executor: pending.stamp.executor.clone() };
        self.brace_navigation.cache = Some(Cached { stamp, result });
        if let Some(navigation) = navigation {
            if self.apply_brace_navigation(navigation, result, cx) { self.resume_prepared_macro(playback, window, cx); }
            else if playback.is_some() && playback == self.active_macro_playback_id() { self.stop_prepared_macro("brace matching failed"); }
        }
        cx.notify();
    }

    fn apply_brace_navigation(&mut self, navigation: Navigation, result: Result<Option<Pair>, &'static str>, cx: &mut Context<Self>) -> bool {
        let pair = match result {
            Ok(pair) => pair,
            Err(message) => { self.set_status(message); cx.notify(); return false; }
        };
        if self.status_message.as_deref() == Some(PREPARING) { self.status_message = None; }
        let Some(pair) = pair else { return true; };
        match navigation {
            Navigation::GoTo => self.move_to_byte(pair.partner, false, true),
            Navigation::Select => {
                let (left, right) = pair.ordered();
                let Some(end) = right.checked_add(1).filter(|&end| end <= self.buffer().rope.len_bytes()) else { return false; };
                self.clear_block_selection();
                let tab = self.active_tab_mut();
                tab.selection.anchor = left; tab.selection.head = end; tab.selection.is_dragging = false;
                tab.desired_col_x = None;
                self.sync_source_input_caret();
                self.caret_visible = true;
            }
        }
        self.close_autocomplete(cx); self.close_function_hint(cx);
        self.ensure_cursor_visible(cx);
        cx.notify(); true
    }

    pub(super) fn run_brace_navigation(&mut self, navigation: Navigation, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !available(self, window, true) {
            self.set_status("Focus Source or Raw View and finish loading or composition before navigating braces");
            cx.notify(); return false;
        }
        if self.active_macro_playback_id().is_some() && !macros::is_replaying() {
            self.set_status("Finish or cancel the running macro before navigating braces");
            cx.notify(); return false;
        }
        self.sync_source_input_tab();
        macros::record(match navigation { Navigation::GoTo => MacroEvent::GoToMatchingBrace, Navigation::Select => MacroEvent::SelectInBetween });
        if let Some(result) = self.brace_navigation.cache.as_ref().filter(|cache| cache.stamp.valid(self, window)).map(|cache| cache.result) {
            return self.apply_brace_navigation(navigation, result, cx);
        }
        let playback = self.active_macro_playback_id();
        if self.brace_navigation.pending.as_ref().is_some_and(|pending| pending.stamp.valid(self, window) && pending.navigation.is_none()) {
            if let Some(pending) = self.brace_navigation.pending.as_mut() { pending.navigation = Some(navigation); pending.playback = playback; }
        } else {
            self.cancel_brace_navigation();
            self.brace_navigation.cache = None;
            self.start_brace_match(Some(navigation), window, cx);
        }
        if self.brace_navigation_pending() { self.set_status(PREPARING); cx.notify(); }
        true
    }

    pub(crate) fn go_to_matching_brace(&mut self, _: &SearchGoToMatchingBrace, window: &mut Window, cx: &mut Context<Self>) {
        self.run_brace_navigation(Navigation::GoTo, window, cx);
    }

    pub(crate) fn select_in_between(&mut self, _: &SearchSelectInBetween, window: &mut Window, cx: &mut Context<Self>) {
        self.run_brace_navigation(Navigation::Select, window, cx);
    }
}

#[cfg(test)]
#[path = "brace_navigation_tests.rs"]
mod tests;
