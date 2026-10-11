// Folder-panel tree rendering. Phase 2/3.
//
// Layout: outer flex_row holds [panel | splitter]. The panel itself is a
// flex_col of header + scrollable tree body. Tree rows are flat divs with
// per-depth left padding — no nested overflow:hidden, so a deeply-indented
// row stays clickable.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use gpui::{
    AnyElement, App, Bounds, CursorStyle, DispatchPhase, Element, ElementId, Entity, FocusHandle,
    GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels,
    SharedString, Style, Styled, WeakEntity, div, prelude::*, px, rems, rgb,
};

use crate::find_state::FolderInlineCreateKind;
use crate::input_field::InputFieldElement;
use crate::settings;
use crate::tab::InputFocus;
use crate::theme::Theme;
use crate::workspace::Workspace;

use super::context_menu::RowKind;
use super::{DirEntry, cached_read_dir_sorted, expanded_set};

/// Snapshot of the inline-create draft passed into the tree render. Tree rows
/// compare `parent_dir` against their own path to decide whether to inject
/// the typing row before their children. Carrying the focus handle here lets
/// the typing row anchor it via `.track_focus()` so `handle_input` resolves
/// correctly even with other tree rows in the same dispatch parent.
#[derive(Clone)]
pub(crate) struct InlineCreateSnapshot {
    pub(crate) parent_dir: PathBuf,
    pub(crate) kind: FolderInlineCreateKind,
    pub(crate) focus: FocusHandle,
    pub(crate) error: Option<String>,
}

/// One line of a root's tree, in paint order.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TreeLine {
    /// A root, folder or file row. Roots always list as folders.
    Row {
        depth: u32,
        path: PathBuf,
        is_dir: bool,
        is_open: bool,
        is_root: bool,
    },
    /// The inline-create typing row, listed first under its parent folder.
    InlineCreate { depth: u32 },
}

/// Flatten `root` and its expanded descendants into rows, in the order the
/// panel paints them. `read_dir` lists one folder's children.
///
/// The walk keeps its own stack, so a very deep expanded tree can't
/// overflow the UI thread's stack, and opens each folder at most once, so
/// a folder that lists itself (a link loop) can't walk forever.
pub(crate) fn visible_rows(
    root: &Path,
    expanded: &HashSet<PathBuf>,
    inline_parent: Option<&Path>,
    read_dir: &mut dyn FnMut(&Path) -> Vec<DirEntry>,
) -> Vec<TreeLine> {
    let mut out = Vec::new();
    let is_open = expanded.contains(root);
    out.push(TreeLine::Row {
        depth: 0,
        path: root.to_path_buf(),
        is_dir: true,
        is_open,
        is_root: true,
    });
    if !is_open {
        return out;
    }
    let mut opened: HashSet<PathBuf> = HashSet::new();
    opened.insert(root.to_path_buf());
    let mut stack = vec![open_folder(root, 1, inline_parent, read_dir, &mut out)];
    while let Some((depth, children)) = stack.last_mut() {
        let depth = *depth;
        let Some(entry) = children.next() else {
            stack.pop();
            continue;
        };
        let is_open = entry.is_dir && expanded.contains(&entry.path);
        out.push(TreeLine::Row {
            depth,
            path: entry.path.clone(),
            is_dir: entry.is_dir,
            is_open,
            is_root: false,
        });
        if is_open && opened.insert(entry.path.clone()) {
            let child_depth = depth.saturating_add(1);
            stack.push(open_folder(&entry.path, child_depth, inline_parent, read_dir, &mut out));
        }
    }
    out
}

/// Start listing `dir`'s children at `depth`, after the inline-create row
/// when the draft targets this folder.
fn open_folder(
    dir: &Path,
    depth: u32,
    inline_parent: Option<&Path>,
    read_dir: &mut dyn FnMut(&Path) -> Vec<DirEntry>,
    out: &mut Vec<TreeLine>,
) -> (u32, std::vec::IntoIter<DirEntry>) {
    if inline_parent == Some(dir) {
        out.push(TreeLine::InlineCreate { depth });
    }
    (depth, read_dir(dir).into_iter())
}

const ROW_HEIGHT_REM: f32 = 22. / 16.;
const INDENT_PX_PER_DEPTH: f32 = 14.0;
const SPLITTER_WIDTH_PX: f32 = 4.0;
const HEADER_HEIGHT_REM: f32 = 22. / 16.;

