// Shortcut Mapper — Notepad++-style keyboard shortcut editor.
//
// Settings ▸ Shortcut Mapper opens a separate top-level window that lists
// every catalog action with its current chord (defaults overlaid with any
// loaded user overrides). Click a row to select it, then click `Modify` to
// enter capture mode — the next keystroke is recorded. Save writes the
// override map to `<config>/Notepad Studio/shortcuts.toml` and rebinds the live
// keymap so the new chord works immediately.
//
// Lifecycle mirrors `find_dialog`: a single window-handle cached at module
// scope, recreated on each open. State is local to the dialog entity since
// the Shortcut Mapper has no Workspace-level interaction beyond reading the
// theme — the whole flow runs against the catalog + the dialog's own
// in-progress override map until the user clicks Save.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, CursorStyle, Entity, FocusHandle, Focusable, IntoElement,
    Keystroke, MouseButton, ParentElement, Render, SharedString, Styled, Subscription,
    TitlebarOptions, Window, WindowHandle, WindowKind, WindowOptions, div, point,
    prelude::*, px, rems, rgb, size,
};

use crate::shortcuts::{self, ActionEntry, Overrides, catalog};
use crate::theme::{Theme, ThemeMode};

thread_local! {
    /// Cached window handle so a second open call activates the existing
    /// window instead of stacking another. Cleared via `observe_release`
    /// when the window closes.
    static OPEN_WINDOW: RefCell<Option<WindowHandle<ShortcutMapper>>> = const { RefCell::new(None) };
}

/// Root entity for the Shortcut Mapper window. Owns its working override
/// map plus capture-mode state — nothing leaks back to Workspace until the
/// user saves.
pub(crate) struct ShortcutMapper {
    focus: FocusHandle,
    /// Working copy of the override map. Initialised from disk when the
    /// window opens; mutated as the user records / resets chords; flushed
    /// to disk on Save.
    overrides: Overrides,
    /// Index into `catalog()` of the currently-selected row. `None` when
    /// nothing is selected (initial state, or after a `Reset all`).
    selected: Option<usize>,
    /// `Some(idx)` when row `idx` is in capture mode — the next keystroke
    /// becomes that row's new chord. The keystroke is intercepted on the
    /// dialog's root div via `capture_key_down` so action dispatch is
    /// suppressed for the duration.
    capturing: Option<usize>,
    /// Status / warning message shown in the dialog footer. Cleared on the
    /// next user interaction (save / cancel / row change). The companion
    /// `is_error` flag picks the colour: `true` paints with `theme.error`,
    /// `false` paints with `theme.chrome_text` (neutral status).
    status: Option<String>,
    is_error: bool,
    /// Pending conflict prompt. When `Some`, the user just recorded a chord
    /// that already maps to another action — render an Override / Cancel
    /// pair and stash the proposed chord here until they decide.
    pending_conflict: Option<PendingConflict>,
    /// Theme mode — read from the active editor window if there is one,
    /// otherwise dark (the app default). Cached at construction so the
    /// dialog's render doesn't have to chase down a Workspace handle every
    /// frame.
    theme_mode: ThemeMode,
    /// App-level keystroke interceptor that swallows the next chord while
    /// `capturing.is_some()`. Installed on `start_capture`, dropped on
    /// `cancel_capture` / `handle_capture` so other windows aren't affected
    /// outside capture mode. Cleared automatically when the dialog is
    /// dropped (subscription holds no static state otherwise).
    _capture_interceptor: Option<Subscription>,
}

#[derive(Clone)]
struct PendingConflict {
    row: usize,
    chord: String,
    /// Action id of the existing owner of `chord` — the row that will lose
    /// the chord if the user clicks Override.
    owner_id: &'static str,
}

impl ShortcutMapper {
    fn new(theme_mode: ThemeMode, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            overrides: shortcuts::load_overrides(),
            selected: None,
            capturing: None,
            status: None,
            is_error: false,
            pending_conflict: None,
            theme_mode,
            _capture_interceptor: None,
        }
    }

    fn theme(&self) -> Theme {
        Theme::for_mode(self.theme_mode)
    }
}

