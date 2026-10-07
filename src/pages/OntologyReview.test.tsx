/**
 * ONTOLOGY REVIEW — component tests
 *
 * These drive the real component through jsdom with the IPC boundary mocked, so
 * they cover the behaviour a reviewer actually experiences: what is on screen, what
 * a keystroke does, what happens when the save fails.
 *
 * The mock stands in for the SERVER, not for the component. The component is the
 * real one; only `api.ontology*` is replaced. That matters because the tests worth
 * writing here are about the component's own promises — no preselection, autosave,
 * honest failure, resume — and mocking the component would test the mock.
 *
 * Fixture data is a faithful SUBSET of the real artefacts: the same field names,
 * the same shapes, the same band distribution. It is labelled as such rather than
 * presented as real data; the real-data verification lives in the Rust end-to-end
 * test against the actual 77-concept index.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { OntologyReview } from "./OntologyReview";
import { api } from "../lib/ipc";
import type {
  FamilyView,
  KbEntityOption,
  StructuralSignals,
  WorkstationConcept,
  WorkstationStatus,
} from "../lib/types";

vi.mock("../lib/ipc", () => ({
  api: {
    ontologyReviewStatus: vi.fn(),
    ontologyReviewConcept: vi.fn(),
    ontologyReviewStructural: vi.fn(),
    ontologyReviewFamily: vi.fn(),
    ontologyReviewKbSearch: vi.fn(),
    ontologyReviewThumb: vi.fn(),
    ontologyReviewFull: vi.fn(),
    ontologyDecide: vi.fn(),
    ontologyUndo: vi.fn(),
    ontologyReviewRead: vi.fn(),
  },
}));

const mocked = vi.mocked(api);

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

const evidence = (n: number) =>
  Array.from({ length: n }, (_, i) => ({
    imageId: `606020203e34beb${i.toString(16)}`,
    sessionId: `session_2026-10-0${(i % 5) + 2}_100000_aaaa`,
    day: `2026-10-0${(i % 5) + 2}`,
    rawOcr: `New Item <Exotic Tigerfin ${i}>`,
    storedEntityId: "fish:tigerfin",
    ocrConfidence: 0.9 - i * 0.01,
    timestampMs: 1700000000000 + i,
    selectedBecause: i === 0 ? "highest OCR confidence" : "different session",
  }));

const concept = (over: Partial<WorkstationConcept> = {}): WorkstationConcept => ({
  rank: 0,
  conceptId: "concept:exotic-tigerfin",
  displayName: "exotic tigerfin",
  normalizedName: "exotic tigerfin",
  occurrences: 262,
  sessionCount: 36,
  dayCount: 5,
  priority: "P0",
  priorityReasons: ["high recurrence"],
  ontologyStatus: "UNKNOWN",
  currentKbCandidates: [],
  baseName: "tigerfin",
  modifiers: ["exotic"],
  familyId: "family:tigerfin",
  storedLabels: [["fish:tigerfin", 100]],
  storedAgreesWithCandidate: 0,
  ocrVariants: ["Exotic Tigerfin"],
  evidence: evidence(12),
  humanDecision: null,
  decisionNote: null,
  ...over,
});

const status = (over: Partial<WorkstationStatus> = {}): WorkstationStatus => ({
  available: true,
  detail: "77 observed game name(s). 0 decided, 77 pending.",
  total: 3,
  counts: { P0: 2, P1: 1 },
  remaining: { P0: 2, P1: 1 },
  decided: 0,
  pending: 3,
  byDecision: {},
  queue: ["concept:exotic-tigerfin", "concept:crimson-snapper", "concept:swordfish"],
  order: ["concept:exotic-tigerfin", "concept:crimson-snapper", "concept:swordfish"],
  ...over,
});

const structural = (): StructuralSignals => ({
  disclaimer: "Evidence only. A modifier recurring across bases shows a PATTERN, not a role.",
  modifiers: [
    {
      modifier: "golden",
      occurrences: 330,
      conceptCount: 3,
      sessionCount: 30,
      dayCount: 5,
      bases: ["tigerfin"],
      concepts: [
        { conceptId: "concept:golden-tigerfin", displayName: "golden tigerfin", occurrences: 102 },
      ],
    },
  ],
  families: [
    {
      familyId: "family:tigerfin",
      baseName: "tigerfin",
      memberCount: 3,
      isSingleton: false,
      occurrences: 449,
      sessionCount: 45,
      dayCount: 5,
      members: [
        {
          conceptId: "concept:exotic-tigerfin",
          displayName: "exotic tigerfin",
          modifiers: ["exotic"],
          occurrences: 262,
          evidence: evidence(3),
          humanDecision: null,
        },
        {
          conceptId: "concept:golden-tigerfin",
          displayName: "golden tigerfin",
          modifiers: ["golden"],
          occurrences: 102,
          evidence: evidence(3),
          humanDecision: null,
        },
        {
          conceptId: "concept:tigerfin",
          displayName: "tigerfin",
          modifiers: [],
          occurrences: 52,
          evidence: evidence(3),
          humanDecision: null,
        },
      ],
    },
  ],
  bases: [
    {
      baseName: "tigerfin",
      occurrences: 449,
      conceptCount: 3,
      sessionCount: 45,
      dayCount: 5,
      occursBare: true,
      bareConcept: "tigerfin",
      kbCandidates: ["fish:tigerfin"],
      modifierTotals: { golden: 330, exotic: 262 },
      concepts: [
        { conceptId: "concept:tigerfin", displayName: "tigerfin", occurrences: 52, modifiers: [] },
      ],
    },
  ],
});

const kbOption = (id: string): KbEntityOption => ({
  id,
  canonicalName: id.split(":")[1] ?? id,
  aliases: [],
  category: "fish",
  rarity: null,
  inFishVocab: true,
});

function setup(over: { status?: Partial<WorkstationStatus>; concept?: Partial<WorkstationConcept> } = {}) {
  const overrides = over.concept ?? {};
  mocked.ontologyReviewStatus.mockResolvedValue(status(over.status));
  /**
   * Id-aware, unlike a single-value mock.
   *
   * The component tracks its position by concept id, so a mock that answered the
   * same concept for every request made "go back" and "go forward"
   * indistinguishable and hid real navigation bugs. This answers with a concept
   * whose `conceptId` matches what was asked for, so the screen reflects genuine
   * movement.
   */
  mocked.ontologyReviewConcept.mockImplementation(async (id: string) =>
    concept({
      ...overrides,
      conceptId: id,
      // Derive the display name from the id, so moving between concepts visibly
      // changes the screen. Without this the name stayed `exotic tigerfin` for
      // every concept and a navigation assertion could not tell movement from a
      // no-op.
      displayName: overrides.displayName ?? id.replace(/^concept:/, "").replace(/-/g, " "),
      normalizedName: overrides.normalizedName ?? id.replace(/^concept:/, "").replace(/-/g, " "),
    }),
  );
  mocked.ontologyReviewStructural.mockResolvedValue(structural());
  mocked.ontologyReviewRead.mockResolvedValue({
    available: true,
    detail: "",
    total: 0,
    counts: {},
    concepts: [],
  } as never);
  mocked.ontologyReviewKbSearch.mockResolvedValue([kbOption("fish:shark"), kbOption("fish:snapper")]);
  mocked.ontologyReviewFamily.mockResolvedValue({
    familyId: "family:tigerfin",
    baseName: "tigerfin",
    memberCount: 3,
    occurrences: 449,
    decisionRequired: "are these one entity, several entities?",
    members: structural().families[0].members as never,
  } as FamilyView);
  // A 1x1 GIF, so the rendered DOM is what a reviewer would actually see and the
  // image buttons have a real accessible name.
  mocked.ontologyReviewThumb.mockResolvedValue(
    "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7",
  );
  mocked.ontologyReviewFull.mockResolvedValue(null);
  mocked.ontologyDecide.mockResolvedValue({
    ok: true,
    nextConceptId: "concept:crimson-snapper",
    decidedTotal: 1,
    pendingTotal: 2,
    message: "Recorded.",
  });
  mocked.ontologyUndo.mockResolvedValue(status());
}

