use gpui::{Context, Window};
use uuid::Uuid;
use super::{Workspace, MAX_CONTEXT_TABS, path_identity};

impl Workspace {
    pub(crate) fn begin_title_clipboard_wait(&mut self) -> Option<(Uuid, crate::buffer::ContentRevision, crate::buffer::ContentRevision, gpui::AnyWindowHandle)> {
        let playback = self.regex_replacement.macro_playback.as_ref()?;
        let tab = self.tabs.get(self.active)?;
        self.automatic_title.title_clipboard = Some(playback.id);
        Some((playback.id, tab.buffer.raw_revision.clone(), tab.content_activity.clone(), playback.window))
    }

    pub(crate) fn abandon_title_clipboard_wait(&mut self) { self.automatic_title.title_clipboard = None; }

    pub(crate) fn title_clipboard_pending(&self) -> bool { self.automatic_title.title_clipboard.is_some() }

    pub(crate) fn finish_title_clipboard_wait(&mut self,
        stamp: Option<(Uuid, crate::buffer::ContentRevision, crate::buffer::ContentRevision, gpui::AnyWindowHandle)>,
        cx: &mut Context<Self>, callback: impl FnOnce(&mut Self, &mut Context<Self>) -> bool + 'static)
    {
        let Some((id, revision, activity, window)) = stamp else { callback(self, cx); return; };
        let owner = cx.entity().entity_id();
        cx.spawn(async move |_, cx| {
            let _ = window.update(cx, |root, window, cx| {
                let Ok(entity) = root.downcast::<Workspace>() else { return; };
                if entity.entity_id() != owner { return; }
                entity.update(cx, |w, cx| {
                    if w.automatic_title.title_clipboard != Some(id) { return; }
                    w.automatic_title.title_clipboard = None;
                    let valid = w.regex_replacement.macro_playback.as_ref().is_some_and(|playback| playback.id == id && playback.valid(w, window))
                        && w.tabs.get(w.active).is_some_and(|tab| tab.buffer.raw_revision == revision && tab.content_activity == activity);
                    if !valid || !callback(w, cx) { w.stop_prepared_macro("the document or clipboard changed while waiting for automatic names"); cx.notify(); return; }
                    w.resume_prepared_macro(Some(id), window, cx);
                });
            });
        }).detach();
    }

    pub(crate) fn defer_automatic_title_tab(&mut self, index: usize, command: &'static str,
        window: &mut Window, cx: &mut Context<Self>, callback: impl FnOnce(&mut Self, usize, &mut Window, &mut Context<Self>) + 'static) -> bool
    {
        let Some(tab) = self.tabs.get(index) else { return false; };
        let id = tab.session_id; let instance = tab.instance.clone();
        let revision = tab.buffer.raw_revision.clone(); let activity = tab.content_activity.clone();
        let path = path_identity(tab); let custom = tab.buffer.new_label_is_custom;
        let key = (id, command); let request = Uuid::new_v4();
        // A newer intent supersedes an already deferred command, even if its
        // naming work has just finished and this invocation runs immediately.
        if let Some((_, previous)) = self.automatic_title.commands.iter_mut().find(|entry| entry.0 == key) { *previous = request; }
        let Some(ready) = self.wait_for_automatic_titles(cx) else {
            if let Some(index) = self.automatic_title.commands.iter().position(|entry| entry.0 == key) { self.automatic_title.commands.swap_remove(index); }
            return false;
        };
        if let Some((_, previous)) = self.automatic_title.commands.iter_mut().find(|entry| entry.0 == key) { *previous = request; }
        else {
            let count = self.automatic_title.commands.len();
            let bytes = if count == self.automatic_title.commands.capacity() { count.saturating_mul(2).saturating_add(1).saturating_mul(std::mem::size_of::<super::Command>()) } else { std::mem::size_of::<super::Command>() };
            if count >= super::MAX_COMMANDS || self.charge_automatic_title_admission(bytes).is_err() || self.automatic_title.commands.try_reserve_exact(1).is_err() {
                self.discard_automatic_titles(Some(super::LIMIT_STATUS), cx); return false;
            }
            self.automatic_title.commands.push((key, request));
        }
        let window_id = self.session_window_id; let native = window.window_handle();
        cx.spawn_in(window, async move |this, cx| {
            let _ = ready.await;
            let _ = this.update_in(cx, |w, window, cx| {
                let Some(command_index) = w.automatic_title.commands.iter().position(|entry| entry.0 == key && entry.1 == request) else { return; };
                w.automatic_title.commands.swap_remove(command_index);
                if w.window_closed || w.session_window_id != window_id || window.window_handle() != native { return; }
                let now = if w.tabs.get(index).is_some_and(|tab| tab.session_id == id && std::sync::Arc::ptr_eq(&tab.instance, &instance)) { Some(index) }
                    else { w.tabs.iter().take(MAX_CONTEXT_TABS).position(|tab| tab.session_id == id && std::sync::Arc::ptr_eq(&tab.instance, &instance)) };
                let Some(now) = now.filter(|&now| w.tabs.get(now).is_some_and(|tab| tab.buffer.raw_revision == revision && tab.content_activity == activity
                    && path_identity(tab) == path && tab.buffer.new_label_is_custom == custom)) else {
                    w.set_status("Document changed while waiting for its automatic title; repeat the command for the current document"); cx.notify(); return;
                };
                callback(w, now, window, cx);
            });
        }).detach(); true
    }

    pub(crate) fn run_after_automatic_titles(&mut self, cx: &mut Context<Self>, callback: impl FnOnce(&mut Self, &mut Context<Self>) + 'static) {
        let Some(ready) = self.wait_for_automatic_titles(cx) else { callback(self, cx); return; };
        let window = self.session_window_id;
        cx.spawn(async move |this, cx| {
            let _ = ready.await;
            let _ = this.update(cx, |w, cx| {
                if !w.window_closed && w.session_window_id == window { callback(w, cx); }
            });
        }).detach();
    }

    pub(crate) fn defer_automatic_title_active(&mut self, command: &'static str, window: &mut Window, cx: &mut Context<Self>,
        callback: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static) -> bool
    {
        self.defer_automatic_title_tab(self.active, command, window, cx, move |w, index, window, cx| {
            let previous = w.tabs.get(w.active).map(|tab| tab.session_id); w.active = index;
            callback(w, window, cx);
            if let Some(previous) = previous.and_then(|id| w.tabs.iter().take(MAX_CONTEXT_TABS).position(|tab| tab.session_id == id)) { w.active = previous; }
        })
    }

    pub(crate) fn defer_automatic_title_window(&mut self, window: &mut Window, cx: &mut Context<Self>,
        callback: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static) -> bool
    {
        let Some(ready) = self.wait_for_automatic_titles(cx) else { return false; };
        let owner = self.session_window_id; let native = window.window_handle();
        cx.spawn_in(window, async move |this, cx| {
            let _ = ready.await;
            let _ = this.update_in(cx, |w, window, cx| {
                if !w.window_closed && w.session_window_id == owner && window.window_handle() == native { callback(w, window, cx); }
            });
        }).detach(); true
    }
}
