//! A fruit notification must not discard the screenshot it already took.
//!
//! # The defect this pins
//!
//! `store_fruit` captures `photo` at the end of its banner loop and hands it to
//! every `fruit_storage_failed` call - six of them, covering duplicate, dropped,
//! protected, failed and unconfirmed outcomes. Exactly one call site passed a
//! hardcoded `None`: the success branch, `fruit_stored(fruit_name, None, ...)`.
//!
//! The visible effect was that the two outcomes you most want a picture of -
//! the fruit was STORED, and the fruit was DROPPED - were the only ones reported
//! as bare text, at the moment the fruit's name is on screen and the frame is
//! already in hand. The catch-time screenshot worked throughout, which is why
//! the gap looked like a settings problem rather than a lost argument.
//!
//! # Why this is asserted structurally
//!
//! The alternative is a behavioural test that drives `store_fruit` through a
//! mocked capture backend and asserts on the outbound webhook payload. That
//! harness does not exist, and building one to check a single argument is a poor
//! trade against a rule that is decidable from the source: **no fruit
//! notification may be constructed with a literal `None` where a photo was
//! available.**
//!
//! This is deliberately a weak check - it can be fooled by rebinding - and it is
//! stated as such rather than dressed up as a behavioural guarantee. It fails
//! loudly and names the call site, which is the failure that actually happened.
//!
//! What the change did NOT do is worth stating too, because it is the part that
//! could have been wrong: it did not add a capture. `photo` is bound before this
//! point on every path, and `capture_drop_screenshot` applies the same
//! `send_screenshot` / `send_drop_screenshot` / `legendary_only` gates that the
//! webhook call itself applies, so no new frame is taken and no new setting is
//! bypassed. This test also checks that the gating is still in place.
//!
//! Read-only: no store, no files, no network.

use std::path::PathBuf;

fn source(rel: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("{} unreadable: {e}", p.display()))
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
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
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
                let flat: String =
                    extent.join(" ").split_whitespace().collect::<Vec<_>>().join(" ");
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
        actions.contains("ctx.webhook.fruit_stored(fruit_name, photo, custom_tg)"),
        "the stored notification must pass the captured `photo`, not a literal \
         `None`. Re-check actions.rs: the call was `fruit_stored(fruit_name, \
         None, custom_tg)`."
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