/** Open the dashboard and start P0, landing on the first unresolved P0 concept. */
async function startReview() {
  render(<OntologyReview />);
  const start = await screen.findByRole("button", { name: /start p0 review/i });
  fireEvent.click(start);
  await screen.findByText("EXOTIC TIGERFIN");
}

beforeEach(() => {
  vi.clearAllMocks();
  cleanup();
  setup();
});

// ===========================================================================
// The P0 queue
// ===========================================================================

describe("the P0 queue", () => {
  it("reports real per-band counts on the dashboard", async () => {
    render(<OntologyReview />);
    await screen.findByText(/0 \/ 3 reviewed/);
    expect(screen.getByText(/START P0 REVIEW \(2\)/)).toBeTruthy();
  });

  it("opens the highest-priority unresolved concept with one click", async () => {
    await startReview();
    expect(mocked.ontologyReviewConcept).toHaveBeenCalledWith("concept:exotic-tigerfin");
  });

  it("opens straight into the review, with no intermediate page", async () => {
    await startReview();
    // Name, evidence and the four decisions are all on screen together.
    expect(screen.getByText("EXOTIC TIGERFIN")).toBeTruthy();
    expect(screen.getByText(/262 rows/)).toBeTruthy();
    expect(screen.getByRole("button", { name: /Unknown/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Existing/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Variant/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /New/ })).toBeTruthy();
  });

  it("shows the observed name prominently and its recurrence counts", async () => {
    await startReview();
    expect(screen.getByText("36 sessions")).toBeTruthy();
    expect(screen.getByText("5 days")).toBeTruthy();
    expect(screen.getByText("P0")).toBeTruthy();
  });

  it("keeps MODEL_NOT_READY visible on both screens", async () => {
    render(<OntologyReview />);
    await screen.findByText("MODEL_NOT_READY");
    cleanup();
    await startReview();
    expect(screen.getAllByText("MODEL_NOT_READY").length).toBeGreaterThan(0);
  });

  it("surfaces the real missing state instead of an error when the index is absent", async () => {
    mocked.ontologyReviewStatus.mockResolvedValue(
      status({ available: false, detail: "No ontology review index found." }),
    );
    render(<OntologyReview />);
    await screen.findByText(/No ontology review index found/);
    expect(screen.getByText(/ml.gpo_ontology_review build/)).toBeTruthy();
  });
});

