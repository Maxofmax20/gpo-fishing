/**
 * ONTOLOGY REVIEW WORKSTATION
 *
 * Optimised for one thing: a human making a decision about what the GAME is
 * naming, fast, with the evidence in front of them.
 *
 * # Design rules this file follows
 *
 * 1. **One screen.** Name, evidence, and the four decision shapes are visible
 *    together. No navigation between a concept and its decision.
 *
 * 2. **Keyboard first.** `1`-`4` select, `Enter` confirms, `N`/`P` move, `Space`
 *    opens the viewer, `J`/`K` step images. The mouse is optional. Focus returns
 *    to the decision buttons after every save so the flow never breaks.
 *
 * 3. **Nothing is preselected.** The panel may show that `crimson snapper`
 *    contains both `fish:crimson` and `fish:snapper`; it must never turn that
 *    into a chosen answer. Every decision starts unset.
 *
 * 4. **Optimistic, but honest.** A save advances the UI immediately and then
 *    persists. If persistence fails the advance is rolled back and the failure is
 *    shown - the screen never claims a decision was saved when it was not.
 *
 * 5. **The reviewer thinks about fish, not about the UI.** Progressive evidence
 *    reveals keep the useful part above the fold on a 1366x768 window.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "../lib/ipc";
import type { OntologyReviewPayload } from "../lib/types";
import { Button, Kbd, Pill, Section } from "../components/primitives";
import {
  DECISIONS,
  EMPTY_DRAFT,
  UNKNOWN_REASONS,
  buildDecisionPayload,
  computeProgress,
  firstOfBand,
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

// ---------------------------------------------------------------------------
// Types mirroring the workstation index (server-built, camelCase)
// ---------------------------------------------------------------------------

type Evidence = {
  imageId: string;
  sessionId: string;
  day: string;
  rawOcr: string;
  storedEntityId: string | null;
  ocrConfidence: number | null;
  timestampMs: number;
  selectedBecause: string;
};

type WorkstationConcept = {
  rank: number;
  conceptId: string;
  displayName: string;
  normalizedName: string;
  occurrences: number;
  sessionCount: number;
  dayCount: number;
  priority: string;
  priorityReasons: string[];
  ontologyStatus: string;
  currentKbCandidates: string[];
  baseName: string | null;
  modifiers: string[];
  familyId: string | null;
  storedLabels: [string, number][];
  storedAgreesWithCandidate: number;
  ocrVariants: string[];
  evidence: Evidence[];
  humanDecision: string | null;
  decisionNote?: string | null;
  decidedAt?: string | null;
};

type WorkstationStatus = {
  available: boolean;
  detail: string;
  total: number;
  counts: Record<string, number>;
  remaining: Record<string, number>;
  decided: number;
  pending: number;
  byDecision: Record<string, number>;
  queue: string[];
  order: string[];
};

type KbOption = {
  id: string;
  canonicalName: string;
  aliases: string[];
  category: string;
  rarity: string | null;
  inFishVocab: boolean;
};

type Structural = {
  disclaimer: string;
  modifiers: {
    modifier: string;
    occurrences: number;
    conceptCount: number;
    sessionCount: number;
    dayCount: number;
    bases: string[];
    concepts: { conceptId: string; displayName: string; occurrences: number }[];
  }[];
  families: {
    familyId: string;
    baseName: string | null;
    memberCount: number;
    isSingleton: boolean;
    occurrences: number;
    sessionCount: number;
    dayCount: number;
    members: {
      conceptId: string;
      displayName: string;
      modifiers: string[];
      occurrences: number;
      evidence: Evidence[];
      humanDecision: string | null;
    }[];
  }[];
  bases: {
    baseName: string;
    occurrences: number;
    conceptCount: number;
    occursBare: boolean;
    bareConcept: string | null;
    kbCandidates: string[];
    modifierTotals: Record<string, number>;
    concepts: {
      conceptId: string;
      displayName: string;
      occurrences: number;
      modifiers: string[];
    }[];
  }[];
};

type FamilyView = {
  familyId: string;
  baseName: string | null;
  memberCount: number;
  occurrences: number;
  decisionRequired: string;
  members: WorkstationConcept[];
};

/** Deterministic band order, matching the server index. */
const BANDS = ["P0", "P1", "P2", "P3"] as const;
const BAND_TONE: Record<string, "bad" | "warn" | "accent" | "mute"> = {
  P0: "bad",
  P1: "warn",
  P2: "accent",
  P3: "mute",
};

// ---------------------------------------------------------------------------
// Thumbnail cache
//
// Fetched once per image id and kept for the session. Reviewing a family or
// paging back must not re-request an image the reviewer has already seen.
// ---------------------------------------------------------------------------
const thumbCache = new Map<string, string | null>();

async function loadThumb(imageId: string): Promise<string | null> {
  if (thumbCache.has(imageId)) return thumbCache.get(imageId) ?? null;
  try {
    const url = await api.ontologyReviewThumb(imageId);
    thumbCache.set(imageId, url);
    return url;
  } catch {
    thumbCache.set(imageId, null);
    return null;
  }
}

function Thumb({ imageId, alt }: { imageId: string; alt: string }) {
  const [src, setSrc] = useState<string | null>(() => thumbCache.get(imageId) ?? null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let live = true;
    loadThumb(imageId).then((u) => {
      if (live) {
        setSrc(u);
        if (!u) setFailed(true);
      }
    });
    return () => {
      live = false;
    };
  }, [imageId]);
  if (failed) {
    return (
      <div className="aspect-[589/199] rounded bg-black/40 flex items-center justify-center text-[9px] text-fg-mute px-1 text-center">
        thumbnail unavailable
      </div>
    );
  }
  return src ? (
    <img
      src={src}
      alt={alt}
      loading="lazy"
      decoding="async"
      className="aspect-[589/199] w-full rounded bg-black/40 object-contain"
    />
  ) : (
    <div className="aspect-[589/199] rounded bg-black/40 animate-pulse" />
  );
}

// ---------------------------------------------------------------------------
// Image viewer
// ---------------------------------------------------------------------------

