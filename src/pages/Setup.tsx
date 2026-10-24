import { useEffect, useState } from "react";
import { BookOpen, Check, Circle, HeartPulse, PencilRuler, ScanText } from "lucide-react";
import { api } from "../lib/ipc";
import { showToast, useStore } from "../lib/store";
import type { BaselineReport, DatasetReport, DatasetSample, HealthCheck, KnowledgeEntity, KnowledgeStats, MlAnnotation, MlModelStatus, OcrTest, TrainingReadiness, WikiSyncResult } from "../lib/types";
import { Button, Kbd, KeyCapture, Pill, Row, Section, Segmented } from "../components/primitives";
import { RegionField } from "../components/RegionField";
import { PointField } from "../components/PointField";

export default function Setup() {
  const s = useStore((st) => st.settings);
  const roblox = useStore((st) => st.roblox);
  const ocrAvailable = useStore((st) => st.ocrAvailable);
  const update = useStore((st) => st.update);
  const [open, setOpen] = useState<string | null>(null);
  const [ocr, setOcr] = useState<OcrTest | null>(null);
  const [ocrErr, setOcrErr] = useState<string | null>(null);
  const [health, setHealth] = useState<HealthCheck | null>(null);
  const [healthBusy, setHealthBusy] = useState(false);
  const [healthErr, setHealthErr] = useState<string | null>(null);

  if (!s) {
    return (
      <div className="pb-4 pt-2">
        <div className="rounded-xl border border-line p-4 text-[12px] text-fg-dim">Loading settings…</div>
      </div>
    );
  }

  const toggle = (k: string) => setOpen((o) => (o === k ? null : k));

  const runOcr = async () => {
    try {
      setOcr(await api.ocrTest());
      setOcrErr(null);
    } catch (e) {
      const msg = String(e);
      setOcrErr(msg);
      showToast("error", `OCR test failed: ${msg}`);
    }
  };

  const runHealth = async () => {
    setHealthBusy(true);
    setHealthErr(null);
    try {
      setHealth(await api.healthCheck());
    } catch (e) {
      const msg = String(e);
      setHealthErr(msg);
      showToast("error", `Health check failed: ${msg}`);
    } finally {
      setHealthBusy(false);
    }
  };

  // Completion derived from actual calibration state (plus the last health
  // check when one was run): a failed probe un-checks its step.
  const probe = (id: string) => health?.items.find((i) => i.id === id);
  const ok = (id: string, base: boolean) => base && (!probe(id) || probe(id)!.status !== "fail");
  const barDone = ok("bar", s.regions.bar.w > 0.02 && s.regions.bar.h > 0.02);
  const castDone = ok("points", true);
  const rodDone = ok("points", s.keys.rod.length > 0);
  const dropDone = ok("drop", s.regions.drop.w > 0.02 && s.regions.drop.h > 0.01);

  return (
    <div className="pb-4 pt-2">
      <Section title="Getting started">
        <Row
          title={<span className="inline-flex items-center gap-2"><BookOpen size={14} className="text-accent" />How to set up</span>}
          sub="Six animated steps from an empty hotbar to your first catch."
          right={
            <Button size="sm" kind="primary" onClick={() => api.guideOpen()}>
              Open guide
            </Button>
          }
        />
      </Section>

      <Section title="Diagnostics">
        <Row
          title={<span className="inline-flex items-center gap-2"><HeartPulse size={14} className="text-accent" />Health check</span>}
          sub="Captures every configured area from the live game and reports vision + OCR status. Nothing is simulated."
          right={
            <Button size="sm" kind="primary" onClick={runHealth} disabled={!roblox || healthBusy}>
              {healthBusy ? "Checking…" : health ? "Re-check" : "Run check"}
            </Button>
          }
        />
        {healthErr && <div className="text-[11px] text-warn px-1 pb-1">{healthErr}</div>}
        {health && (
          <div className="flex flex-col gap-1.5 pb-1">
            {health.items.map((it) => (
              <div key={it.id} className="flex items-start gap-2 rounded-lg border border-line/70 px-2.5 py-1.5">
                <Pill tone={it.status === "pass" ? "ok" : it.status === "warn" ? "warn" : "bad"}>
                  {it.status === "pass" ? "pass" : it.status === "warn" ? "warn" : "fail"}
                </Pill>
                <div className="min-w-0 flex-1">
                  <div className="text-[12px] font-medium">
                    {it.label}
                    {it.score != null && <span className="ml-1.5 font-mono text-fg-mute">{Math.round(it.score * 100)}%</span>}
                  </div>
                  <div className="text-[11px] text-fg-dim break-words">{it.detail}</div>
                  <div className="mt-0.5 font-mono text-[10px] text-fg-mute">
                    configured: {it.configured ? "yes" : "no"} · detected now: {it.detected ? "yes" : "no"}
                  </div>
                </div>
              </div>
            ))}
            <div className="text-[11px] text-fg-mute px-1">
              Knowledge base: {health.knowledge_entities} entities (fruits, fish, bait, UI terms).
            </div>
          </div>
        )}
      </Section>

      <Section title="Required">
        <Row
          title={<Step done={!!roblox} label="Roblox window" />}
          sub={roblox ? `${roblox.client.w} × ${roblox.client.h} · ${roblox.is_foreground ? "focused" : "background"}` : "Open Roblox and join GPO"}
          right={<Pill tone={roblox ? "ok" : "warn"}>{roblox ? "detected" : "not found"}</Pill>}
        />
        <Row
          title={<span className="inline-flex items-center gap-2"><PencilRuler size={14} className="text-accent" />Edit areas on screen</span>}
          sub={<>Drag both areas directly over the game. Press <Kbd>{s.hotkeys.overlay}</Kbd> any time, <Kbd>Tab</Kbd> switches, <Kbd>Enter</Kbd> saves.</>}
          right={
            <Button size="sm" kind="primary" disabled={!roblox} onClick={() => api.overlayOpenRegions()}>
              Open editor
            </Button>
          }
        />
        <Row
          title={<Step done={barDone} label="Fishing bar area" />}
          sub="Where the blue minigame bar appears. Cast once, then use Auto-detect."
          open={open === "bar"}
          onToggle={() => toggle("bar")}
        >
          <RegionField target="bar_region" value={s.regions.bar} />
        </Row>
        <Row
          title={<Step done={ok("server_time", s.regions.server_time.w > 0.02)} label="Server timer area" />}
          sub="The in-game server clock, used by the boss tracker and auto-reconnect timing."
          open={open === "server_time"}
          onToggle={() => toggle("server_time")}
        >
          <RegionField target="server_time_region" value={s.regions.server_time} />
        </Row>
        <Row
          title={<Step done={ok("bait_menu", s.regions.bait_menu.w > 0.02)} label="Bait menu area" />}
          sub="The rod bait menu, used by smart bait detection. Verify with Scan stock in Features."
          open={open === "bait_menu"}
          onToggle={() => toggle("bait_menu")}
        >
          <RegionField target="bait_menu_region" value={s.regions.bait_menu} />
        </Row>
        <Row
          title={<Step done={castDone} label="Cast point" />}
          sub="Where the rod is thrown. Leave on auto to use the screen center."
          open={open === "cast"}
          onToggle={() => toggle("cast")}
          right={
            <Pill tone={s.features.auto_mouse_position ? "accent" : "mute"}>{s.features.auto_mouse_position ? "auto" : "custom"}</Pill>
          }
        >
          <div className="flex items-center gap-3 flex-wrap">
            <Segmented
              value={s.features.auto_mouse_position ? "auto" : "custom"}
              options={[
                { value: "auto", label: "Auto (center)" },
                { value: "custom", label: "Custom" },
              ]}
              onChange={(v) => update((x) => void (x.features.auto_mouse_position = v === "auto"))}
            />
            {!s.features.auto_mouse_position && <PointField target="fishing_point" value={s.points.fishing} />}
          </div>
        </Row>
        <Row
          title={<Step done={rodDone} label="Rod key" />}
          sub="Put the rod in slot 1 and empty the rest of your inventory. Fruit and shop keys live with their features."
          open={open === "keys"}
          onToggle={() => toggle("keys")}
        >
          <KeyRow label="Rod" value={s.keys.rod} onChange={(v) => update((x) => void (x.keys.rod = v))} />
          <div className="mt-2">
            <KeyRow label="Fast Reset (Swap slot)" value={s.keys.reset_slot ?? "2"} onChange={(v) => update((x) => void (x.keys.reset_slot = v))} />
          </div>
          <div className="mt-3 pt-3 border-t border-line/60">
            <div className="text-[12px] text-fg-dim mb-1 flex items-center justify-between">
              <span>Rod slot indicator</span>
              <span className="text-[11px] text-fg-mute">Auto-detected if unset</span>
            </div>
            <PointField
              target="rod_slot"
              value={s.points.rod_slot}
              clearable
              onCleared={() => update((x) => void (x.points.rod_slot = null))}
            />
          </div>
        </Row>
      </Section>

      <Section title="Devil fruits">
        <Row
          title={<Step done={ocrAvailable} label="Text recognition" />}
          sub={ocrAvailable ? "Windows OCR ready" : "Windows OCR language pack missing. Install English in Settings › Language."}
          right={<Pill tone={ocrAvailable ? "ok" : "bad"}>{ocrAvailable ? "ready" : "unavailable"}</Pill>}
        />
        <Row
          title={<Step done={dropDone} label="Drop message area" />}
          sub="The popup at the top middle of the screen: 'New Item <Fruit>' after a catch, 'A Fruit has spawned at Place' for world spawns."
          open={open === "drop"}
          onToggle={() => toggle("drop")}
        >
          <RegionField target="drop_region" value={s.regions.drop} />
          <div className="mt-3 flex items-start gap-3">
            <Button size="sm" onClick={runOcr} disabled={!roblox || !ocrAvailable} icon={<ScanText size={13} />}>
              Read now
            </Button>
            <div className="flex-1 min-w-0 text-[11px]">
              {ocrErr && <div className="text-warn">{ocrErr}</div>}
              {ocr && (
                <div className="font-mono text-fg-dim break-words select-text">
                  {ocr.text.trim() || "(nothing read)"}
                  <span className="ml-1.5 text-fg-mute">[{ocr.ocr_variant}]</span>
                  <div className="mt-1 flex gap-1.5 flex-wrap">
                    {ocr.drop && <Pill tone="fruit">{ocr.drop.is_legendary ? "legendary drop (pity 0)" : "fruit drop"}</Pill>}
                    {ocr.spawn && <Pill tone="accent">spawn: {ocr.spawn.name ?? "unknown fruit"}{ocr.spawn.location ? ` at ${ocr.spawn.location}` : ""}</Pill>}
                    {ocr.observation.entity
                      ? <Pill tone="ok">{ocr.observation.entity.canonical_name} {Math.round(ocr.observation.confidence * 100)}%</Pill>
                      : <Pill tone="warn">perception: unknown ({Math.round(ocr.observation.confidence * 100)}%)</Pill>}
                  </div>
                  {ocr.observation.unknown_reason && (
                    <div className="mt-1 text-fg-mute">{ocr.observation.unknown_reason}</div>
                  )}
                  {ocr.observation.entity && (
                    <div className="mt-1 text-fg-mute">
                      evidence: {ocr.observation.entity.evidence.map((e) => e.kind).join(" + ")}
                    </div>
                  )}
                </div>
              )}
            </div>
          </div>
        </Row>
        <Row
          title="GPO knowledge base"
          sub="Local entity database (fruits, fish, bait, UI terms) used for perception matching. Wiki sync is opt-in and never overwrites curated entries."
          open={open === "kb"}
          onToggle={() => toggle("kb")}
        >
          <KnowledgePanel />
        </Row>
      </Section>

      <Section title="Learning dataset">
        <Row
          title="Uncertain observations"
          sub="Collected when trace recording is on (Settings › Tracking). Label them to build future training data — nothing trains automatically."
          open={open === "dataset"}
          onToggle={() => toggle("dataset")}
        >
          <DatasetPanel />
        </Row>
      </Section>

      <Section title="Training dataset (gpo-vision v1)">
        <Row
          title="Versioned training data"
          sub="Session-split, validated dataset for future model training. Splits are by capture session (each macro run), so near-identical frames never leak across sets."
          open={open === "mldataset"}
          onToggle={() => toggle("mldataset")}
        >
          <MlDatasetPanel />
        </Row>
      </Section>
    </div>
  );
}