// ===========================================================================
// No preselection
// ===========================================================================

describe("no preselection", () => {
  it("starts with no decision selected", async () => {
    await startReview();
    for (const name of [/Unknown/, /Existing/, /Variant/, /New/]) {
      const btn = screen.getByRole("button", { name });
      expect(btn.getAttribute("aria-pressed")).toBe("false");
    }
  });

  it("does not preselect an existing entity even when KB candidates are shown", async () => {
    await startReview();
    fireEvent.click(screen.getByRole("button", { name: new RegExp("^2Existing") }));
    await screen.findByPlaceholderText("shark…");
    await waitFor(() => expect(mocked.ontologyReviewKbSearch).toHaveBeenCalled());
    expect(screen.queryByText(/^selected:/)).toBeNull();
  });

  it("offers the proposed name as a placeholder, never as a filled value", async () => {
    await startReview();
    fireEvent.click(screen.getByRole("button", { name: new RegExp("^4New") }));
    const input = (await screen.findByPlaceholderText(
      "exotic tigerfin",
    )) as HTMLInputElement;
    expect(input.value).toBe("");
  });

  it("shows the KB candidates as evidence, clearly labelled", async () => {
    await startReview();
    expect(screen.getByText(/KB has no entity for this name/)).toBeTruthy();
  });

  it("labels a NEW_ENTITY proposal as not yet in the knowledge base", async () => {
    await startReview();
    fireEvent.click(screen.getByRole("button", { name: new RegExp("^4New") }));
    expect(await screen.findByText("PROPOSAL ONLY")).toBeTruthy();
    expect(screen.getByText("NOT YET IN KNOWLEDGE BASE")).toBeTruthy();
  });
});

// ===========================================================================
// Keyboard
// ===========================================================================

