// A painted Source input handler belongs to one uninterrupted document,
// caret and focus context. Retained native callbacks must not follow the
// Workspace into a newer tab or field.
use std::{ops::Range, rc::Rc};
use gpui::{App, Bounds, Context, ElementInputHandler, Entity, FocusHandle, InputContextToken, InputHandler, Pixels, Point, UTF16Selection, Window};
use uuid::Uuid;
use crate::{buffer::ContentRevision, tab::{InputFocus, TabViewMode}};
use super::Workspace;

// UI-owned context identity, shared with sibling editor guards.
#[derive(Clone, Debug)]
pub(super) struct SourceInputToken(Rc<()>);

impl PartialEq for SourceInputToken {
    fn eq(&self, other: &Self) -> bool { Rc::ptr_eq(&self.0, &other.0) }
}

impl Eq for SourceInputToken {}

impl Default for SourceInputToken {
    fn default() -> Self { Self(Rc::new(())) }
}

pub(crate) struct SourceInputContext {
    epoch: SourceInputToken,
    tab: Option<Uuid>,
    caret: Option<SourceCaret>,
    pending_edit_caret: bool,
}

impl SourceInputContext {
    pub(super) fn new(tab: Uuid) -> Self { Self { epoch: Default::default(), tab: Some(tab), caret: None, pending_edit_caret: false } }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct SourceCaret {
    tab: Uuid,
    selection: (usize, usize, bool),
    block: Option<(usize, usize, usize, usize, bool)>,
}

impl SourceCaret {
    fn capture(w: &Workspace) -> Option<Self> {
        let tab = w.tabs.get(w.active)?;
        Some(Self {
            tab: tab.session_id,
            selection: (tab.selection.anchor, tab.selection.head, tab.selection.is_dragging),
            block: tab.block_selection.map(|b| (b.anchor_line, b.anchor_col, b.head_line, b.head_col, b.is_dragging)),
        })
    }
}

impl Workspace {
    pub(super) fn normalize_saved_csv_source_view(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return; };
        // Save As can change a clean grid's path to .txt. The renderer then
        // displays Source; make its mode agree before focus and input paint.
        if tab.view_mode != TabViewMode::CsvPreview
            || crate::ui::csv_doc::is_csv_path(tab.buffer.file_path.as_deref())
            || tab.csv_draft.is_some()
            || self.csv_models.get(&tab.session_id).is_some_and(|model| model.cell_input.is_some())
        { return; }
        let Some(tab) = self.tabs.get_mut(self.active) else { return; };
        tab.view_mode = TabViewMode::Edit;
        self.retire_source_input();
        self.close_autocomplete(cx);
        self.close_function_hint(cx);
    }

    pub(crate) fn retire_source_input(&mut self) {
        self.source_input_context.epoch = SourceInputToken::default();
        self.source_input_context.caret = SourceCaret::capture(self);
        self.source_input_context.pending_edit_caret = false;
    }

    pub(super) fn retire_source_input_after_edit(&mut self) {
        self.retire_source_input();
        self.source_input_context.pending_edit_caret = true;
    }

    pub(crate) fn settle_source_input_edit_caret(&mut self) {
        if self.source_input_context.pending_edit_caret {
            // Some edits correct their selection after the content flush.
            // Older callbacks are already retired; observe the correction
            // before comparing the next genuine movement, never after it.
            self.source_input_context.caret = SourceCaret::capture(self);
            self.source_input_context.pending_edit_caret = false;
        }
    }

    pub(crate) fn sync_source_input_caret(&mut self) {
        let current = SourceCaret::capture(self);
        if self.source_input_context.caret.is_some() && self.source_input_context.caret != current {
            self.retire_source_input();
        } else {
            // The welcome/restored tab has no observed caret until first paint.
            self.source_input_context.caret = current;
        }
    }

    pub(super) fn source_input_token(&self) -> SourceInputToken {
        self.source_input_context.epoch.clone()
    }

    pub(crate) fn sync_source_input_tab(&mut self) {
        let tab = self.tabs.get(self.active).map(|tab| tab.session_id);
        if self.source_input_context.tab != tab {
            self.retire_source_input();
            self.source_input_context.tab = tab;
        }
        self.settle_source_input_edit_caret();
        self.sync_source_input_caret();
    }
}

struct SourceStamp {
    tab: Uuid,
    epoch: SourceInputToken,
    revision: ContentRevision,
    source: ropey::Rope,
    selection: (usize, usize, bool),
    block: Option<(usize, usize, usize, usize, bool)>,
    focus: InputContextToken,
}

