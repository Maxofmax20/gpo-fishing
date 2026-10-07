"""Tests for evidence selection, decision validation, and the safety guarantees.

The safety group is the important one. Ontology proposals must never become
production ontology without a person applying them, and no machine may be recorded
as a decision maker. Those are enforced here by inspecting the source, because the
guarantee is about the ABSENCE of a capability rather than its behaviour.
"""

from __future__ import annotations

import ast
import inspect
import json
import re
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from gpo_ontology_review import (  # noqa: E402
    concept_extract,
    decisions,
    evidence_select,
    patch,
    pipeline,
    reporting,
    schemas,
)
from gpo_ontology_review.decisions import (  # noqa: E402
    DecisionRejected,
    DecisionStore,
    decision_counts,
    proposals,
    training_ready_counts,
)
from gpo_ontology_review.evidence_select import Candidate, select_representative  # noqa: E402
from gpo_ontology_review.patch import suggested_id, validate  # noqa: E402
from gpo_ontology_review.schemas import (  # noqa: E402
    ALLOWED_DECISION_MAKERS,
    DecisionType,
    HumanDecision,
    OntologyConcept,
    concept_id_for,
)

PKG = Path(__file__).resolve().parents[1]


# --------------------------------------------------------------------------
# Deterministic representative selection
# --------------------------------------------------------------------------

def _cands(n: int, sessions: int, days: int = 1, conf: float = 0.8) -> list[Candidate]:
    out = []
    for i in range(n):
        out.append(Candidate(
            image_id=f"img{i:03d}",
            session_id=f"session_2026-10-{days:02d}_{100000 + i:06d}_aaaaaaa",
            day=f"2026-10-{days:02d}",
            raw_text="Nev Item <crimson snapper*",
            concept_name="crimson snapper",
            stored_entity_id="fish:snapper",
            ocr_confidence=conf - i * 0.001,
            timestamp_ms=i,
        ))
    return out


def test_selection_prefers_different_sessions_over_duplicate_frames():
    """Consecutive frames of one catch prove nothing about recurrence."""
    cands = []
    for s in range(6):
        for f in range(10):
            cands.append(Candidate(
                image_id=f"s{s}f{f:02d}", session_id=f"session_2026-10-0{s+2}_100000_aaaa",
                day=f"2026-10-0{s+2}", raw_text="x", concept_name="crimson snapper",
                stored_entity_id="fish:snapper", ocr_confidence=0.9 - f * 0.01,
                timestamp_ms=f,
            ))
    sel = select_representative(cands, limit=6)
    assert len(sel) == 6
    # Six images must span six sessions, not one.
    assert len({e["session_id"] for e in sel}) == 6, sel
    assert sel[0]["selected_because"] == "highest OCR confidence"


def test_selection_covers_distinct_days_labels_and_variants():
    cands = [
        Candidate("a", "session_2026-10-02_1_aa", "2026-10-02", "Nev Item <crimson",
                  "crimson snapper", "fish:snapper", 0.80, 1),
        Candidate("b", "session_2026-10-03_1_bb", "2026-10-03", "New Item <Crimson Snapper",
                  "crimson snapper", "fish:squid", 0.90, 2),
        Candidate("c", "session_2026-10-04_1_cc", "2026-10-04", "New Item crimsonSnapper",
                  "crimsonsnapper", "fish:crimson", 0.95, 3),
        Candidate("d", "session_2026-10-05_1_dd", "2026-10-05", "NEV ITEM CRIMSON SNAPPER",
                  "crimson snapper", "fish:shark", 0.85, 4),
    ]
    sel = select_representative(cands, limit=4)
    assert len(sel) == 4
    assert len({e["day"] for e in sel}) == 4
    assert len({e["stored_entity_id"] for e in sel}) == 4
    # At least two distinct OCR renderings must be represented.
    assert len({e["concept_name"] for e in sel}) >= 2, sel
    # Highest confidence first, always.
    confs = [e["ocr_confidence"] for e in sel]
    assert confs == sorted(confs, reverse=True), confs
    # Every pick states why.
    assert all(e["selected_because"] for e in sel)


def test_selection_is_deterministic_under_input_reordering():
    """A pure function of the input, not of its arrival order."""
    cands = _cands(40, 5, 3)
    a = select_representative(list(cands), limit=6)
    b = select_representative(list(reversed(cands)), limit=6)
    assert [x["image_id"] for x in a] == [x["image_id"] for x in b]
    c = select_representative(sorted(cands, key=lambda x: x.image_id), limit=6)
    assert [x["image_id"] for x in a] == [x["image_id"] for x in c]


