// PowerEditor/src/ScintillaComponent/FindReplaceDlg.cpp:3198,3667 —
// Regex Replace/Replace All expand captures from the fresh match.
use std::ops::Range;

use gpui::{AppContext, Context, Task};
use uuid::Uuid;

use super::{
    find::{search_regex, SearchFlags},
    Workspace,
};
use crate::text::regex_replacement::{self as replacement, Replacement, Template};
use crate::{
    buffer::{ContentRevision, Selection},
    find_state::DialogTab,
    tab::TabViewMode,
};

const PREPARING: &str = "Preparing regex replacement…";
const MAX_APPLY_BYTES: usize = 4 * 1024 * 1024;
const MAX_ANNOTATION_VISITS: usize = 262_144;
const MAX_RICH_CAPTURE_BYTES: usize = 8 * 1024 * 1024;
const MAX_RICH_CAPTURE_NODES: usize = 65_536;

type BlockKey = (usize, usize, usize, usize, bool);

fn block_key(tab: &crate::tab::Tab) -> Option<BlockKey> {
    tab.block_selection.map(|s| {
        (
            s.anchor_line,
            s.anchor_col,
            s.head_line,
            s.head_col,
            s.is_dragging,
        )
    })
}

#[derive(Default)]
pub(super) struct Runtime {
    pending: Option<Uuid>,
    task: Option<Task<()>>,
    #[cfg(test)]
    pub(super) last_apply: Option<std::time::Duration>,
    #[cfg(test)]
    pub(super) last_capture: Option<std::time::Duration>,
    #[cfg(test)]
    hold_worker: bool,
    #[cfg(test)]
    held: Option<(Request, Request)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Current,
    All,
    AllOpened,
}

struct Snapshot {
    tab: Uuid,
    rope: ropey::Rope,
    revision: ContentRevision,
    language: Option<crate::syntax::LanguageId>,
    view: TabViewMode,
    selection: Selection,
    block: Option<BlockKey>,
    caret: usize,
    rich: Option<RichSnapshot>,
    read_only: bool,
}

struct RichSnapshot {
    revision: ContentRevision,
    selection: Option<crate::rich_doc::TextSelection>,
    text: String,
    candidate: Option<crate::rich_doc::RichDocPayload>,
}

struct Request {
    id: Uuid,
    mode: Mode,
    active: Uuid,
    query: String,
    template: String,
    flags: SearchFlags,
    scope: Option<Range<usize>>,
    preferred: Option<Range<usize>>,
    previous_index: Option<usize>,
    dont_move: bool,
    in_selection: bool,
    dialog: DialogTab,
    open: bool,
    status: Option<String>,
    tabs: Vec<Snapshot>,
}

struct Plan {
    tab: Uuid,
    edits: Vec<Replacement>,
    matches: Vec<Range<usize>>,
    scope: Option<Range<usize>>,
    source_ops: Vec<crate::buffer::EditOp>,
    rich: Option<super::rich_replacement::Prepared>,
}

pub(super) struct Confirmation {
    id: Uuid,
    active: Uuid,
    query: String,
    template: String,
    flags: SearchFlags,
    tabs: Vec<ConfirmationTab>,
}

struct ConfirmationTab {
    id: Uuid,
    revision: ContentRevision,
    rope: ropey::Rope,
    rich_revision: Option<ContentRevision>,
    view: TabViewMode,
    read_only: bool,
}

