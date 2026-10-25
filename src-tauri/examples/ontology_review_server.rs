//! Boots ONLY the ontology review web server. No Tauri, no bot, no game, no updater.
//!
//! This exists so the review workstation can be exercised in a real browser
//! independently of the desktop application - and so that independence is
//! demonstrable rather than asserted. If this binary can serve the page, then the
//! page does not need the macro.
//!
//! It is not a second server: it calls `ontology_web::spawn()`, the same entry
//! point the application calls, so what runs here is exactly what runs there.
//!
//! Usage:  cargo run --example ontology_review_server
//!         (then open the printed URL)

fn main() {
    // Point at the real report artefacts. Absolute, because this binary's working
    // directory differs from the app's when run from a target directory.
    if std::env::var("GPO_ONTOLOGY_REVIEW_DIR").is_err() {
        if let Ok(root) = std::env::var("GPO_REPO_ROOT") {
            let dir = format!("{root}/reports/ontology_review");
            std::env::set_var("GPO_ONTOLOGY_REVIEW_DIR", dir);
        }
    }

    gpo_autofish_lib::ontology_web::spawn();

    println!();
    println!("  Ontology Review Workstation (standalone)");
    println!("  ----------------------------------------");
    println!("  {}", gpo_autofish_lib::ontology_web::review_url());
    println!("  bound to loopback only; no game, macro, or updater involved");
    println!();
    println!("  Ctrl+C to stop.");

    // Park forever; the server runs on its own thread.
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}