// Runtime recovery keys never enter the persisted schema. Preparation must run
// before restore/publication by the owned startup metadata worker.
use super::{SessionManifest, SessionTab, SessionClosedTab};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RecoveryIdentity {
    original: Option<Uuid>,
    order: usize,
    owner: Option<Owner>,
    claimed: bool,
    recovered: bool,
    mru_seen: bool,
}

#[derive(Clone, Copy, Debug)]
enum Owner { Open(usize), Closed(usize) }

impl RecoveryIdentity {
    pub(super) fn source_id(self, runtime_id: Uuid) -> Uuid { self.original.unwrap_or(runtime_id) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IdentityError { AlreadyPrepared, TooManyEntries, CandidatesExhausted, InvalidRecord }

impl std::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::AlreadyPrepared => "session recovery identities are already prepared",
            Self::TooManyEntries => "session recovery identity count exceeds the supported range",
            Self::CandidatesExhausted => "session recovery identity candidates are exhausted",
            Self::InvalidRecord => "session recovery identity record is inconsistent",
        })
    }
}
impl std::error::Error for IdentityError {}

#[derive(Debug)]
pub(crate) struct PreparedRecoveryIds { welcome: Uuid, welcome_replacement: Option<Uuid> }

impl PreparedRecoveryIds {
    // Only an empty constructor tab changes identity on this exceptional path.
    pub(crate) fn welcome_for_closed_fallback(&self, collides: bool) -> Result<Uuid, IdentityError> {
        if collides { self.welcome_replacement.ok_or(IdentityError::InvalidRecord) } else { Ok(self.welcome) }
    }
}

struct Candidates { bytes: [u8; 16], remaining: usize }

impl Candidates {
    fn new(seed: Uuid, remaining: usize) -> Self {
        let mut bytes = *seed.as_bytes();
        if let Some(byte) = bytes.get_mut(6) { *byte = (*byte & 0x0f) | 0x40; }
        if let Some(byte) = bytes.get_mut(8) { *byte = (*byte & 0x3f) | 0x80; }
        Self { bytes, remaining }
    }
    fn advance(&mut self) {
        // Carry through 122 payload bits, preserving v4 and RFC4122 variant.
        for (index, byte) in self.bytes.iter_mut().enumerate().rev() {
            let mask = match index { 6 => 0x0f, 8 => 0x3f, _ => 0xff };
            let low = *byte & mask;
            let next = low.checked_add(1).filter(|next| *next <= mask);
            *byte = (*byte & !mask) | next.unwrap_or(0);
            if next.is_some() { return; }
        }
    }
    fn next(&mut self, tabs: &[SessionTab], closed: &[SessionClosedTab], welcome: Uuid) -> Result<Uuid, IdentityError> {
        loop {
            self.remaining = self.remaining.checked_sub(1).ok_or(IdentityError::CandidatesExhausted)?;
            let id = Uuid::from_bytes(self.bytes);
            self.advance();
            if id != welcome
                && tabs.binary_search_by_key(&id, |t| t.recovery_identity.source_id(t.id)).is_err()
                && closed.binary_search_by_key(&id, |t| t.recovery_identity.source_id(t.id)).is_err() {
                return Ok(id);
            }
        }
    }
}

pub(crate) fn prepare(manifest: &mut SessionManifest, welcome: Uuid) -> Result<PreparedRecoveryIds, IdentityError> {
    let count = manifest.tabs.len().checked_add(manifest.closed_tabs.len()).ok_or(IdentityError::TooManyEntries)?;
    // At most N original keys + welcome are skipped, and N + 1 IDs emitted.
    // This is far below a complete 122-bit payload cycle on supported targets.
    let attempts = count.checked_mul(2).and_then(|n| n.checked_add(2)).ok_or(IdentityError::TooManyEntries)?;
    prepare_with_candidates(manifest, welcome, Candidates::new(Uuid::new_v4(), attempts))
}

