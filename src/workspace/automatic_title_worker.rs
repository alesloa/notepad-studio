use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}};

use gpui::SharedString;
use uuid::Uuid;

use crate::{automatic_title::{Candidate, history}, buffer::{ContentRevision, prepared::PreparedGroup}};
use super::Effect;

pub(super) const MAX_NOTICES: usize = 4096;
pub(super) const MAX_SOURCE_BYTES: usize = 128 * 1024 * 1024;

#[derive(Default)]
pub(super) struct Credit { roots: AtomicUsize, bytes: AtomicUsize }

impl Credit {
    pub(super) fn available(&self) -> bool {
        self.roots.load(Ordering::Acquire) == 0 && self.bytes.load(Ordering::Acquire) == 0
    }

    pub(super) fn retain_context(self: &Arc<Self>, bytes: usize) -> Result<ContextLease, &'static str> {
        self.bytes.fetch_update(Ordering::AcqRel, Ordering::Acquire, |charged|
            charged.checked_add(bytes).filter(|&value| value <= MAX_SOURCE_BYTES)).map_err(|_| super::LIMIT_STATUS)?;
        Ok(ContextLease { credit: self.clone(), bytes })
    }

    pub(super) fn admit(self: &Arc<Self>, bytes: usize) -> Result<Lease, &'static str> {
        self.roots.fetch_update(Ordering::AcqRel, Ordering::Acquire, |roots|
            roots.checked_add(1).filter(|&value| value <= MAX_NOTICES)).map_err(|_| super::LIMIT_STATUS)?;
        if self.bytes.fetch_update(Ordering::AcqRel, Ordering::Acquire, |charged|
            charged.checked_add(bytes).filter(|&value| value <= MAX_SOURCE_BYTES)).is_err()
        {
            self.roots.fetch_sub(1, Ordering::AcqRel); return Err(super::LIMIT_STATUS);
        }
        Ok(Lease { credit: self.clone(), bytes })
    }
}

pub(super) struct ContextLease { credit: Arc<Credit>, bytes: usize }
impl Drop for ContextLease {
    fn drop(&mut self) { self.credit.bytes.fetch_sub(self.bytes, Ordering::AcqRel); }
}

