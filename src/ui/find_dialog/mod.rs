// Find dialog — a separate top-level window with Find / Replace / Find in
// Files / Mark tabs. Notepad++'s find dialog moved over to GPUI.
//
// Lifecycle: the dialog window is recreated on every open. State (queries,
// toggles, results, marks) lives on `Workspace`; the dialog itself is a
// thin entity that holds a `WeakEntity<Workspace>` and renders the active
// tab. Closing the dialog drops the window; reopening rebuilds it from
// scratch with the same state. This keeps the cross-window plumbing tiny —
// no entity-alive-but-window-hidden gymnastics.
//
// Submodules:
//   * `find_tab` — query input, Find Next / Prev / Count / Find All in
//     Current Document, match-case / whole-word / wrap-around / regex /
//     extended toggles.
//   * `replace_tab` — find input + replace input, Replace / Replace All /
//     Find Next / Find Previous, same toggles.
//   * `fif_tab` — Find in Files: query, replace, filters, directory,
//     Follow current doc / sub-folders / hidden folders toggles, Find All /
//     Replace in Files. Wires to the existing engine in `find_in_files.rs`.
//   * `mark_tab` — query input, Mark All / Clear / Copy Marked Text,
//     Bookmark Line companion toggle, match-case / whole-word / regex.

mod close;
mod find_tab;
mod fif_tab;
mod find_in_project_tab;
mod mark_tab;
mod replace_tab;
mod widgets;

use gpui::{
    App, Context, CursorStyle, Entity, FocusHandle, Focusable, IntoElement, MouseButton,
    ParentElement, Render, Rgba, Styled, Subscription, TitlebarOptions, WeakEntity, Window,
    WindowBackgroundAppearance, WindowKind, WindowOptions, div, point,
    prelude::*, px, rems, rgb, size,
};

use crate::actions::{
    EditBackspace, EditCopy, EditCut, EditDelete, EditFindClose, EditFindNext, EditFindPrev,
    EditInsertNewline, EditPaste, EditSelectAll, FindInFilesClose, FindInFilesRun,
    FindInProjectsRun, MoveDocEnd, MoveDocStart, MoveLeft, MoveLineEnd, MoveLineStart, MoveRight,
    MoveWordLeft, MoveWordRight, SelectDocEnd, SelectDocStart, SelectLeft, SelectLineEnd,
    SelectLineStart, SelectRight, SelectWordLeft, SelectWordRight,
};
use crate::find_state::DialogTab;
use crate::theme::Theme;
use crate::workspace::Workspace;

/// Compose a `Rgba` from a packed `0xRRGGBB` theme colour plus an alpha
/// derived from the Find-in-Files transparency state (NPP's "Transparency"
/// group on the FIF tab) and the current focus. When transparency is off,
/// the dialog is always fully opaque. When transparency is on, the alpha
/// drops to `level/100` either always or only when the window is inactive,
/// matching NPP's `Always` vs `On losing focus` radio.
fn focus_aware_bg(
    rgb_color: u32,
    window_active: bool,
    enabled: bool,
    always: bool,
    level: u8,
) -> Rgba {
    let [_, r, g, b] = rgb_color.to_be_bytes().map(|c| (c as f32) / 255.0);
    let faded = (level.clamp(10, 100) as f32) / 100.0;
    let a = if !enabled {
        1.0
    } else if always {
        faded
    } else if window_active {
        1.0
    } else {
        faded
    };
    Rgba { r, g, b, a }
}

/// Root entity rendered into the Find dialog's window. Holds a weak handle
/// back to the editor's `Workspace` so click handlers and action listeners
/// can mutate shared state. The dialog itself owns no editor state — see
/// the module header for the rationale.
///
/// `window_active` mirrors the OS-level activation of the dialog window so
/// `render` can fade the background fill to `UNFOCUSED_ALPHA` when the user
/// clicks away. Activation is tracked via `cx.observe_window_activation` and
/// pushed through `cx.notify()` so the next render closure picks up the new
/// alpha.
pub(crate) struct FindDialog {
    pub(crate) workspace: WeakEntity<Workspace>,
    pub(crate) focus: FocusHandle,
    pub(crate) window_active: bool,
    _activation_subscription: Subscription,
}

impl FindDialog {
    pub(crate) fn new(
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial_active = window.is_window_active();
        let _activation_subscription =
            cx.observe_window_activation(window, |this, window, cx| {
                let now = window.is_window_active();
                if this.window_active != now {
                    this.window_active = now;
                    cx.notify();
                }
            });
        Self {
            workspace,
            focus: cx.focus_handle(),
            window_active: initial_active,
            _activation_subscription,
        }
    }
}