def test_selection_handles_no_candidates():
    assert select_representative([], limit=6) == []


def test_selection_never_exceeds_the_limit():
    assert len(select_representative(_cands(50, 5), limit=4)) == 4


# --------------------------------------------------------------------------
# Human decision validation
# --------------------------------------------------------------------------

def _d(**kw) -> HumanDecision:
    base = dict(concept_id="concept:x", decision="UNKNOWN", decision_maker="human")
    base.update(kw)
    return HumanDecision(**base)


def test_unknown_needs_nothing_but_a_concept():
    assert _d().validate() == []


def test_existing_entity_requires_an_entity_id():
    problems = _d(decision="EXISTING_ENTITY").validate()
    assert any("existing_entity_id" in p for p in problems), problems


def test_new_entity_requires_a_proposed_name():
    problems = _d(decision="NEW_ENTITY").validate()
    assert any("proposed_entity_name" in p for p in problems), problems


def test_variant_requires_a_descriptor():
    """Calling something a variant is a claim about the base entity, so the
    human has to say which variant."""
    problems = _d(decision="VARIANT_OF_EXISTING", existing_entity_id="fish:swordfish").validate()
    assert any("variant_descriptor" in p for p in problems), problems


def test_an_invalid_decision_type_is_rejected():
    assert _d(decision="PROBABLY_A_FISH").validate()


def test_only_human_may_be_a_decision_maker():
    assert ALLOWED_DECISION_MAKERS == frozenset({"human"})
    for maker in ("collector", "ocr", "ram++", "model", "auto"):
        assert _d(decision_maker=maker).validate(), f"{maker} was accepted"


def test_machine_decisions_are_refused_on_write():
    with tempfile.TemporaryDirectory() as td:
        store = DecisionStore(Path(td) / "d.jsonl")
        for maker in decisions.FORBIDDEN_MAKERS:
            try:
                store.record(_d(decision_maker=maker))
                raise AssertionError(f"{maker} was recorded as a decision maker")
            except DecisionRejected:
                pass
        assert store.all() == [], "a rejected decision must not reach the log"


def test_rejection_is_never_swallowed():
    with tempfile.TemporaryDirectory() as td:
        store = DecisionStore(Path(td) / "d.jsonl")
        try:
            store.record(_d(decision="EXISTING_ENTITY"))
            raise AssertionError("invalid decision was accepted")
        except DecisionRejected as exc:
            assert "existing_entity_id" in str(exc)


# --------------------------------------------------------------------------
# Decision store semantics
# --------------------------------------------------------------------------

def test_the_log_is_append_only_and_the_last_decision_wins():
    with tempfile.TemporaryDirectory() as td:
        store = DecisionStore(Path(td) / "d.jsonl")
        store.record(_d(decision="EXISTING_ENTITY", existing_entity_id="fish:shark",
                        timestamp="t1", notes="first look"))
        store.record(_d(decision="UNKNOWN", timestamp="t2", notes="changed my mind"))
        assert len(store.all()) == 2, "the earlier decision must remain on disk"
        assert store.current()["concept:x"].decision == "UNKNOWN"
        assert len(store.history_for("concept:x")) == 2


def test_a_corrupt_decision_log_is_reported_not_skipped():
    with tempfile.TemporaryDirectory() as td:
        p = Path(td) / "d.jsonl"
        p.write_text('{"concept_id":"a","decision":"UNKNOWN","decision_maker":"human"}\n'
                     "{ this is not json\n", encoding="utf-8")
        store = DecisionStore(p)
        try:
            store.all()
            raise AssertionError("a corrupt decision log must be reported")
        except DecisionRejected as exc:
            assert "append-only" in str(exc)


def test_proposals_only_include_new_entity_decisions():
    with tempfile.TemporaryDirectory() as td:
        store = DecisionStore(Path(td) / "d.jsonl")
        store.record(_d(concept_id="c:a", decision="NEW_ENTITY",
                        proposed_entity_name="Tigerfin", evidence_image_ids=["x"]))
        store.record(_d(concept_id="c:b", decision="EXISTING_ENTITY",
                        existing_entity_id="fish:swordfish"))
        store.record(_d(concept_id="c:c", decision="UNKNOWN"))
        props = proposals(store.all())
        assert len(props) == 1
        assert props[0]["proposed_entity_name"] == "Tigerfin"
        counts = decision_counts(store.all())
        assert counts == {"EXISTING_ENTITY": 1, "NEW_ENTITY": 1, "UNKNOWN": 1}


