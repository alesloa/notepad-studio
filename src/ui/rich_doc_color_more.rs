// "More colors…" popover for the rich-doc color bar.
//
// The bar's quick-pick row is intentionally short (12 swatches) so it can
// sit on a single line above the editor. Most users will reach further at
// some point — the gradient row covers reds/oranges/yellows/greens/teals/
// blues/purples but skips intermediate hues and lightness levels. This
// popover drops a larger hue × lightness grid + greyscale strip below the
// bar so the user can pick a finer-grained colour without rebuilding the
// entire colour picker UI. The Style Configurator's picker is the heavy-
// duty surface (HSV gradient + hue strip + swatch grid + hex input — see
// `style_configurator/hsv_surface.rs`); this is the lightweight in-bar
// companion.
//
// Layout: HUES (10) × LIGHTNESSES (6) plus a trailing greyscale column.
// Each cell click routes through `Workspace::rich_doc_color_pick_more_swatch`
// which applies the colour via the existing `apply_rich_doc_color_buffer`
// path and then closes the popover. The bar's hex input stays the
// source of truth — typing a hex still wins over the popover's last pick.
//
// Anchored to the More button via the `relative()` + `absolute()` pattern
// other dropdowns use (find_dialog history chevron, prefs default-lang
// dropdown). Painted in a `deferred(...).with_priority(2)` overlay so the
// panel lands on top of every sibling regardless of paint order. The
// scrim is a window-sized rect that catches outside clicks.

use gpui::{
    CursorStyle, Entity, IntoElement, MouseButton, ParentElement, SharedString, Styled, deferred,
    div, prelude::*, px, rems, rgb,
};

use crate::theme::Theme;
use crate::workspace::Workspace;

/// Hue ring — 10 evenly-spaced positions around the colour wheel. Wider
/// coverage than the quick-pick row (which only hits 7 hues) so the user
/// has cyan / teal / magenta within easy reach. Values are 0..1 (HSL).
const HUES: [f32; 10] = [
    0.0,             // red
    0.05,            // red-orange
    30.0 / 360.0,    // orange
    60.0 / 360.0,    // yellow
    100.0 / 360.0,   // lime
    140.0 / 360.0,   // green
    180.0 / 360.0,   // cyan
    220.0 / 360.0,   // blue
    270.0 / 360.0,   // purple
    315.0 / 360.0,   // magenta
];

/// Lightness levels — light to dark, skipping pure white/black (those
/// live in the greyscale column). 6 rows keeps the grid compact while
/// still covering pastel → vivid → deep accents.
const LIGHTNESSES: [f32; 6] = [0.90, 0.74, 0.58, 0.44, 0.30, 0.18];

/// Greyscale strip — pure white through pure black. One column on the
/// right of the hue grid so neutrals are one click away.
const GREYS: [u32; 7] = [
    0xFFFFFF, 0xE0E0E0, 0xC0C0C0, 0x808080, 0x606060, 0x303030, 0x000000,
];

const SWATCH_PX: f32 = 18.0;
const SWATCH_GAP_PX: f32 = 2.0;

