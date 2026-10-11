// Notepad++-style primitive widgets used by every Find-dialog tab. These are
// intentionally small and focused: just enough to land a Notepad++ look across
// `find_tab` / `replace_tab` / `fif_tab` / `mark_tab` without growing into a
// general-purpose component library.
//
// Conventions:
//   * Group boxes use a 1px border tinted with `theme.group_box_border` and a
//     small label that sits over the top edge — same visual language as Win32
//     dialogs.
//   * Checkboxes are 14px squares with a 1px border. Checked state fills the
//     box with `theme.checkbox_fill` and overlays a thin white check.
//   * Radios are 14px circles with the same accent fill plus an inner dot.
//   * Action buttons are rectangular with a 1px border and a light hover
//     background. No pill rounding; no toggle styling.
//
// Every interactive widget below uses `div().id(...)` so gpui treats it as a
// stateful element — that's what wires `on_mouse_down` (and any future hover
// styling) into the dispatch tree. Plain `div()` without an id silently drops
// click handlers in nested contexts; that's why the dialog widgets must opt
// in. Each `id` is the label string so we get free uniqueness within a tab.

use gpui::{
    AnyElement, App, CursorStyle, Entity, FocusHandle, MouseButton, SharedString, Window, deferred,
    div, prelude::*, px, rems, rgb,
};

use crate::settings::FindHistorySlot;
use crate::tab::InputFocus;
use crate::theme::Theme;
use crate::workspace::Workspace;

/// A bordered rectangle with a small text label that overlays the top border —
/// the classic Win32 "GroupBox" look. The caller supplies the inner content
/// via `body`. Pass an empty `title` for an untitled group (matches the
/// unlabeled "In selection / Count / Find All in Current Document" cluster
/// in Notepad++'s Find/Replace dialog).
pub(super) fn group_box(
    theme: &Theme,
    title: &'static str,
    body: AnyElement,
) -> impl IntoElement {
    // The label is rendered inside the top-left corner with a horizontal
    // padding so the border underneath appears to break for the text.
    //
    // `w_full()` makes the group stretch to fill its parent column. Without
    // this, GPUI's flex column does not auto-stretch children on the cross
    // axis — the group would size to its content's preferred width and, when
    // that exceeds the parent column (e.g. the unlabeled "In selection"
    // group whose two buttons are wider than the 300px right column), the
    // group bleeds out past the column boundary instead of clipping inside
    // it.
    let mut container = div()
        .relative()
        .mt(rems(8. / 16.))
        .w_full()
        .border_1()
        .border_color(rgb(theme.group_box_border))
        .pt(rems(10. / 16.))
        .pb(rems(8. / 16.))
        .px(rems(10. / 16.));

    if !title.is_empty() {
        container = container.child(
            // Title chip — absolute-positioned so it visually overlaps the
            // top border. Background matches the dialog body so the line
            // underneath the chip is hidden.
            div()
                .absolute()
                .top(px(-7.))
                .left(px(8.))
                .px(rems(4. / 16.))
                .bg(rgb(theme.bg))
                .text_size(rems(11. / 16.))
                .text_color(rgb(theme.chrome_text))
                .child(title),
        );
    }

    container.child(body)
}

/// A 14px checkbox with the supplied label rendered to its right. `on_change`
/// fires on left-mouse-down on either the box or the label. Stateful via
/// `id(label)` so gpui keeps the click handler wired across re-renders.
pub(super) fn checkbox<F>(
    theme: &Theme,
    label: &'static str,
    checked: bool,
    enabled: bool,
    on_change: F,
) -> impl IntoElement
where
    F: Fn(&mut Window, &mut App) + 'static,
{
    let label_color = if enabled {
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
    let row = div()
        .id(SharedString::from(format!("checkbox::{label}")))
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .h(rems(20. / 16.))
        .cursor(cursor)
        .child(checkbox_box(theme, checked, enabled))
        .child(
            div()
                .text_size(rems(11.5 / 16.))
                .text_color(rgb(label_color))
                .child(label),
        );

    if enabled {
        // Row hover is a click-affordance cue — like Win32 native
        // checkboxes, the row is the hit target, not just the box itself.
        // No "active" state here: a checkbox toggles on release.
        row.hover(move |s| s.bg(rgb(hover_bg)))
            .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                on_change(window, cx)
            })
            .into_any_element()
    } else {
        row.into_any_element()
    }
}

