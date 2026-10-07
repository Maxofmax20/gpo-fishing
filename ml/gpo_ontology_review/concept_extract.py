"""Turn raw OCR strings into reviewable CONCEPTS.

WHY THIS MODULE EXISTS
======================

The audit found 411 distinct raw OCR strings. Those are not 411 game names. They
are roughly sixteen game names observed through a lossy channel, plus:

* banner-template residue glued onto the front (``ne item``, ``nevitem``,
  ``newitemskeletalshark``, ``sgolden tigerfin``)
* the same name rendered with and without spaces (``crimsonsnapper`` vs
  ``crimson snapper``)
* a misread bracket glyph left as a trailing letter (``tigerfinz``)

A human asked to rule on 411 rows would rule on the same decision sixteen times
and give up. So grouping is done first - and it is done in a way that cannot
invent anything.

HOW GROUPING IS JUSTIFIED (and what it deliberately does NOT do)
================================================================

**It merges renderings, not names.** Two strings merge only when they are
identical after removing spaces and punctuation:

    "crimson snapper"  ->  crimsonsnapper
    "crimsonsnapper"   ->  crimsonsnapper     (same group)

That is evidence that the OCR sometimes drops a space. It is NOT evidence that
the two strings mean different things.

**It never merges different names.**

    "tigerfin"          ->  tigerfin
    "exotic tigerfin"   ->  exotictigerfin    (different group)
    "golden tigerfin"   ->  goldentigerfin    (different group)

All three are reported as separate concepts, and all three share a FAMILY. Whether
that family is one entity, three entities, or a base fish plus modifiers is left
open on purpose.

**It never invents a name.** Stripping the banner template removes words the KB
already models as ``ui_term`` entities. Trailing ``z`` is removed because ``<``
and ``>`` are misread as ``z`` in this font - a named rendering artifact, not a
guess. Neither operation can promote an unfamiliar word into a familiar one, and
``tests/`` asserts that the excluded concepts survive intact.

FAMILIES, MODIFIERS, BASE NAMES
===============================

A family is named by the concept's terminal word. Modifiers are the leading
words. Both are stored as **linguistic evidence only** - the module never claims
that ``golden`` is a colour rather than a species, because the whole point of the
review is that nobody currently knows.
"""

from __future__ import annotations

import json
import re
import unicodedata
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable

from .schemas import (
    OntologyConcept,
    OntologyStatus,
    ReviewPriority,
    concept_id_for,
)

# --------------------------------------------------------------------------
# Banner-template residue
# --------------------------------------------------------------------------

#: Fragments the drop banner leaves on the front of a name when the template box
#: is clipped or partly recognised.
#:
#: Derived from the KB's own ``ui_term`` entities ("new item") plus the renderings
#: actually observed in this dataset. All of these are template words, NOT entity
#: names, so removing them cannot remove information - it recovers the name the
#: banner was wrapping.
BANNER_RESIDUE = (
    "ne item", "nev item", "new item", "nevitem", "newitem", "ne items",
    "new items", "nevitems", "newitems", "ne itern", "new itern",
    "newitem", "newitern", "nev itern",
)

#: A DELIBERATELY ABSENT rule, kept as a record of a rejected idea.
#:
#: Observed renderings sometimes carry a stray leading character off the banner
#: border (`sgolden tigerfin`, `scrimson polka puffer`). Stripping it looks
#: attractive and is catastrophic: a heuristic of "drop a leading character
#: followed by three or more letters" matches EVERY name, and in testing it turned
#: `zebra ribbon angelfish` into `ebra ribbon angelfish`, `crimson snapper` into
#: `rimson snapper` and `swordfish` into `wordfish`.
#:
#: The prefixes it would remove affect a handful of rows; the damage it does
#: affects the concept list the human actually rules on. The few `s`-prefixed
#: variants therefore remain separate concepts, which is the honest outcome - they
#: are visibly odd, and a human can see that immediately.

#: A trailing ``z`` is a misread ``<`` or ``>``: the game's banner wraps the name
#: in angle brackets and this font renders them so the OCR reads ``z``
#: (``tigerfinz``, ``pufferz``, ``zebra ribbon angelfishz``).
#:
#: Restricted to MULTI-TOKEN names. A blanket rule turns ``quiz`` into ``qui`` and
#: ``fizz`` into ``fiz``, which corrupts real names - the same class of damage as
#: the rejected leading-junk rule. A multi-token name cannot be ``quiz``, so the
#: rule is safe there, and most of the affected rows
#: (``crimson polka pufferz``, ``zebra ribbon angelfishz``) are multi-token anyway.
#: Single-token cases such as ``tigerfinz`` therefore stay separate concepts, which
#: is the honest outcome and is consistent with how stray prefixes are handled.
TRAILING_BRACKET_JUNK = re.compile(r"z$")
MIN_TOKENS_FOR_BRACKET_REPAIR = 2