pub(super) struct Lease { credit: Arc<Credit>, bytes: usize }
impl Drop for Lease {
    fn drop(&mut self) {
        self.credit.bytes.fetch_sub(self.bytes, Ordering::AcqRel);
        self.credit.roots.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(super) struct Notice {
    pub(super) tab: Uuid,
    pub(super) instance: Arc<()>,
    pub(super) effect: Effect,
    // Release logical credits after the immutable source payload is dropped.
    pub(super) _lease: Lease,
}

#[derive(Clone)]
pub(super) struct NameKey { bytes: [u8; 190], len: usize }
impl NameKey {
    pub(super) fn new(value: &[u8]) -> Self {
        let mut result = Self { bytes: [0; 190], len: 0 };
        for (index, byte) in value.iter().take(190).enumerate() {
            if *byte == 0 { break; }
            if let Some(slot) = result.bytes.get_mut(index) { *slot = *byte; result.len += 1; }
        }
        result
    }
    pub(super) fn bytes(&self) -> &[u8] { self.bytes.get(..self.len).unwrap_or(&[]) }
}

#[cfg(test)]
pub(super) struct DropProbe {
    pub(super) executor: gpui::BackgroundExecutor,
    pub(super) observed: Arc<std::sync::Mutex<Vec<bool>>>,
}
#[cfg(test)]
impl Drop for DropProbe {
    fn drop(&mut self) {
        if let Ok(mut observed) = self.observed.lock() { observed.push(self.executor.is_main_thread()); }
    }
}

pub(super) struct Row {
    pub(super) index: usize,
    pub(super) tab: Uuid,
    pub(super) instance: Arc<()>,
    pub(super) label: Option<SharedString>,
    pub(super) path_identity: Option<(usize, usize)>,
    pub(super) custom: bool,
    pub(super) revision: ContentRevision,
    pub(super) activity: ContentRevision,
    pub(super) key: NameKey,
    pub(super) accepted: Option<Candidate>,
    pub(super) publish: Option<SharedString>,
    #[cfg(test)] pub(super) _drop_probe: Option<DropProbe>,
    // Labels are released before their conservative logical credit.
    pub(super) _context_lease: Option<ContextLease>,
}

fn candidate(effect: &Effect, cancel: &AtomicBool, budget: &mut usize) -> Result<Option<Candidate>, &'static str> {
    match effect {
        Effect::Ready(name) => Ok(Some(name.clone())),
        Effect::Pending { rope, cursor } => {
            *budget = budget.checked_sub(cursor.scanned_bytes()).ok_or(history::WORK_LIMIT_ERROR)?;
            history::scan(rope, cursor.clone(), cancel, budget)
        },
        Effect::History { .. } => Ok(None),
    }
}

pub(super) fn reduce(mut rows: Vec<Row>, notices: Vec<Vec<Notice>>, cancel: &AtomicBool,
    mut budget: usize) -> Result<(Vec<Row>, usize), &'static str>
{
    for notice in notices.iter().flatten() {
        if cancel.load(Ordering::Relaxed) { return Err("Automatic naming cancelled"); }
        let Some(index) = rows.iter().position(|row| row.tab == notice.tab && Arc::ptr_eq(&row.instance, &notice.instance)) else { continue; };
        if rows.get(index).is_none_or(|row| row.custom || row.path_identity.is_some()) { continue; }
        let next = if let Effect::History { group, undo } = &notice.effect {
            let mut conflicts = Vec::new();
            conflicts.try_reserve_exact(rows.len().saturating_sub(1)).map_err(|_| "Not enough memory to prepare automatic names")?;
            for (other, row) in rows.iter().enumerate() {
                if other == index { continue; }
                let mut key = Vec::new(); key.try_reserve_exact(row.key.bytes().len()).map_err(|_| "Not enough memory to prepare automatic names")?;
                key.extend_from_slice(row.key.bytes()); conflicts.push(key);
            }
            let summary = history::summarize(group, *undo, &conflicts, cancel, &mut budget)?;
            let before = rows.get(index).map(|row| row.key.bytes()).ok_or("Automatic-name tab disappeared")?;
            // Stored Windows paths never reach this arm; untitled keys are UTF-8.
            let before = std::str::from_utf8(before).unwrap_or("");
            summary.resolve(before)?.cloned()
        } else { candidate(&notice.effect, cancel, &mut budget)? };
        let Some(next) = next else { continue; };
        let name = next.as_str()?;
        let mut conflict = false;
        for (other, row) in rows.iter().enumerate() {
            if other == index && matches!(notice.effect, Effect::History { .. }) { continue; }
            if cancel.load(Ordering::Relaxed) { return Err("Automatic naming cancelled"); }
            if crate::automatic_title::same_full_name(name, row.key.bytes()) { conflict = true; break; }
        }
        if !conflict {
            let Some(row) = rows.get_mut(index) else { return Err("Automatic-name tab disappeared"); };
            row.key = NameKey::new(name.as_bytes()); row.accepted = Some(next);
        }
    }
    for row in &mut rows {
        if cancel.load(Ordering::Relaxed) { return Err("Automatic naming cancelled"); }
        if let Some(candidate) = &row.accepted { row.publish = Some(candidate.to_string()?.into()); }
    }
    Ok((rows, budget))
}

pub(super) fn history_charge(group: &PreparedGroup) -> Option<usize> {
    group.expected_rope(false).len_bytes().checked_add(group.expected_rope(true).len_bytes())?
        .checked_add(group.retained_title_bytes()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automatic_title::{Cursor, Scan};
    fn name(text: &str) -> Candidate {
        match Cursor::new().advance(&ropey::Rope::from_str(text), 4096, &AtomicBool::new(false)).expect("normalizer") {
            Scan::Complete(Some(name)) => name, _ => panic!("short name"),
        }
    }
    fn row(index: usize, label: &str) -> Row {
        Row { index, tab: Uuid::new_v4(), instance: Arc::new(()), label: Some(label.to_owned().into()), path_identity: None,
            custom: false, revision: ContentRevision::default(), activity: ContentRevision::default(), key: NameKey::new(label.as_bytes()), accepted: None, publish: None, _drop_probe: None, _context_lease: None }
    }
    #[test]
    fn fifo_acceptance_includes_other_tabs_and_case_only_self_conflicts() {
        let rows = vec![row(0, "old"), row(1, "other")]; let credit = Arc::new(Credit::default());
        let notices = [(0, "shared"), (1, "SHARED"), (0, "different"), (1, "SHARED"), (0, "DIFFERENT")]
            .into_iter().map(|(index, label)| Notice { tab: rows[index].tab, instance: rows[index].instance.clone(),
                effect: Effect::Ready(name(label)), _lease: credit.admit(0).expect("admitted") }).collect();
        let (result, _) = reduce(rows, vec![notices], &AtomicBool::new(false), history::WORK_LIMIT).expect("reduce");
        assert_eq!(result[0].accepted.as_ref().expect("first").as_str().expect("utf8"), "different");
        assert_eq!(result[1].accepted.as_ref().expect("second").as_str().expect("utf8"), "SHARED");
        assert!(credit.available());
    }
    #[test]
    fn root_and_byte_exhaustion_do_not_release_active_credits_early() {
        let credit = Arc::new(Credit::default()); let lease = credit.admit(MAX_SOURCE_BYTES).expect("limit inclusive");
        assert!(credit.admit(1).is_err()); assert!(!credit.available()); drop(lease); assert!(credit.available());
        let leases: Vec<_> = (0..MAX_NOTICES).map(|_| credit.admit(0).expect("admitted")).collect();
        assert!(credit.admit(0).is_err()); drop(leases); assert!(credit.available());
        assert!(credit.admit(usize::MAX).is_err()); assert!(credit.available());
    }
    #[test]
    fn context_and_notification_credits_share_bytes_without_sharing_root_counts() {
        let credit = Arc::new(Credit::default());
        let notice = credit.admit(17).expect("notice");
        let label = credit.retain_context(MAX_SOURCE_BYTES - 17).expect("retained context exact limit");
        assert!(credit.retain_context(1).is_err()); assert!(credit.admit(1).is_err());
        assert!(!credit.available()); drop(notice); assert!(!credit.available());
        let notice = credit.admit(17).expect("released notice bytes reusable"); drop(label); assert!(!credit.available());
        drop(notice); assert!(credit.available());
        assert!(credit.retain_context(usize::MAX).is_err()); assert!(credit.available());
    }

}
