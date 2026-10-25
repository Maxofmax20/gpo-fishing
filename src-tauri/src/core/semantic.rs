//! # Semantic-teacher evidence fusion
//!
//! RAM++ (Recognize Anything) is being introduced as a **semantic teacher** —
//! an independent evidence source that helps a human reviewer decide faster. It
//! is NOT ground truth, and this module is structured so that it structurally
//! cannot become ground truth.
//!
//! ## What this module does
//!
//! It takes the four evidence sources that exist for one dataset row and
//! produces an [`EvidenceFusion`] — an agreement classification plus a
//! recommended human action. Nothing here writes a label, sets
//! `training_eligible`, touches the knowledge base, or promotes a model.
//!
//! ## What this module deliberately cannot do
//!
//! * It has no code path that creates or mutates a KB entity. A name RAM++ has
//!   never seen stays a name; resolving it is a human decision.
//! * It never converts a RAM++ prediction into a label. The only thing it
//!   produces is a *recommendation* that a human then accepts, corrects,
//!   marks unknown, or skips.
//! * It cannot promote anything. Promotion authority stays in the readiness
//!   engine and the registry gates.
//!
//! ## The distinction that matters
//!
//! ```text
//! "RAM++ recognises something"      !=  "the GPO ontology says it exists"
//! ```
//!
//! A row whose OCR reads `Candy Corn Squid` while the KB has only `fish:squid`
//! is [`Agreement::Unresolved`], not a licence to invent an entity. It is a
//! request for human review, and the report it feeds calls it an
//! *unresolved visual concept*.
//!
//! ## Provenance
//!
//! [`LabelSource`] distinguishes `collector` (the bot's own OCR+KB guess),
//! `human` (a person decided), and `ram_assisted_human` (a person decided with
//! RAM++ evidence on screen). Only the last two may become training-eligible —
//! and that is enforced elsewhere too; this type exists so the distinction is
//! recorded rather than inferred.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Which model produced a semantic tag result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RamModel {
    /// Recognize Anything (RAM), swin_base.
    Ram,
    /// Recognize Anything Plus (RAM++), swin_large_384.
    RamPlus,
}

impl RamModel {
    pub fn as_str(self) -> &'static str {
        match self {
            RamModel::Ram => "ram",
            RamModel::RamPlus => "ram_plus",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "ram" | "RAM" => Some(RamModel::Ram),
            "ram_plus" | "ram++" | "RAM++" | "ram-plus" => Some(RamModel::RamPlus),
            _ => None,
        }
    }
}

/// Provenance of a training label.
///
/// `RamAssistedHuman` means RAM++ evidence was VISIBLE to a human who then made
/// the decision. It does not mean RAM++ supplied the label, and it is not a
/// weaker form of `Human`: a human still owns the verdict either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelSource {
    /// The bot's own OCR + knowledge-base guess. Never training-eligible on
    /// its own.
    Collector,
    /// A human reviewed the row and decided, without semantic-teacher evidence.
    Human,
    /// A human reviewed the row with RAM++ evidence shown and decided.
    RamAssistedHuman,
}

impl LabelSource {
    /// May a row with this provenance become training-eligible?
    ///
    /// Note this is a *provenance* precondition, not the whole gate: the review
    /// eligibility checks (image decodes, canonical id exists, provenance
    /// present) still apply separately. There is deliberately no variant that
    /// makes a teacher-derived label eligible.
    pub fn is_human_authored(self) -> bool {
        matches!(self, LabelSource::Human | LabelSource::RamAssistedHuman)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            LabelSource::Collector => "collector",
            LabelSource::Human => "human",
            LabelSource::RamAssistedHuman => "ram_assisted_human",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "collector" => Some(LabelSource::Collector),
            "human" => Some(LabelSource::Human),
            "ram_assisted_human" => Some(LabelSource::RamAssistedHuman),
            _ => None,
        }
    }
}

