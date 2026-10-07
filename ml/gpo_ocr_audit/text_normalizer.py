"""OCR text normalisation, mirroring the Rust `canon` contract.

Why a Python mirror at all? Because the audit is a Python pipeline, but the
normalisation RULES must be identical to the ones the running app already uses
in `core::canon` (`unicode_fold` -> `fruit::normalize` -> `ocr_variant_fold`).
If the audit normalised differently from the app, a discrepancy between stored
and freshly-read text could be an artefact of two different normalisers rather
than a real OCR difference.

So the three functions below are line-for-line ports of the Rust originals, and
``tests/test_text_normalizer.py`` pins the shared behaviour. The Rust source
remains authoritative; this is a documented mirror, not a second standard.

On top of that the module carries :data:`OCR_CORRECTION_RULES`, a small table of
*explainable* word-level fixes. Each rule says what it repairs and why it is
safe. Rules are never derived from frequency alone: a rule exists because a
specific, nameable failure mode produces it (see the ``reason`` field), and a
test demonstrates the exact input it fixes.
"""

from __future__ import annotations

import re
import unicodedata
from typing import Iterable

from .schemas import NormalizationStep

# --------------------------------------------------------------------------
# Step 1 - unicode_fold (port of core::canon::unicode_fold)
# --------------------------------------------------------------------------

# Characters the Rust version folds to a plain space. Kept explicit rather than
# delegated to a Unicode category so the two implementations cannot drift.
_NBSP_FAMILY = {
    0x00A0, 0x2007, 0x202F, 0x3000,
}
_CURLY_QUOTES = {
    0x2018, 0x2019, 0x201A, 0x201B,
    0x201C, 0x201D, 0x201E,
    0x2032, 0x2033,
}
_FULLWIDTH_START = 0xFF01
_FULLWIDTH_END = 0xFF5E
_FULLWIDTH_OFFSET = 0xFEE0
_COMBINING_START = 0x0300
_COMBINING_END = 0x036F


def unicode_fold(s: str) -> tuple[str, list[NormalizationStep]]:
    """Fullwidth -> ASCII, drop combining marks, normalise exotic spaces.

    Deliberately minimal and nearly invertible: nothing is removed except
    combining marks, so a fold cannot quietly swallow a character.
    """
    out: list[str] = []
    for ch in s:
        cp = ord(ch)
        if _COMBINING_START <= cp <= _COMBINING_END:
            continue
        if _FULLWIDTH_START <= cp <= _FULLWIDTH_END:
            out.append(chr(cp - _FULLWIDTH_OFFSET))
            continue
        if cp in _NBSP_FAMILY or cp in _CURLY_QUOTES:
            out.append(" ")
            continue
        out.append(ch)
    result = "".join(out)
    steps: list[NormalizationStep] = []
    if result != s:
        steps.append(
            NormalizationStep(
                "unicode_fold", s, result,
                "fullwidth->ascii, combining marks dropped, exotic spaces->space",
            )
        )
    return result, steps


# --------------------------------------------------------------------------
# Step 2 - normalize (port of fruit::normalize)
# --------------------------------------------------------------------------

def normalize(s: str) -> tuple[str, list[NormalizationStep]]:
    """Lowercase, turn punctuation into spaces, collapse runs of whitespace.

    This is the key form used for every comparison. ``Nev Item<crimsonSnapper>``
    and ``nev item (crimson snapper)`` both reduce to the same string, which is
    what makes an exact-key comparison meaningful.
    """
    # NFKC first so ligatures and compatibility forms settle before we decide
    # what counts as a letter. The Rust side does not need this because its
    # inputs are already narrow, but OCR emits fullwidth and ligatured text.
    decomposed = unicodedata.normalize("NFKC", s).lower()
    buf: list[str] = []
    for ch in decomposed:
        buf.append(" " if not ch.isalnum() else ch)
    collapsed = " ".join("".join(buf).split())
    steps: list[NormalizationStep] = []
    if collapsed != s:
        steps.append(
            NormalizationStep(
                "normalize", s, collapsed,
                "lowercase, non-alphanumeric->space, whitespace collapsed",
            )
        )
    return collapsed, steps


# --------------------------------------------------------------------------
# Step 3 - ocr_variant_fold (port of core::canon::ocr_variant_fold)
# --------------------------------------------------------------------------

#: The ONLY confusable classes the app is willing to guess. `i`/`l` genuinely
#: collide, so a folded match is never authoritative on its own.
_CONFUSABLE_MAP = {
    "0": "o", "O": "o", "o": "o",
    "1": "l", "l": "l", "I": "l", "i": "l",
    "5": "s", "S": "s", "s": "s",
}


def ocr_variant_fold(s: str) -> tuple[str, list[NormalizationStep]]:
    """LOSSY confusable folding. Never treat the result as authoritative.

    ``5w0rdf15h`` -> ``swordfish``. Because ``i`` and ``l`` collapse together,
    a folded match must be reported as :attr:`MatchKind.CONFUSABLE` and never as
    an exact match.
    """
    folded = "".join(_CONFUSABLE_MAP.get(c, c) for c in s)
    steps: list[NormalizationStep] = []
    if folded != s:
        steps.append(
            NormalizationStep(
                "ocr_variant_fold", s, folded,
                "lossy digit/letter confusable folding (0/o, 1/l, 5/s); not injective",
            )
        )
    return folded, steps


# --------------------------------------------------------------------------
# Registered OCR correction rules
# --------------------------------------------------------------------------