describe("keyboard shortcuts", () => {
  it("1-4 select the four decisions", async () => {
    await startReview();
    const press = (k: string) => fireEvent.keyDown(window, { key: k });

    press("2");
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /Existing/ }).getAttribute("aria-pressed")).toBe(
        "true",
      ),
    );
    press("3");
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /Variant/ }).getAttribute("aria-pressed")).toBe(
        "true",
      ),
    );
    press("4");
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /New/ }).getAttribute("aria-pressed")).toBe("true"),
    );
    press("1");
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /Unknown/ }).getAttribute("aria-pressed")).toBe(
        "true",
      ),
    );
  });

  it("pressing the same digit twice clears the selection", async () => {
    await startReview();
    fireEvent.keyDown(window, { key: "1" });
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /Unknown/ }).getAttribute("aria-pressed")).toBe(
        "true",
      ),
    );
    fireEvent.keyDown(window, { key: "1" });
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /Unknown/ }).getAttribute("aria-pressed")).toBe(
        "false",
      ),
    );
  });

  it("N and P move between concepts", async () => {
    await startReview();
    fireEvent.keyDown(window, { key: "N" });
    await waitFor(() =>
      expect(mocked.ontologyReviewConcept).toHaveBeenCalledWith("concept:crimson-snapper"),
    );
  });

  it("Enter confirms a complete decision", async () => {
    await startReview();
    fireEvent.keyDown(window, { key: "1" });
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /Unknown/ }).getAttribute("aria-pressed")).toBe(
        "true",
      ),
    );
    fireEvent.keyDown(window, { key: "Enter" });
    await waitFor(() => expect(mocked.ontologyDecide).toHaveBeenCalledTimes(1));
  });

  it("Enter on an incomplete decision explains what is missing and saves nothing", async () => {
    await startReview();
    fireEvent.keyDown(window, { key: "2" });
    await screen.findByPlaceholderText("shark…");
    fireEvent.keyDown(window, { key: "Enter" });
    // The message appears twice on purpose: in the alert banner and beside the
    // disabled Confirm button, so the reviewer sees it whether they look up or down.
    expect(await screen.findByText(/Cannot save yet/)).toBeTruthy();
    expect(screen.getAllByText(/choose the existing entity/).length).toBeGreaterThanOrEqual(2);
    expect(mocked.ontologyDecide).not.toHaveBeenCalled();
  });

  it("Space opens the evidence viewer and Escape closes it", async () => {
    await startReview();
    fireEvent.keyDown(window, { key: " " });
    await screen.findByRole("dialog", { name: /evidence image/ });
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("arrow keys step through the evidence viewer", async () => {
    await startReview();
    fireEvent.keyDown(window, { key: " " });
    const dialog = await screen.findByRole("dialog", { name: /evidence image/ });
    expect(within(dialog).getByText("1 / 12")).toBeTruthy();
    fireEvent.keyDown(window, { key: "ArrowRight" });
    await waitFor(() => expect(within(dialog).getByText("2 / 12")).toBeTruthy());
    fireEvent.keyDown(window, { key: "ArrowLeft" });
    await waitFor(() => expect(within(dialog).getByText("1 / 12")).toBeTruthy());
  });

  it("a digit typed into the notes field is a digit, not a decision", async () => {
    await startReview();
    fireEvent.keyDown(window, { key: "4" });
    const input = await screen.findByPlaceholderText("exotic tigerfin");
    // Typing into a focused input must not trip the shortcut.
    fireEvent.keyDown(input, { key: "1" });
    fireEvent.change(input, { target: { value: "123" } });
    expect(((input as HTMLInputElement).value)).toBe("123");
    expect(screen.getByRole("button", { name: /Unknown/ }).getAttribute("aria-pressed")).toBe(
      "false",
    );
  });

  it("a modified keystroke does not select a decision", async () => {
    await startReview();
    fireEvent.keyDown(window, { key: "1", ctrlKey: true });
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.getByRole("button", { name: /Unknown/ }).getAttribute("aria-pressed")).toBe(
      "false",
    );
  });

  it("shows the shortcuts on the screen", async () => {
    await startReview();
    // The legend is split across <Kbd> elements, so match against the rendered
    // text of the toolbar rather than a single node.
    const legend = screen.getAllByText(/choose/).map((n) => n.textContent).join(" ");
    expect(legend).toMatch(/choose/);
    expect(legend).toMatch(/1/);
    expect(screen.getAllByText(/move/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/view/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/undo/).length).toBeGreaterThan(0);
  });
});

// ===========================================================================
// Autosave, advance, failure
// ===========================================================================

