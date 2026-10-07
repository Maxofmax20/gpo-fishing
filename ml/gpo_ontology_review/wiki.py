"""GPO wiki bestiary: fetch, parse, snapshot, and match against observed names.

WHY THIS EXISTS
===============

The review question is "what entity does the GAME appear to be naming?". Until now
the only evidence was internal: OCR text, and a knowledge base written by this
project. Both are circular - the KB was built from assumptions, and OCR says only
what the pixels look like.

The game's own wiki is independent external evidence. Grand Piece Online publishes a
`Fishing/Bestiary` page listing every fish with its rarity and whether it is
seasonal. That answers "is this name the thing, or not?" directly, and it settles the
structural questions the internal data could only pose:

* There is no bare `angelfish` in the game. `fish:angelfish` in the knowledge base is
  fictional. The real fish are `Zebra Ribbon Angelfish` and
  `Golden Ribbon Angelfish`.
* Likewise there is no bare `puffer`, `snapper`, `squid` or `shark`. Every one of
  those is part of a longer real name.
* So `golden`, `crimson`, `zebra` and `exotic` are not modifiers that colour a base
  species - they are parts of species names.

SNAPSHOT, NOT LIVE
==================

Review speed and reproducibility both matter here, so the default is a committed
snapshot:

* a reviewer making a decision never waits on a network call;
* two reviewers looking at the same concept see the same reference list;
* a decision recorded months ago can still be explained, because the snapshot that
  informed it is in the repository with its retrieval time and content hash.

A live refresh is available (see `refresh_snapshot`) and is deliberately explicit:
it is an outbound request the human asks for, not something a page load triggers.

THE WIKI IS EVIDENCE, NOT TRUTH
===============================

The Grand Piece Online wiki is community-editable. Someone can add a fish that does
not exist, or misspell one. So a match is labelled as a *reference*, never as a
decision, and the human still chooses. This matters especially in the negative
direction: "no wiki match" does NOT mean the game does not have that fish, because
the wiki may simply lag the game. Both directions of the match are reported with
that caveat attached.
"""

from __future__ import annotations

import hashlib
import json
import re
import time
import unicodedata
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

#: The only host this module will ever contact.
#:
#: Hard-coded, and validated against the request URL before the request is made. A
#: configurable wiki URL would turn a review tool into a server-side request forgery
#: primitive and a way to pull arbitrary remote content into a local file - neither is
#: acceptable for a page whose write path is already sensitive.
WIKI_HOST = "grand-piece-online.fandom.com"

#: The MediaWiki API endpoint, pinned to HTTPS on the pinned host.
API_URL = f"https://{WIKI_HOST}/api.php"

#: The page holding the fish table.
BESTIARY_PAGE = "Fishing/Bestiary"

#: The human-facing page, for the "open the wiki" link.
BESTIARY_WEB_URL = f"https://{WIKI_HOST}/wiki/{BESTIARY_PAGE.replace(' ', '_')}"

#: Rarity template codes, in the wiki's shorthand.
RARITY_CODES = {"C": "Common", "R": "Rare", "E": "Epic", "L": "Legendary"}

#: Timeout for the network call. A review tool must not hang on a slow wiki.
FETCH_TIMEOUT_SECONDS = 10

#: User-Agent. Set deliberately: an unnamed scraper is the kind of thing a wiki
#: operator blocks, and being identifiable is the courteous default.
USER_AGENT = "gpo-autofish-ontology-review/1.0 (local review tool; one request on demand)"


# ---------------------------------------------------------------------------
# Matching key
# ---------------------------------------------------------------------------

def match_key(name: str) -> str:
    """Reduce a name to its rendering-equivalence key.

    Strips accents, punctuation and whitespace. This is deliberately the SAME rule
    the ontology pipeline already uses for `group_key`, which merges renderings of one
    name (`skeletalshark` / `Skeletal Shark`) and never two different names
    (`sgolden tigerfin` stays separate from `golden tigerfin`).

    Reusing one rule matters: if the wiki matcher and the concept grouper disagreed,
    the same fish would be one concept internally and "unmatched" against the wiki,
    which is exactly the confusion this tool exists to remove.
    """
    s = unicodedata.normalize("NFKD", name)
    s = "".join(c for c in s if not unicodedata.combining(c))
    return re.sub(r"[^a-z0-9]+", "", s.lower())


