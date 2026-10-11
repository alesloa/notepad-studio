// PowerEditor/src/WinControls/StaticDialog/RunDlg/RunDlg.cpp:413-454 and
// WinControls/shortcut/shortcut.rc:31-38 — named Save prompt. Commands have
// no assigned keyboard shortcut until dynamic Shortcut Mapper support lands.
use gpui::{Context, IntoElement, SharedString, Window, div, prelude::*, rems, rgb};

use super::{
    RunDialog,
    input::{CommandInput, InputElement},
};
use crate::run_command;
use crate::theme::Theme;

impl RunDialog {
    pub(super) fn begin_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.running || !self.history_loaded {
            return;
        }
        match run_command::parse(&self.input.text, run_command::Platform::current()) {
            Ok(words) if words.first().is_some_and(|word| !word.is_empty()) => {}
            Ok(_) => {
                self.status = "Enter a program or document to run before saving.".into();
                return;
            }
            Err(err) => {
                self.status = err;
                return;
            }
        }
        let name = self
            .editing_id
            .as_deref()
            .and_then(|id| self.history.saved.iter().find(|saved| saved.id == id))
            .map(|saved| saved.name.clone())
            .unwrap_or_default();
        let mut input = CommandInput::for_name();
        if let Err(err) = input.set(name) {
            self.status = err;
            return;
        }
        self.save_name = Some(input);
        self.history_open = false;
        self.variables_open = false;
        self.status.clear();
        window.focus(&self.focus, cx);
    }

    pub(super) fn cancel_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_name = None;
        self.status.clear();
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn choose_saved(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(saved) = self.history.saved.get(index) else {
            return;
        };
        if let Err(err) = self.input.set(saved.command.clone()) {
            self.status = err;
        } else {
            self.editing_id = Some(saved.id.clone());
            self.input.touched = true;
            self.status = format!(
                "Selected {}. Use Save... to rename, update, copy or delete it.",
                saved.name
            );
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(super) fn save_named(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.save_name.as_ref().map(|input| input.text.clone()) else {
            return;
        };
        let source = self.input.text.clone();
        let id = self.editing_id.clone();
        let handle = window.window_handle();
        self.saving = true;
        self.status = "Saving command…".into();
        let task = cx.background_spawn(async move {
            run_command::update_config(|history| history.save_named(id.as_deref(), &name, &source))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            cx.update(crate::app::menus::rebuild_menus);
            let _ = cx.update_window(handle, |_, window, cx| {
                let _ = this.update(cx, |dialog, cx| {
                    dialog.saving = false;
                    match result {
                        Ok((saved, history)) => {
                            dialog.editing_id = Some(saved.id);
                            dialog.history = history;
                            dialog.save_name = None;
                            dialog.status = format!("Saved {} in the Run menu.", saved.name);
                            window.focus(&dialog.focus, cx);
                        }
                        Err(err) => dialog.status = err,
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    pub(super) fn delete_named(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.editing_id.clone() else {
            return;
        };
        let handle = window.window_handle();
        self.saving = true;
        self.status = "Deleting command…".into();
        let task = cx.background_spawn(async move {
            run_command::update_config(|history| history.remove_named(&id))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            cx.update(crate::app::menus::rebuild_menus);
            let _ = cx.update_window(handle, |_, window, cx| {
                let _ = this.update(cx, |dialog, cx| {
                    dialog.saving = false;
                    match result {
                        Ok(((), history)) => {
                            dialog.history = history;
                            dialog.editing_id = None;
                            dialog.save_name = None;
                            dialog.status = "Deleted saved command.".into();
                            window.focus(&dialog.focus, cx);
                        }
                        Err(err) => dialog.status = err,
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    pub(super) fn render_saved_list(
        &self,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut list = div()
            .id("run-saved-list")
            .flex()
            .flex_col()
            .gap(rems(0.15))
            .max_h(rems(8.))
            .overflow_y_scroll()
            .border_1()
            .border_color(rgb(theme.border))
            .child(
                div()
                    .p(rems(0.3))
                    .child("Saved commands — select to edit or remove"),
            );
        for (index, saved) in self.history.saved.iter().enumerate() {
            let selected = self.editing_id.as_deref() == Some(saved.id.as_str());
            list =
                list.child(
                    div()
                        .id(("run-saved-item", index))
                        .p(rems(0.3))
                        .when(selected, |row| row.bg(rgb(theme.button_hover_bg)))
                        .hover(|row| row.bg(rgb(theme.button_hover_bg)))
                        .cursor(gpui::CursorStyle::PointingHand)
                        .child(SharedString::from(saved.name.clone()))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.choose_saved(index, window, cx)
                        })),
                );
        }
        list
    }

    pub(super) fn render_saved_prompt(
        &self,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let button = |index: usize, label: &'static str, cx: &mut Context<Self>| {
            let focus = self.buttons.get(index).unwrap_or(&self.focus).clone();
            div()
                .id(("run-save-button", index))
                .track_focus(&focus)
                .px(rems(0.65))
                .py(rems(0.25))
                .border_1()
                .border_color(rgb(theme.border))
                .bg(rgb(theme.button_bg))
                .hover(|s| s.bg(rgb(theme.button_hover_bg)))
                .focus(|s| s.border_color(rgb(theme.text)))
                .cursor(gpui::CursorStyle::PointingHand)
                .child(label)
                .on_click(cx.listener(move |this, _, window, cx| this.button(index, window, cx)))
        };
        let mut buttons = div()
            .flex()
            .justify_center()
            .gap(rems(0.4))
            .child(button(4, "OK", cx))
            .child(button(5, "Cancel", cx));
        if self.editing_id.is_some() {
            buttons = buttons
                .child(button(6, "Delete", cx))
                .child(button(7, "Save a Copy", cx));
        }
        div()
            .id("run-save-prompt")
            .key_context("RunDialog")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::undo_input))
            .on_action(cx.listener(Self::redo_input))
            .size_full()
            .flex()
            .flex_col()
            .gap(rems(0.7))
            .p(rems(0.75))
            .overflow_y_scroll()
            .bg(rgb(theme.chrome))
            .text_color(rgb(theme.text))
            .text_size(rems(0.8))
            .on_key_down(cx.listener(Self::key_down))
            .child("Save Run Command")
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.4))
                    .child("Name:")
                    .child(
                        div()
                            .id("run-name-input")
                            .track_focus(&self.focus)
                            .flex_1()
                            .min_w_0()
                            .h(rems(1.8))
                            .px(rems(0.3))
                            .bg(rgb(theme.bg))
                            .border_1()
                            .border_color(rgb(theme.border))
                            .overflow_hidden()
                            .focus(|s| s.border_color(rgb(theme.text)))
                            .child(InputElement(cx.entity())),
                    ),
            )
            .child(
                div().text_color(rgb(theme.disabled_text)).child(
                    "No keyboard shortcut is assigned. Launch this command from the Run menu.",
                ),
            )
            .child(buttons)
            .child(
                div()
                    .text_color(rgb(theme.disabled_text))
                    .child(SharedString::from(self.status.clone())),
            )
    }
}