/// One row of RAM++ open-set output for a single image.
///
/// This is the JSONL contract the worker writes (Phase 5). Everything needed to
/// reproduce or challenge the result travels with it: which model, which
/// checkpoint, which vocabulary, what preprocessing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RamResult {
    pub image_id: String,
    pub ram_model: String,
    /// SHA-256 of the checkpoint that produced this. Required: an unpinned
    /// teacher result is not evidence, it is an anecdote.
    pub checkpoint_sha256: String,
    /// Semantic tags (open-set/common tagging head output).
    #[serde(default)]
    pub tags: Vec<String>,
    /// Candidate custom categories with their scores, best first.
    #[serde(default)]
    pub candidate_categories: Vec<(String, f32)>,
    /// The semantic tagger's own vocabulary, for reproducibility.
    #[serde(default)]
    pub ram_vocabulary: String,
    /// Rejection threshold applied, if any. `None` means the teacher emitted no
    /// calibrated rejection and every result needs human eyes.
    #[serde(default)]
    pub ram_threshold: Option<f32>,
    pub preprocessing: String,
    pub timestamp_ms: u64,
    /// `ok` | `skipped` | `error:<reason>`.
    pub status: String,
    /// Inference latency, for the cost comparison in Phase 23.
    #[serde(default)]
    pub latency_ms: Option<u64>,
}

impl RamResult {
    /// Was this a real, successful inference?
    ///
    /// An errored or skipped row must never be read as "the teacher found
    /// nothing interesting" — that is an absence of evidence, not evidence of
    /// absence, and conflating the two is how a broken teacher silently
    /// degrades into a labelling authority.
    pub fn is_ok(&self) -> bool {
        self.status == "ok"
    }

    pub fn top_category(&self) -> Option<(&str, f32)> {
        self.candidate_categories
            .first()
            .map(|(c, s)| (c.as_str(), *s))
    }

    pub fn top_score(&self) -> Option<f32> {
        self.candidate_categories.first().map(|(_, s)| *s)
    }

    /// Basic structural validation.
    ///
    /// Deliberately strict about the checkpoint fingerprint: a teacher result
    /// with no checkpoint identity cannot be reproduced or audited, so it is
    /// rejected rather than accepted-with-a-warning.
    pub fn validate(&self) -> Result<(), String> {
        if self.image_id.trim().is_empty() {
            return Err("image_id is empty".into());
        }
        if RamModel::parse(&self.ram_model).is_none() {
            return Err(format!("unknown ram_model {:?}", self.ram_model));
        }
        let sha = self.checkpoint_sha256.trim();
        if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!(
                "checkpoint_sha256 must be 64 hex chars, got {:?}",
                self.checkpoint_sha256
            ));
        }
        if self.preprocessing.trim().is_empty() {
            return Err("preprocessing is empty; a result must state how the image was prepared".into());
        }
        Ok(())
    }
}

/// How the independent evidence sources relate for one row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Agreement {
    /// Every available source names the same canonical entity.
    AllAgree,
    /// OCR and RAM agree with each other but not with the stored label.
    OcrRamAgree,
    /// The stored model label and RAM agree but OCR disagrees.
    ModelRamAgree,
    /// OCR and the stored label agree but RAM disagrees.
    OcrModelAgree,
    /// Only RAM produced a usable candidate.
    RamOnly,
    /// Only the stored model label is present.
    ModelOnly,
    /// Only OCR produced a usable name.
    OcrOnly,
    /// Sources actively disagree and no majority exists.
    NoAgreement,
    /// The name is not a KB entity, so there is nothing to agree with.
    Unresolved,
}

