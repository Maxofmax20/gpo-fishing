# Ontology review HTTP fixture

A snapshot of `reports/ontology_review/workstation/` (which is gitignored and
regenerated from live data) plus the one evidence image the HTTP test fetches
both ways: the JPEG thumbnail from the sandbox, the full-resolution PNG through
`GPO_DATASET_ROOT`.

It exists so `tests/ontology_web_http.rs` is hermetic: the reviewer-facing
numbers are asserted against THIS file, and cross-checked against the server's
own `counts`, so the contract is 'the API reports exactly the index it serves',
not a magic constant that breaks whenever the operator collects a fish.

Recorded state: 79 concepts, 14 at P0, bands {'P0': 14, 'P1': 2, 'P2': 16, 'P3': 47}.
Those numbers are pinned in `fixture_manifest.json` together with every file's
SHA-256, so replacing this fixture is a visible, deliberate act.

Refresh: `python scripts/gen_ontology_review_fixture.py`
