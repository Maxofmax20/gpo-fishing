//! Policy layer: perception -> POLICY -> action (§10, §27 of v5.3.0).
//!
//! Business decisions live HERE, never inside the vision model and never
//! scattered across call sites. Inputs are the project's EXISTING settings
//! (plain bools — this module takes no dependency on config shapes):
//!
//! - `never_drop_legendary` = fruit_storage.never_drop_legendary_or_mythical
//! - `keep_pity_zero`       = fruit_storage.keep_pity_zero_fruit
//!
//! `features.fruit_storage == false` is gated at the call site (store_fruit
//! early-returns, exactly as before); protection semantics are identical
//! with the feature on or off so pause-on-protected keeps working.
//!
//! Hard safety invariants (tested):
//! - UNKNOWN entity -> NEVER auto-drop (NeedsReview).
//! - Drop is allowed only for a KNOWN, non-protected entity.
//! - Missing confirmation is not success (see workflow.rs + actions.rs).

use serde::{Deserialize, Serialize};

use super::entity::{EntityType, ResultEntity};
use super::fruit::FruitRarity;

/// What the policy authorizes for one recognized result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyAction {
    /// Attempt store; Backspace drop path is disabled for this fruit.
    ProtectNoDrop,
    /// Unprotected duplicate path: store attempt, Backspace allowed ONLY on
    /// a duplicate/storage-full banner, outcome must still be confirmed.
    AllowDuplicateDrop,
    /// Feature off: nothing to decide.
    NoAction,
    /// Do not act automatically; require human review.
    NeedsReview,
}