impl SourceStamp {
    fn capture(w: &Workspace, window: &Window) -> Option<Self> {
        let tab = w.tabs.get(w.active)?;
        if !window.is_window_active() || !w.focus_handle.is_focused(window)
            || tab.view_mode != TabViewMode::Edit || w.input_focus(window) != InputFocus::Editor
            || w.bulk_edit || tab.edit_transaction.is_some()
        { return None; }
        Some(Self {
            tab: tab.session_id, epoch: w.source_input_token(),
            revision: tab.buffer.raw_revision.clone(), source: tab.buffer.rope.clone(),
            selection: (tab.selection.anchor, tab.selection.head, tab.selection.is_dragging),
            block: tab.block_selection.map(|block| (block.anchor_line, block.anchor_col, block.head_line, block.head_col, block.is_dragging)),
            focus: window.input_context_token(),
        })
    }

    fn matches(&self, w: &Workspace, window: &Window) -> bool {
        let Some(current) = Self::capture(w, window) else { return false; };
        self.tab == current.tab && self.epoch == current.epoch
            && self.revision == current.revision && self.selection == current.selection
            && self.source.is_instance(&current.source) && self.block == current.block && self.focus == current.focus
    }
}

pub(crate) struct SourceInputHandler {
    inner: ElementInputHandler<Workspace>,
    workspace: Entity<Workspace>,
    focus: FocusHandle,
    stamp: Option<SourceStamp>,
}

impl SourceInputHandler {
    pub(crate) fn new(bounds: Bounds<Pixels>, workspace: Entity<Workspace>, window: &Window, cx: &mut App) -> Self {
        workspace.update(cx, |w, _| w.sync_source_input_tab());
        let w = workspace.read(cx);
        Self {
            inner: ElementInputHandler::new(bounds, workspace.clone()),
            focus: w.focus_handle.clone(), stamp: SourceStamp::capture(w, window), workspace,
        }
    }

    fn is_current(&self, window: &Window, cx: &App) -> bool {
        let w = self.workspace.read(cx);
        self.focus == w.focus_handle && self.stamp.as_ref().is_some_and(|stamp| stamp.matches(w, window))
    }

    fn can_edit(&self, window: &Window, cx: &App) -> bool {
        if !self.is_current(window, cx) { return false; }
        let w = self.workspace.read(cx);
        w.tabs.get(w.active).is_some_and(|tab| !tab.buffer.read_only && tab.buffer.loading.is_none())
    }

    fn refresh(&mut self, window: &Window, cx: &mut App) {
        // The delegate's own edit/caret flush retires old callbacks. Adopt
        // that new identity only after this handler accepted genuine input.
        self.workspace.update(cx, |w, _| {
            w.settle_source_input_edit_caret();
            w.sync_source_input_caret();
        });
        self.stamp = SourceStamp::capture(self.workspace.read(cx), window);
    }
}

impl InputHandler for SourceInputHandler {
    fn selected_text_range(&mut self, ignore_disabled: bool, window: &mut Window, cx: &mut App) -> Option<UTF16Selection> {
        if !self.is_current(window, cx) { return None; }
        self.inner.selected_text_range(ignore_disabled, window, cx)
    }

    fn marked_text_range(&mut self, window: &mut Window, cx: &mut App) -> Option<Range<usize>> {
        if !self.is_current(window, cx) { return None; }
        self.inner.marked_text_range(window, cx)
    }

    fn text_for_range(&mut self, range: Range<usize>, adjusted: &mut Option<Range<usize>>, window: &mut Window, cx: &mut App) -> Option<String> {
        if !self.is_current(window, cx) { return None; }
        self.inner.text_for_range(range, adjusted, window, cx)
    }

