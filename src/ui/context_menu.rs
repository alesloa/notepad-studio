use gpui::{
    Corner, CursorStyle, Entity, IntoElement, MouseButton, Pixels, Point, anchored, deferred, div,
    prelude::*, rems, rgb,
};

use crate::actions::*;
use crate::theme::Theme;
use crate::workspace::Workspace;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Entry {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Delete,
    SelectAll,
    ToggleBookmark,
    GotoLine,
    Find,
    Sep,
}

impl Entry {
    fn label(self) -> &'static str {
        match self {
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::Cut => "Cut",
            Self::Copy => "Copy",
            Self::Paste => "Paste",
            Self::Delete => "Delete",
            Self::SelectAll => "Select All",
            Self::ToggleBookmark => "Toggle Bookmark",
            Self::GotoLine => "Go to Line…",
            Self::Find => "Find…",
            Self::Sep => "",
        }
    }
}

const ENTRIES: &[Entry] = &[
    Entry::Undo,
    Entry::Redo,
    Entry::Sep,
    Entry::Cut,
    Entry::Copy,
    Entry::Paste,
    Entry::Delete,
    Entry::Sep,
    Entry::SelectAll,
    Entry::Sep,
    Entry::ToggleBookmark,
    Entry::GotoLine,
    Entry::Find,
];

pub(crate) struct ContextMenuFlags {
    pub(crate) can_undo: bool,
    pub(crate) can_redo: bool,
    pub(crate) has_selection: bool,
}

pub(crate) fn context_menu(
    theme: &Theme,
    position: Point<Pixels>,
    flags: ContextMenuFlags,
    correction: Option<crate::workspace::proofreading::CorrectionMenu>,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    let dismiss_workspace = workspace.clone();
    let backdrop = div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .on_mouse_down(MouseButton::Left, move |_e, _window, cx| {
            dismiss_workspace.update(cx, |w, cx| w.hide_context_menu(cx));
        })
        .on_mouse_down(MouseButton::Right, {
            let dismiss_workspace = workspace.clone();
            move |_e, _window, cx| {
                dismiss_workspace.update(cx, |w, cx| w.hide_context_menu(cx));
            }
        });

    let bg = theme.chrome;
    let border = theme.border;
    let text = theme.text;
    let chrome_text = theme.chrome_text;
    let hover = theme.toggle_active;
    let sep_color = theme.border;

    let mut menu = div()
        .flex()
        .flex_col()
        .py(rems(4. / 16.))
        .min_w(rems(180. / 16.))
        .bg(rgb(bg))
        .border_1()
        .border_color(rgb(border))
        .text_size(rems(12. / 16.))
        .text_color(rgb(text));

    menu = crate::ui::proofreading_menu::append_rows(menu, theme, correction, workspace.clone());

    for entry in ENTRIES {
        let entry = *entry;
        if entry == Entry::Sep {
            menu = menu.child(
                div()
                    .h(rems(1. / 16.))
                    .my(rems(4. / 16.))
                    .mx(rems(8. / 16.))
                    .bg(rgb(sep_color)),
            );
            continue;
        }

        let enabled = match entry {
            Entry::Undo => flags.can_undo,
            Entry::Redo => flags.can_redo,
            Entry::Cut | Entry::Copy | Entry::Delete => flags.has_selection,
            _ => true,
        };

        let row_workspace = workspace.clone();
        let row_text = if enabled { text } else { chrome_text };

        let mut row = div()
            .flex()
            .items_center()
            .h(rems(22. / 16.))
            .px(rems(12. / 16.))
            .text_color(rgb(row_text))
            .child(entry.label());

        if enabled {
            row = row
                .cursor(CursorStyle::PointingHand)
                .hover(|s| s.bg(rgb(hover)).text_color(rgb(theme.bg)))
                .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                    row_workspace.update(cx, |w, cx| {
                        w.hide_context_menu(cx);
                        match entry {
                            Entry::Undo => w.undo(&EditUndo, window, cx),
                            Entry::Redo => w.redo(&EditRedo, window, cx),
                            Entry::Cut => w.cut(&EditCut, window, cx),
                            Entry::Copy => w.copy(&EditCopy, window, cx),
                            Entry::Paste => w.paste(&EditPaste, window, cx),
                            Entry::Delete => w.backspace(&EditBackspace, window, cx),
                            Entry::SelectAll => w.select_all(&EditSelectAll, window, cx),
                            Entry::ToggleBookmark => {
                                w.toggle_bookmark(&BookmarkToggle, window, cx)
                            }
                            Entry::GotoLine => w.open_goto_line(&EditGotoLine, window, cx),
                            Entry::Find => w.open_find(&EditFind, window, cx),
                            Entry::Sep => {}
                        }
                    });
                });
        }

        menu = menu.child(row);
    }

    deferred(
        div().absolute().top_0().left_0().size_full().child(
            div()
                .size_full()
                .child(backdrop)
                .child(
                    anchored()
                        .position(position)
                        .anchor(Corner::TopLeft)
                        .snap_to_window_with_margin(gpui::px(4.))
                        .child(menu),
                ),
        ),
    )
    .with_priority(2)
}