/// Render the folder-as-workspace panel (with splitter on its right edge).
/// Caller mounts this inside a horizontal flex; the splitter takes its
/// own width so the panel proper measures `width` exactly. `selected`
/// drives the last-clicked-row highlight — caller passes a snapshot so
/// the render path doesn't have to read the workspace entity.
pub(crate) fn folder_panel(
    theme: &Theme,
    workspace: Entity<Workspace>,
    width: f32,
    selected: Option<PathBuf>,
    inline_create: Option<InlineCreateSnapshot>,
) -> impl IntoElement {
    let snap = settings::snapshot();
    let roots = snap.folder_panel_roots.clone();
    let expanded = expanded_set(&snap.folder_panel_expanded);
    let show_hidden = snap.folder_panel_show_hidden;

    let body: AnyElement = if roots.is_empty() {
        empty_state(theme).into_any_element()
    } else {
        let mut col = div()
            .id("folder-panel-tree")
            .flex()
            .flex_col()
            .flex_grow()
            .min_h(px(0.))
            .overflow_y_scroll();
        let inline_parent = inline_create.as_ref().map(|snap| snap.parent_dir.as_path());
        let mut read_dir = |dir: &Path| cached_read_dir_sorted(dir, show_hidden);
        for root in &roots {
            for line in visible_rows(root, &expanded, inline_parent, &mut read_dir) {
                if let Some(row) = render_line(
                    theme,
                    line,
                    selected.as_ref(),
                    workspace.clone(),
                    inline_create.as_ref(),
                ) {
                    col = col.child(row);
                }
            }
        }
        col.into_any_element()
    };

    let panel = div()
        .flex()
        .flex_col()
        .w(px(width))
        .h_full()
        .min_w(px(settings::FOLDER_PANEL_MIN_WIDTH))
        .max_w(px(settings::FOLDER_PANEL_MAX_WIDTH))
        .bg(rgb(theme.chrome))
        .text_color(rgb(theme.text))
        .text_size(rems(12. / 16.))
        .child(panel_header(theme, workspace.clone()))
        .child(body);

    let splitter = FolderPanelSplitter {
        workspace: workspace.downgrade(),
        theme_chrome: theme.chrome,
        theme_border: theme.border,
    };

    div()
        .flex()
        .flex_row()
        .h_full()
        .child(panel)
        .child(splitter)
}

fn panel_header(theme: &Theme, workspace: Entity<Workspace>) -> impl IntoElement {
    let close_btn = div()
        .flex()
        .items_center()
        .justify_center()
        .h(px(18.))
        .w(px(18.))
        .text_size(px(13.))
        .text_color(rgb(theme.chrome_text))
        .cursor(CursorStyle::PointingHand)
        .hover(|s| s.text_color(rgb(theme.text)))
        .child("×")
        .on_mouse_down(MouseButton::Left, move |_e, _window, cx| {
            crate::ui::folder_panel::set_visible(false);
            workspace.update(cx, |_, cx| cx.notify());
        });

    div()
        .flex()
        .items_center()
        .h(rems(HEADER_HEIGHT_REM))
        .pl(rems(8. / 16.))
        .pr(rems(4. / 16.))
        .bg(rgb(theme.chrome))
        .border_b_1()
        .border_color(rgb(theme.border))
        .text_color(rgb(theme.chrome_text))
        .text_size(rems(11. / 16.))
        .child(div().flex_grow().child("FOLDERS"))
        .child(close_btn)
}

fn empty_state(theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .flex_grow()
        .items_center()
        .justify_center()
        .px(rems(12. / 16.))
        .py(rems(12. / 16.))
        .text_color(rgb(theme.chrome_text))
        .text_size(rems(11. / 16.))
        .child(div().child("No folders yet."))
        .child(div().pt(rems(8. / 16.)).child("File ▸ Open Folder as"))
        .child(div().child("Workspace…"))
}

/// Render one flattened tree line. `None` for an inline-create line with no
/// draft to show.
fn render_line(
    theme: &Theme,
    line: TreeLine,
    selected: Option<&PathBuf>,
    workspace: Entity<Workspace>,
    inline_create: Option<&InlineCreateSnapshot>,
) -> Option<AnyElement> {
    match line {
        TreeLine::InlineCreate { depth } => {
            inline_create.map(|snap| inline_create_row(theme, depth, snap, workspace))
        }
        TreeLine::Row {
            depth,
            path,
            is_dir,
            is_open,
            is_root,
        } => {
            let label = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            let is_selected = selected.map(|p| p == &path).unwrap_or(false);
            let kind = if is_root {
                RowKind::Root
            } else if is_dir {
                RowKind::Folder
            } else {
                RowKind::File
            };
            Some(
                tree_row(theme, depth, is_dir, is_open, is_selected, &label, path, kind, workspace)
                    .into_any_element(),
            )
        }
    }
}

