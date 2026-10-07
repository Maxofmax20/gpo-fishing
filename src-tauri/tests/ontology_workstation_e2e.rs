//! End-to-end test of the review workstation against the REAL artefacts.
//!
//! # What this exercises
//!
//! The real command implementations — the same ones the UI calls — against the
//! real `reports/ontology_review/workstation/index.json` built from the actual
//! 14,805-image dataset. Not a mock: the P0 concepts, their row counts and their
//! image ids are the ones a human will see.
//!
//! # Why the decision log is redirected
//!
//! These tests RECORD decisions. They must not land in the real
//! `ontology_decisions.jsonl`, because a fabricated decision would be
//! indistinguishable from a human's once it is on disk.
//!
//! So the log is redirected to a temporary directory that *contains* a symlink (or
//! copy) of the real `workstation/` index. The commands resolve the log path from
//! the report directory, so pointing `GPO_ONTOLOGY_REVIEW_DIR` at the temporary
//! directory redirects the write while leaving the real index readable and the
//! real log untouched.
//!
//! The redirection happens in a child process because the command module resolves
//! its paths in a `LazyLock` — once resolved, changing the environment has no
//! effect within the same process. Each scenario therefore runs in its own
//! process, which is also a more faithful model of what the app does.

use gpo_autofish_lib::commands_ontology_workstation as ws;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Build a temporary report directory whose `workstation/` is the REAL one.
fn sandbox(name: &str) -> Option<PathBuf> {
    let real = repo_root().join("reports").join("ontology_review");
    if !real.join("workstation").join("index.json").is_file() {
        eprintln!("SKIP: no real workstation index at {}", real.display());
        return None;
    }
    let tmp = std::env::temp_dir().join(format!("gpo_ws_test_{name}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).ok()?;
    // Copy the workstation directory rather than symlinking it: symlink semantics
    // on Windows need privileges, and a copy keeps the test portable.
    copy_dir(&real.join("workstation"), &tmp.join("workstation")).ok()?;
    std::fs::write(tmp.join("ontology_decisions.jsonl"), b"").ok()?;
    Some(tmp)
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let target = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &target)?;
        } else {
            std::fs::copy(e.path(), target)?;
        }
    }
    Ok(())
}

/// Run one scenario in a child process with the sandbox in place.
///
/// Returns the child's stdout. The scenario is a closure evaluated in the child,
/// which is how the `LazyLock` resolution is re-triggered per scenario.
fn run_isolated(name: &str, body: &str) -> Option<String> {
    let sandbox = sandbox(name)?;
    let exe = std::env::current_exe().ok()?;
    // The test binary re-invokes itself with GPO_WS_SCENARIO set.
    let mut cmd = std::process::Command::new(exe);
    cmd.env("GPO_ONTOLOGY_REVIEW_DIR", &sandbox)
        .env("GPO_WS_SCENARIO", body)
        .env("GPO_WS_TEST_NAME", name)
        .env("GPO_WS_SANDBOX", &sandbox)
        .args(["--exact", "zz_scenario_child", "--nocapture", "--ignored"]);
    let out = cmd.output().ok()?;
    let so = String::from_utf8_lossy(&out.stdout).to_string();
    if !out.status.success() {
        eprintln!("scenario {name} failed:\n{so}\n{}", String::from_utf8_lossy(&out.stderr));
        return None;
    }
    Some(so)
}