describe("autosave and advance", () => {
  it("saves and advances to the next unresolved concept", async () => {
    await startReview();
    fireEvent.keyDown(window, { key: "1" });
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /Unknown/ }).getAttribute("aria-pressed")).toBe(
        "true",
      ),
    );
    fireEvent.keyDown(window, { key: "Enter" });
    await waitFor(() => expect(mocked.ontologyReviewConcept).toHaveBeenCalledWith("concept:crimson-snapper"));
  });

  it("sends the concept id and the decision", async () => {
    await startReview();
    fireEvent.click(screen.getByRole("button", { name: /Confirm decision/ }));
    expect(mocked.ontologyDecide).not.toHaveBeenCalled();
    fireEvent.keyDown(window, { key: "1" });
    await waitFor(() => expect(screen.getByRole("button", { name: /Unknown/ }).getAttribute("aria-pressed")).toBe("true"));
    fireEvent.keyDown(window, { key: "Enter" });
    await waitFor(() => expect(mocked.ontologyDecide).toHaveBeenCalledTimes(1));
    const payload = mocked.ontologyDecide.mock.calls[0][0];
    expect(payload.conceptId).toBe("concept:exotic-tigerfin");
    expect(payload.decision).toBe("UNKNOWN");
  });

  it("attaches real representative image ids as evidence", async () => {
    await startReview();
    fireEvent.keyDown(window, { key: "1" });
    await waitFor(() => expect(screen.getByRole("button", { name: /Unknown/ }).getAttribute("aria-pressed")).toBe("true"));
    fireEvent.keyDown(window, { key: "Enter" });
    await waitFor(() => expect(mocked.ontologyDecide).toHaveBeenCalled());
    const payload = mocked.ontologyDecide.mock.calls[0][0];
    const ids = payload.evidenceImageIds ?? [];
    expect(ids.length).toBeGreaterThan(0);
    expect(ids.every((i: string) => /^[0-9a-f]{16}$/.test(i))).toBe(true);
  });

  it("resets the draft on the next concept so a decision cannot carry over", async () => {
    await startReview();
    fireEvent.click(screen.getByRole("button", { name: new RegExp("^4New") }));
    await screen.findByPlaceholderText("exotic tigerfin");
    fireEvent.change(screen.getByPlaceholderText("exotic tigerfin"), {
      target: { value: "Exoticfin" },
    });
    fireEvent.keyDown(window, { key: "Enter" });
    // The save advances to concept 1. That is also where a draft-carrying bug
    // would be visible: the NEW_ENTITY field would still hold "Exoticfin".
    await screen.findByText("CRIMSON SNAPPER");
    fireEvent.click(screen.getByRole("button", { name: new RegExp("^4New") }));
    const input = (await screen.findByPlaceholderText("crimson snapper")) as HTMLInputElement;
    expect(input.value).toBe("");

    // And on the way back, still empty.
    fireEvent.keyDown(window, { key: "P" });
    await screen.findByText("EXOTIC TIGERFIN");
    fireEvent.click(screen.getByRole("button", { name: new RegExp("^4New") }));
    const back = (await screen.findByPlaceholderText("exotic tigerfin")) as HTMLInputElement;
    expect(back.value).toBe("");
  });

  it("NEVER claims a decision was saved when persistence fails", async () => {
    await startReview();
    mocked.ontologyDecide.mockRejectedValue("disk full");
    fireEvent.keyDown(window, { key: "1" });
    await waitFor(() => expect(screen.getByRole("button", { name: /Unknown/ }).getAttribute("aria-pressed")).toBe("true"));
    fireEvent.keyDown(window, { key: "Enter" });

    await screen.findByText(/NOT SAVED/);
    expect(screen.getByText(/disk full/)).toBeTruthy();
    expect(screen.queryByText(/^Saved\./)).toBeNull();
  });

  it("returns the reviewer to the concept they were on after a failure", async () => {
    await startReview();
    mocked.ontologyDecide.mockRejectedValue("disk full");
    fireEvent.keyDown(window, { key: "1" });
    await waitFor(() => expect(screen.getByRole("button", { name: /Unknown/ }).getAttribute("aria-pressed")).toBe("true"));
    fireEvent.keyDown(window, { key: "Enter" });
    await screen.findByText(/NOT SAVED/);
    // Still on exotic tigerfin, and its decision is still chosen — the decision
    // was not silently discarded, because it was not saved.
    expect(screen.getByText("EXOTIC TIGERFIN")).toBeTruthy();
    expect(screen.getByRole("button", { name: /Unknown/ }).getAttribute("aria-pressed")).toBe(
      "true",
    );
  });

  it("does not advance when persistence fails", async () => {
    // Asserted on the SCREEN, not on which concepts were requested: navigating
    // prefetches the next concept's thumbnails, so a request alone would not mean
    // the reviewer was moved there.
    await startReview();
    mocked.ontologyDecide.mockRejectedValue("disk full");
    fireEvent.keyDown(window, { key: "1" });
    await waitFor(() => expect(screen.getByRole("button", { name: /Unknown/ }).getAttribute("aria-pressed")).toBe("true"));
    fireEvent.keyDown(window, { key: "Enter" });
    await screen.findByText(/NOT SAVED/);
    expect(screen.getByText("EXOTIC TIGERFIN")).toBeTruthy();
    expect(screen.queryByText("CRIMSON SNAPPER")).toBeNull();
  });

  it("surfaces the proposal-only message after a successful save", async () => {
    await startReview();
    fireEvent.keyDown(window, { key: "1" });
    await waitFor(() => expect(screen.getByRole("button", { name: /Unknown/ }).getAttribute("aria-pressed")).toBe("true"));
    fireEvent.keyDown(window, { key: "Enter" });
    await screen.findByText(/Saved\. 2 pending\./);
  });
});

