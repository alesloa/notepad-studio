// scintilla/src/UndoHistory.cxx:253-350,565-622 — adjacent insertion actions,
// savepoint/explicit-group boundaries, and eligibility retained by Undo/Redo.
use crate::buffer::{ContentRevision, EditOp, UndoEntry};
use crate::tab::Tab;

#[derive(Default)]
pub(crate) struct State {
    run: Option<Run>,
    #[cfg(test)]
    pub(super) fail_group_reserve: bool,
    #[cfg(test)]
    pub(super) fail_history_reserve: bool,
    #[cfg(test)]
    pub(super) fail_insert_reserve: bool,
    #[cfg(test)]
    pub(super) fail_remove_reserve: bool,
    #[cfg(test)]
    pub(super) fail_macro_reserve: bool,
}

struct Run {
    entry: EntryStamp,
    after: ContentRevision,
    next_byte: usize,
}

impl State {
    pub(crate) fn seal(&mut self) { self.run = None; }

    fn adjacent(&self, tab: &Tab, byte: usize) -> bool {
        self.run.as_ref().is_some_and(|run| {
            run.next_byte == byte && run.after == tab.buffer.raw_revision
                && !tab.buffer.at_raw_savepoint()
                && tab.undo_stack.last().is_some_and(|entry| run.entry.matches(entry))
        })
    }
}

#[derive(Clone)]
enum EntryStamp {
    Legacy { group: bool, count: usize, before: Option<ContentRevision>,
        after: Option<ContentRevision>, anchor: usize, head: usize },
    Prepared(std::sync::Arc<crate::buffer::prepared::PreparedGroup>),
}

impl EntryStamp {
    fn capture(entry: &UndoEntry) -> Self {
        match entry {
            UndoEntry::Single(op) => Self::Legacy { group: false, count: 1,
                before: Some(op.before_revision.clone()), after: Some(op.after_revision.clone()),
                anchor: op.sel_before_anchor, head: op.sel_before_head },
            UndoEntry::Group { ops, sel_before_anchor, sel_before_head } => Self::Legacy {
                group: true, count: ops.len(), before: ops.first().map(|op| op.before_revision.clone()),
                after: ops.last().map(|op| op.after_revision.clone()),
                anchor: *sel_before_anchor, head: *sel_before_head },
            UndoEntry::Prepared(group) => Self::Prepared(group.clone()),
        }
    }

    fn matches(&self, entry: &UndoEntry) -> bool {
        match (self, entry) {
            (Self::Prepared(expected), UndoEntry::Prepared(current)) => std::sync::Arc::ptr_eq(expected, current),
            (Self::Legacy { group: false, count, before, after, anchor, head }, UndoEntry::Single(op)) => {
                *count == 1 && before.as_ref() == Some(&op.before_revision)
                    && after.as_ref() == Some(&op.after_revision)
                    && *anchor == op.sel_before_anchor && *head == op.sel_before_head
            }
            (Self::Legacy { group: true, count, before, after, anchor, head },
                UndoEntry::Group { ops, sel_before_anchor, sel_before_head }) => {
                *count == ops.len() && before.as_ref() == ops.first().map(|op| &op.before_revision)
                    && after.as_ref() == ops.last().map(|op| &op.after_revision)
                    && *anchor == *sel_before_anchor && *head == *sel_before_head
            }
            _ => false,
        }
    }
}

#[derive(Clone)]
pub(crate) struct HistoryStamp {
    revision: ContentRevision,
    activity: ContentRevision,
    undo_len: usize,
    redo_len: usize,
    undo: Option<EntryStamp>,
    redo: Option<EntryStamp>,
}

impl HistoryStamp {
    pub(crate) fn capture(tab: &Tab) -> Self {
        Self { revision: tab.buffer.raw_revision.clone(), activity: tab.content_activity.clone(),
            undo_len: tab.undo_stack.len(), redo_len: tab.redo_stack.len(),
            undo: tab.undo_stack.last().map(EntryStamp::capture), redo: tab.redo_stack.last().map(EntryStamp::capture) }
    }

    pub(crate) fn matches(&self, tab: &Tab) -> bool {
        fn same(stamp: &Option<EntryStamp>, entry: Option<&UndoEntry>) -> bool {
            match (stamp, entry) { (None, None) => true, (Some(stamp), Some(entry)) => stamp.matches(entry), _ => false }
        }
        self.revision == tab.buffer.raw_revision && self.activity == tab.content_activity
            && self.undo_len == tab.undo_stack.len() && self.redo_len == tab.redo_stack.len()
            && same(&self.undo, tab.undo_stack.last()) && same(&self.redo, tab.redo_stack.last())
    }
}