def test_apply_to_stamps_decisions_onto_concepts():
    with tempfile.TemporaryDirectory() as td:
        store = DecisionStore(Path(td) / "d.jsonl")
        store.record(_d(concept_id=concept_id_for("swordfish"),
                        decision="EXISTING_ENTITY", existing_entity_id="fish:swordfish"))
        cs = [OntologyConcept(concept_id=concept_id_for("swordfish"),
                              display_name="swordfish", normalized_name="swordfish",
                              group_key="swordfish")]
        assert store.apply_to(cs) == 1
        assert cs[0].human_decision == "EXISTING_ENTITY"


# --------------------------------------------------------------------------
# Training consequences: HUMAN decisions only, never stored labels
# --------------------------------------------------------------------------

def test_training_counts_use_human_decisions_not_stored_labels():
    """The number that matters for training must NOT be the collector's count."""
    c = OntologyConcept(
        concept_id=concept_id_for("tigerfin"), display_name="tigerfin",
        normalized_name="tigerfin", group_key="tigerfin",
        occurrences=65, stored_labels={"fish:snapper": 60}, total_labeled_rows=60,
        sessions=["a"], calendar_days=["2026-10-02"],
    )
    decided = {
        c.concept_id: HumanDecision(concept_id=c.concept_id, decision="NEW_ENTITY",
                                    proposed_entity_name="Tigerfin")
    }
    out = training_ready_counts([c], decided)
    ent = out["entities"][0]
    assert ent["entity_id"] == "proposed:Tigerfin"
    assert ent["human_confirmed_rows"] == 65
    assert "fish:snapper" not in ent["entity_id"]
    assert "stored" in out["note"].lower()


def test_undecided_concepts_contribute_nothing_to_training_counts():
    """Until a human rules, a concept is not training evidence."""
    c = OntologyConcept(
        concept_id="concept:tigerfin", display_name="tigerfin",
        normalized_name="tigerfin", group_key="tigerfin",
        occurrences=65, stored_labels={"fish:snapper": 60},
    )
    out = training_ready_counts([c], {})
    assert out["entities"] == []


def test_variants_count_toward_their_base_entity():
    c = OntologyConcept(
        concept_id="concept:golden-swordfish", display_name="golden swordfish",
        normalized_name="golden swordfish", group_key="goldenswordfish",
        occurrences=12, sessions=["a"], calendar_days=["2026-10-02"],
    )
    d = {c.concept_id: HumanDecision(
        concept_id=c.concept_id, decision="VARIANT_OF_EXISTING",
        existing_entity_id="fish:swordfish", variant_descriptor="golden colour")}
    out = training_ready_counts([c], d)
    assert out["entities"][0]["entity_id"] == "fish:swordfish"
    assert out["entities"][0]["kind"] == "variant_of"


# --------------------------------------------------------------------------
# Validation of decisions against the real KB
# --------------------------------------------------------------------------

def _kb_payload() -> dict:
    root = Path(__file__).resolve().parents[3]
    p = root / "ml" / "gpo_ocr_audit" / "kb_export.json"
    if not p.is_file():
        return {"kb_version": 1, "fish_vocab": [], "entities": []}
    return json.loads(p.read_text(encoding="utf-8"))


def _concept(cid: str, name: str) -> OntologyConcept:
    return OntologyConcept(concept_id=cid, display_name=name, normalized_name=name,
                           group_key=name.replace(" ", ""), occurrences=10)


def test_a_proposal_colliding_with_an_existing_canonical_name_is_an_error():
    kb = _kb_payload()
    if not kb["entities"]:
        print("SKIP: kb_export.json absent")
        return
    with tempfile.TemporaryDirectory() as td:
        store = DecisionStore(Path(td) / "d.jsonl")
        store.record(_d(concept_id="c:a", decision="NEW_ENTITY",
                        proposed_entity_name="Swordfish"))
        rep = validate([_concept("c:a", "swordfish")], store, kb)
        assert not rep.ok
        assert any("ALREADY has that canonical name" in e for e in rep.errors), rep.errors


