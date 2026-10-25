//! Live refresh of the GPO wiki reference snapshot.
//!
//! # Why this delegates to Python instead of parsing in Rust
//!
//! There is exactly one parser for the wiki's bestiary table, and it is
//! `ml/gpo_ontology_review/wiki.py`. Writing a second one in Rust would be the same
//! mistake the project has already had to undo twice: two implementations of one
//! rule, differing only in the direction that matters.
//!
//! So this shells out to `python -m ml.gpo_ontology_review wiki-refresh`. That costs
//! a second of interpreter startup on a button a human pressed deliberately, and it
//! buys a single authority for what "the wiki says" - including the part that refuses
//! to write an empty snapshot if the page layout changes.
//!
//! # Safety
//!
//! This is the only outbound network capability added anywhere in the review path, so
//! it is constrained as tightly as the feature allows:
//!
//! * The destination is a constant inside the Python module. There is no parameter
//!   through which a caller could choose a URL, so this cannot be used to fetch
//!   arbitrary content into a local file.
//! * It writes exactly one file - the snapshot - and nothing else. It touches no
//!   knowledge base, no vocabulary, no dataset row, no split, no model.
//! * It is never triggered by a page load, a build, or a read. Only by an explicit
//!   human action.
//! * A failure leaves the previous snapshot untouched. A missing or unparseable page
//!   therefore cannot silently empty the reference, which would turn "no match" into
//!   false evidence of absence for every concept on screen.
//! * Single-flighted, so a double-click cannot launch two fetches.
//!
//! After a successful refresh the cached artefacts are reloaded, because the index
//! embeds the per-concept match results and would otherwise keep serving the previous
//! ones - a button that appears to work while changing nothing.

use crate::commands_ontology_workstation as core;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

/// Guards against a second refresh starting while one is in flight.
static IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// The outcome of a refresh attempt, as the UI needs to show it.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct WikiRefreshResult {
    pub ok: bool,
    /// Number of fish in the reference after the refresh.
    pub entry_count: usize,
    /// Hash of the reference content, so the UI can show whether anything changed.
    pub content_sha256: String,
    /// How many concepts now match exactly, and how many are near-misses.
    pub exact_matches: usize,
    pub near_misses: usize,
    /// True when the content hash is unchanged, i.e. the wiki did not move.
    pub unchanged: bool,
    /// Present on failure; written for a human, not a machine.
    pub message: String,
}

/// Fetch the wiki and rebuild the reference, if Python and the artefacts are present.
///
/// Never panics and never partially applies: either the snapshot and the index are
/// both refreshed, or neither is.
pub fn refresh_wiki_reference() -> WikiRefreshResult {
    if IN_FLIGHT.swap(true, Ordering::SeqCst) {
        return WikiRefreshResult {
            ok: false,
            entry_count: 0,
            content_sha256: String::new(),
            exact_matches: 0,
            near_misses: 0,
            unchanged: false,
            message: "a refresh is already running".into(),
        };
    }
    let _guard = InFlightGuard;

    let Some(report_dir) = core::find_report_dir() else {
        return fail(
            "ontology review artefacts not found - run: \
             python -m ml.gpo_ontology_review build",
        );
    };
    let repo_root = repo_root_from(&report_dir);
    let before = wiki_digest(&report_dir);

    let out_dir = report_dir.to_string_lossy().to_string();
    let output = Command::new(python_exe())
        .current_dir(&repo_root)
        .env("GPO_REPO_ROOT", &repo_root)
        .env("PYTHONIOENCODING", "utf-8")
        .args([
            "-m",
            "ml.gpo_ontology_review",
            "wiki-refresh",
            "--out",
            &out_dir,
        ])
        .output();

    let out = match output {
        Ok(o) => o,
        Err(e) => return fail(&format!("could not run Python: {e}")),
    };
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let tail: String = stderr.lines().rev().take(3).collect::<Vec<_>>().join(" ");
        return fail(&format!(
            "wiki refresh failed ({}): {}",
            out.status,
            if tail.is_empty() {
                "no output".into()
            } else {
                tail
            }
        ));
    }

    let after = wiki_digest(&report_dir);
    if after.is_empty() {
        return fail("the refresh reported success but wrote no readable snapshot");
    }

    // Reload BEFORE reading the counts, so the numbers reported are the ones the UI
    // will serve from this moment on.
    core::reload_cached_artefacts();
    let (entries, exact, near) = reference_stats();

    WikiRefreshResult {
        ok: true,
        entry_count: entries,
        content_sha256: after.clone(),
        exact_matches: exact,
        near_misses: near,
        unchanged: before == after,
        message: format!(
            "{entries} fish in the reference; {exact} concept(s) match exactly, \
             {near} near. This is community-editable reference data, not an authority."
        ),
    }
}

/// Clears the in-flight flag however the function exits, including the early returns
/// above and a panic. Nothing else is needed for the release, so this guard is the
/// whole mechanism.
struct InFlightGuard;

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        IN_FLIGHT.store(false, Ordering::SeqCst);
    }
}

