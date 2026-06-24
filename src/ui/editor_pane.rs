use gpui::{App, Context, CursorStyle, Entity, IntoElement, MouseButton, Window, div, prelude::*, rems, rgb};

use crate::editor_element::EditorElement;
use crate::gutter::GutterElement;
use crate::scrollbar::{HorizontalScrollbarElement, SCROLLBAR_THICKNESS, VerticalScrollbarElement};
use crate::theme::Theme;
use crate::workspace::Workspace;

pub(crate) fn editor_pane(
    theme: &Theme,
    total_lines: usize,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    let prefs = crate::settings::snapshot();
    // Constant-width baseline = 56px = matches what 5 digits comfortably need
    // at the gutter's 12px font. Dynamic mode bumps the width per-digit beyond
    // 5 so 6-, 7-, 8-digit line counts still right-align without clipping.
    // Digit width ~7.2px at 12px font; rounded up to 8 for safe margin.
    let baseline_px = 56.0_f32;
    let per_extra_digit_px = 8.0_f32;
    let digit_count = digit_count_of(total_lines);
    let gutter_px = if prefs.line_number_width_dynamic && prefs.show_line_numbers {
        baseline_px + (digit_count.saturating_sub(5) as f32) * per_extra_digit_px
    } else {
        baseline_px
    };
    let gutter_width = rems(gutter_px / 16.);

    let gutter = div()
        .flex_none()
        .w(gutter_width)
        .relative()
        .overflow_hidden()
        .py(rems(10. / 16.))
        .pr(rems(12. / 16.))
        .bg(rgb(theme.gutter_bg))
        .border_r_1()
        .border_color(rgb(theme.border))
        .child(GutterElement {
            workspace: workspace.clone(),
        });

    let menu_workspace = workspace.clone();
    let content = div()
        .flex_1()
        .relative()
        .overflow_hidden()
        .py(rems(10. / 16.))
        .px(rems(12. / 16.))
        .cursor(CursorStyle::IBeam)
        .on_mouse_down(
            MouseButton::Left,
            workspace_listener(&workspace, Workspace::on_mouse_down),
        )
        .on_mouse_down(MouseButton::Right, move |event, _window, cx| {
            menu_workspace.update(cx, |w, cx| w.show_context_menu(event.position, cx));
        })
        .child(EditorElement {
            workspace: workspace.clone(),
        });

    let v_scrollbar = div()
        .flex_none()
        .w(rems(SCROLLBAR_THICKNESS / 16.))
        .relative()
        .child(VerticalScrollbarElement {
            workspace: workspace.clone(),
        });

    // Horizontal scrollbar lives below the editor row, spanning the full
    // width of the pane. The element itself is responsible for hiding when
    // word wrap is on or every line fits the viewport — at that point the
    // strip simply paints nothing, leaving a thin transparent gap. Its
    // height is reserved unconditionally so the editor body doesn't jitter
    // by `SCROLLBAR_THICKNESS` pixels every time the longest line crosses
    // the viewport-width threshold.
    let h_scrollbar = div()
        .flex_none()
        .h(rems(SCROLLBAR_THICKNESS / 16.))
        .relative()
        .child(HorizontalScrollbarElement {
            workspace: workspace.clone(),
        });

    let editor_row = div()
        .flex_1()
        .flex()
        .overflow_hidden()
        .child(gutter)
        .child(content)
        .child(v_scrollbar);

    div()
        .flex_1()
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(editor_row)
        .child(h_scrollbar)
}

fn digit_count_of(n: usize) -> u32 {
    if n == 0 {
        return 1;
    }
    let mut d = 0;
    let mut v = n;
    while v > 0 {
        d += 1;
        v /= 10;
    }
    d
}

fn workspace_listener<E: 'static>(
    workspace: &Entity<Workspace>,
    handler: fn(&mut Workspace, &E, &mut Window, &mut Context<Workspace>),
) -> impl Fn(&E, &mut Window, &mut App) + 'static {
    let workspace = workspace.clone();
    move |event, window, cx| {
        workspace.update(cx, |w, cx| handler(w, event, window, cx));
    }
}
