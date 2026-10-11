// Doc View replacement prepares and verifies an AST candidate before mutation.
use crate::rich_doc::{ast::Block, edits::Edit, RichDocPayload, TextSelection};
use crate::text::regex_replacement::Replacement;

const MAX_AST_BYTES: usize = 2 * 1024 * 1024;
const MAX_LOG_BYTES: usize = 1024 * 1024;
const MAX_COPY_WORK: usize = 16 * 1024 * 1024;
const MAX_NODE_VISITS: usize = 262_144;

#[cfg(test)]
thread_local! { static CANDIDATE_CAPTURES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

#[cfg(test)]
pub(super) fn captures_for_test() -> usize {
    CANDIDATE_CAPTURES.with(std::cell::Cell::get)
}

pub(super) struct Budget {
    bytes: usize,
    nodes: usize,
}

pub(super) struct Prepared {
    pub(super) ops: Vec<Edit>,
    pub(super) selection: Option<TextSelection>,
    pub(super) offsets: Vec<(usize, usize, usize)>,
}

pub(super) fn validate(payload: &RichDocPayload) -> Result<Budget, String> {
    if payload.ast.blocks.len() > 4096 || payload.edit_log.len() > 4096 {
        return Err("Rich regex replacement supports up to 4096 blocks and prior edits; save and reopen a larger document first".into());
    }
    let mut bytes = payload
        .ast
        .blocks
        .len()
        .saturating_mul(std::mem::size_of::<Block>());
    let mut nodes = payload.ast.blocks.len();
    for block in &payload.ast.blocks {
        let (Block::Paragraph { runs, .. } | Block::Heading { runs, .. }) = block else {
            return Err("Rich regex replacement currently supports paragraph and heading documents; tables and images need a separate replacement path".into());
        };
        nodes = nodes.saturating_add(runs.len());
        bytes = bytes.saturating_add(
            runs.len()
                .saturating_mul(std::mem::size_of::<crate::rich_doc::ast::InlineRun>()),
        );
        for run in runs {
            bytes = bytes
                .saturating_add(run.text.len())
                .saturating_add(run.style.link.as_ref().map_or(0, String::len));
            if bytes > MAX_AST_BYTES {
                break;
            }
        }
        if bytes > MAX_AST_BYTES || nodes > 65_536 {
            return Err(
                "Rich regex replacement AST exceeds 2 MiB or 65536 nodes; use a smaller document"
                    .into(),
            );
        }
    }
    let mut log_bytes = payload
        .edit_log
        .len()
        .saturating_mul(std::mem::size_of::<Edit>());
    for edit in &payload.edit_log {
        log_bytes = log_bytes.saturating_add(match edit {
            Edit::InsertText { text, .. } => text.len(),
            Edit::SetBlockLink { url, .. } => url.as_ref().map_or(0, String::len),
            Edit::InsertBlocks { .. } => return Err("Rich regex replacement cannot snapshot prior formatted-block paste history yet; save and reopen the document first".into()),
            _ => 0,
        });
        if log_bytes > MAX_LOG_BYTES {
            return Err(
                "Rich regex replacement history exceeds 1 MiB; save and reopen the document first"
                    .into(),
            );
        }
    }
    Ok(Budget {
        bytes: bytes.saturating_add(log_bytes),
        nodes,
    })
}

pub(super) fn apply_work(budget: &Budget, ops: usize) -> Result<(usize, usize), String> {
    if ops == 0 {
        return Ok((0, 0));
    }
    let copies = budget.bytes.saturating_mul(ops.saturating_add(1));
    let visits = budget.nodes.saturating_mul(ops);
    if copies > MAX_COPY_WORK || visits > MAX_NODE_VISITS {
        return Err("Rich regex replacement exceeds its text/formatting work limit; use a smaller selection".into());
    }
    Ok((copies, visits))
}

pub(super) fn capture_work(budget: &Budget) -> (usize, usize) {
    // One AST and two flat-text snapshots are captured for the worker/stamp.
    // The estimate includes history, which is conservative but never cloned.
    (budget.bytes.saturating_mul(3), budget.nodes)
}

pub(super) fn total_work(copies: usize, visits: usize) -> Result<(), String> {
    if copies > MAX_COPY_WORK || visits > MAX_NODE_VISITS {
        Err("Rich regex replacement exceeds its text/formatting work limit across documents; use a smaller selection".into())
    } else {
        Ok(())
    }
}

/// Copy only current editable state. Undo, container bytes and headers are
/// deliberately absent from this scratch candidate, which is never saved.
pub(super) fn candidate(payload: &RichDocPayload) -> RichDocPayload {
    #[cfg(test)]
    CANDIDATE_CAPTURES.with(|count| count.set(count.get().saturating_add(1)));
    RichDocPayload {
        revision: payload.revision.clone(),
        format: payload.format,
        ast: payload.ast.clone(),
        raw_view_text: String::new(),
        original_bytes: std::sync::Arc::new(Vec::new()),
        edit_log: Vec::new(),
        active_block: payload.active_block,
        selection: payload.selection,
        drag_active: false,
        pending_drag_anchor: None,
        headers: Vec::new(),
        footers: Vec::new(),
        undo_stack: Vec::new(),
        redo_stack: Vec::new(),
        edit_group_start: None,
    }
}

pub(super) fn prepare(
    mut candidate: RichDocPayload,
    edits: &[Replacement],
    output: &str,
) -> Result<Prepared, String> {
    let mut ops = Vec::new();
    let changed = edits.iter().filter(|edit| edit.changed).count();
    ops.try_reserve_exact(changed.saturating_mul(2))
        .map_err(|_| "Not enough memory to prepare rich replacement history".to_string())?;
    let mut caret = None;
    for edit in edits.iter().rev().filter(|edit| edit.changed) {
        let (text, offsets) = candidate.flat_search_text();
        if text.get(edit.range.clone()).is_none() {
            return Err("Rich replacement match range is stale; run Replace again".into());
        }
        let lo = candidate
            .pos_from_global_flat(&offsets, edit.range.start, false)
            .ok_or_else(|| "Match has no editable Doc View start position".to_string())?;
        let hi = candidate
            .pos_from_global_flat(&offsets, edit.range.end, false)
            .ok_or_else(|| "Match has no editable Doc View end position".to_string())?;
        if !edit.range.is_empty() {
            let delete = if lo.block == hi.block {
                Edit::DeleteRange {
                    block: lo.block,
                    run_lo: lo.run,
                    byte_lo: lo.byte,
                    run_hi: hi.run,
                    byte_hi: hi.byte,
                }
            } else {
                Edit::DeleteAcrossBlocks {
                    from_block: lo.block,
                    from_run: lo.run,
                    from_byte: lo.byte,
                    to_block: hi.block,
                    to_run: hi.run,
                    to_byte: hi.byte,
                }
            };
            crate::rich_doc::edits::apply(&delete, &mut candidate.ast);
            ops.push(delete);
        }
        if !edit.text.is_empty() {
            let insert = Edit::InsertText {
                block: lo.block,
                run: lo.run,
                byte: lo.byte,
                text: edit.text.clone(),
            };
            crate::rich_doc::edits::apply(&insert, &mut candidate.ast);
            ops.push(insert);
        }
        caret = Some(edit.range.start.saturating_add(edit.text.len()));
    }
    let (text, offsets) = candidate.flat_search_text();
    if text != output {
        return Err(
            "Some matches cannot be replaced faithfully in Doc View; use a smaller selection"
                .into(),
        );
    }
    let selection = match caret {
        Some(caret) => {
            let pos = candidate
                .pos_from_global_flat(&offsets, caret, false)
                .ok_or_else(|| "Replacement has no editable Doc View caret position".to_string())?;
            Some(TextSelection {
                anchor: pos,
                cursor: pos,
            })
        }
        None => candidate.selection,
    };
    Ok(Prepared {
        ops,
        selection,
        offsets,
    })
}