#: Characters that carry no naming information once a name has been read.
_TRIM_CHARS = " \t\r\n.,;:!|'\"`*_"


def _basic(text: str) -> str:
    """Lowercase, NFKC, collapse punctuation to spaces."""
    folded = unicodedata.normalize("NFKC", text).lower()
    out = []
    for ch in folded:
        out.append(" " if not ch.isalnum() else ch)
    return " ".join("".join(out).split())


def group_key(normalized: str) -> str:
    """Space- and punctuation-insensitive grouping key.

    Two raw strings that differ only in spacing or punctuation produce the SAME
    key, which is how ``crimsonsnapper`` and ``crimson snapper`` end up in one
    concept. Two different names never collide, because the key preserves every
    letter and its order.
    """
    return "".join(ch for ch in normalized if ch.isalnum())


def strip_banner_residue(normalized: str) -> tuple[str, list[str]]:
    """Remove banner-template words from the front.

    Returns ``(name, removed_fragments)``. The removed fragments are returned, not
    discarded, so the evidence record shows what was taken off.
    """
    removed: list[str] = []
    current = normalized
    changed = True
    while changed:
        changed = False
        for frag in BANNER_RESIDUE:
            if current.startswith(frag):
                current = current[len(frag):].strip()
                removed.append(frag)
                changed = True
                break
    return current, removed


#: Longest observed drop name is 4 tokens (`crimson polka puffer`,
#: `golden ribbon angelfish`, `zebra ribbon angelfish`, `dark skeletal shark`).
#: Anything longer is announcement text - the Seeing Eye bait banner, the trading
#: hub spawn notices, the fruit-despawn timers.
#:
#: Length is the discriminator because the OCR mangles the wording badly enough
#: that phrase matching against the KB's ``ui_term`` entries misses nearly
#: everything: `eeing eye you drop checkyourbackpa pity 3 40 legendary` shares no
#: whole phrase with `check your backpack`.
#:
#: Excluding these loses nothing: every real name also occurs in clean form
#: (`crimson snapper` 245 times on its own), so the clean observation is always
#: present. The banner-wrapped variants are still recorded, in
#: ``system_text.json``, so nothing is quietly discarded.
MAX_NAME_TOKENS = 5


def looks_like_ui(text: str, ui_phrases: Iterable[str]) -> bool:
    """Is this banner text rather than an entity name?

    Used to keep UI noise out of the concept dataset. The bait-drop banner
    (``legendary bait pity 3 40 check your backpack``) is real text the game
    renders, but it is not an entity name and must not become a concept.
    """
    if len(text.split()) > MAX_NAME_TOKENS:
        return True

    # Secondary signal: two or more KB banner phrases present as whole phrases.
    t = f" {text} "
    hits = 0
    for phrase in ui_phrases:
        p = _basic(phrase)
        if p and f" {p} " in t:
            hits += 1
    return hits >= 2


def extract_concept_name(raw_residual: str, ui_phrases: Iterable[str]) -> tuple[str | None, dict[str, Any]]:
    """Reduce one raw OCR string to a reviewable concept name.

    Returns ``(concept_name_or_None, notes)``. ``None`` means the string is banner
    text, not an entity name.
    """
    notes: dict[str, Any] = {"banner_removed": [], "stripped_trailing_z": False,
                             "ui_noise": False}

    basic = _basic(raw_residual)
    if not basic:
        return None, notes

    if looks_like_ui(basic, ui_phrases):
        notes["ui_noise"] = True
        return None, notes

    current, removed = strip_banner_residue(basic)
    notes["banner_removed"] = removed

    if not current:
        return None, notes

    # Trailing bracket glyph, only when the name has more than one token.
    if (TRAILING_BRACKET_JUNK.search(current)
            and len(current.split()) >= MIN_TOKENS_FOR_BRACKET_REPAIR):
        current = TRAILING_BRACKET_JUNK.sub("", current)
        notes["stripped_trailing_z"] = True

    current = " ".join(current.split())
    if not current:
        return None, notes
    return current, notes


