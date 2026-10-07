/**
 * ONTOLOGY REVIEW — decision logic
 *
 * Everything in this file is pure and has no React or Tauri dependency, for two
 * reasons:
 *
 *  1. **It can be tested directly.** The review flow's correctness does not depend
 *     on rendering; it depends on "which fields does this decision require" and
 *     "what is the queue afterwards". Testing that through a 1000-line component
 *     through jsdom would test the DOM, not the rule.
 *  2. **The rule lives in one place.** The required-field rules here MUST mirror
 *     `validation_fixtures.json`, which is also read by the Python decision store
 *     and by the Rust command. If they drift, a decision the UI thinks is valid is
 *     rejected on save — which is safe, but it wastes the reviewer's time and
 *     teaches them the tool is unreliable.
 *
 * The authoritative gate remains the server. This layer exists to fail fast and to
 * give an immediate, specific message — never to be the only check.
 */

/** The four decisions. Order is increasing commitment: "I can't tell" first. */
export const DECISIONS = [
  { id: "UNKNOWN", key: "1", label: "Unknown", hint: "Evidence insufficient" },
  { id: "EXISTING_ENTITY", key: "2", label: "Existing", hint: "KB already has it" },
  { id: "VARIANT_OF_EXISTING", key: "3", label: "Variant", hint: "Appearance variant" },
  { id: "NEW_ENTITY", key: "4", label: "New", hint: "KB lacks it (proposal)" },
] as const;

export type DecisionId = (typeof DECISIONS)[number]["id"];

export const DECISION_IDS: DecisionId[] = DECISIONS.map((d) => d.id);

/**
 * Keyboard map. `1` selects UNKNOWN deliberately: the cheapest-to-commit answer is
 * first, so a reviewer who has genuinely learned nothing can dismiss a concept in
 * one key and move on, rather than hunting through four buttons.
 */
export const KEY_TO_DECISION: Record<string, DecisionId> = {
  "1": "UNKNOWN",
  "2": "EXISTING_ENTITY",
  "3": "VARIANT_OF_EXISTING",
  "4": "NEW_ENTITY",
};

export const UNKNOWN_REASONS = [
  "OCR artifact",
  "Cannot identify",
  "Insufficient evidence",
  "Non-fish",
  "Other",
] as const;

/** The reviewer's in-progress decision for one concept. */
export type DecisionDraft = {
  decision: DecisionId | null;
  existingEntityId: string;
  proposedEntityName: string;
  variantDescriptor: string;
  notes: string;
  unknownReason: string;
};

/**
 * A draft with nothing chosen.
 *
 * This is what every concept starts from, and it is what makes "no preselection"
 * structural rather than a matter of styling: there is no path that starts a
 * concept with a decision set. `isPristineDraft` is asserted in the tests.
 */
export const EMPTY_DRAFT: DecisionDraft = Object.freeze({
  decision: null,
  existingEntityId: "",
  proposedEntityName: "",
  variantDescriptor: "",
  notes: "",
  unknownReason: "",
});

export function isPristineDraft(d: DecisionDraft): boolean {
  return (
    d.decision === null &&
    d.existingEntityId.trim() === "" &&
    d.proposedEntityName.trim() === "" &&
    d.variantDescriptor.trim() === "" &&
    d.notes.trim() === "" &&
    d.unknownReason.trim() === ""
  );
}

const nonEmpty = (s: string) => s.trim().length > 0;

/**
 * What is still missing before this decision can be saved.
 *
 * Returned as problems rather than a boolean so the UI can tell the reviewer
 * *which* field, instead of disabling a button and leaving them to guess.
 *
 * Mirrors `validate_shape` in `commands_ontology_workstation.rs` and
 * `HumanDecision.validate()` in Python. UNKNOWN deliberately requires nothing: an
 * honest "I don't know" should cost one keystroke, not a form.
 */