fn prepare_with_candidates(manifest: &mut SessionManifest, welcome: Uuid, mut candidates: Candidates) -> Result<PreparedRecoveryIds, IdentityError> {
    if manifest.tabs.iter().any(|e| e.recovery_identity.original.is_some())
        || manifest.closed_tabs.iter().any(|e| e.recovery_identity.original.is_some()) {
        return Err(IdentityError::AlreadyPrepared);
    }
    for (order, tab) in manifest.tabs.iter_mut().enumerate() {
        tab.recovery_identity = RecoveryIdentity { original: Some(tab.id), order, ..Default::default() };
    }
    for (order, tab) in manifest.closed_tabs.iter_mut().enumerate() {
        tab.recovery_identity = RecoveryIdentity { original: Some(tab.id), order, ..Default::default() };
    }
    manifest.tabs.sort_unstable_by_key(|e| (e.recovery_identity.source_id(e.id), e.recovery_identity.order));
    manifest.closed_tabs.sort_unstable_by_key(|e| (e.recovery_identity.source_id(e.id), e.recovery_identity.order));
    let result = assign_sorted(manifest, welcome, &mut candidates);
    // Restore source order on success AND failure. No recovered Tab needs sorting.
    manifest.tabs.sort_unstable_by_key(|e| e.recovery_identity.order);
    manifest.closed_tabs.sort_unstable_by_key(|e| e.recovery_identity.order);
    if result.is_err() {
        for e in &mut manifest.tabs { e.id = e.recovery_identity.source_id(e.id); e.recovery_identity = RecoveryIdentity::default(); }
        for e in &mut manifest.closed_tabs { e.id = e.recovery_identity.source_id(e.id); e.recovery_identity = RecoveryIdentity::default(); }
    }
    result
}

fn assign_sorted(manifest: &mut SessionManifest, welcome: Uuid, candidates: &mut Candidates) -> Result<PreparedRecoveryIds, IdentityError> {
    let mut previous = None;
    let mut owner = None;
    for slot in 0..manifest.tabs.len() {
        let entry = manifest.tabs.get(slot).ok_or(IdentityError::InvalidRecord)?;
        let source = entry.recovery_identity.source_id(entry.id);
        let duplicate = previous == Some(source);
        if !duplicate { owner = Some(Owner::Open(entry.recovery_identity.order)); }
        let id = if duplicate { candidates.next(&manifest.tabs, &manifest.closed_tabs, welcome)? } else { source };
        let entry = manifest.tabs.get_mut(slot).ok_or(IdentityError::InvalidRecord)?;
        entry.id = id; entry.recovery_identity.owner = owner;
        previous = Some(source);
    }
    previous = None;
    let mut has_open = false;
    for slot in 0..manifest.closed_tabs.len() {
        let entry = manifest.closed_tabs.get(slot).ok_or(IdentityError::InvalidRecord)?;
        let source = entry.recovery_identity.source_id(entry.id);
        let duplicate = previous == Some(source);
        if !duplicate {
            let first = manifest.tabs.partition_point(|e| e.recovery_identity.source_id(e.id) < source);
            let open = manifest.tabs.get(first).filter(|e| e.recovery_identity.source_id(e.id) == source);
            has_open = open.is_some();
            owner = open.and_then(|e| e.recovery_identity.owner).or(Some(Owner::Closed(entry.recovery_identity.order)));
        }
        let id = if duplicate || has_open { candidates.next(&manifest.tabs, &manifest.closed_tabs, welcome)? } else { source };
        let entry = manifest.closed_tabs.get_mut(slot).ok_or(IdentityError::InvalidRecord)?;
        entry.id = id; entry.recovery_identity.owner = owner;
        previous = Some(source);
    }
    let welcome_replacement = if manifest.tabs.binary_search_by_key(&welcome, |e| e.recovery_identity.source_id(e.id)).is_ok()
        || manifest.closed_tabs.binary_search_by_key(&welcome, |e| e.recovery_identity.source_id(e.id)).is_ok() {
        Some(candidates.next(&manifest.tabs, &manifest.closed_tabs, welcome)?)
    } else { None };
    // Persisted duplicate IDs cannot encode individual recency. Retain the first
    // surviving original match; existing active-first/backfill policy follows.
    let SessionManifest { tabs, mru, .. } = manifest;
    mru.retain(|id| {
        let first = tabs.partition_point(|e| e.recovery_identity.source_id(e.id) < *id);
        let Some(entry) = tabs.get_mut(first).filter(|e| e.recovery_identity.source_id(e.id) == *id) else { return false; };
        let keep = !entry.recovery_identity.mru_seen;
        entry.recovery_identity.mru_seen = true;
        keep
    });
    for e in tabs { e.recovery_identity.mru_seen = false; }
    Ok(PreparedRecoveryIds { welcome, welcome_replacement })
}

