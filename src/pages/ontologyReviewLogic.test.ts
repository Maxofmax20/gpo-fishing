/**
 * ONTOLOGY REVIEW — logic tests
 *
 * These cover the decision rules in isolation: which fields each decision requires,
 * what the payload looks like, how the queue advances, and how keystrokes route.
 *
 * The point is not coverage for its own sake. Each test below corresponds to a way
 * this screen could mislead a human reviewer — silently filing a decision under the
 * wrong fish, accepting a name they never typed, advancing past an unsaved decision —
 * and those are the failures that would corrupt the ontology review, which is the
 * one thing this whole pipeline exists to get right.
 */

import { describe, expect, it } from "vitest";
import {
  DECISIONS,
  DECISION_IDS,
  EMPTY_DRAFT,
  KEY_TO_DECISION,
  buildDecisionPayload,
  canSave,
  computeProgress,
  firstOfBand,
  isPristineDraft,
  isTextEntry,
  keyToAction,
  missingRequirements,
  queueAfterDecision,
  queueForBand,
  resolveNextConcept,
  stepIndex,
  suggestProposedName,
  type DecisionDraft,
} from "./ontologyReviewLogic";

const draft = (p: Partial<DecisionDraft>): DecisionDraft => ({ ...EMPTY_DRAFT, ...p });

// ===========================================================================
// No preselection — the invariant the whole screen rests on
// ===========================================================================

describe("no preselection", () => {
  it("a fresh draft has no decision and no fields filled", () => {
    expect(isPristineDraft(EMPTY_DRAFT)).toBe(true);
    expect(EMPTY_DRAFT.decision).toBeNull();
    for (const v of Object.values(EMPTY_DRAFT)) {
      if (typeof v === "string") expect(v).toBe("");
    }
  });

  it("EMPTY_DRAFT is frozen, so nothing can mutate the shared default", () => {
    // If a caller mutated the module-level constant, every later concept would
    // start with the previous concept's decision still attached.
    expect(Object.isFrozen(EMPTY_DRAFT)).toBe(true);
  });

  it("no decision is saveable until the human chooses one", () => {
    expect(canSave(EMPTY_DRAFT)).toBe(false);
    expect(missingRequirements(EMPTY_DRAFT)).toEqual(["choose a decision"]);
  });

  it("KB candidates never become a decision by being visible", () => {
    // The panel may show `fish:snapper` as a candidate; the draft must still be
    // empty until the reviewer presses a key.
    const withCandidateInSight = draft({ notes: "fish:snapper is a candidate" });
    expect(withCandidateInSight.decision).toBeNull();
    expect(canSave(withCandidateInSight)).toBe(false);
  });

  it("the proposed name is a suggestion, never a value", () => {
    // The UI puts this in the input's placeholder. It must not be a legal way to
    // satisfy the requirement by itself.
    const suggestion = suggestProposedName("Exotic Tigerfin");
    expect(suggestion).toBe("exotic tigerfin");
    const withSuggestionOnly = draft({ decision: "NEW_ENTITY" });
    expect(suggestion).not.toBe(withSuggestionOnly.proposedEntityName);
    expect(canSave(withSuggestionOnly)).toBe(false);
  });
});

// ===========================================================================
// Required-field validation
// ===========================================================================

