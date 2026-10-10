// Clipboard History panel — Edit ▸ Clipboard History.
//
// Lightweight floating window that lists recent clipboard payloads recorded
// by `crate::clipboard_history`. Click a row (or press the row's keyboard
// shortcut) to paste it at the active editor's caret without disturbing
// what the OS clipboard currently holds — the live clipboard is for what
// the user *just* copied; this panel is for what they copied a couple of
// operations ago.
//
// Notepad++ source:
//   PowerEditor/src/WinControls/ClipboardHistoryPanel/ClipboardHistoryPanel.cpp
//   PowerEditor/src/NppCommands.cpp:1147 — IDM_EDIT_CLIPBOARDHISTORY_PANEL
//
// NPP renders this as a dockable side panel. Notepad Studio v1 ships a
// floating popup window for simplicity — Workspace doesn't yet host
// dockable side panels generally, and the floating window already covers
// the "give me a previous clipboard entry" use case the feature is for.

use std::cell::RefCell;

use gpui::{
    App, Context, CursorStyle, Entity, FocusHandle, Focusable, IntoElement, MouseButton,
    ParentElement, Render, SharedString, Styled, TitlebarOptions, Window, WindowHandle,
    WindowKind, WindowOptions, div, point, prelude::*, px, rems, rgb, size,
};

use crate::clipboard_history;
use crate::theme::{Theme, ThemeMode};
use crate::workspace::Workspace;

thread_local! {
    /// Cached window handle so a second open call activates the existing
    /// window instead of stacking another. Cleared via `observe_release`
    /// when the window closes.
    static OPEN_WINDOW: RefCell<Option<WindowHandle<ClipboardHistoryPanel>>> =
        const { RefCell::new(None) };
}

/// Maximum number of characters from each entry that the row preview shows
/// before truncating with a trailing ellipsis. NPP truncates around 40
/// characters; we get more horizontal room so 80 reads naturally without
/// wrapping the row.
const PREVIEW_CHAR_LIMIT: usize = 80;

pub(crate) struct ClipboardHistoryPanel {
    focus: FocusHandle,
    theme_mode: ThemeMode,
    /// Workspace window handle the panel was opened from. Paste-on-click
    /// routes through this handle so the entry lands in the editor the
    /// user was working in when they opened the panel — even if they
    /// alt-tabbed to a different workspace window in between.
    source_workspace: Option<WindowHandle<Workspace>>,
    /// Transient toast shown when an action runs ("Pasted entry 3",
    /// "History cleared"). Auto-clears on the next render that fires after
    /// the user interacts again — kept simple, no timer.
    status: Option<String>,
}

impl ClipboardHistoryPanel {
    fn new(
        theme_mode: ThemeMode,
        source_workspace: Option<WindowHandle<Workspace>>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            focus: cx.focus_handle(),
            theme_mode,
            source_workspace,
            status: None,
        }
    }

    fn theme(&self) -> Theme {
        Theme::for_mode(self.theme_mode)
    }

    /// Insert the entry at `index` into the source workspace's active
    /// editor. Updates `status`; does NOT close the panel — Notepad++
    /// keeps its panel open across pastes so the user can paste several
    /// entries in sequence.
    fn paste_entry(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(text) = clipboard_history::get(index) else {
            return;
        };
        let Some(handle) = self.source_workspace else {
            self.status = Some("No editor window to paste into.".into());
            cx.notify();
            return;
        };
        let text_for_status = text.clone();
        let pasted = handle
            .update(cx, |ws, _window, cx| {
                ws.paste_string_into_editor(&text, cx)
            })
            .unwrap_or(false);
        self.status = Some(if pasted {
            format!(
                "Pasted entry {} ({} chars).",
                index + 1,
                text_for_status.chars().count()
            )
        } else {
            "Active tab can't accept a plain-text paste right now.".into()
        });
        cx.notify();
    }

    fn clear_history(&mut self, cx: &mut Context<Self>) {
        clipboard_history::clear();
        self.status = Some("History cleared.".into());
        cx.notify();
    }
}

