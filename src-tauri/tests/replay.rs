use std::path::PathBuf;

use gpo_autofish_lib::core::types::Frame;
use gpo_autofish_lib::core::vision::{self, Palette};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/reels")
}

fn fixtures() -> Vec<PathBuf> {
    let dir = fixtures_dir();
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "png") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn load(p: &PathBuf) -> Frame {
    let img = image::open(p).expect("png").to_rgba8();
    let (w, h) = img.dimensions();
    Frame::new(w as usize, h as usize, img.into_raw())
}

#[test]
fn replay_dumped_frames() {
    // A missing directory is a repo layout bug (fail); an empty directory is
    // an explicit coverage gap: real gameplay captures are not checked in,
    // so replay coverage is intentionally zero and detection is covered by
    // the vision unit tests instead. Never commit synthetic filler PNGs here.
    assert!(
        fixtures_dir().is_dir(),
        "fixtures dir missing: {}",
        fixtures_dir().display()
    );
    let files = fixtures();
    if files.is_empty() {
        eprintln!("WARNING: tests/fixtures/reels has no PNG fixtures — replay coverage is 0 frames by design (no real captures available). This pass is NOT detection coverage.");
        return;
    }
    let p = Palette::default();
    let mut seen = 0;
    let mut failed = Vec::new();
    for f in &files {
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        let frame = load(f);
        let r = vision::read(&frame, &p);
        let expect_bar = name.contains("bite") || name.contains("frame");
        if expect_bar {
            seen += 1;
            if r.is_none() {
                failed.push(name);
            }
        } else if name.contains("end") {
            assert!(vision::find_bar(&frame, &p).is_none(), "{name}: bar still detected after reel end");
        }
    }
    eprintln!("replayed {} frames, {} expected-bar frames, {} failed", files.len(), seen, failed.len());
    let allowed = (seen as f32 * 0.02).ceil() as usize;
    assert!(failed.len() <= allowed, "detection failed on: {failed:?}");
}
