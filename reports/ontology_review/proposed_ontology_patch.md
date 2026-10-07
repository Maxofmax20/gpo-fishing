# PROPOSED ontology patch

> **THIS IS A PROPOSAL, NOT AN APPLIED CHANGE.**
> Nothing here has been written to `knowledge.rs` or `FISH_VOCAB.txt`.
> A person must review this file and apply it deliberately.

## Validation

- validation passed: **True**
- concepts: 77
- decided: 0
- pending: 77
- proposed new entities: 0

## Proposed new entities

Each row is a human decision that the game names something the knowledge
base does not contain. **None of these exist yet.**

| observed game name | proposed canonical name | suggested id | rows | sessions | days |
|---|---|---|---:|---:|---:|
| _(no NEW_ENTITY decisions yet)_ | | | | | |

## Decided as an EXISTING entity

_none yet_

## Decided as a VARIANT of an existing entity

_none yet_

## Decided UNKNOWN (evidence insufficient)

_none yet_

## How to apply (manually, after review)

1. Add each proposed entity to the knowledge base with its aliases.
2. Append its id to `FISH_VOCAB.txt` **in the intended output-logit order** -
   order is load-bearing, and the deployed `fish_v1` artifact pins a
   `vocab_sha` over it, so changing it invalidates the shipped model.
3. Re-run `cargo test` (`class_map_contract` and `kb_export_contract`) - the
   contract tests will refuse an inconsistent class map.
4. Re-export the KB: `cargo run --example kb_export`.
5. Re-run the OCR audit so the new entities are recognised.

Only after all of the above should a training snapshot be rebuilt.
