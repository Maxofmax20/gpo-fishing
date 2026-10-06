import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "../lib/ipc";
import { showToast } from "../lib/store";
import { useVisiblePoll } from "../lib/useVisiblePoll";
import { Button, CustomSelect, Kbd, Pill, Section, Toggle } from "../components/primitives";
import ConfirmModal from "../components/ConfirmModal";
import type {
  BackendStatus,
  CandidateRecord,
  CompareView,
  ClassReadinessStatus,
  DatasetExplorer,
  DropEntry,
  EntityHit,
  HistoryEntry,
  ModelReadiness,
  PriorityItem,
  PrioritySort,
  ReadinessStage,
  ReadinessStatus,
  ReviewCoverageView,
  ReviewImage,
  ReviewIntegrity,
  ReviewRecord,
  TrainingJob,
  TrainingOverview,
  TrainingSettings,
} from "../lib/types";

type SubTab =
  | "overview" | "dataset" | "fish" | "fruits" | "jobs"
  | "candidates" | "models" | "compare" | "review" | "coverage"
  | "drops" | "readiness" | "history" | "settings";

const TABS: { id: SubTab; label: string }[] = [
  { id: "overview", label: "Overview" },
  { id: "dataset", label: "Dataset" },
  { id: "fish", label: "Fish" },
  { id: "fruits", label: "Fruits" },
  { id: "jobs", label: "Jobs" },
  { id: "candidates", label: "Candidates" },
  { id: "models", label: "Models" },
  { id: "compare", label: "Compare" },
  { id: "review", label: "Review" },
  { id: "coverage", label: "Coverage" },
  { id: "drops", label: "Drops" },
  { id: "readiness", label: "Readiness" },
  { id: "history", label: "History" },
  { id: "settings", label: "Settings" },
];

function fmtTs(ms: number | null | undefined): string {
  if (!ms) return "—";
  return new Date(ms).toLocaleString();
}