impl Agreement {
    pub fn as_str(self) -> &'static str {
        match self {
            Agreement::AllAgree => "ALL_AGREE",
            Agreement::OcrRamAgree => "OCR_RAM_AGREE",
            Agreement::ModelRamAgree => "MODEL_RAM_AGREE",
            Agreement::OcrModelAgree => "OCR_MODEL_AGREE",
            Agreement::RamOnly => "RAM_ONLY",
            Agreement::ModelOnly => "MODEL_ONLY",
            Agreement::OcrOnly => "OCR_ONLY",
            Agreement::NoAgreement => "NO_AGREEMENT",
            Agreement::Unresolved => "UNRESOLVED",
        }
    }
}

/// What a reviewer should do next. A RECOMMENDATION, never an action taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewAdvice {
    /// All sources agree and the name is canonical: the reviewer may confirm
    /// cheaply. Still a human decision.
    LikelyConfirm,
    /// Sources disagree but every candidate is a canonical entity: the reviewer
    /// picks or corrects.
    NeedsCorrection,
    /// The name is not in the KB. Human decides between "UNKNOWN" and
    /// recording a CONFLICT. No entity may be auto-created.
    UnresolvedConcept,
    /// Too little evidence to rank; ordinary review order.
    Neutral,
}

/// Review priority band. Lower number = review sooner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewPriority {
    /// OCR disagrees with the stored label.
    Highest = 0,
    /// RAM disagrees with the stored label.
    High = 1,
    /// The name has no KB entity at all.
    Unresolved = 2,
    /// OCR and RAM disagree with each other.
    Medium = 3,
    /// Everything agrees - confirm cheaply, review last.
    Low = 4,
}

/// The fused evidence view for one dataset row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvidenceFusion {
    pub image_id: String,
    /// The label currently stored on the row (machine-derived or human).
    pub current_entity: Option<String>,
    /// What OCR read, verbatim. NOT resolved to a canonical id here: resolving
    /// it is exactly the step that hides "Crimson Snapper" behind
    /// `fish:snapper`.
    pub ocr_text: Option<String>,
    /// The canonical entity the current label maps to, if any.
    pub kb_match: Option<String>,
    /// The strongest RAM++ candidate, if the teacher ran successfully.
    pub ram_top: Option<String>,
    pub ram_score: Option<f32>,
    /// True when a human has already reviewed this row.
    pub human_reviewed: bool,
    pub agreement: Agreement,
    pub advice: ReviewAdvice,
    pub priority: ReviewPriority,
    /// Names proposed by evidence that are NOT KB entities. These are
    /// *unresolved visual concepts*, never candidates for automatic creation.
    pub unresolved_concepts: Vec<String>,
}

impl EvidenceFusion {
    /// Review-order band for this row.
    ///
    /// Ordering is: disagreement first, agreement last. That is the whole point
    /// of adding a teacher — it should surface the rows where the cheap label
    /// is most likely wrong, and get out of the way of the rest.
    pub fn priority(&self) -> ReviewPriority {
        if self.ocr_text.is_some() && self.current_entity.is_some() && !self.ocr_agrees() {
            return ReviewPriority::Highest;
        }
        if self.ram_top.is_some() && self.current_entity.is_some() && !self.ram_agrees() {
            return ReviewPriority::High;
        }
        if !self.unresolved_concepts.is_empty() {
            return ReviewPriority::Unresolved;
        }
        if self.ocr_text.is_some() && self.ram_top.is_some() && !self.ocr_ram_agree() {
            return ReviewPriority::Medium;
        }
        ReviewPriority::Low
    }

    /// Does OCR's raw text name the same thing as the stored label?
    ///
    /// Compared on NORMALISED text, not on a resolved entity id. Resolving
    /// first would make `Crimson Snapper` look like agreement with
    /// `fish:snapper` when the KB has never claimed they are the same thing.
    fn ocr_agrees(&self) -> bool {
        let (Some(o), Some(c)) = (&self.ocr_text, &self.current_entity) else { return false };
        let short = norm(short_name(c));
        // Guard against an empty short name matching everything.
        !short.is_empty() && norm(o).contains(&short)
    }

