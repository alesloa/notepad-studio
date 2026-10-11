// scintilla/src/Document.cxx:1464-1469,1508-1509 — read-only deletion/insertion.
// Doc View mutations share unsaved-state and recovery completion here.

use gpui::Context;

use super::Workspace;

impl Workspace {
    pub(super) fn rich_doc_edit_allowed(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(tab) = self.tabs.get(self.active) else {
            return false;
        };
        if !tab.buffer.read_only {
            return true;
        }
        self.set_status("Buffer is read-only");
        cx.notify();
        false
    }

    pub(super) fn after_rich_doc_edit(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        tab.buffer.dirty = true;
        self.schedule_autosave(cx);
        cx.notify();
    }
}
