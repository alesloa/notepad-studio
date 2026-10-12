use crate::{buffer::{Buffer, ContentRevision}, session::CsvDraft, tab::TabViewMode, workspace::Workspace};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(PartialEq)]
struct BufferStamp {
    raw: ContentRevision,
    rich: Option<ContentRevision>,
    path: Option<PathBuf>,
    encoding: crate::text::encoding::Encoding,
    eol: crate::text::line_ending::LineEnding,
    dirty: bool,
    loading: bool,
    read_only: bool,
    label: Option<gpui::SharedString>,
    language: Option<crate::syntax::LanguageId>,
    color: Option<u8>,
    changelog: bool,
    bookmarks: Vec<usize>,
}

impl BufferStamp {
    fn capture(buffer: &Buffer) -> Self {
        Self {
            raw: buffer.raw_revision.clone(), rich: buffer.rich_doc.as_ref().map(|doc| doc.revision.clone()),
            path: buffer.file_path.clone(), encoding: buffer.encoding, eol: buffer.line_ending,
            dirty: buffer.dirty, loading: buffer.loading.is_some(), read_only: buffer.read_only,
            label: buffer.new_label.clone(), language: buffer.language, color: buffer.tab_color,
            changelog: buffer.is_changelog, bookmarks: buffer.bookmarks.iter().copied().collect(),
        }
    }
}

struct DraftStamp(CsvDraft);
impl PartialEq for DraftStamp {
    fn eq(&self, other: &Self) -> bool {
        let (a, b) = (&self.0, &other.0);
        a.value == b.value && a.row == b.row && a.col == b.col && a.is_header == b.is_header
            && a.cursor == b.cursor && a.anchor == b.anchor && a.delimiter == b.delimiter
            && a.has_header == b.has_header && a.source_matches == b.source_matches
            && a.backup_id == b.backup_id && a.source_digest == b.source_digest && a.refusal == b.refusal
    }
}

#[derive(PartialEq)]
struct TabStamp {
    id: Uuid,
    buffer: BufferStamp,
    view: TabViewMode,
    draft: Option<DraftStamp>,
    anchor: usize,
    head: usize,
    scroll_top: u32,
    scroll_left: u32,
    undo: usize,
    redo: usize,
    transaction: bool,
}

/// Compare runtime content identities and recoverable metadata. Recovery-job
/// generations alone cannot see edits still waiting for the autosave debounce.
#[derive(PartialEq)]
pub(super) struct WorkspaceStamp {
    pub(super) id: Uuid,
    tabs: Vec<TabStamp>,
    closed: Vec<(Uuid, BufferStamp)>,
    active: usize,
    mru: Vec<Uuid>,
    autosave: u64,
    post_edit: u64,
    window_closed: bool,
    automatic_title: (ContentRevision, ContentRevision, bool),
    theme: crate::theme::ThemeMode,
    whitespace: bool,
    wrap: bool,
}

impl WorkspaceStamp {
    pub(super) fn capture(workspace: &Workspace) -> Self {
        Self {
            id: workspace.session_window_id,
            tabs: workspace.tabs.iter().map(|tab| TabStamp {
                id: tab.session_id, buffer: BufferStamp::capture(&tab.buffer), view: tab.view_mode,
                draft: workspace.csv_models.get(&tab.session_id)
                    .and_then(|model| model.pending_draft(&tab.buffer.raw_revision))
                    .or_else(|| tab.csv_draft.clone()).map(DraftStamp),
                anchor: tab.selection.anchor, head: tab.selection.head,
                scroll_top: f32::from(tab.scroll_top).to_bits(), scroll_left: f32::from(tab.scroll_left).to_bits(),
                undo: tab.undo_stack.len(), redo: tab.redo_stack.len(), transaction: tab.edit_transaction.is_some(),
            }).collect(),
            closed: workspace.closed_tabs.iter().map(|tab| (tab.session_id, BufferStamp::capture(&tab.buffer))).collect(),
            active: workspace.active, mru: workspace.tab_mru.iter().copied().collect(),
            autosave: workspace.autosave_generation, post_edit: workspace.post_edit_generation,
            window_closed: workspace.window_closed, theme: workspace.theme_mode,
            automatic_title: workspace.automatic_title_state_stamp(),
            whitespace: workspace.show_whitespace, wrap: workspace.word_wrap,
        }
    }
}