// ===========================================================================
// Resume after reload
// ===========================================================================

describe("resume after reload", () => {
  it("restores progress from the server on mount", async () => {
    mocked.ontologyReviewStatus.mockResolvedValue(
      status({
        total: 77,
        decided: 4,
        pending: 73,
        counts: { P0: 11, P1: 3, P2: 16, P3: 47 },
        remaining: { P0: 7, P1: 3, P2: 16, P3: 47 },
        byDecision: { EXISTING_ENTITY: 1, NEW_ENTITY: 2, UNKNOWN: 1 },
        queue: ["concept:crimson-snapper", "concept:swordfish"],
      }),
    );
    render(<OntologyReview />);
    await screen.findByText(/4 \/ 77 reviewed/);
    expect(screen.getByText(/START P0 REVIEW \(7\)/)).toBeTruthy();
    expect(screen.getByText("EXISTING_ENTITY 1")).toBeTruthy();
    expect(screen.getByText("NEW_ENTITY 2")).toBeTruthy();
  });

  it("opens the first still-unresolved concept, not the first concept overall", async () => {
    mocked.ontologyReviewStatus.mockResolvedValue(
      status({
        queue: ["concept:crimson-snapper", "concept:swordfish"],
        order: [
          "concept:exotic-tigerfin",
          "concept:crimson-snapper",
          "concept:swordfish",
        ],
      }),
    );
    render(<OntologyReview />);
    fireEvent.click(await screen.findByRole("button", { name: /START P0 REVIEW/ }));
    await waitFor(() =>
      expect(mocked.ontologyReviewConcept).toHaveBeenCalledWith("concept:crimson-snapper"),
    );
  });

  it("shows an empty band as empty rather than reviewing something else", async () => {
    mocked.ontologyReviewStatus.mockResolvedValue(
      status({ remaining: { P1: 0 }, counts: { P1: 0 } }),
    );
    mocked.ontologyReviewConcept.mockResolvedValue(concept({ conceptId: "concept:swordfish" }));
    render(<OntologyReview />);
    fireEvent.click(await screen.findByRole("button", { name: /^P1/ }));
    await screen.findByText(/Nothing pending in P1/);
  });
});

// ===========================================================================
// Undo
// ===========================================================================

describe("undo", () => {
  it("records a reversal and says the original decision is retained", async () => {
    // Returning to an already-decided concept, which is how a reviewer reaches a
    // decision they want to correct.
    mocked.ontologyReviewConcept.mockImplementation(async (id: string) =>
      concept({ conceptId: id, humanDecision: "UNKNOWN" }),
    );
    await startReview();
    mocked.ontologyUndo.mockResolvedValue(status({ decided: 0, pending: 3 }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /Undo last decision/ })).toBeTruthy(),
    );
    fireEvent.click(screen.getByRole("button", { name: /Undo last decision/ }));
    await waitFor(() => expect(mocked.ontologyUndo).toHaveBeenCalledTimes(1));
    // Undo targets the concept ON SCREEN, not a remembered one.
    expect(mocked.ontologyUndo.mock.calls[0][0]).toBe("concept:exotic-tigerfin");
    await screen.findByText(/Reversal recorded/);
    expect(screen.getByText(/original decision remains in the log/i)).toBeTruthy();
  });

  it("does not offer undo for a concept with no decision, by button or by key", async () => {
    // Keyboard and mouse must agree on what is possible. A shortcut that can do
    // something the UI says you cannot do is worse than no shortcut at all.
    await startReview();
    expect(screen.queryByRole("button", { name: /Undo last decision/ })).toBeNull();
    fireEvent.keyDown(window, { key: "U" });
    await new Promise((r) => setTimeout(r, 20));
    expect(mocked.ontologyUndo).not.toHaveBeenCalled();
  });

  it("the U key undoes exactly like the button", async () => {
    // Keyboard-first is a requirement, so the shortcut must be a real peer of the
    // button rather than a decoration.
    mocked.ontologyReviewConcept.mockImplementation(async (id: string) =>
      concept({ conceptId: id, humanDecision: "UNKNOWN" }),
    );
    await startReview();
    mocked.ontologyUndo.mockResolvedValue(status({ decided: 0, pending: 3 }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /Undo last decision/ })).toBeTruthy(),
    );
    fireEvent.keyDown(window, { key: "u" });
    await waitFor(() => expect(mocked.ontologyUndo).toHaveBeenCalledTimes(1));
    expect(mocked.ontologyUndo.mock.calls[0][0]).toBe("concept:exotic-tigerfin");
  });

  it("offers undo once the concept has a recorded decision", async () => {
    mocked.ontologyReviewConcept.mockImplementation(async (id: string) =>
      concept({ conceptId: id, humanDecision: "UNKNOWN" }),
    );
    await startReview();
    expect(screen.getByRole("button", { name: /Undo last decision/ })).toBeTruthy();
  });

  it("says the decision is still recorded when undo fails", async () => {
    mocked.ontologyReviewConcept.mockImplementation(async (id: string) =>
      concept({ conceptId: id, humanDecision: "UNKNOWN" }),
    );
    await startReview();
    mocked.ontologyUndo.mockRejectedValue("read-only volume");
    fireEvent.click(screen.getByRole("button", { name: /Undo last decision/ }));
    await screen.findByText(/Undo failed/);
    // The important half: the reviewer is told the decision SURVIVED the failure.
    expect(screen.getByText(/still recorded/)).toBeTruthy();
  });
});