impl Focusable for FindDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Open (or focus) the Find dialog window targeting `target_tab`. Re-opens a
/// closed window if no live handle is cached; otherwise activates the
/// existing window and switches it to the requested tab.
pub(crate) fn open_or_focus_find_dialog(
    workspace: &mut Workspace,
    target_tab: DialogTab,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    workspace.ensure_dialog_focus_handles(cx);
    workspace.dialog_tab = target_tab;
    workspace.find_state.open =
        matches!(target_tab, DialogTab::Find | DialogTab::Replace);

    // Existing window? Activate + repaint.
    //
    // We defer the cross-window `handle.update` for the same reason the
    // first-open path defers `cx.open_window`: this function is called from
    // inside a `workspace.update` (action handler), so Workspace has been
    // leased out of the entity map. `handle.update` re-enters the same
    // Workspace via `ws.update(dialog_cx, ...)` to compute focus targets,
    // which trips gpui's double-lease guard → `panic_cannot_unwind` →
    // SIGABRT through the Cocoa `performKeyEquivalent` shim. `window.defer`
    // runs after the current effect cycle, so the lease is released.
    if let Some(handle) = workspace.find_dialog_window {
        let weak_ws = cx.entity().downgrade();
        window.defer(cx, move |_window, cx| {
            // If the underlying window vanished (user clicked the OS close
            // box and we missed the on-window-closed observer), `update`
            // returns Err — clear the cached handle and recreate.
            let result = handle.update(cx, |_dialog, dialog_window, dialog_cx| {
                dialog_window.activate_window();
                // Focus the newly-active tab's primary input on the *dialog*
                // window. `window.focus` writes the focused FocusId into the
                // window's own dispatch tree — focusing on the editor window
                // here would not deliver keystrokes to the dialog inputs.
                if let Some(ws) = weak_ws.upgrade() {
                    ws.update(dialog_cx, |w, cx| {
                        focus_initial_input(w, target_tab, dialog_window, cx);
                    });
                }
                dialog_cx.notify();
            });
            if result.is_err() {
                if let Some(ws) = weak_ws.upgrade() {
                    ws.update(cx, |w, _| {
                        w.find_dialog_window = None;
                    });
                }
                open_new_dialog_window(weak_ws.clone(), target_tab, cx);
            }
        });
        return;
    }

    // First open. Defer the actual window creation: `cx.open_window`
    // synchronously paints the new window (gpui calls `window.draw` inside
    // `open_window` so the window has at least one frame before returning),
    // and that paint runs `FindDialog::render`, which calls
    // `workspace.read(cx)`. We're currently inside an action handler
    // dispatched via `entity.update`, so the `Workspace` entity has been
    // *leased* out of the entity map — `read` would panic with the gpui
    // "double lease" error (which surfaces as SIGABRT through the Cocoa
    // key-equivalent shim because panics can't unwind through ObjC).
    //
    // `window.defer` enqueues the closure to run after the current effect
    // cycle, by which point the action's `update` has ended and the
    // workspace is back in the map.
    let weak_ws = cx.entity().downgrade();
    window.defer(cx, move |_window, cx| {
        open_new_dialog_window(weak_ws, target_tab, cx);
    });
}