pub(super) enum Plan { Separate, Pair(Vec<EditOp>), Append, Transaction }

pub(super) fn prepare_text(
    tab: &mut Tab, range: &std::ops::Range<usize>, new_text: &str,
) -> Result<(String, String), &'static str> {
    let mut inserted = String::new();
    let mut removed = String::new();
    #[cfg(test)]
    if std::mem::take(&mut tab.typing_undo.fail_insert_reserve) {
        return Err("Cannot reserve inserted typing Undo text; no changes applied");
    }
    if inserted.try_reserve_exact(new_text.len()).is_err() {
        return Err("Cannot reserve inserted typing Undo text; no changes applied");
    }
    inserted.push_str(new_text);
    let Some(slice) = tab.buffer.rope.get_byte_slice(range.clone()) else {
        return Err("Typing range changed; no changes applied");
    };
    #[cfg(test)]
    if std::mem::take(&mut tab.typing_undo.fail_remove_reserve) {
        return Err("Cannot reserve removed typing Undo text; no changes applied");
    }
    if removed.try_reserve_exact(slice.len_bytes()).is_err() {
        return Err("Cannot reserve removed typing Undo text; no changes applied");
    }
    for chunk in slice.chunks() { removed.push_str(chunk); }
    Ok((removed, inserted))
}

pub(super) fn prepare(tab: &mut Tab, byte: usize, coalesce: bool) -> Result<Plan, &'static str> {
    #[cfg(test)]
    let fail_history = std::mem::take(&mut tab.typing_undo.fail_history_reserve);
    #[cfg(not(test))]
    let fail_history = false;
    if let Some(transaction) = tab.edit_transaction.as_mut() {
        if fail_history || transaction.ops.try_reserve(1).is_err() { return Err("Cannot reserve typing Undo history; no changes applied"); }
        return Ok(Plan::Transaction);
    }
    if coalesce && tab.typing_undo.adjacent(tab, byte) {
        #[cfg(test)]
        let fail_group = std::mem::take(&mut tab.typing_undo.fail_group_reserve);
        #[cfg(not(test))]
        let fail_group = false;
        if !fail_group {
            match tab.undo_stack.last_mut() {
                Some(UndoEntry::Single(_)) => {
                    let mut ops = Vec::new();
                    if ops.try_reserve_exact(2).is_ok() { return Ok(Plan::Pair(ops)); }
                }
                Some(UndoEntry::Group { ops, .. }) => {
                    if ops.try_reserve(1).is_ok() { return Ok(Plan::Append); }
                }
                _ => {}
            }
        }
    }
    if fail_history || tab.undo_stack.try_reserve(1).is_err() { return Err("Cannot reserve typing Undo history; no changes applied"); }
    Ok(Plan::Separate)
}

pub(super) fn commit(tab: &mut Tab, plan: Plan, op: EditOp, coalesce: bool) {
    let after = op.after_revision.clone();
    let next_byte = op.byte_start.checked_add(op.inserted.len());
    let eligible = coalesce && op.removed.is_empty() && !op.inserted.is_empty()
        && op.before_revision != op.after_revision;
    match plan {
        Plan::Separate => tab.undo_stack.push(UndoEntry::Single(op)),
        Plan::Transaction => {
            if let Some(transaction) = tab.edit_transaction.as_mut() { transaction.ops.push(op); }
            else { tab.undo_stack.push(UndoEntry::Single(op)); }
        }
        Plan::Append => {
            if let Some(UndoEntry::Group { ops, .. }) = tab.undo_stack.last_mut() { ops.push(op); }
            else { tab.undo_stack.push(UndoEntry::Single(op)); }
        }
        Plan::Pair(mut ops) => {
            match tab.undo_stack.pop() {
            Some(UndoEntry::Single(previous)) => {
                let anchor = previous.sel_before_anchor; let head = previous.sel_before_head;
                ops.push(previous); ops.push(op);
                tab.undo_stack.push(UndoEntry::Group { ops, sel_before_anchor: anchor, sel_before_head: head });
            }
            previous => {
                if let Some(previous) = previous { tab.undo_stack.push(previous); }
                tab.undo_stack.push(UndoEntry::Single(op));
            }
            }
        }
    }
    tab.redo_stack.clear();
    tab.typing_undo.run = if eligible && tab.edit_transaction.is_none() {
        next_byte.and_then(|next_byte| tab.undo_stack.last().map(|entry| Run {
            entry: EntryStamp::capture(entry), after, next_byte,
        }))
    } else { None };
}
