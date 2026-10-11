// Read recovery sources before an editor entity can register save hooks.
// Startup reads bodies on its admitted worker; callers supply captured sources.
use super::{SessionManifest, RestoreOutcome};
use super::recovery_identity::PreparedRecoveryIds;
use crate::tab::{Tab, ClosedTab};
use uuid::Uuid;

pub(crate) struct RestoredSession {
    pub(crate) manifest: SessionManifest,
    pub(crate) tabs: Vec<Tab>,
    pub(crate) closed: Vec<ClosedTab>,
    pub(crate) welcome: Uuid,
    pub(crate) skipped: usize,
    pub(crate) raw_recoveries: usize,
    pub(crate) repaired: usize,
}

pub(crate) struct ReadFailure {
    pub(crate) error: anyhow::Error,
    pub(crate) retained: RestoredSession,
}

#[cfg(test)]
pub(crate) struct ReadProbe {
    pub(crate) executor: gpui::BackgroundExecutor,
    pub(crate) affinity: std::sync::mpsc::Sender<bool>,
    pub(crate) fail_after_first: bool,
}

impl RestoredSession {
    #[cfg(test)]
    // The synchronous test/legacy adoption seam captures its own source root.
    #[allow(clippy::result_large_err)]
    pub(crate) fn read(manifest: SessionManifest, ids: PreparedRecoveryIds, welcome: Uuid) -> Result<Self, ReadFailure> {
        let root = super::capture_restore_backup_root();
        Self::read_in(manifest, ids, welcome, root.as_deref(), None)
    }

    // Refusal retains the owned manifest/buffers for background retirement.
    // Boxing this error would add an infallible allocation to that failure path.
    #[allow(clippy::result_large_err)]
    pub(crate) fn read_in(manifest: SessionManifest, ids: PreparedRecoveryIds, welcome: Uuid,
        root: Option<&std::path::Path>, #[cfg(test)] probe: Option<ReadProbe>) -> Result<Self, ReadFailure> {
        #[cfg(test)]
        if let Some(probe) = probe.as_ref() { let _ = probe.affinity.send(probe.executor.is_main_thread()); }
        let mut restored = Self { manifest, tabs: Vec::new(), closed: Vec::new(), welcome,
            skipped: 0, raw_recoveries: 0, repaired: 0 };
        match restored.read_into(ids, root, #[cfg(test)] probe) {
            Ok(()) => Ok(restored),
            Err(error) => Err(ReadFailure { error, retained: restored }),
        }
    }

    fn read_into(&mut self, ids: PreparedRecoveryIds, root: Option<&std::path::Path>, #[cfg(test)] probe: Option<ReadProbe>) -> anyhow::Result<()> {
        self.tabs.try_reserve_exact(self.manifest.tabs.len())?;
        self.closed.try_reserve_exact(self.manifest.closed_tabs.len())?;
        let original_active = self.manifest.active;
        let mut restored_active = None;
        for index in 0..self.manifest.tabs.len() {
            let entry = self.manifest.tabs.get(index).ok_or(super::recovery_identity::IdentityError::InvalidRecord)?;
            let original = entry.recovery_identity.source_id(entry.id);
            match root.map_or_else(|| RestoreOutcome::Skipped("no config directory".into()), |root| super::restore_tab_in(entry, root)) {
                RestoreOutcome::Restored(mut tab) => {
                    if entry.rich_backup && tab.buffer.rich_doc.is_none() { self.raw_recoveries += 1; }
                    tab.session_id = match self.manifest.claim_recovered_open(index) {
                        Ok(id) => id,
                        Err(err) => { self.tabs.push(tab); return Err(err.into()); }
                    };
                    if tab.session_id != original { self.repaired += 1; }
                    if index == original_active { restored_active = Some(self.tabs.len()); }
                    self.tabs.push(tab);
                    #[cfg(test)]
                    if probe.as_ref().is_some_and(|probe| probe.fail_after_first) {
                        anyhow::bail!("injected refusal after a real recovered body");
                    }
                }
                RestoreOutcome::Skipped(reason) => {
                    eprintln!("[session] skipped tab: {reason}");
                    self.skipped += 1;
                }
            }
        }
        // If the selected original failed, adoption keeps its existing clamped
        // fallback. A successful selected entry follows its filtered position.
        if let Some(active) = restored_active { self.manifest.active = active; }
        for index in 0..self.manifest.closed_tabs.len() {
            let entry = self.manifest.closed_tabs.get(index).ok_or(super::recovery_identity::IdentityError::InvalidRecord)?;
            let original = entry.recovery_identity.source_id(entry.id);
            if let Some(mut tab) = root.and_then(|root| super::restore_closed_tab_in(entry, root)) {
                tab.session_id = match self.manifest.claim_recovered_closed(index) {
                    Ok(id) => id,
                    Err(err) => { self.closed.push(tab); return Err(err.into()); }
                };
                if tab.session_id != original { self.repaired += 1; }
                self.closed.push(tab);
            }
        }
        let collides = self.tabs.iter().any(|tab| tab.session_id == self.welcome)
            || self.closed.iter().any(|tab| tab.session_id == self.welcome);
        self.welcome = ids.welcome_for_closed_fallback(collides)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_reader_missing_root_keeps_skips_and_one_welcome() {
        let id = Uuid::new_v4(); let welcome = Uuid::new_v4();
        let mut manifest: SessionManifest = toml::from_str(&format!("[[tabs]]\nid='{id}'\ndirty=true\n[[closed_tabs]]\nid='{id}'\ndirty=true")).unwrap();
        let ids = super::super::recovery_identity::prepare(&mut manifest, welcome).unwrap();
        let restored = RestoredSession::read_in(manifest, ids, welcome, None, None).map_err(|failure| failure.error).unwrap();
        assert!(restored.tabs.is_empty()); assert!(restored.closed.is_empty());
        assert_eq!(restored.skipped, 1); assert_eq!(restored.welcome, welcome);
    }
}