impl Workspace {
    pub(super) fn begin_replace_all_confirmation(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<Confirmation> {
        self.cancel_regex_replacement();
        if self.window_closed { return None; }
        if self.find_state.query.len() > replacement::MAX_TEMPLATE_BYTES
            || self.find_state.replacement.len() > replacement::MAX_TEMPLATE_BYTES
        {
            self.regex_replacement_error(
                "Search and replacement must each be at most 1 MiB; shorten them",
                cx,
            );
            return None;
        }
        if self.tabs.len() > 256 {
            self.regex_replacement_error(
                "Replace All supports up to 256 open documents; close some tabs",
                cx,
            );
            return None;
        }
        let active = self.tabs.get(self.active)?.session_id;
        let id = Uuid::new_v4();
        self.regex_replacement.pending = Some(id);
        Some(Confirmation {
            id,
            active,
            query: self.find_state.query.clone(),
            template: self.find_state.replacement.clone(),
            flags: self.find_flags(),
            tabs: self
                .tabs
                .iter()
                .map(|tab| ConfirmationTab {
                    id: tab.session_id,
                    revision: tab.buffer.raw_revision.clone(),
                    rope: tab.buffer.rope.clone(),
                    rich_revision: tab.buffer.rich_doc.as_ref().map(|p| p.revision.clone()),
                    view: tab.view_mode,
                    read_only: tab.buffer.read_only,
                })
                .collect(),
        })
    }

    pub(super) fn replacement_confirmation_valid(&self, confirmation: &Confirmation) -> bool {
        !self.window_closed
            && self.regex_replacement.pending == Some(confirmation.id)
            && self
                .tabs
                .get(self.active)
                .is_some_and(|tab| tab.session_id == confirmation.active)
            && self.find_state.query == confirmation.query
            && self.find_state.replacement == confirmation.template
            && self.find_flags() == confirmation.flags
            && self.tabs.len() == confirmation.tabs.len()
            && self
                .tabs
                .iter()
                .zip(&confirmation.tabs)
                .all(|(tab, snapshot)| {
                    tab.session_id == snapshot.id
                        && tab.buffer.raw_revision == snapshot.revision
                        && tab.buffer.rope.is_instance(&snapshot.rope)
                        && tab.buffer.rich_doc.as_ref().map(|p| &p.revision)
                            == snapshot.rich_revision.as_ref()
                        && tab.view_mode == snapshot.view
                        && tab.buffer.read_only == snapshot.read_only
                })
    }

    pub(super) fn resume_replace_all_confirmation(
        &mut self,
        confirmation: Confirmation,
        cx: &mut Context<Self>,
    ) {
        if self.replacement_confirmation_valid(&confirmation) {
            self.cancel_regex_replacement();
            self.run_replace_all_in_all(cx);
        } else {
            self.retire_replace_all_confirmation(&confirmation, cx);
        }
    }

    pub(super) fn retire_replace_all_confirmation(
        &mut self,
        confirmation: &Confirmation,
        cx: &mut Context<Self>,
    ) {
        if self.regex_replacement.pending == Some(confirmation.id) {
            self.cancel_regex_replacement();
            cx.notify();
        }
    }

    pub(crate) fn cancel_regex_replacement(&mut self) {
        self.regex_replacement.pending = None;
        self.regex_replacement.task = None;
        if self.find_state.status.as_deref() == Some(PREPARING) {
            self.find_state.status = None;
        }
    }

    pub(super) fn start_regex_replacement(&mut self, mode: Mode, cx: &mut Context<Self>) {
        self.cancel_regex_replacement();
        if self.window_closed { return; }
        self.close_function_hint(cx);
        let Some(active) = self.tabs.get(self.active) else {
            return;
        };
        if self.find_state.query.is_empty() {
            return;
        }
        if self.find_state.query.len() > replacement::MAX_TEMPLATE_BYTES {
            self.regex_replacement_error("Regex search pattern exceeds 1 MiB; simplify it", cx);
            return;
        }
        if self.find_state.replacement.len() > replacement::MAX_TEMPLATE_BYTES {
            self.regex_replacement_error(
                "Regex replacement template exceeds 1 MiB; shorten it",
                cx,
            );
            return;
        }
        if active.buffer.read_only && mode != Mode::AllOpened {
            self.set_status("Buffer is read-only");
            self.find_state.status = Some("Buffer is read-only".into());
            cx.notify();
            return;
        }
        let indexes: Vec<usize> = if mode == Mode::AllOpened {
            if self.tabs.len() > 256 {
                self.regex_replacement_error(
                    "Regex replacement supports up to 256 open documents; close some tabs",
                    cx,
                );
                return;
            }
            (0..self.tabs.len()).collect()
        } else {
            vec![self.active]
        };
        #[cfg(test)]
        let capture_started = std::time::Instant::now();
        // Validate the entire capture budget before flattening/cloning any
        // AST. Read-only tabs keep identity stamps but need no candidate.
        let mut capture_bytes = 0usize;
        let mut capture_nodes = 0usize;
        for index in &indexes {
            let Some(tab) = self.tabs.get(*index) else {
                continue;
            };
            if tab.buffer.read_only || tab.view_mode != TabViewMode::RichDocView {
                continue;
            }
            let Some(payload) = tab.buffer.rich_doc.as_ref() else {
                continue;
            };
            let budget = match super::rich_replacement::validate(payload) {
                Ok(budget) => budget,
                Err(error) => {
                    self.regex_replacement_error(error, cx);
                    return;
                }
            };
            let (bytes, nodes) = super::rich_replacement::capture_work(&budget);
            capture_bytes = capture_bytes.saturating_add(bytes);
            capture_nodes = capture_nodes.saturating_add(nodes);
            if capture_bytes > MAX_RICH_CAPTURE_BYTES || capture_nodes > MAX_RICH_CAPTURE_NODES {
                self.regex_replacement_error("Rich regex replacement capture exceeds 8 MiB or 65536 nodes across documents; replace in fewer documents", cx);
                return;
            }
        }
        let mut snapshots = Vec::new();
        let mut source_bytes = 0usize;
        for index in indexes {
            let Some(tab) = self.tabs.get(index) else {
                continue;
            };
            if tab.buffer.read_only {
                snapshots.push(Snapshot {
                    tab: tab.session_id,
                    rope: tab.buffer.rope.clone(),
                    revision: tab.buffer.raw_revision.clone(),
                    language: tab.buffer.language,
                    view: tab.view_mode,
                    selection: tab.selection,
                    block: block_key(tab),
                    caret: tab.selection.head,
                    rich: tab
                        .buffer
                        .rich_doc
                        .as_ref()
                        .filter(|_| tab.view_mode == TabViewMode::RichDocView)
                        .map(|p| RichSnapshot {
                            revision: p.revision.clone(),
                            selection: p.selection,
                            text: String::new(),
                            candidate: None,
                        }),
                    read_only: true,
                });
                continue;
            }
            if tab.buffer.loading.is_some()
                || matches!(
                    tab.view_mode,
                    TabViewMode::CsvPreview | TabViewMode::MarkdownPreview
                )
                || self
                    .csv_models
                    .get(&tab.session_id)
                    .is_some_and(|model| model.cell_input.is_some())
            {
                self.regex_replacement_error(
                    "Finish loading or commit the CSV cell and switch to Source before replacing",
                    cx,
                );
                return;
            }
            if tab.buffer.rope.len_bytes() > replacement::MAX_SOURCE_BYTES {
                self.regex_replacement_error(
                    "Regex replacement supports documents up to 64 MiB; use a smaller file",
                    cx,
                );
                return;
            }
            let mut caret = tab.selection.head;
            let rich = if tab.view_mode == TabViewMode::RichDocView {
                let Some(payload) = tab.buffer.rich_doc.as_ref() else {
                    return;
                };
                if let Err(error) = super::rich_replacement::validate(payload) {
                    self.regex_replacement_error(error, cx);
                    return;
                }
                // Flattening an AST is currently synchronous; keep this first
                // rich-doc batch small and reject before allocating its text.
                let mut bytes = 0usize;
                let mut runs_count = 0usize;
                if payload.ast.blocks.len() > 4096 {
                    self.regex_replacement_error(
                        "Rich-document regex replacement supports up to 4096 blocks",
                        cx,
                    );
                    return;
                }
                for index in 0..payload.ast.blocks.len() {
                    if let Some(runs) = payload.block_runs(index) {
                        runs_count = runs_count.saturating_add(runs.len());
                        bytes = bytes.saturating_add(1);
                        for run in runs {
                            bytes = bytes.saturating_add(run.text.len());
                        }
                    }
                    if bytes > 1024 * 1024 || runs_count > 65_536 {
                        self.regex_replacement_error("Rich-document regex replacement supports up to 1 MiB of text; use Raw View for larger documents", cx);
                        return;
                    }
                }
                let (text, offsets) = payload.flat_search_text();
                caret = payload.selection.map_or(0, |selection| {
                    payload.global_flat_for_pos(&offsets, selection.cursor)
                });
                Some(RichSnapshot {
                    revision: payload.revision.clone(),
                    selection: payload.selection,
                    text,
                    candidate: Some(super::rich_replacement::candidate(payload)),
                })
            } else {
                None
            };
            source_bytes = source_bytes.saturating_add(
                rich.as_ref()
                    .map_or(tab.buffer.rope.len_bytes(), |rich| rich.text.len()),
            );
            if source_bytes > replacement::MAX_SOURCE_BYTES {
                self.regex_replacement_error(
                    "Regex replacement supports up to 64 MiB across the requested documents",
                    cx,
                );
                return;
            }
            snapshots.push(Snapshot {
                tab: tab.session_id,
                rope: tab.buffer.rope.clone(),
                revision: tab.buffer.raw_revision.clone(),
                language: tab.buffer.language,
                view: tab.view_mode,
                selection: tab.selection,
                block: block_key(tab),
                caret,
                rich,
                read_only: tab.buffer.read_only,
            });
        }
        let Some(active) = self.tabs.get(self.active) else {
            return;
        };
        let mut request = Request {
            id: Uuid::new_v4(),
            mode,
            active: active.session_id,
            query: self.find_state.query.clone(),
            template: self.find_state.replacement.clone(),
            flags: self.find_flags(),
            in_selection: self.find_state.in_selection,
            scope: if mode != Mode::AllOpened
                && self.find_state.in_selection
                && (active.view_mode == TabViewMode::RichDocView
                    || active.block_selection.is_none())
            {
                self.find_state.in_selection_scope.clone()
            } else {
                None
            },
            preferred: self
                .find_state
                .current
                .and_then(|i| self.find_state.matches.get(i).cloned()),
            previous_index: self.find_state.current,
            dont_move: crate::settings::snapshot().replace_dont_move_to_next,
            dialog: self.dialog_tab,
            open: self.find_state.open,
            status: self.status_message.clone(),
            tabs: snapshots,
        };
        let job = request_for_background(&mut request);
        #[cfg(test)]
        {
            self.regex_replacement.last_capture = Some(capture_started.elapsed());
        }
        self.regex_replacement.pending = Some(request.id);
        self.find_state.error = None;
        self.find_state.status = Some(PREPARING.into());
        cx.notify();
        #[cfg(test)]
        if self.regex_replacement.hold_worker {
            self.regex_replacement.held = Some((request, job));
            return;
        }
        let task = cx.background_spawn(async move { prepare(job) });
        self.regex_replacement.task = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |w, cx| w.finish_regex_replacement(request, result, cx));
        }));
    }

    fn regex_replacement_error(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        let message = message.into();
        self.set_status(message.clone());
        self.find_state.error = Some(message);
        self.find_state.status = None;
        cx.notify();
    }

    fn request_valid(&self, request: &Request) -> bool {
        !self.window_closed
            && self.regex_replacement.pending == Some(request.id)
            && self
                .tabs
                .get(self.active)
                .is_some_and(|tab| tab.session_id == request.active)
            && self.find_state.query == request.query
            && self.find_state.replacement == request.template
            && self.find_flags() == request.flags
            && self.find_state.in_selection == request.in_selection
            && (request.mode == Mode::AllOpened
                || !request.in_selection
                || self.tabs.get(self.active).is_some_and(|t| {
                    t.view_mode != TabViewMode::RichDocView && t.block_selection.is_some()
                })
                || self.find_state.in_selection_scope == request.scope)
            && self.dialog_tab == request.dialog
            && self.find_state.open == request.open
            && (request.mode != Mode::AllOpened || self.tabs.len() == request.tabs.len())
            && request.tabs.iter().all(|snapshot| {
                self.tabs
                    .iter()
                    .find(|tab| tab.session_id == snapshot.tab)
                    .is_some_and(|tab| {
                        tab.buffer.raw_revision == snapshot.revision
                            && tab.buffer.language == snapshot.language
                            && tab.buffer.rope.is_instance(&snapshot.rope)
                            && tab.view_mode == snapshot.view
                            && tab.selection.head == snapshot.selection.head
                            && tab.selection.anchor == snapshot.selection.anchor
                            && block_key(tab) == snapshot.block
                            && tab.buffer.read_only == snapshot.read_only
                            && (tab.buffer.read_only || tab.buffer.loading.is_none())
                            && (tab.buffer.read_only
                                || self
                                    .csv_models
                                    .get(&tab.session_id)
                                    .is_none_or(|model| model.cell_input.is_none()))
                            && match (&snapshot.rich, &tab.buffer.rich_doc) {
                                (Some(rich), Some(payload)) => {
                                    payload.revision == rich.revision
                                        && payload.selection == rich.selection
                                }
                                (None, _) => true,
                                _ => false,
                            }
                    })
            })
    }

    fn finish_regex_replacement(
        &mut self,
        request: Request,
        result: Result<Vec<Plan>, String>,
        cx: &mut Context<Self>,
    ) {
        if !self.request_valid(&request) {
            if self.regex_replacement.pending == Some(request.id) {
                self.cancel_regex_replacement();
                cx.notify();
            }
            return;
        }
        #[cfg(test)]
        let apply_started = std::time::Instant::now();
        self.regex_replacement.pending = None;
        let plans = match result {
            Ok(plans) => plans,
            Err(error) => {
                let newer_status =
                    (self.status_message != request.status).then(|| self.status_message.clone());
                self.regex_replacement_error(error, cx);
                if let Some(status) = newer_status {
                    self.status_message = status;
                }
                return;
            }
        };
        if let Err(error) = self.validate_regex_apply_work(&plans) {
            let newer_status =
                (self.status_message != request.status).then(|| self.status_message.clone());
            self.regex_replacement_error(error, cx);
            if let Some(status) = newer_status {
                self.status_message = status;
            }
            return;
        }
        let original_active = self.active;
        let newer_status =
            (self.status_message != request.status).then(|| self.status_message.clone());
        let mut count = 0usize;
        let mut docs = 0usize;
        let mut active_matches = None;
        let original_offsets = self.find_state.rich_doc_block_offsets.clone();
        for mut plan in plans {
            let Some(index) = self.tabs.iter().position(|tab| tab.session_id == plan.tab) else {
                continue;
            };
            if index == original_active {
                active_matches = Some((
                    plan.matches,
                    plan.scope,
                    plan.rich.as_ref().map(|p| p.offsets.clone()),
                ));
            }
            if plan.edits.is_empty() {
                continue;
            }
            self.active = index;
            let changed = plan.edits.iter().any(|edit| edit.changed);
            if changed {
                if self.tabs[index].view_mode == TabViewMode::RichDocView {
                    if let Some(prepared) = plan.rich.as_mut() {
                        self.with_rich_doc_edit_group(cx, |w, cx| {
                            if let Some(payload) = w.active_tab_mut().buffer.rich_doc.as_mut() {
                                for op in std::mem::take(&mut prepared.ops) {
                                    payload.apply_edit(op);
                                }
                                payload.selection = prepared.selection;
                                payload.active_block = prepared.selection.map(|s| s.cursor.block);
                                payload.drag_active = false;
                                payload.pending_drag_anchor = None;
                            }
                            w.after_rich_doc_edit(cx);
                        });
                    }
                } else {
                    let existing_group = self.tabs[index].edit_transaction.is_some();
                    self.begin_edit_group();
                    if !existing_group {
                        if let Some(group) = self.tabs[index].edit_transaction.as_mut() {
                            group.ops = std::mem::take(&mut plan.source_ops);
                        }
                    }
                    for edit in plan.edits.iter().rev().filter(|edit| edit.changed) {
                        self.replace_byte_range(edit.range.clone(), &edit.text, cx);
                    }
                    self.end_edit_group(cx);
                    self.csv_models.remove(&plan.tab);
                }
            }
            count = count.saturating_add(plan.edits.len());
            docs = docs.saturating_add(1);
        }
        self.active = original_active;
        if let Some((matches, scope, rich_offsets)) = active_matches {
            if request.in_selection && request.mode != Mode::AllOpened {
                self.find_state.in_selection_scope = scope;
            }
            let caret = self.tabs[self.active]
                .buffer
                .rich_doc
                .as_ref()
                .zip(rich_offsets.as_ref())
                .map_or(self.selection().head, |(payload, offsets)| {
                    payload
                        .selection
                        .map_or(0, |s| payload.global_flat_for_pos(offsets, s.cursor))
                });
            self.find_state.matches = matches;
            self.find_state.rich_doc_block_offsets = rich_offsets;
            self.find_state.current = self
                .find_state
                .matches
                .iter()
                .position(|r| r.start >= caret)
                .or_else(|| (!self.find_state.matches.is_empty()).then_some(0));
            if request.mode == Mode::Current && request.dont_move {
                self.find_state.current = self
                    .find_state
                    .matches
                    .len()
                    .checked_sub(1)
                    .map(|last| request.previous_index.unwrap_or(0).min(last));
            }
        } else {
            self.find_state.rich_doc_block_offsets = original_offsets;
        }
        if count > 0 {
            crate::settings::push_find_history(
                crate::settings::FindHistorySlot::FindQuery,
                &request.query,
            );
            crate::settings::push_find_history(
                crate::settings::FindHistorySlot::Replacement,
                &request.template,
            );
        }
        self.find_state.error = None;
        let status = if count == 0 {
            "No matches to replace".to_string()
        } else if request.mode == Mode::AllOpened {
            format!("Replaced {count} occurrences across {docs} documents")
        } else {
            format!(
                "Replaced {count} occurrence{}",
                if count == 1 { "" } else { "s" }
            )
        };
        self.find_state.status = Some(status.clone());
        // after_edit_flush clears the previous editor status. Do not replace
        // a newer status raised while the background plan was preparing.
        if let Some(newer_status) = newer_status {
            self.status_message = newer_status;
        } else {
            self.set_status(status);
        }
        self.last_lines.clear();
        #[cfg(test)]
        {
            self.regex_replacement.last_apply = Some(apply_started.elapsed());
        }
        cx.notify();
    }

    fn validate_regex_apply_work(&mut self, plans: &[Plan]) -> Result<(), String> {
        let mut bytes = 0usize;
        let mut annotation_visits = 0usize;
        let mut rich_copies = 0usize;
        let mut rich_visits = 0usize;
        for plan in plans {
            let Some(tab) = self.tabs.iter().find(|tab| tab.session_id == plan.tab) else {
                return Err("Replacement target changed; run Replace again".into());
            };
            let mut changed = 0usize;
            for edit in plan.edits.iter().filter(|edit| edit.changed) {
                changed += 1;
                bytes = bytes
                    .saturating_add(edit.range.end.saturating_sub(edit.range.start))
                    .saturating_add(edit.text.len());
            }
            // The existing edit funnel shifts and normalizes each annotation
            // for each edit. Refuse excessive work before any tab is changed.
            let annotations = tab
                .buffer
                .bookmarks
                .len()
                .saturating_mul(2)
                .saturating_add(tab.buffer.marks.len().saturating_mul(3));
            annotation_visits =
                annotation_visits.saturating_add(changed.saturating_mul(annotations));
            if tab.view_mode == TabViewMode::RichDocView {
                let prepared = plan.rich.as_ref().ok_or_else(|| {
                    "Rich replacement was not prepared; run Replace again".to_string()
                })?;
                let payload = tab
                    .buffer
                    .rich_doc
                    .as_ref()
                    .ok_or_else(|| "Rich replacement target disappeared".to_string())?;
                let budget = super::rich_replacement::validate(payload)?;
                let (copies, visits) =
                    super::rich_replacement::apply_work(&budget, prepared.ops.len())?;
                rich_copies = rich_copies.saturating_add(copies);
                rich_visits = rich_visits.saturating_add(visits);
            }
        }
        if bytes > MAX_APPLY_BYTES {
            return Err(
                "Regex replacement changes more than 4 MiB at once; use a smaller selection".into(),
            );
        }
        if annotation_visits > MAX_ANNOTATION_VISITS {
            return Err("Regex replacement would shift too many bookmarks or marks; use a smaller selection or clear annotations".into());
        }
        super::rich_replacement::total_work(rich_copies, rich_visits)?;
        for plan in plans {
            let changed = plan.edits.iter().filter(|edit| edit.changed).count();
            if changed == 0 {
                continue;
            }
            let Some(tab) = self.tabs.iter_mut().find(|tab| tab.session_id == plan.tab) else {
                return Err("Replacement target changed; run Replace again".into());
            };
            let reserve_error = || {
                "Not enough memory to reserve replacement Undo history; use a smaller selection"
                    .to_string()
            };
            if tab.view_mode == TabViewMode::RichDocView {
                if let Some(payload) = tab.buffer.rich_doc.as_mut() {
                    payload
                        .undo_stack
                        .try_reserve(1)
                        .map_err(|_| reserve_error())?;
                    payload
                        .edit_log
                        .try_reserve(changed.saturating_mul(2))
                        .map_err(|_| reserve_error())?;
                }
            } else {
                tab.undo_stack.try_reserve(1).map_err(|_| reserve_error())?;
                if let Some(group) = tab.edit_transaction.as_mut() {
                    group
                        .ops
                        .try_reserve(changed)
                        .map_err(|_| reserve_error())?;
                }
            }
        }
        Ok(())
    }
}

