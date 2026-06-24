use gpui::{
    ClickEvent, Context, Entity, IntoElement, MouseButton, Render, SharedString, Window, div,
    prelude::*, point, px, rems, rgb,
};

use crate::settings;
use crate::theme::Theme;
use crate::workspace::Workspace;

/// Char-budget for the label segment of a tab. Sized to fit inside the
/// 220-rem max-w cap with 20px of side padding at a 12px font — wide
/// glyphs ('W', 'M') may still trail off but the truncation pre-empts the
/// pathological mid-glyph clipping artifact GPUI produces when the raw
/// string overflows its container.
const TAB_LABEL_MAX_CHARS: usize = 28;
/// Tail length for middle truncation — large enough to show ".docx"/
/// ".rs" plus a handful of stem chars (date stamps, counter suffixes) so
/// the user can still tell similar filenames apart at a glance.
const TAB_LABEL_TAIL_CHARS: usize = 10;

/// Middle-truncate a tab name to roughly fit the label cap. Mirrors
/// macOS Finder behaviour: keep the head of the stem, drop a horizontal
/// ellipsis, then keep the tail so the extension stays visible. Falls
/// back to a no-op when the name already fits — the call is essentially
/// free for normal-length filenames.
fn truncate_tab_label(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= TAB_LABEL_MAX_CHARS {
        return name.to_string();
    }
    let tail_n = TAB_LABEL_TAIL_CHARS.min(TAB_LABEL_MAX_CHARS / 2);
    let head_n = TAB_LABEL_MAX_CHARS.saturating_sub(tail_n + 1);
    let head: String = chars.iter().take(head_n).collect();
    let tail: String = chars.iter().skip(chars.len() - tail_n).collect();
    format!("{head}\u{2026}{tail}")
}

pub(crate) struct TabSummary {
    pub(crate) name: SharedString,
    pub(crate) dirty: bool,
    /// NPP "Apply Color to Tab" — `Some(1..=5)` paints a thin top stripe in
    /// the matching theme color; `None` leaves the tab unmarked.
    pub(crate) color: Option<u8>,
}

/// Payload carried during a tab drag. Holds the source index plus a
/// snapshot of the tab's display data so `Render` can paint a small
/// floating ghost that follows the cursor — without that visual the
/// drag feels broken even when reorder is wired correctly. Colour
/// fields are captured at drag-start (theme rarely changes mid-drag).
#[derive(Clone)]
pub(crate) struct DraggedTab {
    pub(crate) from: usize,
    pub(crate) label: SharedString,
    pub(crate) bg: u32,
    pub(crate) text: u32,
    pub(crate) border: u32,
}

impl Render for DraggedTab {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // Lightweight floating preview — same proportions as a real tab,
        // 1px border, slight inner padding, no close button. The cursor
        // sits at the top-left of this element, so the user sees the
        // tab "following" their pointer until they drop.
        div()
            .flex()
            .flex_row()
            .items_center()
            .h(rems(24. / 16.))
            .px(rems(12. / 16.))
            .bg(rgb(self.bg))
            .text_color(rgb(self.text))
            .border_1()
            .border_color(rgb(self.border))
            .text_size(rems(12. / 16.))
            .shadow_md()
            .child(self.label.clone())
    }
}