    fn replace_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, window: &mut Window, cx: &mut App) {
        if !self.can_edit(window, cx) { return; }
        self.inner.replace_text_in_range(range, text, window, cx);
        self.refresh(window, cx);
    }

    fn replace_and_mark_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, selection: Option<Range<usize>>, window: &mut Window, cx: &mut App) {
        if !self.can_edit(window, cx) { return; }
        self.inner.replace_and_mark_text_in_range(range, text, selection, window, cx);
        self.refresh(window, cx);
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut App) {
        if !self.can_edit(window, cx) { return; }
        self.inner.unmark_text(window, cx);
        self.refresh(window, cx);
    }

    fn bounds_for_range(&mut self, range: Range<usize>, window: &mut Window, cx: &mut App) -> Option<Bounds<Pixels>> {
        if !self.is_current(window, cx) { return None; }
        self.inner.bounds_for_range(range, window, cx)
    }

    fn character_index_for_point(&mut self, point: Point<Pixels>, window: &mut Window, cx: &mut App) -> Option<usize> {
        if !self.is_current(window, cx) { return None; }
        self.inner.character_index_for_point(point, window, cx)
    }

    fn accepts_text_input(&mut self, window: &mut Window, cx: &mut App) -> bool {
        self.can_edit(window, cx) && self.inner.accepts_text_input(window, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{actions::{EditUndo, TabGoto2, ViewCsvPreview}, editor_element::source_input_origin_tests::with_sources};
    use gpui::VisualContext;

    #[test]
    fn source_caret_snapshot_has_fixed_storage_and_does_not_dereference_offsets() {
        assert!(std::mem::size_of::<SourceCaret>() <= 128);
        with_sources(|entity, visual| {
            visual.update_window_entity(entity, |w, _, _| {
                let selection = w.active_tab_mut().selection;
                let block = w.block_selection();
                w.active_tab_mut().selection = crate::buffer::Selection {
                    anchor: usize::MAX, head: 7, is_dragging: true,
                };
                w.set_block_selection(Some(crate::buffer::BlockSelection {
                    anchor_line: usize::MAX, anchor_col: usize::MAX, head_line: usize::MAX,
                    head_col: usize::MAX - 1, is_dragging: true,
                }));
                let state = SourceCaret::capture(w).unwrap();
                assert_eq!(state.selection, (usize::MAX, 7, true));
                assert_eq!(state.block, Some((usize::MAX, usize::MAX, usize::MAX, usize::MAX - 1, true)));
                let epoch = w.source_input_token();
                w.sync_source_input_caret(); assert_eq!(w.source_input_token(), epoch);
                w.active_tab_mut().selection = selection;
                w.set_block_selection(block); w.sync_source_input_caret();
            });
        });
    }

    fn draft_boundary(restored: bool) {
        with_sources(|entity, visual| {
            let directory = crate::persist::TempDir::new("source-csv-view-boundary");
            let path = directory.join("neutral.csv");
            std::fs::write(&path, "label,value\r\nseed,1\r\n").unwrap();
            visual.update_window_entity(entity, |w, _, cx| { w.tabs[1].buffer = crate::io::read_buffer(&path).unwrap(); cx.notify(); });
            visual.dispatch_action(TabGoto2);
            visual.run_until_parked();
            visual.simulate_input("source_");
            visual.dispatch_action(EditUndo);
            visual.dispatch_action(ViewCsvPreview);
            visual.run_until_parked();
            visual.update_window_entity(entity, crate::ui::csv_doc::actions::open_active_cell_editor);
            visual.run_until_parked();
            visual.simulate_input("kept 中😀");
            // Guard boundary controls, using a genuine editor draft. The
            // helper must not consume recovery or live input to show Source.
            visual.update_window_entity(entity, |w, window, cx| {
                let id = w.tabs[1].session_id;
                let draft = w.csv_models.get(&id).unwrap().pending_draft(&w.tabs[1].buffer.raw_revision).unwrap();
                assert!(draft.value.contains("kept 中😀"));
                if restored {
                    w.csv_models.remove(&id);
                    w.tabs[1].csv_draft = Some(draft);
                }
                w.tabs[1].buffer.file_path = Some(directory.join("neutral.txt"));
                assert!(!w.tabs[1].undo_stack.is_empty());
                assert!(!w.tabs[1].redo_stack.is_empty());
                let before = format!("{:?}:{:?}:{}:{}", w.tabs[1].csv_draft,
                    w.csv_models.get(&id).and_then(|model| model.cell_input.as_ref()).map(|input| (&input.buffer, input.anchor, input.cursor)),
                    w.tabs[1].undo_stack.len(), w.tabs[1].redo_stack.len());
                let source = w.tabs[1].buffer.rope.clone();
                let revision = w.tabs[1].buffer.raw_revision.clone();
                let epoch = w.source_input_token();
                let focus = window.input_context_token();
                w.normalize_saved_csv_source_view(cx);
                assert_eq!(w.tabs[1].view_mode, TabViewMode::CsvPreview);
                assert_eq!(before, format!("{:?}:{:?}:{}:{}", w.tabs[1].csv_draft,
                    w.csv_models.get(&id).and_then(|model| model.cell_input.as_ref()).map(|input| (&input.buffer, input.anchor, input.cursor)),
                    w.tabs[1].undo_stack.len(), w.tabs[1].redo_stack.len()));
                assert!(source.is_instance(&w.tabs[1].buffer.rope));
                assert_eq!(w.tabs[1].buffer.raw_revision, revision);
                assert_eq!(w.source_input_token(), epoch);
                assert_eq!(window.input_context_token(), focus);
            });
        });
    }

    #[test]
    fn source_input_origin_non_csv_live_cell_preserves_preview_and_draft() { draft_boundary(false); }

    #[test]
    fn source_input_origin_non_csv_recovery_preserves_preview_and_draft() { draft_boundary(true); }
}