impl Focusable for ClipboardHistoryPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Open the Clipboard History panel. If a previous instance is still on
/// screen, activate it; otherwise spawn a fresh window anchored to the
/// active workspace so paste-on-click targets the right editor.
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
        OPEN_WINDOW.with(|cell| *cell.borrow_mut() = None);
    }

    let theme_mode = current_theme_mode(cx);
    let source = active_workspace(cx);

    let bounds = crate::app::window::modal_bounds_centered_on_workspace(
        cx,
        size(px(520.), px(360.)),
    );
    let result = cx.open_window(
        WindowOptions {
            window_bounds: Some(bounds),
            kind: WindowKind::Floating,
            is_resizable: true,
            window_min_size: Some(size(px(360.), px(220.))),
            titlebar: Some(TitlebarOptions {
                title: Some("Clipboard History".into()),
                appears_transparent: false,
                traffic_light_position: Some(point(px(10.), px(10.))),
            }),
            ..Default::default()
        },
        |_window, cx| {
            cx.new(|cx| ClipboardHistoryPanel::new(theme_mode, source, cx))
        },
    );

    let handle = match result {
        Ok(h) => h,
        Err(err) => {
            eprintln!("failed to open Clipboard History: {err}");
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

fn current_theme_mode(cx: &mut App) -> ThemeMode {
    active_workspace(cx)
        .and_then(|h| h.read(cx).ok().map(|w| w.theme_mode))
        .unwrap_or(ThemeMode::Dark)
}

fn active_workspace(cx: &mut App) -> Option<WindowHandle<Workspace>> {
    cx.active_window()
        .and_then(|h| h.downcast::<Workspace>())
        .or_else(|| {
            cx.windows()
                .into_iter()
                .find_map(|h| h.downcast::<Workspace>())
        })
}

impl Render for ClipboardHistoryPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme();
        let entries = clipboard_history::snapshot();
        let entity = cx.entity().clone();

        let root = div()
            .key_context("ClipboardHistoryPanel")
            .track_focus(&self.focus)
            .cursor(CursorStyle::Arrow)
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(theme.bg))
            .text_color(rgb(theme.text))
            .font_family(crate::fonts::mono())
            .text_size(rems(13. / 16.))
            .line_height(rems(20. / 16.));

        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .px(rems(12. / 16.))
            .h(rems(28. / 16.))
            .bg(rgb(theme.chrome))
            .border_b_1()
            .border_color(rgb(theme.border))
            .text_size(rems(11.5 / 16.))
            .text_color(rgb(theme.chrome_text))
            .child(format!(
                "Recent clipboard entries — {}/{}",
                entries.len(),
                clipboard_history_capacity()
            ));

        // List body — newest entry first. Empty state takes over the whole
        // body region when there's nothing to show.
        let body = if entries.is_empty() {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .px(rems(24. / 16.))
                .text_color(rgb(theme.chrome_text))
                .text_size(rems(12. / 16.))
                .child(
                    "No clipboard history yet. Copy or cut something \
                     in any editor tab to start filling it.",
                )
                .into_any_element()
        } else {
            let mut list = div()
                .flex_1()
                .flex()
                .flex_col()
                .min_h(px(0.))
                .id(SharedString::from("clip_hist_list"))
                .overflow_y_scroll();
            for (idx, entry) in entries.iter().enumerate() {
                list = list.child(self.render_row(&theme, idx, entry, &entity));
            }
            list.into_any_element()
        };

        let footer = self.render_footer(&theme, &entity);
        let status_bar = self.render_status_bar(&theme);

        root.child(header).child(body).child(status_bar).child(footer).into_any_element()
    }
}

impl ClipboardHistoryPanel {
    fn render_row(
        &self,
        theme: &Theme,
        idx: usize,
        entry: &str,
        entity: &Entity<ClipboardHistoryPanel>,
    ) -> gpui::AnyElement {
        let preview = preview_text(entry);
        let char_count = entry.chars().count();
        let hover_bg = theme.button_hover_bg;
        let entity_click = entity.clone();
        div()
            .id(SharedString::from(format!("clip_hist_row::{idx}")))
            .flex()
            .flex_row()
            .items_center()
            .h(rems(26. / 16.))
            .px(rems(10. / 16.))
            .gap(rems(8. / 16.))
            .bg(rgb(theme.bg))
            .text_color(rgb(theme.text))
            .text_size(rems(12. / 16.))
            .border_b_1()
            .border_color(rgb(theme.border))
            .cursor(CursorStyle::PointingHand)
            .hover(move |s| s.bg(rgb(hover_bg)))
            .on_mouse_down(MouseButton::Left, move |_e, _window, cx| {
                let click = entity_click.clone();
                cx.defer(move |cx| {
                    let _ = click.update(cx, |this, cx| this.paste_entry(idx, cx));
                });
            })
            .child(
                div()
                    .w(rems(28. / 16.))
                    .text_color(rgb(theme.chrome_text))
                    .child(format!("{}", idx + 1)),
            )
            .child(div().flex_1().child(preview))
            .child(
                div()
                    .text_color(rgb(theme.chrome_text))
                    .text_size(rems(11. / 16.))
                    .child(format!("{char_count} ch")),
            )
            .into_any_element()
    }