fn checkbox_box(theme: &Theme, checked: bool, enabled: bool) -> impl IntoElement {
    let bg: u32 = if checked && enabled {
        theme.checkbox_fill
    } else {
        theme.bg
    };
    let border: u32 = if enabled {
        theme.group_box_border
    } else {
        theme.disabled_text
    };
    let mut node = div()
        .w(px(14.))
        .h(px(14.))
        .border_1()
        .border_color(rgb(border))
        .bg(rgb(bg))
        .flex()
        .items_center()
        .justify_center();
    if checked {
        // The check itself: a thin "✓" rendered in white over the fill. The
        // glyph stays legible at 14px because we drop the line height to
        // match the box height exactly.
        node = node
            .text_size(px(10.))
            .line_height(px(12.))
            .text_color(rgb(0xffffff))
            .child("✓");
    }
    node
}

/// A 14px radio button with the supplied label. Visual match for the Search
/// Mode group's three options. Stateful for the same reason as `checkbox`.
pub(super) fn radio<F>(
    theme: &Theme,
    label: &'static str,
    selected: bool,
    enabled: bool,
    on_select: F,
) -> impl IntoElement
where
    F: Fn(&mut Window, &mut App) + 'static,
{
    let label_color = if enabled {
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
    let row = div()
        .id(SharedString::from(format!("radio::{label}")))
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .h(rems(20. / 16.))
        .cursor(cursor)
        .child(radio_dot(theme, selected, enabled))
        .child(
            div()
                .text_size(rems(11.5 / 16.))
                .text_color(rgb(label_color))
                .child(label),
        );
    if enabled {
        // Hover-only — radios commit on release like real OS controls.
        row.hover(move |s| s.bg(rgb(hover_bg)))
            .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                on_select(window, cx)
            })
            .into_any_element()
    } else {
        row.into_any_element()
    }
}

fn radio_dot(theme: &Theme, selected: bool, enabled: bool) -> impl IntoElement {
    let border: u32 = if enabled {
        theme.group_box_border
    } else {
        theme.disabled_text
    };
    let mut node = div()
        .w(px(14.))
        .h(px(14.))
        .rounded_full()
        .border_1()
        .border_color(rgb(border))
        .bg(rgb(theme.bg))
        .flex()
        .items_center()
        .justify_center();
    if selected {
        node = node.child(
            div()
                .w(px(6.))
                .h(px(6.))
                .rounded_full()
                .bg(rgb(if enabled {
                    theme.checkbox_fill
                } else {
                    theme.disabled_text
                })),
        );
    }
    node
}

/// Action button — rectangular, 1px border, hover lift, click inset.
/// Matches the Notepad++ Find/Replace dialog's button look. `primary` only
/// changes the border tint (Notepad++ subtly highlights the default button);
/// the body color stays the same so the row reads as a coherent stack.
/// Stateful so click handlers and hover/active styling reach this element
/// via the dispatch tree.
pub(super) fn dialog_action_button<F>(
    theme: &Theme,
    label: &'static str,
    primary: bool,
    enabled: bool,
    handler: F,
) -> impl IntoElement
where
    F: Fn(&mut Window, &mut App) + 'static,
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
    let node = div()
        .id(SharedString::from(format!("dialog_button::{label}")))
        .flex()
        .items_center()
        .justify_center()
        .h(rems(24. / 16.))
        .min_w(rems(160. / 16.))
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
            .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                handler(window, cx)
            })
            .into_any_element()
    } else {
        node.into_any_element()
    }
}

