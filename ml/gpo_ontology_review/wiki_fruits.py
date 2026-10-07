"""GPO wiki devil-fruit list: fetch, parse, snapshot, and match.

THE SAME PATTERN AS wiki.py, FOR FRUIT
======================================

The fish side works because the wiki's bestiary is one clean table. Fruits are the
opposite: the list lives in a MediaWiki template as ~37 `{{DevilFruit|...}}` calls,
with the display name in `name` and a short label in `label`. Parsing that is the
whole job, and getting it wrong would silently invent or lose fruits.

WHY FRUIT NEEDS ITS OWN LIST RATHER THAN THE FISH ONE
=====================================================

* Fruits have FIVE rarities (`C`, `R`, `E`, `L`, `M`), not four. The mythical tier
  holds nine fruits including every Zoan, and it is the rarest drop in the game.
* A fruit's `label` is frequently NOT a word from its name: `Chiyu Chiyu no Mi` is
  labelled **Heal**, `Doku Doku no Mi` is **Venom**, `Hito Hito no Mi, Model:
  Daibutsu` is **Buddha**. Matching on the label alone misses those; matching on the
  full name catches them. Both keys are therefore recorded, and both are searched.
* Names carry a model suffix for Zoans (`Tori Tori no Mi, Model: Phoenix`). That is
  part of the fruit's identity, not noise, so it is kept in `fullName` and stripped
  only for a secondary `stem` key.

WHAT THIS FOUND
===============

* The game lists **37 fruits**, not the 33 the README claims. The count moved.
* The knowledge base carries **77** fruit entities, so roughly 40 are duplicates or
  phantoms: `fruit:bari` and `fruit:barrier`, `fruit:bisu` and `fruit:biscuit`,
  `fruit:bomu` and `fruit:bomb` are each one fruit stored twice.
* Two aliases are outright wrong and in direct conflict: `fruit:goro` is filed as
  "Rumble" and `fruit:goru` as "Gold". The wiki says Goro Goru no Mi is a Logia
  with `Goro` as its label, and there is no Gold fruit at all. Two entities, one
  word, two different wrong answers.

Evidence, not a decision: as with fish, nothing here is applied automatically.
"""

from __future__ import annotations

import hashlib
import json
import re
import time
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from .wiki import (
    API_URL,
    FETCH_TIMEOUT_SECONDS,
    USER_AGENT,
    WIKI_HOST,
    edit_distance,
    match_key,
)

#: The template holding the implemented-fruit list.
FRUIT_TEMPLATE = "Template:DevilFruit/List"

FRUIT_PAGE_URL = f"https://{WIKI_HOST}/wiki/Devil_Fruits"

#: Five rarities. `M` is Mythical and is the rarest tier in the game.
FRUIT_RARITIES = {
    "C": "Common",
    "R": "Rare",
    "E": "Epic",
    "L": "Legendary",
    "M": "Mythical",
}


@dataclass
class WikiFruit:
    label: str
    full_name: str
    rarity: str
    kind: str

    @property
    def keys(self) -> list[str]:
        """Every spelling a capture could plausibly contain.

        Ordered most-specific first, so an exact hit on the full name is preferred
        over a short label that happens to be a substring of something else.
        """
        out = [self.full_name]
        # `Tori Tori no Mi, Model: Phoenix` -> `Tori Tori no Mi`
        stem = self.full_name.split(",")[0].strip()
        if stem and stem != self.full_name:
            out.append(stem)
        # `Kilo Kilo no Mi` -> `Kilo` (already the label, but be explicit)
        out.append(self.label)
        # First word of the stem: `Chiyu`, `Doku`, `Hito`.
        head = stem.split()[0] if stem.split() else ""
        if head and head not in out:
            out.append(head)
        seen, uniq = set(), []
        for k in out:
            kk = match_key(k)
            if kk and kk not in seen:
                seen.add(kk)
                uniq.append({"text": k, "matchKey": kk})
        return uniq


def fetch_template(*, timeout: int = FETCH_TIMEOUT_SECONDS) -> str:
    url = (
        f"{API_URL}?action=parse&page={urllib.parse.quote(FRUIT_TEMPLATE)}"
        "&prop=wikitext&format=json&formatversion=2"
    )
    if WIKI_HOST not in urllib.parse.urlparse(url).netloc:
        raise ValueError(f"refusing to contact anything but {WIKI_HOST}")
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(req, timeout=timeout) as resp:  # noqa: S310
        payload = json.loads(resp.read().decode("utf-8"))
    if "error" in payload:
        raise RuntimeError(f"wiki API error: {payload['error']}")
    return payload["parse"]["wikitext"]