function fmtDur(s: number | null | undefined): string {
  if (s == null) return "N/A";
  if (s < 60) return `${Math.round(s)}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${Math.round(s % 60)}s`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

// ---------------------------------------------------------------------------
// Review queue
// ---------------------------------------------------------------------------

/** Backend `review_priority` clamp is 1..=500 and its default is 200. */
const QUEUE_LIMIT = 200;
/** Downscaled crop: enough to judge OCR text, cheap enough to move through
 *  hundreds of rows. Matches `region_preview`'s convention. */
const REVIEW_IMAGE_DIM = 480;

const SORT_OPTIONS: { value: PrioritySort; label: string }[] = [
  { value: "priority", label: "Priority (backend score)" },
  { value: "newest", label: "Newest first" },
  { value: "oldest", label: "Oldest first" },
  { value: "least_confidence", label: "Lowest model confidence" },
  { value: "most_confidence", label: "Highest model confidence" },
  { value: "rarest", label: "Rarest / no OCR text" },
];

/** `review_priority` has no status parameter, so a status filter switches the
 *  queue source to `review_list` (which filters server-side on the serde
 *  SCREAMING_SNAKE_CASE names) instead of filtering rows in the browser. */
const STATUS_OPTIONS: { value: "" | PriorityQueueStatus; label: string }[] = [
  { value: "", label: "Pending queue (all)" },
  { value: "UNREVIEWED", label: "UNREVIEWED" },
  { value: "CONFLICT", label: "CONFLICT" },
  { value: "REVIEWED_CORRECT", label: "REVIEWED_CORRECT" },
  { value: "REVIEWED_CORRECTED", label: "REVIEWED_CORRECTED" },
  { value: "REVIEWED_UNKNOWN", label: "REVIEWED_UNKNOWN" },
  { value: "REVIEWED_SKIPPED", label: "REVIEWED_SKIPPED" },
];

type PriorityQueueStatus =
  | "UNREVIEWED" | "CONFLICT" | "REVIEWED_CORRECT" | "REVIEWED_CORRECTED"
  | "REVIEWED_UNKNOWN" | "REVIEWED_SKIPPED";

/** One display row in the workbench. Backed either by `review_priority`
 *  (`score` present, full reason text) or by `review_list` (no priority
 *  score exists for those rows, so it is `null` rather than invented). */
type QueueRow = {
  image_id: string;
  session_id: string;
  timestamp_ms: number;
  ocr_text: string;
  entity_id: string | null;
  review_status: string;
  score: number | null;
  reason: string;
  is_hard_example: boolean;
  ocr_disagreement: boolean;
  model_confidence: number | null;
  from_status_filter: boolean;
};

function fromPriorityItem(p: PriorityItem): QueueRow {
  return {
    image_id: p.image_id,
    session_id: p.session_id,
    timestamp_ms: p.timestamp_ms,
    ocr_text: p.ocr_text,
    entity_id: p.entity_id,
    review_status: p.review_status,
    score: p.score,
    reason: p.reason,
    is_hard_example: p.is_hard_example,
    ocr_disagreement: p.ocr_disagreement,
    model_confidence: p.model_confidence,
    from_status_filter: false,
  };
}


/** Types the target of a keydown/click as an element, for the text-entry
 *  guard. Returns null for anything that is not a DOM node. */
function el(target: EventTarget | null): HTMLElement | null {
  return target instanceof HTMLElement ? target : null;
}

/**
 * Where the cursor should land after a verdict.
 *
 * A normal verdict removes the row from the pending queue, so the next
 * un-reviewed item slides into the same index. A CONFLICT (or an UNREVIEWED
 * row when a status filter is active) stays queued, so we step past it.
 */
function advanceFrom(items: QueueRow[], cursor: number, reviewedId: string): number {
  if (items.length === 0) return 0;
  const at = items[cursor];
  const staysQueued = at?.image_id === reviewedId;
  return Math.max(0, Math.min(cursor + (staysQueued ? 1 : 0), items.length - 1));
}

/** True when the event originated in something the user is typing into.
 *  Without this, typing an entity id in the correction field would fire
 *  verdicts. */
function isTextEntryTarget(target: EventTarget | null): boolean {
  const t = el(target);
  if (!t) return false;
  const tag = t.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || t.isContentEditable;
}

// ---------------------------------------------------------------------------
// Readiness presentation
// ---------------------------------------------------------------------------

const STAGE_ORDER: ReadinessStage[] = ["DATA", "REVIEW", "TRAIN", "EVALUATE", "SHADOW", "PRODUCTION"];

const STAGE_LABEL: Record<ReadinessStage, string> = {
  DATA: "Data gate",
  REVIEW: "Human review",
  TRAIN: "Training",
  EVALUATE: "Evaluation",
  SHADOW: "Shadow soak",
  PRODUCTION: "Production",
};

const STAGE_BLURB: Record<ReadinessStage, string> = {
  DATA: "Is there enough qualified, session-diverse, TEST-covered data?",
  REVIEW: "Is enough of it human-verified to learn from?",
  TRAIN: "Is a trainer running right now?",
  EVALUATE: "Does an evaluated candidate clear the absolute quality bars?",
  SHADOW: "Is a deployed revision soaking with enough events, agreement and session spread?",
  PRODUCTION: "Structural: no production path exists in this build.",
};

/** Statuses that mean "a human may start training on this". */
const READY_STATUSES: ReadinessStatus[] = ["DATA_READY", "CANDIDATE_READY", "SHADOW_READY", "PRODUCTION_READY"];

function isReadyStatus(s: ReadinessStatus): boolean {
  return READY_STATUSES.includes(s);
}

/** Distinct tone per status - the v5.6.0 UI painted every non-production
 *  status the same amber, which told the reviewer nothing. */
function statusTone(s: ReadinessStatus): "ok" | "warn" | "bad" | "accent" | "mute" | "fruit" {
  switch (s) {
    case "NOT_ENOUGH_DATA":
      return "bad";
    case "NOT_ENOUGH_CLASSES":
      return "bad";
    case "NOT_ENOUGH_SESSIONS":
      return "bad";
    case "NOT_ENOUGH_REVIEW":
      return "warn";
    case "NOT_ENOUGH_TEST":
      return "warn";
    case "DATA_READY":
      return "accent";
    case "TRAINING":
    case "EVALUATING":
      return "accent";
    case "CANDIDATE_READY":
      return "fruit";
    case "SHADOW_READY":
    case "PRODUCTION_READY":
      return "ok";
    case "NOT_READY":
      return "mute";
  }
}

/** Plain-English sentence for each status, so the pill is never the only
 *  signal. These mirror the documented meaning of each enum variant. */
const STATUS_MEANING: Record<ReadinessStatus, string> = {
  NOT_ENOUGH_DATA: "No class qualifies yet — nothing to learn from.",
  NOT_ENOUGH_CLASSES: "Some classes qualify, fewer than the gate requires.",
  NOT_ENOUGH_SESSIONS: "Qualified classes, but one appears in too few sessions.",
  NOT_ENOUGH_REVIEW: "Data qualifies, but too little of it is human-verified.",
  NOT_ENOUGH_TEST: "Qualified classes lack held-out TEST coverage.",
  DATA_READY: "Data and review both pass — training may start.",
  TRAINING: "A training job is running right now.",
  EVALUATING: "A candidate is being evaluated right now.",
  CANDIDATE_READY: "An evaluated candidate clears the quality bars; shadow soak is not complete.",
  SHADOW_READY: "Deployed to shadow with enough events, agreement and sessions.",
  PRODUCTION_READY: "Production-authorized (no such mechanism exists in this build).",
  NOT_READY: "Not ready — see the failing checks.",
};

const CLASS_TONE: Record<ClassReadinessStatus, "ok" | "warn" | "bad" | "mute"> = {
  READY: "ok",
  INSUFFICIENT_REVIEW: "warn",
  INSUFFICIENT_SESSIONS: "warn",
  INSUFFICIENT_TEST: "warn",
  NO_DATA: "mute",
};

export default function TrainingCenter() {
  const [tab, setTab] = useState<SubTab>("overview");
  const [overview, setOverview] = useState<TrainingOverview | null>(null);
  const [explorer, setExplorer] = useState<DatasetExplorer | null>(null);
  const [jobs, setJobs] = useState<TrainingJob[]>([]);
  const [candidates, setCandidates] = useState<CandidateRecord[]>([]);
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  const [coverage, setCoverage] = useState<ReviewCoverageView | null>(null);
  const [queue, setQueue] = useState<QueueRow[]>([]);
  const [queueTruncated, setQueueTruncated] = useState(false);
  /** Rows matching the current filter, before the request limit. */
  const [queueTotal, setQueueTotal] = useState(0);
  const [integrity, setIntegrity] = useState<ReviewIntegrity | null>(null);
  const [drops, setDrops] = useState<DropEntry[]>([]);
  const [readiness, setReadiness] = useState<ModelReadiness[]>([]);
  const [cursor, setCursor] = useState(0);
  const [selPng, setSelPng] = useState<ReviewImage | null>(null);
  const [imgError, setImgError] = useState<string | null>(null);
  const [selRecord, setSelRecord] = useState<ReviewRecord | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchHits, setSearchHits] = useState<EntityHit[]>([]);
  const [correctEntity, setCorrectEntity] = useState("");
  const [resolveReason, setResolveReason] = useState("");
  const [fStatus, setFStatus] = useState<"" | PriorityQueueStatus>("");
  const [fEntity, setFEntity] = useState("");
  const [fSession, setFSession] = useState("");
  const [fHard, setFHard] = useState(false);
  const [fDisagree, setFDisagree] = useState(false);
  const [fSort, setFSort] = useState<PrioritySort>("priority");
  const [settings, setSettings] = useState<TrainingSettings | null>(null);
  const [backend, setBackend] = useState<BackendStatus | null>(null);
  const [compare, setCompare] = useState<CompareView | null>(null);
  const [compareId, setCompareId] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<{ title: string; body: string; label: string; run: () => Promise<void> } | null>(null);
  /** Result of the last verdict, so the reviewer sees CORRECT vs CORRECTED
   *  (and a dataset write failure) after the workbench auto-advances. */
  const [lastVerdict, setLastVerdict] = useState<{
    image_id: string;
    review_status: string;
    dataset_updated: boolean | null;
    dataset_error: string | null;
  } | null>(null);

  const correctRef = useRef<HTMLInputElement>(null);
  /** Read inside the keydown listener without re-registering it. */
  const modalOpenRef = useRef(false);
  modalOpenRef.current = confirm !== null;
  /** Synchronous busy latch. React state updates are async, so a double
   *  click could otherwise land two `review_apply` calls and manufacture a
   *  phantom CONFLICT. */
  const busyRef = useRef(false);

  const selected: QueueRow | null = queue[cursor] ?? null;
  const reviewing = tab === "review";

  const entityOptions = useMemo(
    () => (coverage?.per_entity ?? []).map((e) => ({ entity: e.entity, n: e.collected, r: e.reviewed })),
    [coverage],
  );
  const sessionOptions = useMemo(
    () => (coverage?.per_session ?? []).map((s) => ({ id: s.session_id, n: s.collected, split: s.split })),
    [coverage],
  );

  // -------------------------------------------------------------------------
  // Refresh tiers. `refreshSlow` is deliberately NOT called after a verdict:
  // it is 7 commands that read whole datasets.
  // -------------------------------------------------------------------------
  const refreshAll = useCallback(async () => {
    try {
      const [o, j, c, h] = await Promise.all([
        api.trainingOverview(),
        api.trainingJobs(),
        api.trainingCandidates(),
        api.trainingHistory(100),
      ]);
      setOverview(o);
      setJobs(j);
      setCandidates(c);
      setHistory(h);
    } catch (e) {
      showToast("error", "Training Center refresh" + ": " + String(e));
    }
  }, []);

  /**
   * The one place the queue is fetched. Every filter - including the status
   * filter - is pushed to `review_priority`, which also returns the true
   * `total_matching` before `limit` was applied. Nothing is filtered in the
   * browser.
   */
  const fetchQueue = useCallback(async (): Promise<{ rows: QueueRow[]; total: number }> => {
    const page = await api.reviewPriority({
      limit: QUEUE_LIMIT,
      entity: fEntity || undefined,
      session: fSession || undefined,
      onlyHard: fHard,
      onlyDisagreement: fDisagree,
      sort: fSort,
      status: fStatus || undefined,
    });
    return { rows: page.items.map(fromPriorityItem), total: page.total_matching };
  }, [fStatus, fEntity, fSession, fHard, fDisagree, fSort]);

  const applyQueue = useCallback((rows: QueueRow[], total: number) => {
    setQueue(rows);
    setQueueTotal(total);
    setQueueTruncated(total > rows.length);
  }, []);

  /** Queue + coverage. This is the only tier a human verdict triggers —
   *  `refreshSlow` reads seven whole-dataset commands. */
  const refreshQueue = useCallback(async (): Promise<QueueRow[]> => {
    try {
      const [{ rows, total }, cov] = await Promise.all([fetchQueue(), api.reviewCoverage()]);
      applyQueue(rows, total);
      setCoverage(cov);
      return rows;
    } catch (e) {
      showToast("error", "Review queue" + ": " + String(e));
      return [];
    }
  }, [fetchQueue, applyQueue]);

  const refreshIntegrity = useCallback(async () => {
    try {
      setIntegrity(await api.reviewIntegrity(10));
    } catch (e) {
      showToast("error", "Review integrity" + ": " + String(e));
    }
  }, []);

  const refreshSlow = useCallback(async () => {
    try {
      const [e, s, b, cov, dr, rd, ig] = await Promise.all([
        api.datasetExplorer(),
        api.trainingSettingsGet(),
        api.trainingBackend(),
        api.reviewCoverage(),
        api.dropsExplorer(),
        api.readinessStatus(),
        api.reviewIntegrity(10),
      ]);
      setExplorer(e);
      setSettings(s);
      setBackend(b);
      setCoverage(cov);
      setDrops(dr);
      setReadiness(rd);
      setIntegrity(ig);
    } catch (e) {
      showToast("error", "Training Center details" + ": " + String(e));
    }
  }, []);

  useEffect(() => {
    refreshAll();
    refreshSlow();
  }, [refreshAll, refreshSlow]);

  // Full detail tier on sub-tab change (skipped on the first render, which
  // already refreshed above). This is also the settings-save path, because
  // the bars edited there feed readiness_status.
  const firstTab = useRef(true);
  useEffect(() => {
    if (firstTab.current) {
      firstTab.current = false;
      return;
    }
    void refreshSlow();
  }, [tab, refreshSlow]);

  // Re-query the backend whenever a queue filter changes, and on first mount.
  useEffect(() => {
    let live = true;
    (async () => {
      try {
        const { rows, total } = await fetchQueue();
        if (!live) return;
        applyQueue(rows, total);
        // A filter change redefines the list, so the old index is meaningless.
        setCursor(0);
        setLastVerdict(null);
      } catch (e) {
        if (live) showToast("error", "Review queue" + ": " + String(e));
      }
    })();
    return () => {
      live = false;
    };
  }, [fetchQueue, applyQueue]);

  const anyActive = jobs.some((j) => ["QUEUED", "RUNNING", "EVALUATING"].includes(j.status));
  // Poll only while a job is live. On mount `anyActive` is false (jobs start
  // empty), so useVisiblePoll's immediate call does not duplicate the mount
  // refresh above.
  useVisiblePoll(refreshAll, anyActive ? 3000 : 15000, anyActive);

  async function run(label: string, fn: () => Promise<unknown>, okMsg?: string) {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(label);
    try {
      await fn();
      if (okMsg) showToast("info", okMsg);
      await refreshAll();
      await refreshSlow();
    } catch (e) {
      showToast("error", `${label}: ${String(e)}`);
    } finally {
      busyRef.current = false;
      setBusy(null);
    }
  }

  const startTrain = (family: string) =>
    run(`train-${family}`, () => api.trainingStart(family), `${family} training started`);

  const cancelJob = (id: string) =>
    run("cancel", () => api.trainingCancel(id), "Job cancelled");

  const decideJob = (id: string) =>
    run("decide", async () => {
      const outcome = await api.trainingDecide(id);
      showToast("info", outcome);
    });

  const askPromote = (id: string) =>
    setConfirm({
      title: "Promote candidate to shadow?",
      body: "The candidate already passed the deterministic comparison gate. Promotion archives the running shadow model (rollback point) and deploys the candidate for observation only. Production control stays OFF.",
      label: "Promote to shadow",
      run: async () => {
        const msg = await api.trainingPromote(id);
        showToast("info", msg);
        setConfirm(null);
        await refreshAll();
      },
    });

  const askRollback = (family: string) =>
    setConfirm({
      title: `Roll back ${family} shadow model?`,
      body: "Restores the newest archived shadow files after re-verifying them. The displaced version is recorded as ROLLED_BACK.",
      label: "Roll back",
      run: async () => {
        const msg = await api.trainingRollback(family);
        showToast("info", msg);
        setConfirm(null);
        await refreshAll();
      },
    });

  const loadCompare = async (id: string) => {
    setCompareId(id);
    if (!id) {
      setCompare(null);
      return;
    }
    try {
      setCompare(await api.trainingCompare(id));
    } catch (e) {
      showToast("error", "Compare" + ": " + String(e));
    }
  };

  const goTo = useCallback((idx: number) => {
    setCursor(queue.length === 0 ? 0 : Math.max(0, Math.min(idx, queue.length - 1)));
  }, [queue.length]);

  const nextItem = useCallback(() => goTo(cursor + 1), [goTo, cursor]);
  const prevItem = useCallback(() => goTo(cursor - 1), [goTo, cursor]);

  // Load the crop + existing record whenever the selection changes. The
  // abort flag stops a slow earlier request from overwriting a newer one.
  // Selection is derived from `queue[cursor]`, so keying on the image id is
  // both correct and avoids re-fetching on unrelated re-renders.
  useEffect(() => {
    const id = reviewing ? selected?.image_id : undefined;
    if (!id) {
      setSelPng(null);
      setSelRecord(null);
      setImgError(null);
      return;
    }
    let live = true;
    setSelPng(null);
    setSelRecord(null);
    setImgError(null);
    (async () => {
      try {
        const [png, rec] = await Promise.all([
          api.reviewImage(id, REVIEW_IMAGE_DIM),
          api.reviewGet(id),
        ]);
        if (!live) return;
        setSelPng(png);
        setSelRecord(rec);
      } catch (e) {
        if (!live) return;
        setImgError(String(e));
      }
    })();
    return () => {
      live = false;
    };
  }, [selected?.image_id, reviewing]);

  // Seed the correction field from the row's own entity, and drop the search
  // results so a hit list from the previous image cannot be submitted.
  useEffect(() => {
    setCorrectEntity(selected?.entity_id ?? "");
    setSearchQuery("");
    setSearchHits([]);
    setResolveReason("");
  }, [selected?.image_id]);

  const runSearch = async (q: string) => {
    setSearchQuery(q);
    if (q.trim().length < 2) {
      setSearchHits([]);
      return;
    }
    try {
      setSearchHits(await api.reviewSearch(q.trim()));
    } catch (e) {
      showToast("error", "Entity search" + ": " + String(e));
    }
  };

  const undoSelected = useCallback(async () => {
    if (!selected) {
      showToast("info", "Nothing selected to undo");
      return;
    }
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy("undo");
    try {
      const rec = await api.reviewUndo(selected.image_id);
      setLastVerdict({
        image_id: rec.image_id,
        review_status: rec.review_status,
        dataset_updated: null,
        dataset_error: null,
      });
      showToast("info", `Undo: ${rec.image_id} is now ${rec.review_status}`);
      // The undone row is pending again; jump straight back to it if the
      // queue still contains it, otherwise stay put.
      const items = await refreshQueue();
      const back = items.findIndex((q) => q.image_id === rec.image_id);
      setCursor(back >= 0 ? back : advanceFrom(items, cursor, rec.image_id));
    } catch (e) {
      showToast("error", "Undo" + ": " + String(e));
    } finally {
      busyRef.current = false;
      setBusy(null);
    }
  }, [selected, cursor, refreshQueue]);

  /**
   * Record a human verdict, then advance to the next un-reviewed item.
   *
   * `modelPrediction` is the row's dataset entity. Omitting it leaves the
   * backend unable to tell a confirmation from a correction, so every
   * "Correct" click lands as REVIEWED_CORRECTED.
   */
  const submitReview = useCallback(async (kind: "correct" | "change" | "unknown" | "skip" | "resolve") => {
    if (!selected) return;
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(kind);
    try {
      const modelPrediction = selected.entity_id ?? undefined;
      const modelConfidence = selected.model_confidence ?? undefined;
      let status: string;
      let datasetUpdated: boolean | null = null;
      let datasetError: string | null = null;

      if (kind === "skip") {
        status = (await api.reviewSkip(selected.image_id)).review_status;
      } else if (kind === "resolve") {
        const entity = correctEntity.trim();
        const reason = resolveReason.trim();
        if (!entity) {
          showToast("error", "Resolve needs a canonical entity id");
          return;
        }
        if (!reason) {
          showToast("error", "Resolve needs a reason — it goes into the audit trail");
          return;
        }
        status = (await api.reviewResolve(selected.image_id, entity, reason)).review_status;
      } else {
        // "unknown" deliberately sends NO human entity — the backend records
        // REVIEWED_UNKNOWN. It must not fall back to the pre-filled field.
        const id = kind === "unknown" ? undefined : kind === "correct" ? selected.entity_id : correctEntity.trim();
        if (kind !== "unknown" && !id) {
          showToast("error", "No entity to confirm — pick one from search, or mark UNKNOWN");
          return;
        }
        const res = await api.reviewApply(selected.image_id, {
          humanEntityId: id ?? undefined,
          modelPrediction,
          modelConfidence,
          ...(kind === "change" ? { correctionReason: "human correction from review workbench" } : {}),
        });
        status = res.record.review_status;
        datasetUpdated = res.dataset_updated;
        datasetError = res.dataset_error;
      }

      setLastVerdict({
        image_id: selected.image_id,
        review_status: status,
        dataset_updated: datasetUpdated,
        dataset_error: datasetError,
      });
      if (datasetError) {
        showToast("error", `Label recorded as ${status} but the dataset write FAILED: ${datasetError}`);
      } else {
        showToast("info", `Recorded ${status} — ${selected.image_id}`);
      }

      // Auto-advance: the verdict drops this row from the pending queue, so
      // re-reading the list leaves the cursor pointing at the next
      // un-reviewed item. The image just reviewed is NOT re-fetched.
      const items = await refreshQueue();
      setCursor((c) => advanceFrom(items, c, selected.image_id));
    } catch (e) {
      showToast("error", "Review" + ": " + String(e));
    } finally {
      busyRef.current = false;
      setBusy(null);
    }
  }, [selected, correctEntity, resolveReason, refreshQueue]);

  const rebuildFromAudit = useCallback(async () => {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy("rebuild");
    try {
      const n = await api.reviewRebuild();
      showToast("info", `Rebuilt review state from ${n} audit record(s)`);
      await refreshQueue();
      await refreshIntegrity();
    } catch (e) {
      showToast("error", "Rebuild from audit" + ": " + String(e));
    } finally {
      busyRef.current = false;
      setBusy(null);
    }
  }, [refreshQueue, refreshIntegrity]);

  const saveSettings = async () => {
    if (!settings) return;
    await run("settings", () => api.trainingSettingsSet(settings), "Training settings saved");
  };

  // -------------------------------------------------------------------------
  // Keyboard review. `KeyCapture` is deliberately NOT used: it capture-blocks
  // window keydown while listening, which would swallow these shortcuts.
  // Registered once per Training tab visit (the component only mounts while
  // the Training tab is shown) and removed on unmount or tab change.
  // -------------------------------------------------------------------------
  const onCorrection = useCallback(() => {
    // `c` opens the correction field; pressing it again with a value typed
    // submits, so a correction never needs the mouse.
    if (correctEntity.trim() && correctEntity.trim() !== selected?.entity_id) {
      void submitReview("change");
    } else {
      correctRef.current?.focus();
      correctRef.current?.select();
    }
  }, [correctEntity, selected, submitReview]);

  const kbRef = useRef({
    verdict: (kind: "correct" | "unknown" | "skip") => void submitReview(kind),
    correction: onCorrection,
    next: nextItem,
    prev: prevItem,
    undo: undoSelected,
  });
  kbRef.current = {
    verdict: (kind) => void submitReview(kind),
    correction: onCorrection,
    next: nextItem,
    prev: prevItem,
    undo: undoSelected,
  };

  useEffect(() => {
    if (!reviewing) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.repeat) return;
      if (e.ctrlKey || e.metaKey || e.altKey || e.shiftKey) return;
      if (isTextEntryTarget(e.target)) return;
      // A confirm dialog owns the keyboard while it is open.
      if (modalOpenRef.current) return;
      // F-keys belong to the backend global hotkeys (F1-F7) — never bind one.
      if (/^F\d+$/.test(e.key)) return;
      const kb = kbRef.current;
      switch (e.key) {
        case "Enter":
          e.preventDefault();
          kb.verdict("correct");
          break;
        case "c":
        case "C":
          e.preventDefault();
          kb.correction();
          break;
        case "u":
        case "U":
          e.preventDefault();
          kb.verdict("unknown");
          break;
        case "s":
        case "S":
          e.preventDefault();
          kb.verdict("skip");
          break;
        case "ArrowRight":
          e.preventDefault();
          kb.next();
          break;
        case "ArrowLeft":
          e.preventDefault();
          kb.prev();
          break;
        case "z":
        case "Z":
          e.preventDefault();
          kb.undo();
          break;
        default:
          break;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [reviewing]);

  const queuePosition = queue.length === 0 ? "empty" : `${cursor + 1} of ${queue.length}`;

  return (
    <div className="flex flex-col gap-3 px-4 pb-6">
      <div className="flex items-center justify-between pt-3">
        <div>
          <div className="text-sm font-semibold text-fg">ML Training Center</div>
          <div className="text-[11px] text-fg-dim">
            Real dataset, real jobs, real evaluations. Production control:{" "}
            <span className="font-mono font-bold text-ok">OFF</span>
          </div>
        </div>
        <Pill tone={overview?.shadow_enabled ? "ok" : "mute"}>
          SHADOW {overview?.shadow_enabled ? "ON" : "OFF"}
        </Pill>
      </div>

      <div className="flex gap-1.5 flex-wrap">
        {TABS.map((t) => (
          <Button key={t.id} size="sm" kind={tab === t.id ? "primary" : "ghost"} onClick={() => setTab(t.id)}>
            {t.label}
          </Button>
        ))}
      </div>

      {tab === "overview" && (
        <Section title="Learning status">
          {!overview ? (
            <div className="text-[11px] text-fg-mute">Loading…</div>
          ) : (
            <div className="text-[11px] font-mono text-fg-dim break-words flex flex-col gap-1">
              <div>
                Dataset v{overview.dataset_version} · {overview.rows} samples · {overview.sessions} sessions ·
                last collection {fmtTs(overview.last_collection_ms)} · new since training {overview.new_since_training}
              </div>
              {overview.models.map((m) => (
                <div key={m.name}>
                  {m.family}: {m.name} v{m.version} · acc {m.test_accuracy?.toFixed(4) ?? "N/A"} · F1{" "}
                  {m.macro_f1?.toFixed(4) ?? "N/A"}
                </div>
              ))}
              {overview.eligibility.map((e) => (
                <div key={e.family}>
                  {e.family} training:{" "}
                  <span className={e.eligible ? "text-ok" : "text-warn"}>{e.eligible ? "ELIGIBLE" : "BLOCKED"}</span>
                  {e.checks.filter((c) => !c.ok).map((c) => (
                    <div key={c.text} className="text-fg-mute">✗ {c.text}</div>
                  ))}
                </div>
              ))}
              <div>
                Auto-training: {overview.auto_enabled ? "ON" : "OFF"} · backend:{" "}
                {overview.backend_available ? "ready" : overview.backend_detail}
              </div>
              <div className="flex gap-2 flex-wrap mt-1 items-center">
                <Button size="sm" kind="primary" disabled={busy !== null} onClick={() => startTrain("fish")}>
                  Train Fish
                </Button>
                {/* `Button` has no "disabled" kind, and `pointer-events-none`
                    on a disabled button suppresses its own tooltip, so the
                    explanation lives in the wrapper's title + visible text. */}
                <span
                  title="No fruit trainer exists: training_start(&quot;fruit&quot;) has no trainer module and always returns an error."
                  className="inline-flex"
                >
                  <Button size="sm" disabled onClick={() => startTrain("fruit")}>
                    Train Fruit
                  </Button>
                </span>
                <Button size="sm" disabled={busy !== null} onClick={() => startTrain("state")}>
                  Train State
                </Button>
              </div>
              <div className="text-fg-mute">
                Train Fruit is unavailable: no fruit trainer module exists in this build, so the request always
                fails. Fruit readiness is tracked on the Readiness tab instead.
              </div>
              {!overview.backend_available && (
                <div className="text-warn">
                  Training backend unavailable: {overview.backend_detail}. Set python_path + trainer_dir in Settings.
                </div>
              )}
            </div>
          )}
        </Section>
      )}

      {tab === "dataset" && (
        <Section title="Dataset explorer">
          {!explorer ? (
            <div className="text-[11px] text-fg-mute">Loading…</div>
          ) : (
            <div className="text-[11px] font-mono text-fg-dim break-words flex flex-col gap-1">
              <div>
                {explorer.rows} rows · {explorer.sessions} sessions · {explorer.entities} entities · hard{" "}
                {explorer.hard_examples} · OCR-bearing {explorer.ocr_bearing}
              </div>
              <div>
                States: {Object.entries(explorer.states).map(([k, v]) => `${k}:${v}`).join(" · ")}
              </div>
              <div>
                RESULT: OCR-empty {explorer.ocr_empty_result} · entity-less {explorer.null_entity_result}
              </div>
            </div>
          )}
        </Section>
      )}

      {(tab === "fish" || tab === "fruits") && (
        <Section title={tab === "fish" ? "Fish coverage (per class)" : "Fruit coverage (per class)"}>
          {!explorer ? (
            <div className="text-[11px] text-fg-mute">Loading…</div>
          ) : (
            <div className="flex flex-col">
              {(tab === "fish" ? explorer.fish : explorer.fruits).map((e) => (
                <div key={e.entity} className="flex items-center gap-2 px-1 py-1 border-b border-line/50 text-[11px] font-mono">
                  <span className="w-44 truncate text-fg">{e.entity}</span>
                  <span className="text-fg-dim">n={e.examples} tr={e.train}/v={e.validation}/t={e.test}</span>
                  <span className="text-fg-dim">sess={e.sessions}</span>
                  <span className="text-fg-dim">ocr±={e.ocr_agree}/{e.ocr_disagree}</span>
                  <Pill tone={e.qualified ? "ok" : "warn"}>{e.qualified ? "qualified" : "collecting"}</Pill>
                  {!e.qualified && <span className="text-fg-mute truncate">{e.reason}</span>}
                </div>
              ))}
              {(tab === "fish" ? explorer.fish : explorer.fruits).length === 0 && (
                <div className="text-[11px] text-fg-mute">No visual examples yet — collect gameplay with trace on.</div>
              )}
            </div>
          )}
        </Section>
      )}

      {tab === "jobs" && (
        <Section title="Training jobs">
          <div className="flex flex-col gap-2">
            {jobs.length === 0 && <div className="text-[11px] text-fg-mute">No jobs yet.</div>}
            {jobs.map((j) => (
              <div key={j.job_id} className="rounded-lg border border-line/70 px-2.5 py-1.5 text-[11px]">
                <div className="flex items-center gap-2 flex-wrap">
                  <Pill tone={j.status === "PASSED" ? "ok" : j.status === "FAILED" || j.status === "REJECTED" ? "bad" : ["RUNNING", "EVALUATING"].includes(j.status) ? "accent" : "mute"}>
                    {j.status}
                  </Pill>
                  <span className="font-mono text-fg">{j.model_family}</span>
                  <span className="font-mono text-fg-mute truncate">{j.job_id}</span>
                  <span className="font-mono text-fg-mute">
                    ep {j.progress.epoch}/{j.progress.total_epochs}
                    {j.progress.val_metric != null && ` · val ${j.progress.val_metric.toFixed(4)}`}
                    {j.progress.eta_s != null && ` · ETA ${fmtDur(j.progress.eta_s)}`}
                  </span>
                </div>
                <div className="font-mono text-fg-dim break-words mt-1">
                  {j.progress.stage} · data {j.dataset_fingerprint} ({j.frozen_rows} rows) · by {j.requested_by}
                  {j.error && <span className="text-bad"> · {j.error.slice(0, 200)}</span>}
                  {j.candidate_id && <span> · candidate {j.candidate_id}</span>}
                </div>
                {j.progress.log_tail.length > 0 && (
                  <div className="font-mono text-[10px] text-fg-mute mt-1 break-words">
                    {j.progress.log_tail.slice(-2).map((l, i) => <div key={i}>{l.slice(0, 160)}</div>)}
                  </div>
                )}
                <div className="flex gap-1.5 mt-1.5 flex-wrap">
                  {["RUNNING", "EVALUATING", "QUEUED"].includes(j.status) && (
                    <Button size="sm" kind="danger" disabled={busy !== null} onClick={() => cancelJob(j.job_id)}>
                      Cancel
                    </Button>
                  )}
                  {j.status === "PASSED" && !j.candidate_id && (
                    <Button size="sm" kind="primary" disabled={busy !== null} onClick={() => decideJob(j.job_id)}>
                      Evaluate &amp; decide
                    </Button>
                  )}
                  {(j.status === "INTERRUPTED" || j.status === "FAILED" || j.status === "CANCELLED") && (
                    <Button size="sm" disabled={busy !== null} onClick={() => run("restart", () => api.trainingRestart(j.job_id), "Restarted with fresh snapshot")}>
                      Restart
                    </Button>
                  )}
                  {j.status === "INTERRUPTED" && (
                    <Button size="sm" kind="ghost" disabled={busy !== null} onClick={() => run("discard", () => api.trainingDiscard(j.job_id), "Discarded")}>
                      Discard
                    </Button>
                  )}
                </div>
              </div>
            ))}
          </div>
        </Section>
      )}

      {tab === "candidates" && (
        <Section title="Candidates">
          <div className="flex flex-col gap-2">
            {candidates.length === 0 && <div className="text-[11px] text-fg-mute">No candidates yet.</div>}
            {candidates.map((c) => (
              <div key={c.candidate_id} className="rounded-lg border border-line/70 px-2.5 py-1.5 text-[11px]">
                <div className="flex items-center gap-2 flex-wrap">
                  <Pill tone={c.status === "SHADOW" ? "ok" : c.status === "REJECTED" ? "bad" : c.status === "EVALUATED" ? "accent" : "mute"}>
                    {c.status}
                  </Pill>
                  <span className="font-mono text-fg">{c.candidate_id}</span>
                  <span className="font-mono text-fg-dim">
                    acc {c.metrics.accuracy.toFixed(4)} · F1 {c.metrics.macro_f1.toFixed(4)} · test {c.metrics.test_sessions}sess/{c.metrics.test_n}
                  </span>
                </div>
                {c.decision_reason && <div className="font-mono text-fg-mute mt-1 break-words">{c.decision_reason.slice(0, 300)}</div>}
                <div className="flex gap-1.5 mt-1.5 flex-wrap">
                  <Button size="sm" onClick={() => { loadCompare(c.candidate_id); setTab("compare"); }}>
                    Compare
                  </Button>
                  {c.status === "EVALUATED" && (
                    <Button size="sm" kind="primary" disabled={busy !== null} onClick={() => askPromote(c.candidate_id)}>
                      Promote to shadow
                    </Button>
                  )}
                </div>
              </div>
            ))}
          </div>
        </Section>
      )}

      {tab === "models" && (
        <Section title="Deployed models + rollback">
          <div className="flex flex-col gap-2">
            {(overview?.models ?? []).map((m) => (
              <div key={m.name} className="rounded-lg border border-line/70 px-2.5 py-1.5 text-[11px] font-mono">
                <div className="text-fg">{m.family}: {m.name} v{m.version}</div>
                <div className="text-fg-dim">
                  acc {m.test_accuracy?.toFixed(4) ?? "N/A"} · F1 {m.macro_f1?.toFixed(4) ?? "N/A"} · observation only
                </div>
                <div className="mt-1.5">
                  <Button size="sm" kind="danger" disabled={busy !== null} onClick={() => askRollback(m.family)}>
                    Roll back {m.family}
                  </Button>
                </div>
              </div>
            ))}
          </div>
        </Section>
      )}

      {tab === "compare" && (
        <Section title="Current vs candidate">
          <div className="flex gap-2 items-center mb-2">
            <select value={compareId} onChange={(e) => loadCompare(e.target.value)} className="rounded-lg bg-black/30 border border-line px-1.5 py-1 text-[11px] text-fg">
              <option value="">Select candidate…</option>
              {candidates.map((c) => <option key={c.candidate_id} value={c.candidate_id}>{c.candidate_id}</option>)}
            </select>
          </div>
          {!compare ? (
            <div className="text-[11px] text-fg-mute">Pick a candidate to compare against the deployed model.</div>
          ) : (
            <div className="text-[11px] font-mono text-fg-dim break-words flex flex-col gap-1">
              <div>
                Δacc {compare.comparison.acc_delta >= 0 ? "+" : ""}{compare.comparison.acc_delta.toFixed(4)} · ΔF1{" "}
                {compare.comparison.f1_delta >= 0 ? "+" : ""}{compare.comparison.f1_delta.toFixed(4)} · ΔECE{" "}
                {compare.comparison.ece_delta?.toFixed(4) ?? "N/A"}
              </div>
              <div>
                Verdict:{" "}
                <span className={compare.comparison.verdict === "Pass" ? "text-ok" : compare.comparison.verdict === "Reject" ? "text-bad" : "text-warn"}>
                  {compare.comparison.verdict}
                </span>
              </div>
              {compare.comparison.reasons.map((r) => <div key={r} className="text-fg-mute">{r.slice(0, 220)}</div>)}
              {compare.comparison.regressions.length > 0 && (
                <div className="text-bad">
                  Regressions: {compare.comparison.regressions.map(([c, a, b]) => `${c} ${a.toFixed(2)}→${b.toFixed(2)}`).join(", ")}
                </div>
              )}
              {compare.comparison.improvements.length > 0 && (
                <div className="text-ok">
                  Improvements: {compare.comparison.improvements.map(([c, a, b]) => `${c} ${a.toFixed(2)}→${b.toFixed(2)}`).join(", ")}
                </div>
              )}
            </div>
          )}
        </Section>
      )}

      {tab === "review" && (
        <>
          {/* Filters live in the Section `action` slot so they sit beside the
              queue title and apply to the backend query below them. */}
          <Section
            title="Review workbench — priority queue"
            action={
              <div className="flex items-center gap-1.5 flex-wrap justify-end">
                <CustomSelect
                  value={fStatus}
                  onChange={(v) => setFStatus(v)}
                  options={STATUS_OPTIONS}
                  className="max-w-[190px]"
                />
                <CustomSelect
                  value={fEntity}
                  onChange={setFEntity}
                  options={[
                    { value: "", label: "All entities" },
                    ...entityOptions.map((e) => ({
                      value: e.entity,
                      label: e.entity,
                      sub: `collected ${e.n} · reviewed ${e.r}`,
                    })),
                  ]}
                  className="max-w-[220px]"
                />
                <CustomSelect
                  value={fSession}
                  onChange={setFSession}
                  options={[
                    { value: "", label: "All sessions" },
                    ...sessionOptions.map((s) => ({
                      value: s.id,
                      label: s.id,
                      sub: `${s.split} · ${s.n} rows`,
                    })),
                  ]}
                  className="max-w-[220px]"
                />
                <CustomSelect value={fSort} onChange={setFSort} options={SORT_OPTIONS} className="max-w-[210px]" />
                <Button size="sm" kind={fHard ? "primary" : "ghost"} onClick={() => setFHard((v) => !v)}>
                  Hard only
                </Button>
                <Button size="sm" kind={fDisagree ? "primary" : "ghost"} onClick={() => setFDisagree((v) => !v)}>
                  OCR disagree
                </Button>
              </div>
            }
          >
            <div className="px-4 py-2 text-[11px] font-mono text-fg-dim flex flex-wrap items-center gap-x-3 gap-y-1">
              <span>
                {queue.length === 0
                  ? "Queue empty — nothing pending for this filter."
                  : queueTruncated
                    ? `showing first ${queue.length} (backend caps one request at ${QUEUE_LIMIT}; the true pending total is larger — narrow the filters)`
                    : `showing all ${queue.length} pending`}
              </span>
              <span className="text-fg-mute">position {queuePosition}</span>
              <span className="flex items-center gap-1">
                <Kbd>←</Kbd>
                <Kbd>→</Kbd> move
                <Kbd>Enter</Kbd> confirm
                <Kbd>c</Kbd> correct
                <Kbd>u</Kbd> unknown
                <Kbd>s</Kbd> skip
                <Kbd>z</Kbd> undo
              </span>
            </div>
            {fStatus ? (
              <div className="px-4 pb-2 text-[11px] text-warn">
                Status filter is served by review_list, which has no priority/filter parameters — entity, session,
                hard-example and OCR-disagreement filters do not apply to it.
              </div>
            ) : null}
            {integrity && integrity.unreadable_lines > 0 && (
              <div className="mx-4 mb-2 rounded-lg border border-warn/40 bg-warn-soft px-3 py-2 text-[11px] text-warn flex items-center justify-between gap-3 flex-wrap">
                <span className="break-words">
                  {integrity.unreadable_lines} unreadable line{integrity.unreadable_lines === 1 ? "" : "s"} in
                  reviews.jsonl. Those verdicts are being carried through untouched but are not counted anywhere.
                </span>
                <Button size="sm" disabled={busy !== null} onClick={rebuildFromAudit}>
                  Rebuild from audit
                </Button>
              </div>
            )}
          </Section>

          {selected && (
            <Section title={`Reviewing ${selected.image_id}`}>
              <div className="flex flex-col gap-2">
                {selPng ? (
                  <img
                    src={`data:image/png;base64,${selPng.png_base64}`}
                    alt={selected.image_id}
                    width={selPng.width}
                    height={selPng.height}
                    className="rounded-lg border border-line/70 max-w-full"
                    style={{ maxHeight: 320, width: "auto", height: "auto" }}
                  />
                ) : imgError ? (
                  <div className="rounded-lg border border-bad/40 bg-bad-soft px-3 py-2 text-[11px] text-bad break-words">
                    Image unavailable: {imgError}
                  </div>
                ) : (
                  <div className="text-[11px] text-fg-mute">Loading PNG…</div>
                )}
                <div className="text-[11px] font-mono text-fg-dim break-words">
                  <div>
                    Session {selected.session_id} ·{" "}
                    {selected.timestamp_ms
                      ? new Date(selected.timestamp_ms).toLocaleString()
                      : "no capture timestamp on this record"}
                  </div>
                  <div>OCR: {selected.ocr_text || "(none)"}</div>
                  <div className="flex items-center gap-1.5 flex-wrap">
                    <span>
                      Dataset entity: {selected.entity_id ?? "(none)"} · status {selected.review_status}
                    </span>
                    {selected.is_hard_example && <Pill tone="warn">hard example</Pill>}
                    {selected.ocr_disagreement && <Pill tone="warn">OCR disagrees with entity</Pill>}
                    {selected.model_confidence != null && (
                      <Pill tone="mute">model confidence {selected.model_confidence.toFixed(2)}</Pill>
                    )}
                    {selected.from_status_filter && <Pill tone="mute">from review_list</Pill>}
                  </div>
                  <div className="text-fg-mute">Queue reason: {selected.reason}</div>
                  {selRecord && (
                    <div>
                      Existing record: {selRecord.review_status}
                      {selRecord.human_entity_id ? ` → ${selRecord.human_entity_id}` : " (no human entity)"}
                      {selRecord.model_prediction
                        ? ` (model said ${selRecord.model_prediction}${
                            selRecord.model_confidence != null
                              ? ` @${selRecord.model_confidence.toFixed(2)}`
                              : ""
                          })`
                        : " (no model prediction recorded)"}
                      {selRecord.training_eligible
                        ? " · training-eligible"
                        : ` · excluded: ${selRecord.excluded_reason ?? "reason not recorded"}`}
                      {selRecord.correction_reason ? ` · note: ${selRecord.correction_reason}` : ""}
                    </div>
                  )}
                </div>

                {lastVerdict && (
                  <div
                    className={`rounded-lg border px-3 py-2 text-[11px] break-words ${
                      lastVerdict.dataset_error
                        ? "border-bad/40 bg-bad-soft text-bad"
                        : "border-line/70 bg-white/[0.03] text-fg-dim"
                    }`}
                  >
                    <span className="font-mono">{lastVerdict.image_id}</span> recorded as{" "}
                    <span className="font-semibold">{lastVerdict.review_status}</span>
                    {lastVerdict.dataset_updated != null && (
                      <span>
                        {" "}
                        · dataset {lastVerdict.dataset_updated ? "updated" : "not updated"}
                      </span>
                    )}
                    {lastVerdict.dataset_error && (
                      <div className="text-bad mt-0.5">Dataset label did not land: {lastVerdict.dataset_error}</div>
                    )}
                  </div>
                )}

                <div className="flex gap-1.5 flex-wrap items-center">
                  <Button
                    size="sm"
                    kind="primary"
                    disabled={busy !== null || !selected.entity_id}
                    onClick={() => submitReview("correct")}
                  >
                    Correct: {selected.entity_id ?? "(no entity)"}
                  </Button>
                  <Button size="sm" kind="ghost" disabled={busy !== null} onClick={() => submitReview("unknown")}>
                    Unknown
                  </Button>
                  <Button size="sm" kind="ghost" disabled={busy !== null} onClick={() => submitReview("skip")}>
                    Skip
                  </Button>
                  <Button size="sm" kind="ghost" disabled={busy !== null} onClick={undoSelected}>
                    Undo
                  </Button>
                  <Button size="sm" disabled={busy !== null || cursor === 0} onClick={prevItem}>
                    ← Prev
                  </Button>
                  <Button
                    size="sm"
                    disabled={busy !== null || cursor >= queue.length - 1}
                    onClick={nextItem}
                  >
                    Next →
                  </Button>
                  {selRecord?.review_status === "CONFLICT" && (
                    <Button size="sm" kind="danger" disabled={busy !== null} onClick={() => submitReview("resolve")}>
                      Resolve conflict
                    </Button>
                  )}
                </div>

                <div className="flex gap-1.5 flex-wrap items-center">
                  <input
                    ref={correctRef}
                    value={correctEntity}
                    onChange={(e) => {
                      setCorrectEntity(e.target.value);
                      runSearch(e.target.value);
                    }}
                    placeholder="entity_id to assign… (search below)"
                    className="w-56 rounded-lg bg-black/30 border border-line px-2 py-1 text-[11px] text-fg font-mono"
                  />
                  <Button
                    size="sm"
                    disabled={busy !== null || !correctEntity.trim() || correctEntity.trim() === (selected.entity_id ?? "")}
                    onClick={() => submitReview("change")}
                  >
                    Assign + correct
                  </Button>
                </div>
                {searchQuery.trim().length >= 2 && (
                  <div className="flex flex-col gap-1">
                    {searchHits.length === 0 && (
                      <div className="text-[11px] text-warn">NO CANONICAL MATCH — pick nothing, or Mark Unknown. New entities are never invented here.</div>
                    )}
                    {searchHits.map((h) => (
                      <button
                        key={h.entity_id}
                        onClick={() => {
                          setCorrectEntity(h.entity_id);
                          setSearchHits([]);
                        }}
                        className="text-left rounded-lg border border-line/70 px-2 py-1 text-[11px] hover:border-accent"
                      >
                        <span className="font-mono text-fg">{h.entity_id}</span>{" "}
                        <span className="text-fg-dim">{h.canonical_name} ({h.category})</span>{" "}
                        <Pill tone={h.kind === "exact" ? "ok" : "warn"}>{h.kind}</Pill>
                      </button>
                    ))}
                  </div>
                )}

                {selRecord?.review_status === "CONFLICT" && (
                  <div className="rounded-lg border border-bad/30 bg-bad-soft/40 px-3 py-2 flex flex-col gap-1.5">
                    <div className="text-[11px] text-bad">
                      CONFLICT: two verdicts disagree on this image. Resolve it with a canonical entity id and a
                      reason — the reason is written to the permanent audit trail.
                    </div>
                    <div className="flex gap-1.5 flex-wrap items-center">
                      <input
                        value={resolveReason}
                        onChange={(e) => setResolveReason(e.target.value)}
                        placeholder="why this entity is correct… (required)"
                        className="w-72 rounded-lg bg-black/30 border border-line px-2 py-1 text-[11px] text-fg"
                      />
                      <Button
                        size="sm"
                        kind="danger"
                        disabled={busy !== null || !correctEntity.trim() || !resolveReason.trim()}
                        onClick={() => submitReview("resolve")}
                      >
                        Resolve to {correctEntity.trim() || "(pick an entity)"}
                      </Button>
                    </div>
                  </div>
                )}
              </div>
            </Section>
          )}

          <Section
            title={`Queue (${queue.length} loaded${
              queueTruncated ? ` of ${queueTotal} matching, request capped at ${QUEUE_LIMIT}` : ""
            })`}
          >
            <div className="flex flex-col gap-1.5 p-3">
              {queue.slice(0, 15).map((p, i) => (
                <button
                  key={p.image_id}
                  onClick={() => goTo(i)}
                  className={`text-left rounded-lg border px-2.5 py-1.5 text-[11px] hover:border-accent ${
                    selected?.image_id === p.image_id ? "border-accent" : "border-line/70"
                  }`}
                >
                  <div className="flex items-center gap-2 flex-wrap">
                    {p.score != null ? (
                      <Pill tone={p.score >= 128 ? "bad" : p.score >= 32 ? "warn" : "mute"}>{p.score}</Pill>
                    ) : (
                      <Pill tone="mute">no score</Pill>
                    )}
                    <span className="font-mono text-fg-mute truncate">{p.image_id}</span>
                    {p.entity_id && <Pill tone="accent">{p.entity_id}</Pill>}
                    <Pill tone={p.review_status === "CONFLICT" ? "bad" : "mute"}>{p.review_status}</Pill>
                    {p.is_hard_example && <Pill tone="warn">hard</Pill>}
                    {p.ocr_disagreement && <Pill tone="warn">ocr±</Pill>}
                  </div>
                  <div className="text-fg-dim mt-0.5">{p.reason}</div>
                  <div className="font-mono text-fg-mute truncate">{p.ocr_text || "(no OCR)"} · {p.session_id}</div>
                </button>
              ))}
              {queue.length === 0 && (
                <div className="text-[11px] text-fg-mute">Nothing pending for this filter combination.</div>
              )}
              {queue.length > 15 && (
                <div className="text-[11px] text-fg-mute">
                  Showing the first 15 of {queue.length} loaded rows — use ← / → or the Next / Prev buttons.
                </div>
              )}
            </div>
          </Section>
        </>
      )}

      {tab === "coverage" && (
        <Section title="Review coverage — what was actually reviewed">
          {!coverage ? (
            <div className="text-[11px] text-fg-mute">Loading…</div>
          ) : (
            <div className="flex flex-col gap-3 px-4 py-2">
              <div className="text-[11px] font-mono text-fg-dim break-words flex flex-col gap-1">
                <div>
                  Dataset rows: {coverage.total_rows} · review records: {coverage.coverage.total} · unreviewed rows:{" "}
                  {coverage.unreviewed} · hard examples: {coverage.hard_examples}
                </div>
                <div>
                  Reviewed {coverage.coverage.reviewed} of {coverage.coverage.total} records · correct{" "}
                  {coverage.coverage.correct} · corrected {coverage.coverage.corrected} · unknown{" "}
                  {coverage.coverage.unknown} · skipped {coverage.coverage.skipped} · conflicts{" "}
                  {coverage.coverage.conflicts}
                </div>
                <div>
                  Training-eligible {coverage.coverage.eligible} of {coverage.coverage.total} records · excluded{" "}
                  {coverage.coverage.excluded}
                </div>
                <div className="text-fg-mute">
                  Dataset sessions (rows exist): {coverage.per_session.length} · sessions holding at least one review
                  record: {coverage.coverage.sessions}
                </div>
                {coverage.coverage.correct === 0 && coverage.coverage.reviewed > 0 && (
                  <div className="text-warn">
                    confirmed = 0 because REVIEWED_CORRECT requires the record's model_prediction to equal the human
                    entity. Records written without a model prediction can only be REVIEWED_CORRECTED.
                  </div>
                )}
              </div>

              <div>
                <div className="text-[11px] font-semibold text-fg mb-1">Class readiness (backend verdict per class)</div>
                <div className="flex flex-col">
                  {coverage.class_readiness.map((c) => (
                    <div
                      key={c.entity}
                      className="flex items-center gap-2 px-1 py-1 border-b border-line/50 text-[11px] font-mono flex-wrap"
                    >
                      <span className="w-44 truncate text-fg">{c.entity}</span>
                      <Pill tone={CLASS_TONE[c.status]}>{c.status}</Pill>
                      <span className="text-fg-dim">
                        collected {c.collected} · reviewed {c.reviewed} · eligible {c.eligible} · dataset sessions{" "}
                        {c.sessions}
                      </span>
                      <span className="text-fg-mute break-words">{c.reason}</span>
                    </div>
                  ))}
                  {coverage.class_readiness.length === 0 && (
                    <div className="text-[11px] text-fg-mute">No entity-linked rows yet.</div>
                  )}
                </div>
              </div>

              <div>
                <div className="text-[11px] font-semibold text-fg mb-1">Per entity</div>
                <div className="flex flex-col">
                  {coverage.per_entity.map((e) => (
                    <div
                      key={e.entity}
                      className="flex items-center gap-2 px-1 py-1 border-b border-line/50 text-[11px] font-mono flex-wrap"
                    >
                      <span className="w-44 truncate text-fg">{e.entity}</span>
                      <span className="text-fg-dim">collected {e.collected}</span>
                      <span className="text-fg-dim">
                        reviewed {e.reviewed} (confirmed {e.confirmed} · corrected {e.corrected} · unknown{" "}
                        {e.unknown})
                      </span>
                      <span className="text-fg-dim">skipped {e.skipped} · conflicts {e.conflicts}</span>
                      <span className="text-fg-dim">hard {e.hard_examples}</span>
                      <span className="text-fg-dim">
                        split train {e.train}/val {e.validation}/test {e.test}
                      </span>
                      <span className="text-fg-dim">dataset sessions {e.sessions}</span>
                      <Pill tone={e.eligible > 0 ? "ok" : "mute"}>eligible {e.eligible}</Pill>
                    </div>
                  ))}
                  {coverage.per_entity.length === 0 && (
                    <div className="text-[11px] text-fg-mute">No entity-linked rows yet.</div>
                  )}
                </div>
              </div>

              <div>
                <div className="text-[11px] font-semibold text-fg mb-1">Per split</div>
                <div className="flex flex-col">
                  {coverage.per_split.map((s) => (
                    <div
                      key={s.split}
                      className="flex items-center gap-2 px-1 py-1 border-b border-line/50 text-[11px] font-mono flex-wrap"
                    >
                      <span className="w-24 text-fg">{s.split}</span>
                      <span className="text-fg-dim">collected {s.collected}</span>
                      <span className="text-fg-dim">reviewed {s.reviewed}</span>
                      <span className="text-fg-dim">eligible {s.eligible}</span>
                      <span className="text-fg-dim">dataset sessions {s.sessions}</span>
                    </div>
                  ))}
                  {coverage.per_split.length === 0 && <div className="text-[11px] text-fg-mute">No rows.</div>}
                </div>
              </div>

              <div>
                <div className="text-[11px] font-semibold text-fg mb-1">
                  Per dataset session (rows → reviewed → eligible)
                </div>
                <div className="flex flex-col">
                  {coverage.per_session.map((s) => (
                    <div
                      key={s.session_id}
                      className="flex items-center gap-2 px-1 py-1 border-b border-line/50 text-[11px] font-mono flex-wrap"
                    >
                      <span className="w-56 truncate text-fg">{s.session_id}</span>
                      <Pill tone="mute">{s.split}</Pill>
                      <span className="text-fg-dim">
                        collected {s.collected} · reviewed {s.reviewed} · eligible {s.eligible}
                      </span>
                    </div>
                  ))}
                  {coverage.per_session.length === 0 && (
                    <div className="text-[11px] text-fg-mute">No sessions yet.</div>
                  )}
                </div>
              </div>

              <div>
                <div className="text-[11px] font-semibold text-fg mb-1">Exclusion reasons (not training-eligible)</div>
                <div className="flex flex-col">
                  {coverage.exclusions.map((x) => (
                    <div
                      key={x.reason}
                      className="flex items-center gap-2 px-1 py-1 border-b border-line/50 text-[11px] font-mono"
                    >
                      <span className="text-fg-dim break-words">{x.reason}</span>
                      <Pill tone="bad">{x.count}</Pill>
                    </div>
                  ))}
                  {coverage.exclusions.length === 0 && (
                    <div className="text-[11px] text-fg-mute">No excluded rows recorded.</div>
                  )}
                </div>
              </div>
            </div>
          )}
        </Section>
      )}

      {tab === "drops" && (
        <Section title="Fishing drops — canonical registry vs reality">
          <div className="flex flex-col">
            {drops.map((d) => (
              <div key={d.entity_id} className="px-1 py-1.5 border-b border-line/50 text-[11px] flex flex-col gap-0.5">
                <div className="flex items-center gap-2 flex-wrap">
                  <span className="font-semibold text-fg">{d.canonical_name}</span>
                  <span className="font-mono text-fg-mute">{d.entity_id}</span>
                  <Pill tone="mute">{d.category}</Pill>
                  {d.rarity && <Pill tone="fruit">{d.rarity}</Pill>}
                  {!d.fishing_drop && <Pill tone="mute">not a fishing drop</Pill>}
                  {d.shadowed_by ? (
                    <Pill tone="warn">alias of {d.shadowed_by} — cannot be resolved by name</Pill>
                  ) : (
                    <Pill tone={d.model_status === "IN_SCOPE" ? "ok" : "mute"}>{d.model_status}</Pill>
                  )}
                </div>
                <div className="font-mono text-fg-dim">
                  collected {d.collected} · reviewed {d.reviewed} · eligible {d.eligible} · sessions {d.sessions}
                  {d.aliases.length > 0 && <span> · aka {d.aliases.slice(0, 4).join(", ")}</span>}
                </div>
                <div className="font-mono text-fg-mute">wiki: {d.wiki_source}{d.wiki_url ? ` · ${d.wiki_url}` : " (no per-entity URL in KB)"}</div>
              </div>
            ))}
            {drops.length === 0 && <div className="text-[11px] text-fg-mute">Loading…</div>}
          </div>
        </Section>
      )}

      {tab === "readiness" && (
        <Section title="Model readiness — why ready or not">
          <div className="flex flex-col gap-3 px-4 py-2">
            {readiness.length === 0 && <div className="text-[11px] text-fg-mute">Loading…</div>}
            {readiness.map((r) => {
              const ready = isReadyStatus(r.status);
              const passing = r.checks.filter((c) => c.passed && !c.structural);
              const failing = r.checks.filter((c) => !c.passed && !c.structural);
              const structural = r.checks.filter((c) => c.structural);
              return (
                <div key={r.family} className="rounded-lg border border-line/70 px-2.5 py-2">
                  <div className="flex items-center gap-2 flex-wrap text-[12px]">
                    <span className="font-semibold text-fg uppercase">{r.family} model</span>
                    <Pill tone={statusTone(r.status)}>{r.status}</Pill>
                  </div>
                  <div className="text-[11px] text-fg-dim mt-0.5">{STATUS_MEANING[r.status]}</div>

                  <div className="mt-2 text-[11px] font-semibold text-fg">
                    {ready ? "WHY IS THIS MODEL READY?" : "WHY IS THIS MODEL NOT READY?"}
                  </div>
                  {ready ? (
                    <div className="text-[11px] font-mono text-fg-dim break-words">
                      {passing.length === 0
                        ? "No check passed; the status is reported by the backend but no evidence backs it."
                        : passing.map((c) => (
                            <div key={c.name}>
                              ✓ {c.name} — {c.actual} (required {c.required})
                            </div>
                          ))}
                      {structural.length > 0 && (
                        <div className="text-fg-mute">
                          {structural.map((c) => (
                            <div key={c.name}>· {c.name} — {c.actual} (cannot pass in this build)</div>
                          ))}
                        </div>
                      )}
                    </div>
                  ) : (
                    <div className="text-[11px] font-mono text-warn break-words">
                      {failing.map((c) => (
                        <div key={c.name}>
                          • {c.name}: {c.detail}
                        </div>
                      ))}
                      {failing.length === 0 && <div className="text-fg-mute">No failing check reported.</div>}
                    </div>
                  )}

                  {r.next_actions.length > 0 && (
                    <div className="mt-2 text-[11px]">
                      <div className="text-fg font-semibold">Next actions</div>
                      {r.next_actions.map((a) => (
                        <div key={a} className="font-mono text-accent break-words">
                          → {a}
                        </div>
                      ))}
                    </div>
                  )}

                  <div className="mt-2 flex flex-col gap-2">
                    {STAGE_ORDER.map((stage) => {
                      const checks = r.checks.filter((c) => c.stage === stage);
                      if (checks.length === 0) return null;
                      return (
                        <div key={stage}>
                          <div className="text-[11px] font-semibold text-fg-mute uppercase tracking-[0.1em]">
                            {stage} · {STAGE_LABEL[stage]}
                          </div>
                          <div className="text-[10px] text-fg-mute break-words">{STAGE_BLURB[stage]}</div>
                          <div className="flex flex-col mt-0.5">
                            {checks.map((c) => (
                              <div
                                key={c.name}
                                className={`text-[11px] font-mono flex flex-col gap-0.5 px-1 py-1 border-b border-line/40 ${
                                  c.structural ? "opacity-55" : ""
                                }`}
                              >
                                <div className="flex gap-2 flex-wrap items-center">
                                  <span className={c.passed ? "text-ok" : "text-bad"}>
                                    {c.passed ? "PASS" : "FAIL"}
                                  </span>
                                  <span className="text-fg">{c.name}</span>
                                  <span className="text-fg-dim">
                                    actual {c.actual} · required {c.required} · {c.difference}
                                  </span>
                                  {c.structural && (
                                    <Pill tone="mute">by design — not actionable</Pill>
                                  )}
                                </div>
                                <div className="text-fg-mute break-words">{c.detail}</div>
                                {!c.passed && !c.structural && c.next_action && (
                                  <div className="text-accent break-words">→ {c.next_action}</div>
                                )}
                              </div>
                            ))}
                          </div>
                        </div>
                      );
                    })}
                  </div>

                  {r.blockers.length > 0 && (
                    <div className="mt-2 text-[11px]">
                      <div className="text-fg font-semibold">Actionable blockers (structural checks excluded)</div>
                      {r.blockers.map((b) => (
                        <div key={b} className="font-mono text-warn break-words">• {b}</div>
                      ))}
                    </div>
                  )}
                </div>
              );
            })}
            <div className="text-[11px] font-mono text-fg-mute break-words">
              Reviewed ≠ trained. Trained ≠ evaluated. Evaluated ≠ shadow-validated. Shadow-validated ≠ production-enabled.
            </div>
          </div>
        </Section>
      )}

      {tab === "history" && (
        <Section title="Learning history (immutable log)">
          <div className="flex flex-col text-[11px] font-mono">
            {history.length === 0 && <div className="text-fg-mute">No events yet.</div>}
            {history.slice().reverse().slice(0, 60).map((h, i) => (
              <div key={i} className="px-1 py-1 border-b border-line/50 text-fg-dim break-words">
                <span className="text-fg-mute">{fmtTs(h.ts)}</span> <span className="text-accent">{h.kind}</span>{" "}
                {JSON.stringify(h.detail).slice(0, 220)}
              </div>
            ))}
          </div>
        </Section>
      )}

      {tab === "settings" && (
        <Section title="Training settings">
          {!settings ? (
            <div className="text-[11px] text-fg-mute">Loading…</div>
          ) : (
            <div className="flex flex-col gap-2 text-[11px]">
              <label className="flex items-center justify-between gap-2">
                <span>Automatic training (candidates only, never production)</span>
                <Toggle value={settings.auto_enabled} onChange={(v) => setSettings({ ...settings, auto_enabled: v })} />
              </label>
              <label className="flex items-center justify-between gap-2">
                <span>Auto-promote passed candidates to shadow</span>
                <Toggle value={settings.auto_promote_to_shadow} onChange={(v) => setSettings({ ...settings, auto_promote_to_shadow: v })} />
              </label>
              <label className="flex items-center justify-between gap-2">
                <span>Auto-rollback on shadow regression</span>
                <Toggle value={settings.auto_rollback} onChange={(v) => setSettings({ ...settings, auto_rollback: v })} />
              </label>
              <label className="flex items-center justify-between gap-2">
                <span>Defer auto-training while fishing</span>
                <Toggle value={settings.defer_while_fishing} onChange={(v) => setSettings({ ...settings, defer_while_fishing: v })} />
              </label>
              {[
                ["min_new_samples", "Min new samples to trigger"],
                ["min_new_sessions", "Min new sessions to trigger"],
                ["trigger_cooldown_hours", "Trigger cooldown (hours)"],
                ["readiness_min_macro_f1", "Readiness: min macro-F1"],
                ["readiness_min_worst_f1", "Readiness: min worst-class F1"],
                ["readiness_min_shadow_events", "Readiness: min shadow events"],
                ["readiness_min_shadow_agreement", "Readiness: min shadow agreement (0-1)"],
                ["readiness_min_shadow_sessions", "Readiness: min shadow sessions"],
                ["readiness_min_review_coverage", "Readiness: min reviewed share"],
              ].map(([k, label]) => (
                <label key={k} className="flex items-center justify-between gap-2">
                  <span>{label}</span>
                  <input
                    type="number"
                    value={settings[k as keyof TrainingSettings] as number}
                    onChange={(e) => setSettings({ ...settings, [k]: Math.max(0, Number(e.target.value)) })}
                    className="w-28 rounded-lg bg-black/30 border border-line px-2 py-1 text-[11px] text-fg font-mono"
                  />
                </label>
              ))}
              {[["python_path", "Python with torch (absolute path)"], ["trainer_dir", "Trainer checkout dir (contains ml/gpo_train)"]].map(([k, label]) => (
                <label key={k} className="flex flex-col gap-1">
                  <span className="text-fg-dim">{label}</span>
                  <input
                    value={settings[k as keyof TrainingSettings] as string}
                    onChange={(e) => setSettings({ ...settings, [k]: e.target.value })}
                    placeholder="not configured"
                    className="rounded-lg bg-black/30 border border-line px-2 py-1 text-[11px] text-fg font-mono"
                  />
                </label>
              ))}
              <div className="font-mono text-fg-dim break-words">
                Backend: {backend ? (backend.available ? `ready (${backend.torch_version ?? "torch"})` : backend.detail) : "checking…"}
              </div>
              <div>
                <Button size="sm" kind="primary" disabled={busy !== null} onClick={saveSettings}>
                  Save training settings
                </Button>
              </div>
              <div className="font-mono text-fg-mute break-words">
                There is no production-control switch anywhere: vision can never drive the macro regardless of these settings.
              </div>
            </div>
          )}
        </Section>
      )}

      <ConfirmModal
        open={confirm !== null}
        title={confirm?.title ?? ""}
        body={confirm?.body ?? ""}
        backupNote="Rollback point is preserved automatically."
        confirmLabel={confirm?.label ?? "Confirm"}
        danger={true}
        busy={busy !== null}
        onConfirm={() => confirm?.run()}
        onCancel={() => setConfirm(null)}
      />
    </div>
  );
}