def test_a_proposal_colliding_with_an_existing_alias_is_an_error():
    kb = _kb_payload()
    aliases = {a.lower(): e["id"] for e in kb["entities"]
               for a in (e.get("aliases") or []) + (e.get("ocr_aliases") or [])}
    if not aliases:
        print("SKIP: KB has no aliases to collide with")
        return
    alias, owner = sorted(aliases.items())[0]
    with tempfile.TemporaryDirectory() as td:
        store = DecisionStore(Path(td) / "d.jsonl")
        store.record(_d(concept_id="c:a", decision="NEW_ENTITY",
                        proposed_entity_name=alias.title()))
        rep = validate([_concept("c:a", alias)], store, kb)
        assert not rep.ok
        assert any("collides with an existing ALIAS" in e for e in rep.errors), rep.errors


def test_two_concepts_cannot_both_propose_the_same_name():
    kb = _kb_payload()
    with tempfile.TemporaryDirectory() as td:
        store = DecisionStore(Path(td) / "d.jsonl")
        store.record(_d(concept_id="c:a", decision="NEW_ENTITY", proposed_entity_name="Mystery Fish"))
        store.record(_d(concept_id="c:b", decision="NEW_ENTITY", proposed_entity_name="Mystery Fish"))
        rep = validate([_concept("c:a", "x"), _concept("c:b", "y")], store, kb)
        assert any("same name" in e for e in rep.errors), rep.errors


def test_a_decision_for_a_nonexistent_concept_is_an_error():
    with tempfile.TemporaryDirectory() as td:
        store = DecisionStore(Path(td) / "d.jsonl")
        store.record(_d(concept_id="c:ghost", decision="UNKNOWN"))
        rep = validate([_concept("c:real", "real")], store, _kb_payload())
        assert any("unknown concept" in e for e in rep.errors), rep.errors


def test_existing_entity_must_name_a_real_kb_entity():
    kb = _kb_payload()
    if not kb["entities"]:
        print("SKIP: kb_export.json absent")
        return
    with tempfile.TemporaryDirectory() as td:
        store = DecisionStore(Path(td) / "d.jsonl")
        store.record(_d(concept_id="c:a", decision="EXISTING_ENTITY",
                        existing_entity_id="fish:definitely-not-real"))
        rep = validate([_concept("c:a", "a")], store, kb)
        assert any("is not a KB entity" in e for e in rep.errors), rep.errors


def test_pending_concepts_are_reported():
    with tempfile.TemporaryDirectory() as td:
        store = DecisionStore(Path(td) / "d.jsonl")
        cs = [_concept("c:a", "a"), _concept("c:b", "b"), _concept("c:c", "c")]
        store.record(_d(concept_id="c:a", decision="UNKNOWN"))
        rep = validate(cs, store, _kb_payload())
        assert rep.pending == ["c:b", "c:c"]
        assert rep.counts["concepts_pending"] == 2


def test_suggested_ids_are_wellformed_and_unique_per_name():
    assert suggested_id("Tigerfin") == "fish:tigerfin"
    assert suggested_id("Blue-Lip Grouper") == "fish:blue-lip-grouper"
    assert suggested_id("  Crimson  Snapper  ") == "fish:crimson-snapper"
    assert patch.ID_RE.match(suggested_id("Crimson Snapper"))


# --------------------------------------------------------------------------
# Patch export: a document, never a mutation
# --------------------------------------------------------------------------

def test_export_writes_a_patch_and_applies_nothing():
    kb = _kb_payload()
    with tempfile.TemporaryDirectory() as td:
        out = Path(td)
        store = DecisionStore(out / "d.jsonl")
        store.record(_d(concept_id="c:tf", decision="NEW_ENTITY",
                        proposed_entity_name="Tigerfin", evidence_image_ids=["i1", "i2"]))
        cs = [_concept("c:tf", "tigerfin")]
        rep = validate(cs, store, kb)
        res = patch.export_patch(cs, store, kb, rep, out)
        assert res["applied"] is False
        assert res["proposed_count"] == 1
        payload = json.loads((out / "proposed_ontology.json").read_text(encoding="utf-8"))
        assert payload["APPLIED"] is False
        assert payload["proposed_new_entities"][0]["suggested_id"] == "fish:tigerfin"
        md = (out / "proposed_ontology_patch.md").read_text(encoding="utf-8")
        assert "NOT AN APPLIED CHANGE" in md
        assert "fish:tigerfin" in md


