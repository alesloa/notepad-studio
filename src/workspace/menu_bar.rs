// Workspace side of the in-window menu bar (`crate::ui::menu_bar`): creates
// the bar on first render and forwards the Alt / F10 / Alt+<key> actions.

use gpui::{AppContext, Context, Entity, Focusable, Window};

use crate::actions::{MenuBarActivate, MenuBarAltTap, MenuBarOpen};
use crate::theme::Theme;
use crate::ui::menu_bar::MenuBar;
use crate::workspace::Workspace;

impl Workspace {
    /// This window's menu bar, with the current theme. `None` on macOS,
    /// where gpui installs the menus natively.
    pub(crate) fn menu_bar_view(
        &mut self,
        theme: Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<MenuBar>> {
        if cfg!(target_os = "macos") {
            return None;
        }
        let bar = match &self.menu_bar {
            Some(bar) => bar.clone(),
            None => {
                let focus = self.focus_handle(cx);
                let bar = cx.new(|cx| MenuBar::new(focus, theme, window, cx));
                self.menu_bar = Some(bar.clone());
                bar
            }
        };
        bar.update(cx, |bar, _| bar.theme = theme);
        Some(bar)
    }

    pub(crate) fn menu_bar_alt_tap(
        &mut self,
        _: &MenuBarAltTap,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(bar) = self.menu_bar.clone() {
            bar.update(cx, |bar, cx| bar.alt_tap(cx));
        }
    }

    pub(crate) fn menu_bar_activate(
        &mut self,
        _: &MenuBarActivate,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(bar) = self.menu_bar.clone() {
            bar.update(cx, |bar, cx| bar.toggle(cx));
        }
    }

    pub(crate) fn menu_bar_open(
        &mut self,
        action: &MenuBarOpen,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(bar) = self.menu_bar.clone() {
            bar.update(cx, |bar, cx| bar.open_by_key(action.key, cx));
        }
    }
}