    fn render_status_bar(&self, theme: &Theme) -> gpui::AnyElement {
        let status = self.status.clone().unwrap_or_default();
        div()
            .h(rems(20. / 16.))
            .px(rems(12. / 16.))
            .flex()
            .items_center()
            .bg(rgb(theme.chrome))
            .border_t_1()
            .border_color(rgb(theme.border))
            .text_color(rgb(theme.chrome_text))
            .text_size(rems(11. / 16.))
            .child(status)
            .into_any_element()
    }

    fn render_footer(
        &self,
        theme: &Theme,
        entity: &Entity<ClipboardHistoryPanel>,
    ) -> gpui::AnyElement {
        let entity_clear = entity.clone();
        let entity_close = entity.clone();
        let hover_bg = theme.button_hover_bg;
        div()
            .h(rems(36. / 16.))
            .px(rems(12. / 16.))
            .flex()
            .flex_row()
            .items_center()
            .justify_end()
            .gap(rems(8. / 16.))
            .bg(rgb(theme.chrome))
            .border_t_1()
            .border_color(rgb(theme.border))
            .child(
                div()
                    .id(SharedString::from("clip_hist_clear"))
                    .px(rems(10. / 16.))
                    .py(rems(4. / 16.))
                    .bg(rgb(theme.button_bg))
                    .border_1()
                    .border_color(rgb(theme.border))
                    .text_color(rgb(theme.text))
                    .text_size(rems(12. / 16.))
                    .cursor(CursorStyle::PointingHand)
                    .hover(move |s| s.bg(rgb(hover_bg)))
                    .on_mouse_down(MouseButton::Left, move |_e, _window, cx| {
                        let clear = entity_clear.clone();
                        cx.defer(move |cx| {
                            let _ = clear.update(cx, |this, cx| this.clear_history(cx));
                        });
                    })
                    .child("Clear History"),
            )
            .child(
                div()
                    .id(SharedString::from("clip_hist_close"))
                    .px(rems(10. / 16.))
                    .py(rems(4. / 16.))
                    .bg(rgb(theme.button_bg))
                    .border_1()
                    .border_color(rgb(theme.border))
                    .text_color(rgb(theme.text))
                    .text_size(rems(12. / 16.))
                    .cursor(CursorStyle::PointingHand)
                    .hover(move |s| s.bg(rgb(hover_bg)))
                    .on_mouse_down(MouseButton::Left, move |_e, window, _cx| {
                        // Closing the window drops the entity, which fires
                        // the `observe_release` registered in `open_or_focus`
                        // and clears the cached handle.
                        let _ = entity_close.clone();
                        window.remove_window();
                    })
                    .child("Close"),
            )
            .into_any_element()
    }
}

/// Render `text` as a single-line preview suitable for one row of the
/// list: collapses embedded newlines and tabs into visible glyphs, trims
/// to `PREVIEW_CHAR_LIMIT` chars with an ellipsis if longer. Operates on
/// chars rather than bytes so multi-byte UTF-8 sequences don't truncate
/// mid-grapheme.
fn preview_text(text: &str) -> String {
    let mut out = String::with_capacity(PREVIEW_CHAR_LIMIT.min(text.len()));
    let mut taken = 0usize;
    for ch in text.chars() {
        if taken >= PREVIEW_CHAR_LIMIT {
            out.push('…');
            return out;
        }
        match ch {
            '\n' | '\r' => out.push('⏎'),
            '\t' => out.push('→'),
            c if c.is_control() => out.push('·'),
            c => out.push(c),
        }
        taken += 1;
    }
    out
}

/// Mirrored from `clipboard_history::CAPACITY` for the header readout.
/// Kept in sync by inspection — the header just reports a denominator,
/// the history module is the source of truth for actual cap behaviour.
fn clipboard_history_capacity() -> usize {
    16
}

#[cfg(test)]
mod tests {
    use super::preview_text;

    #[test]
    fn preview_collapses_newlines_and_tabs() {
        let s = "line one\nline two\tcol";
        assert_eq!(preview_text(s), "line one⏎line two→col");
    }

    #[test]
    fn preview_truncates_long_content_with_ellipsis() {
        let s = "x".repeat(200);
        let out = preview_text(&s);
        // 80 'x' chars + the ellipsis glyph.
        assert!(out.ends_with('…'));
        assert_eq!(out.chars().filter(|c| *c == 'x').count(), 80);
    }

    #[test]
    fn preview_respects_char_boundary_for_multibyte() {
        let s = "é".repeat(120);
        let out = preview_text(&s);
        assert!(out.ends_with('…'));
        // 80 'é' chars before the ellipsis — not 80 bytes worth.
        assert_eq!(out.chars().filter(|c| *c == 'é').count(), 80);
    }

    #[test]
    fn preview_handles_empty() {
        assert_eq!(preview_text(""), "");
    }
}