/// Compact action button used inside tight clusters where the standard
/// `dialog_action_button` 160px floor would overflow the column. Takes an
/// explicit `width` (no min_w floor) and lets the label wrap onto two lines
/// inside the requested width rather than overflowing or being truncated to
/// gibberish — matches Notepad++'s "Find All in Current Document" button,
/// which renders the label across two lines inside a button the same width
/// as its single-line `Count` neighbour.
///
/// Used by the Find/Replace tabs' unlabeled "In selection" group, where the
/// inner buttons share a row with the In-selection checkbox and have to fit
/// inside whatever space the right column has after accounting for the
/// checkbox width.
///
/// Wrapping mechanics: the label sits inside an inner `div().w_full()` whose
/// width is the button's content area (button width minus horizontal
/// padding). gpui's text element receives that div's resolved width as
/// `known_dimensions.width`, which becomes the line-wrap width for the
/// shaped text. Without this wrapper, the text element is a direct child of
/// the button's flex row and gpui's measurement pass uses the available
/// space (whole parent column) instead of the button width, so text doesn't
/// wrap and overflows the button on both sides because the button is
/// `justify_center`. `min_h` (instead of `h`) lets the button grow taller
/// when the label wraps to two lines, while a single-word label like
/// `Count` stays at the floor height — matching NPP, which lays its `Count`
/// button at one line and its `Find All in Current Document` button at two.
pub(super) fn dialog_action_button_compact<F>(
    theme: &Theme,
    label: &'static str,
    width: gpui::Rems,
    primary: bool,
    enabled: bool,
    handler: F,
) -> impl IntoElement
where
    F: Fn(&mut Window, &mut App) + 'static,
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
    let node = div()
        .id(SharedString::from(format!("dialog_button::{label}")))
        .flex()
        .items_center()
        .justify_center()
        .min_h(rems(24. / 16.))
        .w(width)
        .flex_shrink_0()
        .overflow_hidden()
        .px(rems(10. / 16.))
        .py(rems(2. / 16.))
        .bg(rgb(bg))
        .border_1()
        .border_color(rgb(border))
        .text_size(rems(11.5 / 16.))
        .text_color(rgb(fg))
        .cursor(cursor)
        .child(
            // Inner div pins the wrap-width to the button's content area
            // (`w_full` resolves against the button's inner box). Without
            // this wrapper, the text element falls back to the parent
            // flex's available space and renders unwrapped, overflowing
            // the centered button on both sides.
            div()
                .w_full()
                .text_center()
                .line_height(rems(14. / 16.))
                .child(label),
        );

    if enabled {
        node.hover(move |s| s.bg(rgb(hover_bg)))
            .active(move |s| s.bg(rgb(pressed_bg)))
            .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                handler(window, cx)
            })
            .into_any_element()
    } else {
        node.into_any_element()
    }
}

/// Bordered text input with a small history-dropdown chevron painted on the
/// right edge. The chevron is interactive when `history_slot` is `Some` —
/// clicking it toggles a recall dropdown anchored below the row.
///
/// `track_focus` on the input row anchors the input's `FocusHandle` into the
/// dispatch tree of whichever window paints this element. Without that, the
/// dialog window has no dispatch-tree node for `find_focus` (etc.), so when
/// the user clicks the input gpui sets `window.focus` but the next paint
/// never re-establishes the focusable node and `handle_input` registration
/// silently drops on subsequent frames.
pub(super) fn text_input_with_chevron(
    theme: &Theme,
    workspace: &Workspace,
    workspace_entity: Entity<Workspace>,
    focus_handle: FocusHandle,
    target: InputFocus,
    error: bool,
    placeholder: Option<String>,
) -> AnyElement {
    let slot = history_slot_for(target);
    let history_open =
        slot.map(|s| workspace.find_history_open == Some(s)).unwrap_or(false);
    let history_entries: Vec<String> = slot
        .map(|s| crate::settings::snapshot().find_history.slot(s).clone())
        .unwrap_or_default();

    let chevron_clickable = slot.is_some();
    let chevron_fg = if chevron_clickable {
        theme.text
    } else {
        theme.disabled_text
    };
    let chevron_cursor = if chevron_clickable {
        CursorStyle::PointingHand
    } else {
        CursorStyle::OperationNotAllowed
    };
    let chevron_hover_bg = theme.button_hover_bg;
    let chevron_id = SharedString::from(format!(
        "text_input_chevron::{}",
        input_focus_id(target)
    ));
    let mut chevron = div()
        .id(chevron_id)
        .flex()
        .items_center()
        .justify_center()
        .w(rems(18. / 16.))
        .h(rems(22. / 16.))
        .px(rems(2. / 16.))
        // Inputs use `chrome` (the tab-strip / dialog-header tone) instead of
        // `bg` so the text-field surface reads as a *sunken* control against
        // the dialog body — matches Notepad++'s field colouring.
        .bg(rgb(theme.chrome))
        .border_l_1()
        .border_color(rgb(theme.group_box_border))
        .text_size(rems(10. / 16.))
        .text_color(rgb(chevron_fg))
        .cursor(chevron_cursor)
        .child(if history_open { "^" } else { "v" });
    if let Some(history_slot) = slot {
        let ws = workspace_entity.clone();
        chevron = chevron
            .hover(move |s| s.bg(rgb(chevron_hover_bg)))
            .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                ws.update(cx, |w, cx| {
                    let next = if w.find_history_open == Some(history_slot) {
                        None
                    } else {
                        Some(history_slot)
                    };
                    w.find_history_open = next;
                    cx.notify();
                });
            });
    }

    let dropdown = match slot {
        Some(history_slot) if history_open && !history_entries.is_empty() => Some(
            history_dropdown(
                theme,
                workspace_entity.clone(),
                history_slot,
                history_entries,
                target,
            ),
        ),
        _ => None,
    };

    let mut row = div()
        .relative()
        .flex()
        .flex_row()
        .flex_1()
        .items_stretch()
        .border_1()
        .border_color(rgb(if error {
            theme.error
        } else {
            theme.group_box_border
        }))
        // Same `chrome` sunken-control tone as the chevron above.
        .bg(rgb(theme.chrome))
        .child(
            div()
                .id(SharedString::from(format!("text_input::{}", input_focus_id(target))))
                .track_focus(&focus_handle)
                .flex()
                .flex_1()
                .overflow_hidden()
                .items_center()
                .h(rems(22. / 16.))
                .px(rems(6. / 16.))
                .cursor(CursorStyle::IBeam)
                .on_mouse_down(MouseButton::Left, {
                    let ws = workspace_entity.clone();
                    move |_e, window, cx| {
                        ws.update(cx, |w, cx| w.focus_input(target, window, cx));
                    }
                })
                .child(crate::input_field::InputFieldElement {
                    workspace: workspace_entity.clone(),
                    target,
                    placeholder,
                    rename_prompt: None,
                }),
        )
        .child(chevron);
    if let Some(dropdown) = dropdown {
        row = row.child(deferred(dropdown).with_priority(2));
    }
    row.into_any_element()
}