def test_export_does_not_touch_the_kb_or_the_vocabulary():
    """The patch must be a file, not an edit. Assert the source has no write to
    either target, in any form."""
    for mod in (patch, decisions, pipeline, concept_extract, reporting):
        src = inspect.getsource(mod)
        for target in ("knowledge.rs", "FISH_VOCAB.txt"):
            for line in src.splitlines():
                if target in line and any(
                    w in line for w in ('"w"', "'w'", "write_text", "write(", "open(")
                ):
                    raise AssertionError(f"{mod.__name__} may write {target}: {line.strip()}")


# --------------------------------------------------------------------------
# Package-level safety: no control-path imports, no write capability
# --------------------------------------------------------------------------

#: Importing any of these would mean the ontology toolchain can reach the game or
#: the production ML control path.
FORBIDDEN_IMPORTS = {"controller", "macro", "input", "keyboard", "mouse",
                     "tauri", "registry", "ml_dataset", "readiness"}


def _imports(path: Path) -> set[str]:
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    out: set[str] = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            out |= {a.name.split(".")[0] for a in node.names}
        elif isinstance(node, ast.ImportFrom) and node.module and node.level == 0:
            out.add(node.module.split(".")[0])
            out.add(node.module)
    return out


def test_no_module_reaches_the_control_path_or_the_production_registries():
    for path in sorted(PKG.glob("*.py")):
        bad = _imports(path) & FORBIDDEN_IMPORTS
        assert not bad, f"{path.name} imports {bad}"


def test_no_module_can_mark_training_eligible():
    for path in sorted(PKG.glob("*.py")):
        src = path.read_text(encoding="utf-8")
        assert "training_eligible" not in src or "must" in src.lower() or \
            "not" in src.lower(), (
            f"{path.name} mentions training_eligible; it must not be able to set it"
        )
        for pat in ("training_eligible = True", '"training_eligible": True',
                    "set_training_eligible"):
            assert pat not in src, f"{path.name} contains {pat!r}"


def test_no_module_writes_an_absolute_path():
    """Every write must be relative to a caller-supplied output directory."""
    for path in sorted(PKG.glob("*.py")):
        tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
        for node in ast.walk(tree):
            if isinstance(node, ast.Call) and getattr(node.func, "attr", None) in (
                "write_text", "write_bytes", "open", "unlink", "save"
            ):
                for arg in node.args:
                    lit = getattr(arg, "value", None)
                    if isinstance(lit, str) and (lit.startswith(("/", "\\", "C:")) or
                                                 ":" in lit[:3]):
                        raise AssertionError(
                            f"{path.name} writes to an absolute path {lit!r}"
                        )


def test_no_kb_mutator_is_referenced():
    for path in sorted(PKG.glob("*.py")):
        src = path.read_text(encoding="utf-8")
        for forbidden in ("merge_import", "validate_import", "create_entity",
                          "add_entity", "upsert", "KnowledgeBase::bundled"):
            assert f".{forbidden}(" not in src, (
                f"{path.name} calls {forbidden}(); ontology proposals must never "
                f"mutate the knowledge base"
            )


def test_concept_id_is_stable_and_readable():
    assert concept_id_for("crimson snapper") == "concept:crimson-snapper"
    assert concept_id_for("blue lip grouper") == "concept:blue-lip-grouper"
    # Stable: the same name always yields the same id, so a decision recorded
    # against it keeps applying across rebuilds.
    assert concept_id_for("tigerfin") == concept_id_for("tigerfin")


def test_decision_types_are_exactly_the_four_supported():
    assert {d.value for d in DecisionType} == {
        "EXISTING_ENTITY", "NEW_ENTITY", "VARIANT_OF_EXISTING", "UNKNOWN"
    }


def test_a_proposal_cannot_become_production_without_the_step_in_between():
    """The workflow is observed -> human proposal -> REPORT -> approval -> update.

    This asserts the gap exists: an explicit `export` step writes a document, and
    the CLI deliberately offers NO `apply` step.
    """
    import gpo_ontology_review.__main__ as cli

    # Module-level source, not just the `main` function - `export_patch` is called
    # from cmd_export, which lives outside main().
    src = inspect.getsource(cli)
    assert "export_patch(" in src, "an export step must write the patch document"

    subparsers = re.findall(r'add_parser\(\s*"([a-z_]+)"', src)
    assert "export" in subparsers, subparsers
    assert "apply" not in subparsers, (
        f"the CLI must not offer an apply step (subcommands: {subparsers}); "
        f"applying an ontology patch is a manual, reviewed act"
    )

    # And the patch writer declares itself unapplied.
    from gpo_ontology_review.patch import export_patch
    assert "applied" in inspect.getsource(export_patch)
