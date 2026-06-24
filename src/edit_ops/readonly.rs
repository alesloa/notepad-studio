// Read-Only flag — Edit menu entries for the active tab. Wraps the existing
// per-tab plumbing in `workspace::tabs::set_tab_read_only` /
// `clear_tab_read_only_flag` (which the tab right-click menu also drives) so
// the Edit menu and the tab right-click menu share state.
//
// Notepad++ source:
//   PowerEditor/src/NppCommands.cpp:2230 — IDM_EDIT_TOGGLEREADONLY
//   menuCmdID.h:188                       — IDM_EDIT_CLEARREADONLYFORALLDOCS
//
// Both variants hit the active tab only — NPP's "for all docs" sweep is a
// separate menu item and not in scope for this fork.

use gpui::{Context, Window};

use crate::actions::*;
use crate::workspace::Workspace;

impl Workspace {
    /// Edit ▸ Set Read-Only — flips the active tab's read-only flag.
    pub(crate) fn edit_toggle_read_only(
        &mut self,
        _: &EditToggleReadOnly,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let idx = self.active;
        let cur = self
            .tabs
            .get(idx)
            .map(|t| t.buffer.read_only)
            .unwrap_or(false);
        self.set_tab_read_only(idx, !cur, cx);
    }

    /// Edit ▸ Clear Read-Only Flag — drops the in-memory flag AND chmods the
    /// underlying file writable so a follow-up save doesn't trip OS
    /// permissions. No-op for untitled buffers (no path to chmod).
    pub(crate) fn edit_clear_read_only_flag(
        &mut self,
        _: &EditClearReadOnlyFlag,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let idx = self.active;
        self.clear_tab_read_only_flag(idx, cx);
    }
}