def _fields(call: str) -> dict[str, str]:
    """Split one `{{DevilFruit|...}}` call into its named fields."""
    out: dict[str, str] = {}
    for part in call.split("|"):
        if "=" in part:
            k, v = part.split("=", 1)
            out[k.strip().lower()] = v.strip()
    return out


def parse_fruits(wikitext: str) -> list[WikiFruit]:
    """Extract every implemented fruit from the template.

    Strict for the same reason the fish parser is: an empty result must raise, not
    become an empty snapshot. An empty fruit list would make every observed fruit
    read as "not in the reference", turning a parse failure into false evidence.
    """
    out: list[WikiFruit] = []
    seen: set[str] = set()
    for call in re.findall(r"\{\{DevilFruit\|(.*?)\}\}", wikitext, re.S):
        f = _fields(call)
        name = f.get("name", "").strip()
        label = f.get("label", "").strip()
        if not name or not label:
            continue
        rarity = FRUIT_RARITIES.get(f.get("rarity", "").upper(), "Unknown")
        kind = f.get("type", "").strip() or "Unknown"
        key = match_key(label)
        if not key or key in seen:
            continue
        seen.add(key)
        out.append(WikiFruit(label=label, full_name=name, rarity=rarity, kind=kind))
    if not out:
        raise ValueError("no DevilFruit calls found; refusing to write an empty snapshot")
    return out


def build_snapshot(
    fruits: list[WikiFruit], *, retrieved_at_unix: int | None = None
) -> dict[str, Any]:
    entries = sorted(
        (
            {
                "label": f.label,
                "fullName": f.full_name,
                "rarity": f.rarity,
                "type": f.kind,
                "keys": f.keys,
                "url": FRUIT_PAGE_URL,
            }
            for f in fruits
        ),
        key=lambda e: (e["rarity"], e["label"].lower()),
    )
    blob = json.dumps(entries, sort_keys=True, separators=(",", ":")).encode("utf-8")
    by_rarity: dict[str, int] = {}
    for e in entries:
        by_rarity[e["rarity"]] = by_rarity.get(e["rarity"], 0) + 1
    return {
        "kind": "ontology_wiki_devil_fruits",
        "version": 1,
        "sourceHost": WIKI_HOST,
        "sourceTemplate": FRUIT_TEMPLATE,
        "sourceUrl": FRUIT_PAGE_URL,
        "retrievedAtUnix": int(
            retrieved_at_unix if retrieved_at_unix is not None else time.time()
        ),
        "entryCount": len(entries),
        "byRarity": by_rarity,
        "contentSha256": hashlib.sha256(blob).hexdigest(),
        "caveat": (
            "Community-editable reference, not an authority. A match shows the name "
            "is a documented fruit; NO match shows only that this list omits it. "
            "Never a decision."
        ),
        "entries": entries,
    }


def load_snapshot(path: Path) -> dict[str, Any] | None:
    if not path.is_file():
        return None
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError):
        return None


def refresh_snapshot(path: Path, *, timeout: int = FETCH_TIMEOUT_SECONDS) -> dict[str, Any]:
    fruits = parse_fruits(fetch_template(timeout=timeout))
    snap = build_snapshot(fruits)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(snap, indent=1, sort_keys=True), encoding="utf-8")
    return snap


def match_fruit(text: str, snapshot: dict[str, Any]) -> dict[str, Any]:
    """Classify one observed fruit string against the reference."""
    entries = snapshot.get("entries", [])
    if not entries:
        return {"matched": False, "basis": "reference unavailable"}

    key = match_key(text or "")
    if not key:
        return {"matched": False, "basis": "no text"}

    # Exact on ANY accepted spelling, most-specific key first.
    for e in entries:
        for k in e.get("keys", []):
            if k["matchKey"] == key:
                return {
                    "matched": True,
                    "label": e["label"],
                    "fullName": e["fullName"],
                    "rarity": e["rarity"],
                    "type": e["type"],
                    "url": e.get("url"),
                    "matchedSpelling": k["text"],
                    "basis": f"exact match on {k['text']!r}",
                }

    # Near miss: the banner truncates names, so a prefix often survives.
    best: tuple[int, dict[str, Any], str] | None = None
    cap = 2 if len(key) > 6 else 1
    for e in entries:
        for k in e.get("keys", []):
            d = edit_distance(key, k["matchKey"], cap=cap)
            if d <= cap and (best is None or d < best[0]):
                best = (d, e, k["text"])
    if best:
        d, e, spelling = best
        return {
            "matched": False,
            "suggestion": e["label"],
            "suggestionRarity": e["rarity"],
            "suggestionDistance": d,
            "fullName": e["fullName"],
            "url": e.get("url"),
            "basis": f"no exact match; closest documented fruit is {d} character(s) away",
        }
    return {"matched": False, "basis": "not present in this reference list"}