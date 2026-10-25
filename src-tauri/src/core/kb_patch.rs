//! Reviewable KB patches, applied through the canonical mutation path.
//!
//! A patch is a committed, deterministic artifact (see
//! `ml/gpo_ontology_review/fish_kb_patch.py`, which only ANALYSES). This module
//! is the APPLY half, in the language that owns the KB:
//!
//! ```text
//! fish_kb_patch.json
//!       ↓ parse_patch            (shape, kind, version)
//!       ↓ validate_additions     (validate_import + identity/provenance rules,
//!       │                         re-checked against the LIVE effective KB -
//!       │                         the file's own validation_results are not
//!       │                         trusted)
//!       ↓ apply_patch            (backup -> merge -> atomic write -> verify)
//! effective knowledge = bundled + knowledge_custom.json overlay
//! ```
//!
//! Invariants:
//!
//! * the overlay is the ONLY thing written; the compiled bundled base and every
//!   human-decision/review file are untouched;
//! * applying is atomic (`Store::save_knowledge_overlay` writes tmp+rename),
//!   idempotent (identical existing entities are skipped; same-id/different-
//!   content is an error, never a silent overwrite), and reversible (the
//!   pre-apply overlay is copied aside first);
//! * unresolved concepts in a patch are reported and skipped, never created.

use crate::config::Store as ConfigStore;
use crate::core::knowledge::{ImportEntity, KnowledgeBase};
use crate::core::ml_model::sha256_hex;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct PatchDocument {
    pub kind: String,
    pub version: u32,
    pub entities_to_add: Vec<AddEntry>,
    #[serde(default)]
    pub entities_reused: Vec<ReusedEntry>,
    #[serde(default)]
    pub unresolved_concepts: Vec<UnresolvedEntry>,
}

#[derive(Debug, Deserialize)]
pub struct AddEntry {
    pub entity: ImportEntity,
    #[serde(default)]
    pub human_reviewed_rows: usize,
}

#[derive(Debug, Deserialize)]
pub struct ReusedEntry {
    pub entity_id: String,
}

