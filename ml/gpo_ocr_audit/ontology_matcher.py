"""Ontology matching: from text to candidate KB entities, never to an answer.

This module is where the audit could most easily do harm, so its central rule is
stated once and enforced everywhere below:

    A name that matches zero or two-or-more canonical entities is NOT resolved.

The 8-class failure this project already suffered from came from a resolver that
picked one plausible component of a compound name. `Crimson Snapper` contains
`crimson` and `snapper`, both canonical, and the old behaviour collapsed them
into one. Here that case returns two candidates and status AMBIGUOUS, and the
caller must route it to a human.

Matching runs the same five escalating stages as the Rust `canon::resolve`, and
stops at the first one that produces a definitive result:

===========  ==========================================================
Stage        Meaning
===========  ==========================================================
EXACT        raw text is byte-identical to a canonical name
NORMALIZED   differs only by case / punctuation / whitespace / unicode
ALIAS        reached through a KB-declared alias
OCR_VARIANT  reached through a registered correction rule
CONFUSABLE   reached through lossy digit/letter folding
FUZZY        edit-distance candidate - a SUGGESTION, never a resolution
===========  ==========================================================

FUZZY deliberately terminates the process. It returns candidates and nothing
else; no downstream code may treat them as labels.
"""

from __future__ import annotations

import difflib
import json
from dataclasses import dataclass
from functools import lru_cache
from pathlib import Path
from typing import Iterable

from .schemas import (
    CompoundFinding,
    MatchKind,
    OntologyStatus,
)
from .text_normalizer import (
    normalize,
    ocr_variant_fold,
    unicode_fold,
)

#: Minimum normalised-input length before a token may be matched as a prefix.
#: Mirrors `canon::MIN_SUBSTRING_INPUT_LEN`: without a floor, "on" would match
#: `crimson`, `marlin`, `cod` and `pufferfish`.
MIN_SUBSTRING_INPUT_LEN = 4

#: Above this edit-distance similarity a fuzzy candidate is worth showing.
#: It is a *display* threshold, not a decision: nothing is resolved at this level.
FUZZY_CUTOFF = 0.82

#: How a compound-token hit is reported in the ontology status.
_TOKEN_STATUS = {
    MatchKind.EXACT: OntologyStatus.EXACT_ENTITY,
    MatchKind.NORMALIZED: OntologyStatus.EXACT_ENTITY,
    MatchKind.ALIAS: OntologyStatus.ALIAS_ENTITY,
    MatchKind.OCR_VARIANT: OntologyStatus.OCR_VARIANT_ENTITY,
    MatchKind.CONFUSABLE: OntologyStatus.CONFUSABLE_ENTITY,
    MatchKind.FUZZY: OntologyStatus.FUZZY_CANDIDATE,
}


@dataclass(frozen=True)
class Entity:
    id: str
    canonical_name: str
    aliases: tuple[str, ...]
    ocr_aliases: tuple[str, ...]
    category: str
    rarity: str | None


@dataclass
class MatchResult:
    """Outcome of matching one text against the ontology."""

    kind: MatchKind
    status: OntologyStatus
    matched: list[str]
    candidates: list[str]
    fuzzy: list[str]
    compound_tokens: list[CompoundFinding]
    ui_templates: list[str]
    residual: str
    reason: str
    #: entity_id -> KB category, so a report can say "this candidate is a FRUIT
    #: inside a FISH drop" instead of silently listing a fruit next to a fish.
    categories: dict[str, str]
    #: secondary observations that are NOT resolutions - fragment-only entity
    #: hits. Reported so a human sees them; never treated as candidates.
    extras: list[str]

    @property
    def is_unresolved(self) -> bool:
        """True when no single entity may be chosen."""
        return self.kind in (MatchKind.AMBIGUOUS, MatchKind.NONE)


