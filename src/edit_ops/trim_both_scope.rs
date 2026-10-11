// PowerEditor/src/NppCommands.cpp:2238–2245, Notepad_plus.cpp:1651–1704:
// trim both ends of the expanded selected physical lines, or the whole document.
use gpui::Context;

use crate::{tab::TabViewMode, workspace::Workspace};

pub(super) fn apply(ws: &mut Workspace, cx: &mut Context<Workspace>) {
    let Some(tab) = ws.tabs.get(ws.active) else {
        return;
    };
    if tab.block_selection.is_some() {
        return;
    }
    if tab.view_mode != TabViewMode::Edit {
        ws.set_status("Switch to Source or Raw View to trim leading and trailing space");
        cx.notify();
        return;
    }
    if tab.buffer.read_only {
        ws.set_status("Buffer is read-only");
        cx.notify();
        return;
    }
    let selection = tab.selection.clamped(&tab.buffer.rope);
    if selection.is_empty() {
        if !tab.selection.is_empty() {
            return;
        }
        super::run_full_buffer_transform(ws, cx, |s| Ok(super::trim_both(s)));
        return;
    }
    let range = super::trim_selected_line_range(&tab.buffer.rope, selection.ordered());
    let Some(slice) = tab.buffer.rope.get_byte_slice(range.clone()) else {
        return;
    };
    let mut out = String::new();
    if out
        .try_reserve(range.end.saturating_sub(range.start))
        .is_err()
    {
        ws.set_status("Trim Leading and Trailing Space skipped: not enough memory");
        cx.notify();
        return;
    }
    trim_into(slice.chars(), &mut out);
    if slice.chars().eq(out.chars()) {
        return;
    }
    ws.replace_byte_range(range.clone(), &out, cx);
    let tab = ws.active_tab_mut();
    tab.selection.anchor = range.start;
    tab.selection.head = range.start.saturating_add(out.len());
    tab.desired_col_x = None;
    ws.commit_caret(cx);
}

fn trim_into(chars: impl Iterator<Item = char>, out: &mut String) {
    let mut leading = true;
    let mut content_end = out.len();
    for ch in chars {
        if matches!(ch, '\r' | '\n') {
            out.truncate(content_end);
            out.push(ch);
            content_end = out.len();
            leading = true;
        } else if !leading || !matches!(ch, ' ' | '\t') {
            out.push(ch);
            leading = false;
            if !matches!(ch, ' ' | '\t') {
                content_end = out.len();
            }
        }
    }
    out.truncate(content_end);
}

#[cfg(test)]
mod tests {
    #[test]
    fn rope_transform_preserves_eols_unicode_and_interior_spaces() {
        for text in [" \tcafé  \r\n \t\r  x y\t\n", "  a\u{85}  b \t", " \t", ""] {
            let mut out = String::with_capacity(text.len());
            super::trim_into(ropey::Rope::from_str(text).chars(), &mut out);
            assert_eq!(out, super::super::trim_both(text));
        }
    }
}