/// Render the inline-create draft row: chevron blank, icon matches the
/// draft kind, custom text input wired through `InputFieldElement` so typing
/// flows via `Workspace::replace_text_in_range` for the `FolderInlineCreate`
/// target. The wrapping div uses `.id()` + `.track_focus()` so the focus
/// handle anchors into the dispatch tree and `handle_input` registration
/// survives the next paint (GPUI focus-handle caveat).
fn inline_create_row(
    theme: &Theme,
    depth: u32,
    snap: &InlineCreateSnapshot,
    workspace: Entity<Workspace>,
) -> AnyElement {
    let indent_px = px(depth as f32 * INDENT_PX_PER_DEPTH);
    let icon = match snap.kind {
        FolderInlineCreateKind::File => "📄",
        FolderInlineCreateKind::Folder => "📁",
    };
    let placeholder = match snap.kind {
        FolderInlineCreateKind::File => "new-file.txt",
        FolderInlineCreateKind::Folder => "new-folder",
    };
    let ws_click = workspace.clone();
    let row = div()
        .flex()
        .items_center()
        .h(rems(ROW_HEIGHT_REM))
        .pl(indent_px)
        .pr(rems(4. / 16.))
        .text_color(rgb(theme.text))
        .child(
            div()
                .w(rems(14. / 16.))
                .text_color(rgb(theme.chrome_text))
                .child(" "),
        )
        .child(div().w(rems(20. / 16.)).child(icon))
        .child(
            div()
                .id(SharedString::from("folder-panel-inline-create-input"))
                .debug_selector(|| "folder-draft-input".into())
                .track_focus(&snap.focus)
                .flex()
                .flex_1()
                .items_center()
                .h(rems(ROW_HEIGHT_REM))
                .px(rems(4. / 16.))
                .bg(rgb(theme.bg))
                .border_1()
                .border_color(rgb(theme.toggle_active))
                .cursor(CursorStyle::IBeam)
                .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                    ws_click.update(cx, |w, cx| {
                        w.focus_input(InputFocus::FolderInlineCreate, window, cx)
                    });
                })
                .child(InputFieldElement {
                    workspace: workspace.clone(),
                    target: InputFocus::FolderInlineCreate,
                    placeholder: Some(placeholder.to_string()),
                    rename_prompt: None,
                }),
        );

    let row = if let Some(err) = snap.error.as_ref() {
        // Stack the error message under the input — same depth padding,
        // disabled-text-style so it reads as supplementary feedback.
        div()
            .flex()
            .flex_col()
            .child(row)
            .child(
                div()
                    .pl(indent_px + px(34.))
                    .pr(rems(4. / 16.))
                    .text_size(rems(10.5 / 16.))
                    .text_color(rgb(theme.error))
                    .child(err.clone()),
            )
            .into_any_element()
    } else {
        row.into_any_element()
    };
    row
}

#[allow(clippy::too_many_arguments)]
fn tree_row(
    theme: &Theme,
    depth: u32,
    is_dir: bool,
    is_open: bool,
    is_selected: bool,
    label: &str,
    path: PathBuf,
    kind: RowKind,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    let chevron = if is_dir {
        if is_open { "▾" } else { "▸" }
    } else {
        " "
    };
    let icon = if is_dir { "📁" } else { "📄" };
    let indent_px = px(depth as f32 * INDENT_PX_PER_DEPTH);
    let label_str = label.to_string();
    let row_path_click = path.clone();
    let row_path_ctx = path.clone();
    let ws_click = workspace.clone();
    let ws_ctx = workspace.clone();

    let mut row = div()
        .flex()
        .items_center()
        .h(rems(ROW_HEIGHT_REM))
        .pl(indent_px)
        .pr(rems(4. / 16.))
        .text_color(rgb(theme.text))
        .child(
            div()
                .w(rems(14. / 16.))
                .text_color(rgb(theme.chrome_text))
                .child(chevron),
        )
        .child(div().w(rems(20. / 16.)).child(icon))
        .child(div().flex_grow().child(label_str))
        .cursor(CursorStyle::PointingHand)
        .hover(|s| s.bg(rgb(theme.toggle_active)).text_color(rgb(theme.bg)))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
            let p = row_path_click.clone();
            ws_click.update(cx, |w, cx| {
                w.handle_folder_panel_click(p, is_dir, kind, cx);
            });
        })
        .on_mouse_down(
            MouseButton::Right,
            move |e: &MouseDownEvent, _w, cx| {
                let pos = e.position;
                let p = row_path_ctx.clone();
                ws_ctx.update(cx, |w, cx| {
                    w.show_folder_panel_context_menu(p, kind, pos, cx);
                });
            },
        );

    if is_selected {
        row = row.bg(rgb(theme.toggle_active)).text_color(rgb(theme.bg));
    }
    row
}