impl PolicyAction {
    pub fn as_str(self) -> &'static str {
        match self {
            PolicyAction::ProtectNoDrop => "protect_no_drop",
            PolicyAction::AllowDuplicateDrop => "allow_duplicate_drop",
            PolicyAction::NoAction => "no_action",
            PolicyAction::NeedsReview => "needs_review",
        }
    }

    /// The Backspace/drop path may run only under this action.
    pub fn drop_permitted(self) -> bool {
        matches!(self, PolicyAction::AllowDuplicateDrop)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyDecision {
    pub action: PolicyAction,
    pub reason: String,
    pub protects_valuable: bool,
}

/// Mirror of the production post_catch rule, as a pure inspectable
/// function. `is_protected = decision.protects_valuable`.
pub fn decide_fruit_policy(
    rarity: FruitRarity,
    pity_zero: bool,
    never_drop_legendary: bool,
    keep_pity_zero: bool,
) -> PolicyDecision {
    let high = rarity.is_high_tier();
    if pity_zero && keep_pity_zero {
        return PolicyDecision {
            action: PolicyAction::ProtectNoDrop,
            reason: "pity-zero fruit kept (keep_pity_zero_fruit)".to_string(),
            protects_valuable: true,
        };
    }
    if high && never_drop_legendary {
        return PolicyDecision {
            action: PolicyAction::ProtectNoDrop,
            reason: format!("{} fruit protected (never_drop_legendary_or_mythical)", rarity.as_str()),
            protects_valuable: true,
        };
    }
    PolicyDecision {
        action: PolicyAction::AllowDuplicateDrop,
        reason: format!("unprotected {} fruit: duplicate path allowed, outcome must be confirmed", rarity.as_str()),
        protects_valuable: false,
    }
}

/// Policy for a recognized RESULT entity. UNKNOWN (or missing identity)
/// can never authorize a drop.
pub fn decide_entity_policy(entity: &ResultEntity) -> PolicyDecision {
    if entity.entity_type == EntityType::Unknown || entity.entity_id.is_none() {
        return PolicyDecision {
            action: PolicyAction::NeedsReview,
            reason: "UNKNOWN entity: no automatic drop; require confirmation / safe fallback".to_string(),
            protects_valuable: false,
        };
    }
    match entity.entity_type {
        EntityType::Fish => PolicyDecision {
            action: PolicyAction::NoAction,
            reason: "fish are recorded, never dropped by policy".to_string(),
            protects_valuable: false,
        },
        EntityType::Bait => PolicyDecision {
            action: PolicyAction::NoAction,
            reason: "bait menu labels are not inventory actions".to_string(),
            protects_valuable: false,
        },
        // Concrete fruit identity: protection is decided by decide_fruit_policy
        // (rarity + pity + settings), never here. This arm only routes.
        EntityType::DevilFruit | EntityType::Other => PolicyDecision {
            action: PolicyAction::NeedsReview,
            reason: "concrete identity without rarity/pity context: route to fruit policy, default review".to_string(),
            protects_valuable: false,
        },
        EntityType::Unknown => PolicyDecision {
            action: PolicyAction::NeedsReview,
            reason: "UNKNOWN entity: no automatic drop".to_string(),
            protects_valuable: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::entity::{EntitySource, EntityType, ResultEntity};
    use super::super::perception::Evidence;

    fn ent(t: EntityType, id: Option<&str>) -> ResultEntity {
        ResultEntity {
            event_id: None,
            state: "RESULT".to_string(),
            entity_type: t,
            entity_id: id.map(|s| s.to_string()),
            entity_name: None,
            confidence: 0.9,
            source: EntitySource::Ocr,
            evidence: vec![Evidence { kind: "t".into(), detail: "d".into(), weight: 1.0 }],
            confirmed: false,
            unknown_reason: None,
        }
    }

    #[test]
    fn unknown_entity_never_authorizes_drop() {
        let d = decide_entity_policy(&ent(EntityType::Unknown, None));
        assert_eq!(d.action, PolicyAction::NeedsReview);
        assert!(!d.action.drop_permitted());
    }

    #[test]
    fn typeless_identity_still_needs_review() {
        // Defensive: a claim with no type must not slip through.
        let mut e = ent(EntityType::Fish, None);
        e.entity_type = EntityType::Unknown;
        assert!(!decide_entity_policy(&e).action.drop_permitted());
    }

    #[test]
    fn fish_are_recorded_never_dropped() {
        let d = decide_entity_policy(&ent(EntityType::Fish, Some("fish:golden")));
        assert_eq!(d.action, PolicyAction::NoAction);
        assert!(!d.action.drop_permitted());
    }

    #[test]
    fn legendary_protected_by_setting() {
        let d = decide_fruit_policy(FruitRarity::Legendary, false, true, true);
        assert_eq!(d.action, PolicyAction::ProtectNoDrop);
        assert!(d.protects_valuable);
        assert!(!d.action.drop_permitted());
    }

    #[test]
    fn pity_zero_protected_by_setting() {
        let d = decide_fruit_policy(FruitRarity::Common, true, true, true);
        assert_eq!(d.action, PolicyAction::ProtectNoDrop);
        assert!(d.protects_valuable);
    }

    #[test]
    fn common_unprotected_allows_duplicate_path_only() {
        let d = decide_fruit_policy(FruitRarity::Common, false, true, true);
        assert_eq!(d.action, PolicyAction::AllowDuplicateDrop);
        assert!(!d.protects_valuable);
    }

    #[test]
    fn settings_can_narrow_but_never_widen() {
        // With protection flags OFF, legendary is droppable by policy —
        // the flags are the only thing standing between rarity and drop.
        let d = decide_fruit_policy(FruitRarity::Legendary, false, false, false);
        assert_eq!(d.action, PolicyAction::AllowDuplicateDrop);
    }

    #[test]
    fn mirrors_legacy_inline_rule() {
        // post_catch historically computed:
        // (never_drop && legendary_or_mythical) || (keep_pity && pity_zero).
        for rarity in [FruitRarity::Common, FruitRarity::Rare, FruitRarity::Epic,
                       FruitRarity::Legendary, FruitRarity::Mythical, FruitRarity::Unknown] {
            for pity in [false, true] {
                let legacy = (true && rarity.is_high_tier()) || (true && pity);
                let d = decide_fruit_policy(rarity, pity, true, true);
                assert_eq!(d.protects_valuable, legacy, "{rarity:?} pity={pity}");
            }
        }
    }
}