impl Focusable for ShortcutMapper {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Open the Shortcut Mapper window. If a previous instance is still on
/// screen, activate it; otherwise spawn a fresh window.
pub(crate) fn open_or_focus(cx: &mut App) {
    let cached = OPEN_WINDOW.with(|cell| cell.borrow().clone());
    if let Some(handle) = cached {
        let result = handle.update(cx, |_, window, cx| {
            window.activate_window();
            cx.notify();
        });
        if result.is_ok() {
            return;
        }
        // Stale handle (window already destroyed) — clear and recreate.
        OPEN_WINDOW.with(|cell| *cell.borrow_mut() = None);
    }

    let theme_mode = current_theme_mode(cx);
    let bounds = crate::app::window::modal_bounds_centered_on_workspace(
        cx,
        size(px(720.), px(560.)),
    );
    let result = cx.open_window(
        WindowOptions {
            window_bounds: Some(bounds),
            kind: WindowKind::Floating,
            is_resizable: true,
            window_min_size: Some(size(px(640.), px(400.))),
            titlebar: Some(TitlebarOptions {
                title: Some("Shortcut Mapper".into()),
                appears_transparent: false,
                traffic_light_position: Some(point(px(10.), px(10.))),
            }),
            ..Default::default()
        },
        |_window, cx| cx.new(|cx| ShortcutMapper::new(theme_mode, cx)),
    );

    let handle = match result {
        Ok(h) => h,
        Err(err) => {
            eprintln!("failed to open Shortcut Mapper: {err}");
            return;
        }
    };

    OPEN_WINDOW.with(|cell| *cell.borrow_mut() = Some(handle));
    if let Ok(entity) = handle.entity(cx) {
        cx.observe_release(&entity, move |_, _| {
            OPEN_WINDOW.with(|cell| *cell.borrow_mut() = None);
        })
        .detach();
    }
    let _ = handle.update(cx, |this, window, cx| {
        window.focus(&this.focus, cx);
        cx.notify();
    });
}

/// Probe the active editor window for its theme mode so the Shortcut Mapper
/// matches. Falls back to dark when no Workspace window is around (the
/// default surface tone).
fn current_theme_mode(cx: &mut App) -> ThemeMode {
    use crate::workspace::Workspace;
    let handle = cx
        .active_window()
        .and_then(|h| h.downcast::<Workspace>())
        .or_else(|| {
            cx.windows()
                .into_iter()
                .find_map(|h| h.downcast::<Workspace>())
        });
    let Some(handle) = handle else {
        return ThemeMode::Dark;
    };
    handle
        .read(cx)
        .map(|w| w.theme_mode)
        .unwrap_or(ThemeMode::Dark)
}

impl Render for ShortcutMapper {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme();
        let entries = catalog();
        let entity = cx.entity().clone();

        let root = div()
            .key_context("ShortcutMapper")
            .track_focus(&self.focus)
            .cursor(CursorStyle::Arrow)
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(theme.bg))
            .text_color(rgb(theme.text))
            .font_family("Menlo")
            .text_size(rems(13. / 16.))
            .line_height(rems(20. / 16.));
        // Capture mode does NOT install an `on_key_down`-style listener
        // here: gpui dispatches matched-action listeners *before* element
        // key listeners, so a binding like cmd-q would fire before we
        // could swallow it. Instead we attach an `App::intercept_keystrokes`
        // subscription in `start_capture`, which fires before action
        // dispatch and lets us call `cx.stop_propagation()` to consume
        // the event.

        // Header — single tab strip ("Main menu") so the surface mirrors
        // Notepad++'s tabbed dialog even though v1 only ships the one tab.
        let header = div()
            .flex()
            .flex_row()
            .h(rems(28. / 16.))
            .bg(rgb(theme.chrome))
            .border_b_1()
            .border_color(rgb(theme.border))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .h_full()
                    .min_w(rems(120. / 16.))
                    .px(rems(12. / 16.))
                    .bg(rgb(theme.bg))
                    .border_b_2()
                    .border_color(rgb(theme.toggle_active))
                    .text_size(rems(11.5 / 16.))
                    .text_color(rgb(theme.text))
                    .child("Main menu"),
            );

        // Column header row.
        let columns = div()
            .flex()
            .flex_row()
            .h(rems(22. / 16.))
            .border_b_1()
            .border_color(rgb(theme.border))
            .bg(rgb(theme.chrome))
            .text_size(rems(11. / 16.))
            .text_color(rgb(theme.chrome_text))
            .child(
                div()
                    .flex()
                    .items_center()
                    .px(rems(10. / 16.))
                    .w(rems(280. / 16.))
                    .child("Action"),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .items_center()
                    .px(rems(10. / 16.))
                    .child("Shortcut"),
            );

        // List body — one row per catalog entry.
        let mut list = div()
            .flex_1()
            .flex()
            .flex_col()
            .min_h(px(0.))
            .id(SharedString::from("shortcut_list"))
            .overflow_y_scroll();

        for (idx, entry) in entries.iter().enumerate() {
            list = list.child(self.render_row(&theme, idx, entry, &entity));
        }

        let footer = self.render_footer(&theme, &entity);
        let status_bar = self.render_status_bar(&theme);

        root.child(header)
            .child(columns)
            .child(list)
            .child(status_bar)
            .child(footer)
            .into_any_element()
    }
}