/// Open a brand-new Find-dialog window and wire it into the workspace.
/// MUST run from a context where Workspace is NOT leased (defer body or
/// post-action), since `cx.open_window` paints the dialog synchronously and
/// the paint reads workspace state.
fn open_new_dialog_window(
    weak_ws: WeakEntity<Workspace>,
    target_tab: DialogTab,
    cx: &mut App,
) {
    let Some(workspace) = weak_ws.upgrade() else {
        return;
    };
    // Notepad++ Find dialog is roughly 600x320 in its body. We tack on
    // ~80px of vertical chrome for the tab strip + footer + group-box
    // title chips, so 600x400 keeps the layout proportional without
    // forcing a scroll. That floor is also our `window_min_size` so the
    // user can't shrink the window below the natural content size.
    // Resize is allowed in both axes — Notepad++ only lets you make the
    // window wider, but the user wants taller too.
    let bounds = crate::app::window::modal_bounds_centered_on_workspace(
        cx,
        size(px(720.), px(450.)),
    );
    let weak_ws_for_root = weak_ws.clone();
    // `WindowKind::Floating` makes the dialog float above the editor while
    // the app is active and disappear when the user switches to another
    // app (NSPanel default `hidesOnDeactivate=YES`). The user explicitly
    // wants the dialog to "stay together with" the main window — i.e.
    // follow app activation — instead of remaining on screen behind
    // unrelated apps where they can't see how to get back to it.
    //
    // `WindowBackgroundAppearance::Transparent` lets the rendered dialog
    // body composite its alpha against whatever's behind the window — that
    // is what makes the focus-fade actually look translucent on screen.
    let result = cx.open_window(
        WindowOptions {
            window_bounds: Some(bounds),
            kind: WindowKind::Floating,
            is_resizable: true,
            window_background: WindowBackgroundAppearance::Transparent,
            window_min_size: Some(size(px(720.), px(450.))),
            titlebar: Some(TitlebarOptions {
                title: Some("Find".into()),
                appears_transparent: false,
                traffic_light_position: Some(point(px(10.), px(10.))),
            }),
            ..Default::default()
        },
        |window, cx| cx.new(|cx| FindDialog::new(weak_ws_for_root.clone(), window, cx)),
    );

    let handle = match result {
        Ok(h) => h,
        Err(err) => {
            eprintln!("failed to open Find dialog: {err}");
            return;
        }
    };

    workspace.update(cx, |workspace, cx| {
        workspace.find_dialog_window = Some(handle);
        if let Ok(dialog_entity) = handle.entity(cx) {
            cx.observe_release(&dialog_entity, move |w, _, cx| {
                // Fires when the dialog window is dropped — including the
                // native macOS red close button, which bypasses
                // `close_find_dialog`. Clear the find-match highlight state
                // and repaint the editor, otherwise the as-you-type match
                // highlights stay painted after the window is gone.
                w.find_dialog_window = None;
                w.cancel_regex_replacement();
                w.find_state.open = false;
                w.find_history_open = None;
                cx.notify();
            })
            .detach();
        }
    });
    // Initial focus is set inside `handle.update` so the FocusId lands in
    // the *dialog* window's dispatch tree — focusing on the editor's
    // `window` here would never reach the dialog's input handlers.
    let weak_ws_for_focus = weak_ws.clone();
    let _ = handle.update(cx, |_dialog, dialog_window, dialog_cx| {
        if let Some(ws) = weak_ws_for_focus.upgrade() {
            ws.update(dialog_cx, |w, cx| {
                focus_initial_input(w, target_tab, dialog_window, cx);
            });
        }
    });
}

/// Place focus on the input that the targeted tab's user-facing convention
/// expects — query box for Find / Replace / Mark, query box for the FIF tab.
fn focus_initial_input(
    workspace: &mut Workspace,
    target_tab: DialogTab,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    match target_tab {
        DialogTab::Find | DialogTab::Replace => {
            window.focus(&workspace.find_focus, cx);
        }
        DialogTab::FindInFiles | DialogTab::FindInProject => {
            window.focus(&workspace.fif_query_focus, cx);
        }
        DialogTab::Mark => {
            if let Some(h) = workspace.mark_focus.clone() {
                window.focus(&h, cx);
            }
        }
    }
    cx.notify();
}

