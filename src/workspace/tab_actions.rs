// PowerEditor/src/NppCommands.cpp:1304-1412,3532-3537 — tab color,
// positional selection and tab reordering. Existing strip/context handlers
// hold the document identity and persistence behavior.

use gpui::{Context, Window};

use crate::actions::*;
use super::Workspace;

impl Workspace {
    pub(crate) fn first_tab(&mut self, _: &TabFirst, _w: &mut Window, cx: &mut Context<Self>) {
        self.set_active_tab(0, cx);
    }

    pub(crate) fn last_tab(&mut self, _: &TabLast, _w: &mut Window, cx: &mut Context<Self>) {
        if let Some(last) = self.tabs.len().checked_sub(1) {
            self.set_active_tab(last, cx);
        }
    }

    pub(crate) fn move_active_tab_to_start(&mut self, _: &TabMoveToStart, _w: &mut Window, cx: &mut Context<Self>) {
        self.move_tab_to_start(self.active, cx);
    }

    pub(crate) fn move_active_tab_to_end(&mut self, _: &TabMoveToEnd, _w: &mut Window, cx: &mut Context<Self>) {
        self.move_tab_to_end(self.active, cx);
    }

    pub(crate) fn move_active_tab_forward(&mut self, _: &TabMoveForward, _w: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.active.checked_add(1).filter(|target| *target < self.tabs.len()) else { return; };
        self.move_tab(self.active, target, cx);
    }

    pub(crate) fn move_active_tab_backward(&mut self, _: &TabMoveBackward, _w: &mut Window, cx: &mut Context<Self>) {
        if let Some(target) = self.active.checked_sub(1) {
            self.move_tab(self.active, target, cx);
        }
    }

    pub(crate) fn tab_color_1(&mut self, _: &TabColor1, _w: &mut Window, cx: &mut Context<Self>) {
        self.set_tab_color(self.active, Some(1), cx);
    }

    pub(crate) fn tab_color_2(&mut self, _: &TabColor2, _w: &mut Window, cx: &mut Context<Self>) {
        self.set_tab_color(self.active, Some(2), cx);
    }

    pub(crate) fn tab_color_3(&mut self, _: &TabColor3, _w: &mut Window, cx: &mut Context<Self>) {
        self.set_tab_color(self.active, Some(3), cx);
    }

    pub(crate) fn tab_color_4(&mut self, _: &TabColor4, _w: &mut Window, cx: &mut Context<Self>) {
        self.set_tab_color(self.active, Some(4), cx);
    }

    pub(crate) fn tab_color_5(&mut self, _: &TabColor5, _w: &mut Window, cx: &mut Context<Self>) {
        self.set_tab_color(self.active, Some(5), cx);
    }

    pub(crate) fn tab_clear_color(&mut self, _: &TabClearColor, _w: &mut Window, cx: &mut Context<Self>) {
        self.set_tab_color(self.active, None, cx);
    }
}

#[cfg(test)]
#[path = "tab_actions_tests.rs"]
mod tests;