class Ontology:
    """The authoritative GPO knowledge base, loaded once and queried read-only.

    Loaded from ``kb_export.json``, which a Rust *example* target serialises
    straight out of `core::knowledge::build_bundled`. Loading it any other way
    would mean maintaining a second ontology in Python, which is precisely the
    failure mode `FISH_VOCAB.txt` was introduced to eliminate.
    """

    def __init__(self, payload: dict) -> None:
        self.kb_version: int = payload["kb_version"]
        self.schema: int = payload["schema"]
        self.fish_vocab: list[str] = list(payload["fish_vocab"])
        self.fish_vocab_sha256: str = payload["fish_vocab_sha256"]
        self.entities: list[Entity] = [
            Entity(
                id=e["id"],
                canonical_name=e["canonical_name"],
                aliases=tuple(e.get("aliases") or ()),
                ocr_aliases=tuple(e.get("ocr_aliases") or ()),
                category=e["category"],
                rarity=e.get("rarity"),
            )
            for e in payload["entities"]
        ]
        self._by_id = {e.id: e for e in self.entities}

        # Three indexes, deliberately separated by what a hit MEANS.
        #
        # `_exact` and `_alias` hold NON-template names only. UI templates are
        # banner furniture, not entities: if they entered this index, every drop
        # would read as "ambiguous between fish:snapper and ui:new-item", which
        # is both noise and a lie about the ontology.
        self._exact: dict[str, set[str]] = {}
        self._alias: dict[str, set[str]] = {}
        self._confusable: dict[str, set[str]] = {}
        #: (folded_name, entity_id) for NON-template entities, longest first.
        self._folded_names: list[tuple[str, str]] = []

        for ent in self.entities:
            if ent.category == "ui_term":
                continue
            for name in self._names(ent):
                key = self._key(name)
                if key:
                    self._exact.setdefault(key, set()).add(ent.id)
                folded = self._folded(key)
                if folded:
                    self._confusable.setdefault(folded, set()).add(ent.id)
                    self._folded_names.append((folded, ent.id))
            for alias in tuple(ent.aliases) + tuple(ent.ocr_aliases):
                akey = self._key(alias)
                if akey:
                    self._alias.setdefault(akey, set()).add(ent.id)

        # Longest first, then alphabetical: a greedy longest-span scan is what
        # makes `golden` win over `gold` (fruit:goru) and `common bait` over
        # `common`. Every substring collision in this KB resolves to the longer,
        # more specific name under that ordering.
        self._folded_names.sort(key=lambda p: (-len(p[0]), p[0], p[1]))

        # Banner templates, stripped before any entity lookup.
        self._ui_keys = sorted(
            (self._key(n) for e in self.entities if e.category == "ui_term"
             for n in self._names(e) if self._key(n)),
            key=len, reverse=True,
        )
        self._ui_id_by_key = {
            self._key(n): e.id
            for e in self.entities if e.category == "ui_term"
            for n in self._names(e)
        }

    # -- construction ----------------------------------------------------

    @staticmethod
    def _names(ent: Entity) -> tuple[str, ...]:
        return (ent.canonical_name,) + ent.aliases + ent.ocr_aliases

    @staticmethod
    def _key(s: str) -> str:
        folded, _ = normalize(unicode_fold(s)[0])
        return folded

    @staticmethod
    def _folded(s: str) -> str:
        return ocr_variant_fold(s)[0]

    @classmethod
    def load(cls, path: str | Path) -> "Ontology":
        payload = json.loads(Path(path).read_text(encoding="utf-8"))
        if payload.get("schema") != 1:
            raise ValueError(
                f"unsupported kb_export schema {payload.get('schema')!r}; "
                "regenerate with `cargo run --example kb_export`"
            )
        return cls(payload)

    def name_of(self, entity_id: str) -> str:
        ent = self._by_id.get(entity_id)
        return ent.canonical_name if ent else entity_id

    def category_of(self, entity_id: str) -> str:
        ent = self._by_id.get(entity_id)
        return ent.category if ent else "unknown"

    # -- stage lookups ---------------------------------------------------

    def _lookup_exact(self, key: str) -> set[str]:
        return set(self._exact.get(key, ()))

    def _lookup_alias(self, key: str) -> set[str]:
        return set(self._alias.get(key, ()))

    def _lookup_confusable(self, folded: str) -> set[str]:
        return set(self._confusable.get(folded, ()))

    def _lookup_fuzzy(self, key: str) -> list[str]:
        """Edit-distance suggestions. Never a resolution."""
        if len(key) < MIN_SUBSTRING_INPUT_LEN:
            return []
        names = {}
        for folded, ent_id in self._folded_names:
            if abs(len(folded) - len(key)) > 3:
                continue
            names.setdefault(ent_id, folded)
        scored = []
        for ent_id, cand in names.items():
            ratio = difflib.SequenceMatcher(None, key, cand).ratio()
            if ratio >= FUZZY_CUTOFF:
                scored.append((ratio, ent_id))
        scored.sort(key=lambda p: (-p[0], p[1]))
        return [e for _, e in scored[:5]]

    def strip_ui_templates(self, key: str) -> tuple[str, list[str]]:
        """Remove banner template words, returning (residual, template_ids).

        The KB models the drop-message templates themselves (`ui:new-item` =
        "new item"), so they must be separated from the entity name before any
        entity lookup runs.

        Operates on the NORMALISED text, never the confusable-folded one.
        Folding maps `i`->`l`, so the folded template reads `new ltem` while the
        name being matched reads `crlmson`; searching folded text for folded
        templates therefore misses both, and the template leaks into the entity
        candidate list as a phantom ambiguity.
        """
        residual = key
        found: list[str] = []
        for tmpl in self._ui_keys:
            idx = residual.find(tmpl)
            if idx >= 0:
                residual = residual[:idx] + " " + residual[idx + len(tmpl):]
                tid = self._ui_id_by_key.get(tmpl)
                if tid:
                    found.append(tid)
        return " ".join(residual.split()), sorted(set(found))

    # -- the public matcher ----------------------------------------------

    def match(self, raw_text: str, normalized_text: str) -> MatchResult:
        """Resolve one text against the ontology.

        ``raw_text`` is what the engine literally read; ``normalized_text`` is the
        registered-correction form. All matching runs on the NORMALISED text. The
        confusable fold is applied only for candidate scanning, and a hit reached
        through it is always reported as :attr:`MatchKind.CONFUSABLE`, because the
        fold is lossy: `5hark` really does reach `fish:shark`, but the pixels never
        said "shark", and calling that exact is the misrepresentation this audit
        exists to prevent.

        Every return uses keyword arguments: the ``MatchResult`` field list is long
        and mostly empty, and positional construction is how a stray ``[]`` lands
        in the wrong slot and turns a template list into a token list.
        """
        del raw_text  # kept in the signature so call sites read clearly

        def res(kind, status, *, matched=(), candidates=(), fuzzy=(), tokens=(),
                ui=(), residual="", reason="", categories=None, extras=()) -> MatchResult:
            return MatchResult(
                kind=kind, status=status,
                matched=list(matched), candidates=list(candidates), fuzzy=list(fuzzy),
                compound_tokens=list(tokens), ui_templates=list(ui),
                residual=residual, reason=reason, categories=dict(categories or {}),
                extras=list(extras),
            )

        def cats(ids):
            return {e: self.category_of(e) for e in ids}

        if not normalized_text.strip():
            return res(MatchKind.NONE, OntologyStatus.UNREADABLE,
                       reason="no text recovered from the image")

        # --- stage 0: strip banner templates -----------------------------
        residual, ui = self.strip_ui_templates(normalized_text)
        if not residual:
            return res(MatchKind.NONE, OntologyStatus.UI_TEMPLATE, ui=ui,
                       reason="text is entirely a known banner template with no entity name")

        # --- stage 1: whole-text exact on the NORMALISED key ------------
        hit = self._lookup_exact(residual)
        if len(hit) == 1:
            only = next(iter(hit))
            return res(MatchKind.EXACT, OntologyStatus.EXACT_ENTITY, matched=[only],
                       candidates=[only], ui=ui, residual=residual, categories=cats([only]),
                       reason="the whole name is one canonical entity, matched exactly")
        if len(hit) > 1:
            return res(MatchKind.AMBIGUOUS, OntologyStatus.COMPOUND, candidates=sorted(hit),
                       ui=ui, residual=residual, categories=cats(sorted(hit)),
                       reason=f"one name matched {len(hit)} distinct entities: {sorted(hit)}")

        # --- stage 2: alias ---------------------------------------------
        alias_hit = self._lookup_alias(residual)
        if len(alias_hit) == 1:
            only = next(iter(alias_hit))
            return res(MatchKind.ALIAS, OntologyStatus.ALIAS_ENTITY, matched=[only],
                       candidates=[only], ui=ui, residual=residual, categories=cats([only]),
                       reason="reached the entity through a KB-declared alias")
        if len(alias_hit) > 1:
            return res(MatchKind.AMBIGUOUS, OntologyStatus.COMPOUND,
                       candidates=sorted(alias_hit), ui=ui, residual=residual,
                       categories=cats(sorted(alias_hit)),
                       reason=f"alias matched {len(alias_hit)} entities: {sorted(alias_hit)}")

        # --- stage 3: compound analysis over the name's words ------------
        toks, covered = self.scan_tokens(residual)
        word_hits = sorted({t.entity_id for t in toks if t.whole_word})
        partial_hits = sorted({t.entity_id for t in toks if not t.whole_word})
        leftover = self._leftover(residual, covered)

        # Two or more whole-word entities: no single answer exists. This is the
        # `Crimson Snapper` case and the reason the old resolver was wrong.
        if len(word_hits) >= 2:
            return res(MatchKind.AMBIGUOUS, OntologyStatus.COMPOUND, candidates=word_hits,
                       tokens=toks, ui=ui, residual=residual, categories=cats(word_hits),
                       extras=partial_hits,
                       reason=f"{len(word_hits)} canonical entities appear as whole words in "
                              f"one name ({', '.join(word_hits)}); NO single choice is made")

        # Exactly one whole-word entity.
        if len(word_hits) == 1:
            only = word_hits[0]
            finding = next(t for t in toks if t.entity_id == only and t.whole_word)
            if not leftover:
                kind = MatchKind(finding.match_kind)
                how = {
                    MatchKind.EXACT: "the name is exactly the canonical entity",
                    MatchKind.NORMALIZED: "the name is the canonical entity after normalisation",
                    MatchKind.ALIAS: "the name reaches the entity through a KB alias",
                    MatchKind.CONFUSABLE: "the name is the entity only after lossy digit/letter "
                                          "folding, so it is CONFUSABLE and not exact",
                }.get(kind, "the name is the canonical entity")
                return res(kind, _TOKEN_STATUS.get(kind, OntologyStatus.EXACT_ENTITY),
                           matched=[only], candidates=[only], tokens=toks, ui=ui,
                           residual=residual, categories=cats([only]), extras=partial_hits,
                           reason=f"{how} {only!r}")
            # A known species plus words the ontology cannot explain. The species
            # match is real, but the name is not accounted for, so it is NOT a
            # confirmation: `golden polka puffer` contains `golden` and two words
            # the KB has never heard of.
            return res(MatchKind.PARTIAL, OntologyStatus.UNKNOWN_ENTITY,
                       tokens=toks, ui=ui, residual=residual, categories=cats([only]),
                       extras=partial_hits,
                       reason=f"{only!r} appears in the name but {leftover!r} has no KB "
                              f"entity; the name is only partly explained, so a human "
                              f"decides whether {leftover!r} is a variant of {only!r}, a "
                              f"separate entity, or an OCR artefact")

        # No whole-word entity.
        if len(partial_hits) >= 2:
            return res(MatchKind.AMBIGUOUS, OntologyStatus.COMPOUND,
                       candidates=partial_hits, tokens=toks, ui=ui, residual=residual,
                       categories=cats(partial_hits),
                       reason=f"{len(partial_hits)} KB names occur only INSIDE longer words "
                              f"({', '.join(partial_hits)}); reported as ambiguous because "
                              f"the game concatenated them")

        # --- stage 4: lossy confusable retry, only when the name is a
        # single word that folds onto an entity and nothing else explains it
        # A single WORD whose characters fold onto an entity (`5hark` -> `shark`)
        # is a genuine lossy hit and is reported as CONFUSABLE.
        #
        # The whole-word requirement is load-bearing. Without it, `tigerfin` -
        # one word, containing `tiger` - would resolve to `fish:tiger`, which is
        # precisely the invented mapping this audit must never make.
        whole = [t for t in toks if t.whole_word and t.entity_id in partial_hits]
        if len(partial_hits) == 1 and len(whole) == 1 and len(residual.split()) == 1:
            only = partial_hits[0]
            return res(MatchKind.CONFUSABLE, OntologyStatus.CONFUSABLE_ENTITY,
                       matched=[only], candidates=[only], tokens=toks, ui=ui,
                       residual=residual, categories=cats([only]),
                       reason=f"the single word {residual!r} folds onto {only!r}; reported as "
                              f"CONFUSABLE because the fold is lossy, never as an exact match")

        if len(partial_hits) == 1:
            only = partial_hits[0]
            return res(MatchKind.PARTIAL, OntologyStatus.UNKNOWN_ENTITY, tokens=toks,
                       ui=ui, residual=residual, categories=cats([only]),
                       reason=f"the name contains {only!r} only as a fragment of a longer "
                              f"word, so it does NOT resolve to {only!r}; a human decides")

        # --- stage 5: fuzzy SUGGESTIONS only, never a resolution --------
        fuzzy = self._lookup_fuzzy(residual)
        if fuzzy:
            return res(MatchKind.FUZZY, OntologyStatus.FUZZY_CANDIDATE, fuzzy=fuzzy,
                       ui=ui, residual=residual,
                       reason="no exact match; closest KB names are SUGGESTIONS only: "
                              f"{fuzzy}")

        # --- stage 6: nothing matched -----------------------------------
        return res(MatchKind.NONE, OntologyStatus.UNKNOWN_ENTITY, ui=ui, residual=residual,
                   reason=f"{residual!r} has no canonical entity in KB version "
                          f"{self.kb_version} and no template match; a human decides "
                          f"whether it is a new concept, a variant, or an OCR artefact")

    @staticmethod
    def _leftover(text: str, covered: list[tuple[int, int]]) -> str:
        """Words of ``text`` not explained by any WHOLE-WORD entity match.

        Fragment-only hits are deliberately left in the remainder. Masking them
        would report `golden tigerfin` as unexplained ``fin``, which reads as a
        separate mystery; leaving them in yields the honest ``tigerfin``.
        """
        masked = list(text)
        for start, end in covered:
            for i in range(start, end):
                masked[i] = " "
        return " ".join("".join(masked).split())

    def scan_tokens(self, text: str) -> tuple[list[CompoundFinding], list[tuple[int, int]]]:
        """Find canonical entity names occurring inside a longer name.

        Returns ``(findings, covered_spans)``.

        The compound analyzer. It reports EVERY entity it finds and chooses none:
        `crimson snapper` yields both `fish:crimson` and `fish:snapper`.

        Three properties make the greedy scan trustworthy here:

        1. **Longest span wins.** Patterns are tried longest-first and a claimed
           span blocks shorter overlaps, so `golden` beats the substring `gold`
           (fruit:goru) and `common bait` beats `common`. Every substring
           collision in this 141-entity KB resolves correctly under that order.
        2. **Substring, not word-boundary.** The game frequently concatenates the
           compound (`CrimsonSnapper`), so a word-boundary scan would silently miss
           the case this function exists to find.
        3. **Whole-word flag.** Property 2 creates a hazard: `tiger` occurs inside
           `tigerfin`. Such a hit is recorded but marked ``whole_word = False``, and
           the caller refuses to resolve it. That refusal is what stops
           `Exotic Tigerfin` from being reported as `fish:tiger`.

        Both indexes are built from NORMALISED keys, and the confusable fold is
        1:1 per character, so folded offsets align with the source string.
        """
        if not text:
            return [], []
        folded = self._folded(text)
        taken = [False] * len(folded)
        found: list[tuple[int, CompoundFinding]] = []
        covered: list[tuple[int, int]] = []

        for cand, ent_id in self._folded_names:
            start = 0
            while True:
                idx = folded.find(cand, start)
                if idx < 0:
                    break
                end = idx + len(cand)
                if not any(taken[idx:end]):
                    for i in range(idx, end):
                        taken[i] = True
                    original = text[idx:end]
                    if original in self._exact:
                        kind = MatchKind.EXACT
                    elif original in self._alias:
                        kind = MatchKind.ALIAS
                    else:
                        kind = MatchKind.CONFUSABLE
                    left_ok = idx == 0 or not text[idx - 1].isalnum()
                    right_ok = end == len(text) or not text[end].isalnum()
                    ent = self._by_id[ent_id]
                    found.append((idx, CompoundFinding(
                        token=original,
                        entity_id=ent_id,
                        canonical_name=ent.canonical_name,
                        match_kind=kind.value,
                        whole_word=left_ok and right_ok,
                        start=idx,
                    )))
                    if left_ok and right_ok:
                        covered.append((idx, end))
                start = idx + 1

        found.sort(key=lambda p: (p[0], p[1].entity_id))
        return [f for _, f in found], covered

    # -- reporting helpers ------------------------------------------------

    def entity_ids(self, category: str | None = None) -> list[str]:
        return sorted(
            e.id for e in self.entities if category is None or e.category == category
        )

    def summary(self) -> dict:
        by_cat: dict[str, int] = {}
        for e in self.entities:
            by_cat[e.category] = by_cat.get(e.category, 0) + 1
        return {
            "kb_version": self.kb_version,
            "entities": len(self.entities),
            "by_category": by_cat,
            "fish_vocab": self.fish_vocab,
            "fish_vocab_sha256": self.fish_vocab_sha256,
            "entities_with_aliases": sum(
                1 for e in self.entities if e.aliases or e.ocr_aliases
            ),
        }


@lru_cache(maxsize=1)
def load_ontology(path: str | None = None) -> Ontology:
    """Load and cache the ontology.

    Cached because the matcher builds several indexes and the audit calls
    ``match`` once per image; rebuilding for 14k images would dominate runtime.
    """
    if path is None:
        path = str(Path(__file__).with_name("kb_export.json"))
    return Ontology.load(path)