impl Render for FindDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Resolve the live workspace state. If the editor went away, render
        // an empty body — the dialog window will close itself shortly via
        // observe_release.
        let Some(workspace) = self.workspace.upgrade() else {
            return div().into_any_element();
        };
        let workspace_ref = workspace.read(cx);
        let theme = workspace_ref.theme();
        let active = workspace_ref.dialog_tab;
        // Window is opened with `WindowBackgroundAppearance::Transparent`,
        // so the body's `bg(...)` is the only thing the user sees behind
        // the controls. The transparency state lives on `find_in_files`
        // (shared with the NPP-style Transparency group on the FIF tab):
        // `enabled` gates the effect, `always` picks always-faded vs
        // only-when-inactive, and `level` is the alpha (0..100).
        let fif = &workspace_ref.find_in_files;
        let body_bg = focus_aware_bg(
            theme.bg,
            self.window_active,
            fif.transparency_enabled,
            fif.transparency_always,
            fif.transparency_level,
        );

        let body = match active {
            DialogTab::Find => find_tab::render(&theme, workspace_ref, workspace.clone()),
            DialogTab::Replace => replace_tab::render(&theme, workspace_ref, workspace.clone()),
            DialogTab::FindInFiles => fif_tab::render(&theme, workspace_ref, workspace.clone()),
            DialogTab::FindInProject => {
                find_in_project_tab::render(&theme, workspace_ref, workspace.clone())
            }
            DialogTab::Mark => mark_tab::render(&theme, workspace_ref, workspace.clone()),
        };

        let footer = footer_status(&theme, workspace_ref);
        let close_ws = workspace.clone();
        let next_ws = workspace.clone();
        let prev_ws = workspace.clone();

        div()
            .key_context("FindDialog")
            .track_focus(&self.focus)
            // Default the dialog chrome/background to the normal pointer.
            // Text inputs and buttons below set more specific cursors, so
            // those still win when hovered.
            .cursor(CursorStyle::Arrow)
            // Esc closes the dialog regardless of which input has focus.
            .on_action({
                let ws = close_ws.clone();
                move |_: &EditFindClose, window, cx| {
                    ws.update(cx, |w, cx| {
                        // Esc closes an open history dropdown first; if no
                        // dropdown is showing, fall through to close the
                        // dialog entirely.
                        if w.find_history_open.is_some() {
                            w.find_history_open = None;
                            cx.notify();
                        } else {
                            close_find_dialog(w, window, cx);
                        }
                    });
                }
            })
            // Enter inside an input goes through `Workspace::insert_newline`
            // which dispatches per-input behavior; F3 / Shift-F3 also work.
            .on_action({
                let ws = next_ws.clone();
                move |_: &EditFindNext, window, cx| {
                    ws.update(cx, |w, cx| w.find_next(&EditFindNext, window, cx));
                }
            })
            .on_action({
                let ws = prev_ws.clone();
                move |_: &EditFindPrev, window, cx| {
                    ws.update(cx, |w, cx| w.find_prev(&EditFindPrev, window, cx));
                }
            })
            // FindInFilesRun lets Enter inside the FIF inputs kick off the
            // search even from this window's dispatch tree.
            .on_action({
                let ws = workspace.clone();
                move |_: &FindInFilesRun, window, cx| {
                    ws.update(cx, |w, cx| {
                        w.find_in_files_run_action(&FindInFilesRun, window, cx)
                    });
                }
            })
            // FindInProjectsRun — same idea, but for the FIP tab.
            .on_action({
                let ws = workspace.clone();
                move |_: &FindInProjectsRun, window, cx| {
                    ws.update(cx, |w, cx| {
                        w.find_in_projects_run_action(&FindInProjectsRun, window, cx)
                    });
                }
            })
            .on_action({
                let ws = workspace.clone();
                move |_: &FindInFilesClose, window, cx| {
                    ws.update(cx, |w, cx| close_find_dialog(w, window, cx));
                }
            })
            // Enter inside any dialog input goes through `insert_newline` —
            // the same dispatcher the editor window uses — so per-input
            // behavior (Find Next / Replace / FIF run / Mark all) routes
            // through one path.
            .on_action({
                let ws = workspace.clone();
                move |_: &EditInsertNewline, window, cx| {
                    ws.update(cx, |w, cx| {
                        w.insert_newline(&EditInsertNewline, window, cx)
                    });
                }
            })
            // Editing chords inside the dialog inputs. Every handler routes
            // back to the matching `Workspace::*` method, which already
            // branches on `input_focus(window)` — so the same code path that
            // serves the editor pane and goto-line input now serves the
            // dialog inputs once we register the listener on this window's
            // dispatch tree.
            .on_action({
                let ws = workspace.clone();
                move |_: &EditBackspace, window, cx| {
                    ws.update(cx, |w, cx| w.backspace(&EditBackspace, window, cx));
                }
            })
            .on_action({
                let ws = workspace.clone();
                move |_: &EditDelete, window, cx| {
                    ws.update(cx, |w, cx| w.delete(&EditDelete, window, cx));
                }
            })
            .on_action({
                let ws = workspace.clone();
                move |_: &MoveLeft, window, cx| {
                    ws.update(cx, |w, cx| w.move_left(&MoveLeft, window, cx));
                }
            })
            .on_action({
                let ws = workspace.clone();
                move |_: &MoveRight, window, cx| {
                    ws.update(cx, |w, cx| w.move_right(&MoveRight, window, cx));
                }
            })
            .on_action({
                let ws = workspace.clone();
                move |_: &SelectLeft, window, cx| {
                    ws.update(cx, |w, cx| w.select_left(&SelectLeft, window, cx));
                }
            })
            .on_action({
                let ws = workspace.clone();
                move |_: &SelectRight, window, cx| {
                    ws.update(cx, |w, cx| w.select_right(&SelectRight, window, cx));
                }
            })
            .on_action({
                let ws = workspace.clone();
                move |_: &MoveLineStart, window, cx| {
                    ws.update(cx, |w, cx| w.move_line_start(&MoveLineStart, window, cx));
                }
            })
            .on_action({
                let ws = workspace.clone();
                move |_: &MoveLineEnd, window, cx| {
                    ws.update(cx, |w, cx| w.move_line_end(&MoveLineEnd, window, cx));
                }
            })
            .on_action({
                let ws = workspace.clone();
                move |_: &SelectLineStart, window, cx| {
                    ws.update(cx, |w, cx| {
                        w.select_line_start(&SelectLineStart, window, cx)
                    });
                }
            })
            .on_action({
                let ws = workspace.clone();
                move |_: &SelectLineEnd, window, cx| {
                    ws.update(cx, |w, cx| {
                        w.select_line_end(&SelectLineEnd, window, cx)
                    });
                }
            })
            .on_action({
                let ws = workspace.clone();
                move |_: &MoveWordLeft, window, cx| {
                    ws.update(cx, |w, cx| w.move_word_left(&MoveWordLeft, window, cx));
                }
            })
            .on_action({
                let ws = workspace.clone();
                move |_: &MoveWordRight, window, cx| {
                    ws.update(cx, |w, cx| w.move_word_right(&MoveWordRight, window, cx));
                }
            })
            .on_action({
                let ws = workspace.clone();
                move |_: &SelectWordLeft, window, cx| {
                    ws.update(cx, |w, cx| {
                        w.select_word_left(&SelectWordLeft, window, cx)
                    });
                }
            })
            .on_action({
                let ws = workspace.clone();
                move |_: &SelectWordRight, window, cx| {
                    ws.update(cx, |w, cx| {
                        w.select_word_right(&SelectWordRight, window, cx)
                    });
                }
            })
            .on_action({
                let ws = workspace.clone();
                move |_: &MoveDocStart, window, cx| {
                    ws.update(cx, |w, cx| w.move_doc_start(&MoveDocStart, window, cx));
                }
            })
            .on_action({
                let ws = workspace.clone();
                move |_: &MoveDocEnd, window, cx| {
                    ws.update(cx, |w, cx| w.move_doc_end(&MoveDocEnd, window, cx));
                }
            })
            .on_action({
                let ws = workspace.clone();
                move |_: &SelectDocStart, window, cx| {
                    ws.update(cx, |w, cx| w.select_doc_start(&SelectDocStart, window, cx));
                }
            })
            .on_action({
                let ws = workspace.clone();
                move |_: &SelectDocEnd, window, cx| {
                    ws.update(cx, |w, cx| w.select_doc_end(&SelectDocEnd, window, cx));
                }
            })
            .on_action({
                let ws = workspace.clone();
                move |_: &EditSelectAll, window, cx| {
                    ws.update(cx, |w, cx| w.select_all(&EditSelectAll, window, cx));
                }
            })
            .on_action({
                let ws = workspace.clone();
                move |_: &EditCopy, window, cx| {
                    ws.update(cx, |w, cx| w.copy(&EditCopy, window, cx));
                }
            })
            .on_action({
                let ws = workspace.clone();
                move |_: &EditCut, window, cx| {
                    ws.update(cx, |w, cx| w.cut(&EditCut, window, cx));
                }
            })
            .on_action({
                let ws = workspace.clone();
                move |_: &EditPaste, window, cx| {
                    ws.update(cx, |w, cx| w.paste(&EditPaste, window, cx));
                }
            })
            .flex()
            .flex_col()
            .size_full()
            .bg(body_bg)
            .text_color(rgb(theme.text))
            .font_family(crate::fonts::mono())
            .text_size(rems(13. / 16.))
            .line_height(rems(20. / 16.))
            .child(tab_bar(&theme, active, workspace.clone()))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .min_h(px(0.))
                    .child(body),
            )
            .child(footer)
            .into_any_element()
    }
}

