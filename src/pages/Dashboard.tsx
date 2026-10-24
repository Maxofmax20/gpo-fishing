import { useEffect, useState } from "react";
import { ArrowRight, Check, Fish, Focus, Pause, Play, Repeat, ShieldCheck, Square, TriangleAlert } from "lucide-react";
import { api } from "../lib/ipc";
import { showToast, useStore, isActive } from "../lib/store";
import { useVisiblePoll } from "../lib/useVisiblePoll";
import { STATE_LABEL } from "../lib/types";
import { Button, CustomSelect, Disclosure, Pill, cx, fmtRuntime } from "../components/primitives";
import { StateBadge } from "../components/StateIcon";
import { LogList } from "../components/LogList";
import { LiveAreas } from "../components/LiveAreas";
import { CollectionControl } from "../components/CollectionControl";
import ConfirmModal from "../components/ConfirmModal";
import type { CustomMacro, MlCollectionStatus, MlPreflight, RecorderStatus } from "../lib/types";

export default function Dashboard({
  onNavigate,
}: {
  onNavigate?: (tab: "dashboard" | "journal" | "setup" | "features" | "macros" | "settings") => void;
}) {
  const state = useStore((s) => s.state);
  const detail = useStore((s) => s.detail);
  const stats = useStore((s) => s.stats);
  const roblox = useStore((s) => s.roblox);
  const settings = useStore((s) => s.settings);
  const [runtime, setRuntime] = useState(stats.runtime_s);
  const active = isActive(state);
  const total = stats.total;
  const totalRuntime = total.runtime_s + (runtime - stats.runtime_s);

  useEffect(() => {
    setRuntime(stats.runtime_s);
    if (!active) return;
    const t = setInterval(() => setRuntime((r) => r + 1), 1000);
    return () => clearInterval(t);
  }, [stats.runtime_s, active]);

  const hk = settings?.hotkeys.toggle ?? "F1";
  const updateSettings = useStore((s) => s.update);
  const [collectGate, setCollectGate] = useState(false);
  const [preflightFail, setPreflightFail] = useState<MlPreflight | null>(null);
  const [starting, setStarting] = useState(false);

  const doStart = async () => {
    setStarting(true);
    try {
      await api.botToggle();
    } catch (e) {
      showToast("error", `Bot toggle failed: ${String(e)}`);
    } finally { setStarting(false); }
  };

  // Start gate: pause/resume is never gated. A fresh start requires either
  // explicit consent (collection OFF → ask) or a passing collection
  // preflight (collection ON → verify, never silently record zero frames).
  const handleStartPress = async () => {
    if (active || state === "paused") {
      void doStart();
      return;
    }
    const traceOn = settings?.fishing.trace ?? false;
    if (!traceOn && localStorage.getItem("collection-off-acknowledged") !== "yes") {
      setCollectGate(true);
      return;
    }
    if (!traceOn) { await doStart(); return; }
    setStarting(true);
    try {
      const pf = await api.mlPreflight();
      if (!pf.ok) {
        setPreflightFail(pf);
        return;
      }
      await doStart();
    } catch (e) {
      showToast("error", `Collection preflight failed: ${String(e)}`);
    } finally {
      setStarting(false);
    }
  };

  const enableAndStart = async () => {
    setCollectGate(false);
    setStarting(true);
    try {
      await updateSettings((x) => void (x.fishing.trace = true));
      const pf = await api.mlPreflight();
      if (!pf.ok) {
        setPreflightFail(pf);
        return;
      }
      await doStart();
    } catch (e) {
      showToast("error", `Enable collection failed: ${String(e)}`);
    } finally {
      setStarting(false);
    }
  };

  const configured = !!settings && settings.regions.bar.w > 0.02 && settings.regions.bar.h > 0.02 && !!settings.keys.rod;
  const readiness = !roblox ? "Open Roblox to fish" : !roblox.is_foreground ? "Focus Roblox to continue" : !configured ? "Check your calibration" : "Ready for fishing";
  const checks = [
    { label: "Game", ok: !!roblox, value: roblox ? "Detected" : "Not found" },
    { label: "Focus", ok: !!roblox?.is_foreground, value: roblox?.is_foreground ? "Active" : "Waiting" },
    { label: "Calibration", ok: configured, value: configured ? "Configured" : "Needs setup" },
  ];
  return (
    <div className="pb-4 pt-4">
      <section className="session-card mx-4 p-4" aria-label="Fishing session">
        <div className="flex items-start gap-3">
          <StateBadge state={state} />
          <div className="flex-1 min-w-0">
            <div className="text-[11px] uppercase tracking-[.12em] text-fg-dim font-semibold">Current session</div>
            <h1 className="text-[20px] font-semibold leading-tight mt-1 break-words">{STATE_LABEL[state]}</h1>
            <p className="text-[12px] text-fg-dim mt-1 leading-relaxed break-words" role="status">{detail || readiness}</p>
          </div>
        </div>
        <div className="grid grid-cols-3 gap-2 my-4">
          {checks.map(check => <div key={check.label} className="text-xs min-w-0">
            <div className="text-fg-mute">{check.label}</div>
            <div className={cx("flex items-center gap-1 mt-1", check.ok ? "text-ok" : "text-warn")}>
              {check.ok ? <Check size={12} className="shrink-0" /> : <TriangleAlert size={12} className="shrink-0" />}<span>{check.value}</span>
            </div>
          </div>)}
        </div>
        <div className="flex gap-2">
          <Button className="flex-1" kind={active ? "default" : "primary"} onClick={handleStartPress} busy={starting}
            icon={active ? <Pause size={15} /> : <Play size={15} />}>
            {starting ? "Please wait…" : active ? "Pause fishing" : state === "paused" ? "Resume fishing" : "Start fishing"}
          </Button>
          {(active || state === "paused") && <Button kind="danger" label="Stop fishing" title="Stop fishing" busy={starting}
            onClick={async () => { setStarting(true); try { await api.botStop(); } catch (e) { showToast("error", `Stop failed: ${String(e)}`); } finally { setStarting(false); } }}
            icon={<Square size={14} />}>Stop</Button>}
        </div>
        <div className="mt-3 flex items-center justify-between gap-2 text-xs text-fg-mute">
          <span><kbd className="font-mono text-fg-dim">{hk}</kbd> start / pause</span>
          <span className="font-mono tabular-nums">{fmtRuntime(runtime)}</span>
        </div>
      </section>
      <div className="grid grid-cols-2 gap-2 mx-4 my-3" aria-label="Session counters">
        <SessionMetric label="Confirmed fish" value={stats.fish} detail={`${Math.round(stats.success_rate * 100)}% verified catch rate`} />
        <SessionMetric label="Devil fruits" value={stats.fruits} detail={stats.last_fruit || "No fruit confirmed this session"} />
        <SessionMetric label="Fruit pity" value={stats.pity_fruit ?? 0} detail="Fish since last confirmed fruit" />
        <SessionMetric label="Legendary pity" value={stats.pity_legendary ?? 0} detail={settings?.fruit_storage?.all_seeing_eye ? "All-Seeing Eye enabled" : "Confirmed game counter"} />
      </div>
      {onNavigate && <div className="mx-4 mb-4 grid grid-cols-2 gap-2">
        <Button onClick={() => onNavigate("setup")} icon={<Focus size={14} />}>Calibration</Button>
        <Button onClick={() => onNavigate("features")} icon={<ShieldCheck size={14} />}>Automation</Button>
      </div>}
      <section className="mx-4 mb-3 rounded-xl border border-line p-3" aria-label="Latest catch">
        <div className="flex items-center gap-2 text-xs text-fg-mute"><Fish size={13} />Latest confirmed catch</div>
        <div className="font-medium mt-1 truncate">{stats.last_fish || "Waiting for your first catch"}</div>
        {(stats.unverified ?? 0) > 0 && <p className="text-xs text-warn mt-1">{stats.unverified} reels unverified · counters were not guessed.</p>}
      </section>
      <Disclosure title="Activity" hint="Recent events, errors and recovery decisions"><LogList height={220} /></Disclosure>
      <Disclosure title="Live detection" hint="Open previews only when diagnosing calibration"><LiveAreas /></Disclosure>
      <Disclosure title="Session details & all-time totals" hint="Pace, restock timing and saved statistics">
        <Stat label="Fish / hour" value={Math.round(stats.fish_per_hour ?? 0)} />
        <Stat label="Failed reels" value={stats.failed} />
        <Stat label="Recoveries" value={stats.restarts} />
        <Stat label="Bait restock" value={settings?.features.auto_purchase ? `${Math.max(0, (settings.purchase.every_n_catches ?? 10) - (stats.since_purchase ?? 0))} fish left` : "Disabled"} />
        <Stat label="All-time fish" value={total.fish} />
        <Stat label="All-time fruits" value={total.fruits} />
        <Stat label="All-time time" value={fmtRuntime(totalRuntime)} />
        <Stat label="Sessions" value={total.sessions} />
        <Stat label="Estimated earnings" value={`~${(total.estimated_peli ?? 0).toLocaleString()} Peli`} />
        {onNavigate && <div className="px-4 py-3"><Button size="sm" onClick={() => onNavigate("journal")} icon={<ArrowRight size={13} />}>Open catch history</Button></div>}
      </Disclosure>
      <Disclosure title="Custom automation" hint="Recorded sequences and playback"><QuickMacroRow onNavigate={onNavigate} /></Disclosure>
      <Disclosure title="Training data collection" hint={settings?.fishing.trace ? "Recording enabled" : "Optional · recording off"}><CollectionStrip /><MlDataCard /></Disclosure>
      {collectGate && (
        <div className="absolute inset-0 z-50 grid place-items-center bg-black/60 backdrop-blur-sm px-6" onClick={() => setCollectGate(false)}>
          <div className="w-full max-w-[340px] rounded-2xl border border-line bg-[#0c101a] p-4 shadow-2xl" onClick={(e) => e.stopPropagation()}>
            <div className="font-semibold text-[14px]">Data collection is OFF</div>
            <div className="mt-1.5 text-[12px] text-fg-dim leading-relaxed">
              Trace recording is disabled, so this run would save <b>zero training frames</b>. Enable collection to record WAITING / BITE / RESULT frames automatically (small game-UI crops only).
            </div>
            <div className="mt-4 flex flex-col gap-2">
              <Button kind="primary" onClick={enableAndStart} disabled={starting}>
                {starting ? "Working…" : "Enable collection + Start"}
              </Button>
              <div className="flex justify-end gap-2">
                <Button size="sm" onClick={() => setCollectGate(false)}>
                  Cancel
                </Button>
                <Button size="sm" onClick={() => { setCollectGate(false); localStorage.setItem("collection-off-acknowledged", "yes"); void doStart(); }}>
                  Start without recording
                </Button>
              </div>
            </div>
          </div>
        </div>
      )}
      <ConfirmModal
        open={preflightFail !== null}
        title="DATA COLLECTION FAILED"
        body={`The fishing macro will not start because gameplay training data cannot be recorded:\n${(preflightFail?.checks ?? []).filter((c) => !c.ok).map((c) => `• ${c.name}: ${c.detail}`).join("\n")}`}
        backupNote="To fish without recording, turn trace recording off in Settings › Tracking."
        confirmLabel="Retry"
        danger={false}
        busy={starting}
        onConfirm={() => { setPreflightFail(null); void handleStartPress(); }}
        onCancel={() => setPreflightFail(null)}
      />
    </div>
  );
}