pub(crate) fn render(theme: &Theme, workspace: Entity<Workspace>) -> gpui::AnyElement {
    let scrim_workspace = workspace.clone();
    // Window-sized rect that catches outside clicks. Generous negative
    // offsets + huge dimensions so any click in the workspace lands here
    // unless it's inside the panel itself (which `.occlude()`s).
    let scrim = div()
        .id("rd-more-scrim")
        .absolute()
        .top(px(-9999.))
        .left(px(-9999.))
        .w(px(20000.))
        .h(px(20000.))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            scrim_workspace.update(cx, |w, cx| w.close_rich_doc_more_colors(cx));
        });

    let mut panel = div()
        .id("rd-more-panel")
        .absolute()
        // Anchored just below the bar row (bar is 30/16 rem tall, this
        // popover drops 2/16 rem under so a thin gap separates the two).
        .top(rems(32. / 16.))
        // Right-align under the More button — the button is on the right
        // side of the bar, so anchor the popover's right edge to roughly
        // match. The relative parent is the More button wrapper, so
        // `right: 0` lines the popover's right edge with the button's.
        .right(px(0.))
        .p(rems(8. / 16.))
        .bg(rgb(theme.bg))
        .border_1()
        .border_color(rgb(theme.group_box_border))
        .flex()
        .flex_col()
        .gap(rems(6. / 16.))
        // Block clicks from punching through to the scrim — same trick as
        // the prefs default-lang dropdown. Without `.occlude()`, a swatch
        // click would fire both the cell's handler and the scrim's close
        // handler and the close would win on the next frame.
        .occlude();

    // HSV surface (SV gradient + hue strip) reused from Style
    // Configurator — same shared implementation Preferences ▸ Dark Mode
    // tones uses for slot-colour picking. Sits above the swatch grid so
    // the user can drag a free-form colour without having to type a hex.
    // Wired through `HsvHost for Workspace` — see `workspace/mod.rs`.
    let hsv_row = crate::ui::style_configurator::hsv_surface::render(
        theme,
        "rd-more-picker",
        &workspace,
    );
    panel = panel.child(hsv_row);

    // Hue × lightness grid plus the matching greyscale column on the
    // right. Built row by row so the greyscale entries line up with the
    // lightness rows.
    let grid = build_grid(theme, &workspace);
    panel = panel.child(grid);

    // Anchor wrapper — `.absolute()` panel needs a `.relative()` ancestor
    // for the right/top values to mean "relative to the More button".
    // That `.relative()` lives on the More button wrapper in
    // rich_doc_color_bar; this function just hands back the deferred
    // overlay.
    deferred(div().absolute().child(scrim).child(panel))
        .with_priority(2)
        .into_any_element()
}

fn build_grid(theme: &Theme, workspace: &Entity<Workspace>) -> gpui::AnyElement {
    let mut grid = div().flex().flex_col().gap(px(SWATCH_GAP_PX));

    for (row_idx, &lightness) in LIGHTNESSES.iter().enumerate() {
        let mut row = div().flex().flex_row().gap(px(SWATCH_GAP_PX));
        for (col_idx, &hue) in HUES.iter().enumerate() {
            let rgb_val = hsl_to_rgb(hue, 0.85, lightness);
            row = row.child(swatch_cell(
                theme,
                workspace,
                format!("rd-more-h{col_idx}-l{row_idx}"),
                rgb_val,
            ));
        }
        if let Some(&g) = GREYS.get(row_idx) {
            row = row.child(spacer());
            row = row.child(swatch_cell(
                theme,
                workspace,
                format!("rd-more-grey-{row_idx}"),
                g,
            ));
        }
        grid = grid.child(row);
    }
    // Trailing pure-black cell aligned under the greyscale column.
    let trailing = div()
        .flex()
        .flex_row()
        .justify_end()
        .child(swatch_cell(
            theme,
            workspace,
            "rd-more-grey-last".to_string(),
            *GREYS.last().unwrap(),
        ));
    grid = grid.child(trailing);
    grid.into_any_element()
}

fn swatch_cell(
    theme: &Theme,
    workspace: &Entity<Workspace>,
    id: String,
    rgb_val: u32,
) -> gpui::AnyElement {
    let workspace = workspace.clone();
    div()
        .id(SharedString::from(id))
        .w(px(SWATCH_PX))
        .h(px(SWATCH_PX))
        .bg(rgb(rgb_val))
        .border_1()
        .border_color(rgb(theme.group_box_border))
        .cursor(CursorStyle::PointingHand)
        .hover(|s| s.border_color(rgb(theme.text)))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            workspace.update(cx, |w, cx| {
                w.rich_doc_color_pick_more_swatch(rgb_val, cx);
            });
        })
        .into_any_element()
}

fn spacer() -> gpui::AnyElement {
    div().w(px(SWATCH_GAP_PX * 2.)).into_any_element()
}

/// HSL → packed 0xRRGGBB. Standard formula, mirrors the implementation in
/// `style_configurator::colour_picker::hsl_to_rgb`. Kept local so this
/// module is self-contained — no cross-module reach into a sibling's
/// private helper.
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> u32 {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h_prime = h * 6.0;
    let x = c * (1.0 - ((h_prime % 2.0) - 1.0).abs());
    let (r1, g1, b1) = match h_prime as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let r = ((r1 + m) * 255.0).round().clamp(0.0, 255.0) as u32;
    let g = ((g1 + m) * 255.0).round().clamp(0.0, 255.0) as u32;
    let b = ((b1 + m) * 255.0).round().clamp(0.0, 255.0) as u32;
    (r << 16) | (g << 8) | b
}