/// Vertical splitter between the folder panel and the editor body. Custom
/// `Element` so its `paint` can register window-global mouse handlers —
/// without those, a drag would cancel the moment the cursor left the
/// 4-px hit area. Mirrors `PreviewPaneResizeHandle` in `text_tools_preview_panel`.
struct FolderPanelSplitter {
    workspace: WeakEntity<Workspace>,
    theme_chrome: u32,
    theme_border: u32,
}

impl IntoElement for FolderPanelSplitter {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for FolderPanelSplitter {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut gpui::Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = px(SPLITTER_WIDTH_PX).into();
        style.size.height = gpui::relative(1.).into();
        style.flex_shrink = 0.;
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut gpui::Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut gpui::Window,
        _cx: &mut App,
    ) {
        window.set_cursor_style(CursorStyle::ResizeLeftRight, hitbox);
        window.paint_quad(gpui::fill(bounds, rgb(self.theme_chrome)));
        let left_line = Bounds::new(
            bounds.origin,
            gpui::size(px(1.), bounds.size.height),
        );
        window.paint_quad(gpui::fill(left_line, rgb(self.theme_border)));

        let ws_for_down = self.workspace.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            if !bounds.contains(&event.position) {
                return;
            }
            let _ = ws_for_down.update(cx, |w, cx| {
                w.begin_folder_panel_drag(event.position.x, cx);
            });
        });

        let ws_for_move = self.workspace.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let _ = ws_for_move.update(cx, |w, cx| {
                if w.folder_panel.drag.is_some() {
                    w.drag_folder_panel(event.position.x, cx);
                }
            });
        });

        let ws_for_up = self.workspace.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            let _ = ws_for_up.update(cx, |w, cx| {
                w.end_folder_panel_drag(cx);
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(path: &str) -> DirEntry {
        DirEntry {
            path: PathBuf::from(path),
            is_dir: true,
        }
    }

    fn file(path: &str) -> DirEntry {
        DirEntry {
            path: PathBuf::from(path),
            is_dir: false,
        }
    }

    fn row(depth: u32, path: &str, is_dir: bool, is_open: bool) -> TreeLine {
        TreeLine::Row {
            depth,
            path: PathBuf::from(path),
            is_dir,
            is_open,
            is_root: false,
        }
    }

    #[test]
    fn rows_follow_paint_order() {
        let expanded: HashSet<PathBuf> = ["r", "r/a"].iter().map(PathBuf::from).collect();
        let mut read_dir = |p: &Path| match p.to_str() {
            Some("r") => vec![dir("r/a"), dir("r/b"), file("r/z.txt")],
            Some("r/a") => vec![file("r/a/x.txt")],
            _ => Vec::new(),
        };
        let rows = visible_rows(Path::new("r"), &expanded, Some(Path::new("r/a")), &mut read_dir);
        assert_eq!(
            rows,
            vec![
                TreeLine::Row {
                    depth: 0,
                    path: PathBuf::from("r"),
                    is_dir: true,
                    is_open: true,
                    is_root: true,
                },
                row(1, "r/a", true, true),
                TreeLine::InlineCreate { depth: 2 },
                row(2, "r/a/x.txt", false, false),
                row(1, "r/b", true, false),
                row(1, "r/z.txt", false, false),
            ]
        );
    }

    #[test]
    fn collapsed_root_lists_only_itself() {
        let mut read_dir = |_: &Path| vec![file("r/x")];
        let rows = visible_rows(Path::new("r"), &HashSet::new(), None, &mut read_dir);
        assert_eq!(rows.len(), 1);
    }

    /// A 100k-deep chain of expanded folders, as a symlink loop or a
    /// generated tree can produce. The walk must not grow the stack with
    /// depth.
    #[test]
    fn very_deep_tree_does_not_overflow_the_stack() {
        const DEPTH: usize = 100_000;
        let expanded: HashSet<PathBuf> =
            (0..=DEPTH).map(|n| PathBuf::from(format!("d{n}"))).collect();
        let mut read_dir = |p: &Path| {
            let n = p
                .to_str()
                .and_then(|s| s.strip_prefix('d'))
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(DEPTH);
            if n < DEPTH { vec![dir(&format!("d{}", n + 1))] } else { Vec::new() }
        };
        let rows = visible_rows(Path::new("d0"), &expanded, None, &mut read_dir);
        assert_eq!(rows.len(), DEPTH + 1);
    }

    /// A folder listing itself (a self-referencing link) lists once and
    /// stops instead of walking forever.
    #[test]
    fn self_listing_folder_terminates() {
        let expanded: HashSet<PathBuf> = [PathBuf::from("loop")].into_iter().collect();
        let mut read_dir = |_: &Path| vec![dir("loop")];
        let rows = visible_rows(Path::new("loop"), &expanded, None, &mut read_dir);
        assert_eq!(rows.len(), 2);
    }
}