function CollectionStrip() {
  const settings = useStore((s) => s.settings);
  const ml = useStore((s) => s.ml);
  const state = useStore((s) => s.state);
  const active = isActive(state);
  if (!settings) {
    return (
      <div className="px-4 pb-2">
        <Pill tone="mute">DATA COLLECTION: …</Pill>
      </div>
    );
  }
  const traceOn = settings.fishing.trace;
  const recording = traceOn && (ml?.collecting ?? false);
  if (recording) {
    return (
      <div className="px-4 pb-2 flex items-center gap-2">
        <Pill tone="ok">● DATA COLLECTION: RECORDING</Pill>
        <span className="text-[11px] text-fg-mute font-mono">{ml?.samples ?? 0} frames this run</span>
      </div>
    );
  }
  if (active) {
    return (
      <div className="px-4 pb-2 flex items-center gap-2 flex-wrap">
        <Pill tone="warn">○ DATA COLLECTION: OFF</Pill>
        <span className="text-[11px] text-warn">Fishing without recording — no training frames this run.</span>
      </div>
    );
  }
  if (traceOn) {
    return (
      <div className="px-4 pb-2 flex items-center gap-2">
        <Pill tone="accent">○ DATA COLLECTION: ARMED</Pill>
        <span className="text-[11px] text-fg-mute">Recording starts with the macro.</span>
      </div>
    );
  }
  return (
    <div className="px-4 pb-2 flex items-center gap-2 flex-wrap">
      <Pill tone="mute">○ DATA COLLECTION: OFF</Pill>
      <span className="text-[11px] text-fg-mute">Turn on trace recording (Settings › Tracking) to build training data.</span>
    </div>
  );
}