#[derive(Debug, Deserialize)]
pub struct UnresolvedEntry {
    pub entity_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyOutcome {
    /// Entities written into the overlay this run (0 on a second apply).
    pub added: usize,
    /// Entities already present with identical content (idempotent skips).
    pub already_present: usize,
    pub backup: Option<PathBuf>,
    pub pre_digest: String,
    pub post_digest: String,
}

pub fn parse_patch(json: &str) -> Result<PatchDocument, String> {
    let doc: PatchDocument =
        serde_json::from_str(json).map_err(|e| format!("patch is not valid JSON: {e}"))?;
    if doc.kind != "fish-kb-patch" {
        return Err(format!(
            "patch kind {:?} is not \"fish-kb-patch\"",
            doc.kind
        ));
    }
    if doc.version != 1 {
        return Err(format!("patch version {} is not supported", doc.version));
    }
    if doc.entities_to_add.is_empty() {
        return Err("patch adds no entities - nothing to do is not a patch".to_string());
    }
    Ok(doc)
}

/// Re-validate every addition against the LIVE effective KB. Returns the
/// validated entities, or every error found (never a partial success).
pub fn validate_additions(
    doc: &PatchDocument,
    kb: &KnowledgeBase,
) -> Result<Vec<ImportEntity>, Vec<String>> {
    let mut errors: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut validated: Vec<ImportEntity> = Vec::new();
    for entry in &doc.entities_to_add {
        let ent = &entry.entity;
        if !seen.insert(ent.id.clone()) {
            errors.push(format!("duplicate entity id in patch: {}", ent.id));
        }
        if entry.human_reviewed_rows == 0 {
            errors.push(format!("{}: no human-reviewed rows as evidence", ent.id));
        }
        match KnowledgeBase::validate_import(ent) {
            Err(e) => errors.push(e),
            Ok(_) => {
                if kb.entities().iter().any(|e| e.id == ent.id) {
                    errors.push(format!(
                        "{}: already in the effective KB; belongs in entities_reused",
                        ent.id
                    ));
                }
                // A canonical NAME that resolves to a DIFFERENT id would split
                // one fish into two identities.
                if let Some(hit) = kb.find_by_name(&ent.canonical_name) {
                    if hit.id != ent.id {
                        errors.push(format!(
                            "{}: canonical name {:?} already resolves to {}",
                            ent.id, ent.canonical_name, hit.id
                        ));
                    }
                }
                validated.push(ent.clone());
            }
        }
    }
    if errors.is_empty() {
        Ok(validated)
    } else {
        Err(errors)
    }
}

/// Has this patch already been consumed by the effective KB?
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsumedState {
    /// At least one addition is not in the KB yet: the patch is applicable.
    Fresh,
    /// Every addition is present with identical content: a re-apply is a
    /// legitimate no-op, not an error.
    Consumed,
    /// An addition's id exists with DIFFERENT content: a curator decision,
    /// never a silent overwrite.
    Conflict(String),
}

pub fn consumed_state(doc: &PatchDocument, kb: &KnowledgeBase) -> ConsumedState {
    let mut any_missing = false;
    for entry in &doc.entities_to_add {
        let ent = &entry.entity;
        match kb.entities().iter().find(|e| e.id == ent.id) {
            None => any_missing = true,
            Some(existing) => {
                let Ok(valid) = KnowledgeBase::validate_import(ent) else {
                    return ConsumedState::Conflict(format!(
                        "{}: patch entity fails validation",
                        ent.id
                    ));
                };
                let same = existing.id == valid.id
                    && existing.canonical_name == valid.canonical_name
                    && existing.aliases == valid.aliases
                    && existing.category == valid.category
                    && existing.rarity == valid.rarity
                    && existing.description == valid.description
                    && existing.ocr_aliases == valid.ocr_aliases;
                if !same {
                    return ConsumedState::Conflict(format!(
                        "{}: the KB already carries this id with DIFFERENT content",
                        ent.id
                    ));
                }
            }
        }
    }
    if any_missing {
        ConsumedState::Fresh
    } else {
        ConsumedState::Consumed
    }
}

/// Deterministic digest over the KB's identity-bearing fields, so before/after
/// states are comparable without diffing a whole serialisation.
pub fn kb_digest(kb: &KnowledgeBase) -> String {
    let mut rows: Vec<String> = kb
        .entities()
        .iter()
        .map(|e| {
            format!(
                "{}|{}|{}|{}|{}|{}",
                e.id,
                e.canonical_name,
                e.aliases.join(","),
                e.ocr_aliases.join(","),
                e.category.as_str(),
                e.rarity.as_deref().unwrap_or("")
            )
        })
        .collect();
    rows.sort();
    sha256_hex(rows.join("\n").as_bytes())
}

/// Apply validated additions to the persisted overlay: backup, merge, atomic
/// write, post-write verification. Idempotent; reversible via the backup file.
pub fn apply_patch(
    store: &ConfigStore,
    config_dir: &std::path::Path,
    validated: &[ImportEntity],
) -> Result<ApplyOutcome, String> {
    let pre = store.effective_knowledge();
    let pre_digest = kb_digest(&pre);

    let overlay = store.load_knowledge_overlay();
    let mut to_add: Vec<ImportEntity> = Vec::new();
    let mut already_present = 0usize;
    for ent in validated {
        match overlay.iter().find(|o| o.id == ent.id) {
            None => to_add.push(ent.clone()),
            Some(existing) => {
                let same = serde_json::to_string(existing).ok() == serde_json::to_string(ent).ok();
                if !same {
                    return Err(format!(
                        "{}: overlay already carries this id with DIFFERENT content; \
                         that is a curator decision, not an apply",
                        ent.id
                    ));
                }
                already_present += 1;
            }
        }
    }

    if to_add.is_empty() {
        return Ok(ApplyOutcome {
            added: 0,
            already_present,
            backup: None,
            pre_digest: pre_digest.clone(),
            post_digest: pre_digest,
        });
    }

    // Backup BEFORE the write, next to the live overlay.
    let overlay_path = config_dir.join("knowledge_custom.json");
    let mut backup = None;
    if overlay_path.is_file() {
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let dest = config_dir.join(format!("knowledge_custom.backup-{epoch}.json"));
        std::fs::copy(&overlay_path, &dest).map_err(|e| format!("backup failed: {e}"))?;
        backup = Some(dest);
    }

    let mut merged = overlay;
    merged.extend(to_add.iter().cloned());
    store
        .save_knowledge_overlay(&merged)
        .map_err(|e| format!("overlay write failed: {e}"))?;

    // Verify from a FRESH read, not from what we think we wrote.
    let post = ConfigStore::new(config_dir.to_path_buf()).effective_knowledge();
    for ent in &to_add {
        if !post.entities().iter().any(|e| e.id == ent.id) {
            return Err(format!(
                "post-apply verification failed: {} is not in the effective KB",
                ent.id
            ));
        }
    }
    Ok(ApplyOutcome {
        added: to_add.len(),
        already_present,
        backup,
        pre_digest,
        post_digest: kb_digest(&post),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, name: &str) -> ImportEntity {
        ImportEntity {
            id: id.to_string(),
            canonical_name: name.to_string(),
            aliases: vec![],
            category: "fish".to_string(),
            rarity: Some("Rare".to_string()),
            description: None,
            ocr_aliases: vec![],
            visual_refs: vec![],
            provenance: crate::core::knowledge::EntityProvenance {
                source: "test".to_string(),
                url: Some("https://example.test/wiki".to_string()),
                updated: None,
            },
        }
    }

    fn patch_json(entries: &[(&str, &str)], rows: usize) -> String {
        let adds: Vec<String> = entries
            .iter()
            .map(|(id, name)| {
                let e = entry(id, name);
                format!(
                    r#"{{"entity": {}, "human_reviewed_rows": {rows}}}"#,
                    serde_json::to_string(&e).unwrap()
                )
            })
            .collect();
        format!(
            r#"{{"kind": "fish-kb-patch", "version": 1, "entities_to_add": [{}]}}"#,
            adds.join(",")
        )
    }

    fn temp_store(name: &str) -> (ConfigStore, PathBuf) {
        static C: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = C.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("gpo-kbpatch-{name}-{n}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (ConfigStore::new(dir.clone()), dir)
    }

    #[test]
    fn parse_rejects_wrong_kind_version_and_empty() {
        assert!(parse_patch(r#"{"kind":"other","version":1,"entities_to_add":[1]}"#).is_err());
        assert!(
            parse_patch(r#"{"kind":"fish-kb-patch","version":2,"entities_to_add":[1]}"#).is_err()
        );
        assert!(
            parse_patch(r#"{"kind":"fish-kb-patch","version":1,"entities_to_add":[]}"#).is_err()
        );
    }

    #[test]
    fn validate_catches_duplicates_and_kb_collisions() {
        let kb = KnowledgeBase::bundled();
        // duplicate id within the patch
        let doc = parse_patch(&patch_json(&[("fish:x", "X"), ("fish:x", "X")], 5)).unwrap();
        assert!(validate_additions(&doc, kb)
            .unwrap_err()
            .iter()
            .any(|e| e.contains("duplicate")));
        // id already in the bundled KB
        let doc = parse_patch(&patch_json(&[("fish:swordfish", "Swordfish")], 5)).unwrap();
        assert!(validate_additions(&doc, kb)
            .unwrap_err()
            .iter()
            .any(|e| e.contains("already in")));
        // a name that resolves to a different id
        let doc = parse_patch(&patch_json(&[("fish:notswordfish", "Swordfish")], 5)).unwrap();
        assert!(validate_additions(&doc, kb)
            .unwrap_err()
            .iter()
            .any(|e| e.contains("already resolves")));
        // no human evidence
        let doc = parse_patch(&patch_json(&[("fish:newfish", "Newfish")], 0)).unwrap();
        assert!(validate_additions(&doc, kb)
            .unwrap_err()
            .iter()
            .any(|e| e.contains("no human")));
    }

    #[test]
    fn consumed_state_distinguishes_fresh_consumed_and_conflict() {
        let (store, dir) = temp_store("consumed");
        let doc = parse_patch(&patch_json(&[("fish:testfish", "Testfish")], 25)).unwrap();
        let kb = store.effective_knowledge();
        assert_eq!(consumed_state(&doc, &kb), ConsumedState::Fresh);

        let validated = validate_additions(&doc, &kb).unwrap();
        apply_patch(&store, &dir, &validated).unwrap();
        let kb_after = ConfigStore::new(dir.clone()).effective_knowledge();
        assert_eq!(consumed_state(&doc, &kb_after), ConsumedState::Consumed);

        let other = parse_patch(&patch_json(&[("fish:testfish", "A Different Name")], 25)).unwrap();
        assert!(matches!(
            consumed_state(&other, &kb_after),
            ConsumedState::Conflict(_)
        ));
    }

    #[test]
    fn apply_is_atomic_idempotent_and_verifiable() {
        let (store, dir) = temp_store("apply");
        let kb = store.effective_knowledge();
        let doc = parse_patch(&patch_json(&[("fish:testfish", "Testfish")], 25)).unwrap();
        let validated = validate_additions(&doc, &kb).unwrap();

        let first = apply_patch(&store, &dir, &validated).unwrap();
        assert_eq!(first.added, 1);
        assert!(ConfigStore::new(dir.clone())
            .effective_knowledge()
            .entities()
            .iter()
            .any(|e| e.id == "fish:testfish"));
        assert_ne!(first.pre_digest, first.post_digest);

        // Second apply: zero mutations, digest stable, no error.
        let kb2 = ConfigStore::new(dir.clone()).effective_knowledge();
        let doc2 = parse_patch(&patch_json(&[("fish:testfish", "Testfish")], 25)).unwrap();
        // validate would now reject it as "already in the KB", which is correct:
        // the patch has been consumed. Idempotency lives in apply itself.
        let _ = validate_additions(&doc2, &kb2);
        let second = apply_patch(&ConfigStore::new(dir.clone()), &dir, &validated).unwrap();
        assert_eq!(second.added, 0);
        assert_eq!(second.already_present, 1);
        assert_eq!(second.pre_digest, second.post_digest);
    }
}