/// Tear down the dialog window from a `Workspace` action context. Drops the
/// cached handle and asks the underlying window to remove itself.
pub(crate) fn close_find_dialog(
    workspace: &mut Workspace,
    _window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    workspace.cancel_regex_replacement();
    workspace.find_state.open = false;
    workspace.find_history_open = None;
    if let Some(handle) = workspace.find_dialog_window.take() {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    }
    cx.notify();
}

fn tab_bar(
    theme: &Theme,
    active: DialogTab,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    // The container paints the dialog-body tone so the empty strip *to the
    // right of the last tab* reads as body — not as a stretched tab. Each
    // tab button paints its own `chrome` (inactive) or `bg` (active) tone
    // on top, so the visible tab labels still look correct.
    div()
        .flex()
        .flex_row()
        .h(rems(28. / 16.))
        .bg(rgb(theme.bg))
        .border_b_1()
        .border_color(rgb(theme.border))
        .child(tab_button(theme, "Find", active == DialogTab::Find, DialogTab::Find, workspace.clone()))
        .child(tab_button(
            theme,
            "Replace",
            active == DialogTab::Replace,
            DialogTab::Replace,
            workspace.clone(),
        ))
        .child(tab_button(
            theme,
            "Find in Files",
            active == DialogTab::FindInFiles,
            DialogTab::FindInFiles,
            workspace.clone(),
        ))
        .child(tab_button(
            theme,
            "Find in Projects",
            active == DialogTab::FindInProject,
            DialogTab::FindInProject,
            workspace.clone(),
        ))
        .child(tab_button(
            theme,
            "Mark",
            active == DialogTab::Mark,
            DialogTab::Mark,
            workspace,
        ))
}