export function missingRequirements(d: DecisionDraft): string[] {
  if (d.decision === null) return ["choose a decision"];
  switch (d.decision) {
    case "UNKNOWN":
      return [];
    case "EXISTING_ENTITY":
      return nonEmpty(d.existingEntityId) ? [] : ["choose the existing entity this refers to"];
    case "NEW_ENTITY":
      return nonEmpty(d.proposedEntityName)
        ? []
        : ["enter the proposed canonical name"];
    case "VARIANT_OF_EXISTING": {
      const out: string[] = [];
      if (!nonEmpty(d.existingEntityId)) out.push("choose the entity this is a variant of");
      if (!nonEmpty(d.variantDescriptor)) {
        out.push("describe how it differs (a variant is a claim about the parent)");
      }
      return out;
    }
  }
}

export function canSave(d: DecisionDraft): boolean {
  return missingRequirements(d).length === 0;
}

/**
 * A SUGGESTION for the proposed name. Never applied automatically.
 *
 * The value is derived from the observed game name so the reviewer starts from
 * something close, but `NEW_ENTITY` only saves when the reviewer types their own
 * value into the field. The suggestion is passed as the input's `placeholder`, not
 * its `value` — that distinction is the whole point, and it is asserted in the
 * tests.
 */
export function suggestProposedName(displayName: string): string {
  return displayName.trim().toLowerCase().replace(/\s+/g, " ");
}

/**
 * Turn a draft into the payload the command accepts.
 *
 * Returns null when the draft is incomplete, so no caller can accidentally
 * construct a partial decision. Empty optional strings become `undefined` rather
 * than `""`, because `""` is a *value* in the log and would read as a decision to
 * record an empty name.
 */
export type DecisionPayload = {
  conceptId: string;
  decision: DecisionId;
  existingEntityId?: string;
  proposedEntityName?: string;
  variantDescriptor?: string;
  notes?: string;
  evidenceImageIds: string[];
};

export function buildDecisionPayload(
  conceptId: string,
  draft: DecisionDraft,
  evidenceImageIds: string[],
  maxEvidence = 6,
): DecisionPayload | null {
  if (!canSave(draft) || draft.decision === null) return null;
  const notes = [draft.unknownReason, draft.notes.trim()].filter(Boolean).join(" - ");
  return {
    conceptId,
    decision: draft.decision,
    ...(nonEmpty(draft.existingEntityId) ? { existingEntityId: draft.existingEntityId.trim() } : {}),
    ...(nonEmpty(draft.proposedEntityName)
      ? { proposedEntityName: draft.proposedEntityName.trim() }
      : {}),
    ...(nonEmpty(draft.variantDescriptor)
      ? { variantDescriptor: draft.variantDescriptor.trim() }
      : {}),
    ...(notes ? { notes } : {}),
    evidenceImageIds: evidenceImageIds.slice(0, maxEvidence),
  };
}

// ---------------------------------------------------------------------------
// Queue arithmetic
// ---------------------------------------------------------------------------

/**
 * The unresolved queue with `conceptId` removed — the optimistic update applied
 * the instant the reviewer confirms.
 *
 * Pure so the optimistic step and its rollback are exact inverses. A rollback that
 * could not reproduce the original list would strand the reviewer on a concept they
 * had already passed.
 */
export function queueAfterDecision(queue: string[], conceptId: string): string[] {
  return queue.filter((id) => id !== conceptId);
}

/**
 * Where to go after deciding `conceptId`.
 *
 * Prefers the server's `nextConceptId`, because the server knows about decisions
 * recorded outside this window (the CLI, another window). Falls back to the first
 * locally-unresolved concept. Returns undefined only when nothing is left.
 */
export function resolveNextConcept(
  serverNext: string | null | undefined,
  queue: string[],
  conceptId: string,
): string | undefined {
  if (serverNext && serverNext !== conceptId) return serverNext;
  const local = queueAfterDecision(queue, conceptId);
  return local.find((id) => id !== conceptId);
}

