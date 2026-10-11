// Copy legacy history in bounded foreground turns. Sealed history shares its
// immutable body; the combined Rope and annotation maps are built on a worker.
use super::{dispose_on_worker, Workspace};
use crate::buffer::{EditOp, UndoEntry};
use gpui::{Context, Window};
use std::task::Poll;

const TURN_BYTES: usize = 64 * 1024;
const LIMIT: usize = 128 * 1024 * 1024;
const MEMORY: &str = "Not enough memory to capture macro history; Undo remains in separate steps";
const BUDGET: &str = "The full Undo timeline exceeds the 128 MiB macro capture limit; Undo remains in separate steps";

fn reserve_cost(cost: usize, count: usize, size: usize) -> Result<usize, &'static str> {
    count.checked_mul(size).and_then(|bytes| cost.checked_add(bytes)).filter(|&cost| cost <= LIMIT).ok_or(BUDGET)
}

pub(super) struct Capture {
    entries: Vec<UndoEntry>, entry: usize, end: usize, op: usize,
    group: Option<(Vec<EditOp>, usize, usize)>, current: Option<EditOp>,
    inserted: bool, offset: usize, cost: usize,
    #[cfg(test)]
    turn: (usize, usize),
}

impl Capture {
    pub(super) fn new(base: usize, end: usize) -> Result<Self, &'static str> {
        let count = end.checked_sub(base).ok_or("Macro history changed")?;
        if count > 1_000_000 { return Err("Macro history has too many entries to combine"); }
        let cost = reserve_cost(0, count, std::mem::size_of::<UndoEntry>())?;
        let mut entries = Vec::new(); entries.try_reserve_exact(count).map_err(|_| MEMORY)?;
        Ok(Self { entries, entry: base, end, op: 0, group: None, current: None, inserted: false, offset: 0, cost,
            #[cfg(test)] turn: (0, 0) })
    }

    fn step(&mut self, w: &Workspace) -> Result<bool, &'static str> {
        let Some(tab) = w.tabs.get(w.active) else { return Err("Macro editor closed"); };
        let (mut bytes, mut count) = (0usize, 0usize);
        #[cfg(test)]
        { self.turn = (0, 0); }
        while self.entry < self.end && bytes < TURN_BYTES && count < 256 {
            let entry = tab.undo_stack.get(self.entry).ok_or("Macro history changed")?;
            if let UndoEntry::Prepared(group) = entry {
                self.entries.push(UndoEntry::Prepared(group.clone())); self.entry += 1; count += 1;
                #[cfg(test)]
                { self.turn = (bytes, count); }
                continue;
            }
            let source = match entry {
                UndoEntry::Single(op) => std::slice::from_ref(op),
                UndoEntry::Group { ops, sel_before_anchor, sel_before_head } => {
                    if self.group.is_none() {
                        self.cost = reserve_cost(self.cost, ops.len(), std::mem::size_of::<EditOp>())?;
                        let mut group = Vec::new(); group.try_reserve_exact(ops.len()).map_err(|_| MEMORY)?;
                        self.group = Some((group, *sel_before_anchor, *sel_before_head));
                    }
                    ops.as_slice()
                }
                UndoEntry::Prepared(_) => &[],
            };
            let Some(source_op) = source.get(self.op) else {
                if let Some((ops, sel_before_anchor, sel_before_head)) = self.group.take() {
                    self.entries.push(UndoEntry::Group { ops, sel_before_anchor, sel_before_head });
                }
                self.entry += 1; self.op = 0; count += 1; continue;
            };
            if self.current.is_none() {
                self.cost = self.cost.checked_add(source_op.removed.len()).and_then(|n| n.checked_add(source_op.inserted.len()))
                    .and_then(|n| n.checked_add(std::mem::size_of::<EditOp>() + 128)).filter(|&n| n <= LIMIT)
                    .ok_or(BUDGET)?;
                self.current = Some(EditOp { before_revision: source_op.before_revision.clone(), after_revision: source_op.after_revision.clone(),
                    byte_start: source_op.byte_start, removed: String::new(), inserted: String::new(),
                    sel_before_anchor: source_op.sel_before_anchor, sel_before_head: source_op.sel_before_head });
                let current = self.current.as_mut().ok_or("Macro history capture lost its edit")?;
                current.removed.try_reserve_exact(source_op.removed.len()).map_err(|_| MEMORY)?;
                current.inserted.try_reserve_exact(source_op.inserted.len()).map_err(|_| MEMORY)?;
                self.offset = 0; self.inserted = false;
            }
            let current = self.current.as_mut().ok_or("Macro history capture lost its edit")?;
            if current.before_revision != source_op.before_revision || current.after_revision != source_op.after_revision
                || current.byte_start != source_op.byte_start { return Err("Macro history changed during capture"); }
            let text = if self.inserted { &source_op.inserted } else { &source_op.removed };
            let mut end = self.offset.saturating_add(TURN_BYTES.saturating_sub(bytes)).min(text.len());
            while end > self.offset && !text.is_char_boundary(end) { end -= 1; }
            if end == self.offset && end < text.len() { return Ok(false); }
            let part = text.get(self.offset..end).ok_or("Invalid macro history text offset")?;
            if self.inserted { current.inserted.push_str(part); } else { current.removed.push_str(part); }
            bytes += part.len(); self.offset = end;
            #[cfg(test)]
            { self.turn = (bytes, count); }
            if end == text.len() {
                if !self.inserted { self.inserted = true; self.offset = 0; }
                else {
                    let Some(op) = self.current.take() else { return Err("Macro history capture lost its edit"); };
                    if let Some((group, _, _)) = self.group.as_mut() { group.push(op); self.op += 1; }
                    else { self.entries.push(UndoEntry::Single(op)); self.entry += 1; self.op = 0; }
                    count += 1;
                    #[cfg(test)]
                    { self.turn = (bytes, count); }
                }
            }
        }
        Ok(self.entry == self.end)
    }
}