    fn ram_agrees(&self) -> bool {
        let (Some(r), Some(c)) = (&self.ram_top, &self.current_entity) else { return false };
        same_entity(r, c)
    }

    fn ocr_ram_agree(&self) -> bool {
        let (Some(o), Some(r)) = (&self.ocr_text, &self.ram_top) else { return false };
        let short = norm(short_name(r));
        !short.is_empty() && norm(o).contains(&short)
    }
}

/// Aggressively normalise an OCR/name string for comparison: lowercase, strip
/// punctuation the OCR engine invents (`<`, `>`, `*`, `.`), collapse spaces.
fn norm(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .filter(|c| !matches!(c, '<' | '>' | '*' | '.' | ',' | '"' | '\'' | '|' | '\\'))
        .collect();
    let mut out = String::with_capacity(cleaned.len());
    let mut prev_space = true;
    for c in cleaned.chars().map(|c| c.to_ascii_lowercase()) {
        let c = if c.is_ascii_alphanumeric() { c } else { ' ' };
        if c == ' ' {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

/// Canonical form of a KB entity id: `fish:shark` -> `shark`.
fn short_name(entity: &str) -> &str {
    entity.split_once(':').map(|(_, s)| s).unwrap_or(entity)
}

/// Do two entity references name the same class? Both sides are compared on
/// their SHORT name, because OCR produces `Shark` while the ontology stores
/// `fish:shark` and comparing the raw strings would never match.
fn same_entity(a: &str, b: &str) -> bool {
    let (x, y) = (norm(short_name(a)), norm(short_name(b)));
    !x.is_empty() && x == y
}

/// Fuse the evidence for one row.
///
/// `kb_entities` is the verified canonical entity set. `ram` is the teacher's
/// result, if it ran successfully.
///
/// This function is pure and has no write capability. That is deliberate: the
/// decision it makes is "how should a human look at this?", and nothing else.
pub fn fuse(
    image_id: &str,
    ocr_text: Option<&str>,
    current_entity: Option<&str>,
    ram: Option<&RamResult>,
    human_reviewed: bool,
    kb_entities: &BTreeSet<String>,
) -> EvidenceFusion {
    // Canonical check on the CURRENT label. An unresolved name is reported as
    // such rather than being quietly mapped onto the nearest KB entry.
    let kb_match = current_entity
        .filter(|e| kb_entities.contains(*e))
        .map(|e| e.to_string());

    let mut unresolved: BTreeSet<String> = BTreeSet::new();

    // OCR: does its raw text correspond to a canonical entity?
    let ocr_canonical = ocr_text.and_then(|t| resolve_ocr_against_kb(t, kb_entities));

    // RAM: its top candidate is a suggestion in our own vocabulary, so it is
    // canonical BY CONSTRUCTION if the vocabulary was built from the KB. If a
    // name is not in the KB, it is an unresolved concept.
    let (ram_top, ram_score) = match ram.filter(|r| r.is_ok()).and_then(|r| r.top_category()) {
        Some((c, s)) => (Some(c.to_string()), Some(s)),
        None => (None, None),
    };
    if let Some(r) = &ram_top {
        if !kb_entities.contains(r) {
            unresolved.insert(r.clone());
        }
    }

    // OCR ambiguity is TWO distinct failures, and both must reach a human.
    //
    //   0 KB names in the text  -> the text names something the ontology has
    //                             never heard of (`Exotic Tiserfin`).
    //   >=2 KB names in the text -> the text is AMBIGUOUS between canonical
    //                             entities (`Crimson Snapper` names both
    //                             fish:crimson and fish:snapper). Resolving it
    //                             either way is exactly what produced 166
    //                             mislabelled rows, so neither way is taken.
    //
    // Treating only the first case as unresolved is why `Crimson Snapper`
    // slipped through: its text is full of KB names, which is the problem.
    if let Some(t) = ocr_text {
        let n = norm(t);
        if !n.is_empty() {
            let hits: Vec<String> = kb_entities
                .iter()
                .filter(|e| {
                    let s = norm(short_name(e));
                    !s.is_empty() && n.contains(&s)
                })
                .cloned()
                .collect();
            match hits.len() {
                0 => {
                    unresolved.insert(n);
                }
                1 => {} // exactly one canonical name: resolved, not a concept
                _ => {
                    let mut names: Vec<&str> =
                        hits.iter().map(|e| short_name(e)).collect();
                    names.sort_unstable();
                    unresolved.insert(format!("ambiguous: {}", names.join(" | ")));
                }
            }
        }
    }

    let agreement = classify(
        kb_match.as_deref(),
        ocr_canonical.as_deref(),
        ram_top.as_deref(),
        !unresolved.is_empty(),
    );

    let advice = match (&agreement, human_reviewed) {
        (_, true) => ReviewAdvice::Neutral,
        (Agreement::AllAgree, false) => ReviewAdvice::LikelyConfirm,
        (Agreement::Unresolved, false) => ReviewAdvice::UnresolvedConcept,
        (Agreement::NoAgreement, false)
        | (Agreement::OcrRamAgree, false)
        | (Agreement::ModelRamAgree, false)
        | (Agreement::OcrModelAgree, false) => ReviewAdvice::NeedsCorrection,
        _ => ReviewAdvice::Neutral,
    };

    let mut fusion = EvidenceFusion {
        image_id: image_id.to_string(),
        current_entity: current_entity.map(|s| s.to_string()),
        ocr_text: ocr_text.map(|s| s.to_string()),
        kb_match,
        ram_top,
        ram_score,
        human_reviewed,
        agreement,
        advice,
        priority: ReviewPriority::Low,
        unresolved_concepts: unresolved.into_iter().collect(),
    };
    fusion.priority = fusion.priority();
    fusion
}

fn classify(
    kb: Option<&str>,
    ocr: Option<&str>,
    ram: Option<&str>,
    unresolved: bool,
) -> Agreement {
    if unresolved {
        // A name with no canonical entity cannot "agree" with anything. This
        // is the case that must never be resolved by inventing an entity.
        return Agreement::Unresolved;
    }
    let same = |a: Option<&str>, b: Option<&str>| match (a, b) {
        (Some(x), Some(y)) => same_entity(x, y),
        _ => false,
    };
    match (kb, ocr, ram) {
        (Some(k), Some(o), Some(r)) if same(Some(k), Some(o)) && same(Some(k), Some(r)) => {
            Agreement::AllAgree
        }
        (_, Some(o), Some(r)) if same(Some(o), Some(r)) => Agreement::OcrRamAgree,
        (Some(k), _, Some(r)) if same(Some(k), Some(r)) => Agreement::ModelRamAgree,
        (Some(k), Some(o), _) if same(Some(k), Some(o)) => Agreement::OcrModelAgree,
        (_, _, Some(_)) => Agreement::RamOnly,
        (Some(_), _, _) => Agreement::ModelOnly,
        (_, Some(_), _) => Agreement::OcrOnly,
        // Any remaining combination is an active disagreement. Never resolved by
        // picking a side.
        _ => Agreement::NoAgreement,
    }
}

/// Resolve OCR text to a canonical entity ONLY when the text unambiguously
/// contains a canonical short name. Never a fuzzy or nearest match.
///
/// This is the whole reason the audit found 354 contaminated rows: the real
/// resolver maps `Crimson Snapper` onto `fish:snapper` and onto `fish:crimson`
/// depending on the path taken, so the stored label stopped carrying the fact
/// that the screen said something the KB has no entity for.
pub fn resolve_ocr_against_kb(text: &str, kb_entities: &BTreeSet<String>) -> Option<String> {
    let n = norm(text);
    if n.is_empty() {
        return None;
    }
    let hits: Vec<&String> = kb_entities
        .iter()
        .filter(|e| {
            let s = norm(short_name(e));
            !s.is_empty() && n.contains(&s)
        })
        .collect();
    // Exactly one hit resolves. Two hits (e.g. "crimson snapper" matching both
    // `fish:crimson` and `fish:snapper`) is EXACTLY the case that must reach a
    // human: picking either one is the decision that produced the contamination.
    if hits.len() == 1 {
        Some(hits[0].clone())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kb() -> BTreeSet<String> {
        [
            "fish:angelfish",
            "fish:crimson",
            "fish:golden",
            "fish:pufferfish",
            "fish:shark",
            "fish:snapper",
            "fish:squid",
            "fish:swordfish",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    fn ram_ok(cat: &str, score: f32) -> RamResult {
        RamResult {
            image_id: "img".into(),
            ram_model: "ram_plus".into(),
            checkpoint_sha256: "a".repeat(64),
            tags: vec!["fish".into()],
            candidate_categories: vec![(cat.into(), score)],
            ram_vocabulary: "gpo-kb-v1".into(),
            ram_threshold: None,
            preprocessing: "rgb->letterbox-square(black)->bicubic->224".into(),
            timestamp_ms: 1,
            status: "ok".into(),
            latency_ms: Some(12),
        }
    }

    #[test]
    fn every_source_agreeing_is_the_cheap_confirm_case() {
        let f = fuse(
            "i1",
            Some("Nev Item <Shark"),
            Some("fish:shark"),
            Some(&ram_ok("fish:shark", 0.8)),
            false,
            &kb(),
        );
        assert_eq!(f.agreement, Agreement::AllAgree);
        assert_eq!(f.advice, ReviewAdvice::LikelyConfirm);
        assert_eq!(f.priority, ReviewPriority::Low, "agreement reviews LAST");
    }

    #[test]
    fn crimson_snapper_is_unresolved_not_snapper() {
        // THE measured contamination case. `Crimson Snapper` contains BOTH
        // `crimson` and `snapper`, and the KB has never claimed they are the
        // same entity. Resolving it either way is what produced 166 mislabelled
        // rows, so it must reach a human.
        let f = fuse(
            "i2",
            Some("Nev Item <Crimson snapper"),
            Some("fish:snapper"),
            Some(&ram_ok("fish:snapper", 0.6)),
            false,
            &kb(),
        );
        assert_eq!(
            f.agreement,
            Agreement::Unresolved,
            "two KB names in one OCR string must not resolve silently"
        );
        assert_eq!(f.advice, ReviewAdvice::UnresolvedConcept);
        assert_eq!(f.priority, ReviewPriority::Unresolved);
        assert!(
            f.unresolved_concepts
                .iter()
                .any(|c| c.contains("crimson") && c.contains("snapper")),
            "the ambiguous concept must be surfaced, not dropped: {:?}",
            f.unresolved_concepts
        );
    }

    #[test]
    fn a_teacher_name_outside_the_kb_is_a_concept_never_an_entity() {
        // RAM++ proposing something the KB has never heard of must not become a
        // candidate entity. It is recorded as an unresolved visual concept.
        let f = fuse(
            "i3",
            Some("Nev Item <Tiserfin"),
            Some("fish:shark"),
            Some(&ram_ok("tigerfin", 0.71)),
            false,
            &kb(),
        );
        assert_eq!(f.agreement, Agreement::Unresolved);
        assert!(f.unresolved_concepts.iter().any(|c| c == "tigerfin"));
        // No KB match is invented for it.
        assert_eq!(f.kb_match, Some("fish:shark".to_string()));
    }

    #[test]
    fn ocr_disagreeing_with_the_stored_label_is_the_top_priority() {
        let f = fuse(
            "i4",
            Some("Nev Item <Golden"),
            Some("fish:shark"),
            Some(&ram_ok("fish:shark", 0.8)),
            false,
            &kb(),
        );
        assert_eq!(f.priority, ReviewPriority::Highest);
        assert_eq!(f.advice, ReviewAdvice::NeedsCorrection);
    }

    #[test]
    fn ram_disagreeing_with_the_stored_label_outranks_agreement() {
        let f = fuse(
            "i5",
            Some("Nev Item <Shark"),
            Some("fish:shark"),
            Some(&ram_ok("fish:squid", 0.55)),
            false,
            &kb(),
        );
        // OCR agrees with the label, RAM does not: the teacher earns attention
        // exactly here.
        assert_eq!(f.priority, ReviewPriority::High);
        assert_eq!(f.agreement, Agreement::OcrModelAgree);
    }

    #[test]
    fn a_failed_teacher_is_absence_of_evidence_not_evidence_of_absence() {
        let mut bad = ram_ok("fish:shark", 0.9);
        bad.status = "error:checkpoint-missing".into();
        let f = fuse(
            "i6",
            Some("Nev Item <Shark"),
            Some("fish:shark"),
            Some(&bad),
            false,
            &kb(),
        );
        // A broken teacher must not silently become evidence that the label is
        // fine, nor evidence that it is wrong.
        assert!(f.ram_top.is_none(), "a non-ok result carries no candidate");
        assert_ne!(f.agreement, Agreement::AllAgree);
    }

    #[test]
    fn ram_results_are_structurally_validated() {
        let mut r = ram_ok("fish:shark", 0.5);
        assert!(r.validate().is_ok());

        r.checkpoint_sha256 = "short".into();
        assert!(r.validate().is_err(), "an unpinned checkpoint is not evidence");

        r = ram_ok("fish:shark", 0.5);
        r.preprocessing = String::new();
        assert!(r.validate().is_err(), "preprocessing must be stated");

        r = ram_ok("fish:shark", 0.5);
        r.ram_model = "gpt-whatever".into();
        assert!(r.validate().is_err());
    }

    #[test]
    fn only_human_authored_labels_can_be_training_eligible() {
        assert!(!LabelSource::Collector.is_human_authored());
        assert!(LabelSource::Human.is_human_authored());
        assert!(
            LabelSource::RamAssistedHuman.is_human_authored(),
            "a human decided, with the teacher on screen"
        );
        // There is no variant that would make a teacher-derived label eligible.
        assert_eq!(LabelSource::parse("ram_plus"), None);
    }

    #[test]
    fn ocr_resolution_refuses_to_guess_between_two_canonical_names() {
        assert_eq!(
            resolve_ocr_against_kb("Nev Item <Shark", &kb()).as_deref(),
            Some("fish:shark")
        );
        assert_eq!(
            resolve_ocr_against_kb("Nev Item <Crimson snapper", &kb()),
            None,
            "two candidates is a human decision, not a coin flip"
        );
        assert_eq!(
            resolve_ocr_against_kb("Nev Item <Exotic Tiserfin", &kb()),
            None
        );
    }

    #[test]
    fn human_reviewed_rows_are_never_re_recommended() {
        let f = fuse(
            "i7",
            Some("Nev Item <Shark"),
            Some("fish:shark"),
            Some(&ram_ok("fish:shark", 0.9)),
            true,
            &kb(),
        );
        assert!(f.human_reviewed);
        assert_eq!(
            f.advice,
            ReviewAdvice::Neutral,
            "a reviewed row must not keep nagging the reviewer"
        );
    }

    #[test]
    fn results_serialise_to_the_documented_jsonl_shape() {
        let r = ram_ok("fish:squid", 0.42);
        let j = serde_json::to_string(&r).unwrap();
        for key in [
            "image_id",
            "ram_model",
            "checkpoint_sha256",
            "tags",
            "candidate_categories",
            "preprocessing",
            "timestamp_ms",
            "status",
        ] {
            assert!(j.contains(key), "missing {key} in {j}");
        }
        let back: RamResult = serde_json::from_str(&j).unwrap();
        assert_eq!(back, r, "round-trip must be lossless for reproducibility");
    }
}