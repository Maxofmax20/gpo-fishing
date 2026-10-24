//! Update intent is local, short-lived and consumed once by the exact target.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeIntent {
    pub request_id: String,
    pub source: String,
    pub target: String,
    pub created_ms: u64,
    pub was_running: bool,
}

impl ResumeIntent {
    pub fn valid_for(&self, version: &str, now_ms: u64) -> bool {
        let Ok(source) = semver::Version::parse(&self.source) else { return false; };
        let Ok(target) = semver::Version::parse(&self.target) else { return false; };
        !self.request_id.is_empty() && self.was_running && target > source
            && self.target == version && now_ms >= self.created_ms
            && now_ms - self.created_ms <= 10 * 60 * 1000
    }
}

#[derive(Default)]
pub struct UpdateGate {
    active: Option<ResumeIntent>,
    safe: bool,
    prepared: bool,
}
impl UpdateGate {
    pub fn begin(&mut self, intent: ResumeIntent) -> Result<(), String> {
        if self.active.is_some() { return Err("Another update is already pending".into()); }
        let source = semver::Version::parse(&intent.source).map_err(|_| "Invalid current version")?;
        let target = semver::Version::parse(&intent.target).map_err(|_| "Invalid update version")?;
        if target <= source { return Err("Update must be newer than this version".into()); }
        self.safe = !intent.was_running;
        self.prepared = false;
        self.active = Some(intent);
        Ok(())
    }
    pub fn pending(&self) -> bool { self.active.is_some() }
    pub fn boundary(&mut self) -> bool {
        if self.active.is_none() { return false; }
        self.safe = true;
        true
    }
    pub fn ready(&self, id: &str) -> Result<bool, String> {
        if self.active.as_ref().map_or(true, |intent| intent.request_id != id) {
            return Err("Update cancelled by a newer Start, Pause or Stop action".into());
        }
        Ok(self.safe)
    }
    pub fn prepare(&mut self, id: &str) -> Result<ResumeIntent, String> {
        if !self.ready(id)? { return Err("Current reel is not finished".into()); }
        if self.prepared { return Err("Update is already prepared".into()); }
        self.prepared = true;
        Ok(self.active.clone().expect("ready checked active intent"))
    }
    pub fn cancel(&mut self, id: Option<&str>) -> Option<(ResumeIntent, bool)> {
        if id.is_some_and(|id| self.active.as_ref().map_or(true, |intent| intent.request_id != id)) { return None; }
        let prepared = self.prepared;
        self.safe = false;
        self.prepared = false;
        self.active.take().map(|intent| (intent, prepared))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn intent() -> ResumeIntent { ResumeIntent { request_id:"one".into(), source:"5.9.2".into(), target:"5.9.3".into(),created_ms:100,was_running:true } }
    #[test] fn running_waits_for_boundary_and_cancel_invalidates_resume() {
        let mut gate = UpdateGate::default(); gate.begin(intent()).unwrap();
        assert_eq!(gate.ready("one"),Ok(false)); assert!(gate.prepare("one").is_err());
        assert!(gate.boundary()); assert!(gate.prepare("one").unwrap().was_running);
        assert!(gate.cancel(None).is_some()); assert!(gate.ready("one").is_err());
        assert!(gate.cancel(Some("one")).is_none());
    }
    #[test] fn stopped_update_does_not_request_resume() {
        let mut gate=UpdateGate::default(); let mut i=intent();i.was_running=false;
        gate.begin(i).unwrap(); assert_eq!(gate.ready("one"),Ok(true));
        assert!(!gate.prepare("one").unwrap().was_running);
    }
    #[test] fn checkpoint_requires_exact_fresh_newer_target() {
        let i=intent();assert!(i.valid_for("5.9.3",200));
        assert!(!i.valid_for("5.9.2",200));assert!(!i.valid_for("5.9.4",200));
        assert!(!i.valid_for("5.9.3",99));assert!(!i.valid_for("5.9.3",700_000));
        let mut stale=i;stale.source="5.9.3".into();assert!(!stale.valid_for("5.9.3",200));
    }
    #[test] fn a_stale_failure_cannot_cancel_a_new_update() {
        let mut gate=UpdateGate::default();gate.begin(intent()).unwrap();gate.cancel(None);
        let mut next=intent();next.request_id="two".into();gate.begin(next).unwrap();
        assert!(gate.cancel(Some("one")).is_none());assert_eq!(gate.ready("two"),Ok(false));
    }
}