pub(super) fn start(mut capture: Capture, sender: futures::channel::oneshot::Sender<Result<Vec<UndoEntry>, &'static str>>,
    id: uuid::Uuid, window: &mut Window, cx: &mut Context<Workspace>,
) {
    cx.spawn_in(window, async move |this, cx| {
        #[cfg(test)]
        let mut gate = None;
        let result = loop {
            let result = this.update_in(cx, |w, window, _| {
                #[cfg(test)]
                let started = std::time::Instant::now();
                let valid = w.pending_prepared_edit.as_ref().is_some_and(|pending| {
                    pending.id == id && pending.stamp.as_ref().is_some_and(|stamp| stamp.valid(w, window))
                });
                if !valid { return Err("Macro history capture cancelled because the editor changed"); }
                #[cfg(test)]
                if let Some(cost) = w.macro_capture_test_budget.take() { capture.cost = cost; }
                let result = capture.step(w);
                #[cfg(test)]
                {
                    w.macro_capture_turns.push(capture.turn);
                    w.macro_capture_timings.push(started.elapsed());
                    gate = w.macro_capture_test_gate.take();
                }
                result
            });
            match result {
                Ok(Ok(true)) => break Ok(std::mem::take(&mut capture.entries)),
                Ok(Ok(false)) => {},
                Ok(Err(message)) => break Err(message),
                Err(_) => break Err("Macro history capture cancelled because the window closed"),
            }
            #[cfg(test)]
            if let Some(gate) = gate.take() { let _ = gate.await; }
            let mut yielded = false;
            futures::future::poll_fn(|context| {
                if yielded { Poll::Ready(()) } else { yielded = true; context.waker().wake_by_ref(); Poll::Pending }
            }).await;
        };
        if let Err(result) = sender.send(result) { dispose_on_worker(result, cx.background_executor()); }
        dispose_on_worker(capture, cx.background_executor());
    }).detach();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{buffer::ContentRevision, edit_ops::bookmark_clipboard::tests::fixture};
    use gpui::{TestAppContext, VisualContext};

    #[test]
    fn capture_reservations_charge_cumulative_record_storage_before_allocation() {
        assert!(reserve_cost(0, usize::MAX, std::mem::size_of::<UndoEntry>()).is_err());
        assert!(reserve_cost(LIMIT - 1, 1, std::mem::size_of::<EditOp>()).is_err());
        assert!(reserve_cost(0, LIMIT / std::mem::size_of::<UndoEntry>() + 1, std::mem::size_of::<UndoEntry>()).is_err());
        assert_eq!(reserve_cost(LIMIT - 32, 2, 16).unwrap(), LIMIT);
    }

    #[test]
    fn capture_many_empty_group_ops_yields_and_failure_keeps_the_source_history() {
        let mut cx = TestAppContext::single(); let (workspace, visual) = fixture(&mut cx, "a\nb", &[2]);
        visual.update_window_entity(&workspace, |w, _, _| {
            let revision = ContentRevision::default();
            w.active_tab_mut().undo_stack = vec![UndoEntry::Group { ops: (0..600).map(|_| EditOp {
                before_revision: revision.clone(), after_revision: revision.clone(), byte_start: 0, removed: String::new(), inserted: String::new(),
                sel_before_anchor: 0, sel_before_head: 0 }).collect(), sel_before_anchor: 0, sel_before_head: 0 }];
        });
        let mut capture = Capture::new(0, 1).unwrap();
        visual.update(|_, cx| {
            let w = workspace.read(cx);
            assert!(!capture.step(w).unwrap()); assert_eq!(capture.turn, (0, 256));
            assert!(capture.cost >= std::mem::size_of::<UndoEntry>() + 600 * std::mem::size_of::<EditOp>());
            capture.cost = LIMIT;
            assert!(capture.step(w).is_err());
            assert!(matches!(&w.tabs[0].undo_stack[0], UndoEntry::Group { ops, .. } if ops.len() == 600));
            assert_eq!(w.buffer().rope.to_string(), "a\nb");
        });
        cx.quit();
    }
}