describe("required fields per decision", () => {
  it("UNKNOWN requires nothing — one keystroke must be enough", () => {
    expect(canSave(draft({ decision: "UNKNOWN" }))).toBe(true);
    expect(missingRequirements(draft({ decision: "UNKNOWN" }))).toEqual([]);
  });

  it("EXISTING_ENTITY requires an entity the human picked", () => {
    expect(canSave(draft({ decision: "EXISTING_ENTITY" }))).toBe(false);
    expect(
      missingRequirements(draft({ decision: "EXISTING_ENTITY" }))[0],
    ).toMatch(/existing entity/);
    expect(
      canSave(draft({ decision: "EXISTING_ENTITY", existingEntityId: "fish:shark" })),
    ).toBe(true);
  });

  it("NEW_ENTITY requires a human-written name", () => {
    expect(canSave(draft({ decision: "NEW_ENTITY" }))).toBe(false);
    expect(canSave(draft({ decision: "NEW_ENTITY", proposedEntityName: "Tigerfin" }))).toBe(
      true,
    );
  });

  it("VARIANT_OF_EXISTING needs BOTH the parent and the descriptor", () => {
    // Either alone is a claim that cannot be checked: a parent with no difference
    // is just EXISTING_ENTITY, and a difference with no parent names nothing.
    expect(canSave(draft({ decision: "VARIANT_OF_EXISTING" }))).toBe(false);
    expect(
      canSave(draft({ decision: "VARIANT_OF_EXISTING", existingEntityId: "fish:swordfish" })),
    ).toBe(false);
    expect(
      canSave(draft({ decision: "VARIANT_OF_EXISTING", variantDescriptor: "golden" })),
    ).toBe(false);
    expect(
      canSave(
        draft({
          decision: "VARIANT_OF_EXISTING",
          existingEntityId: "fish:swordfish",
          variantDescriptor: "golden",
        }),
      ),
    ).toBe(true);
  });

  it("whitespace does not count as a filled field", () => {
    expect(canSave(draft({ decision: "EXISTING_ENTITY", existingEntityId: "   " }))).toBe(false);
    expect(canSave(draft({ decision: "NEW_ENTITY", proposedEntityName: "\t\n" }))).toBe(false);
  });

  it("every decision kind has a rule", () => {
    for (const id of DECISION_IDS) {
      const bare = missingRequirements(draft({ decision: id }));
      const filled = canSave(
        draft({
          decision: id,
          existingEntityId: "fish:shark",
          proposedEntityName: "Thing",
          variantDescriptor: "thing",
        }),
      );
      expect(typeof bare.length, `${id} must have a defined rule`).toBe("number");
      expect(filled, `${id} must become saveable once fully filled`).toBe(true);
    }
  });

  it("there are exactly four decisions", () => {
    // Adding a fifth silently would widen the ontology vocabulary without anyone
    // deciding to.
    expect(DECISION_IDS).toEqual([
      "UNKNOWN",
      "EXISTING_ENTITY",
      "VARIANT_OF_EXISTING",
      "NEW_ENTITY",
    ]);
  });
});

// ===========================================================================
// Payload construction
// ===========================================================================

describe("decision payload", () => {
  const ev = ["aaaaaaaaaaaaaaa1", "aaaaaaaaaaaaaaa2", "aaaaaaaaaaaaaaa3"];

  it("returns null for an incomplete draft, so no partial decision can be built", () => {
    expect(buildDecisionPayload("concept:a", EMPTY_DRAFT, ev)).toBeNull();
    expect(
      buildDecisionPayload("concept:a", draft({ decision: "EXISTING_ENTITY" }), ev),
    ).toBeNull();
  });

  it("omits empty optionals rather than sending empty strings", () => {
    const p = buildDecisionPayload("concept:a", draft({ decision: "UNKNOWN" }), ev);
    expect(p).not.toBeNull();
    expect(p!).not.toHaveProperty("existingEntityId");
    expect(p!).not.toHaveProperty("proposedEntityName");
    expect(p!).not.toHaveProperty("variantDescriptor");
  });

  it("trims the fields it sends", () => {
    const p = buildDecisionPayload(
      "concept:a",
      draft({ decision: "NEW_ENTITY", proposedEntityName: "  Tigerfin  " }),
      ev,
    );
    expect(p!.proposedEntityName).toBe("Tigerfin");
  });

  it("folds the UNKNOWN reason into the notes, prefixed so it reads first", () => {
    const p = buildDecisionPayload(
      "concept:a",
      draft({ decision: "UNKNOWN", unknownReason: "OCR artifact", notes: "partly occluded" }),
      ev,
    );
    expect(p!.notes).toBe("OCR artifact - partly occluded");
  });

  it("does not emit an empty notes field", () => {
    const p = buildDecisionPayload("concept:a", draft({ decision: "UNKNOWN" }), ev);
    expect(p).not.toHaveProperty("notes");
  });

  it("caps how much evidence is attached", () => {
    const many = Array.from({ length: 30 }, (_, i) => `img${i}`);
    const p = buildDecisionPayload("concept:a", draft({ decision: "UNKNOWN" }), many);
    expect(p!.evidenceImageIds).toHaveLength(6);
  });

  it("the payload carries no decision-maker field", () => {
    // Attribution is fixed server-side to `human`. If the client could send one, a
    // caller could file a decision as `collector` or `model`.
    const p = buildDecisionPayload("concept:a", draft({ decision: "UNKNOWN" }), ev);
    expect(Object.keys(p!)).not.toContain("decisionMaker");
    expect(Object.keys(p!)).not.toContain("decision_maker");
  });
});

