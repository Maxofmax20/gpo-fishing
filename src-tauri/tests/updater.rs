//! Updater/release configuration tests (no network, no secrets).
//!
//! Guards the production update pipeline statically:
//! - HTTPS release endpoint + configured public key (signature verification)
//! - version consistency across manifests (tag gate runs the PS script in CI)
//! - NSIS bundle + updater artifacts enabled
//! - repo packager script present
//!
//! These tests never touch private keys and never perform an install.

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri parent")
        .to_path_buf()
}

fn tauri_conf() -> serde_json::Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
    let raw = std::fs::read_to_string(&p).expect("tauri.conf.json readable");
    serde_json::from_str(&raw).expect("tauri.conf.json parses")
}

fn cargo_version() -> String {
    let raw = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
        .expect("Cargo.toml readable");
    raw.lines()
        .find_map(|l| {
            let t = l.trim();
            t.strip_prefix("version").and_then(|v| {
                v.trim()
                    .strip_prefix('=')
                    .map(|s| s.trim().trim_matches('"').to_string())
            })
        })
        .expect("Cargo.toml has version")
}

#[test]
fn updater_endpoint_is_https_github_releases() {
    let conf = tauri_conf();
    let endpoints = conf
        .pointer("/plugins/updater/endpoints")
        .and_then(|e| e.as_array())
        .expect("updater.endpoints array");
    assert!(!endpoints.is_empty(), "updater needs at least one endpoint");
    for ep in endpoints {
        let url = ep.as_str().expect("endpoint is a string");
        assert!(
            url.starts_with("https://github.com/") && url.contains("/releases/"),
            "endpoint must be an HTTPS GitHub releases URL, got: {url}"
        );
    }
}

#[test]
fn updater_pubkey_configured() {
    let conf = tauri_conf();
    let key = conf
        .pointer("/plugins/updater/pubkey")
        .and_then(|k| k.as_str())
        .expect("updater.pubkey string");
    // minisign public key: base64 comment + key, well over 50 chars.
    assert!(key.len() > 50, "pubkey looks truncated");
    assert!(
        !key.contains("PRIVATE"),
        "private key must never be configured here"
    );
}

#[test]
fn manifest_versions_agree() {
    let conf = tauri_conf();
    let tauri_ver = conf
        .get("version")
        .and_then(|v| v.as_str())
        .expect("tauri version");
    let cargo_ver = cargo_version();
    assert_eq!(tauri_ver, cargo_ver, "tauri.conf.json vs Cargo.toml");
    let pkg_raw =
        std::fs::read_to_string(repo_root().join("package.json")).expect("package.json readable");
    let pkg: serde_json::Value = serde_json::from_str(&pkg_raw).expect("package.json parses");
    let pkg_ver = pkg
        .get("version")
        .and_then(|v| v.as_str())
        .expect("package version");
    assert_eq!(tauri_ver, pkg_ver, "tauri.conf.json vs package.json");
}

#[test]
fn bundle_targets_nsis_with_updater_artifacts() {
    let conf = tauri_conf();
    let targets = conf
        .pointer("/bundle/targets")
        .and_then(|t| t.as_array())
        .expect("bundle.targets array");
    assert!(
        targets.iter().any(|t| t.as_str() == Some("nsis")),
        "nsis target required"
    );
    assert_eq!(
        conf.pointer("/bundle/createUpdaterArtifacts")
            .and_then(|v| v.as_bool()),
        Some(true),
        "updater artifacts must be enabled"
    );
}

#[test]
fn repo_packager_scripts_present() {
    let root = repo_root();
    assert!(
        root.join("src-tauri/latest-json.ps1").is_file(),
        "latest-json.ps1 present"
    );
    assert!(
        root.join("scripts/check-versions.ps1").is_file(),
        "check-versions.ps1 present"
    );
    assert!(
        root.join("build_release.ps1").is_file(),
        "build_release.ps1 present"
    );
}

#[test]
fn release_workflow_has_no_failure_hiding() {
    let wf = std::fs::read_to_string(repo_root().join(".github/workflows/release.yml"))
        .expect("release.yml readable");
    let quality = std::fs::read_to_string(repo_root().join(".github/workflows/quality.yml"))
        .expect("shared quality workflow readable");
    let combined = format!("{wf}\n{quality}");
    let code_lines: Vec<&str> = combined
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !(t.starts_with('#') || t.starts_with("//"))
        })
        .collect();
    let code = code_lines.join("\n");
    assert!(
        !code.contains("continue-on-error"),
        "critical release steps must not hide failures"
    );
    assert!(
        !code.contains("|| true"),
        "critical release steps must not hide failures"
    );
    assert!(
        wf.contains("check-versions"),
        "version gate must run in release"
    );
    let quality_job = wf
        .split("  quality-gates:")
        .nth(1)
        .expect("quality gate job")
        .split("\n  # Build")
        .next()
        .unwrap();
    assert!(
        quality_job.contains("uses: ./.github/workflows/quality.yml"),
        "release must call the tested shared workflow"
    );
    assert!(
        quality.contains("cargo test --locked"),
        "shared gate must run locked Rust tests"
    );
    assert!(
        quality.contains("npm test") && quality.contains("python -m pytest"),
        "all quality suites remain mandatory"
    );
    let publish = wf
        .split("  publish:")
        .nth(1)
        .expect("separate publication job");
    assert!(
        publish.contains("needs: [quality-gates, release]"),
        "publication must depend on both all tests and the signed build"
    );
    assert!(
        publish.contains("sha256sum --check SHA256SUMS.txt"),
        "published files must pass checksum verification"
    );
    let build = wf
        .split("  release:")
        .nth(1)
        .unwrap()
        .split("  publish:")
        .next()
        .unwrap();
    assert!(
        build.contains("releaseDraft: true"),
        "parallel build can create only a private draft"
    );
    assert!(
        !build.contains("--draft=false"),
        "parallel build must not publish before tests"
    );
    assert!(
        wf.contains("TAURI_SIGNING_PRIVATE_KEY"),
        "signing must use CI secrets"
    );
    assert!(
        !wf.contains("PRIVATE KEY----") && !wf.contains("BEGIN PRIVATE"),
        "no keys in workflow"
    );
}