/// Which history slot (if any) a given Find dialog input maps to. Goto,
/// MacroPrompt, TabRename, RichDoc inputs, etc. don't participate — their
/// `text_input_with_chevron` calls still render the chevron, just inert.
fn history_slot_for(target: InputFocus) -> Option<FindHistorySlot> {
    match target {
        InputFocus::FindQuery => Some(FindHistorySlot::FindQuery),
        InputFocus::Replacement => Some(FindHistorySlot::Replacement),
        InputFocus::FifQuery => Some(FindHistorySlot::FifQuery),
        InputFocus::FifReplacement => Some(FindHistorySlot::FifReplacement),
        InputFocus::FifFilter => Some(FindHistorySlot::FifFilter),
        InputFocus::FifDirectory => Some(FindHistorySlot::FifDirectory),
        InputFocus::MarkQuery => Some(FindHistorySlot::MarkQuery),
        _ => None,
    }
}

/// Recall dropdown anchored below the input row. Painted via `deferred(...)`
/// so it lands on top of every subsequent sibling regardless of paint order;
/// positioned with `.absolute()` relative to the row's `.relative()` parent.
/// The scrim covers the whole dialog so a click anywhere outside the panel
/// closes it cleanly without firing on the underlying control.
fn history_dropdown(
    theme: &Theme,
    workspace_entity: Entity<Workspace>,
    slot: FindHistorySlot,
    entries: Vec<String>,
    target: InputFocus,
) -> AnyElement {
    let scrim_ws = workspace_entity.clone();
    let scrim = div()
        .id(SharedString::from(format!(
            "text_input_history_scrim::{}",
            input_focus_id(target)
        )))
        .absolute()
        // The scrim is a window-sized rect anchored at the dialog's top-left
        // — gpui's coordinate space for `absolute()` inside a `relative()`
        // parent is the parent's box. To cover the whole dialog, pin into
        // the negative on both axes by a generous amount and stretch wide.
        // `min(left)=-9999px, w=20000px` is the standard "fill the world"
        // pattern documented in style_configurator/header.rs's overlay.
        .top(px(-9999.))
        .left(px(-9999.))
        .w(px(20000.))
        .h(px(20000.))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            scrim_ws.update(cx, |w, cx| {
                w.find_history_open = None;
                cx.notify();
            });
        });

    let mut panel = div()
        .id(SharedString::from(format!(
            "text_input_history_panel::{}",
            input_focus_id(target)
        )))
        .absolute()
        // Drop just under the input row's bottom border. Row height is
        // 22/16rem + 2px border = ~24/16rem; +1/16rem visual breathing room.
        .top(rems(23. / 16.))
        .left(px(0.))
        .min_w(rems(220. / 16.))
        .max_h(rems(220. / 16.))
        .overflow_y_scroll()
        .bg(rgb(theme.bg))
        .border_1()
        .border_color(rgb(theme.group_box_border))
        .flex()
        .flex_col()
        // Block clicks from reaching the scrim sibling so picking a row
        // registers cleanly without the scrim's close-on-click firing.
        .occlude();

    for (i, entry) in entries.into_iter().enumerate() {
        let row_id = SharedString::from(format!(
            "text_input_history_row::{}::{i}",
            input_focus_id(target)
        ));
        let ws = workspace_entity.clone();
        let entry_for_click = entry.clone();
        let label = if entry.is_empty() {
            SharedString::from("(empty)")
        } else {
            SharedString::from(entry)
        };
        let row = div()
            .id(row_id)
            .px(rems(8. / 16.))
            .py(rems(3. / 16.))
            .text_color(rgb(theme.text))
            .text_size(rems(11.5 / 16.))
            .cursor(CursorStyle::PointingHand)
            .hover(|hs| hs.bg(rgb(theme.button_hover_bg)))
            .child(label)
            .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                let value = entry_for_click.clone();
                ws.update(cx, |w, cx| {
                    w.apply_find_history_pick(slot, value, window, cx);
                });
            });
        panel = panel.child(row);
    }

    div()
        .absolute()
        .top(px(0.))
        .left(px(0.))
        .w(px(0.))
        .h(px(0.))
        .child(scrim)
        .child(panel)
        .into_any_element()
}