// ===========================================================================
// Queue arithmetic
// ===========================================================================

describe("queue", () => {
  const q = ["c1", "c2", "c3", "c4"];

  it("removes exactly the decided concept", () => {
    expect(queueAfterDecision(q, "c2")).toEqual(["c1", "c3", "c4"]);
  });

  it("removing a concept never disturbs the relative order of the rest", () => {
    // The optimistic step reorders nothing, so a rollback that restores the saved
    // list reproduces the reviewer's original order exactly. If removal shifted
    // anything, a failed save would shuffle the queue under the reviewer.
    const optimistic = queueAfterDecision(q, "c3");
    expect(optimistic).toEqual(["c1", "c2", "c4"]);
    expect(optimistic.filter((x) => x !== "c3")).toEqual(q.filter((x) => x !== "c3"));
  });

  it("an unknown concept id leaves the queue untouched", () => {
    expect(queueAfterDecision(q, "nope")).toEqual(q);
  });

  it("prefers the server's next concept", () => {
    // The server knows about decisions recorded elsewhere (CLI, another window).
    expect(resolveNextConcept("c4", q, "c1")).toBe("c4");
  });

  it("falls back to the first still-unresolved concept", () => {
    // FIRST unresolved, not "the one after the current position". This mirrors
    // the server, which returns `queue.iter().find(|c| c != current)`. The two
    // implementations must agree, or the UI would jump to a different concept
    // depending on whether the server answered.
    expect(resolveNextConcept(null, q, "c1")).toBe("c2");
    expect(resolveNextConcept(undefined, q, "c4")).toBe("c1");
    // Reviewing out of order returns to the oldest pending item, which is the
    // intended behaviour for a priority queue: the reviewer is sent to work still
    // outstanding, not past it.
    expect(resolveNextConcept(null, q, "c3")).toBe("c1");
  });

  it("never returns the concept just decided", () => {
    // A server that echoed the current id back would trap the reviewer in a loop.
    expect(resolveNextConcept("c1", q, "c1")).toBe("c2");
  });

  it("returns undefined only when nothing remains", () => {
    expect(resolveNextConcept(null, ["c1"], "c1")).toBeUndefined();
    expect(resolveNextConcept(null, [], "c1")).toBeUndefined();
  });
});

describe("band filtering", () => {
  const q = ["a", "b", "c", "d"];
  const bands = new Map([
    ["a", "P0"],
    ["b", "P0"],
    ["c", "P1"],
    ["d", "P3"],
  ]);

  it("filters to the band, preserving server order", () => {
    expect(queueForBand(q, "P0", bands)).toEqual(["a", "b"]);
    expect(queueForBand(q, "P1", bands)).toEqual(["c"]);
  });

  it("ALL returns the queue unchanged", () => {
    expect(queueForBand(q, "ALL", bands)).toBe(q);
  });

  it("a concept with no known band is excluded from every specific band", () => {
    expect(queueForBand(["z"], "P0", bands)).toEqual([]);
    expect(queueForBand(["z"], "ALL", bands)).toEqual(["z"]);
  });

  it("firstOfBand jumps to that band's head", () => {
    expect(firstOfBand(q, "P0", bands)).toBe("a");
    expect(firstOfBand(q, "P3", bands)).toBe("d");
    expect(firstOfBand(q, "ALL", bands)).toBe("a");
  });

  it("firstOfBand is undefined for an empty band, not a fallback concept", () => {
    // Returning some other concept would silently review the wrong fish.
    expect(firstOfBand(q, "P2", bands)).toBeUndefined();
    expect(firstOfBand([], "P0", bands)).toBeUndefined();
  });

  it("concepts with unknown bands are still reachable via ALL", () => {
    const partial = new Map([["a", "P0"]]);
    expect(firstOfBand(q, "P0", partial)).toBe("a");
    expect(queueForBand(q, "ALL", partial)).toEqual(q);
  });
});

// ===========================================================================
// Progress
// ===========================================================================