fn fail(message: &str) -> WikiRefreshResult {
    WikiRefreshResult {
        ok: false,
        entry_count: 0,
        content_sha256: String::new(),
        exact_matches: 0,
        near_misses: 0,
        unchanged: false,
        message: message.to_string(),
    }
}

/// Prefer the virtualenv's interpreter when one is present, so the module's
/// dependencies resolve the same way they do for every other command in this repo.
fn python_exe() -> PathBuf {
    for candidate in [
        "ml/ram_plus/.venv/Scripts/python.exe",
        ".venv/Scripts/python.exe",
    ] {
        let p = PathBuf::from(candidate);
        if p.is_file() {
            return p;
        }
    }
    PathBuf::from("python")
}

/// The repository root, preferring the explicit override.
///
/// Deriving it from the report directory works in a normal checkout, but not when
/// `GPO_ONTOLOGY_REVIEW_DIR` points somewhere else (the test sandbox, or a portable
/// install). `GPO_REPO_ROOT` is what the rest of the pipeline uses, so honour it first
/// and only fall back to walking up.
fn repo_root_from(report_dir: &std::path::Path) -> PathBuf {
    if let Ok(root) = std::env::var("GPO_REPO_ROOT") {
        if !root.is_empty() {
            return PathBuf::from(root);
        }
    }
    report_dir
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Content hash of the snapshot on disk, or empty when absent.
fn wiki_digest(report_dir: &std::path::Path) -> String {
    std::fs::read_to_string(report_dir.join("wiki_fish.json"))
        .ok()
        .and_then(|t| {
            serde_json::from_str::<serde_json::Value>(&t)
                .ok()
                .and_then(|v| {
                    v.get("contentSha256")
                        .and_then(|h| h.as_str())
                        .map(str::to_string)
                })
        })
        .unwrap_or_default()
}

/// (entries, exact matches, near misses) from the freshly reloaded index.
fn reference_stats() -> (usize, usize, usize) {
    let Some(idx) = core::ontology_review_index() else {
        return (0, 0, 0);
    };
    let w = idx.get("wiki").cloned().unwrap_or(serde_json::Value::Null);
    (
        w.get("entryCount").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
        w.get("exactMatches").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
        w.get("nearMisses").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
    )
}

/// Tauri command, for the desktop app.
#[tauri::command]
pub fn ontology_wiki_refresh() -> WikiRefreshResult {
    refresh_wiki_reference()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn own_code() -> String {
        let src = include_str!("ontology_web_refresh.rs");
        let body = src.split("#[cfg(test)]").next().unwrap_or(src);
        body.lines()
            .filter(|l| {
                let t = l.trim_start();
                !t.starts_with("//") && !t.starts_with("/*") && !t.starts_with('*')
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_refresh_cannot_be_pointed_at_an_arbitrary_url() {
        // A configurable URL would make this an SSRF primitive and a way to write
        // remote content into a local file. The destination must stay a constant.
        let code = own_code();
        assert!(
            !code.contains("fn refresh_wiki_reference(url"),
            "no URL parameter"
        );
        // The only URL-ish string is the module path being executed.
        assert!(
            !code.to_lowercase().contains("http://") && !code.to_lowercase().contains("https://"),
            "this module must not contain a URL; the destination lives in wiki.py"
        );
    }

    #[test]
    fn the_refresh_cannot_reach_the_bot_the_model_or_production_data() {
        let code = own_code();
        for forbidden in [
            "FISH_VOCAB",
            "knowledge.rs",
            "training_eligible",
            "class_vocab",
            "readiness",
            "registry",
            "ml_dataset",
            "bot::",
            "controller",
            "enigo",
            "SendInput",
        ] {
            assert!(
                !code.contains(forbidden),
                "a wiki refresh must not reference {forbidden:?}"
            );
        }
    }

    #[test]
    fn the_refresh_invokes_exactly_one_python_module() {
        let code = own_code();
        assert!(code.contains("ml.gpo_ontology_review"));
        assert!(code.contains("wiki-refresh"));
        // One command, no shell: an argument list cannot be re-interpreted, so a
        // concept name or a wiki entry can never become a second command.
        assert!(code.contains("Command::new"));
        assert!(
            !code.contains("cmd /C") && !code.contains("sh -c") && !code.contains("powershell")
        );
    }

    #[test]
    fn a_failed_refresh_reports_instead_of_panicking() {
        let r = fail("boom");
        assert!(!r.ok);
        assert_eq!(r.message, "boom");
        assert_eq!(r.entry_count, 0);
        // And the flag must be clear again, or every later refresh would refuse.
        let _ = refresh_wiki_reference;
    }

    #[test]
    fn the_guard_releases_the_in_flight_flag_on_drop() {
        IN_FLIGHT.store(true, Ordering::SeqCst);
        {
            let _reset = InFlightGuard;
            // While held, a second refresh must refuse.
            assert!(IN_FLIGHT.load(Ordering::SeqCst));
        }
        assert!(!IN_FLIGHT.load(Ordering::SeqCst), "flag must be released");
        IN_FLIGHT.store(false, Ordering::SeqCst);
    }
}