/// Stable, unique id-suffix for a given input target. Used by
/// `text_input_with_chevron` so each input gets a distinct stateful id and
/// gpui doesn't collapse them (which would erase per-input click state).
fn input_focus_id(target: InputFocus) -> &'static str {
    match target {
        InputFocus::FindQuery => "find_query",
        InputFocus::Replacement => "replacement",
        InputFocus::Goto => "goto",
        InputFocus::FifQuery => "fif_query",
        InputFocus::FifReplacement => "fif_replacement",
        InputFocus::FifFilter => "fif_filter",
        InputFocus::FifDirectory => "fif_directory",
        InputFocus::MarkQuery => "mark_query",
        InputFocus::MacroPrompt => "macro_prompt",
        InputFocus::TabRename => "tab_rename",
        InputFocus::RichDocLinkUrl => "rich_doc_link_url",
        InputFocus::RichDocColorHex => "rich_doc_color_hex",
        InputFocus::FolderInlineCreate => "folder_inline_create",
        InputFocus::Editor
        | InputFocus::RichDocBody
        | InputFocus::CsvCell
        | InputFocus::CsvGrid
        | InputFocus::CsvFindBar
        | InputFocus::CsvReplaceBar => "editor",
    }
}

/// Notepad++ paints a "▲" arrow next to `Find Next` — it's a one-shot
/// "Find Previous" momentary button (NOT a toggle). Clicking it performs a
/// single backward search regardless of the persistent `Backward direction`
/// checkbox in the left column. `selected` paints the pressed-in look (kept
/// in the signature for API symmetry with the rest of the toolbar widgets,
/// but in practice callers pass `false` since the button is momentary).
/// `enabled=true` lets the user click it.
pub(super) fn direction_arrow_button<F>(
    theme: &Theme,
    selected: bool,
    enabled: bool,
    on_click: F,
) -> AnyElement
where
    F: Fn(&mut Window, &mut App) + 'static,
{
    let border = if enabled {
        theme.group_box_border
    } else {
        theme.disabled_text
    };
    let bg = if selected && enabled {
        theme.checkbox_fill
    } else {
        theme.button_bg
    };
    let fg = if selected && enabled {
        0xffffff
    } else if enabled {
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
    let node = div()
        .id("direction_arrow")
        .flex()
        .items_center()
        .justify_center()
        .w(rems(22. / 16.))
        .h(rems(24. / 16.))
        .bg(rgb(bg))
        .border_1()
        .border_color(rgb(border))
        .text_size(rems(11. / 16.))
        .text_color(rgb(fg))
        .cursor(cursor)
        .child("▲");
    if enabled {
        // Same hover/press tones whether the toggle is selected or not. When
        // selected, the resting fill is the accent, and the standard hover
        // tone still reads as a clear delta against it; when unselected, the
        // resting fill is `button_bg` and hover/press follow the regular
        // action-button treatment.
        node.hover(move |s| s.bg(rgb(hover_bg)))
            .active(move |s| s.bg(rgb(pressed_bg)))
            .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                on_click(window, cx)
            })
            .into_any_element()
    } else {
        node.into_any_element()
    }
}

/// A wide labeled row: `Find what:` / `Replace with:` / `Filters:` /
/// `Directory:`. The label sits on the left at a fixed width so successive
/// rows align column-wise, matching the Notepad++ dialog grid.
pub(super) fn labeled_row(
    theme: &Theme,
    label: &'static str,
    body: AnyElement,
) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(8. / 16.))
        .child(
            div()
                .w(rems(80. / 16.))
                .flex_shrink_0()
                .text_size(rems(11.5 / 16.))
                .text_color(rgb(theme.text))
                .child(label),
        )
        .child(body)
}