const UI_LABELS = ["fishing_bar", "bait_menu", "drop_indicator", "server_time", "fruit_indicator", "fish_result", "disconnect_screen", "reconnect_screen"];
const GAME_STATES = ["idle", "fishing", "waiting_for_bite", "bite", "catch_result", "bait_menu", "loading", "disconnected", "unknown"];

function MlDatasetPanel() {
  const [report, setReport] = useState<DatasetReport | null>(null);
  const [baseline, setBaseline] = useState<BaselineReport | null>(null);
  const [model, setModel] = useState<MlModelStatus | null>(null);
  const [readiness, setReadiness] = useState<TrainingReadiness | null>(null);
  const [samples, setSamples] = useState<MlAnnotation[]>([]);
  const [entities, setEntities] = useState<KnowledgeEntity[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [skipped, setSkipped] = useState<Record<string, boolean>>({});
  const [forms, setForms] = useState<Record<string, { ui: string; state: string; entity: string }>>({});

  const refresh = () => {
    api.mlSamples().then(setSamples).catch((e) => showToast("warn", `ML samples unavailable: ${String(e)}`));
    api.knowledgeList().then(setEntities).catch(() => undefined);
    api.mlModelStatus().then(setModel).catch(() => undefined);
  };
  useEffect(() => {
    refresh();
  }, []);

  const runValidate = async () => {
    setBusy("validate");
    try {
      setReport(await api.mlValidate());
    } catch (e) {
      showToast("error", `Validation failed: ${String(e)}`);
    } finally {
      setBusy(null);
    }
  };

  const runBaseline = async () => {
    setBusy("baseline");
    try {
      setBaseline(await api.mlBaseline());
    } catch (e) {
      showToast("error", `Baseline failed: ${String(e)}`);
    } finally {
      setBusy(null);
    }
  };

  const runReadiness = async () => {
    setBusy("readiness");
    try {
      setReadiness(await api.mlReadiness());
    } catch (e) {
      showToast("error", `Readiness check failed: ${String(e)}`);
    } finally {
      setBusy(null);
    }
  };

  const annotate = async (s: MlAnnotation, mode: "confirm" | "correct" | "unknown") => {
    const f = forms[s.image_id] ?? { ui: "", state: "", entity: "" };
    try {
      if (mode === "confirm") {
        const guess = s.entity_id ?? "";
        if (!guess) {
          showToast("warn", "Nothing to confirm — pick an entity and use Correct.");
          return;
        }
        await api.mlAnnotate(s.image_id, f.ui || undefined, f.state || undefined, guess, false, undefined);
      } else if (mode === "correct") {
        if (!f.entity.trim()) {
          showToast("warn", "Choose a GPO entity first (type to search the knowledge base).");
          return;
        }
        await api.mlAnnotate(s.image_id, f.ui || undefined, f.state || undefined, f.entity.trim(), false, undefined);
      } else {
        await api.mlAnnotate(s.image_id, f.ui || undefined, f.state || undefined, undefined, true, "human: unknown");
      }
      refresh();
    } catch (e) {
      showToast("error", `Annotate failed: ${String(e)}`);
    }
  };

  return (
    <div className="flex flex-col gap-2">
      <div className="text-[11px] text-fg-dim">
        {model ? (model.available ? `Model ${model.name} v${model.version} loaded (${model.classes.length} classes).` : `No model loaded (${model.reason ?? "none"}). OCR + heuristics active.`) : "Model status…"}
      </div>
      <div className="flex gap-2 flex-wrap">
        <Button size="sm" onClick={runValidate} disabled={busy !== null}>{busy === "validate" ? "Validating…" : "Validate dataset"}</Button>
        <Button size="sm" onClick={runBaseline} disabled={busy !== null}>{busy === "baseline" ? "Scoring…" : "Run baseline"}</Button>
        <Button size="sm" onClick={runReadiness} disabled={busy !== null}>{busy === "readiness" ? "Checking…" : "Training readiness"}</Button>
      </div>
      {readiness && (
        <div className="text-[11px] font-mono text-fg-dim break-words">
          <div>
            DATA QUALITY:{" "}
            <span className={readiness.ready ? "text-ok" : "text-warn"}>
              {readiness.training}
            </span>
          </div>
          <div className="text-fg-mute">
            These are data-quality gates only (rows, sessions, held-out test,
            coverage, leakage, validation). They say nothing about human review.
            The authoritative verdict — including the review floor — is{" "}
            <span className="text-fg-dim">Training › Readiness</span>.
          </div>
          <div>
            Entity-linked RESULTs: {readiness.entity_linked_result}/{readiness.required_entity_linked}
            <span className="ml-2 inline-block w-24 h-1.5 rounded-full bg-white/10 overflow-hidden align-middle">
              <span
                className="block h-full rounded-full bg-ok"
                style={{ width: `${Math.min(100, Math.round((readiness.entity_linked_result / Math.max(1, readiness.required_entity_linked)) * 100))}%` }}
              />
            </span>
          </div>
          <div>
            State eligible — WAITING {readiness.state_coverage?.waiting_for_bite?.eligible ?? 0}
            {" "}· BITE {readiness.state_coverage?.bite?.eligible ?? 0}
            {" "}· RESULT {readiness.state_coverage?.catch_result?.eligible ?? 0}
            {" "}(transition W/B: {((readiness.state_coverage?.waiting_for_bite?.transition ?? 0) + (readiness.state_coverage?.bite?.transition ?? 0))})
          </div>
          <div>Sessions: {readiness.sessions}/{readiness.required_sessions} · Entities: {readiness.entities} · Hard: {readiness.hard_examples}</div>
          <div>Coverage: {readiness.class_coverage_ok ? "pass" : "FAIL"} · Validation: {readiness.validation_ok ? "pass" : "FAIL"}</div>
          <div>
            Leakage: exact-file {readiness.leakage_detail?.exact_duplicate_files?.length ?? 0}
            {" "}· near-similarity {readiness.leakage_detail?.near_similarity_groups ?? 0}
            {" "}(<span className={readiness.leakage_ok ? "text-ok" : "text-warn"}>{readiness.leakage_ok ? "pass" : "FAIL"}</span>)
          </div>
          {!readiness.ready && (
            <div className="text-warn">Blocking requirement: {readiness.blocking_requirement}</div>
          )}
          {readiness.reasons.slice(0, 4).map((r) => <div key={r} className="text-fg-mute">{r.slice(0, 180)}</div>)}
        </div>
      )}
      {report && (
        <div className="text-[11px] font-mono text-fg-dim break-words">
          {report.dataset}: {report.images} images, {report.labeled} labeled / {report.unlabeled} unlabeled ·
          train {report.sessions_train} / val {report.sessions_validation} / test {report.sessions_test} sessions ·
          leakage groups {report.leakage_sessions.length} (exact-file {(report.same_file_cross_split ?? []).length} · near-sim {report.near_similarity_groups ?? 0}) · corrupt {report.corrupt_files.length} ·
          invalid {report.invalid_entity_ids.length + report.invalid_bboxes.length} · orphans {report.orphan_annotations.length + report.orphan_images.length} ·
          dup groups {report.duplicate_groups} · near-dups {report.near_duplicate_pairs} ·
          class ratio {report.min_max_class_ratio.toFixed(2)} · <span className={report.ok ? "text-ok" : "text-bad"}>{report.ok ? "OK" : "FAIL"}</span>
        </div>
      )}
      {baseline && (
        <div className="text-[11px] font-mono text-fg-dim break-words">
          {baseline.insufficient_data
            ? `INSUFFICIENT LABELED DATA (${baseline.labeled_samples} samples).`
            : Object.entries(baseline.stages).map(([k, m]) => (
              <div key={k}>{k}: acc {m.accuracy.toFixed(3)} · F1 {m.f1.toFixed(3)} · unknown {Math.round(m.unknown_rate * 100)}% · {m.mean_latency_ms.toFixed(2)}ms</div>
            ))}
          {baseline.notes.slice(0, 2).map((n) => <div key={n} className="text-fg-mute">{n.slice(0, 160)}</div>)}
        </div>
      )}
      {samples.filter((s) => !skipped[s.image_id]).slice(0, 8).map((s) => {
        const f = forms[s.image_id] ?? { ui: "", state: "", entity: "" };
        const setF = (k: "ui" | "state" | "entity", v: string) => setForms((m) => ({ ...m, [s.image_id]: { ...f, [k]: v } }));
        return (
          <div key={s.image_id} className="rounded-lg border border-line/70 px-2.5 py-1.5 text-[11px]">
            <div className="flex items-center gap-2 flex-wrap">
              <Pill tone={s.hard_example ? "warn" : "mute"}>{s.hard_example ? `hard: ${s.hard_reason ?? "?"}`.slice(0, 60) : s.task}</Pill>
              <span className="font-mono text-fg-mute truncate">{s.ocr_text || "(no OCR)"} · {s.session_id}</span>
              {s.entity_id && <Pill tone="accent">{s.entity_id}</Pill>}
            </div>
            <div className="mt-1.5 flex items-center gap-1.5 flex-wrap">
              <select value={f.ui} onChange={(e) => setF("ui", e.target.value)} className="rounded-lg bg-black/30 border border-line px-1.5 py-1 text-[11px] text-fg">
                <option value="">UI label…</option>
                {UI_LABELS.map((l) => <option key={l} value={l}>{l}</option>)}
              </select>
              <select value={f.state} onChange={(e) => setF("state", e.target.value)} className="rounded-lg bg-black/30 border border-line px-1.5 py-1 text-[11px] text-fg">
                <option value="">Game state…</option>
                {GAME_STATES.map((g) => <option key={g} value={g}>{g}</option>)}
              </select>
              <input
                value={f.entity}
                onChange={(e) => setF("entity", e.target.value)}
                placeholder="GPO entity…"
                list={`kb-${s.image_id}`}
                className="w-36 rounded-lg bg-black/30 border border-line px-2 py-1 text-[11px] text-fg"
              />
              <datalist id={`kb-${s.image_id}`}>
                {entities.slice(0, 400).map((e) => <option key={e.id} value={e.id}>{e.name} ({e.category})</option>)}
              </datalist>
              <Button size="sm" kind="primary" onClick={() => annotate(s, "confirm")}>Confirm</Button>
              <Button size="sm" onClick={() => annotate(s, "correct")}>Correct</Button>
              <Button size="sm" kind="ghost" onClick={() => annotate(s, "unknown")}>Unknown</Button>
              <Button size="sm" kind="ghost" onClick={() => setSkipped((m) => ({ ...m, [s.image_id]: true }))}>Skip</Button>
            </div>
          </div>
        );
      })}
      {samples.length === 0 && (
        <div className="text-[11px] text-fg-mute">No training samples yet. They are collected once per ended reel while trace recording is on.</div>
      )}
    </div>
  );
}

function Step({ done, label }: { done: boolean; label: string }) {
  return (
    <span className="inline-flex items-center gap-2">
      {done ? <Check size={14} className="text-ok" /> : <Circle size={14} className="text-fg-mute" />}
      {label}
    </span>
  );
}

function KeyRow({ label, value, onChange }: { label: string; value: string; onChange: (v: string) => void }) {
  return (
    <div className="flex items-center">
      <div className="text-fg-dim w-28">{label}</div>
      <KeyCapture single value={value} onChange={onChange} />
    </div>
  );
}

function KnowledgePanel() {
  const [stats, setStats] = useState<KnowledgeStats | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [result, setResult] = useState<WikiSyncResult | null>(null);

  const refresh = () => {
    api.knowledgeStats().then(setStats).catch((e) => showToast("warn", `Knowledge stats unavailable: ${String(e)}`));
  };
  useEffect(() => {
    refresh();
  }, []);

  const sync = async () => {
    setSyncing(true);
    try {
      const r = await api.knowledgeSyncWiki();
      setResult(r);
      refresh();
      showToast("info", `Wiki sync: ${r.added} added, ${r.updated} updated, ${r.skipped} skipped.`);
    } catch (e) {
      showToast("error", `Wiki sync failed: ${String(e)}`);
    } finally {
      setSyncing(false);
    }
  };

  return (
    <div className="flex flex-col gap-2">
      <div className="text-[11px] text-fg-dim">
        {stats
          ? `${stats.entities} entities (v${stats.version}): ${stats.fruits} fruits, ${stats.fish} fish, ${stats.bait} bait, ${stats.ui_terms} UI terms, ${stats.overlay} wiki overlay.`
          : "Loading…"}
      </div>
      <div>
        <Button size="sm" onClick={sync} disabled={syncing}>
          {syncing ? "Syncing wiki (up to a minute)…" : "Sync GPO Wiki (opt-in)"}
        </Button>
      </div>
      {result && (
        <div className="text-[11px] font-mono text-fg-dim break-words">
          pages: {result.fetched_pages}{result.truncated ? "+" : ""} · parsed: {result.parsed} · added: {result.added} ·
          updated: {result.updated} · skipped: {result.skipped}
          {result.errors.slice(0, 3).map((e) => (
            <div key={e} className="text-warn">{e.slice(0, 120)}</div>
          ))}
        </div>
      )}
    </div>
  );
}

function DatasetPanel() {
  const [samples, setSamples] = useState<DatasetSample[]>([]);
  const [labels, setLabels] = useState<Record<string, string>>({});

  const refresh = () => {
    api.datasetList().then(setSamples).catch((e) => showToast("warn", `Dataset unavailable: ${String(e)}`));
  };
  useEffect(() => {
    refresh();
  }, []);

  const saveLabel = async (id: string, correct: boolean) => {
    const label = (labels[id] ?? "").trim();
    if (!label && !correct) {
      showToast("warn", "Type the correct entity name first, or use Confirm when the prediction is right.");
      return;
    }
    try {
      const guess = samples.find((s) => s.id === id)?.observation.entity?.canonical_name ?? "";
      await api.datasetLabel(id, correct ? guess || label : label, correct);
      refresh();
    } catch (e) {
      showToast("error", `Label failed: ${String(e)}`);
    }
  };

  const needLabel = samples.filter((s) => s.needs_label && !s.label);
  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <div className="text-[11px] text-fg-dim">
          {samples.length} samples{samples.length >= 200 ? " (capped)" : ""} · {needLabel.length} need labels
        </div>
        <div className="ml-auto">
          <Button size="sm" kind="ghost" onClick={() => api.datasetOpen().catch((e) => showToast("error", `Open failed: ${String(e)}`))}>
            Open folder
          </Button>
        </div>
      </div>
      {samples.slice(0, 10).map((s) => (
        <div key={s.id} className="rounded-lg border border-line/70 px-2.5 py-1.5 text-[11px]">
          <div className="flex items-center gap-2 flex-wrap">
            <Pill tone={s.needs_label && !s.label ? "warn" : "mute"}>
              {s.label ? `labeled: ${s.label}` : s.observation.entity ? `guess: ${s.observation.entity.canonical_name} ${Math.round(s.observation.confidence * 100)}%` : `unknown ${Math.round(s.observation.confidence * 100)}%`}
            </Pill>
            <span className="font-mono text-fg-mute truncate">{s.ocr_text || "(no OCR)"} · {s.event_type} · {s.region}</span>
          </div>
          {!s.label && (
            <div className="mt-1.5 flex items-center gap-1.5 flex-wrap">
              <input
                value={labels[s.id] ?? ""}
                onChange={(e) => setLabels((m) => ({ ...m, [s.id]: e.target.value }))}
                placeholder="Correct entity name…"
                className="w-40 rounded-lg bg-black/30 border border-line px-2 py-1 text-[12px] text-fg"
              />
              <Button size="sm" kind="primary" onClick={() => saveLabel(s.id, false)}>Correct</Button>
              {s.observation.entity && <Button size="sm" onClick={() => saveLabel(s.id, true)}>Confirm</Button>}
            </div>
          )}
        </div>
      ))}
      {samples.length === 0 && (
        <div className="text-[11px] text-fg-mute">No samples yet. Enable trace recording, run a health check with uncertain readings, and they appear here.</div>
      )}
    </div>
  );
}