impl SessionManifest {
    fn claim_owner(&mut self, identity: RecoveryIdentity) -> Result<bool, IdentityError> {
        let master = match identity.owner {
            Some(Owner::Open(i)) => self.tabs.get_mut(i).map(|e| &mut e.recovery_identity),
            Some(Owner::Closed(i)) => self.closed_tabs.get_mut(i).map(|e| &mut e.recovery_identity),
            None => None,
        }.ok_or(IdentityError::InvalidRecord)?;
        if identity.original.is_none() || master.original != identity.original { return Err(IdentityError::InvalidRecord); }
        let first = !master.claimed;
        master.claimed = true;
        Ok(first)
    }
    // Call only after the complete reader outcome succeeds, before publication.
    pub(crate) fn claim_recovered_open(&mut self, index: usize) -> Result<Uuid, IdentityError> {
        let entry = self.tabs.get(index).ok_or(IdentityError::InvalidRecord)?;
        let (identity, candidate) = (entry.recovery_identity, entry.id);
        if identity.recovered { return Ok(candidate); }
        let id = if self.claim_owner(identity)? { identity.source_id(candidate) } else { candidate };
        let entry = self.tabs.get_mut(index).ok_or(IdentityError::InvalidRecord)?;
        entry.id = id; entry.recovery_identity.recovered = true;
        Ok(id)
    }
    pub(crate) fn claim_recovered_closed(&mut self, index: usize) -> Result<Uuid, IdentityError> {
        let entry = self.closed_tabs.get(index).ok_or(IdentityError::InvalidRecord)?;
        let (identity, candidate) = (entry.recovery_identity, entry.id);
        if identity.recovered { return Ok(candidate); }
        let id = if self.claim_owner(identity)? { identity.source_id(candidate) } else { candidate };
        let entry = self.closed_tabs.get_mut(index).ok_or(IdentityError::InvalidRecord)?;
        entry.id = id; entry.recovery_identity.recovered = true;
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{self, SessionTab, SessionClosedTab, RestoreOutcome};
    use crate::persist::TempDir;

    fn open(original: Uuid, runtime: Uuid) -> SessionTab {
        let mut e: SessionTab = toml::from_str(&format!("id = '{runtime}'\ndirty = true")).expect("entry");
        e.recovery_identity.original = Some(original);
        e
    }
    fn restored(e: &SessionTab, dir: &std::path::Path) -> crate::tab::Tab {
        match session::restore_tab_in(e, dir) {
            RestoreOutcome::Restored(tab) => tab,
            RestoreOutcome::Skipped(reason) => panic!("skipped: {reason}"),
        }
    }
    #[test]
    fn recovery_identity_remapped_reader_uses_original_backup_and_history() {
        use crate::buffer::{EditOp, UndoEntry, ContentRevision, serialize_undo_stack};
        let dir = TempDir::new("identity-original-backup");
        let original = Uuid::new_v4(); let runtime = Uuid::new_v4();
        let e = open(original, runtime);
        let text = "saved \u{e9}\r\n\u{4e2d}";
        std::fs::write(session::bak_path_in(dir.path(), original), text).expect("backup");
        let history = vec![UndoEntry::Single(EditOp {
            byte_start: 0, removed: "before".into(), inserted: text.into(),
            sel_before_anchor: 0, sel_before_head: 0,
            before_revision: ContentRevision::default(), after_revision: ContentRevision::default(),
        })];
        std::fs::write(session::undo_path_in(dir.path(), original), serialize_undo_stack(&history)).expect("history");
        let tab = restored(&e, dir.path());
        assert_eq!(tab.session_id, runtime); assert_eq!(tab.buffer.rope.to_string(), text);
        assert_eq!(tab.undo_stack.len(), 1); assert!(tab.buffer.dirty);
        let mut closed: SessionClosedTab = toml::from_str(&format!("id = '{runtime}'\ndirty = true")).expect("closed");
        closed.recovery_identity.original = Some(original);
        let closed = session::restore_closed_tab_in(&closed, dir.path()).expect("closed backup");
        assert_eq!(closed.session_id, runtime); assert_eq!(closed.buffer.rope.to_string(), text);
    }
    #[test]
    fn recovery_identity_remapped_reader_uses_original_native_and_raw_fallback() {
        let dir = TempDir::new("identity-native");
        let original = Uuid::new_v4(); let runtime = Uuid::new_v4();
        let path = dir.join("source.rtf"); let raw = r"{\rtf1\ansi recovered source}";
        std::fs::write(&path, raw).expect("native");
        let payload = crate::rich_doc::open(&path).expect("payload");
        session::rich_backup::Capture::new(&payload).write(&session::rich_backup::path_in(dir.path(), original), raw).expect("native backup");
        let mut e = open(original, runtime); e.rich_backup = true; e.path = Some(path.clone());
        let tab = restored(&e, dir.path());
        assert_eq!(tab.session_id,runtime); assert!(tab.buffer.rich_doc.is_some());
        let mut closed: SessionClosedTab = toml::from_str(&format!("id = '{runtime}'\ndirty = true\nrich_backup = true")).expect("closed");
        closed.path = Some(path.clone()); closed.recovery_identity.original = Some(original);
        assert!(session::restore_closed_tab_in(&closed, dir.path()).expect("closed native").buffer.rich_doc.is_some());
        std::fs::remove_file(session::rich_backup::path_in(dir.path(), original)).expect("remove rich");
        let fallback = "raw \u{e9}\r\n";
        std::fs::write(session::bak_path_in(dir.path(), original), fallback).expect("raw");
        let tab = restored(&e, dir.path()); assert_eq!(tab.buffer.rope.to_string(),fallback);
        assert!(tab.buffer.file_path.is_none()); assert!(tab.buffer.rich_doc.is_none());
        let raw_closed = session::restore_closed_tab_in(&closed, dir.path()).expect("closed raw");
        assert_eq!(raw_closed.buffer.rope.to_string(),fallback); assert!(raw_closed.buffer.file_path.is_none());
        e.rich_backup = false;
        std::fs::write(session::bak_path_in(dir.path(), original), raw).expect("legacy native");
        std::fs::remove_file(&path).expect("remove original");
        assert!(restored(&e,dir.path()).buffer.rich_doc.is_some(),"legacy RTF backup read through original key");
        closed.rich_backup=false;
        assert!(session::restore_closed_tab_in(&closed,dir.path()).expect("legacy closed RTF").buffer.rich_doc.is_some());
    }
    #[test]
    fn recovery_identity_csv_basis_namespace_and_raw_fallback() {
        let dir = TempDir::new("identity-csv");
        let original=Uuid::new_v4(); let runtime=Uuid::new_v4();
        let mut e=open(original,runtime); e.path=Some(dir.join("missing.csv"));
        let source="name,value\r\n\u{e9},1\r\n";
        std::fs::write(session::csv_basis_path_in(dir.path(),runtime),source).expect("basis equal runtime UUID");
        std::fs::write(session::bak_path_in(dir.path(),original),"raw fallback").expect("raw");
        e.csv_draft=Some(session::CsvDraft { backup_id:Some(runtime), source_digest:Some(session::csv_source_digest(source)), source_matches:true, value:"draft".into(), ..Default::default() });
        let tab=restored(&e,dir.path()); assert_eq!(tab.session_id,runtime);
        assert_eq!(tab.buffer.rope.to_string(),source); assert!(tab.csv_draft.as_ref().expect("draft").source_matches);
        std::fs::remove_file(session::csv_basis_path_in(dir.path(),runtime)).expect("remove basis");
        let tab=restored(&e,dir.path()); assert_eq!(tab.buffer.rope.to_string(),"raw fallback");
        assert!(!tab.csv_draft.as_ref().expect("draft").source_matches);
    }

    fn manifest(open_ids: &[Uuid], closed_ids: &[Uuid]) -> SessionManifest {
        SessionManifest {
            tabs: open_ids.iter().map(|id| toml::from_str(&format!("id = '{id}'\nnew_label = 'open'")).expect("open")).collect(),
            closed_tabs: closed_ids.iter().map(|id| toml::from_str(&format!("id = '{id}'\nnew_label = 'closed'")).expect("closed")).collect(),
            ..Default::default()
        }
    }
    fn seeded(m: &mut SessionManifest, welcome: Uuid, seed: Uuid) -> PreparedRecoveryIds {
        let n = m.tabs.len()+m.closed_tabs.len();
        prepare_with_candidates(m,welcome,Candidates::new(seed,2*n+2)).expect("prepare")
    }
    #[test]
    fn recovery_identity_prepare_preserves_order_and_first_success_across_lists() {
        let a=Uuid::from_u128(11); let b=Uuid::from_u128(7); let c=Uuid::from_u128(19);
        let welcome=Uuid::new_v4();
        let mut m=manifest(&[a,b,a,a], &[a,c,c,c]);
        m.active=3; m.mru=vec![a,a,c,b,a,Uuid::nil()];
        let capacities=(m.tabs.capacity(),m.closed_tabs.capacity(),m.mru.capacity());
        let labels:[usize;4]=std::array::from_fn(|i| m.tabs[i].new_label.as_ref().expect("label").as_ptr() as usize);
        seeded(&mut m,welcome,Uuid::nil());
        assert_eq!(capacities,(m.tabs.capacity(),m.closed_tabs.capacity(),m.mru.capacity()));
        for (i,e) in m.tabs.iter().enumerate() { assert_eq!(labels[i],e.new_label.as_ref().expect("label").as_ptr() as usize); }
        assert_eq!(m.tabs.iter().map(|e|e.recovery_identity.source_id(e.id)).collect::<Vec<_>>(),vec![a,b,a,a]);
        assert_eq!(m.closed_tabs.iter().map(|e|e.recovery_identity.source_id(e.id)).collect::<Vec<_>>(),vec![a,c,c,c]);
        assert_eq!(m.active,3); assert_eq!(m.mru,vec![a,b]);
        // Skip open[0] and closed[1]. The retained failed masters still arbitrate.
        assert_eq!(m.claim_recovered_open(1).expect("b"),b);
        assert_eq!(m.claim_recovered_open(2).expect("a"),a);
        let later=m.claim_recovered_open(3).expect("later a"); assert_ne!(later,a);
        assert_eq!(m.claim_recovered_open(2).expect("repeat"),a);
        let closed_a=m.claim_recovered_closed(0).expect("closed a"); assert_ne!(closed_a,a);
        assert_eq!(m.claim_recovered_closed(2).expect("c"),c);
        let closed_c=m.claim_recovered_closed(3).expect("later c"); assert_ne!(closed_c,c);
        let mut ids=vec![b,a,later,closed_a,c,closed_c]; let count=ids.len(); ids.sort_unstable(); ids.dedup(); assert_eq!(ids.len(),count);
        assert_eq!(prepare(&mut m,welcome).unwrap_err(),IdentityError::AlreadyPrepared);
    }
    #[test]
    fn recovery_identity_all_open_skipped_closed_claims_and_welcome_collision() {
        let welcome=Uuid::new_v4();
        let mut m=manifest(&[welcome,welcome], &[welcome,welcome]);
        let prepared=seeded(&mut m,welcome,welcome);
        assert_eq!(m.claim_recovered_closed(0).expect("first closed"),welcome);
        let second=m.claim_recovered_closed(1).expect("second closed"); assert_ne!(second,welcome);
        let replacement=prepared.welcome_for_closed_fallback(true).expect("replacement");
        assert_ne!(replacement,welcome); assert_ne!(replacement,second);
        assert_eq!(prepared.welcome_for_closed_fallback(false).expect("normal welcome"),welcome);
        let mut absent=manifest(&[],&[]);
        assert_eq!(seeded(&mut absent,welcome,Uuid::nil()).welcome_for_closed_fallback(true).unwrap_err(),IdentityError::InvalidRecord);
    }
    #[test]
    fn recovery_identity_generator_preserves_v4_variant_wrap_and_reserved_originals() {
        let mut generator=Candidates::new(Uuid::from_bytes([0xff;16]),3);
        let mut expected=[0xff;16]; expected[6]=0x4f; expected[8]=0xbf;
        assert_eq!(generator.next(&[],&[],Uuid::nil()).expect("max payload"),Uuid::from_bytes(expected));
        let mut zero=[0;16]; zero[6]=0x40; zero[8]=0x80;
        let zero=Uuid::from_bytes(zero);
        assert_eq!(generator.next(&[],&[],Uuid::nil()).expect("wrap"),zero);
        let next=generator.next(&[],&[],Uuid::nil()).expect("next");
        assert_eq!(next.get_version_num(),4); assert_eq!(next.get_variant(),uuid::Variant::RFC4122);
        assert_eq!(generator.next(&[],&[],Uuid::nil()).unwrap_err(),IdentityError::CandidatesExhausted);
        let mut m=manifest(&[Uuid::nil(),Uuid::nil(),zero,Uuid::from_bytes([0xff;16])], &[zero,Uuid::from_bytes([0xff;16])]);
        seeded(&mut m,next,zero);
        // Output mutation does not disturb immutable original-key searches.
        let originals=[Uuid::nil(),zero,Uuid::from_bytes([0xff;16]),next];
        for e in &m.tabs {
            if e.id!=e.recovery_identity.source_id(e.id) { assert!(!originals.contains(&e.id)); assert_eq!(e.id.get_version_num(),4); }
        }
        for e in &m.closed_tabs { assert!(!originals.contains(&e.id)); assert_eq!(e.id.get_variant(),uuid::Variant::RFC4122); }
        assert_eq!(m.claim_recovered_open(0).expect("nil original"),Uuid::nil());
        assert_eq!(m.claim_recovered_open(3).expect("max original"),Uuid::from_bytes([0xff;16]));
    }
    #[test]
    fn recovery_identity_candidate_exhaustion_rolls_back_without_mru_or_order_loss() {
        let a=Uuid::from_u128(20); let b=Uuid::from_u128(2);
        let mut m=manifest(&[a,b,a,a], &[b,a]); m.mru=vec![a,b,a];
        let before=toml::to_string(&m).expect("before");
        assert_eq!(prepare_with_candidates(&mut m,Uuid::new_v4(),Candidates::new(Uuid::nil(),1)).unwrap_err(),IdentityError::CandidatesExhausted);
        assert_eq!(toml::to_string(&m).expect("after"),before);
        assert!(m.tabs.iter().all(|e| e.recovery_identity.original.is_none()));
        assert!(m.closed_tabs.iter().all(|e| e.recovery_identity.original.is_none()));
        prepare(&mut m,Uuid::new_v4()).expect("retry after rollback");
        assert_eq!(m.claim_recovered_open(usize::MAX).unwrap_err(),IdentityError::InvalidRecord);
        assert_eq!(m.claim_recovered_closed(usize::MAX).unwrap_err(),IdentityError::InvalidRecord);
    }
    #[test]
    fn recovery_identity_old_schema_and_unprepared_reader_defaults_are_unchanged() {
        let dir=TempDir::new("identity-legacy-schema"); let id=Uuid::new_v4();
        let mut m:SessionManifest=toml::from_str(&format!("active=0\n[[tabs]]\nid='{id}'\ndirty=true\n[[closed_tabs]]\nid='{id}'\ndirty=true")).expect("old schema");
        std::fs::write(session::bak_path_in(dir.path(),id),"legacy \u{e9}\r\n").expect("backup");
        assert_eq!(restored(&m.tabs[0],dir.path()).session_id,id);
        assert_eq!(session::restore_closed_tab_in(&m.closed_tabs[0],dir.path()).expect("closed").session_id,id);
        prepare(&mut m,Uuid::new_v4()).expect("prepare");
        let encoded=toml::to_string(&m).expect("encode");
        for field in ["recovery_identity","original","owner","claimed","recovered","mru_seen"] { assert!(!encoded.contains(field),"{field}"); }
        let decoded:SessionManifest=toml::from_str(&encoded).expect("roundtrip");
        assert!(decoded.tabs[0].recovery_identity.original.is_none());
        assert!(decoded.closed_tabs[0].recovery_identity.original.is_none());
        let snapshot=session::snapshot_tab_with_policy(&crate::tab::Tab::empty(),session::BakPolicy::Full);
        assert!(snapshot.recovery_identity.original.is_none());
    }
    #[test]
    fn recovery_identity_prepared_actual_reader_then_success_claim_retains_source() {
        let dir=TempDir::new("identity-prepared-readers"); let original=Uuid::new_v4();
        let mut m=manifest(&[original,original],&[original,original]);
        m.tabs[1].dirty=true; m.tabs[1].read_only=true; m.tabs[1].cursor_head=usize::MAX;
        for closed in &mut m.closed_tabs { closed.dirty=Some(true); }
        let text="original recovery \u{e9}\r\n\u{4e2d}";
        std::fs::write(session::bak_path_in(dir.path(),original),text).expect("source");
        prepare(&mut m,Uuid::new_v4()).expect("prepare");
        assert!(matches!(session::restore_tab_in(&m.tabs[0],dir.path()),RestoreOutcome::Skipped(_)));
        let mut tab=restored(&m.tabs[1],dir.path());
        assert_ne!(tab.session_id,original); assert_eq!(tab.buffer.rope.to_string(),text);
        assert!(tab.buffer.read_only&&tab.buffer.dirty); assert_eq!(tab.selection.head,text.len());
        tab.session_id=m.claim_recovered_open(1).expect("first success"); assert_eq!(tab.session_id,original);
        let mut closed=session::restore_closed_tab_in(&m.closed_tabs[0],dir.path()).expect("closed");
        assert_eq!(closed.buffer.rope.to_string(),text);
        closed.session_id=m.claim_recovered_closed(0).expect("closed success"); assert_ne!(closed.session_id,original);
        let second=session::restore_closed_tab_in(&m.closed_tabs[1],dir.path()).expect("later closed");
        assert_ne!(second.session_id,original); assert_ne!(second.session_id,closed.session_id);
        println!("RecoveryIdentity inline bytes={}, SessionTab bytes={}, SessionClosedTab bytes={}",std::mem::size_of::<RecoveryIdentity>(),std::mem::size_of::<SessionTab>(),std::mem::size_of::<SessionClosedTab>());
    }
}