/** The concepts to review for a band, in the server's order. */
export function queueForBand(
  queue: string[],
  band: string,
  conceptBand: Map<string, string>,
): string[] {
  if (band === "ALL") return queue;
  return queue.filter((id) => conceptBand.get(id) === band);
}

/**
 * The first concept of a band — what the band's button jumps to.
 *
 * Returns undefined when the band is empty, so the UI can say "nothing pending"
 * rather than jumping somewhere unrelated.
 */
export function firstOfBand(
  queue: string[],
  band: string,
  conceptBand: Map<string, string>,
): string | undefined {
  if (band === "ALL") return queue[0];
  return queue.find((id) => conceptBand.get(id) === band);
}

/** Progress counters, derived rather than accumulated so they cannot drift. */
export type Progress = {
  total: number;
  decided: number;
  pending: number;
  byDecision: Record<string, number>;
  /** remaining per band */
  remaining: Record<string, number>;
  /** decided per band */
  donePerBand: Record<string, number>;
  /** 0-10, for the block-character progress bar */
  blocks: number;
};

export function computeProgress(
  total: number,
  decided: number,
  byDecision: Record<string, number>,
  remaining: Record<string, number>,
): Progress {
  const donePerBand: Record<string, number> = {};
  for (const [band, rem] of Object.entries(remaining)) {
    donePerBand[band] = Math.max(0, total - rem);
  }
  return {
    total,
    decided,
    pending: Math.max(0, total - decided),
    byDecision,
    remaining,
    donePerBand,
    blocks: total > 0 ? Math.round((decided / total) * 10) : 0,
  };
}

// ---------------------------------------------------------------------------
// Keyboard routing
// ---------------------------------------------------------------------------

export type KeyAction =
  | { kind: "select"; decision: DecisionId }
  | { kind: "confirm" }
  | { kind: "next" }
  | { kind: "previous" }
  | { kind: "viewer" }
  | { kind: "undo" }
  | { kind: "close" }
  | { kind: "none" };

/**
 * Map a keystroke to an action.
 *
 * Returns "none" when the key is not a shortcut, and the caller must then leave it
 * to the browser. It deliberately does NOT decode modifier combinations: a reviewer
 * pressing Ctrl+1 to switch browser tabs must not fire a decision.
 *
 * `isTextEntry` is checked separately by the caller, because a `1` typed into the
 * notes field must produce a "1" and not select UNKNOWN.
 */
export function keyToAction(key: string, hasModifier: boolean): KeyAction {
  if (hasModifier) return { kind: "none" };
  if (key in KEY_TO_DECISION) return { kind: "select", decision: KEY_TO_DECISION[key] };
  switch (key) {
    case "Enter":
      return { kind: "confirm" };
    case "n":
    case "N":
      return { kind: "next" };
    case "p":
    case "P":
      return { kind: "previous" };
    case " ":
      return { kind: "viewer" };
    case "u":
    case "U":
      return { kind: "undo" };
    case "Escape":
      return { kind: "close" };
    default:
      return { kind: "none" };
  }
}

/** True when a keystroke belongs to a text field and must not be intercepted. */
export function isTextEntry(el: EventTarget | null): boolean {
  const t = el as HTMLElement | null;
  if (!t || typeof t.tagName !== "string") return false;
  return (
    t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT" || t.isContentEditable === true
  );
}

// ---------------------------------------------------------------------------
// Viewer navigation
// ---------------------------------------------------------------------------

/**
 * Step through evidence, wrapping at both ends.
 *
 * Wrapping rather than clamping: a reviewer comparing the last screenshot against
 * the first should not have to remember which arrow key stops moving.
 */
export function stepIndex(current: number, delta: number, length: number): number {
  if (length <= 0) return 0;
  return (((current + delta) % length) + length) % length;
}