function agoStr(ms: number | null): string {
  if (ms == null) return "no frames yet";
  const s = Math.max(0, Math.round((Date.now() - ms) / 1000));
  if (s < 60) return `${s}s ago`;
  return `${Math.floor(s / 60)}m ${s % 60}s ago`;
}

function MlDataCard() {
  const ml = useStore((s) => s.ml);
  const settings = useStore((s) => s.settings);
  const state = useStore((s) => s.state);
  const active = isActive(state);
  const traceOn = settings?.fishing.trace ?? false;
  const [col, setCol] = useState<MlCollectionStatus | null>(null);
  useVisiblePoll(async () => {
    try {
      setCol(await api.mlCollectionStatus());
    } catch {
      // Keep last known state; the session event below stays authoritative.
    }
  }, 3000, active || traceOn);

  if (col) {
    const shortId = col.session_id ? col.session_id.split("_").slice(-1)[0] ?? col.session_id : null;
    return (
      <div className="px-4 py-2.5 flex flex-col gap-1.5">
        <div className="flex items-center gap-2 flex-wrap">
          <Pill tone={col.recording ? "ok" : col.session_active ? "warn" : "mute"}>
            {col.recording ? "● RECORDING" : col.session_active ? "○ session open — NOT recording" : traceOn ? "○ ARMED" : "○ OFF"}
          </Pill>
          {shortId && <span className="font-mono text-[11px] text-fg-mute">session {shortId}</span>}
          <span className="font-mono text-[11px] text-fg-mute">{col.dataset}</span>
          <CollectionControl />
        </div>
        <div className="flex flex-wrap gap-x-4 gap-y-1 font-mono text-[12px] text-fg-dim">
          <span>Frames: <b className="text-fg">{col.samples_written}</b></span>
          <span>Reels: <b className="text-fg">{col.reels}</b></span>
          <span>Verified: <b className="text-fg">{col.verified}</b></span>
          <span>Hard: <b className="text-warn">{col.hard_examples}</b></span>
          {col.dropped > 0 && <span>Dropped: <b className="text-bad">{col.dropped}</b></span>}
          {col.write_errors > 0 && <span>Write errors: <b className="text-bad">{col.write_errors}</b></span>}
        </div>
        <div className="text-[11px] text-fg-dim font-mono">
          Last capture: {agoStr(col.last_capture_ms)} · dataset total: {col.dataset_samples} samples
        </div>
      </div>
    );
  }
  if (!ml) {
    return (
      <div className="px-4 py-2.5 text-[12px] text-fg-mute">
        {traceOn
          ? "Start the macro — collection arms automatically."
          : "Collection is OFF — enable trace recording (Settings › Tracking) or accept the prompt at Start."}
      </div>
    );
  }
  const shortId = ml.session_id ? ml.session_id.split("_").slice(-1)[0] ?? ml.session_id : null;
  return (
    <div className="px-4 py-2.5 flex flex-col gap-1.5">
      <div className="flex items-center gap-2">
        <Pill tone={ml.collecting ? "ok" : "mute"}>
          {ml.collecting ? "● ML DATA: COLLECTING" : "ML DATA: session complete"}
        </Pill>
        {shortId && <span className="font-mono text-[11px] text-fg-mute">session {shortId}</span>}
      </div>
      <div className="flex flex-wrap gap-x-4 gap-y-1 font-mono text-[12px] text-fg-dim">
        <span>Samples: <b className="text-fg">{ml.samples}</b></span>
        <span>Reels: <b className="text-fg">{ml.reels}</b></span>
        <span>Hard: <b className="text-warn">{ml.hard_examples}</b></span>
        {ml.dropped > 0 && <span>Dropped: <b className="text-bad">{ml.dropped}</b></span>}
      </div>
      {!ml.collecting && ml.session_id && (
        <div className="text-[11px] text-fg-dim">
          Collected {ml.samples} samples ({ml.hard_examples} hard) · {ml.pending_annotation} pending annotation · {ml.total_samples} total
          {ml.quality_ok === false && <span className="text-bad"> · quality WARNING</span>}
          {ml.quality_warnings.slice(0, 2).map((w) => (
            <div key={w} className="text-bad break-words">{w.slice(0, 160)}</div>
          ))}
        </div>
      )}
    </div>
  );
}