// ===========================================================================
// Family comparison and structural panel
// ===========================================================================

describe("family comparison", () => {
  it("opens a comparison showing every member of the family", async () => {
    await startReview();
    fireEvent.click(screen.getByRole("button", { name: /Compare family/ }));
    const dialog = await screen.findByRole("dialog", { name: /family comparison/i });
    expect(within(dialog).getByText(/Family: tigerfin/)).toBeTruthy();
    expect(within(dialog).getByText("exotic tigerfin")).toBeTruthy();
    expect(within(dialog).getByText("golden tigerfin")).toBeTruthy();
    expect(within(dialog).getByText("tigerfin")).toBeTruthy();
  });

  it("asks the structural question without answering it", async () => {
    await startReview();
    fireEvent.click(screen.getByRole("button", { name: /Compare family/ }));
    const dialog = await screen.findByRole("dialog", { name: /family comparison/i });
    expect(within(dialog).getByText(/are these one entity/i)).toBeTruthy();
  });

  it("navigating from the comparison jumps to that sibling", async () => {
    await startReview();
    fireEvent.click(screen.getByRole("button", { name: /Compare family/ }));
    const dialog = await screen.findByRole("dialog", { name: /family comparison/i });
    const buttons = within(dialog).getAllByRole("button", { name: /Open this concept/ });
    fireEvent.click(buttons[1]);
    await waitFor(() =>
      expect(mocked.ontologyReviewConcept).toHaveBeenCalledWith("concept:golden-tigerfin"),
    );
  });
});

describe("structural signals panel", () => {
  it("shows only the modifiers this concept actually carries", async () => {
    // The panel is filtered to the current concept: a reviewer looking at `exotic
    // tigerfin` should not be shown `golden`'s numbers, which are a different
    // question.
    await startReview();
    expect(screen.queryByText("golden")).toBeNull();
  });

  it("shows a recurring modifier as evidence with a 'possible role' and no verdict", async () => {
    mocked.ontologyReviewConcept.mockImplementation(async (id: string) =>
      concept({ conceptId: id, displayName: "golden tigerfin", modifiers: ["golden"] }),
    );
    mocked.ontologyReviewStatus.mockResolvedValue(
      status({ queue: ["concept:golden-tigerfin", "concept:crimson-snapper", "concept:swordfish"] }),
    );
    render(<OntologyReview />);
    fireEvent.click(await screen.findByRole("button", { name: /START P0 REVIEW/ }));
    await screen.findByText("GOLDEN TIGERFIN");
    expect(screen.getByText("golden")).toBeTruthy();
    expect(screen.getByText(/330 rows/)).toBeTruthy();
    // The role is offered as a possibility and explicitly left undecided.
    expect(screen.getByText(/possible role: MODIFIER or ENTITY — undecided/)).toBeTruthy();
  });

  it("states that the panel is evidence only", async () => {
    await startReview();
    expect(screen.getByText(/Evidence only\./)).toBeTruthy();
  });

  it("reports whether a base name ever appears bare", async () => {
    await startReview();
    expect(screen.getByText(/does appear bare/)).toBeTruthy();
  });
});

// ===========================================================================
// Existing-entity workflow
// ===========================================================================