def edit_distance(a: str, b: str, cap: int = 3) -> int:
    """Levenshtein distance, short-circuited above `cap`.

    A two-row band is plenty to rank near-misses and keeps the suggestion pass linear
    enough to run over every concept at build time.
    """
    if abs(len(a) - len(b)) > cap:
        return cap + 1
    prev = list(range(len(b) + 1))
    for i, ca in enumerate(a, 1):
        cur = [i]
        best = i
        for j, cb in enumerate(b, 1):
            cost = 0 if ca == cb else 1
            v = min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + cost)
            cur.append(v)
            best = min(best, v)
        if best > cap:
            return cap + 1
        prev = cur
    return prev[-1]


@dataclass
class WikiFish:
    name: str
    rarity: str
    seasonal: bool
    description: str = ""
    key: str = field(init=False)

    def __post_init__(self) -> None:
        self.key = match_key(self.name)


@dataclass
class Match:
    """How one observed name relates to the wiki list."""

    matched: bool
    canonical_name: str | None = None
    rarity: str | None = None
    seasonal: bool | None = None
    description: str | None = None
    url: str | None = None
    #: Populated when there is no exact match but something is clearly close.
    suggestion: str | None = None
    suggestion_rarity: str | None = None
    suggestion_distance: int | None = None
    #: Why the match happened, so the UI can show it rather than assert it.
    basis: str = "none"

    def to_json(self) -> dict[str, Any]:
        return {
            "matched": self.matched,
            "canonicalName": self.canonical_name,
            "rarity": self.rarity,
            "seasonal": self.seasonal,
            "description": self.description,
            "url": self.url,
            "suggestion": self.suggestion,
            "suggestionRarity": self.suggestion_rarity,
            "suggestionDistance": self.suggestion_distance,
            "basis": self.basis,
        }


# ---------------------------------------------------------------------------
# Fetch + parse
# ---------------------------------------------------------------------------

def fetch_wikitext(*, timeout: int = FETCH_TIMEOUT_SECONDS) -> str:
    """Fetch the bestiary wikitext.

    The URL is rebuilt here and re-checked against `WIKI_HOST`, so even a future edit
    that parameterised the page name could not redirect the request elsewhere.
    """
    url = (
        f"{API_URL}?action=parse&page={urllib.parse.quote(BESTIARY_PAGE)}"
        "&prop=wikitext&format=json&formatversion=2"
    )
    if WIKI_HOST not in urllib.parse.urlparse(url).netloc:
        raise ValueError(f"refusing to contact anything but {WIKI_HOST}")

    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(req, timeout=timeout) as resp:  # noqa: S310 - host pinned above
        raw = resp.read()
    payload = json.loads(raw.decode("utf-8"))
    if "error" in payload:
        raise RuntimeError(f"wiki API error: {payload['error']}")
    return payload["parse"]["wikitext"]


def parse_bestiary(wikitext: str) -> list[WikiFish]:
    """Extract the fish table from the page's wikitext.

    Deliberately strict: a row that yields no name is DROPPED, but if the whole page
    yields nothing recognizable this raises rather than returning an empty list. An
    empty snapshot is the dangerous outcome - it would make every concept report "no
    wiki match" and quietly turn a missing reference into evidence of absence.
    """
    section = wikitext.split("== Bestiary ==", 1)
    if len(section) < 2:
        raise ValueError("no '== Bestiary ==' section found; the wiki layout changed")
    body = section[1]

    fish: list[WikiFish] = []
    seen: set[str] = set()
    for row in re.split(r"\n\|-", body):
        if "[[File:" not in row and "''" not in row:
            continue

        # The name is the bold run: '''Something'''.
        names = re.findall(r"'''(.+?)'''", row)
        if not names:
            continue
        name = names[0].strip()
        # Strip any wiki markup inside the name.
        name = re.sub(r"<[^>]+>", "", name)
        name = name.replace("[[", "").replace("]]", "").strip()
        if not name:
            continue

        key = match_key(name)
        if not key or key in seen:
            continue
        seen.add(key)

        # Rarity: the first {{Rarity|X}} template. Rows may carry a second, in a
        # parenthetical about bait crafting; the first is the fish's own rarity.
        codes = re.findall(r"\{\{Rarity\|([A-Z])\}\}", row)
        rarity = RARITY_CODES.get(codes[0], "Unknown") if codes else "Unknown"

        # Seasonal: a bare Yes/No on its own line.
        seasonal = bool(re.search(r"^\s*\|\s*yes\s*$", row, re.IGNORECASE | re.MULTILINE))

        lines = [l.strip() for l in row.split("\n")]
        description = ""
        for l in lines:
            if not l.startswith("|"):
                continue
            cell = l[1:].strip()
            if (
                not cell
                or cell.startswith("[[File:")
                or "''" in cell
                or cell.startswith("{{Rarity")
                or cell.lower() in ("yes", "no")
                or cell.startswith("|+")
            ):
                continue
            description = re.sub(r"<[^>]+>", "", cell).strip()
            break

        fish.append(
            WikiFish(
                name=name,
                rarity=rarity,
                seasonal=seasonal,
                description=description,
            )
        )

    if not fish:
        raise ValueError("the bestiary table produced no fish; refusing to write an empty snapshot")
    return fish