function Stat({ label, value, extra, sub }: { label: string; value: React.ReactNode; extra?: React.ReactNode; sub?: string }) {
  return (
    <div className="flex items-center px-4 h-11 border-b border-line">
      <div className="text-fg-dim">
        {label}
        {sub && <span className="ml-2 text-[11px] text-fg-mute font-mono">{sub}</span>}
      </div>
      <div className="ml-auto flex items-center gap-3">
        {extra}
        <div className="font-mono tabular-nums text-[14px] font-medium">{value}</div>
      </div>
    </div>
  );
}

function QuickMacroRow({
  onNavigate,
}: {
  onNavigate?: (tab: "dashboard" | "journal" | "setup" | "features" | "macros" | "settings") => void;
}) {
  const [macros, setMacros] = useState<CustomMacro[]>([]);
  const [status, setStatus] = useState<RecorderStatus | null>(null);
  const [selectedName, setSelectedName] = useState("");
  const [speed, setSpeed] = useState("1.0");

  const refresh = () => {
    Promise.all([api.macroList(), api.macroStatus()])
      .then(([list, st]) => {
        setMacros(list);
        setStatus(st);
        if (!selectedName && list.length > 0) {
          setSelectedName(list[0].name);
        }
      })
      .catch((e) => showToast("warn", `Macro status unavailable: ${String(e)}`));
  };

  useVisiblePoll(refresh, status?.is_recording || status?.is_playing ? 800 : 2500);

  const activeMacro =
    macros.find((m) => m.name === selectedName || m.id === selectedName) || macros[0];

  if (status?.is_recording) {
    return (
      <div className="flex items-center px-4 h-12 border-b border-line bg-bad-soft/20">
        <div className="flex items-center gap-2">
          <Pill tone="bad">🔴 Recording ({status.recorded_steps_count} steps)</Pill>
          <span className="text-[12px] text-fg-dim">Press F8 in Roblox to finish</span>
        </div>
        <div className="ml-auto flex items-center gap-1.5">
          <Button size="sm" kind="primary" onClick={() => api.macroRecord("stop", "").then(refresh)}>
            Save (F8)
          </Button>
          <Button size="sm" kind="danger" onClick={() => api.macroRecord("cancel").then(refresh)}>
            Cancel
          </Button>
        </div>
      </div>
    );
  }

  if (status?.is_playing) {
    return (
      <div className="flex items-center px-4 h-12 border-b border-line bg-white/[0.02]">
        <div className="flex items-center gap-2">
          <Pill tone="fruit">▶️ Playing &ldquo;{status.playing_macro_name}&rdquo;</Pill>
          <span className="text-[12px] text-fg-dim font-mono">
            {status.is_looping ? `Loop #${status.current_loop}` : "1x"}
          </span>
        </div>
        <div className="ml-auto">
          <Button size="sm" kind="danger" icon={<Square size={12} />} onClick={() => api.macroPlay("stop").then(refresh)}>
            Stop
          </Button>
        </div>
      </div>
    );
  }

  if (macros.length === 0) {
    return (
      <div className="flex items-center justify-between px-4 h-12 border-b border-line text-[12px] text-fg-dim">
        <span>No custom macros recorded yet.</span>
        {onNavigate && (
          <Button size="sm" kind="default" onClick={() => onNavigate("macros")}>
            Record in Studio →
          </Button>
        )}
      </div>
    );
  }

  return (
    <div className="flex items-center px-4 h-12 border-b border-line">
      <div className="flex items-center gap-2">
        <CustomSelect
          value={activeMacro?.name || ""}
          options={macros.map((m) => ({
            value: m.name,
            label: m.name,
            sub: `${m.steps.length} steps`,
          }))}
          onChange={(v) => setSelectedName(v)}
        />
        <CustomSelect
          value={speed}
          options={[
            { value: "0.5", label: "0.5x" },
            { value: "0.75", label: "0.75x" },
            { value: "1.0", label: "1.0x (Normal)" },
            { value: "1.5", label: "1.5x" },
            { value: "2.0", label: "2.0x" },
            { value: "3.0", label: "3.0x" },
            { value: "5.0", label: "5.0x" },
            { value: "10.0", label: "10.0x" },
            { value: "25.0", label: "25.0x" },
            { value: "50.0", label: "50.0x" },
            { value: "100.0", label: "100x (Turbo Max)" },
          ]}
          onChange={(v) => setSpeed(v)}
        />
      </div>
      <div className="ml-auto flex items-center gap-1.5">
        <Button
          size="sm"
          kind="primary"
          icon={<Play size={12} />}
          onClick={() =>
            activeMacro &&
            api.macroPlay("play", activeMacro.name, false, parseFloat(speed), 1).then(refresh)
          }
        >
          Play
        </Button>
        <Button
          size="sm"
          kind="default"
          icon={<Repeat size={12} />}
          onClick={() =>
            activeMacro &&
            api.macroPlay("loop", activeMacro.name, true, parseFloat(speed), undefined).then(refresh)
          }
        >
          Loop
        </Button>
      </div>
    </div>
  );
}

function SessionMetric({ label, value, detail }: { label: string; value: number; detail: string }) {
  return <div className="metric-card"><div className="text-xs text-fg-dim">{label}</div><div className="metric-value mt-1">{value}</div><div className="text-xs text-fg-mute mt-1 leading-snug break-words">{detail}</div></div>;
}
