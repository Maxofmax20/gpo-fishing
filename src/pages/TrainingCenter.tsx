import { useCallback, useEffect, useState } from "react";
import { api } from "../lib/ipc";
import { showToast } from "../lib/store";
import { useVisiblePoll } from "../lib/useVisiblePoll";
import { Button, Pill, Section, Toggle } from "../components/primitives";
import ConfirmModal from "../components/ConfirmModal";
import type {
  BackendStatus,
  CandidateRecord,
  CompareView,
  DatasetExplorer,
  DropEntry,
  EntityHit,
  HistoryEntry,
  ModelReadiness,
  PriorityItem,
  ReviewCoverageView,
  ReviewImage,
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

export default function TrainingCenter() {
  const [tab, setTab] = useState<SubTab>("overview");
  const [overview, setOverview] = useState<TrainingOverview | null>(null);
  const [explorer, setExplorer] = useState<DatasetExplorer | null>(null);
  const [jobs, setJobs] = useState<TrainingJob[]>([]);
  const [candidates, setCandidates] = useState<CandidateRecord[]>([]);
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  const [coverage, setCoverage] = useState<ReviewCoverageView | null>(null);
  const [priority, setPriority] = useState<PriorityItem[]>([]);
  const [drops, setDrops] = useState<DropEntry[]>([]);
  const [readiness, setReadiness] = useState<ModelReadiness[]>([]);
  const [selected, setSelected] = useState<PriorityItem | null>(null);
  const [selPng, setSelPng] = useState<ReviewImage | null>(null);
  const [selRecord, setSelRecord] = useState<ReviewRecord | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchHits, setSearchHits] = useState<EntityHit[]>([]);
  const [correctEntity, setCorrectEntity] = useState("");
  const [settings, setSettings] = useState<TrainingSettings | null>(null);
  const [backend, setBackend] = useState<BackendStatus | null>(null);
  const [compare, setCompare] = useState<CompareView | null>(null);
  const [compareId, setCompareId] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<{ title: string; body: string; label: string; run: () => Promise<void> } | null>(null);

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

  const refreshSlow = useCallback(async () => {
    try {
      const [e, s, b, cov, pri, dr, rd] = await Promise.all([
        api.datasetExplorer(),
        api.trainingSettingsGet(),
        api.trainingBackend(),
        api.reviewCoverage(),
        api.reviewPriority(60),
        api.dropsExplorer(),
        api.readinessStatus(),
      ]);
      setExplorer(e);
      setSettings(s);
      setBackend(b);
      setCoverage(cov);
      setPriority(pri);
      setDrops(dr);
      setReadiness(rd);
    } catch (e) {
      showToast("error", "Training Center details" + ": " + String(e));
    }
  }, []);

  useEffect(() => {
    refreshAll();
    refreshSlow();
  }, [refreshAll, refreshSlow]);

  const anyActive = jobs.some((j) => ["QUEUED", "RUNNING", "EVALUATING"].includes(j.status));
  useVisiblePoll(refreshAll, anyActive ? 3000 : 15000);

  async function run(label: string, fn: () => Promise<unknown>, okMsg?: string) {
    setBusy(label);
    try {
      await fn();
      if (okMsg) showToast("info", okMsg);
      await refreshAll();
      await refreshSlow();
    } catch (e) {
      showToast("error", `${label}: ${String(e)}`);
    } finally {
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

  const selectForReview = async (item: PriorityItem) => {
    setSelected(item);
    setSelPng(null);
    setSelRecord(null);
    setCorrectEntity(item.entity_id ?? "");
    setSearchQuery("");
    setSearchHits([]);
    try {
      const [png, rec] = await Promise.all([
        api.reviewImage(item.image_id),
        api.reviewGet(item.image_id),
      ]);
      setSelPng(png);
      setSelRecord(rec);
    } catch (e) {
      showToast("error", "Load review image" + ": " + String(e));
    }
  };

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

  const submitReview = async (kind: "correct" | "change" | "unknown" | "skip" | "resolve") => {
    if (!selected) return;
    try {
      if (kind === "skip") {
        await api.reviewSkip(selected.image_id);
      } else if (kind === "unknown") {
        await api.reviewApply(selected.image_id, {});
      } else if (kind === "resolve") {
        if (!correctEntity.trim()) {
          showToast("error", "Resolve needs an entity id");
          return;
        }
        await api.reviewResolve(selected.image_id, correctEntity.trim(), "manual conflict resolution");
      } else {
        const id = kind === "correct" ? selected.entity_id : correctEntity.trim();
        if (!id) {
          showToast("error", "No entity to confirm — pick one from search or mark UNKNOWN");
          return;
        }
        await api.reviewApply(selected.image_id, { humanEntityId: id });
      }
      showToast("info", "Review recorded");
      await refreshSlow();
      await selectForReview(selected);
    } catch (e) {
      showToast("error", "Review" + ": " + String(e));
    }
  };

  const saveSettings = async () => {
    if (!settings) return;
    await run("settings", () => api.trainingSettingsSet(settings), "Training settings saved");
  };

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
              <div className="flex gap-2 flex-wrap mt-1">
                <Button size="sm" kind="primary" disabled={busy !== null} onClick={() => startTrain("fish")}>
                  Train Fish
                </Button>
                <Button size="sm" disabled={busy !== null} onClick={() => startTrain("fruit")}>
                  Train Fruit
                </Button>
                <Button size="sm" disabled={busy !== null} onClick={() => startTrain("state")}>
                  Train State
                </Button>
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
          <Section title={`Review workbench — next best (${priority.length} queued)`}>
            <div className="flex flex-col gap-1.5">
              {priority.slice(0, 15).map((p) => (
                <button
                  key={p.image_id}
                  onClick={() => selectForReview(p)}
                  className={`text-left rounded-lg border px-2.5 py-1.5 text-[11px] hover:border-accent ${selected?.image_id === p.image_id ? "border-accent" : "border-line/70"}`}
                >
                  <div className="flex items-center gap-2 flex-wrap">
                    <Pill tone={p.score >= 128 ? "bad" : p.score >= 32 ? "warn" : "mute"}>{p.score}</Pill>
                    <span className="font-mono text-fg-mute truncate">{p.image_id}</span>
                    {p.entity_id && <Pill tone="accent">{p.entity_id}</Pill>}
                    <Pill tone="mute">{p.review_status}</Pill>
                  </div>
                  <div className="text-fg-dim mt-0.5">{p.reason}</div>
                  <div className="font-mono text-fg-mute truncate">{p.ocr_text || "(no OCR)"} · {p.session_id}</div>
                </button>
              ))}
              {priority.length === 0 && (
                <div className="text-[11px] text-fg-mute">Queue empty — everything collected is reviewed.</div>
              )}
            </div>
          </Section>
          {selected && (
            <Section title={`Review: ${selected.image_id}`}>
              <div className="flex flex-col gap-2">
                {selPng ? (
                  <img
                    src={`data:image/png;base64,${selPng.png_base64}`}
                    alt={selected.image_id}
                    className="rounded-lg border border-line/70 max-w-full"
                    style={{ imageRendering: "pixelated", maxHeight: 320 }}
                  />
                ) : (
                  <div className="text-[11px] text-fg-mute">Loading PNG…</div>
                )}
                <div className="text-[11px] font-mono text-fg-dim break-words">
                  <div>Session {selected.session_id} · {new Date(selected.timestamp_ms).toLocaleString()}</div>
                  <div>OCR: {selected.ocr_text || "(none)"}</div>
                  <div>
                    Dataset entity: {selected.entity_id ?? "(none)"} · status {selected.review_status}
                  </div>
                  {selRecord && (
                    <div>
                      Prior review: {selRecord.review_status}
                      {selRecord.human_entity_id ? ` → ${selRecord.human_entity_id}` : ""}
                      {selRecord.model_prediction ? ` (model said ${selRecord.model_prediction}${selRecord.model_confidence != null ? ` @${selRecord.model_confidence.toFixed(2)}` : ""})` : " (no model prediction recorded)"}
                      {selRecord.training_eligible ? " · training-eligible" : ` · excluded: ${selRecord.excluded_reason ?? "?"}`}
                    </div>
                  )}
                </div>
                <div className="flex gap-1.5 flex-wrap">
                  {selected.entity_id && (
                    <Button size="sm" kind="primary" disabled={busy !== null} onClick={() => submitReview("correct")}>
                      Correct: {selected.entity_id}
                    </Button>
                  )}
                  <Button size="sm" kind="ghost" disabled={busy !== null} onClick={() => submitReview("unknown")}>
                    Unknown
                  </Button>
                  <Button size="sm" kind="ghost" disabled={busy !== null} onClick={() => submitReview("skip")}>
                    Skip
                  </Button>
                  {selRecord?.review_status === "CONFLICT" && (
                    <Button size="sm" kind="danger" disabled={busy !== null} onClick={() => submitReview("resolve")}>
                      Resolve conflict
                    </Button>
                  )}
                </div>
                <div className="flex gap-1.5 flex-wrap items-center">
                  <input
                    value={correctEntity}
                    onChange={(e) => { setCorrectEntity(e.target.value); runSearch(e.target.value); }}
                    placeholder="entity_id to assign… (search below)"
                    className="w-56 rounded-lg bg-black/30 border border-line px-2 py-1 text-[11px] text-fg font-mono"
                  />
                  <Button size="sm" disabled={busy !== null || !correctEntity.trim()} onClick={() => submitReview("change")}>
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
                        onClick={() => { setCorrectEntity(h.entity_id); setSearchHits([]); }}
                        className="text-left rounded-lg border border-line/70 px-2 py-1 text-[11px] hover:border-accent"
                      >
                        <span className="font-mono text-fg">{h.entity_id}</span>{" "}
                        <span className="text-fg-dim">{h.canonical_name} ({h.category})</span>{" "}
                        <Pill tone={h.kind === "exact" ? "ok" : "warn"}>{h.kind}</Pill>
                      </button>
                    ))}
                  </div>
                )}
              </div>
            </Section>
          )}
        </>
      )}

      {tab === "coverage" && (
        <Section title="Review coverage — what was actually reviewed">
          {!coverage ? (
            <div className="text-[11px] text-fg-mute">Loading…</div>
          ) : (
            <div className="flex flex-col gap-2">
              <div className="text-[11px] font-mono text-fg-dim break-words">
                Total PNGs {coverage.total_rows} · reviewed {coverage.coverage.reviewed} · correct{" "}
                {coverage.coverage.correct} · corrected {coverage.coverage.corrected} · unknown{" "}
                {coverage.coverage.unknown} · skipped {coverage.coverage.skipped} · conflicts{" "}
                {coverage.coverage.conflicts} · sessions {coverage.coverage.sessions} · eligible{" "}
                {coverage.coverage.eligible} · excluded {coverage.coverage.excluded}
              </div>
              <div className="flex flex-col">
                {coverage.per_entity.map((e) => (
                  <div key={e.entity} className="flex items-center gap-2 px-1 py-1 border-b border-line/50 text-[11px] font-mono flex-wrap">
                    <span className="w-44 truncate text-fg">{e.entity}</span>
                    <span className="text-fg-dim">got {e.collected}</span>
                    <span className="text-fg-dim">rev {e.reviewed} (✓{e.confirmed} ~{e.corrected} ?{e.unknown})</span>
                    <span className="text-fg-dim">sess {e.sessions}</span>
                    <Pill tone={e.eligible > 0 ? "ok" : "mute"}>eligible {e.eligible}</Pill>
                  </div>
                ))}
                {coverage.per_entity.length === 0 && (
                  <div className="text-[11px] text-fg-mute">No entity-linked rows yet.</div>
                )}
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
                  <Pill tone={d.model_status === "IN_SCOPE" ? "ok" : "mute"}>{d.model_status}</Pill>
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
          <div className="flex flex-col gap-3">
            {(readiness.length === 0) && <div className="text-[11px] text-fg-mute">Loading…</div>}
            {readiness.map((r) => (
              <div key={r.family} className="rounded-lg border border-line/70 px-2.5 py-2">
                <div className="flex items-center gap-2 flex-wrap text-[12px]">
                  <span className="font-semibold text-fg uppercase">{r.family} model</span>
                  <Pill tone={r.status === "PRODUCTION_READY" ? "ok" : r.status === "NOT_READY" ? "bad" : "warn"}>
                    {r.status}
                  </Pill>
                </div>
                <div className="flex flex-col gap-0.5 mt-1.5">
                  {r.checks.map((c) => (
                    <div key={c.name} className="text-[11px] font-mono flex gap-2">
                      <span className={c.passed ? "text-ok" : "text-bad"}>{c.passed ? "PASS" : "FAIL"}</span>
                      <span className="text-fg">{c.name}</span>
                      <span className="text-fg-dim break-words">{c.detail}</span>
                    </div>
                  ))}
                </div>
                {r.blockers.length > 0 && (
                  <div className="mt-1.5 text-[11px]">
                    <div className="text-fg font-semibold">
                      {r.status === "NOT_READY" ? "WHY IS THIS MODEL NOT READY?" : "Remaining blockers:"}
                    </div>
                    {r.blockers.map((b) => (
                      <div key={b} className="font-mono text-warn break-words">• {b}</div>
                    ))}
                  </div>
                )}
              </div>
            ))}
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
