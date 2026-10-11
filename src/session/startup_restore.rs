// Read recovery sources before an editor entity can register save hooks.
// Metadata preparation runs separately; body IO/import remains foreground.
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

impl RestoredSession {
    // Refusal retains the owned manifest/buffers for background retirement.
    // Boxing this error would add an infallible allocation to that failure path.
    #[allow(clippy::result_large_err)]
    pub(crate) fn read(manifest: SessionManifest, ids: PreparedRecoveryIds, welcome: Uuid) -> Result<Self, ReadFailure> {
        let mut restored = Self { manifest, tabs: Vec::new(), closed: Vec::new(), welcome,
            skipped: 0, raw_recoveries: 0, repaired: 0 };
        match restored.read_into(ids) {
            Ok(()) => Ok(restored),
            Err(error) => Err(ReadFailure { error, retained: restored }),
        }
    }

    fn read_into(&mut self, ids: PreparedRecoveryIds) -> anyhow::Result<()> {
        self.tabs.try_reserve_exact(self.manifest.tabs.len())?;
        self.closed.try_reserve_exact(self.manifest.closed_tabs.len())?;
        let original_active = self.manifest.active;
        let mut restored_active = None;
        for index in 0..self.manifest.tabs.len() {
            let entry = self.manifest.tabs.get(index).ok_or(super::recovery_identity::IdentityError::InvalidRecord)?;
            let original = entry.recovery_identity.source_id(entry.id);
            match super::restore_tab(entry) {
                RestoreOutcome::Restored(mut tab) => {
                    if entry.rich_backup && tab.buffer.rich_doc.is_none() { self.raw_recoveries += 1; }
                    tab.session_id = match self.manifest.claim_recovered_open(index) {
                        Ok(id) => id,
                        Err(err) => { self.tabs.push(tab); return Err(err.into()); }
                    };
                    if tab.session_id != original { self.repaired += 1; }
                    if index == original_active { restored_active = Some(self.tabs.len()); }
                    self.tabs.push(tab);
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
            if let Some(mut tab) = super::restore_closed_tab(entry) {
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