fn request_for_background(request: &mut Request) -> Request {
    Request {
        id: request.id,
        mode: request.mode,
        active: request.active,
        query: request.query.clone(),
        template: request.template.clone(),
        flags: request.flags.clone(),
        scope: request.scope.clone(),
        preferred: request.preferred.clone(),
        previous_index: request.previous_index,
        dont_move: request.dont_move,
        in_selection: request.in_selection,
        dialog: request.dialog,
        open: request.open,
        status: request.status.clone(),
        tabs: request
            .tabs
            .iter_mut()
            .map(|tab| Snapshot {
                tab: tab.tab,
                rope: tab.rope.clone(),
                revision: tab.revision.clone(),
                language: tab.language,
                view: tab.view,
                selection: tab.selection,
                block: tab.block,
                caret: tab.caret,
                rich: tab.rich.as_mut().map(|rich| RichSnapshot {
                    revision: rich.revision.clone(),
                    selection: rich.selection,
                    text: rich.text.clone(),
                    candidate: rich.candidate.take(),
                }),
                read_only: tab.read_only,
            })
            .collect(),
    }
}

fn prepare(request: Request) -> Result<Vec<Plan>, String> {
    let regex = search_regex(&request.query, &request.flags).map_err(|e| {
        let mut message = format!("Invalid regex: {e}");
        if message.len() > 4096 {
            message.truncate(message.floor_char_boundary(4096));
            message.push('…');
        }
        message
    })?;
    let template = Template::parse(&request.template, &regex)?;
    let mut plans = Vec::new();
    let mut remaining = replacement::MAX_EDITOR_REPLACEMENTS;
    let mut expanded_bytes = 0usize;
    let mut cached_matches = 0usize;
    for mut snapshot in request.tabs {
        if snapshot.read_only {
            continue;
        }
        let text = snapshot
            .rich
            .as_ref()
            .map_or_else(|| snapshot.rope.to_string(), |rich| rich.text.clone());
        let scope = if request.mode == Mode::AllOpened {
            None
        } else {
            request.scope.as_ref()
        };
        let limit = if snapshot.rich.is_some() {
            remaining.min(256)
        } else {
            remaining
        };
        let edits = if request.mode == Mode::Current {
            template.plan_one(
                &regex,
                &text,
                scope,
                request.preferred.as_ref(),
                snapshot.caret,
            )?
        } else {
            template.plan(&regex, &text, scope, limit)?
        };
        remaining = remaining.saturating_sub(edits.len());
        for edit in &edits {
            expanded_bytes = expanded_bytes.saturating_add(edit.text.len());
            if expanded_bytes > replacement::MAX_EXPANDED_BYTES {
                return Err("Regex replacement expands more than 16 MiB across the requested documents; use a smaller selection".into());
            }
        }
        let new_scope = replacement::shifted_scope(scope, &edits);
        let output = replacement::apply_to_string(&text, &edits)?;
        let rich = if let Some(rich) = snapshot.rich.as_mut() {
            let candidate = rich
                .candidate
                .take()
                .ok_or_else(|| "Rich replacement snapshot is missing".to_string())?;
            let budget = super::rich_replacement::validate(&candidate)?;
            super::rich_replacement::apply_work(
                &budget,
                edits
                    .iter()
                    .filter(|edit| edit.changed)
                    .count()
                    .saturating_mul(2),
            )?;
            Some(super::rich_replacement::prepare(
                candidate, &edits, &output,
            )?)
        } else {
            None
        };
        let mut matches = Vec::new();
        for matched in regex.find_iter(&output) {
            let range = matched.start()..matched.end();
            if !replacement::within_scope(&range, new_scope.as_ref()) {
                continue;
            }
            if cached_matches >= replacement::MAX_CACHED_MATCHES {
                return Err("Regex replacement creates more than one million matches; use a smaller selection".into());
            }
            matches.push(range);
            cached_matches += 1;
        }
        let mut source_ops = Vec::new();
        if snapshot.rich.is_none() {
            source_ops
                .try_reserve_exact(edits.iter().filter(|edit| edit.changed).count())
                .map_err(|_| {
                    "Not enough memory to prepare replacement Undo history; use a smaller selection"
                        .to_string()
                })?;
        }
        plans.push(Plan {
            tab: snapshot.tab,
            edits,
            matches,
            scope: new_scope,
            source_ops,
            rich,
        });
    }
    Ok(plans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{actions::EditReplaceAll, settings::Settings};
    use gpui::TestAppContext;

    #[test]
    fn regex_replacement_guard_only_invalidation_retires_preparing_status() {
        let _settings = crate::settings::override_for_test(Settings::default());
        let mut cx = TestAppContext::single();
        let (w, visual) =
            super::super::regex_replacement_tests::fixture(&mut cx, "a a", "(a)", "$1$1");
        visual.update(|window, cx| {
            w.update(cx, |w, cx| {
                w.regex_replacement.hold_worker = true;
                w.replace_all_matches(&EditReplaceAll, window, cx);
                let (request, job) = w.regex_replacement.held.take().unwrap();
                w.active_tab_mut().selection.collapse_to(1);
                w.finish_regex_replacement(request, prepare(job), cx);
                assert!(w.regex_replacement.pending.is_none());
                assert!(w.find_state.status.is_none());
                assert_eq!(w.buffer().rope.to_string(), "a a");
            })
        });
        cx.quit();
    }

    #[test]
    fn regex_replacement_old_reply_cannot_retire_a_newer_preparing_request() {
        let _settings = crate::settings::override_for_test(Settings::default());
        let mut cx = TestAppContext::single();
        let (w, visual) =
            super::super::regex_replacement_tests::fixture(&mut cx, "a a", "(a)", "$1$1");
        visual.update(|window, cx| {
            w.update(cx, |w, cx| {
                w.regex_replacement.hold_worker = true;
                w.replace_all_matches(&EditReplaceAll, window, cx);
                let (old, old_job) = w.regex_replacement.held.take().unwrap();
                w.replace_all_matches(&EditReplaceAll, window, cx);
                let (new, new_job) = w.regex_replacement.held.take().unwrap();
                w.finish_regex_replacement(old, prepare(old_job), cx);
                assert_eq!(w.regex_replacement.pending, Some(new.id));
                assert_eq!(w.find_state.status.as_deref(), Some(PREPARING));
                w.set_status("newer user-visible status");
                w.finish_regex_replacement(new, prepare(new_job), cx);
                assert_eq!(w.buffer().rope.to_string(), "aa aa");
                assert_eq!(
                    w.status_message.as_deref(),
                    Some("newer user-visible status")
                );
                assert!(w.regex_replacement.pending.is_none());
            })
        });
        cx.quit();
    }

    #[test]
    fn stale_confirmation_no_retires_only_its_own_invocation() {
        let _settings = crate::settings::override_for_test(Settings::default());
        let mut cx = TestAppContext::single();
        let (w, visual) = super::super::regex_replacement_tests::fixture(&mut cx, "a", "a", "z");
        visual.update(|_, cx| {
            w.update(cx, |w, cx| {
                let old = w.begin_replace_all_confirmation(cx).unwrap();
                w.active_tab_mut().buffer.read_only = true;
                w.find_state.status = Some("newer status".into());
                assert!(!w.replacement_confirmation_valid(&old));
                w.retire_replace_all_confirmation(&old, cx);
                assert!(w.regex_replacement.pending.is_none());
                assert_eq!(w.find_state.status.as_deref(), Some("newer status"));
                let old = w.begin_replace_all_confirmation(cx).unwrap();
                let new = w.begin_replace_all_confirmation(cx).unwrap();
                w.retire_replace_all_confirmation(&old, cx);
                assert_eq!(w.regex_replacement.pending, Some(new.id));
                assert_eq!(w.find_state.status.as_deref(), Some("newer status"));
            })
        });
        cx.quit();
    }

    fn retained_close_fixture(cx: &mut TestAppContext, rich: bool, saved: bool)
        -> (gpui::Entity<Workspace>, &mut gpui::VisualTestContext)
    {
        let (entity, visual) = cx.add_window_view(|_, cx| {
            let mut w = Workspace::new(
                cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
                cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
                1., gpui::WindowAppearance::Dark,
            );
            w.active_tab_mut().buffer.rope = ropey::Rope::from_str("a a");
            if rich {
                w.active_tab_mut().buffer.rich_doc = Some(
                    crate::rich_doc::rtf::open_bytes(br"{\rtf1\ansi a a}".to_vec()).unwrap(),
                );
                w.active_tab_mut().view_mode = TabViewMode::RichDocView;
            }
            let mut other = crate::tab::Tab::empty();
            other.buffer.rope = ropey::Rope::from_str("a a");
            w.tabs.push(other);
            w.session_snapshot_sink = Some(std::sync::mpsc::channel().0);
            w.session_final_save_result = Some(saved);
            w.dialog_tab = DialogTab::Replace;
            w.find_state.open = true;
            w.find_state.query = "(a)".into();
            w.find_state.replacement = "$1$1".into();
            w.find_state.regex = true;
            w.find_state.match_case = true;
            w.refresh_matches();
            w
        });
        visual.update(|window, cx| {
            window.activate_window();
            window.focus(&entity.read(cx).focus_handle.clone(), cx);
            let close_entity = entity.clone();
            window.on_window_should_close(cx, move |_, cx| {
                close_entity.update(cx, crate::app::quit::save_before_window_close)
            });
        });
        visual.run_until_parked();
        (entity, visual)
    }

    fn retained_close_case(rich: bool, mode: Mode, saved: bool) {
        let _settings = crate::settings::override_for_test(Settings {
            toolbar_hidden: true,
            folder_panel_visible: false,
            ..Settings::default()
        });
        let mut cx = TestAppContext::single();
        let (entity, visual) = retained_close_fixture(&mut cx, rich, saved);
        let (request, result, original) = visual.update(|_, cx| {
            entity.update(cx, |w, cx| {
                assert!(!w.window_closed);
                w.regex_replacement.hold_worker = true;
                w.start_regex_replacement(mode, cx);
                let (request, job) = w.regex_replacement.held.take().unwrap();
                let original = w.tabs.iter().map(|tab| (
                    tab.session_id, tab.buffer.rope.clone(), tab.buffer.raw_revision.clone(),
                )).collect::<Vec<_>>();
                (request, prepare(job), original)
            })
        });
        assert_eq!(visual.simulate_close(), saved);
        if saved { visual.update(|window, _| window.remove_window()); }
        visual.cx.update(|cx| {
            assert_eq!(cx.windows().is_empty(), saved);
            entity.update(cx, |w, cx| {
                assert_eq!(w.window_closed, saved);
                assert_eq!(w.tabs.len(), original.len());
                w.finish_regex_replacement(request, result, cx);
                let expected = if saved { "a a" } else if mode == Mode::Current { "aa a" } else { "aa aa" };
                if rich {
                    let payload = w.buffer().rich_doc.as_ref().unwrap();
                    assert_eq!(payload.flat_search_text().0, expected);
                    assert_eq!(payload.undo_stack.len(), usize::from(!saved));
                } else {
                    assert_eq!(w.buffer().rope.to_string(), expected);
                    assert_eq!(w.tabs[0].undo_stack.len(), usize::from(!saved));
                }
                assert_eq!(w.tabs[1].buffer.rope.to_string(),
                    if !saved && mode == Mode::AllOpened { "aa aa" } else { "a a" });
                if saved {
                    for (tab, (id, rope, revision)) in w.tabs.iter().zip(&original) {
                        assert_eq!(tab.session_id, *id);
                        assert!(tab.buffer.rope.is_instance(rope));
                        assert_eq!(tab.buffer.raw_revision, *revision);
                        assert!(tab.undo_stack.is_empty());
                    }
                    w.start_regex_replacement(mode, cx);
                    assert!(w.regex_replacement.pending.is_none());
                    assert!(w.regex_replacement.held.is_none());
                } else {
                    assert!(w.status_message.as_deref().unwrap().contains("recovery could not be saved"));
                }
            });
        });
        visual.run_until_parked();
        cx.quit();
    }

    #[test]
    fn regex_source_close_refuses_late_workers_with_retained_original_buffers() {
        for mode in [Mode::Current, Mode::All, Mode::AllOpened] { retained_close_case(false, mode, true); }
    }

    #[test]
    fn regex_rich_close_refuses_late_workers_with_retained_original_buffers() {
        for mode in [Mode::Current, Mode::All, Mode::AllOpened] { retained_close_case(true, mode, true); }
    }

    #[test]
    fn regex_failed_close_keeps_source_and_rich_workers_usable() {
        for rich in [false, true] {
            for mode in [Mode::Current, Mode::All, Mode::AllOpened] { retained_close_case(rich, mode, false); }
        }
    }

    #[test]
    fn replacement_confirmation_cannot_resume_after_successful_application_close() {
        for regex in [false, true] {
            let _settings = crate::settings::override_for_test(Settings {
                toolbar_hidden: true, folder_panel_visible: false,
                ..Settings::default()
            });
            let mut cx = TestAppContext::single();
            let (entity, visual) = retained_close_fixture(&mut cx, false, true);
            let confirmation = visual.update(|_, cx| entity.update(cx, |w, cx| {
                w.find_state.regex = regex;
                if !regex { w.find_state.query = "a".into(); }
                w.begin_replace_all_confirmation(cx).unwrap()
            }));
            assert!(visual.simulate_close());
            visual.update(|window, _| window.remove_window());
            visual.cx.update(|cx| entity.update(cx, |w, cx| {
                assert!(w.window_closed);
                assert!(!w.replacement_confirmation_valid(&confirmation));
                w.resume_replace_all_confirmation(confirmation, cx);
                assert!(w.begin_replace_all_confirmation(cx).is_none());
            }));
            visual.run_until_parked();
            visual.cx.update(|cx| {
                let w = entity.read(cx);
                assert!(w.tabs.iter().all(|tab| tab.buffer.rope.chars().eq("a a".chars()) && tab.undo_stack.is_empty()));
            });
            cx.quit();
        }
    }
}