impl ShortcutMapper {
    fn render_row(
        &self,
        theme: &Theme,
        idx: usize,
        entry: &'static ActionEntry,
        entity: &Entity<ShortcutMapper>,
    ) -> AnyElement {
        let selected = self.selected == Some(idx);
        let capturing = self.capturing == Some(idx);
        let row_bg = if capturing {
            theme.checkbox_fill
        } else if selected {
            theme.button_pressed_bg
        } else {
            theme.bg
        };
        let row_fg = if capturing {
            0xffffff
        } else {
            theme.text
        };
        let hover_bg = theme.button_hover_bg;
        let chord_text = if capturing {
            "Press a key…".to_string()
        } else {
            format_chord_list(&shortcuts::effective_keys(entry, &self.overrides))
        };

        let entity_click = entity.clone();
        div()
            .id(SharedString::from(format!("row::{}", entry.id)))
            .flex()
            .flex_row()
            .h(rems(24. / 16.))
            .bg(rgb(row_bg))
            .text_color(rgb(row_fg))
            .text_size(rems(11.5 / 16.))
            .border_b_1()
            .border_color(rgb(theme.border))
            .cursor(CursorStyle::PointingHand)
            .hover(move |s| s.bg(rgb(hover_bg)))
            .on_mouse_down(MouseButton::Left, move |_e, _window, cx| {
                let entity = entity_click.clone();
                entity.update(cx, |this, cx| {
                    this.select_row(idx, cx);
                });
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .px(rems(10. / 16.))
                    .w(rems(280. / 16.))
                    .child(entry.label),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .items_center()
                    .px(rems(10. / 16.))
                    .text_color(rgb(if capturing {
                        0xffffff
                    } else if shortcuts::effective_keys(entry, &self.overrides).is_empty() {
                        theme.disabled_text
                    } else {
                        theme.text
                    }))
                    .child(chord_text),
            )
            .into_any_element()
    }

    fn render_footer(
        &self,
        theme: &Theme,
        entity: &Entity<ShortcutMapper>,
    ) -> AnyElement {
        let row = self.selected;
        let capturing = self.capturing.is_some();
        let conflict = self.pending_conflict.is_some();

        // Standard buttons. While a conflict prompt is up we replace the
        // Modify cluster with Override / Cancel so the user can resolve
        // it without leaving the row.
        let mut bar = div()
            .flex()
            .flex_row()
            .gap(rems(8. / 16.))
            .items_center()
            .h(rems(40. / 16.))
            .px(rems(10. / 16.))
            .border_t_1()
            .border_color(rgb(theme.border))
            .bg(rgb(theme.chrome));

        if conflict {
            let entity_override = entity.clone();
            bar = bar.child(action_button(
                theme,
                "Override",
                row.is_some(),
                true,
                move |cx| {
                    let entity = entity_override.clone();
                    entity.update(cx, |this, cx| this.confirm_conflict_override(cx));
                },
            ));
            let entity_cancel = entity.clone();
            bar = bar.child(action_button(
                theme,
                "Cancel",
                true,
                false,
                move |cx| {
                    let entity = entity_cancel.clone();
                    entity.update(cx, |this, cx| this.cancel_conflict(cx));
                },
            ));
        } else {
            let entity_modify = entity.clone();
            bar = bar.child(action_button(
                theme,
                if capturing { "Cancel Capture" } else { "Modify" },
                row.is_some(),
                true,
                move |cx| {
                    let entity = entity_modify.clone();
                    entity.update(cx, |this, cx| {
                        if this.capturing.is_some() {
                            this.cancel_capture(cx);
                        } else {
                            this.start_capture(cx);
                        }
                    });
                },
            ));

            let entity_clear = entity.clone();
            bar = bar.child(action_button(
                theme,
                "Clear",
                row.is_some() && !capturing,
                false,
                move |cx| {
                    let entity = entity_clear.clone();
                    entity.update(cx, |this, cx| this.clear_row(cx));
                },
            ));

            let entity_reset_row = entity.clone();
            bar = bar.child(action_button(
                theme,
                "Reset Row",
                row.is_some() && !capturing,
                false,
                move |cx| {
                    let entity = entity_reset_row.clone();
                    entity.update(cx, |this, cx| this.reset_row(cx));
                },
            ));

            let entity_reset_all = entity.clone();
            bar = bar.child(action_button(
                theme,
                "Reset All",
                !capturing,
                false,
                move |cx| {
                    let entity = entity_reset_all.clone();
                    entity.update(cx, |this, cx| this.reset_all(cx));
                },
            ));
        }

        // Right-aligned Save / Close.
        let entity_save = entity.clone();
        let save_btn = action_button(theme, "Save", !capturing && !conflict, true, move |cx| {
            let entity = entity_save.clone();
            entity.update(cx, |this, cx| this.save(cx));
        });
        let entity_close = entity.clone();
        let close_btn = action_button(theme, "Close", !capturing, false, move |cx| {
            let entity = entity_close.clone();
            entity.update(cx, |_, _| {});
            close_window(cx);
        });

        bar.child(
            div()
                .flex_1()
                .flex()
                .flex_row()
                .justify_end()
                .gap(rems(8. / 16.))
                .child(save_btn)
                .child(close_btn),
        )
        .into_any_element()
    }

    fn render_status_bar(&self, theme: &Theme) -> AnyElement {
        let (text, color) = match (&self.status, self.pending_conflict.as_ref()) {
            (_, Some(conflict)) => (
                format!(
                    "{} is already mapped to {}. Override?",
                    pretty_chord(&conflict.chord),
                    label_for(conflict.owner_id),
                ),
                theme.error,
            ),
            (Some(text), _) if self.is_error => (text.clone(), theme.error),
            (Some(text), _) => (text.clone(), theme.chrome_text),
            (None, None) => (String::new(), theme.chrome_text),
        };
        div()
            .flex()
            .items_center()
            .h(rems(22. / 16.))
            .px(rems(10. / 16.))
            .bg(rgb(theme.chrome))
            .border_t_1()
            .border_color(rgb(theme.border))
            .text_size(rems(11. / 16.))
            .text_color(rgb(color))
            .child(text)
            .into_any_element()
    }

    // ---------------- state mutation ----------------

    fn select_row(&mut self, idx: usize, cx: &mut Context<Self>) {
        if self.capturing.is_some() {
            return;
        }
        self.selected = Some(idx);
        self.status = None;
        self.pending_conflict = None;
        cx.notify();
    }

    fn start_capture(&mut self, cx: &mut Context<Self>) {
        let Some(idx) = self.selected else { return };
        self.capturing = Some(idx);
        self.status = Some("Press a key combination… (Esc to cancel)".to_string());
        self.is_error = false;
        // Install the global keystroke interceptor. The handler checks
        // the calling window against our cached handle so the editor
        // windows aren't affected. We `cx.defer` the entity update because
        // the interceptor fires while the dialog entity is leased out for
        // the current effect cycle — calling `entity.update` directly
        // would trip gpui's double-lease guard.
        let entity = cx.entity().clone();
        let our_window = OPEN_WINDOW.with(|cell| cell.borrow().clone());
        self._capture_interceptor = Some(cx.intercept_keystrokes(
            move |event, window, cx| {
                let Some(our_handle) = our_window else {
                    return;
                };
                if window.window_handle().window_id() != our_handle.window_id() {
                    return;
                }
                cx.stop_propagation();
                let keystroke = event.keystroke.clone();
                let entity = entity.clone();
                cx.defer(move |cx| {
                    entity.update(cx, |this, cx| {
                        this.handle_capture(&keystroke, cx);
                    });
                });
            },
        ));
        cx.notify();
    }

    fn cancel_capture(&mut self, cx: &mut Context<Self>) {
        self.capturing = None;
        self._capture_interceptor = None;
        self.status = None;
        cx.notify();
    }

    fn cancel_conflict(&mut self, cx: &mut Context<Self>) {
        self.pending_conflict = None;
        self.status = None;
        cx.notify();
    }

    fn confirm_conflict_override(&mut self, cx: &mut Context<Self>) {
        let Some(conflict) = self.pending_conflict.take() else {
            return;
        };
        // Strip the conflicting chord from the previous owner's effective
        // list, persisting that decision as an override so a future load
        // doesn't reintroduce the default.
        let Some(prev_owner) = catalog().iter().find(|e| e.id == conflict.owner_id) else {
            self.status = Some("Conflict owner no longer in catalog; aborted.".to_string());
            self.is_error = true;
            cx.notify();
            return;
        };
        let mut prev_keys = shortcuts::effective_keys(prev_owner, &self.overrides);
        prev_keys.retain(|k| k != &conflict.chord);
        self.overrides
            .insert(prev_owner.id.to_string(), prev_keys);

        // Assign the chord to the row that requested it (single-key model
        // for v1 — overwrites any previous chord on that row).
        self.overrides
            .insert(catalog()[conflict.row].id.to_string(), vec![conflict.chord]);
        self.status = Some(format!(
            "Overrode {}. Click Save to persist.",
            label_for(conflict.owner_id)
        ));
        self.is_error = false;
        cx.notify();
    }

    fn clear_row(&mut self, cx: &mut Context<Self>) {
        let Some(idx) = self.selected else { return };
        let entry = &catalog()[idx];
        // Empty Vec = explicitly unbound. Distinct from Reset (which drops
        // the override entry entirely so the default re-applies).
        self.overrides.insert(entry.id.to_string(), vec![]);
        self.status = Some(format!("{} unbound. Click Save to persist.", entry.label));
        self.is_error = false;
        cx.notify();
    }

    fn reset_row(&mut self, cx: &mut Context<Self>) {
        let Some(idx) = self.selected else { return };
        let entry = &catalog()[idx];
        self.overrides.remove(entry.id);
        self.status = Some(format!(
            "Reset {} to default. Click Save to persist.",
            entry.label
        ));
        self.is_error = false;
        cx.notify();
    }

    fn reset_all(&mut self, cx: &mut Context<Self>) {
        self.overrides.clear();
        self.selected = None;
        self.status = Some("All shortcuts reset to defaults. Click Save to persist.".to_string());
        self.is_error = false;
        cx.notify();
    }

    fn handle_capture(&mut self, ks: &Keystroke, cx: &mut Context<Self>) {
        let Some(idx) = self.capturing else { return };
        // Esc cancels capture without recording.
        if ks.key == "escape" && !ks.modifiers.modified() {
            self.capturing = None;
            self._capture_interceptor = None;
            self.status = None;
            cx.notify();
            return;
        }
        // Reject lone modifier keys (cmd/ctrl/alt/shift on their own can't
        // form a binding). gpui surfaces these as separate keys.
        if matches!(
            ks.key.as_str(),
            "cmd" | "ctrl" | "control" | "alt" | "shift" | "platform" | "super" | "win" | "fn"
        ) {
            return;
        }
        let chord = format_keystroke_for_storage(ks);
        // Validate by re-parsing — if gpui can't read what we just wrote
        // back, the binder would panic on registration. Refuse the chord
        // and prompt the user to try again.
        if gpui::Keystroke::parse(&chord).is_err() {
            self.status = Some(format!("Cannot record {chord}. Try a different chord."));
            self.is_error = true;
            self.capturing = None;
            self._capture_interceptor = None;
            cx.notify();
            return;
        }
        self.capturing = None;
        self._capture_interceptor = None;

        let row_id = catalog()[idx].id;
        if let Some(owner_id) = shortcuts::conflict_for(&chord, &self.overrides, row_id) {
            self.pending_conflict = Some(PendingConflict {
                row: idx,
                chord,
                owner_id,
            });
            self.status = None;
            self.is_error = false;
            cx.notify();
            return;
        }

        // No conflict — single-key model: replace the row's chord list.
        self.overrides.insert(row_id.to_string(), vec![chord.clone()]);
        self.status = Some(format!(
            "Captured {}. Click Save to persist.",
            pretty_chord(&chord)
        ));
        self.is_error = false;
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        match shortcuts::save_overrides(&self.overrides) {
            Ok(()) => {
                shortcuts::reload_keymap(cx, &self.overrides);
                self.status = Some("Saved.".to_string());
                self.is_error = false;
            }
            Err(err) => {
                self.status = Some(format!("Save failed: {err}"));
                self.is_error = true;
            }
        }
        cx.notify();
    }
}

/// Close the dialog window. Routes through `App` rather than the entity so
/// the caller doesn't need a `Window`.
fn close_window(cx: &mut App) {
    let handle = OPEN_WINDOW.with(|cell| cell.borrow().clone());
    if let Some(handle) = handle {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    }
}

/// Produce the chord string we persist to disk and pass back to gpui's
/// `KeyBinding::new`. Mirrors the `cmd-shift-f` / `ctrl-alt-s` syntax used
/// by the catalog defaults.
fn format_keystroke_for_storage(ks: &gpui::Keystroke) -> String {
    let mut out = String::new();
    if ks.modifiers.function {
        out.push_str("fn-");
    }
    if ks.modifiers.control {
        out.push_str("ctrl-");
    }
    if ks.modifiers.alt {
        out.push_str("alt-");
    }
    if ks.modifiers.shift {
        out.push_str("shift-");
    }
    if ks.modifiers.platform {
        out.push_str("cmd-");
    }
    out.push_str(&ks.key);
    out
}

/// Render a chord for the Shortcut column. Uses `Cmd+Shift+F` style on every
/// platform so the dialog reads consistently regardless of where it runs.
fn pretty_chord(chord: &str) -> String {
    let mut parts: Vec<&str> = chord.split('-').collect();
    // Last part is the key itself; everything before is modifiers.
    let key = parts.pop().unwrap_or("");
    let mut out = String::new();
    for part in parts {
        let label = match part {
            "cmd" => "Cmd",
            "ctrl" => "Ctrl",
            "alt" => "Alt",
            "shift" => "Shift",
            "fn" => "Fn",
            other => other,
        };
        out.push_str(label);
        out.push('+');
    }
    let key_label = match key {
        "" => String::new(),
        k if k.len() == 1 => k.to_uppercase(),
        k => {
            // Capitalise the first letter so `pageup` becomes `PageUp`.
            let mut cs = k.chars();
            match cs.next() {
                Some(c) => c.to_uppercase().collect::<String>() + cs.as_str(),
                None => String::new(),
            }
        }
    };
    out.push_str(&key_label);
    out
}

fn format_chord_list(keys: &[String]) -> String {
    if keys.is_empty() {
        return "(none)".to_string();
    }
    keys.iter()
        .map(|k| pretty_chord(k))
        .collect::<Vec<_>>()
        .join(", ")
}

fn label_for(action_id: &str) -> &'static str {
    catalog()
        .iter()
        .find(|e| e.id == action_id)
        .map(|e| e.label)
        .unwrap_or("(unknown)")
}

