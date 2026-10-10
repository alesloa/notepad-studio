// Inline rich-doc color-picker bar — shown above the editor pane when the
// user clicks the toolbar Color button while a rich-doc block is active.
// Mirrors `tab_rename_bar` and `rich_doc_link_bar` in shape. The hex input
// is the same `InputFieldElement` pattern other inline bars use (so caret
// + selection + paste / IME all work the same way as Find / Goto).
// Every keystroke / swatch click live-applies through
// `Workspace::apply_rich_doc_color_buffer` — there is no separate Apply
// step. Clear strips the color and closes the bar.

use gpui::{CursorStyle, Entity, IntoElement, MouseButton, div, prelude::*, px, rems, rgb};

use crate::actions::EditFindClose;
use crate::find_state::RichDocColorBarState;
use crate::input_field::InputFieldElement;
use crate::tab::InputFocus;
use crate::theme::Theme;
use crate::workspace::Workspace;

/// 12 quick-pick swatches that cover the colors a user reaches for in
/// document body text (headings, callouts, link-style accents, dark grey
/// captions). Keeps the bar to one row — the wider hue × lightness grid
/// lives in `rich_doc_color_more.rs` behind the "More…" button.
const SWATCHES: &[u32] = &[
    0x000000, 0x404040, 0x808080, 0xC0C0C0, 0xFFFFFF, 0xCC0000, 0xCC6600, 0xCCCC00, 0x009900,
    0x0066CC, 0x9900CC, 0xCC0066,
];

pub(crate) fn rich_doc_color_bar(
    theme: &Theme,
    state: &RichDocColorBarState,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    let click_workspace = workspace.clone();
    let clear_workspace = workspace.clone();
    let close_workspace = workspace.clone();
    let new_color = parse_hex_local(&state.text);
    let was_color = state.original_color;

    // Hex input box.
    let hex_input = div()
        .flex()
        .w(rems(110. / 16.))
        .h(rems(22. / 16.))
        .px(rems(6. / 16.))
        .items_center()
        .bg(rgb(theme.bg))
        .border_1()
        .border_color(rgb(if new_color.is_none() && !state.text.is_empty() {
            theme.error
        } else {
            theme.border
        }))
        .text_size(rems(12. / 16.))
        .text_color(rgb(theme.text))
        .cursor(CursorStyle::IBeam)
        .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
            click_workspace.update(cx, |w, cx| {
                w.focus_input(InputFocus::RichDocColorHex, window, cx)
            });
        })
        .child(InputFieldElement {
            workspace: workspace.clone(),
            target: InputFocus::RichDocColorHex,
            placeholder: Some("RRGGBB".to_string()),
        });

    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .h(rems(30. / 16.))
        .px(rems(8. / 16.))
        .gap_2()
        .bg(rgb(theme.chrome))
        .border_b_1()
        .border_color(rgb(theme.border))
        .child(
            div()
                .text_size(rems(11. / 16.))
                .text_color(rgb(theme.chrome_text))
                .child("Color"),
        )
        .child(swatch_box(theme, new_color, "New"))
        .child(swatch_box(theme, was_color, "Was"))
        .child(hex_input);

    // Swatch row.
    let mut swatches = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(2.));
    for (i, rgb_val) in SWATCHES.iter().enumerate() {
        let entity = workspace.clone();
        let rgb_val = *rgb_val;
        let cell = div()
            .id(("rd-color-swatch", i))
            .w(rems(18. / 16.))
            .h(rems(18. / 16.))
            .bg(rgb(rgb_val))
            .border_1()
            .border_color(rgb(theme.border))
            .cursor(CursorStyle::PointingHand)
            .hover(|s| s.border_color(rgb(theme.text)))
            .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                entity.update(cx, |w, cx| {
                    w.rich_doc_color_pick_swatch(rgb_val, cx);
                });
            });
        swatches = swatches.child(cell);
    }
    row = row.child(swatches);

    // "More…" button + popover. `.relative()` on the wrapper anchors the
    // popover's `.absolute()` panel right under the button. The popover
    // itself is rendered as a sibling of the button so the chevron stays
    // clickable while the panel is open (clicking the chevron again
    // toggles the popover closed).
    let more_workspace = workspace.clone();
    let pressed = state.more_colors_open;
    let more_btn_bg = if pressed { theme.button_pressed_bg } else { theme.button_bg };
    let more_btn = div()
        .id("rd-color-more")
        .flex()
        .items_center()
        .justify_center()
        .h(rems(22. / 16.))
        .min_w(rems(56. / 16.))
        .px(rems(8. / 16.))
        .bg(rgb(more_btn_bg))
        .border_1()
        .border_color(rgb(theme.border))
        .text_color(rgb(theme.text))
        .text_size(rems(11. / 16.))
        .cursor(CursorStyle::PointingHand)
        .hover(|s| s.bg(rgb(theme.button_hover_bg)))
        .child("More\u{2026}")
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            more_workspace.update(cx, |w, cx| w.toggle_rich_doc_more_colors(cx));
        });
    let mut more_wrapper = div().relative().flex().items_center().child(more_btn);
    if state.more_colors_open {
        more_wrapper = more_wrapper.child(super::rich_doc_color_more::render(
            theme,
            workspace.clone(),
        ));
    }
    row = row.child(more_wrapper);

    // Clear button.
    let clear_btn = div()
        .id("rd-color-clear")
        .flex()
        .items_center()
        .justify_center()
        .h(rems(22. / 16.))
        .min_w(rems(54. / 16.))
        .px(rems(8. / 16.))
        .bg(rgb(theme.button_bg))
        .border_1()
        .border_color(rgb(theme.border))
        .text_color(rgb(theme.text))
        .text_size(rems(11. / 16.))
        .cursor(CursorStyle::PointingHand)
        .hover(|s| s.bg(rgb(theme.button_hover_bg)))
        .child("Clear")
        .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
            clear_workspace.update(cx, |w, cx| {
                w.rich_doc_color_clear(cx);
                w.close_find(&EditFindClose, window, cx);
            });
        });
    row = row.child(clear_btn);

    // × close.
    row = row.child(
        div()
            .id("rd-color-close")
            .flex()
            .items_center()
            .justify_center()
            .h(rems(22. / 16.))
            .w(rems(22. / 16.))
            .text_size(rems(13. / 16.))
            .text_color(rgb(theme.chrome_text))
            .cursor(CursorStyle::PointingHand)
            .child("×")
            .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                close_workspace.update(cx, |w, cx| {
                    w.close_rich_doc_color_bar(cx);
                    w.close_find(&EditFindClose, window, cx);
                });
            }),
    );

    row
}

fn swatch_box(theme: &Theme, color: Option<u32>, label: &'static str) -> impl IntoElement {
    let bg = match color {
        Some(c) => c,
        None => theme.bg,
    };
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(1.))
        .child(
            div()
                .text_size(rems(9. / 16.))
                .text_color(rgb(theme.disabled_text))
                .child(label),
        )
        .child(
            div()
                .w(rems(28. / 16.))
                .h(rems(1.))
                .bg(rgb(bg))
                .border_1()
                .border_color(rgb(theme.border)),
        )
}

fn parse_hex_local(s: &str) -> Option<u32> {
    if s.len() != 6 {
        return None;
    }
    u32::from_str_radix(s, 16).ok().map(|n| n & 0x00FF_FFFF)
}
