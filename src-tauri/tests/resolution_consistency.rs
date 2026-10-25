//! Cross-language drift guard: the Rust resolver and the Python-facing KB
//! export must agree on what a documented fish name resolves to.
//!
//! `kb_export.json` is what the Python ontology matcher reads; the effective
//! Rust KB is what `correlate_text` reads. This test fails if the two ever
//! disagree about the id a canonical fish name resolves to - which is exactly
//! how the 8-class stem contamination existed for months: two implementations
//! of "the same" ontology, quietly different.

#![cfg(windows)]

use gpo_autofish_lib::core::knowledge::{ImportEntity, KnowledgeBase};
use gpo_autofish_lib::core::perception::{match_entities, ScreenKind};

fn find_export() -> Option<std::path::PathBuf> {
    find_repo_file(
        &std::path::Path::new("ml")
            .join("gpo_ocr_audit")
            .join("kb_export.json"),
    )
}

fn find_repo_file(rel: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    for _ in 0..8 {
        let candidate = dir.join(rel);
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

/// The effective KB as the app computes it: bundled + every exported entity
/// the bundled base lacks (the persisted overlay entities), through the same
/// validated merge path. Returns the KB and the ids that were added (the 15
/// confirmed species), so the test knows exactly which ids are species.
fn effective_kb(export: &serde_json::Value) -> (KnowledgeBase, std::collections::HashSet<String>) {
    let mut kb = KnowledgeBase::bundled().clone();
    let have: std::collections::HashSet<String> =
        kb.entities().iter().map(|e| e.id.clone()).collect();
    let imports: Vec<ImportEntity> = export["entities"]
        .as_array()
        .expect("export entities")
        .iter()
        .filter(|e| !have.contains(e["id"].as_str().unwrap_or("")))
        .map(|e| ImportEntity {
            id: e["id"].as_str().unwrap().to_string(),
            canonical_name: e["canonical_name"].as_str().unwrap().to_string(),
            aliases: e["aliases"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            category: e["category"].as_str().unwrap().to_string(),
            rarity: e["rarity"].as_str().map(str::to_string),
            description: None,
            ocr_aliases: e["ocr_aliases"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            visual_refs: vec![],
            provenance: gpo_autofish_lib::core::knowledge::EntityProvenance {
                source: "kb_export.json".to_string(),
                url: Some(
                    "https://grand-piece-online.fandom.com/wiki/Fishing/Bestiary".to_string(),
                ),
                updated: None,
            },
        })
        .collect();
    let added: std::collections::HashSet<String> = imports.iter().map(|i| i.id.clone()).collect();
    let report = kb.merge_import(&imports, false);
    assert_eq!(
        report.added, 15,
        "the export must carry the 15 overlay species: {report:?}"
    );
    (kb, added)
}

#[test]
fn every_documented_fish_name_resolves_to_its_export_id() {
    let Some(path) = find_export() else {
        eprintln!("SKIP: kb_export.json not found");
        return;
    };
    let export: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let (kb, mut species) = effective_kb(&export);
    // The 16 documented fish = the 15 overlay species + bundled fish:swordfish.
    species.insert("fish:swordfish".to_string());

    let mut checked = 0usize;
    for e in export["entities"].as_array().unwrap() {
        let id = e["id"].as_str().unwrap();
        let name = e["canonical_name"].as_str().unwrap();
        if !species.contains(id) {
            continue;
        }
        let candidates = match_entities(&kb, name, ScreenKind::Fishing, 0.85);
        if name.contains(' ') {
            // A multi-word species resolves as a PHRASE: exactly one identity,
            // and no contained stem may compete.
            assert_eq!(
                candidates.len(),
                1,
                "{name} must resolve to exactly one identity, got {:?}",
                candidates.iter().map(|c| &c.entity_id).collect::<Vec<_>>()
            );
        } else {
            // A single-word species resolves through the per-word stage, which
            // legitimately lists up to 5 candidates; the top one confirms.
            assert!(!candidates.is_empty(), "{name} produced no candidates");
        }
        assert_eq!(
            candidates[0].entity_id, id,
            "{name} resolved to the wrong entity"
        );
        checked += 1;
    }
    assert_eq!(checked, 16, "all 16 documented fish must be checked");

    // And the guard the whole fix exists for: a species name must never offer
    // a legacy stem as a competing candidate.
    let candidates = match_entities(&kb, "Crimson Snapper", ScreenKind::Fishing, 0.85);
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].entity_id, "fish:crimsonsnapper");
    assert!(!candidates.iter().any(|c| c.entity_id == "fish:crimson"));
    assert!(!candidates.iter().any(|c| c.entity_id == "fish:snapper"));
}

// ---------------------------------------------------------------------------
// Training-projection drift guard
// ---------------------------------------------------------------------------
//
// The other half of the ontology has two implementations too: the in-app
// snapshot projection (`core::training::project_training_labels`) and the Python
// dataset builder (`ml/gpo_train/reviewed_dataset.py`). Both decide what trains,
// from the same `labels.jsonl` + `reviews.jsonl`. If they ever disagree about a
// class's row count, the trainer builds a different dataset than the gate
// measured - the exact failure this repository exists to prevent.
//
// The fixture holds the inputs AND the manifest the Python builder produces from
// them, so this test compares the two implementations on one set of inputs.
// `tests/test_projected_fixture.py` (Python) re-derives the manifest from the same
// embedded inputs, so a stale expectation cannot hide here.

fn fixture_projection() -> Option<(
    serde_json::Value,
    gpo_autofish_lib::core::training::ProjectedLabels,
)> {
    use gpo_autofish_lib::core::ml_dataset::MlAnnotation;
    use gpo_autofish_lib::core::review::ReviewStore;

    let path = find_repo_file(
        &std::path::Path::new("src-tauri")
            .join("tests")
            .join("fixtures")
            .join("projected_class_distribution.json"),
    )?;
    let fx: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()?;

    let rows: Vec<MlAnnotation> = fx["labels"]
        .as_array()
        .expect("fixture labels")
        .iter()
        .map(|r| serde_json::from_value(r.clone()).expect("fixture label parses"))
        .collect();

    // The projection reads reviews through a ReviewStore, so it is materialised
    // in a temp dir. The fixture's own inputs are never written to.
    // Both projection tests run concurrently. Millisecond timestamps alone
    // can collide, letting one test remove the other's review file.
    static NEXT_FIXTURE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let fixture_id = NEXT_FIXTURE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "gpo-projection-{}-{}-{fixture_id}",
        std::process::id(),
        gpo_autofish_lib::events::now_ms(),
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let mut body = String::new();
    for r in fx["reviews"].as_array().expect("fixture reviews") {
        body.push_str(&serde_json::to_string(r).unwrap());
        body.push('\n');
    }
    std::fs::write(dir.join("reviews.jsonl"), body).unwrap();
    let projected = gpo_autofish_lib::core::training::project_training_labels(
        &rows,
        &ReviewStore::new(dir.clone()),
    );
    assert!(projected.is_ok(), "{:?}", projected.read_error);
    let _ = std::fs::remove_dir_all(&dir);
    Some((fx, projected))
}

#[test]
fn the_projected_class_distribution_equals_the_python_dataset_manifest() {
    use gpo_autofish_lib::core::ml_dataset::MlDatasetStore;

    let Some((fx, projected)) = fixture_projection() else {
        eprintln!("SKIP: projected_class_distribution.json not found");
        return;
    };
    let manifest = &fx["manifest"];
    let by_id: std::collections::HashMap<&str, &gpo_autofish_lib::core::ml_dataset::MlAnnotation> =
        projected
            .rows
            .iter()
            .map(|r| (r.image_id.as_str(), r))
            .collect();

    let manifest_rows = manifest["rows"].as_array().expect("manifest rows");
    assert_eq!(
        manifest_rows.len(),
        manifest["rows_included"].as_u64().expect("rows_included") as usize
    );

    // Per row: the human verdict the projection applied IS the label the Python
    // builder recorded, and the split is the same function on both sides.
    let mut per_class: std::collections::BTreeMap<
        String,
        (usize, std::collections::BTreeSet<String>, usize, usize),
    > = Default::default();
    for m in manifest_rows {
        let image_id = m["image_id"].as_str().unwrap();
        let row = by_id
            .get(image_id)
            .unwrap_or_else(|| panic!("{image_id} is in the manifest but not in the projection"));
        assert_eq!(
            row.entity_id.as_deref(),
            m["entity_id"].as_str(),
            "{image_id}: the projection and the manifest disagree on the label"
        );
        assert_eq!(
            MlDatasetStore::split_of(&row.session_id).as_str(),
            m["split"].as_str().unwrap(),
            "{image_id}: session split drifted between Rust and Python"
        );
        assert_eq!(
            row.hard_example,
            m["hard_example"].as_bool().unwrap_or(false),
            "{image_id}"
        );
        assert_eq!(
            !row.ocr_text.trim().is_empty(),
            m["ocr_text_present"].as_bool().unwrap_or(false),
            "{image_id}"
        );
        let e = row.entity_id.clone().unwrap();
        let split = MlDatasetStore::split_of(&row.session_id).as_str();
        let slot = per_class.entry(e).or_default();
        slot.0 += 1;
        slot.1.insert(row.session_id.clone());
        if split == "validation" {
            slot.2 += 1;
        }
        if split == "test" {
            slot.3 += 1;
        }
    }

    // Per class: rows, sessions, val/test support.
    let declared = manifest["per_class"].as_object().expect("per_class");
    assert_eq!(
        declared.len(),
        per_class.len(),
        "the manifest declares classes the projection does not produce: {:?}",
        declared.keys().collect::<Vec<_>>()
    );
    for (cls, d) in declared {
        let (rows, sessions, val, test) = per_class.get(cls).unwrap_or_else(|| {
            panic!("{cls}: manifest declares a class the projection never produces")
        });
        assert_eq!(d["rows"].as_u64().unwrap() as usize, *rows, "{cls} rows");
        assert_eq!(
            d["sessions"].as_u64().unwrap() as usize,
            sessions.len(),
            "{cls} sessions"
        );
        assert_eq!(
            d["val_support"].as_u64().unwrap() as usize,
            *val,
            "{cls} val support"
        );
        assert_eq!(
            d["test_support"].as_u64().unwrap() as usize,
            *test,
            "{cls} test support"
        );
    }

    // The class MAP: sorted, and exactly the set the projection produces for the
    // manifest's rows.
    let class_map: Vec<String> = manifest["class_map"]
        .as_array()
        .expect("class_map")
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect();
    let mut from_projection: Vec<String> = per_class.keys().cloned().collect();
    from_projection.sort();
    assert_eq!(class_map, from_projection);
    assert_eq!(
        class_map.len(),
        manifest["class_map_width"].as_u64().unwrap() as usize
    );
    // Same fingerprint rule as `class_vocab::fish_vocab_sha` and
    // `reviewed_dataset.build`: SHA-256 of the newline-joined list.
    assert_eq!(
        manifest["vocab_sha256"].as_str().unwrap(),
        gpo_autofish_lib::core::ml_model::sha256_hex(class_map.join("\n").as_bytes())
    );
}

#[test]
fn the_projection_keeps_exactly_the_rows_the_manifest_omits_by_design() {
    let Some((fx, projected)) = fixture_projection() else {
        eprintln!("SKIP: projected_class_distribution.json not found");
        return;
    };
    let manifest_ids: std::collections::HashSet<&str> = fx["manifest"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["image_id"].as_str().unwrap())
        .collect();
    let projected_ids: std::collections::HashSet<&str> =
        projected.rows.iter().map(|r| r.image_id.as_str()).collect();

    // Rows a human rejected are gone from the projection entirely.
    let rejected: Vec<&str> = fx["reviews"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["training_eligible"].as_bool() == Some(false))
        .map(|r| r["image_id"].as_str().unwrap())
        .collect();
    assert!(!rejected.is_empty(), "the fixture must exercise exclusion");
    for id in &rejected {
        assert!(
            !projected_ids.contains(id),
            "{id} is review-excluded but projected"
        );
    }

    // Two DIFFERENCES between the two rule sets, both deliberate, and both
    // accounted for here so a third one cannot appear unnoticed:
    //
    //  1. UNREVIEWED rows: the snapshot keeps them (with the collector's label -
    //     unreviewed data keeps exactly the meaning it had); the Python builder
    //     only admits reviewed rows.
    //  2. CLASS-GATE-BLOCKED rows: the snapshot freezes everything the review
    //     allows, while `reviewed_dataset.build` drops a class that has not
    //     reached ENTITY_MIN_EXAMPLES/ENTITY_MIN_SESSIONS. That gate is applied
    //     LATER and elsewhere - as the trainer's scope assert, and as the
    //     eligibility check in `family_eligibility` - not as a data filter.
    let blocked: Vec<&str> = fx["manifest"]["blocked_classes"]
        .as_object()
        .expect("blocked classes")
        .keys()
        .map(String::as_str)
        .collect();
    assert!(
        !blocked.is_empty(),
        "the fixture must exercise a blocked class"
    );
    let blocked_rows = projected
        .rows
        .iter()
        .filter(|r| r.entity_id.as_deref().is_some_and(|e| blocked.contains(&e)))
        .count();
    assert!(
        blocked_rows > 0,
        "a blocked class with no rows would prove nothing"
    );

    let omitted: Vec<&str> = projected_ids
        .iter()
        .copied()
        .filter(|id| !manifest_ids.contains(id))
        .collect();
    assert_eq!(
        omitted.len(),
        projected.rows_unreviewed + blocked_rows,
        "projection = manifest rows + unreviewed + class-gate-blocked, nothing else"
    );
    for id in &omitted {
        let row = projected
            .rows
            .iter()
            .find(|r| r.image_id == *id)
            .expect("projected");
        let e = row.entity_id.as_deref().unwrap_or("");
        let has_review = fx["reviews"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["image_id"].as_str() == Some(*id));
        assert!(
            !has_review || blocked.contains(&e),
            "{id} is omitted by the manifest for an unexplained reason (entity {e}, reviewed: {has_review})"
        );
    }
    assert!(
        projected.rows_relabelled_by_review > 0,
        "the fixture must exercise relabelling"
    );
    assert_eq!(
        projected.rows.len(),
        manifest_ids.len() + omitted.len(),
        "no projected row may exist outside the manifest for an unaccounted reason"
    );
}