describe("progress", () => {
  it("derives remaining from total and decided rather than accumulating", () => {
    const p = computeProgress(77, 0, {}, { P0: 11, P1: 3, P2: 16, P3: 47 });
    expect(p.pending).toBe(77);
    expect(p.blocks).toBe(0);
    expect(p.donePerBand.P0).toBe(66);
  });

  it("rounds the bar to whole blocks and never exceeds ten", () => {
    expect(computeProgress(77, 4, {}, {}).blocks).toBe(1);
    expect(computeProgress(77, 77, {}, {}).blocks).toBe(10);
  });

  it("survives a zero total without dividing by zero", () => {
    const p = computeProgress(0, 0, {}, {});
    expect(p.blocks).toBe(0);
    expect(p.pending).toBe(0);
  });

  it("never reports a negative pending when the server over-reports", () => {
    expect(computeProgress(10, 12, {}, {}).pending).toBe(0);
  });
});

// ===========================================================================
// Keyboard routing
// ===========================================================================

describe("keyboard", () => {
  it("maps 1-4 to the four decisions", () => {
    expect(keyToAction("1", false)).toEqual({ kind: "select", decision: "UNKNOWN" });
    expect(keyToAction("2", false)).toEqual({ kind: "select", decision: "EXISTING_ENTITY" });
    expect(keyToAction("3", false)).toEqual({ kind: "select", decision: "VARIANT_OF_EXISTING" });
    expect(keyToAction("4", false)).toEqual({ kind: "select", decision: "NEW_ENTITY" });
  });

  it("every decision button advertises its own key", () => {
    // A shortcut shown in the UI must be the shortcut that fires.
    for (const d of DECISIONS) {
      expect(KEY_TO_DECISION[d.key]).toBe(d.id);
    }
  });

  it("maps the navigation and viewer keys", () => {
    expect(keyToAction("Enter", false).kind).toBe("confirm");
    expect(keyToAction("n", false).kind).toBe("next");
    expect(keyToAction("N", false).kind).toBe("next");
    expect(keyToAction("p", false).kind).toBe("previous");
    expect(keyToAction("P", false).kind).toBe("previous");
    expect(keyToAction(" ", false).kind).toBe("viewer");
    expect(keyToAction("u", false).kind).toBe("undo");
    expect(keyToAction("U", false).kind).toBe("undo");
    expect(keyToAction("Escape", false).kind).toBe("close");
  });

  it("ignores every modified keystroke", () => {
    // Ctrl+1 switches browser tabs. It must not file a decision.
    for (const key of ["1", "2", "3", "4", "Enter", "n", "p", "u", " "]) {
      expect(keyToAction(key, true)).toEqual({ kind: "none" });
    }
  });

  it("leaves ordinary typing alone", () => {
    for (const key of ["a", "z", "5", "?", "Shift", "Tab", "F5"]) {
      expect(keyToAction(key, false).kind).toBe("none");
    }
  });

  it("never routes a digit that is not 1-4 to a decision", () => {
    expect(keyToAction("5", false).kind).toBe("none");
    expect(keyToAction("0", false).kind).toBe("none");
  });

  it("recognises text entry so a typed 1 is a 1, not UNKNOWN", () => {
    const input = { tagName: "INPUT" } as HTMLElement;
    const area = { tagName: "TEXTAREA" } as HTMLElement;
    const editable = { tagName: "DIV", isContentEditable: true } as HTMLElement;
    const button = { tagName: "BUTTON" } as HTMLElement;
    expect(isTextEntry(input)).toBe(true);
    expect(isTextEntry(area)).toBe(true);
    expect(isTextEntry(editable)).toBe(true);
    expect(isTextEntry(button)).toBe(false);
    expect(isTextEntry(null)).toBe(false);
  });
});

// ===========================================================================
// Viewer navigation
// ===========================================================================

describe("evidence viewer stepping", () => {
  it("advances and retreats", () => {
    expect(stepIndex(0, 1, 12)).toBe(1);
    expect(stepIndex(5, -1, 12)).toBe(4);
  });

  it("wraps at both ends", () => {
    // Comparing the last screenshot against the first should not require
    // remembering which arrow stops moving.
    expect(stepIndex(11, 1, 12)).toBe(0);
    expect(stepIndex(0, -1, 12)).toBe(11);
  });

  it("is stable for a single image", () => {
    expect(stepIndex(0, 1, 1)).toBe(0);
    expect(stepIndex(0, -1, 1)).toBe(0);
  });

  it("does not divide by zero on an empty set", () => {
    expect(stepIndex(0, 1, 0)).toBe(0);
  });
});