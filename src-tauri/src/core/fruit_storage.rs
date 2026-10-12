//! Local, bounded fruit workflow. Network services never authorize inputs.
use super::fruit::{fruit_rarity, is_pity_zero, DropInfo, StorageBannerResult};
use super::policy::{decide_fruit_policy, PolicyDecision};

#[derive(Debug, Clone)]
pub struct FruitCatchContext {
    pub name: String,
    pub pity: Option<String>,
    pub pity_zero: bool,
    pub policy: PolicyDecision,
}

impl FruitCatchContext {
    pub fn new(drop: &DropInfo, raw: &str, never_drop_high: bool, keep_zero: bool) -> Self {
        let name = drop.name.clone().unwrap_or_else(|| "Devil Fruit".into());
        // A positive zero observation is sticky, even if another reading conflicts.
        let pity_zero = is_pity_zero(drop.pity.as_deref(), "") || is_pity_zero(None, raw);
        Self {
            policy: decide_fruit_policy(fruit_rarity(&name), pity_zero, never_drop_high, keep_zero),
            name,
            pity: drop.pity.clone(),
            pity_zero,
        }
    }
    pub fn high_tier(&self) -> bool {
        self.pity_zero || fruit_rarity(&self.name).is_high_tier()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageOutcome {
    Stored,
    ProtectedKept,
    Dropped,
    Failed(String),
    Unknown,
    Cancelled,
}
impl StorageOutcome {
    pub fn label(&self) -> &str {
        match self {
            Self::Stored => "Stored",
            Self::ProtectedKept => "Protected fruit kept in slot",
            Self::Dropped => "Dropped on ground",
            Self::Failed(_) => "Storage failed",
            Self::Unknown => "Outcome unconfirmed — inspect inventory",
            Self::Cancelled => "Storage cancelled",
        }
    }
    pub fn detail(&self) -> String {
        match self {
            Self::Failed(reason) => format!("{}: {reason}", self.label()),
            _ => self.label().into(),
        }
    }
    pub fn may_continue(&self) -> bool {
        matches!(self, Self::Stored | Self::Dropped)
    }
}

pub trait StorageIo {
    fn active(&self) -> bool;
    fn attempt_store(&mut self) -> Result<Option<StorageBannerResult>, ()>;
    fn drop_once(&mut self) -> bool;
    fn observe_drop(&mut self) -> Result<Option<StorageBannerResult>, ()>;
}

pub fn run_storage(io: &mut impl StorageIo, context: &FruitCatchContext) -> StorageOutcome {
    if !io.active() {
        return StorageOutcome::Cancelled;
    }
    let banner = match io.attempt_store() {
        Ok(b) => b,
        Err(()) => return StorageOutcome::Cancelled,
    };
    match banner {
        // Game contract: a completed store click only banners on failure.
        None | Some(StorageBannerResult::Stored) => StorageOutcome::Stored,
        Some(
            StorageBannerResult::Dropped { .. } | StorageBannerResult::DuplicateDropped { .. },
        ) => StorageOutcome::Dropped,
        Some(StorageBannerResult::Failed { reason }) => StorageOutcome::Failed(reason),
        Some(StorageBannerResult::Duplicate { .. }) => {
            if !context.policy.action.drop_permitted() {
                return StorageOutcome::ProtectedKept;
            }
            // Recheck liveness and the immutable catch policy at the destructive boundary.
            if !io.active() || !context.policy.action.drop_permitted() || !io.drop_once() {
                return StorageOutcome::Cancelled;
            }
            match io.observe_drop() {
                Ok(Some(
                    StorageBannerResult::Dropped { .. }
                    | StorageBannerResult::DuplicateDropped { .. },
                )) => StorageOutcome::Dropped,
                Ok(_) => StorageOutcome::Unknown,
                Err(()) => StorageOutcome::Cancelled,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fake {
        banner: Option<StorageBannerResult>,
        after: Option<StorageBannerResult>,
        drops: u32,
        active: bool,
        cancel_after_store: bool,
    }
    impl StorageIo for Fake {
        fn active(&self) -> bool {
            self.active
        }
        fn attempt_store(&mut self) -> Result<Option<StorageBannerResult>, ()> {
            if self.cancel_after_store {
                self.active = false;
            }
            Ok(self.banner.take())
        }
        fn drop_once(&mut self) -> bool {
            self.drops += 1;
            true
        }
        fn observe_drop(&mut self) -> Result<Option<StorageBannerResult>, ()> {
            Ok(self.after.take())
        }
    }
    fn context(raw: &str) -> FruitCatchContext {
        let d = DropInfo {
            name: None,
            pity: None,
            text: raw.into(),
            is_legendary: false,
            custom_telegram_message: None,
        };
        FruitCatchContext::new(&d, raw, true, true)
    }
    fn io() -> Fake {
        Fake {
            banner: Some(StorageBannerResult::Duplicate {
                fruit_name: "Devil Fruit".into(),
            }),
            after: Some(StorageBannerResult::Dropped {
                fruit_name: "Devil Fruit".into(),
            }),
            drops: 0,
            active: true,
            cancel_after_store: false,
        }
    }
    #[test]
    fn protected_never_drops() {
        for raw in ["Pity 0/40", "Pity 0/100"] {
            let mut f = io();
            assert_eq!(
                run_storage(&mut f, &context(raw)),
                StorageOutcome::ProtectedKept
            );
            assert_eq!(f.drops, 0);
        }
    }
    #[test]
    fn unknown_pity_drops_once_and_continues() {
        let mut f = io();
        let o = run_storage(&mut f, &context("Pity unreadable"));
        assert_eq!(o, StorageOutcome::Dropped);
        assert!(o.may_continue());
        assert_eq!(f.drops, 1);
    }
    #[test]
    fn store_success_has_no_cleanup_input() {
        let mut f = io();
        f.banner = None;
        assert_eq!(
            run_storage(&mut f, &context("4/40")),
            StorageOutcome::Stored
        );
        assert_eq!(f.drops, 0);
    }
    #[test]
    fn already_dropped_has_no_more_inputs() {
        let mut f = io();
        f.banner = f.after.take();
        assert_eq!(
            run_storage(&mut f, &context("4/40")),
            StorageOutcome::Dropped
        );
        assert_eq!(f.drops, 0);
    }
    #[test]
    fn missing_drop_evidence_is_unknown() {
        let mut f = io();
        f.after = None;
        assert_eq!(
            run_storage(&mut f, &context("4/40")),
            StorageOutcome::Unknown
        );
        assert_eq!(f.drops, 1);
    }
    #[test]
    fn cancelled_before_drop_sends_nothing() {
        let mut f = io();
        f.cancel_after_store = true;
        assert_eq!(
            run_storage(&mut f, &context("4/40")),
            StorageOutcome::Cancelled
        );
        assert_eq!(f.drops, 0);
    }
    #[test]
    fn later_conflict_cannot_clear_zero() {
        let d = DropInfo {
            name: None,
            pity: Some("8/40".into()),
            text: "New Item Devil Fruit".into(),
            is_legendary: false,
            custom_telegram_message: None,
        };
        let c = FruitCatchContext::new(&d, "Legendary Pity 0/40 New Item Devil Fruit", true, true);
        assert!(c.pity_zero);
        assert!(!c.policy.action.drop_permitted());
    }
    #[test]
    fn legendary_and_mythical_never_drop_even_with_nonzero_pity() {
        for name in ["Pika", "Tori"] {
            let d = DropInfo {
                name: Some(name.into()),
                pity: Some("8/40".into()),
                text: String::new(),
                is_legendary: true,
                custom_telegram_message: None,
            };
            let mut f = io();
            assert_eq!(
                run_storage(&mut f, &FruitCatchContext::new(&d, "8/40", true, true)),
                StorageOutcome::ProtectedKept
            );
            assert_eq!(f.drops, 0);
        }
    }
    #[test]
    fn storage_error_never_authorizes_drop() {
        let mut f = io();
        f.banner = Some(StorageBannerResult::Failed {
            reason: "OCR unavailable".into(),
        });
        assert_eq!(
            run_storage(&mut f, &context("8/40")),
            StorageOutcome::Failed("OCR unavailable".into())
        );
        assert_eq!(f.drops, 0);
    }
}