pub(crate) fn tab_strip(
    theme: &Theme,
    tabs: &[TabSummary],
    active: usize,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    let prefs = settings::snapshot();
    let strip_height = if prefs.tab_reduce {
        rems(20. / 16.)
    } else {
        rems(28. / 16.)
    };
    // Multi-line mode wraps tabs to additional rows once they overflow the
    // strip width. The row container then sizes to its (possibly multi-row)
    // content instead of being clipped at `strip_height` — each tab still
    // gets `h(strip_height)` explicitly so wrapped rows align cleanly.
    let mut row = div()
        .flex()
        .flex_row()
        .items_stretch()
        .bg(rgb(theme.chrome))
        .border_b_1()
        .border_color(rgb(theme.border));
    if prefs.tab_multi_line {
        row = row.flex_wrap();
    } else {
        row = row.h(strip_height);
    }

    for (idx, tab) in tabs.iter().enumerate() {
        let is_active = idx == active;
        let bg_color = if is_active || !prefs.tab_darken_inactive {
            theme.bg
        } else {
            theme.chrome
        };
        let text_color = if is_active {
            theme.text
        } else {
            theme.chrome_text
        };
        let dot = if tab.dirty { "● " } else { "" };
        let display_name = truncate_tab_label(&tab.name);
        let label = format!("{dot}{display_name}");
        let stripe_color = tab.color.and_then(|c| match c {
            1 => Some(theme.tab_color_1),
            2 => Some(theme.tab_color_2),
            3 => Some(theme.tab_color_3),
            4 => Some(theme.tab_color_4),
            5 => Some(theme.tab_color_5),
            _ => None,
        });

        let switch_handle = workspace.clone();
        let close_handle = workspace.clone();
        let drop_handle = workspace.clone();
        let menu_handle = workspace.clone();
        let dblclick_handle = workspace.clone();

        // Switch on press (on_mouse_down) — feels snappy and, contrary
        // to my earlier theory, does NOT kill drag-detection. GPUI
        // installs the drag tracker on press BEFORE custom mouse_down
        // handlers run, so the tracker survives the re-render that
        // `set_active_tab` + `cx.notify()` triggers. (`on_click` was
        // tempting because it self-suppresses during a drag, but
        // pairing it with `on_drag` made every natural press feel like
        // a near-drag and clicks stopped switching tabs at all.)
        let label_div = div()
            .flex()
            .items_center()
            .h_full()
            .pl(rems(12. / 16.))
            .pr(rems(8. / 16.))
            .min_w(rems(80. / 16.))
            .max_w(rems(220. / 16.))
            .overflow_hidden()
            .text_color(rgb(text_color))
            .text_size(rems(12. / 16.))
            .child(label)
            .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                switch_handle.update(cx, |w, cx| w.set_active_tab(idx, cx));
            });

        // Tab body — stack a thin color stripe above the content so the
        // NPP "Apply Color to Tab" stripe sits flush against the top edge.
        let mut body = div()
            .flex()
            .flex_col()
            .items_stretch()
            .h_full()
            .child(
                div()
                    .h(px(3.))
                    .when_some(stripe_color, |s, color| s.bg(rgb(color))),
            )
            .child({
                let mut row = div()
                    .flex()
                    .flex_row()
                    .items_stretch()
                    .flex_grow()
                    .child(label_div);
                if prefs.tab_close_button_visible {
                    row = row.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .h_full()
                            .w(rems(20. / 16.))
                            .pr(rems(8. / 16.))
                            .text_color(rgb(text_color))
                            .text_size(rems(13. / 16.))
                            .child("×")
                            .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                                close_handle.update(cx, |w, cx| w.close_tab_at(idx, window, cx));
                            }),
                    );
                }
                row
            })
            // NPP `Draw a coloured bar on active tab` — thin theme-accent line
            // along the bottom edge of the active tab. Inactive tabs render
            // a transparent placeholder so the strip baseline stays even.
            .child(
                div()
                    .h(px(2.))
                    .when(is_active && prefs.tab_active_bar, |s| {
                        s.bg(rgb(theme.toggle_active))
                    }),
            );
        // Reapply background — color stripe is on the inner column, but the
        // tab itself still needs the active/inactive bg.
        body = body.bg(rgb(bg_color));

        // `id(...)` makes the tab a stateful element so `on_drag` is allowed
        // — gpui requires a stable id to track the drag origin across
        // frames. SharedString id is unique per tab index within the strip.
        let mut tab_div = div()
            .id(SharedString::from(format!("tab-{idx}")))
            .flex()
            .flex_col()
            .items_stretch()
            .h(strip_height)
            .border_r_1()
            .border_color(rgb(theme.border))
            .child(body)
            .on_mouse_down(MouseButton::Right, move |e, _window, cx| {
                let pos = e.position;
                menu_handle.update(cx, |w, cx| {
                    // Anchor the menu just below the click point — matches
                    // Notepad++'s behaviour where the menu drops from where
                    // the cursor was when the right button went down.
                    w.show_tab_context_menu(idx, point(pos.x, pos.y), cx);
                });
            });

        // NPP `Lock (no drag and drop)` gates drag/drop registration entirely
        // — when locked, gpui never sees the on_drag start, so dragging a
        // tab does nothing.
        if !prefs.tab_lock_drag {
            let drag_label = SharedString::from(format!("{dot}{display_name}"));
            let drag_bg = bg_color;
            let drag_text = text_color;
            let drag_border = theme.border;
            tab_div = tab_div
                .on_drag(
                    DraggedTab {
                        from: idx,
                        label: drag_label,
                        bg: drag_bg,
                        text: drag_text,
                        border: drag_border,
                    },
                    |dragged, _offset, _w, cx| cx.new(|_| dragged.clone()),
                )
                .on_drop(move |dragged: &DraggedTab, _window, cx| {
                    let from = dragged.from;
                    drop_handle.update(cx, |w, cx| w.move_tab(from, idx, cx));
                });
        }

        // NPP `Double click to close document` — gpui's `on_click`
        // reports click_count, we treat ≥2 as a close request. Wired
        // separately from the press-side switch handler so a single
        // press still flips the active tab synchronously.
        if prefs.tab_double_click_to_close {
            tab_div = tab_div.on_click(move |e: &ClickEvent, window, cx| {
                if e.click_count() >= 2 {
                    dblclick_handle.update(cx, |w, cx| w.close_tab_at(idx, window, cx));
                }
            });
        }

        row = row.child(tab_div);
    }

    let new_handle = workspace.clone();
    let new_button = div()
        .flex()
        .items_center()
        .justify_center()
        .h(strip_height)
        .w(rems(28. / 16.))
        .text_color(rgb(theme.chrome_text))
        .text_size(rems(16. / 16.))
        .child("+")
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            new_handle.update(cx, |w, cx| w.new_tab(cx));
        });

    row.child(new_button)
}