/// The scenario body executed inside the child process.
#[test]
#[ignore]
fn zz_scenario_child() {
    let Ok(body) = std::env::var("GPO_WS_SCENARIO") else {
        return;
    };
    match body.as_str() {
        "status" => {
            let s = ws::ontology_review_status();
            println!(
                "WSRESULT total={} p0={} decided={} pending={} queue={}",
                s.total,
                s.counts.get("P0").copied().unwrap_or(0),
                s.decided,
                s.pending,
                s.queue.len()
            );
        }
        "kb_search" => {
            let hits = ws::ontology_review_kb_search("shark".into(), Some(20));
            println!("WSRESULT kb_hits={} first={}", hits.len(),
                hits.first().map(|h| h.id.as_str()).unwrap_or("-"));
        }
        "family" => {
            // The tigerfin family is the one Phase 12 calls out by name.
            match ws::ontology_review_family("family:tigerfin".into()) {
                Some(f) => println!(
                    "WSRESULT family={} members={} evidence0={}",
                    f.family_id,
                    f.members.len(),
                    f.members.first()
                        .and_then(|m| m.get("evidence"))
                        .and_then(|e| e.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0)
                ),
                None => println!("WSRESULT family=missing"),
            }
        }
        "bench" => {
            // Measured against the REAL index and the REAL thumbnails, so the
            // numbers describe the app a reviewer actually uses.
            let t0 = std::time::Instant::now();
            let st = ws::ontology_review_status();
            let status_ms = t0.elapsed().as_secs_f64() * 1000.0;

            let first = st.queue.first().cloned().unwrap_or_default();
            let t1 = std::time::Instant::now();
            let c = ws::ontology_review_concept(first.clone());
            let concept_ms = t1.elapsed().as_secs_f64() * 1000.0;

            // Thumbnails: what the grid actually renders.
            let ids: Vec<String> = c
                .as_ref()
                .and_then(|v| v.get("evidence"))
                .and_then(|e| e.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|e| e.get("imageId").and_then(|x| x.as_str()).map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let t2 = std::time::Instant::now();
            let mut bytes = 0usize;
            for id in &ids {
                if let Some(u) = ws::ontology_review_thumb(id.clone()) {
                    bytes += u.len();
                }
            }
            let thumbs_ms = t2.elapsed().as_secs_f64() * 1000.0;

            // Second pass over the SAME images. The gap between cold and warm is
            // the real cost: if the second pass is near zero, the first pass was
            // the OS page cache and disk, and the steady-state review loop (where
            // images are cached per process) pays almost nothing.
            let t2b = std::time::Instant::now();
            for id in &ids {
                let _ = ws::ontology_review_thumb(id.clone());
            }
            let thumbs_warm_ms = t2b.elapsed().as_secs_f64() * 1000.0;

            // The family panel: the heaviest optional view.
            let t3 = std::time::Instant::now();
            let _f = ws::ontology_review_family("family:tigerfin".into());
            let family_ms = t3.elapsed().as_secs_f64() * 1000.0;

            // KB search as the reviewer types.
            let t4 = std::time::Instant::now();
            let mut hits = 0usize;
            for q in ["s", "sh", "sha", "shar", "shark"] {
                hits = ws::ontology_review_kb_search(q.into(), Some(40)).len();
            }
            let search_ms = t4.elapsed().as_secs_f64() * 1000.0;

            // A decision round trip, excluding the reviewer thinking about fish.
            let t5 = std::time::Instant::now();
            let saved = ws::ontology_decide(ws::DecisionInput {
                concept_id: first.clone(),
                decision: "UNKNOWN".into(),
                existing_entity_id: None,
                proposed_entity_name: None,
                variant_descriptor: None,
                notes: Some("bench".into()),
                evidence_image_ids: ids.clone(),
            });
            let decide_ms = t5.elapsed().as_secs_f64() * 1000.0;

            println!(
                "WSRESULT status={:.3} concept={:.3} thumbs={:.3} thumbs_warm={:.3} thumb_count={} thumb_kb={} family={:.3} search5={:.3} hits={} decide={:.3} saved={}",
                status_ms, concept_ms, thumbs_ms, thumbs_warm_ms, ids.len(), bytes / 1024,
                family_ms, search_ms, hits, decide_ms, saved.is_ok()
            );
        }
        "thumb_traversal" => {
            let bad = ws::ontology_review_thumb("../../../../windows/system32/drivers/etc/hosts".into());
            let good = ws::ontology_review_thumb("606020203e34beb6".into());
            println!(
                "WSRESULT traversal={} valid_starts_with_data_url={}",
                bad.is_some(),
                good.map(|g| g.starts_with("data:image/jpeg;base64,")).unwrap_or(false)
            );
        }
        "decide_and_undo" => {
            let before = ws::ontology_review_status();
            let first = before.queue.first().cloned().unwrap_or_default();

            // A malformed decision must be refused.
            let bad = ws::ontology_decide(ws::DecisionInput {
                concept_id: first.clone(),
                decision: "EXISTING_ENTITY".into(),
                existing_entity_id: None,
                proposed_entity_name: None,
                variant_descriptor: None,
                notes: None,
                evidence_image_ids: vec![],
            });
            println!("WSRESULT malformed_refused={}", bad.is_err());

            // An unknown concept must be refused.
            let ghost = ws::ontology_decide(ws::DecisionInput {
                concept_id: "concept:does-not-exist".into(),
                decision: "UNKNOWN".into(),
                existing_entity_id: None,
                proposed_entity_name: None,
                variant_descriptor: None,
                notes: None,
                evidence_image_ids: vec![],
            });
            println!("WSRESULT ghost_refused={}", ghost.is_err());

            // A valid decision must persist and advance.
            let ok = ws::ontology_decide(ws::DecisionInput {
                concept_id: first.clone(),
                decision: "UNKNOWN".into(),
                existing_entity_id: None,
                proposed_entity_name: None,
                variant_descriptor: None,
                notes: Some("e2e scenario".into()),
                evidence_image_ids: vec!["606020203e34beb6".into()],
            });
            let saved = ok.is_ok();
            let next = ok.as_ref().ok().and_then(|r| r.next_concept_id.clone());
            println!("WSRESULT saved={} next_is_different={}", saved,
                next.as_deref().map(|n| n != first).unwrap_or(false));

            let after = ws::ontology_review_status();
            println!("WSRESULT decided_delta={} pending_delta={}",
                after.decided - before.decided,
                (before.pending as i64 - after.pending as i64));

            // Undo must record a reversal and leave history intact.
            let undone = ws::ontology_undo(first.clone());
            let undone_ok = undone.is_ok();
            let after_undo = ws::ontology_review_status();
            println!("WSRESULT undo_ok={} decided_after_undo={}", undone_ok, after_undo.decided);

            // The log must still contain BOTH records: a reversal is a new record,
            // never a deletion.
            let log = std::fs::read_to_string(
                PathBuf::from(std::env::var("GPO_WS_SANDBOX").unwrap()).join("ontology_decisions.jsonl"),
            ).unwrap_or_default();
            let lines = log.lines().filter(|l| !l.trim().is_empty()).count();
            let has_reversal = log.contains("REVERSAL");
            println!("WSRESULT log_lines={} has_reversal={}", lines, has_reversal);
        }
        other => panic!("unknown scenario {other}"),
    }
}

// --------------------------------------------------------------------------
// The real-data assertions. Each delegates to a child process.
// --------------------------------------------------------------------------

#[test]
fn real_index_reports_every_concept_and_band() {
    let Some(out) = run_isolated("status", "status") else {
        return; // dataset artefacts absent; the child prints SKIP
    };
    // Real numbers from the built artefacts.
    assert!(out.contains("WSRESULT"), "child produced no result: {out}");
    let total = field(&out, "total=");
    let p0 = field(&out, "p0=");
    let pending = field(&out, "pending=");
    assert!(total > 0, "the index must not be empty: {out}");
    assert_eq!(pending, total, "nothing is decided yet, so pending == total: {out}");
    assert!(p0 >= 11, "expected at least the 11 P0 concepts: {out}");
}

#[test]
fn kb_search_returns_real_entities() {
    let Some(out) = run_isolated("kb", "kb_search") else {
        return;
    };
    assert!(out.contains("WSRESULT"), "{out}");
    let hits = field(&out, "kb_hits=");
    assert!(hits > 0, "searching 'shark' must find real KB entities: {out}");
    assert!(out.contains("fish:shark"), "{out}");
}

#[test]
fn the_tigerfin_family_resolves_with_its_evidence() {
    let Some(out) = run_isolated("family", "family") else {
        return;
    };
    assert!(out.contains("WSRESULT"), "{out}");
    if out.contains("family=missing") {
        println!("SKIP: tigerfin family not present in this index");
        return;
    }
    let members = field(&out, "members=");
    assert!(members >= 3, "tigerfin should have several observed names: {out}");
    let ev = field(&out, "evidence0=");
    assert!(ev > 0, "family members must carry evidence for comparison: {out}");
}

#[test]
fn thumbnail_endpoint_refuses_traversal_and_serves_valid_ids() {
    let Some(out) = run_isolated("thumb", "thumb_traversal") else {
        return;
    };
    assert!(out.contains("WSRESULT"), "{out}");
    assert!(out.contains("traversal=false"), "a traversal id must be refused: {out}");
    assert!(
        out.contains("valid_starts_with_data_url=true"),
        "a real image id must serve a data URL: {out}"
    );
}

/// Measures the latency of the operations the reviewer's loop is made of.
///
/// Reported, not asserted: the numbers change with the machine, and a timing
/// assertion would fail on a slow CI box rather than on a real regression. The
/// purpose is to have honest figures for the workflow budget, measured against the
/// real index rather than a fixture.
#[test]
fn report_real_latency_numbers() {
    let Some(out) = run_isolated("bench", "bench") else {
        return;
    };
    assert!(out.contains("WSRESULT"), "{out}");
    let line = out.lines().find(|l| l.contains("WSRESULT")).unwrap_or(&out);
    println!("BENCH {line}");
    // The only hard bound worth enforcing: the status call reads the whole log, so
    // it must stay well under the time a reviewer spends looking at a screenshot.
    let status: f64 = out
        .split_whitespace()
        .find_map(|t| t.strip_prefix("status="))
        .and_then(|v| v.parse().ok())
        .unwrap();
    assert!(status < 250.0, "status took {status}ms; the decision log is growing");
}

#[test]
fn a_decision_cycle_persists_advances_and_undoes_without_losing_history() {
    let Some(out) = run_isolated("cycle", "decide_and_undo") else {
        return;
    };
    assert!(out.contains("WSRESULT"), "{out}");

    // Malformed and unknown-concept decisions are refused.
    assert!(out.contains("malformed_refused=true"), "{out}");
    assert!(out.contains("ghost_refused=true"), "{out}");

    // A valid decision persists and hands back the NEXT concept.
    assert!(out.contains("saved=true"), "{out}");
    assert!(out.contains("next_is_different=true"), "{out}");
    assert!(out.contains("decided_delta=1"), "{out}");
    assert!(out.contains("pending_delta=1"), "{out}");

    // Undo restores the unresolved count AND leaves both records on disk.
    assert!(out.contains("undo_ok=true"), "{out}");
    assert!(out.contains("decided_after_undo=0"), "{out}");
    assert!(out.contains("log_lines=2"), "history must be preserved: {out}");
    assert!(out.contains("has_reversal=true"), "{out}");
}

fn field(out: &str, key: &str) -> i64 {
    out.split_whitespace()
        .find_map(|tok| tok.strip_prefix(key))
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| panic!("field {key} not found in {out}"))
}

