// PowerEditor/src/WinControls/StaticDialog/RunDlg/RunDlg.rc:31-68 — Run
// dialog grouping, labels, history input and variable picker.

mod input;
mod saved;

use std::cell::RefCell;
use std::ops::Range;

use gpui::{
    App, AppContext, Bounds, Context, EntityInputHandler, FocusHandle, Focusable, IntoElement,
    KeyDownEvent, Pixels, Render, SharedString, TitlebarOptions, UTF16Selection, WeakEntity,
    Window, WindowAppearance, WindowHandle, WindowKind, WindowOptions, div, prelude::*, px, rems,
    rgb, size,
};

use crate::run_command::{self, Platform, RunContext, RunHistory};
use crate::text::utf16::{
    input_selection_range_utf16, slice_str_chars_utf16, str_byte_to_utf16, str_utf16_to_byte,
};
use crate::theme::{Theme, ThemeMode};
use crate::workspace::Workspace;
use input::{CommandInput, InputElement};

thread_local! {
    static OPEN_WINDOW: RefCell<Option<WindowHandle<RunDialog>>> = const { RefCell::new(None) };
}

pub(crate) struct RunDialog {
    workspace: WeakEntity<Workspace>,
    mode: ThemeMode,
    appearance: WindowAppearance,
    focus: FocusHandle,
    buttons: [FocusHandle; 8],
    input: CommandInput,
    save_name: Option<CommandInput>,
    editing_id: Option<String>,
    history: RunHistory,
    history_index: Option<usize>,
    history_open: bool,
    variables_open: bool,
    history_loaded: bool,
    running: bool,
    saving: bool,
    status: String,
}