# ---------------------------------------------------------------------------
# Snapshot
# ---------------------------------------------------------------------------

def build_snapshot(
    fish: list[WikiFish],
    *,
    source_url: str = BESTIARY_WEB_URL,
    retrieved_at_unix: int | None = None,
) -> dict[str, Any]:
    """The committed reference artefact.

    Carries provenance so a reviewer can tell how old the list is and where it came
    from, which is the minimum needed to trust a "no match" answer.
    """
    entries = sorted(
        (
            {
                "name": f.name,
                "matchKey": f.key,
                "rarity": f.rarity,
                "seasonal": f.seasonal,
                "description": f.description,
                "url": BESTIARY_WEB_URL,
            }
            for f in fish
        ),
        key=lambda e: e["name"].lower(),
    )
    blob = json.dumps(entries, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return {
        "kind": "ontology_wiki_bestiary",
        "version": 1,
        "sourceHost": WIKI_HOST,
        "sourcePage": BESTIARY_PAGE,
        "sourceUrl": source_url,
        "retrievedAtUnix": int(retrieved_at_unix if retrieved_at_unix is not None else time.time()),
        "entryCount": len(entries),
        "contentSha256": hashlib.sha256(blob).hexdigest(),
        "caveat": (
            "Community-editable reference, not an authority. A match is evidence that "
            "the name is a documented fish; NO match is evidence only that this list "
            "does not contain it, because the wiki can lag the game. Never a decision."
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
    """Fetch the wiki and write the snapshot. Returns the new snapshot.

    Only ever writes `path`. Callers must treat this as the single write this module
    performs; everything else is pure.
    """
    fish = parse_bestiary(fetch_wikitext(timeout=timeout))
    snap = build_snapshot(fish)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(snap, indent=1, sort_keys=True), encoding="utf-8")
    return snap


# ---------------------------------------------------------------------------
# Matching observed names against the snapshot
# ---------------------------------------------------------------------------

def match_name(name: str, snapshot: dict[str, Any]) -> Match:
    """Classify one observed name against the reference list.

    Three outcomes, in order of strength:

    1. **Exact** - the rendering key matches a wiki entry.
    2. **Suggestion** - no exact match, but something is within a small edit
       distance. This is the case that matters most for OCR garbles like
       `blve lipgrouper` -> `Blue-Lip Grouper`: reporting "no match" there would tell
       the reviewer something false.
    3. **No match** - genuinely absent from this list, which is NOT evidence the fish
       does not exist.
    """
    entries = snapshot.get("entries", [])
    if not entries:
        return Match(matched=False, basis="reference unavailable")

    by_key = {e["matchKey"]: e for e in entries}
    k = match_key(name)
    hit = by_key.get(k)
    if hit:
        return Match(
            matched=True,
            canonical_name=hit["name"],
            rarity=hit["rarity"],
            seasonal=hit["seasonal"],
            description=hit.get("description") or None,
            url=hit.get("url"),
            basis="exact match on the name",
        )

    # Near-miss. Scale the tolerance with length so a short name is not "close" to
    # half the list.
    cap = 1 if len(k) <= 6 else 2
    best: tuple[int, dict[str, Any]] | None = None
    for e in entries:
        d = edit_distance(k, e["matchKey"], cap=cap)
        if d <= cap and (best is None or d < best[0]):
            best = (d, e)
    if best:
        d, e = best
        return Match(
            matched=False,
            suggestion=e["name"],
            suggestion_rarity=e["rarity"],
            suggestion_distance=d,
            canonical_name=None,
            rarity=None,
            seasonal=None,
            description=e.get("description") or None,
            url=e.get("url"),
            basis=f"no exact match; closest reference is {d} character(s) away",
        )

    return Match(matched=False, basis="not present in this reference list")


def summarise(matches: list[Match]) -> dict[str, Any]:
    exact = sum(1 for m in matches if m.matched)
    near = sum(1 for m in matches if not m.matched and m.suggestion)
    return {
        "total": len(matches),
        "exact": exact,
        "nearMiss": near,
        "absent": len(matches) - exact - near,
    }