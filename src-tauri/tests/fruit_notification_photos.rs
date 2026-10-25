//! Structural screenshot capture guards. Behavioural storage/notification
//! replays live in actions::storage_replay_tests and webhook::fruit_delivery_tests.
//! No game inputs or external network requests are used.

use std::path::PathBuf;

fn source(rel: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{} unreadable: {e}", p.display()))
}

/// Every `ctx.webhook.fruit_*` call in `src`, with the file and the exact line
/// the call starts on.
///
/// Scanned line-wise rather than by splitting the file on `;`. A `;`-split
/// counts newlines per chunk, and this file's comments are full of semicolons,
/// so the reported line drifted by hundreds - a test that names the wrong line
/// for the bug it found is worse than one that names none.
///
/// Each call's extent runs from the line carrying the call to the line where its
/// parentheses balance. `//` comments inside that extent are dropped, so prose
/// about `None` cannot make a correct call site look like a defect.
fn fruit_webhook_calls() -> Vec<(String, usize, String)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().and_then(|s| s.to_str()) != Some("rs") {
                continue;
            }
            let rel = p
                .strip_prefix(PathBuf::from(env!("CARGO_MANIFEST_DIR")))
                .unwrap_or(&p)
                .to_string_lossy()
                .replace('\\', "/");
            let text = std::fs::read_to_string(&p).unwrap_or_default();
            let lines: Vec<&str> = text.lines().collect();

            for (i, line) in lines.iter().enumerate() {
                if !line.contains("ctx.webhook.fruit_") {
                    continue;
                }
                // Walk forward until the call's parentheses balance.
                let mut depth = 0i32;
                let mut extent: Vec<&str> = Vec::new();
                for l in &lines[i..] {
                    let code = l.split("//").next().unwrap_or("");
                    depth += code.matches('(').count() as i32;
                    depth -= code.matches(')').count() as i32;
                    extent.push(code);
                    if depth <= 0 {
                        break;
                    }
                }
                let flat: String = extent
                    .join(" ")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                out.push((rel.clone(), i + 1, flat));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn no_fruit_notification_drops_a_photo_it_already_had() {
    let calls = fruit_webhook_calls();
    assert!(
        !calls.is_empty(),
        "the scan found no fruit webhook calls at all; the pattern this test \
         relies on has changed and the test is now measuring nothing"
    );

    let offenders: Vec<String> = calls
        .iter()
        .filter(|(_, _, stmt)| {
            // A literal `None` anywhere in the argument list. Every fruit
            // notification takes a photo it could have supplied.
            stmt.contains("None")
        })
        .map(|(f, l, stmt)| format!("{f}:{l}  {}", &stmt[..stmt.len().min(110)]))
        .collect();

    assert!(
        offenders.is_empty(),
        "these fruit notifications are constructed with a literal `None`, \
         discarding the screenshot the caller had in hand:\n  {}\n\n\
         A stored-or-dropped fruit is the case worth having a picture of, and it \
         is the one where the name is on screen. Pass the photo.",
        offenders.join("\n  ")
    );

    // Sanity: the success path specifically must name a photo. If someone
    // refactors `store_fruit` and the `photo` binding moves below this call, the
    // rule above still passes on a different variable - this names the one that
    // must be there.
    let actions = source("src/bot/actions.rs");
    assert!(
        actions.contains("ctx.webhook.fruit_storage_outcome(catch, &outcome, photo)"),
        "the storage outcome notification must pass the captured photo."
    );
}

#[test]
fn the_store_screenshot_is_still_gated_by_the_existing_settings() {
    // Passing the photo through must not have widened what gets captured. The
    // gates live in `capture_drop_screenshot`; the webhook call sites apply the
    // same `legendary_only` condition. If either moves, a fruit notification
    // could start going out for rarities the operator asked to be filtered.
    let actions = source("src/bot/actions.rs");
    let cap = actions
        .split("pub fn capture_drop_screenshot")
        .nth(1)
        .map(|s| s.split('\n').take(10).collect::<Vec<_>>().join("\n"))
        .expect("capture_drop_screenshot must exist");

    assert!(
        cap.contains("!s.webhook.send_screenshot") || cap.contains("s.webhook.send_screenshot"),
        "the drop/store screenshot must remain gated on send_screenshot: {cap}"
    );
    assert!(
        cap.contains("send_drop_screenshot"),
        "the drop/store screenshot must remain gated on send_drop_screenshot: {cap}"
    );
    assert!(
        cap.contains("legendary_only"),
        "the drop/store screenshot must remain gated on legendary_only: {cap}"
    );
}