impl Focusable for RunDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl RunDialog {
    fn raise_for(
        &mut self,
        workspace: WeakEntity<Workspace>,
        mode: ThemeMode,
        appearance: WindowAppearance,
        zoom: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace = workspace;
        self.mode = mode;
        self.appearance = appearance;
        window.set_rem_size(px(crate::consts::ZOOM_REM_BASE * zoom));
        window.activate_window();
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn undo_input(&mut self, _: &crate::actions::EditUndo, _: &mut Window, cx: &mut Context<Self>) {
        if !self.saving {
            self.active_input_mut().undo();
            cx.notify();
        }
    }

    fn redo_input(&mut self, _: &crate::actions::EditRedo, _: &mut Window, cx: &mut Context<Self>) {
        if !self.saving {
            self.active_input_mut().redo();
            cx.notify();
        }
    }

    fn active_input(&self) -> &CommandInput {
        self.save_name.as_ref().unwrap_or(&self.input)
    }

    fn active_input_mut(&mut self) -> &mut CommandInput {
        match self.save_name.as_mut() {
            Some(name) => name,
            None => &mut self.input,
        }
    }

    fn theme(&self) -> Theme {
        let settings = crate::settings::snapshot();
        Theme::resolve(
            self.mode,
            self.appearance,
            settings.tone_palette,
            &settings.customized_slots_array(),
        )
    }

    fn load_history(&mut self, cx: &mut Context<Self>) {
        let task = cx.background_spawn(async {
            run_command::history_path()
                .map(|path| RunHistory::load(&path))
                .unwrap_or_else(|| Ok(RunHistory::default()))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |dialog, cx| {
                dialog.history_loaded = true;
                match result {
                    Ok(history) => {
                        if !dialog.input.touched {
                            if let Some(command) = history.commands.first() {
                                if let Err(err) = dialog.input.set(command.clone()) {
                                    dialog.status = err;
                                }
                            }
                        }
                        dialog.history = history;
                        if dialog.status == "Loading recent commands…" {
                            dialog.status.clear();
                        }
                    }
                    Err(err) => dialog.status = err,
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.running || self.saving || self.save_name.is_some() || !self.history_loaded {
            return;
        }
        let context = self
            .workspace
            .upgrade()
            .ok_or_else(|| "The editor window was closed.".to_string())
            .and_then(|workspace| {
                let workspace = workspace.read(cx);
                let tab = workspace
                    .tabs
                    .get(workspace.active)
                    .ok_or_else(|| "Open a document before running a command.".to_string())?;
                RunContext::from_buffer(&tab.buffer, tab.selection)
            });
        let context = match context {
            Ok(context) => context,
            Err(err) => {
                self.status = err;
                cx.notify();
                return;
            }
        };
        let source = self.input.text.clone();
        let workspace = self.workspace.clone();
        let handle = window.window_handle();
        self.running = true;
        self.status = "Launching…".into();
        let task = cx.background_spawn(async move {
            let prepared = run_command::prepare(&source, &context, Platform::current(), |name| {
                std::env::var(name).ok()
            })?;
            run_command::launch(prepared)?;
            let warning = run_command::update_config(|history| {
                history.record(&source);
                Ok(())
            })
            .err();
            Ok::<_, String>(warning)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            match result {
                Ok(warning) => {
                    let _ = workspace.update(cx, |workspace, cx| {
                        workspace.set_status(match warning {
                            Some(err) => {
                                format!("Command launched; Run history could not be saved: {err}")
                            }
                            None => "Command launched.".into(),
                        });
                        cx.notify();
                    });
                    let _ = cx.update_window(handle, |_, window, _| window.remove_window());
                }
                Err(err) => {
                    let _ = this.update(cx, |dialog, cx| {
                        dialog.running = false;
                        dialog.status = err;
                        cx.notify();
                    });
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn browse(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let file = rfd::AsyncFileDialog::new()
                .set_title("The Program to Run")
                .pick_file()
                .await;
            if let Some(file) = file {
                let _ = this.update(cx, |dialog, cx| {
                    match file.path().to_str() {
                        Some(path) => {
                            if let Err(err) = dialog
                                .input
                                .set(run_command::quote_path(path, Platform::current()))
                            {
                                dialog.status = err;
                            }
                            dialog.input.touched = true;
                        }
                        None => {
                            dialog.status =
                                "This program path cannot be represented in the command input."
                                    .into()
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn button(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        if self.save_name.is_some() {
            match index {
                4 => self.save_named(window, cx),
                5 => self.cancel_save(window, cx),
                6 => self.delete_named(window, cx),
                7 => {
                    self.editing_id = None;
                    self.status = "Choose a new name for this copy.".into();
                    window.focus(&self.focus, cx);
                }
                _ => return,
            }
            cx.notify();
            return;
        }
        match index {
            0 => {
                self.history_open = !self.history_open;
                self.variables_open = false;
            }
            1 => self.browse(cx),
            2 => {
                self.variables_open = !self.variables_open;
                self.history_open = false;
            }
            3 => self.run(window, cx),
            4 => self.begin_save(window, cx),
            5 => window.remove_window(),
            _ => return,
        }
        cx.notify();
    }

    fn choose_history(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(command) = self.history.commands.get(index) {
            if let Err(err) = self.input.set(command.clone()) {
                self.status = err;
                cx.notify();
                return;
            }
            self.input.touched = true;
            self.editing_id = None;
            self.history_index = Some(index);
            self.history_open = false;
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let mods = event.keystroke.modifiers;
        if self.saving {
            cx.stop_propagation();
            return;
        }
        let save_open = self.save_name.is_some();
        if save_open && mods.alt && key == "n" {
            window.focus(&self.focus, cx);
            cx.stop_propagation();
            return;
        }
        if key == "escape" {
            if save_open {
                self.cancel_save(window, cx);
            } else {
                window.remove_window();
            }
            cx.stop_propagation();
            return;
        }
        if key == "tab" {
            let indices: &[usize] = if save_open {
                if self.editing_id.is_some() {
                    &[4, 5, 6, 7]
                } else {
                    &[4, 5]
                }
            } else {
                &[0, 1, 2, 3, 4, 5]
            };
            let current = indices
                .iter()
                .position(|index| {
                    self.buttons
                        .get(*index)
                        .is_some_and(|focus| focus.is_focused(window))
                })
                .map(|i| i + 1)
                .unwrap_or(0);
            let count = indices.len() + 1;
            let next = if mods.shift {
                (current + count - 1) % count
            } else {
                (current + 1) % count
            };
            let focus = if next == 0 {
                &self.focus
            } else {
                indices
                    .get(next - 1)
                    .and_then(|index| self.buttons.get(*index))
                    .unwrap_or(&self.focus)
            };
            window.focus(focus, cx);
        } else if key == "enter" {
            if self.active_input().marked.is_some() {
                return;
            }
            let button = self
                .buttons
                .iter()
                .position(|focus| focus.is_focused(window))
                .unwrap_or(if save_open { 4 } else { 3 });
            self.button(button, window, cx);
        } else if mods.alt && matches!(key, "r" | "s" | "c" | "p" | "+") {
            match key {
                "r" => self.run(window, cx),
                "s" => self.button(4, window, cx),
                "c" => {
                    if save_open {
                        self.cancel_save(window, cx);
                    } else {
                        window.remove_window();
                    }
                }
                "p" => window.focus(&self.focus, cx),
                "+" => self.button(2, window, cx),
                _ => {}
            }
        } else if self.focus.is_focused(window) {
            match key {
                "up" | "down" if !save_open && !self.history.commands.is_empty() => {
                    let last = self.history.commands.len().saturating_sub(1);
                    let next = match (key, self.history_index) {
                        (_, None) => 0,
                        ("up", Some(index)) => index.saturating_sub(1),
                        (_, Some(index)) => index.saturating_add(1).min(last),
                    };
                    self.choose_history(next, window, cx);
                }
                _ => match self.active_input_mut().edit_key(event, cx) {
                    Ok(true) => {}
                    Ok(false) => return,
                    Err(err) => self.status = err,
                },
            }
        } else {
            return;
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for RunDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.appearance = window.appearance();
        let theme = self.theme();
        if self.save_name.is_some() {
            return self.render_saved_prompt(theme, cx).into_any_element();
        }
        let button = |index: usize, label: &'static str, cx: &mut Context<Self>| {
            let focus = self.buttons.get(index).unwrap_or(&self.focus).clone();
            div()
                .id(("run-button", index))
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
        let mut root = div()
            .id("run-dialog")
            .key_context("RunDialog")
            .track_focus(&self.focus)
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
            .on_action(cx.listener(Self::undo_input))
            .on_action(cx.listener(Self::redo_input))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(0.5))
                    .p(rems(0.6))
                    .border_1()
                    .border_color(rgb(theme.group_box_border))
                    .child("The Program to Run")
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(0.25))
                            .child(
                                div()
                                    .id("run-command-input")
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
                            )
                            .child(button(0, "▾", cx))
                            .child(button(1, "...", cx))
                            .child(button(2, "+", cx)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .justify_center()
                    .gap(rems(0.4))
                    .child(button(3, "Run", cx))
                    .child(button(4, "Save...", cx))
                    .child(button(5, "Cancel", cx)),
            );
        if self.history_open {
            let mut list = div()
                .id("run-history")
                .max_h(rems(8.))
                .overflow_y_scroll()
                .border_1()
                .border_color(rgb(theme.border));
            for (index, command) in self.history.commands.iter().enumerate() {
                list = list.child(
                    div()
                        .id(("run-history-item", index))
                        .p(rems(0.3))
                        .hover(|s| s.bg(rgb(theme.button_hover_bg)))
                        .child(SharedString::from(command.clone()))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.choose_history(index, window, cx)
                        })),
                );
            }
            if self.history.commands.is_empty() {
                list = list.child("No recent commands");
            }
            root = root.child(list);
        }
        if self.variables_open {
            let mut list = div()
                .id("run-variables")
                .max_h(rems(9.))
                .overflow_y_scroll()
                .border_1()
                .border_color(rgb(theme.border));
            for (index, &name) in run_command::VARIABLES.iter().enumerate() {
                list = list.child(
                    div()
                        .id(("run-variable", index))
                        .p(rems(0.25))
                        .hover(|s| s.bg(rgb(theme.button_hover_bg)))
                        .child(name)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let Err(err) = this.input.replace(None, &format!("$({name})")) {
                                this.status = err;
                            }
                            this.variables_open = false;
                            window.focus(&this.focus, cx);
                            cx.notify();
                        })),
                );
            }
            root = root.child(list);
        }
        if !self.history.saved.is_empty() {
            root = root.child(self.render_saved_list(theme, cx));
        }
        root.child(
            div()
                .text_color(rgb(theme.disabled_text))
                .child(SharedString::from(self.status.clone())),
        )
        .into_any_element()
    }
}

impl EntityInputHandler for RunDialog {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        Some(slice_str_chars_utf16(
            &self.active_input().text,
            range,
            actual,
        ))
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let input = self.active_input();
        Some(input_selection_range_utf16(
            &input.text,
            input.cursor,
            input.anchor,
        ))
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.active_input().marked.clone()
    }
    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.active_input_mut().marked = None;
    }
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.saving {
            return;
        }
        if let Err(err) = self.active_input_mut().replace(range, text) {
            self.status = err;
        }
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.saving {
            return;
        }
        if let Err(err) = self.active_input_mut().replace_composition(range, text) {
            self.status = err;
            cx.notify();
            return;
        }
        let input = self.active_input_mut();
        let end = str_byte_to_utf16(&input.text, input.cursor);
        let length = text
            .chars()
            .filter(|c| !matches!(c, '\r' | '\n' | '\0'))
            .map(char::len_utf16)
            .sum::<usize>();
        let start = end.saturating_sub(length);
        input.marked = Some(start..end);
        if let Some(selected) = selected {
            input.anchor =
                str_utf16_to_byte(&input.text, start.saturating_add(selected.start).min(end));
            input.cursor =
                str_utf16_to_byte(&input.text, start.saturating_add(selected.end).min(end));
        }
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(bounds)
    }
    fn character_index_for_point(
        &mut self,
        _: gpui::Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

pub(crate) fn open_or_focus(
    workspace: WeakEntity<Workspace>,
    mode: ThemeMode,
    appearance: WindowAppearance,
    zoom: f32,
    cx: &mut App,
) -> Result<(), String> {
    if let Some(handle) = OPEN_WINDOW.with(|cell| *cell.borrow()) {
        if handle
            .update(cx, |dialog, window, cx| {
                dialog.raise_for(workspace.clone(), mode, appearance, zoom, window, cx)
            })
            .is_ok()
        {
            return Ok(());
        }
    }
    let bounds =
        crate::app::window::modal_bounds_centered_on_workspace(cx, size(px(680.), px(340.)));
    let handle = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(bounds),
                kind: WindowKind::Floating,
                is_resizable: true,
                window_min_size: Some(size(px(480.), px(220.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Run...".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                window.set_rem_size(px(crate::consts::ZOOM_REM_BASE * zoom));
                cx.new(|cx| {
                    let focus = cx.focus_handle();
                    window.focus(&focus, cx);
                    let mut dialog = RunDialog {
                        workspace,
                        mode,
                        appearance,
                        focus,
                        buttons: std::array::from_fn(|_| cx.focus_handle()),
                        input: CommandInput::default(),
                        save_name: None,
                        editing_id: None,
                        history: RunHistory::default(),
                        history_index: None,
                        history_open: false,
                        variables_open: false,
                        history_loaded: false,
                        running: false,
                        saving: false,
                        status: "Loading recent commands…".into(),
                    };
                    dialog.load_history(cx);
                    dialog
                })
            },
        )
        .map_err(|err| format!("Could not open Run dialog: {err}"))?;
    OPEN_WINDOW.with(|cell| *cell.borrow_mut() = Some(handle));
    if let Ok(entity) = handle.entity(cx) {
        cx.observe_release(&entity, move |_, _| {
            OPEN_WINDOW.with(|cell| *cell.borrow_mut() = None)
        })
        .detach();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[test]
    fn native_save_prompt_undo_keys_and_actions_never_change_editor_history() {
        let mut cx = TestAppContext::single();
        let workspace = cx.new(|cx| {
            let mut workspace = Workspace::new(
                cx.focus_handle(),
                cx.focus_handle(),
                cx.focus_handle(),
                cx.focus_handle(),
                cx.focus_handle(),
                cx.focus_handle(),
                1.,
                WindowAppearance::Dark,
            );
            workspace.active_tab_mut().buffer.rope =
                ropey::Rope::from_str("editor document stays unchanged");
            workspace
                .active_tab_mut()
                .undo_stack
                .push(crate::buffer::UndoEntry::Single(crate::buffer::EditOp {
                    byte_start: 0,
                    inserted: "editor".into(),
                    removed: "previous".into(),
                    sel_before_anchor: 0,
                    sel_before_head: 0,
                    before_revision: Default::default(),
                    after_revision: Default::default(),
                }));
            workspace.window_closed = true;
            workspace
        });
        let other_workspace = cx.new(|cx| {
            let mut workspace = Workspace::new(
                cx.focus_handle(),
                cx.focus_handle(),
                cx.focus_handle(),
                cx.focus_handle(),
                cx.focus_handle(),
                cx.focus_handle(),
                1.,
                WindowAppearance::Dark,
            );
            workspace.active_tab_mut().buffer.rope =
                ropey::Rope::from_str("other editor stays unchanged");
            workspace.window_closed = true;
            workspace
        });
        let (dialog, visual) = cx.add_window_view(|window, cx| {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            let mut input = CommandInput::default();
            input.set("tool".into()).unwrap();
            input.move_to(input.text.len(), false);
            input.replace(None, " arg").unwrap();
            RunDialog {
                workspace: workspace.downgrade(),
                mode: ThemeMode::Dark,
                appearance: WindowAppearance::Dark,
                focus,
                buttons: std::array::from_fn(|_| cx.focus_handle()),
                input,
                save_name: None,
                editing_id: None,
                history: RunHistory::default(),
                history_index: None,
                history_open: false,
                variables_open: false,
                history_loaded: true,
                running: false,
                saving: false,
                status: String::new(),
            }
        });
        visual.simulate_keystrokes("alt-s");
        visual.simulate_input("café 中");
        visual.update(|_, cx| {
            assert_eq!(dialog.read(cx).active_input().text, "café 中");
            assert_eq!(dialog.read(cx).input.text, "tool arg");
        });
        visual.dispatch_action(crate::actions::EditUndo);
        visual.update(|_, cx| assert_eq!(dialog.read(cx).active_input().text, "café "));
        visual.dispatch_action(crate::actions::EditRedo);
        visual.simulate_keystrokes("ctrl-z");
        visual.update(|_, cx| assert_eq!(dialog.read(cx).active_input().text, "café "));
        visual.simulate_keystrokes("ctrl-y");
        visual.update(|_, cx| assert_eq!(dialog.read(cx).active_input().text, "café 中"));
        visual.simulate_keystrokes("tab");
        visual.update(|window, cx| assert!(dialog.read(cx).buttons[4].is_focused(window)));
        visual.update(|window, cx| {
            dialog.update(cx, |dialog, cx| {
                dialog.raise_for(
                    other_workspace.downgrade(),
                    ThemeMode::Light,
                    WindowAppearance::Light,
                    1.5,
                    window,
                    cx,
                );
                assert!(dialog.focus.is_focused(window));
                assert_eq!(window.rem_size(), px(crate::consts::ZOOM_REM_BASE * 1.5));
                assert_eq!(dialog.mode, ThemeMode::Light);
                assert_eq!(
                    dialog.workspace.upgrade().unwrap().entity_id(),
                    other_workspace.entity_id()
                );
                assert_eq!(dialog.active_input().text, "café 中");
            });
        });
        visual.simulate_keystrokes("tab shift-tab escape");
        visual.dispatch_action(crate::actions::EditUndo);
        visual.update(|_, cx| {
            assert!(dialog.read(cx).save_name.is_none());
            assert_eq!(dialog.read(cx).input.text, "tool");
            assert_eq!(
                workspace.read(cx).buffer().rope.to_string(),
                "editor document stays unchanged"
            );
            assert_eq!(workspace.read(cx).tabs[0].undo_stack.len(), 1);
        });
        visual.update(|_, cx| {
            assert_eq!(
                other_workspace.read(cx).buffer().rope.to_string(),
                "other editor stays unchanged"
            );
            assert!(other_workspace.read(cx).tabs[0].undo_stack.is_empty());
        });
        visual.simulate_keystrokes("cmd-shift-z");
        visual.update(|_, cx| {
            assert_eq!(dialog.read(cx).input.text, "tool arg");
            assert_eq!(
                workspace.read(cx).buffer().rope.to_string(),
                "editor document stays unchanged"
            );
            assert_eq!(workspace.read(cx).tabs[0].undo_stack.len(), 1);
        });
        cx.quit();
    }
}