/// Footer button — small, square, Notepad++-style. Inline copy of the
/// dialog widget rather than reused from `find_dialog/widgets.rs` because
/// this module sits outside the find_dialog tree and the find widgets are
/// `pub(super)` to that tree (touching them would also pull in a Workspace
/// dependency we don't need here).
fn action_button<F>(
    theme: &Theme,
    label: &'static str,
    enabled: bool,
    primary: bool,
    handler: F,
) -> AnyElement
where
    F: Fn(&mut App) + 'static,
{
    let bg: u32 = theme.button_bg;
    let border: u32 = if primary && enabled {
        theme.checkbox_fill
    } else if enabled {
        theme.group_box_border
    } else {
        theme.disabled_text
    };
    let fg: u32 = if enabled {
        theme.text
    } else {
        theme.disabled_text
    };
    let cursor = if enabled {
        CursorStyle::PointingHand
    } else {
        CursorStyle::OperationNotAllowed
    };
    let hover_bg = theme.button_hover_bg;
    let pressed_bg = theme.button_pressed_bg;
    let handler = Rc::new(handler);
    let node = div()
        .id(SharedString::from(format!("sm_button::{label}")))
        .flex()
        .items_center()
        .justify_center()
        .h(rems(24. / 16.))
        .min_w(rems(96. / 16.))
        .px(rems(10. / 16.))
        .bg(rgb(bg))
        .border_1()
        .border_color(rgb(border))
        .text_size(rems(11.5 / 16.))
        .text_color(rgb(fg))
        .cursor(cursor)
        .child(label);
    if enabled {
        node.hover(move |s| s.bg(rgb(hover_bg)))
            .active(move |s| s.bg(rgb(pressed_bg)))
            .on_mouse_down(MouseButton::Left, move |_e, _window, cx| {
                (handler)(cx);
            })
            .into_any_element()
    } else {
        node.into_any_element()
    }
}
