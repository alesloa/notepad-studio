// Publishes this window's checkable state for the menus
// (`crate::app::menus::state` explains why the menus can't read it here).

use gpui::Context;

use super::Workspace;
use crate::app::menus::MenuState;

impl Workspace {
    /// Hand the menus the active document's checkable state. The in-window
    /// menu bar rebuilds on every open; macOS's native bar only changes on
    /// `set_menus`, so a real change rebuilds it.
    pub(crate) fn publish_menu_state(&self, cx: &mut Context<Self>) {
        let buffer = self.buffer();
        let changed = crate::app::menus::publish_menu_state(MenuState {
            has_document: true,
            word_wrap: self.word_wrap,
            show_whitespace: self.show_whitespace,
            overtype: self.overtype,
            encoding: Some(buffer.encoding),
            line_ending: Some(buffer.line_ending),
            language: buffer.language,
            tab_color: buffer.tab_color,
        });
        if changed && cfg!(target_os = "macos") {
            cx.defer(crate::app::menus::rebuild_menus);
        }
    }
}