# --------------------------------------------------------------------------
# Decomposition: base name and modifiers
# --------------------------------------------------------------------------

#: Tokens that are grammar rather than naming. A name may end in one of these.
_TRAILING_FUNCTION_WORDS = {"of", "the", "a", "an"}


def decompose(concept_name: str) -> tuple[str | None, list[str]]:
    """Split ``crimson polka puffer`` into base ``puffer`` + modifiers.

    LINGUISTIC EVIDENCE ONLY.

    The base is the last non-function word. This is a textual heuristic and it is
    explicitly NOT a claim about the game. ``Crimson Polka Puffer`` may be one
    entity, ``polka`` may be a variant and ``crimson`` a colour, or the words may
    be unrelated - which is exactly what the human is being asked to decide.

    Returns ``(base_or_None, modifiers)``.
    """
    words = concept_name.split()
    if not words:
        return None, []
    if len(words) == 1:
        return words[0], []
    trimmed = list(words)
    while len(trimmed) > 1 and trimmed[-1] in _TRAILING_FUNCTION_WORDS:
        trimmed.pop()
    return trimmed[-1], trimmed[:-1]


def family_for(base: str | None, concept_name: str) -> str:
    """Family id for a concept, keyed on its base word.

    Falls back to the whole name when there is no base, so every concept has a
    family and no concept is orphaned.
    """
    stem = base or concept_name
    return f"family:{'-'.join(stem.split())}"


# --------------------------------------------------------------------------
# Priority
# --------------------------------------------------------------------------

def priority_for(occurrences: int, sessions: int, days: int) -> tuple[str, list[str]]:
    """Review-order band, computed from the data.

    ORDER ONLY. It cannot change a label; ``tests/`` asserts the scorer is not
    even given one.
    """
    reasons: list[str] = []
    if occurrences >= 100 and sessions >= 10 and days >= 3:
        reasons.append(f"{occurrences} occurrences across {sessions} sessions / {days} days")
        return ReviewPriority.P0.value, reasons
    if occurrences >= 25 and sessions >= 5:
        reasons.append(f"{occurrences} occurrences across {sessions} sessions")
        return ReviewPriority.P1.value, reasons
    if occurrences >= 5:
        reasons.append(f"{occurrences} occurrences")
        return ReviewPriority.P2.value, reasons
    reasons.append("singleton: one observation")
    return ReviewPriority.P3.value, reasons


# --------------------------------------------------------------------------
# Building concepts from audit evidence
# --------------------------------------------------------------------------

@dataclass
class _Bucket:
    key: str
    names: set[str]
    rows: list[dict[str, Any]]


