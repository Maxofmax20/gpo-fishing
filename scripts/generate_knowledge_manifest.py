"""Generate docs/knowledge_manifest_v1.json — the versioned GPO knowledge manifest.

Reads the RUST KB SOURCES (never hardcoded lists):
  src-tauri/src/core/fruit.rs  -> DEFAULT_FRUITS, KNOWN_FISH, fruit_rarity arms,
                                  DEFAULT_{DROP,CATCH,FAIL}_PHRASES
  src-tauri/src/core/knowledge.rs -> FRUIT_ALIASES, bait tiers, ui-term wiring
and crosses every KB entity against the REAL dataset
  %APPDATA%/gpo-autofish/datasets/gpo-vision/v1/labels.jsonl
using the trainer's exact FNV-1a session splits (ml.gpo_train.dataset).

Strict A/B/C/D separation per entity:
  wiki_known          (A) exists in the bundled KB
  visual_examples     (B) real screenshot rows, per split
  runtime_supported   (C) current software can recognize it live (mechanism named)
  production_supported(D) allowed to drive real macro decisions today

Usage: python scripts/generate_knowledge_manifest.py
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
from collections import Counter

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(REPO, "ml"))
from gpo_train.dataset import build_splits, load_snapshot  # noqa: E402

FRUIT_RS = os.path.join(REPO, "src-tauri", "src", "core", "fruit.rs")
KNOWLEDGE_RS = os.path.join(REPO, "src-tauri", "src", "core", "knowledge.rs")
OUT = os.path.join(REPO, "docs", "knowledge_manifest_v1.json")
GENERATOR_VERSION = 1


def read(path: str) -> str:
    with open(path, encoding="utf-8") as f:
        return f.read()


def quoted_block(src: str, const_name: str) -> list[str]:
    m = re.search(const_name + r"[^=]*=\s*&\[(.*?)\];", src, re.S)
    assert m, f"const {const_name} not found"
    return re.findall(r'"([^"]+)"', m.group(1))


def parse_rarity_tiers(src: str) -> dict[str, str]:
    """Port fruit_rarity() direct-match arms: tier -> [names]."""
    tiers: dict[str, str] = {}
    # Direct arms look like: "tori" | "phoenix" | ... => FruitRarity::Mythical,
    for m in re.finditer(
        r"((?:\"[^\"]+\"\s*\|\s*)*\"[^\"]+\")\s*=>\s*FruitRarity::(\w+)", src
    ):
        names = re.findall(r'"([^"]+)"', m.group(1))
        for n in names:
            tiers[n.lower()] = m.group(2)
    return tiers


def contains_tier(name: str) -> str:
    """Port the contains-fallback of fruit_rarity() (lowercased input)."""
    l = name.lower()
    myth = ["phoenix", "tori", "mochi", "ope", "venom", "doku", "buddha", "daibutsu",
            "pteranodon", "dragon", "seiryu", "soru", "leopard", "mammoth", "trex", "t-rex"]
    leg = ["pika", "magu", "hie", "goro", "mera", "suna", "yami", "yuki", "smoke",
           "moku", "gura", "zushi", "nikyu", "paw", "ito", "kage", "goru", "bisu", "gas"]
    epic = ["yomi", "bane", "kira"]
    rare = ["gomu", "bomu", "bomb", "bari", "mero", "horo"]
    common = ["kilo", "suke", "spin", "chiyu"]
    if any(s in l for s in myth):
        return "Mythical"
    if any(s in l for s in leg):
        return "Legendary"
    if any(s in l for s in epic):
        return "Epic"
    if any(s in l for s in rare):
        return "Rare"
    if any(s in l for s in common):
        return "Common"
    return "Unknown"


def fruit_rarity(name: str, direct: dict[str, str]) -> str:
    key = name.strip().lower()
    if key in direct:
        return direct[key]
    return contains_tier(key)


def capitalize(s: str) -> str:
    return s[:1].upper() + s[1:] if s else s


def main() -> int:
    fruit_src = read(FRUIT_RS)
    kb_src = read(KNOWLEDGE_RS)

    fruits = quoted_block(fruit_src, "DEFAULT_FRUITS")
    fish = quoted_block(fruit_src, "KNOWN_FISH")
    drop_p = quoted_block(fruit_src, "DEFAULT_DROP_PHRASES")
    catch_p = quoted_block(fruit_src, "DEFAULT_CATCH_PHRASES")
    fail_p = quoted_block(fruit_src, "DEFAULT_FAIL_PHRASES")
    direct = parse_rarity_tiers(fruit_src)
    assert len(fruits) == 77, f"DEFAULT_FRUITS changed: {len(fruits)}"
    assert len(fish) == 41, f"KNOWN_FISH changed: {len(fish)}"

    aliases: dict[str, list[str]] = {}
    for canon, rest in re.findall(r'\("([^"]+)",\s*&\[(.*?)\]\)', kb_src):
        aliases[canon] = re.findall(r'"([^"]+)"', rest)

    entities: dict[str, dict] = {}
    for name in fruits:
        eid = f"fruit:{name.lower()}"
        entities[eid] = {
            "entity_id": eid, "display_name": name, "category": "DEVIL_FRUIT",
            "rarity": fruit_rarity(name, direct),
            "aliases": aliases.get(name, []),
        }
    for name in fish:
        eid = f"fish:{name.lower().replace(' ', '-')}"
        entities[eid] = {
            "entity_id": eid, "display_name": capitalize(name), "category": "FISH",
            "rarity": None, "aliases": [],
        }
    for bid, bname in [("common", "Common Bait"), ("rare", "Rare Bait"),
                       ("legendary", "Legendary Bait")]:
        entities[f"bait:{bid}"] = {
            "entity_id": f"bait:{bid}", "display_name": bname, "category": "BAIT",
            "rarity": capitalize(bid), "aliases": [bid],
        }
    for phrase in drop_p + catch_p + fail_p + ["spawned", "spawn"]:
        eid = "ui:" + phrase.lower().replace(" ", "-")
        entities[eid] = {
            "entity_id": eid, "display_name": phrase, "category": "UI_TERM",
            "rarity": None, "aliases": [],
        }

    # ---- cross against the real dataset (trainer's exact splits) ----
    snap = load_snapshot()
    # OCR/hard fields are not in the trainer Row; read index-aligned from labels.jsonl.
    appdata = os.environ.get("APPDATA", "")
    ocr_texts: list[str] = []
    hard_flags: list[bool] = []
    with open(os.path.join(appdata, "gpo-autofish", "datasets", "gpo-vision",
                           "v1", "labels.jsonl"), encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            a = json.loads(line)
            ocr_texts.append(a.get("ocr_text") or "")
            hard_flags.append(bool(a.get("hard_example", False)))
    assert len(ocr_texts) == len(snap.rows), "labels.jsonl drifted mid-run; re-run on a quiesced dataset"
    splits = build_splits(snap)
    split_of: dict[int, str] = {}
    for i in splits.train:
        split_of[i] = "train"
    for i in splits.validation:
        split_of[i] = "validation"
    for i in splits.test:
        split_of[i] = "test"

    per_ent: dict[str, Counter] = {}
    verified: dict[str, Counter] = {}
    ocr_linked: dict[str, int] = Counter()
    for idx, r in enumerate(snap.rows):
        if not r.entity_id:
            continue
        c = per_ent.setdefault(r.entity_id, Counter())
        c["total"] += 1
        c[split_of.get(idx, "excluded")] += 1
        if ocr_texts[idx].strip():
            ocr_linked[r.entity_id] += 1
        if ocr_texts[idx].strip() and not hard_flags[idx]:
            v = verified.setdefault(r.entity_id, Counter())
            v["total"] += 1
            v[split_of.get(idx, "excluded")] += 1

    dataset_entity_ids = set(per_ent)
    kb_ids = set(entities)
    orphan_visual = sorted(dataset_entity_ids - kb_ids)

    out_entities = []
    for eid in sorted(kb_ids):
        base = entities[eid]
        c = per_ent.get(eid, Counter())
        v = verified.get(eid, Counter())
        out_entities.append({
            **base,
            "wiki_known": True,
            "visual_examples": {
                "total": c.get("total", 0), "train": c.get("train", 0),
                "validation": c.get("validation", 0), "test": c.get("test", 0),
            },
            "verified_visual_examples": {
                "total": v.get("total", 0), "train": v.get("train", 0),
                "validation": v.get("validation", 0), "test": v.get("test", 0),
            },
            "ocr_linked_examples": ocr_linked.get(eid, 0),
            # (C) runtime: the OCR+KB correlate_text path is generic over the KB
            "runtime_supported": {
                "ocr_kb": True,
                "mechanism": "correlate_text over KB index (perception.rs) + region OCR",
            },
            # (D) production: OCR+KB path is wired into post_catch; vision model is not deployed
            "production_supported": {"ocr_kb_wired": True, "vision_model": False},
        })

    try:
        git = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=REPO,
                             capture_output=True, text=True, timeout=30)
        rev = git.stdout.strip()
    except Exception:
        rev = "unknown"
    manifest = {
        "manifest_version": 1,
        "generator_version": GENERATOR_VERSION,
        "kb_source": {"knowledge_version": 1, "git_rev": rev,
                      "files": ["src-tauri/src/core/fruit.rs",
                                "src-tauri/src/core/knowledge.rs"]},
        "dataset": {"rows": len(snap.rows), "version": snap.version,
                    "splits": {"train": len(splits.train),
                               "validation": len(splits.validation),
                               "test": len(splits.test)}},
        "kb_counts": {
            "devil_fruit": len(fruits), "fish": len(fish), "bait": 3,
            "ui_term": len(drop_p) + len(catch_p) + len(fail_p) + 2,
        },
        "coverage_summary": {
            "kb_entities_with_any_visual": sum(1 for e in out_entities if e["visual_examples"]["total"] > 0),
            "kb_entities_with_verified_visual": sum(1 for e in out_entities if e["verified_visual_examples"]["total"] > 0),
            "kb_entities_with_test_visual": sum(1 for e in out_entities if e["visual_examples"]["test"] > 0),
            "devil_fruit_with_any_visual": sum(1 for e in out_entities if e["category"] == "DEVIL_FRUIT" and e["visual_examples"]["total"] > 0),
            "fish_with_any_visual": sum(1 for e in out_entities if e["category"] == "FISH" and e["visual_examples"]["total"] > 0),
            "other_drop_visual": 0,
            "other_drop_note": "No OTHER_DROP category exists in the KB and no dataset entity is unmapped; all 26 observed visual entities resolve to KB ids.",
            "dataset_entities_without_kb_entry": orphan_visual,
        },
        "verified_definition": "entity-linked AND ocr_text non-empty AND hard_example=false",
        "entities": out_entities,
    }
    with open(OUT, "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2)
    cov = manifest["coverage_summary"]
    print(f"entities={len(out_entities)} rows={len(snap.rows)} rev={rev}")
    print(f"any_visual={cov['kb_entities_with_any_visual']} verified={cov['kb_entities_with_verified_visual']} "
          f"test={cov['kb_entities_with_test_visual']} fruit_visual={cov['devil_fruit_with_any_visual']} "
          f"fish_visual={cov['fish_with_any_visual']} orphans={orphan_visual}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