describe("existing entity workflow", () => {
  it("requires the human to click an entity", async () => {
    await startReview();
    fireEvent.click(screen.getByRole("button", { name: new RegExp("^2Existing") }));
    const results = await screen.findByRole("button", { name: /fish:shark/ });
    fireEvent.click(results);
    await screen.findByText(/selected: fish:shark/);
  });

  it("saves once an entity is chosen", async () => {
    await startReview();
    fireEvent.click(screen.getByRole("button", { name: new RegExp("^2Existing") }));
    fireEvent.click(await screen.findByRole("button", { name: /fish:snapper/ }));
    await screen.findByText(/selected: fish:snapper/);
    fireEvent.keyDown(window, { key: "Enter" });
    await waitFor(() => expect(mocked.ontologyDecide).toHaveBeenCalled());
    expect(mocked.ontologyDecide.mock.calls[0][0].existingEntityId).toBe("fish:snapper");
  });

  it("says plainly when nothing matches", async () => {
    mocked.ontologyReviewKbSearch.mockResolvedValue([]);
    await startReview();
    fireEvent.click(screen.getByRole("button", { name: new RegExp("^2Existing") }));
    await screen.findByText("no matches");
  });
});

// ===========================================================================
// Variant workflow
// ===========================================================================

describe("variant workflow", () => {
  it("requires both the parent and a descriptor", async () => {
    await startReview();
    fireEvent.click(screen.getByRole("button", { name: new RegExp("^3Variant") }));
    fireEvent.click(await screen.findByRole("button", { name: /fish:shark/ }));
    // Parent chosen, no descriptor yet.
    expect(screen.getByText(/describe how it differs/)).toBeTruthy();
    fireEvent.change(screen.getByPlaceholderText("golden colour"), {
      target: { value: "golden colour" },
    });
    await waitFor(() => expect(screen.queryByText(/describe how it differs/)).toBeNull());
  });

  it("sends both fields", async () => {
    await startReview();
    fireEvent.click(screen.getByRole("button", { name: new RegExp("^3Variant") }));
    fireEvent.click(await screen.findByRole("button", { name: /fish:shark/ }));
    fireEvent.change(screen.getByPlaceholderText("golden colour"), {
      target: { value: "golden colour" },
    });
    fireEvent.keyDown(window, { key: "Enter" });
    await waitFor(() => expect(mocked.ontologyDecide).toHaveBeenCalled());
    const p = mocked.ontologyDecide.mock.calls[0][0];
    expect(p.existingEntityId).toBe("fish:shark");
    expect(p.variantDescriptor).toBe("golden colour");
  });
});

// ===========================================================================
// Unknown workflow
// ===========================================================================

describe("unknown workflow", () => {
  it("needs only one click, plus confirm", async () => {
    await startReview();
    fireEvent.click(screen.getByRole("button", { name: new RegExp("^1Unknown") }));
    // No further fields required.
    expect(screen.getByText(/Insufficient evidence/)).toBeTruthy();
    fireEvent.keyDown(window, { key: "Enter" });
    await waitFor(() => expect(mocked.ontologyDecide).toHaveBeenCalled());
  });

  it("records the optional reason", async () => {
    await startReview();
    fireEvent.click(screen.getByRole("button", { name: new RegExp("^1Unknown") }));
    fireEvent.click(screen.getByRole("button", { name: "OCR artifact" }));
    fireEvent.keyDown(window, { key: "Enter" });
    await waitFor(() => expect(mocked.ontologyDecide).toHaveBeenCalled());
    expect(mocked.ontologyDecide.mock.calls[0][0].notes).toContain("OCR artifact");
  });
});

// ===========================================================================
// Evidence
// ===========================================================================

describe("evidence presentation", () => {
  it("shows six images initially and reveals the rest on request", async () => {
    await startReview();
    const images = () => screen.getAllByRole("button", { name: /606020203e34beb/ });
    expect(images().length).toBe(6);
    fireEvent.click(screen.getByRole("button", { name: /Show all 12/ }));
    await waitFor(() => expect(images().length).toBe(12));
  });

  it("labels stored labels as evidence rather than truth", async () => {
    await startReview();
    expect(screen.getByText(/Stored labels \(evidence, not truth\)/)).toBeTruthy();
    expect(screen.getByText(/fish:tigerfin×100/)).toBeTruthy();
  });

  it("shows the raw OCR and the normalized name", async () => {
    await startReview();
    expect(screen.getByText("Raw OCR")).toBeTruthy();
    expect(screen.getByText("Normalized")).toBeTruthy();
  });

  it("reports the total row count for the concept", async () => {
    await startReview();
    expect(screen.getByText(/262 rows/)).toBeTruthy();
  });
});