class OcrCorrection:
    """One explainable, testable OCR repair.

    Attributes
    ----------
    pattern:
        Regex applied to already-normalised text.
    replacement:
        What the matched text becomes.
    reason:
        Why this failure mode is safe to repair. Recorded verbatim into the
        evidence so a reviewer can disagree with it.
    scope:
        ``word`` corrections apply to a whole token; ``phrase`` corrections may
        span whitespace.
    """

    __slots__ = ("name", "pattern", "replacement", "reason", "scope")

    def __init__(self, name: str, pattern: str, replacement: str, reason: str,
                 scope: str = "word") -> None:
        self.name = name
        self.pattern = re.compile(pattern)
        self.replacement = replacement
        self.reason = reason
        self.scope = scope


#: Deliberately small. Every entry must name a concrete failure mode.
#:
#: NOTE ON SCOPE - the safety property of this table:
#:
#: Every rule below is a WHOLE-TOKEN substitution. A token is replaced by
#: another single token, or dropped. No rule may insert or delete a space, and
#: no rule may rewrite part of a token into a KB entity name.
#:
#: That restriction is what makes the table safe. The original contamination
#: happened because a resolver *reinterpreted* unfamiliar text as a familiar
#: entity. A token-level table can tidy `tis erfin` into `tigerfin` so the same
#: concept groups together across OCR variants, but it can never turn
#: `tigerfin` into `tiger`, `crimson snapper` into `snapper`, or `skeletal shark`
#: into `shark` - all of which would be ontology edits disguised as corrections.
#:
#: Anything the game actually named that the KB does not hold therefore stays
#: UNKNOWN_ENTITY, where a human decides. `tests/test_text_normalizer.py` asserts
#: exactly that.
OCR_CORRECTION_RULES: tuple[OcrCorrection, ...] = (
    OcrCorrection(
        name="banner_ne",
        pattern=r"\bn[e3]v\b",
        replacement="new",
        reason="'Nev' is a glyph rendering of 'New'; the drop banner always opens "
               "'New Item'. 'new' is not an entity, so this cannot relabel a fish.",
    ),
    OcrCorrection(
        name="rn_as_m",
        pattern=r"\bcom\b",
        replacement="corn",
        reason="'rn' is routinely read as 'm' at this text size, so 'Corn' renders "
               "'Com'. 'corn' is not an entity either.",
    ),
    OcrCorrection(
        name="sordfish",
        pattern=r"\b[csz](?:sv|v)?ordfish\b",
        replacement="swordfish",
        reason="leading 's' misread as 'c'/'z', sometimes with a spurious 'v' "
               "('csvordfish', 'csordfish'). Maps onto the existing entity "
               "Swordfish; it does not create one.",
    ),
    OcrCorrection(
        name="angel",
        pattern=r"\ban[gsq]elfish\b",
        replacement="angelfish",
        reason="'g' misread as 'q'/'s' in the trailing syllable ('anqelfish', "
               "'anselfish'). Maps onto the existing entity Angelfish.",
    ),
    OcrCorrection(
        name="g0lden",
        pattern=r"\bg[o0]lden\b",
        replacement="golden",
        reason="'o'/'0' confusion in 'Golden' ('g0lden', 'g01den'). Maps onto the "
               "existing entity Golden.",
    ),
    OcrCorrection(
        name="purfer",
        pattern=r"\bpurfer\b",
        replacement="puffer",
        reason="'ff' ligature misread as 'rf'. The token is still 'puffer', not "
               "a new puffer species.",
    ),
    OcrCorrection(
        name="tiqerfin",
        pattern=r"\bt(?:i[gsqz]|l|z)erfin\b",
        replacement="tigerfin",
        reason="'g' misread as 'q'/'s'/'z', 'i' as 'l'. This REPAIRS THE OCR TEXT "
               "ONLY: 'tigerfin' remains absent from the KB, so the finding stays "
               "UNKNOWN_ENTITY and requires a human decision.",
    ),
    OcrCorrection(
        name="item",
        pattern=r"\b(?:i|l|t)tem\b",
        replacement="item",
        reason="'i'/'l' confusion on the banner noun. 'item' is a template word, "
               "not an entity.",
    ),
)


def apply_corrections(
    normalized: str,
    rules: Iterable[OcrCorrection] = OCR_CORRECTION_RULES,
) -> tuple[str, list[NormalizationStep], list[str]]:
    """Apply registered corrections, logging every one that changes anything.

    Returns the corrected text, the steps taken, and the rule names applied. A
    rule that fires is recorded by name so the evidence shows *which* repair
    produced the text it reports.
    """
    steps: list[NormalizationStep] = []
    applied: list[str] = []
    current = normalized
    for rule in rules:
        new = rule.pattern.sub(rule.replacement, current)
        if new != current:
            steps.append(
                NormalizationStep(
                    "correction_rule", current, new,
                    f"{rule.name}: {rule.reason}",
                )
            )
            applied.append(rule.name)
            current = new
    return current, steps, applied


def normalize_pipeline(raw: str) -> tuple[str, list[NormalizationStep], list[str], str]:
    """The full documented chain.

    Returns ``(normalized, steps, applied_rules, folded_key)``.

    ``steps`` describes ONLY the transformations that produced ``normalized``,
    and composes exactly: each step's ``frm`` equals the previous step's ``to``.
    The confusable fold is returned separately as ``folded_key`` and is
    deliberately NOT a step, because it does not produce the reported text - it
    produces a lookup key. Mixing the two made the evidence record's replay
    end at the folded key rather than at the text it reports.

    Raw text is never destroyed: ``steps[0]`` always carries it, so the whole
    chain can be replayed or audited.
    """
    steps: list[NormalizationStep] = [NormalizationStep("raw", raw, raw, "verbatim OCR output")]
    u, s1 = unicode_fold(raw)
    steps += s1
    n, s2 = normalize(u)
    steps += s2
    c, s3, applied = apply_corrections(n)
    steps += s3
    folded, _s4 = ocr_variant_fold(c)
    return c, steps, applied, folded