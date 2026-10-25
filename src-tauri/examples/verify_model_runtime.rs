//! Read-only verification of deployed models on an explicit image fixture.
//! Usage: cargo run --example verify_model_runtime -- MODELS CASES_JSON OUTPUT_JSON
//! Cases contain model, image_path and label. Never sends gameplay input.
use gpo_autofish_lib::core::shadow_infer::{preprocess_rgba, ShadowEngine};
use std::{path::Path, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err("expected MODELS CASES_JSON OUTPUT_JSON".into());
    }
    let engine = ShadowEngine::load(Path::new(&args[0]))?;
    let cases: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        let name = case["model"].as_str().ok_or("missing model")?;
        let model = engine.models.get(name).ok_or("model unavailable")?;
        let image =
            image::open(case["image_path"].as_str().ok_or("missing image_path")?)?.to_rgba8();
        let started = Instant::now();
        let pixels = preprocess_rgba(
            image.as_raw(),
            image.width() as usize,
            image.height() as usize,
            &model.spec,
        )
        .ok_or("preprocessing failed")?;
        let probabilities = engine
            .infer_probs(name, &pixels)
            .ok_or("inference failed")?;
        if probabilities.iter().any(|(_, p)| !p.is_finite()) {
            return Err("non-finite prediction".into());
        }
        let (prediction, confidence) = probabilities
            .iter()
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .ok_or("empty prediction")?;
        results.push(serde_json::json!({
            "model": name, "revision": model.identity_revision(),
            "image_path": case["image_path"], "label": case["label"],
            "prediction": prediction, "confidence": confidence,
            "latency_ms": started.elapsed().as_secs_f64() * 1000.0,
        }));
    }
    use std::io::Write;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[2])?;
    serde_json::to_writer_pretty(&mut output, &results)?;
    output.flush()?;
    println!(
        "Verified {} images with {:?}",
        results.len(),
        engine.model_names()
    );
    Ok(())
}