function ImageViewer({
  evidence,
  index,
  onIndex,
  onClose,
  conceptName,
}: {
  evidence: Evidence[];
  index: number;
  onIndex: (i: number) => void;
  onClose: () => void;
  conceptName: string;
}) {
  const [full, setFull] = useState<string | null>(null);
  const e = evidence[index];
  const [fullLoading, setFullLoading] = useState(false);

  useEffect(() => {
    setFull(null);
    setFullLoading(false);
  }, [index]);

  const loadFull = useCallback(() => {
    if (full || fullLoading || !e) return;
    setFullLoading(true);
    api
      .ontologyReviewFull(e.imageId)
      .then((u) => setFull(u))
      .catch(() => setFullLoading(false));
  }, [full, fullLoading, e]);

  useEffect(() => {
    const onKey = (ev: KeyboardEvent) => {
      if (ev.key === "Escape") {
        ev.preventDefault();
        ev.stopPropagation();
        onClose();
      } else if (ev.key === "ArrowRight" || ev.key === "j") {
        ev.preventDefault();
        onIndex(stepIndex(index, 1, evidence.length));
      } else if (ev.key === "ArrowLeft" || ev.key === "k") {
        ev.preventDefault();
        onIndex(stepIndex(index, -1, evidence.length));
      } else if (ev.key === "Enter") {
        ev.preventDefault();
        loadFull();
      }
    };
    // Capture phase so the viewer wins over the review shortcuts.
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [index, evidence.length, onClose, onIndex, loadFull]);

  if (!e) return null;
  return (
    <div
      className="fixed inset-0 z-50 bg-black/85 flex items-center justify-center p-6"
      onClick={onClose}
      role="dialog"
      aria-modal="true"
      aria-label={`evidence image ${index + 1} of ${evidence.length}`}
    >
      <div
        className="max-w-[1100px] w-full flex flex-col gap-2"
        onClick={(ev) => ev.stopPropagation()}
      >
        <div className="flex items-center justify-between text-[11px] text-fg-dim">
          <span className="font-mono">{e.imageId}</span>
          <span>
            {index + 1} / {evidence.length}
          </span>
        </div>
        <div className="flex items-center justify-center bg-black/50 rounded p-2 min-h-[220px]">
          {full ? (
            <img src={full} alt={e.imageId} className="max-h-[52vh] w-auto object-contain" />
          ) : (
            <Thumb imageId={e.imageId} alt={e.imageId} />
          )}
        </div>
        <div className="grid grid-cols-2 md:grid-cols-4 gap-x-4 gap-y-1 text-[10px] text-fg-mute">
          <div>
            <span className="text-fg-dim">session: </span>
            {e.sessionId.replace(/^session_/, "")}
          </div>
          <div>
            <span className="text-fg-dim">day: </span>
            {e.day || "-"}
          </div>
          <div>
            <span className="text-fg-dim">stored: </span>
            {e.storedEntityId || "(none)"}
          </div>
          <div>
            <span className="text-fg-dim">conf: </span>
            {e.ocrConfidence != null ? e.ocrConfidence.toFixed(3) : "-"}
          </div>
          <div className="col-span-2">
            <span className="text-fg-dim">raw OCR: </span>
            {e.rawOcr || "(empty)"}
          </div>
          <div className="col-span-2">
            <span className="text-fg-dim">concept: </span>
            {conceptName}
          </div>
          <div className="col-span-2">
            <span className="text-fg-dim">timestamp: </span>
            {e.timestampMs}
          </div>
          <div className="col-span-2">
            <span className="text-fg-dim">normalized: </span>
            {e.rawOcr.toLowerCase().trim()}
          </div>
        </div>
        <div className="flex items-center gap-2 justify-center text-[10px] text-fg-mute">
          <Kbd>←</Kbd>
          <Kbd>→</Kbd> navigate
          <Kbd>Enter</Kbd> full resolution
          <Kbd>Esc</Kbd> close
          <Button size="sm" kind="ghost" onClick={loadFull} disabled={!!full || fullLoading}>
            {full ? "Full resolution loaded" : fullLoading ? "Loading…" : "Load full resolution"}
          </Button>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Family comparison
// ---------------------------------------------------------------------------

function FamilyPanel({
  familyId,
  onJump,
  onClose,
}: {
  familyId: string;
  onJump: (conceptId: string) => void;
  onClose: () => void;
}) {
  const [view, setView] = useState<FamilyView | null>(null);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    setView(null);
    setErr(null);
    api
      .ontologyReviewFamily(familyId)
      .then((v) => live && setView(v))
      .catch((e) => live && setErr(String(e)));
    return () => {
      live = false;
    };
  }, [familyId]);

  return (
    <div
      className="fixed inset-0 z-50 bg-black/85 flex items-center justify-center p-6"
      onClick={onClose}
      role="dialog"
      aria-modal="true"
      aria-label="family comparison"
    >
      <div
        className="max-w-[1200px] w-full flex flex-col gap-3 max-h-full overflow-auto"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between">
          <div className="text-[13px] font-semibold uppercase tracking-wide">
            Family: {view?.baseName ?? familyId}
          </div>
          <Button size="sm" kind="ghost" onClick={onClose}>
            Close
          </Button>
        </div>
        {err && <div className="text-[11px] text-bad">{err}</div>}
        {!view && !err && <div className="text-[11px] text-fg-mute">Loading…</div>}
        {view && (
          <>
            <div className="text-[10px] text-fg-mute">
              {view.occurrences} rows · {view.memberCount} observed name(s) · question:{" "}
              {view.decisionRequired}
            </div>
            <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-3">
              {view.members.map((m) => (
                <div key={m.conceptId} className="rounded border border-line/70 p-2 flex flex-col gap-1.5">
                  <div className="flex items-baseline justify-between gap-2">
                    <span className="text-[12px] font-semibold">{m.displayName}</span>
                    <Pill tone={m.humanDecision ? "ok" : "accent"}>
                      {m.humanDecision ?? "PENDING"}
                    </Pill>
                  </div>
                  <div className="text-[10px] text-fg-dim">
                    {m.occurrences} rows · {m.sessionCount} sessions · {m.dayCount} days
                    {m.modifiers.length > 0 ? ` · ${m.modifiers.join(" + ")} + ${m.baseName}` : " · bare"}
                  </div>
                  {m.evidence.length > 0 && (
                    <Thumb imageId={m.evidence[0].imageId} alt={m.displayName} />
                  )}
                  <div className="text-[10px] text-fg-mute truncate">
                    {(m.evidence[0]?.rawOcr ?? "").slice(0, 40)}
                  </div>
                  <Button size="sm" kind="ghost" onClick={() => onJump(m.conceptId)}>
                    Open this concept
                  </Button>
                </div>
              ))}
            </div>
          </>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// The workstation
// ---------------------------------------------------------------------------

type SaveState = "idle" | "saving" | "error";

export function OntologyReview() {
  const [status, setStatus] = useState<WorkstationStatus | null>(null);
  const [concept, setConcept] = useState<WorkstationConcept | null>(null);
  const [structural, setStructural] = useState<Structural | null>(null);
  const [legacy, setLegacy] = useState<OntologyReviewPayload | null>(null);
  const [band, setBand] = useState<string>("P0");
  const [started, setStarted] = useState(false);

  // --- decision draft ----------------------------------------------------
  //
  // ONE object, not six useState calls. Split state meant a concept change had to
  // reset six setters in the right order, and forgetting one would carry a stale
  // proposed name into the next concept — the most dangerous bug this screen could
  // have, because it would file a real decision under a different fish. A single
  // `EMPTY_DRAFT` assignment makes that impossible.
  const [draft, setDraft] = useState<DecisionDraft>(EMPTY_DRAFT);
  const patch = useCallback((p: Partial<DecisionDraft>) => setDraft((d) => ({ ...d, ...p })), []);
  const decision = draft.decision;
  const entity = draft.existingEntityId;
  const proposedName = draft.proposedEntityName;
  const variantName = draft.variantDescriptor;
  const [kbQuery, setKbQuery] = useState("");
  const [kbOptions, setKbOptions] = useState<KbOption[]>([]);

  // --- ui state --------------------------------------------------------
  const [viewer, setViewer] = useState<number | null>(null);
  const [familyOpen, setFamilyOpen] = useState(false);
  const [saveState, setSaveState] = useState<SaveState>("idle");
  /**
   * Two error channels, not one.
   *
   * `loadError` is "this concept could not be fetched". `saveError` is "your
   * decision was NOT recorded". They used to share one slot, which meant the
   * navigation that followed a failed save cleared the very message explaining
   * why the save failed — leaving a screen that looked identical to a successful
   * save. A reviewer who saw that would reasonably believe their decision was
   * recorded and move on. Separate channels, and neither navigation clears the
   * save error.
   */
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const [showAllEvidence, setShowAllEvidence] = useState(false);

  const decisionRef = useRef<HTMLDivElement>(null);
  const cursorRef = useRef(0);

  // --- boot ------------------------------------------------------------
  useEffect(() => {
    api
      .ontologyReviewStatus()
      .then(setStatus)
      .catch(() => setStatus(null));
    api.ontologyReviewStructural().then(setStructural).catch(() => setStructural(null));
    // Legacy payload keeps the TrainingCenter's older consumers working.
    api
      .ontologyReviewRead()
      .then(setLegacy)
      .catch(() => setLegacy(null));
  }, []);

  const conceptOrder = useMemo(() => {
    // Server order is authoritative; the band filter is applied client-side for
    // instant tab switching without re-ordering anything.
    return status?.order ?? [];
  }, [status]);

  // conceptId -> priority band, from the legacy payload the Training Center
  // already loads. Declared BEFORE anything that reads it.
  // (An earlier version computed band counts here, which both duplicated the
  // server's numbers and read this map before it existed - a temporal dead zone
  // crash on first render.)
  const conceptBand = useMemo(() => {
    const m = new Map<string, string>();
    for (const c of legacy?.concepts ?? []) m.set(c.conceptId, c.priority);
    return m;
  }, [legacy]);

  const loadConcept = useCallback(async (conceptId: string) => {
    try {
      const c = await api.ontologyReviewConcept(conceptId);
      setConcept(c as WorkstationConcept);
      setLoadError(null);
    } catch (e) {
      setLoadError(`could not load ${conceptId}: ${String(e)}`);
      setConcept(null);
    }
  }, []);

  // --- advance / retreat ------------------------------------------------
  const goTo = useCallback(
    (conceptId: string | undefined, opts?: { keepDraft?: boolean }) => {
      if (!conceptId) {
        setConcept(null);
        return;
      }
      const i = conceptOrder.indexOf(conceptId);
      cursorRef.current = i;
      // Reset the draft: a decision must never carry over to another concept.
      // The one exception is a failed save on the SAME concept, where the reviewer
      // needs their wording back to retry. Carrying a draft onto a DIFFERENT
      // concept is the dangerous case and is still impossible.
      if (!opts?.keepDraft) setDraft(EMPTY_DRAFT);
      setKbQuery("");
      setKbOptions([]);
      setShowAllEvidence(false);
      void loadConcept(conceptId);
      // Prefetch the neighbour so the next transition has nothing to wait for.
      const nextId = conceptOrder[i + 1];
      if (nextId) {
        api.ontologyReviewConcept(nextId).then((c) => {
          const e = (c as WorkstationConcept)?.evidence ?? [];
          for (const ev of e.slice(0, 4)) void loadThumb(ev.imageId);
        }).catch(() => undefined);
      }
    },
    [conceptOrder, loadConcept],
  );

  const currentIndex = concept ? conceptOrder.indexOf(concept.conceptId) : -1;
  const nextId = currentIndex >= 0 ? conceptOrder[currentIndex + 1] : undefined;
  const prevId = currentIndex > 0 ? conceptOrder[currentIndex - 1] : undefined;

  const refreshStatus = useCallback(() => {
    api.ontologyReviewStatus().then(setStatus).catch(() => undefined);
  }, []);

  // --- KB search --------------------------------------------------------
  useEffect(() => {
    if (decision !== "EXISTING_ENTITY" && decision !== "VARIANT_OF_EXISTING") return;
    let live = true;
    const t = window.setTimeout(() => {
      api
        .ontologyReviewKbSearch(kbQuery, 40)
        .then((o) => live && setKbOptions(o))
        .catch(() => live && setKbOptions([]));
    }, 90);
    return () => {
      live = false;
      window.clearTimeout(t);
    };
  }, [kbQuery, decision]);

  // --- save -------------------------------------------------------------
  const problems = useMemo(() => missingRequirements(draft), [draft]);
  const canSave = useMemo(() => problems.length === 0 && concept !== null, [problems, concept]);

  const save = useCallback(async () => {
    if (!concept || !canSave || saveState === "saving") return;
    const target = concept.conceptId;
    const payload = buildDecisionPayload(
      target,
      draft,
      concept.evidence.map((e) => e.imageId),
    );
    if (!payload) {
      setSaveError("NOT SAVED: the decision is incomplete.");
      return;
    }
    setSaveState("saving");
    setSaveError(null);

    // Optimistic: remember the exact prior queue so a failure can restore it.
    const priorQueue = status?.queue ?? [];
    const optimisticQueue = queueAfterDecision(priorQueue, target);
    setStatus((s) => (s ? { ...s, queue: optimisticQueue, pending: s.pending - 1 } : s));

    try {
      const res = await api.ontologyDecide(payload);
      setSaveState("idle");
      setToast(`Saved. ${res.pendingTotal} pending.`);
      refreshStatus();
      goTo(resolveNextConcept(res.nextConceptId, priorQueue, target));
      // Focus returns to the decision area so the flow continues on keyboard.
      window.setTimeout(() => decisionRef.current?.focus(), 0);
    } catch (e) {
      // Honest rollback: the screen must never imply the decision was kept. The
      // queue is restored verbatim and the reviewer is returned to the concept they
      // were on, because they have not actually decided it.
      setSaveState("error");
      // Set AFTER the rollback navigation, and in its own channel, so it survives.
      setSaveError(`NOT SAVED: ${String(e)}`);
      setStatus((s) => (s ? { ...s, queue: priorQueue, pending: s.pending + 1 } : s));
      goTo(target, { keepDraft: true });
    }
  }, [concept, canSave, saveState, status, draft, refreshStatus, goTo]);

  const undo = useCallback(async () => {
    if (!concept) return;
    setSaveError(null);
    try {
      await api.ontologyUndo(concept.conceptId);
      setToast("Reversal recorded (the original decision remains in the log).");
      refreshStatus();
      const list = status?.queue ?? [];
      if (!list.includes(concept.conceptId)) {
        goTo([concept.conceptId, ...list][0]);
      }
      const c = await api.ontologyReviewConcept(concept.conceptId);
      setConcept(c as WorkstationConcept);
    } catch (e) {
      setSaveError(`Undo failed — the decision is still recorded: ${String(e)}`);
    }
  }, [concept, status, refreshStatus, goTo]);

  // --- keyboard ---------------------------------------------------------
  useEffect(() => {
    const onKey = (ev: KeyboardEvent) => {
      if (viewer !== null || familyOpen) return; // the modal owns the keyboard
      // Never steal a keystroke from a text field. `Escape` there blurs, so the
      // reviewer can get back to shortcuts without reaching for the mouse.
      if (isTextEntry(ev.target)) {
        if (ev.key === "Escape") (ev.target as HTMLElement).blur();
        return;
      }
      const action = keyToAction(ev.key, ev.ctrlKey || ev.metaKey || ev.altKey);
      if (action.kind === "none") return;
      ev.preventDefault();
      switch (action.kind) {
        case "select":
          // Pressing the same digit twice clears the selection: an escape hatch
          // that costs nothing and needs no Escape key.
          setDraft((d) => (d.decision === action.decision ? { ...d, decision: null } : { ...d, decision: action.decision }));
          window.setTimeout(() => decisionRef.current?.focus(), 0);
          break;
        case "confirm":
          if (canSave) void save();
          else setSaveError(`Cannot save yet: ${missingRequirements(draft).join("; ")}.`);
          break;
        case "next": goTo(nextId); break;
        case "previous": goTo(prevId); break;
        case "viewer":
          if (concept && concept.evidence.length) setViewer(viewer === null ? 0 : null);
          break;
        case "undo":
          // Same guard as the Undo button. Keyboard and mouse must agree on what is
          // possible; a shortcut that can do something the UI says you cannot is
          // worse than no shortcut.
          if (concept?.humanDecision) void undo();
          break;
        case "close":
          if (viewer !== null) setViewer(null);
          else if (familyOpen) setFamilyOpen(false);
          break;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [viewer, familyOpen, concept, nextId, prevId, canSave, draft, save, goTo, undo]);

  // --- not built yet ----------------------------------------------------
  if (status && !status.available) {
    return (
      <Section title="Ontology review">
        <div className="text-[11px] text-fg-dim flex flex-col gap-2">
          <div>{status.detail}</div>
          <pre className="bg-black/50 border border-line/60 rounded p-2 overflow-x-auto">
            python -m ml.gpo_ocr_audit{`\n`}python -m ml.gpo_ontology_review build
          </pre>
        </div>
      </Section>
    );
  }
  if (!status) return <div className="text-[11px] text-fg-mute">Loading…</div>;

  // ======================================================================
  // Landing: progress, then START
  // ======================================================================
  if (!started) {
    const p0 = status.remaining.P0 ?? 0;
    const progress = computeProgress(status.total, status.decided, status.byDecision, status.remaining);
    return (
      <Section
        title="Ontology review"
        action={
          <div className="flex items-center gap-2">
            <Pill tone="bad">MODEL_NOT_READY</Pill>
            <Pill tone={status.pending > 0 ? "warn" : "ok"}>
              {status.decided} / {status.total} reviewed
            </Pill>
          </div>
        }
      >
        <div className="flex flex-col gap-3">
          <div className="text-[11px] text-fg-dim">
            These are the names the <span className="font-semibold">game appears to be writing</span>. The
            current knowledge base&apos;s candidates are shown as evidence, not as an answer — the
            ontology is what is under review, so it must not bias the decision.
          </div>

          <div className="grid grid-cols-2 md:grid-cols-4 gap-2">
            {BANDS.map((b) => (
              <button
                key={b}
                type="button"
                onClick={() => {
                  setBand(b);
                  setStarted(true);
                  goTo(firstOfBand(status.queue ?? [], b, conceptBand));
                }}
                className="text-left rounded-lg border border-line/70 px-3 py-2 hover:border-accent/60 focus:outline-none focus:border-accent"
              >
                <div className="text-[10px] uppercase tracking-wide text-fg-dim">{b}</div>
                <div className="text-[20px] leading-tight">
                  {status.remaining[b] ?? 0}
                  <span className="text-[11px] text-fg-mute"> / {status.counts[b] ?? 0}</span>
                </div>
                <div className="text-[9px] text-fg-mute">remaining</div>
              </button>
            ))}
          </div>

          <div className="flex items-center gap-3 text-[11px]">
            <span className="text-fg-dim">Overall</span>
            <span className="font-mono text-fg">
              {"█".repeat(progress.blocks)}
              {"░".repeat(Math.max(0, 10 - progress.blocks))}
            </span>
            <span className="text-fg-mute">
              {status.decided} / {status.total}
            </span>
          </div>

          {Object.keys(status.byDecision).length > 0 && (
            <div className="flex flex-wrap gap-2 text-[10px]">
              {Object.entries(status.byDecision).map(([k, n]) => (
                <Pill key={k} tone="mute">
                  {k} {n}
                </Pill>
              ))}
            </div>
          )}

          <div className="flex items-center gap-2">
            <Button
              kind="primary"
              onClick={() => {
                setBand("P0");
                setStarted(true);
                goTo(firstOfBand(status.queue ?? [], "P0", conceptBand) ?? status.queue?.[0]);
              }}
            >
              {p0 > 0 ? `START P0 REVIEW (${p0})` : "REVIEW ALL"}
            </Button>
            <Button kind="ghost" onClick={() => { setBand("ALL"); setStarted(true); goTo(status.queue?.[0]); }}>
              Review all {status.pending} pending
            </Button>
          </div>

          <div className="text-[10px] text-fg-mute flex flex-wrap gap-x-3 gap-y-1">
            <span><Kbd>1</Kbd>–<Kbd>4</Kbd> choose decision</span>
            <span><Kbd>Enter</Kbd> confirm</span>
            <span><Kbd>N</Kbd>/<Kbd>P</Kbd> next / previous</span>
            <span><Kbd>Space</Kbd> evidence viewer</span>
            <span><Kbd>J</Kbd>/<Kbd>K</Kbd> step images</span>
            <span><Kbd>U</Kbd> undo</span>
            <span><Kbd>Esc</Kbd> close</span>
          </div>
        </div>
      </Section>
    );
  }

  // ======================================================================
  // Review
  // ======================================================================
  const bandRemaining = status.remaining[band] ?? 0;
  const bandTotal = status.counts[band] ?? 0;
  const bandDone = bandTotal - bandRemaining;

  return (
    <Section
      title="Ontology review"
      action={
        <div className="flex items-center gap-2">
          {band !== "ALL" && (
            <Pill tone={bandRemaining === 0 ? "ok" : BAND_TONE[band] ?? "mute"}>
              {bandDone} / {bandTotal} {band}
            </Pill>
          )}
          <Pill tone="mute">
            {status.decided} / {status.total} reviewed
          </Pill>
          <Pill tone="bad">MODEL_NOT_READY</Pill>
        </div>
      }
    >
      <div className="flex flex-wrap items-center gap-1.5 mb-2">
        {(["ALL", ...BANDS] as const).map((b) => (
          <Button key={b} size="sm" kind={band === b ? "primary" : "ghost"} onClick={() => setBand(b)}>
            {b}
            {b !== "ALL" ? ` (${status.remaining[b] ?? 0})` : ` (${status.pending})`}
          </Button>
        ))}
        <span className="w-2" />
        <Button size="sm" kind="ghost" onClick={() => setStarted(false)}>
          Dashboard
        </Button>
        {concept?.familyId && (
          <Button size="sm" kind="ghost" onClick={() => setFamilyOpen(true)}>
            Compare family
          </Button>
        )}
        {concept?.humanDecision && (
          <Button size="sm" kind="ghost" onClick={() => void undo()}>
            Undo last decision
          </Button>
        )}
        <span className="flex-1" />
        <span className="text-[10px] text-fg-mute">
          <Kbd>1</Kbd>–<Kbd>4</Kbd> choose · <Kbd>Enter</Kbd> save · <Kbd>N</Kbd>/<Kbd>P</Kbd> move ·{" "}
          <Kbd>Space</Kbd> view · <Kbd>U</Kbd> undo
        </span>
      </div>

      {saveError && (
        <div
          role="alert"
          className="text-[11px] text-bad border border-bad/50 rounded px-2 py-1 mb-2 flex items-center justify-between gap-2"
        >
          <span>{saveError}</span>
          <Button size="sm" kind="ghost" onClick={() => setSaveError(null)}>
            Dismiss
          </Button>
        </div>
      )}
      {loadError && (
        <div className="text-[11px] text-bad border border-bad/50 rounded px-2 py-1 mb-2 flex items-center justify-between gap-2">
          <span>{loadError}</span>
          <Button size="sm" kind="ghost" onClick={() => setLoadError(null)}>
            Dismiss
          </Button>
        </div>
      )}
      {toast && (
        <div className="text-[11px] text-ok border border-ok/40 rounded px-2 py-1 mb-2 flex items-center justify-between gap-2">
          <span>{toast}</span>
          <Button size="sm" kind="ghost" onClick={() => setToast(null)}>
            Dismiss
          </Button>
        </div>
      )}

      {!concept ? (
        <div className="text-[11px] text-fg-mute py-6 text-center">
          Nothing pending in {band}.{" "}
          <button type="button" className="underline" onClick={() => setStarted(false)}>
            Back to dashboard
          </button>
        </div>
      ) : (
        <div className="grid grid-cols-1 xl:grid-cols-[minmax(0,1fr)_260px] gap-3">
          {/* ---------------- main column ---------------- */}
          <div className="flex flex-col gap-2.5 min-w-0">
            <div className="rounded-lg border border-line/70 px-3 py-2">
              <div className="text-[9px] uppercase tracking-wide text-fg-dim">
                Observed game name
              </div>
              <div className="text-[24px] font-semibold leading-tight break-words">
                {concept.displayName.toUpperCase()}
              </div>
              <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-[11px] text-fg-dim mt-1">
                <span>{concept.occurrences} rows</span>
                <span>{concept.sessionCount} sessions</span>
                <span>{concept.dayCount} days</span>
                <Pill tone={BAND_TONE[concept.priority] ?? "mute"}>{concept.priority}</Pill>
                <span className="font-mono text-[10px] text-fg-mute">{concept.conceptId}</span>
                {concept.humanDecision && (
                  <Pill tone="ok">decided: {concept.humanDecision}</Pill>
                )}
              </div>
            </div>

            {/* evidence */}
            <div className="rounded-lg border border-line/70 px-3 py-2">
              <div className="flex items-center justify-between mb-1.5">
                <span className="text-[9px] uppercase tracking-wide text-fg-dim">
                  Representative evidence
                </span>
                <span className="text-[10px] text-fg-mute">
                  {showAllEvidence ? concept.evidence.length : Math.min(6, concept.evidence.length)} of{" "}
                  {concept.evidence.length} · <Kbd>Space</Kbd> opens the viewer
                </span>
              </div>
              <div className="grid grid-cols-3 md:grid-cols-4 lg:grid-cols-6 gap-1.5">
                {concept.evidence
                  .slice(0, showAllEvidence ? concept.evidence.length : 6)
                  .map((e, i) => (
                    <button
                      key={e.imageId}
                      type="button"
                      className="text-left rounded focus:outline-none focus:ring-1 focus:ring-accent"
                      onClick={() => setViewer(i)}
                      title={`${e.imageId} — ${e.selectedBecause}`}
                    >
                      <Thumb imageId={e.imageId} alt={e.imageId} />
                      <div className="text-[9px] text-fg-mute truncate mt-0.5">{e.day}</div>
                    </button>
                  ))}
              </div>
              {concept.evidence.length > 6 && (
                <Button
                  size="sm"
                  kind="ghost"
                  className="mt-1.5"
                  onClick={() => setShowAllEvidence((v) => !v)}
                >
                  {showAllEvidence ? "Show fewer" : `Show all ${concept.evidence.length}`}
                </Button>
              )}
            </div>

            {/* OCR / evidence facts */}
            <div className="rounded-lg border border-line/70 px-3 py-2 grid grid-cols-1 md:grid-cols-2 gap-x-4 gap-y-1 text-[11px]">
              <div className="min-w-0">
                <div className="text-fg-dim">Raw OCR</div>
                <div className="font-mono truncate" title={concept.evidence[0]?.rawOcr}>
                  {concept.evidence[0]?.rawOcr || "(empty)"}
                </div>
              </div>
              <div className="min-w-0">
                <div className="text-fg-dim">Normalized</div>
                <div className="font-mono truncate">{concept.normalizedName}</div>
              </div>
              <div className="min-w-0">
                <div className="text-fg-dim">Ontology status</div>
                <div className="font-mono">{concept.ontologyStatus}</div>
              </div>
              <div className="min-w-0">
                <div className="text-fg-dim">Current KB candidates</div>
                <div className="font-mono truncate">
                  {concept.currentKbCandidates.length === 0
                    ? "none — the KB has no entity for this name"
                    : concept.currentKbCandidates.join("  ")}
                </div>
              </div>
              {concept.modifiers.length > 0 && (
                <div className="min-w-0">
                  <div className="text-fg-dim">Textual split (not a claim)</div>
                  <div className="truncate">
                    {concept.modifiers.join(" + ")} + <span className="font-semibold">{concept.baseName}</span>
                  </div>
                </div>
              )}
              <div className="min-w-0">
                <div className="text-fg-dim">Stored labels (evidence, not truth)</div>
                <div className="font-mono truncate">
                  {concept.storedLabels.length === 0
                    ? "(none)"
                    : concept.storedLabels.map(([e, n]) => `${e}×${n}`).join("  ")}
                </div>
              </div>
              {concept.storedAgreesWithCandidate > 0 && (
                <div className="md:col-span-2 text-fg-mute">
                  {concept.storedAgreesWithCandidate} row(s) carry a stored label equal to one
                  candidate — the label agrees with one COMPONENT of this name. That is not a
                  confirmation.
                </div>
              )}
              {concept.ocrVariants.length > 1 && (
                <div className="md:col-span-2 min-w-0">
                  <div className="text-fg-dim">OCR variants ({concept.ocrVariants.length})</div>
                  <div className="font-mono text-[10px] truncate">{concept.ocrVariants.join(" · ")}</div>
                </div>
              )}
            </div>

            {/* decision */}
            <div
              ref={decisionRef}
              tabIndex={-1}
              className="rounded-lg border border-accent/40 px-3 py-2 focus:outline-none"
            >
              <div className="text-[9px] uppercase tracking-wide text-fg-dim mb-1.5">
                What is this? — nothing is pre-selected
              </div>
              <div className="grid grid-cols-2 md:grid-cols-4 gap-1.5">
                {DECISIONS.map((d) => (
                  <button
                    key={d.id}
                    type="button"
                    onClick={() => patch({ decision: decision === d.id ? null : d.id })}
                    className={`rounded border px-2 py-1.5 text-left focus:outline-none focus:ring-1 focus:ring-accent ${
                      decision === d.id
                        ? "border-accent bg-accent/15"
                        : "border-line/70 hover:border-accent/50"
                    }`}
                    aria-pressed={decision === d.id}
                  >
                    <div className="text-[12px] font-semibold flex items-center gap-1.5">
                      <Kbd>{d.key}</Kbd>
                      {d.label}
                    </div>
                    <div className="text-[9px] text-fg-mute">{d.hint}</div>
                  </button>
                ))}
              </div>

              {/* --- decision-specific fields --- */}
              <div className="mt-2 flex flex-col gap-2">
                {(decision === "EXISTING_ENTITY" || decision === "VARIANT_OF_EXISTING") && (
                  <div className="flex flex-col gap-1">
                    <label className="text-[10px] text-fg-dim">
                      {decision === "VARIANT_OF_EXISTING" ? "Variant of (existing entity)" : "Existing entity"}{" "}
                      — search, then click one. Nothing is preselected.
                    </label>
                    <input
                      className="bg-black/40 border border-line/70 rounded px-2 py-1 text-[11px] font-mono"
                      value={kbQuery}
                      placeholder="shark…"
                      onChange={(e) => setKbQuery(e.target.value)}
                    />
                    <div className="max-h-40 overflow-auto border border-line/60 rounded">
                      {kbOptions.length === 0 ? (
                        <div className="text-[10px] text-fg-mute px-2 py-1">no matches</div>
                      ) : (
                        kbOptions.map((o) => (
                          <button
                            key={o.id}
                            type="button"
                            onClick={() => patch({ existingEntityId: o.id })}
                            className={`w-full text-left px-2 py-1 text-[11px] font-mono flex items-center gap-2 hover:bg-white/5 focus:outline-none focus:bg-white/5 ${
                              entity === o.id ? "bg-accent/15 text-accent" : ""
                            }`}
                          >
                            <span>{entity === o.id ? "◉" : "○"}</span>
                            <span>{o.id}</span>
                            {o.inFishVocab && <Pill tone="accent">vocab</Pill>}
                            {o.aliases.length > 0 && (
                              <span className="text-[9px] text-fg-mute truncate">
                                aka {o.aliases.join(", ")}
                              </span>
                            )}
                          </button>
                        ))
                      )}
                    </div>
                    {entity && (
                      <div className="text-[10px] text-fg-mute">selected: {entity}</div>
                    )}
                  </div>
                )}

                {decision === "VARIANT_OF_EXISTING" && (
                  <label className="text-[10px] text-fg-dim flex flex-col gap-1">
                    Variant descriptor (how this differs)
                    <input
                      className="bg-black/40 border border-line/70 rounded px-2 py-1 text-[11px]"
                      value={variantName}
                      placeholder="golden colour"
                      onChange={(e) => patch({ variantDescriptor: e.target.value })}
                    />
                  </label>
                )}

                {decision === "NEW_ENTITY" && (
                  <div className="flex flex-col gap-1">
                    <div className="flex items-center gap-2">
                      <Pill tone="warn">PROPOSAL ONLY</Pill>
                      <Pill tone="bad">NOT YET IN KNOWLEDGE BASE</Pill>
                    </div>
                    <label className="text-[10px] text-fg-dim flex flex-col gap-1">
                      Proposed canonical name (you must confirm this — the suggestion is never
                      accepted for you)
                      <input
                        className="bg-black/40 border border-line/70 rounded px-2 py-1 text-[11px]"
                        /* PLACEHOLDER, not value. The suggestion is shown so the
                           reviewer can accept it by typing; it is never written into
                           the field, because a prefilled proposed name would let a
                           reviewer confirm a name they never read. */
                        value={proposedName}
                        placeholder={suggestProposedName(concept.displayName)}
                        onChange={(e) => patch({ proposedEntityName: e.target.value })}
                      />
                    </label>
                    <div className="text-[10px] text-fg-mute">
                      would be added by hand, after review, to knowledge.rs and FISH_VOCAB.txt.
                      Nothing happens automatically.
                    </div>
                  </div>
                )}

                {decision === "UNKNOWN" && (
                  <div className="flex flex-col gap-1">
                    <div className="text-[10px] text-fg-dim">Reason (optional, one click)</div>
                    <div className="flex flex-wrap gap-1.5">
                      {UNKNOWN_REASONS.map((r) => (
                        <button
                          key={r}
                          type="button"
                          onClick={() =>
                            patch({ unknownReason: draft.unknownReason === r ? "" : r })
                          }
                          className={`rounded border px-2 py-0.5 text-[10px] focus:outline-none focus:ring-1 focus:ring-accent ${
                            draft.unknownReason === r ? "border-accent bg-accent/15" : "border-line/70"
                          }`}
                        >
                          {r}
                        </button>
                      ))}
                    </div>
                  </div>
                )}

                <label className="text-[10px] text-fg-dim flex flex-col gap-1">
                  Notes
                  <input
                    className="bg-black/40 border border-line/70 rounded px-2 py-1 text-[11px]"
                    value={draft.notes}
                    placeholder="optional"
                    onChange={(e) => patch({ notes: e.target.value })}
                  />
                </label>

                <div className="flex items-center gap-2">
                  <Button
                    kind={canSave ? "primary" : "default"}
                    disabled={!canSave || saveState === "saving"}
                    onClick={() => void save()}
                  >
                    {saveState === "saving" ? "Saving…" : "Confirm decision (Enter)"}
                  </Button>
                  {problems.length > 0 && (
                    <span className="text-[10px] text-fg-mute">{problems.join("; ")}</span>
                  )}
                  <Button kind="ghost" onClick={() => goTo(nextId)} disabled={!nextId}>
                    Skip (N)
                  </Button>
                </div>
              </div>
            </div>
          </div>

          {/* ---------------- side column ---------------- */}
          <div className="flex flex-col gap-2 min-w-0">
            <StructuralPanel
              structural={structural}
              concept={concept}
              onJump={(id) => goTo(id)}
            />
            {bandRemaining === 0 && bandTotal > 0 && (
              <div className="rounded-lg border border-ok/40 px-2 py-2 text-[11px]">
                <div className="font-semibold text-ok mb-1">{band} COMPLETE</div>
                <div className="text-fg-dim">
                  Run validation when you are ready:
                </div>
                <pre className="mt-1 bg-black/50 border border-line/60 rounded p-1.5 text-[9px] overflow-x-auto">
                  python -m ml.gpo_ontology_review validate
                </pre>
                <div className="flex gap-1.5 mt-1.5">
                  <Button size="sm" kind="ghost" onClick={() => setBand("P1")}>
                    Review P1 ({status.remaining.P1 ?? 0})
                  </Button>
                  <Button size="sm" kind="ghost" onClick={() => setBand("ALL")}>
                    Review all
                  </Button>
                </div>
              </div>
            )}
            <QueuePanel status={status} band={band} conceptBand={conceptBand} current={concept?.conceptId} onJump={(id) => goTo(id)} />
          </div>
        </div>
      )}

      {viewer !== null && concept && (
        <ImageViewer
          evidence={concept.evidence}
          index={viewer}
          onIndex={setViewer}
          onClose={() => setViewer(null)}
          conceptName={concept.displayName}
        />
      )}
      {familyOpen && concept?.familyId && (
        <FamilyPanel familyId={concept.familyId} onJump={(id) => { setFamilyOpen(false); goTo(id); }} onClose={() => setFamilyOpen(false)} />
      )}
    </Section>
  );
}

// ---------------------------------------------------------------------------
// Side panels
// ---------------------------------------------------------------------------

function StructuralPanel({
  structural,
  concept,
  onJump,
}: {
  structural: Structural | null;
  concept: WorkstationConcept | null;
  onJump: (conceptId: string) => void;
}) {
  if (!structural) return null;

  const modifiers = useMemo(() => {
    if (!concept) return [];
    const mine = new Set([...(concept.modifiers ?? []), concept.baseName].filter(Boolean) as string[]);
    return structural.modifiers.filter((m) => mine.has(m.modifier)).slice(0, 3);
  }, [structural, concept]);

  const family = useMemo(() => {
    if (!concept?.familyId) return null;
    return structural.families.find((f) => f.familyId === concept.familyId) ?? null;
  }, [structural, concept]);

  const bare = useMemo(() => {
    if (!concept?.baseName) return null;
    return structural.bases.find((b) => b.baseName === concept.baseName) ?? null;
  }, [structural, concept]);

  if (!concept) return null;

  return (
    <div className="rounded-lg border border-line/70 px-2.5 py-2 flex flex-col gap-2">
      <div className="text-[9px] uppercase tracking-wide text-fg-dim">Structural signals</div>
      <div className="text-[9px] text-fg-mute leading-tight">{structural.disclaimer}</div>

      {modifiers.map((m) => (
        <div key={m.modifier} className="text-[11px]">
          <div className="flex items-baseline gap-2">
            <span className="font-semibold">{m.modifier}</span>
            <span className="text-fg-mute">{m.occurrences} rows</span>
          </div>
          <div className="text-[9px] text-fg-mute">appears in:</div>
          <ul className="ml-3 list-disc">
            {m.concepts.slice(0, 4).map((c) => (
              <li key={c.conceptId}>
                <button type="button" className="underline hover:text-accent" onClick={() => onJump(c.conceptId)}>
                  {c.displayName}
                </button>
              </li>
            ))}
          </ul>
          <div className="text-[9px] text-fg-mute mt-0.5">
            possible role: MODIFIER or ENTITY — undecided
          </div>
        </div>
      ))}

      {family && (
        <div className="text-[11px] border-t border-line/60 pt-2">
          <div className="flex items-baseline gap-2">
            <span className="font-semibold">{family.baseName} family</span>
            <span className="text-fg-mute">{family.occurrences} rows</span>
          </div>
          <ul className="ml-3 list-disc">
            {family.members.slice(0, 5).map((mm) => (
              <li key={mm.conceptId}>
                <button type="button" className="underline hover:text-accent" onClick={() => onJump(mm.conceptId)}>
                  {mm.displayName}
                </button>{" "}
                <span className="text-fg-mute">{mm.occurrences}</span>
              </li>
            ))}
          </ul>
          <div className="text-[9px] text-fg-mute mt-0.5">
            one entity or several? — undecided
          </div>
        </div>
      )}

      {bare && (
        <div className="text-[10px] border-t border-line/60 pt-2 text-fg-mute">
          Base <span className="font-semibold text-fg">{bare.baseName}</span>:{" "}
          {bare.occurrences} rows across {bare.conceptCount} observed name(s);{" "}
          {bare.occursBare ? "does appear bare" : <span className="text-fg">never appears bare</span>}.
          {bare.kbCandidates.length > 0 && (
            <> Current KB: <span className="font-mono">{bare.kbCandidates.join(", ")}</span>.</>
          )}
        </div>
      )}
    </div>
  );
}

function QueuePanel({
  status,
  band,
  conceptBand,
  current,
  onJump,
}: {
  status: WorkstationStatus;
  band: string;
  conceptBand: Map<string, string>;
  current?: string;
  onJump: (id: string) => void;
}) {
  const items = queueForBand(status.queue ?? [], band, conceptBand).slice(0, 40);
  return (
    <div className="rounded-lg border border-line/70 px-2.5 py-2 flex flex-col gap-1 min-w-0">
      <div className="text-[9px] uppercase tracking-wide text-fg-dim">
        Queue {band} ({items.length})
      </div>
      <div className="flex flex-col gap-0.5 max-h-[38vh] overflow-auto">
        {items.map((id, i) => (
          <button
            key={id}
            type="button"
            onClick={() => onJump(id)}
            className={`text-left text-[10px] px-1 py-0.5 rounded truncate hover:bg-white/5 focus:outline-none focus:bg-white/5 ${
              id === current ? "text-accent" : "text-fg-dim"
            }`}
          >
            {i + 1}. {id.replace("concept:", "")}
          </button>
        ))}
        {items.length === 0 && <div className="text-[10px] text-fg-mute">empty</div>}
      </div>
    </div>
  );
}