/// Vertical tab strip — fixed-width column on the LEFT of the editor body.
/// Each tab is a full-width horizontal cell with the label + close button;
/// the color stripe runs along the LEFT edge (3px) and the active-tab
/// accent bar along the RIGHT edge (2px, pointing at the editor pane).
/// The `+` button sits at the bottom of the column.
///
/// Mirrors `tab_strip` 1:1 for behaviour: same press-to-switch, same drag
/// reorder, same right-click context menu, same close-button + double-
/// click-to-close gating. Only the visual orientation differs.
pub(crate) fn tab_strip_vertical(
    theme: &Theme,
    tabs: &[TabSummary],
    active: usize,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    let prefs = settings::snapshot();
    // Same per-row height as the horizontal strip — `tab_reduce` still
    // applies because users who picked a shorter horizontal strip almost
    // certainly want the same vertical density.
    let cell_height = if prefs.tab_reduce {
        rems(20. / 16.)
    } else {
        rems(28. / 16.)
    };
    // Fixed column width. ~180px matches the practical width of NPP's
    // vertical tab bar — long enough for typical filenames, short enough
    // to leave plenty of editor space on a 1280-wide window.
    let column_width = rems(180. / 16.);

    let mut col = div()
        .w(column_width)
        .h_full()
        .flex()
        .flex_col()
        .items_stretch()
        .bg(rgb(theme.chrome))
        .border_r_1()
        .border_color(rgb(theme.border));

    for (idx, tab) in tabs.iter().enumerate() {
        let is_active = idx == active;
        let bg_color = if is_active || !prefs.tab_darken_inactive {
            theme.bg
        } else {
            theme.chrome
        };
        let text_color = if is_active {
            theme.text
        } else {
            theme.chrome_text
        };
        let dot = if tab.dirty { "● " } else { "" };
        let display_name = truncate_tab_label(&tab.name);
        let label = format!("{dot}{display_name}");
        let stripe_color = tab.color.and_then(|c| match c {
            1 => Some(theme.tab_color_1),
            2 => Some(theme.tab_color_2),
            3 => Some(theme.tab_color_3),
            4 => Some(theme.tab_color_4),
            5 => Some(theme.tab_color_5),
            _ => None,
        });

        let switch_handle = workspace.clone();
        let close_handle = workspace.clone();
        let drop_handle = workspace.clone();
        let menu_handle = workspace.clone();
        let dblclick_handle = workspace.clone();

        let label_div = div()
            .flex()
            .items_center()
            .h_full()
            .flex_grow()
            .min_w(rems(0.))
            .pl(rems(8. / 16.))
            .pr(rems(4. / 16.))
            .overflow_hidden()
            .text_color(rgb(text_color))
            .text_size(rems(12. / 16.))
            .child(label)
            .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                switch_handle.update(cx, |w, cx| w.set_active_tab(idx, cx));
            });

        // Cell layout — flex_row with [stripe | inner_row | active_bar].
        // Inner row carries [label | close button]; the stripes flank it
        // so the cell still has a clean rectangular hit area.
        let inner_row = {
            let mut row = div()
                .flex()
                .flex_row()
                .items_stretch()
                .flex_grow()
                .min_w(rems(0.))
                .child(label_div);
            if prefs.tab_close_button_visible {
                row = row.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .h_full()
                        .w(rems(20. / 16.))
                        .pr(rems(8. / 16.))
                        .text_color(rgb(text_color))
                        .text_size(rems(13. / 16.))
                        .child("×")
                        .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                            close_handle.update(cx, |w, cx| w.close_tab_at(idx, window, cx));
                        }),
                );
            }
            row
        };

        let mut cell = div()
            .id(SharedString::from(format!("vtab-{idx}")))
            .flex()
            .flex_row()
            .items_stretch()
            .w_full()
            .h(cell_height)
            .border_b_1()
            .border_color(rgb(theme.border))
            .bg(rgb(bg_color))
            // Left-edge colour stripe (Apply Color to Tab) — transparent
            // placeholder otherwise so the inner row's left padding stays
            // consistent across coloured / uncoloured cells.
            .child(
                div()
                    .w(px(3.))
                    .h_full()
                    .when_some(stripe_color, |s, color| s.bg(rgb(color))),
            )
            .child(inner_row)
            // Right-edge accent bar — points at the editor pane. Inactive
            // tabs render a transparent placeholder so width stays uniform.
            .child(
                div()
                    .w(px(2.))
                    .h_full()
                    .when(is_active && prefs.tab_active_bar, |s| {
                        s.bg(rgb(theme.toggle_active))
                    }),
            )
            .on_mouse_down(MouseButton::Right, move |e, _window, cx| {
                let pos = e.position;
                menu_handle.update(cx, |w, cx| {
                    w.show_tab_context_menu(idx, point(pos.x, pos.y), cx);
                });
            });

        if !prefs.tab_lock_drag {
            let drag_label = SharedString::from(format!("{dot}{display_name}"));
            let drag_bg = bg_color;
            let drag_text = text_color;
            let drag_border = theme.border;
            cell = cell
                .on_drag(
                    DraggedTab {
                        from: idx,
                        label: drag_label,
                        bg: drag_bg,
                        text: drag_text,
                        border: drag_border,
                    },
                    |dragged, _offset, _w, cx| cx.new(|_| dragged.clone()),
                )
                .on_drop(move |dragged: &DraggedTab, _window, cx| {
                    let from = dragged.from;
                    drop_handle.update(cx, |w, cx| w.move_tab(from, idx, cx));
                });
        }

        if prefs.tab_double_click_to_close {
            cell = cell.on_click(move |e: &ClickEvent, window, cx| {
                if e.click_count() >= 2 {
                    dblclick_handle.update(cx, |w, cx| w.close_tab_at(idx, window, cx));
                }
            });
        }

        col = col.child(cell);
    }

    let new_handle = workspace.clone();
    let new_button = div()
        .flex()
        .items_center()
        .justify_center()
        .w_full()
        .h(cell_height)
        .text_color(rgb(theme.chrome_text))
        .text_size(rems(16. / 16.))
        .child("+")
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            new_handle.update(cx, |w, cx| w.new_tab(cx));
        });

    col.child(new_button)
}
