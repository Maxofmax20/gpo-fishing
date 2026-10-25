//! # Read-only knowledge-base exporter
//!
//! The OCR audit pipeline (`ml/gpo_ocr_audit/`) needs the REAL knowledge base:
//! canonical names, aliases, OCR aliases and categories. That knowledge lives in
//! Rust code (`core::knowledge::build_bundled`), so a Python-only copy of it
//! would be a second ontology that can silently drift — exactly the class of
//! bug `FISH_VOCAB.txt` exists to prevent.
//!
//! So this exporter serialises the live bundled KB to JSON. It is an **example**
//! target, not a binary and not a library function, which means:
//!
//! * it cannot be reached from the app at runtime,
//! * it is not compiled into the shipped executable,
//! * it holds no authority over anything; it only reads.
//!
//! Usage:
//!
//! ```text
//! cargo run --example kb_export -- <output.json>
//! ```
//!
//! The emitted file carries its own SHA-256 payload so a consumer can prove it
//! audited the same KB it read. Nothing here mutates the KB, the vocabulary, the
//! registry, or any dataset row.

use gpo_autofish_lib::core::class_vocab;
use gpo_autofish_lib::core::knowledge::KnowledgeBase;
use std::io::Write;

#[derive(serde::Serialize)]
struct EntityOut {
    id: String,
    canonical_name: String,
    aliases: Vec<String>,
    ocr_aliases: Vec<String>,
    category: String,
    rarity: Option<String>,
}

#[derive(serde::Serialize)]
struct Export {
    /// Format version, so a consumer can refuse a shape it does not know.
    schema: u32,
    kb_version: u32,
    /// The canonical fish class list, copied from the single source of truth so
    /// the consumer never has to re-derive or re-parse it.
    fish_vocab: Vec<String>,
    fish_vocab_sha256: String,
    entity_count: usize,
    entities: Vec<EntityOut>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `--effective` includes the persisted curator overlay (knowledge_custom.json),
    // so the consumer sees the SAME KB the app runs with. Default stays the
    // compiled bundled base, which is what the OCR-audit contract test pins.
    let effective = args.iter().any(|a| a == "--effective");
    let out_path = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "kb_export.json".to_string());

    let kb = if effective {
        gpo_autofish_lib::config::Store::new(gpo_autofish_lib::app::data_dir())
            .effective_knowledge()
    } else {
        KnowledgeBase::bundled().clone()
    };

    // Sort by id so the export is byte-stable across runs. The audit's
    // determinism guarantee depends on this.
    let mut entities: Vec<EntityOut> = kb
        .entities()
        .iter()
        .map(|e| EntityOut {
            id: e.id.clone(),
            canonical_name: e.canonical_name.clone(),
            aliases: e.aliases.clone(),
            ocr_aliases: e.ocr_aliases.clone(),
            category: e.category.as_str().to_string(),
            rarity: e.rarity.clone(),
        })
        .collect();
    entities.sort_by(|a, b| a.id.cmp(&b.id));

    let export = Export {
        schema: 1,
        kb_version: kb.version,
        fish_vocab: class_vocab::fish_vocab()
            .iter()
            .map(|c| c.to_string())
            .collect(),
        fish_vocab_sha256: class_vocab::fish_vocab_sha().to_string(),
        entity_count: entities.len(),
        entities,
    };

    // Pretty-printed so a human reviewing the artefact can read it, and written
    // with a trailing newline so it is a well-behaved text file.
    let mut json = serde_json::to_string_pretty(&export)?;
    json.push('\n');

    let mut f = std::fs::File::create(&out_path)?;
    f.write_all(json.as_bytes())?;
    f.flush()?;

    // The checksum is printed as well as embedded, so CI can compare against the
    // file it just wrote rather than trusting the writer.
    let digest = gpo_autofish_lib::core::ml_model::sha256_hex(json.as_bytes());
    println!(
        "exported {} entities (kb_version={}) to {} sha256={}",
        export.entity_count, export.kb_version, out_path, digest
    );
    Ok(())
}