fn tab_button(
    theme: &Theme,
    label: &'static str,
    on: bool,
    target: DialogTab,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    let bg = if on { theme.bg } else { theme.chrome };
    let fg = if on { theme.text } else { theme.chrome_text };
    let border_color = if on { theme.toggle_active } else { theme.border };
    let hover_bg = theme.button_hover_bg;
    let pressed_bg = theme.button_pressed_bg;
    div()
        .id(gpui::SharedString::from(format!("tab_button::{label}")))
        .flex()
        .items_center()
        .justify_center()
        .h_full()
        .min_w(rems(96. / 16.))
        .px(rems(12. / 16.))
        .bg(rgb(bg))
        .border_b_2()
        .border_color(rgb(border_color))
        .text_size(rems(11.5 / 16.))
        .text_color(rgb(fg))
        .cursor(CursorStyle::PointingHand)
        .child(label)
        .hover(move |s| s.bg(rgb(hover_bg)))
        .active(move |s| s.bg(rgb(pressed_bg)))
        .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
            workspace.update(cx, |w, cx| {
                w.set_dialog_tab(target, window, cx);
            });
        })
}

fn footer_status(theme: &Theme, workspace: &Workspace) -> impl IntoElement {
    let (text, color) = match workspace.dialog_tab {
        DialogTab::Find | DialogTab::Replace => find_footer_text(workspace, theme),
        DialogTab::FindInFiles | DialogTab::FindInProject => fif_footer_text(workspace, theme),
        DialogTab::Mark => mark_footer_text(workspace, theme),
    };
    div()
        .flex()
        .flex_row()
        .items_center()
        .h(rems(22. / 16.))
        .px(rems(10. / 16.))
        .bg(rgb(theme.chrome))
        .border_t_1()
        .border_color(rgb(theme.border))
        .text_size(rems(11. / 16.))
        .text_color(rgb(color))
        .child(text)
}

fn find_footer_text(workspace: &Workspace, theme: &Theme) -> (String, u32) {
    if let Some(err) = &workspace.find_state.error {
        return (err.clone(), theme.error);
    }
    if let Some(status) = &workspace.find_state.status {
        return (status.clone(), theme.chrome_text);
    }
    if workspace.find_state.query.is_empty() {
        return (String::new(), theme.chrome_text);
    }
    if workspace.find_state.matches.is_empty() {
        return ("No matches".to_string(), theme.chrome_text);
    }
    let total = workspace.find_state.matches.len();
    let pos = workspace.find_state.current.map(|i| i + 1).unwrap_or(1);
    (format!("{pos} of {total}"), theme.chrome_text)
}

fn fif_footer_text(workspace: &Workspace, theme: &Theme) -> (String, u32) {
    if let Some(err) = &workspace.find_in_files.error {
        return (err.clone(), theme.error);
    }
    if let Some(status) = &workspace.find_in_files.status {
        return (status.clone(), theme.chrome_text);
    }
    (String::new(), theme.chrome_text)
}

fn mark_footer_text(workspace: &Workspace, theme: &Theme) -> (String, u32) {
    if let Some(err) = &workspace.mark_state.error {
        return (err.clone(), theme.error);
    }
    if let Some(status) = &workspace.mark_state.status {
        return (status.clone(), theme.chrome_text);
    }
    (String::new(), theme.chrome_text)
}
