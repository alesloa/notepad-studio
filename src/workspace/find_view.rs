// PowerEditor/src/ScintillaComponent/FindReplaceDlg.cpp:2337-2347 — current-document selection scope.
// Source View searches raw bytes; Doc View searches rendered paragraph/run text.
use super::Workspace;
use crate::{find_state::FindScopeOwner, tab::{Tab, TabViewMode}};

pub(super) fn uses_rich_text(tab: &Tab) -> bool {
    tab.view_mode == TabViewMode::RichDocView && tab.buffer.rich_doc.is_some()
}
impl Workspace {
    pub(super) fn capture_find_scope_owner(&mut self) {
        self.find_state.in_selection_scope_owner = self.tabs.get(self.active).map(|tab| FindScopeOwner {
            tab: tab.session_id, instance: tab.instance.clone(), view: tab.view_mode,
        });
    }
    pub(super) fn retire_find_scope_if_view_changed(&mut self) {
        let stale = self.find_state.in_selection_scope_owner.as_ref().is_some_and(|owner| {
            self.tabs.get(self.active).is_none_or(|tab| owner.tab != tab.session_id
                || !std::sync::Arc::ptr_eq(&owner.instance, &tab.instance) || owner.view != tab.view_mode)
        });
        if stale { self.retire_find_view_state(); }
    }
    pub(super) fn retire_find_view_state(&mut self) {
        self.find_state.in_selection = false;
        self.find_state.in_selection_scope = None;
        self.find_state.in_selection_scope_owner = None;
        self.find_state.matches.clear();
        self.find_state.rich_doc_block_offsets = None;
        self.find_state.current = None;
    }
}
