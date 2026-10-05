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
  HistoryEntry,
  ReviewItem,
  TrainingJob,
  TrainingOverview,
  TrainingSettings,
} from "../lib/types";

type SubTab =
  | "overview" | "dataset" | "fish" | "fruits" | "jobs"
  | "candidates" | "models" | "compare" | "review" | "history" | "settings";

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
  const [review, setReview] = useState<ReviewItem[]>([]);
  const [settings, setSettings] = useState<TrainingSettings | null>(null);
  const [backend, setBackend] = useState<BackendStatus | null>(null);
  const [compare, setCompare] = useState<CompareView | null>(null);
  const [compareId, setCompareId] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<{ title: string; body: string; label: string; run: () => Promise<void> } | null>(null);
  const [skipped, setSkipped] = useState<Record<string, boolean>>({});

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
      const [e, r, s, b] = await Promise.all([
        api.datasetExplorer(),
        api.reviewQueue(),
        api.trainingSettingsGet(),
        api.trainingBackend(),
      ]);
      setExplorer(e);
      setReview(r);
      setSettings(s);
      setBackend(b);
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

  const reviewAct = async (item: ReviewItem, kind: "confirm" | "unknown") => {
    try {
      if (kind === "confirm" && item.entity_id) {
        await api.mlAnnotate(item.image_id, undefined, undefined, item.entity_id, false, undefined);
      } else {
        await api.mlAnnotate(item.image_id, undefined, undefined, undefined, true, "human: unknown");
      }
      setSkipped((m) => ({ ...m, [item.image_id]: true }));
      showToast("info", kind === "confirm" ? "Label confirmed" : "Marked UNKNOWN (reviewed)");
    } catch (e) {
      showToast("error", "Review annotate" + ": " + String(e));
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
        <Section title={`Review queue (${review.filter((r) => !skipped[r.image_id]).length})`}>
          <div className="flex flex-col gap-2">
            {review.filter((r) => !skipped[r.image_id]).slice(0, 20).map((r) => (
              <div key={r.image_id} className="rounded-lg border border-line/70 px-2.5 py-1.5 text-[11px]">
                <div className="flex items-center gap-2 flex-wrap">
                  {r.reasons.map((x) => <Pill key={x} tone="warn">{x.slice(0, 60)}</Pill>)}
                  {r.entity_id && <Pill tone="accent">{r.entity_id}</Pill>}
                </div>
                <div className="font-mono text-fg-mute mt-1 truncate">{r.ocr_text || "(no OCR)"} · {r.session_id}</div>
                <div className="flex gap-1.5 mt-1.5">
                  {r.entity_id && (
                    <Button size="sm" kind="primary" onClick={() => reviewAct(r, "confirm")}>Confirm {r.entity_id}</Button>
                  )}
                  <Button size="sm" kind="ghost" onClick={() => reviewAct(r, "unknown")}>Confirm UNKNOWN</Button>
                  <Button size="sm" kind="ghost" onClick={() => setSkipped((m) => ({ ...m, [r.image_id]: true }))}>Skip</Button>
                </div>
              </div>
            ))}
            {review.filter((r) => !skipped[r.image_id]).length === 0 && (
              <div className="text-[11px] text-fg-mute">Queue empty — no hard, OCR-empty, or entity-less RESULT rows.</div>
            )}
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