def build_concepts(
    evidence_path: Path,
    ui_phrases: Iterable[str],
    entity_frames: frozenset[str],
) -> tuple[list[OntologyConcept], dict[str, Any]]:
    """Read the audit's evidence and produce one record per observed game name.

    Only rows on entity-bearing frames are considered: an idle fishing-bar frame
    reading ``x1`` is UI, and turning it into an ontology question would bury the
    real ones.
    """
    ui_list = list(ui_phrases)
    buckets: dict[str, _Bucket] = {}

    skipped_ui = 0
    skipped_non_entity = 0
    considered = 0

    with evidence_path.open(encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            row = json.loads(line)

            if row.get("image_category") not in entity_frames:
                skipped_non_entity += 1
                continue
            residual = (row.get("residual_text") or "").strip()
            if not residual:
                skipped_non_entity += 1
                continue

            considered += 1
            name, notes = extract_concept_name(residual, ui_list)
            if name is None:
                skipped_ui += 1
                continue

            key = group_key(name)
            b = buckets.get(key)
            if b is None:
                b = buckets[key] = _Bucket(key=key, names=set(), rows=[])
            b.names.add(name)
            b.rows.append({"name": name, "raw": residual, "row": row, "notes": notes})

    concepts: list[OntologyConcept] = []
    for key, b in buckets.items():
        # The canonical surface name is the most frequent spelling; ties break
        # alphabetically so the choice is deterministic.
        counts: dict[str, int] = {}
        for item in b.rows:
            counts[item["name"]] = counts.get(item["name"], 0) + 1
        display = sorted(counts.items(), key=lambda kv: (-kv[1], kv[0]))[0][0]

        rows = b.rows
        sess = sorted({r["row"]["session_id"] for r in rows if r["row"].get("session_id")})
        days = sorted({
            _session_day(r["row"].get("session_id"))
            for r in rows
            if _session_day(r["row"].get("session_id"))
        })
        stored: dict[str, int] = {}
        for r in rows:
            ent = r["row"].get("stored_entity_id")
            if ent:
                stored[ent] = stored.get(ent, 0) + 1

        # Candidate entities: union across the rows, from the ONTOLOGY only.
        cands: set[str] = set()
        for r in rows:
            cands.update(r["row"].get("candidate_entities") or [])
            cands.update(r["row"].get("matched_entities") or [])
        candidates = sorted(cands)

        if not candidates:
            status = OntologyStatus.UNKNOWN.value
        elif len(candidates) >= 2:
            status = OntologyStatus.AMBIGUOUS.value
        else:
            status = OntologyStatus.EXACT_ENTITY.value

        base, modifiers = decompose(display)
        prio, prio_reasons = priority_for(len(rows), len(sess), len(days))

        stamps = [r["row"].get("timestamp_ms") or 0 for r in rows]
        stamps = [s for s in stamps if s]

        agrees = sum(
            1 for r in rows
            if r["row"].get("stored_entity_id")
            and r["row"].get("stored_entity_id") in (r["row"].get("candidate_entities") or [])
        )

        concepts.append(OntologyConcept(
            concept_id=concept_id_for(display),
            display_name=display,
            normalized_name=display,
            group_key=key,
            occurrences=len(rows),
            sessions=sess,
            calendar_days=days,
            first_seen_ms=min(stamps) if stamps else 0,
            last_seen_ms=max(stamps) if stamps else 0,
            ocr_variants=sorted(counts),
            variant_counts=dict(sorted(counts.items(), key=lambda kv: (-kv[1], kv[0]))),
            current_kb_candidates=candidates,
            ontology_status=status,
            base_name=base,
            modifiers=modifiers,
            family_id=family_for(base, display),
            stored_labels=dict(sorted(stored.items(), key=lambda kv: (-kv[1], kv[0]))),
            total_labeled_rows=sum(stored.values()),
            stored_agrees_with_candidate=agrees,
            priority=prio,
            priority_reasons=prio_reasons,
            all_image_ids=sorted(r["row"]["image_id"] for r in rows),
        ))

    # Strongest evidence first; ties alphabetical so the order is stable.
    concepts.sort(key=lambda c: (-c.occurrences, -len(c.sessions), c.display_name))

    stats = {
        "evidence_rows_considered": considered,
        "rows_skipped_non_entity_frame": skipped_non_entity,
        "rows_skipped_as_ui_noise": skipped_ui,
        "raw_distinct_strings": len({r["raw"] for b in buckets.values() for r in b.rows}),
        "concepts": len(concepts),
    }
    return concepts, stats


def _session_day(session_id: str | None) -> str:
    if not session_id:
        return ""
    parts = session_id.split("_")
    return parts[1] if len(parts) > 1 else ""


# --------------------------------------------------------------------------
# Families, modifiers, base names
# --------------------------------------------------------------------------

def build_families(concepts: list[OntologyConcept]) -> dict[str, dict[str, Any]]:
    """Group concepts by shared base word.

    Explicitly NOT a merge. Each family records its members and is labelled as a
    grouping for navigation, because whether the members are one entity, several,
    or a base plus modifiers is a human question.
    """
    fams: dict[str, dict[str, Any]] = {}
    for c in concepts:
        f = fams.setdefault(c.family_id or family_for(None, c.display_name), {
            "family_id": c.family_id or family_for(None, c.display_name),
            "base_name": c.base_name,
            "member_concepts": [],
            "occurrences": 0,
            "sessions": set(),
            "days": set(),
        })
        f["member_concepts"].append({
            "concept_id": c.concept_id,
            "display_name": c.display_name,
            "modifiers": c.modifiers,
            "occurrences": c.occurrences,
        })
        f["occurrences"] += c.occurrences
        f["sessions"].update(c.sessions)
        f["days"].update(c.calendar_days)

    out: dict[str, dict[str, Any]] = {}
    for fid, f in fams.items():
        members = sorted(f["member_concepts"], key=lambda m: -m["occurrences"])
        # A family is a SINGLETON when it has one member - it is not yet evidence
        # of a base/modifier scheme, and saying so prevents the family view from
        # implying structure that was never observed.
        out[fid] = {
            "family_id": fid,
            "base_name": f["base_name"],
            "members": members,
            "member_count": len(members),
            "is_singleton": len(members) == 1,
            "occurrences": f["occurrences"],
            "session_count": len(f["sessions"]),
            "day_count": len(f["days"]),
            "decision_required": (
                "are these one entity, several entities, a base fish plus "
                "modifiers, or unrelated names?"
                if len(members) > 1 else
                "single observed name; no base/modifier structure established"
            ),
        }
    return out


def build_modifier_report(concepts: list[OntologyConcept]) -> list[dict[str, Any]]:
    """Recurring modifiers across concepts.

    EVIDENCE ONLY. A high count for ``golden`` says the word recurs; it does NOT
    say ``golden`` is a colour rather than a species. That question is precisely
    what the review exists to answer, and the existing KB treating ``golden`` as
    a species is the hypothesis under test.
    """
    mods: dict[str, dict[str, Any]] = {}
    for c in concepts:
        for m in c.modifiers:
            e = mods.setdefault(m, {"modifier": m, "concepts": [], "occurrences": 0,
                                    "sessions": set(), "days": set(),
                                    "bases": set()})
            e["concepts"].append({
                "concept_id": c.concept_id,
                "display_name": c.display_name,
                "occurrences": c.occurrences,
            })
            e["occurrences"] += c.occurrences
            e["sessions"].update(c.sessions)
            e["days"].update(c.calendar_days)
            if c.base_name:
                e["bases"].add(c.base_name)

    out = []
    for m, e in mods.items():
        concepts_sorted = sorted(e["concepts"], key=lambda x: -x["occurrences"])
        out.append({
            "modifier": m,
            "occurrences": e["occurrences"],
            "concept_count": len(concepts_sorted),
            "concepts": concepts_sorted,
            "bases": sorted(e["bases"]),
            "session_count": len(e["sessions"]),
            "day_count": len(e["days"]),
        })
    out.sort(key=lambda x: (-x["occurrences"], x["modifier"]))
    return out


def build_base_report(concepts: list[OntologyConcept]) -> list[dict[str, Any]]:
    """Terminal/base words and what currently maps onto them.

    Particularly important because the existing KB appears to have been built from
    FRAGMENTS of compound names (``fish:crimson``, ``fish:golden``) rather than
    from the names the game actually renders. This table makes that visible.
    """
    bases: dict[str, dict[str, Any]] = {}
    for c in concepts:
        b = c.base_name or c.display_name
        e = bases.setdefault(b, {"base_name": b, "concepts": [], "occurrences": 0,
                                 "sessions": set(), "days": set(),
                                 "kb_entities": set(), "stored_labels": {}})
        e["concepts"].append({
            "concept_id": c.concept_id,
            "display_name": c.display_name,
            "modifiers": c.modifiers,
            "occurrences": c.occurrences,
            "priority": c.priority,
        })
        e["occurrences"] += c.occurrences
        e["sessions"].update(c.sessions)
        e["days"].update(c.calendar_days)
        for ent in c.current_kb_candidates:
            e["kb_entities"].add(ent)
        for lbl, n in c.stored_labels.items():
            e["stored_labels"][lbl] = e["stored_labels"].get(lbl, 0) + n

    out = []
    for b, e in bases.items():
        concepts_sorted = sorted(e["concepts"], key=lambda x: -x["occurrences"])
        # `fragment_of_a_compound` is the key signal: a base whose occurrences are
        # always accompanied by a modifier suggests the KB entity named after it
        # is describing a fragment, not a thing the game calls on its own.
        modifier_counts = {}
        for cc in concepts_sorted:
            for m in cc["modifiers"]:
                modifier_counts[m] = modifier_counts.get(m, 0) + cc["occurrences"]
        bare = [cc for cc in concepts_sorted if not cc["modifiers"]]
        out.append({
            "base_name": b,
            "occurrences": e["occurrences"],
            "concept_count": len(concepts_sorted),
            "concepts": concepts_sorted,
            "kb_candidates": sorted(e["kb_entities"]),
            "stored_labels": dict(sorted(e["stored_labels"].items(),
                                         key=lambda kv: (-kv[1], kv[0]))),
            "session_count": len(e["sessions"]),
            "day_count": len(e["days"]),
            "occurs_bare": len(bare) > 0,
            "bare_concept": bare[0]["display_name"] if bare else None,
            "modifier_totals": dict(sorted(modifier_counts.items(),
                                           key=lambda kv: (-kv[1], kv[0]))),
        })
    out.sort(key=lambda x: (-x["occurrences"], x["base_name"]))
    return out