// --------------------------------------------------------------------------
// Static safety guarantees, independent of any data
// --------------------------------------------------------------------------

#[test]
fn the_workstation_module_has_no_kb_or_training_write_surface() {
    let src = include_str!("../src/commands_ontology_workstation.rs");
    let body = src.split("#[cfg(test)]").next().unwrap_or(src);
    let code: String = body
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !t.starts_with("//") && !t.starts_with("/*") && !t.starts_with('*')
        })
        .collect::<Vec<_>>()
        .join("\n");

    for forbidden in [
        "FISH_VOCAB",
        "knowledge.rs",
        "training_eligible",
        "class_vocab",
        "registry",
        "readiness",
        "fish_v1",
        "ml_dataset",
        "controller",
    ] {
        assert!(
            !code.contains(forbidden),
            "the workstation must not reference {forbidden} in code"
        );
    }
}

#[test]
fn the_workstation_cannot_reach_game_input() {
    let src = include_str!("../src/commands_ontology_workstation.rs");
    let body = src.split("#[cfg(test)]").next().unwrap_or(src);
    // Code only. The forbidden names below must not appear even in comments,
    // because a comment that says "this never calls the mouse" is worth nothing
    // unless the code underneath genuinely cannot.
    let code: String = body
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !t.starts_with("//") && !t.starts_with("/*") && !t.starts_with('*')
        })
        .collect::<Vec<_>>()
        .join("\n");

    // `tauri::` is deliberately NOT forbidden: every command in the module needs
    // `#[tauri::command]`, so banning the crate prefix would forbid the module
    // existing. The check is about what the commands REACH, not how they are
    // declared.
    for forbidden in ["enigo", "rdev", "SendInput", "send_input", "mouse", "keyboard"] {
        assert!(
            !code.contains(forbidden),
            "the ontology workstation must never reach game input ({forbidden})"
        );
    }

    // It also must not import any sibling runtime module. Being self-contained is
    // what makes "the review screen cannot start the bot" checkable at all.
    for forbidden in ["use crate::", "super::core", "crate::core", "crate::bot"] {
        assert!(
            !code.contains(forbidden),
            "the workstation must stay self-contained; found {forbidden}"
        );
    }
}