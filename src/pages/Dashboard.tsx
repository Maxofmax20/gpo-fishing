import { useEffect, useState } from "react";
import { Pause, Play, Repeat, Square } from "lucide-react";
import { api } from "../lib/ipc";
import { useStore, isActive } from "../lib/store";
import { STATE_LABEL } from "../lib/types";
import { Button, CustomSelect, Pill, Section, cx, fmtRuntime } from "../components/primitives";
import { StateBadge } from "../components/StateIcon";
import { LogList } from "../components/LogList";
import { LiveAreas } from "../components/LiveAreas";
import type { CustomMacro, RecorderStatus } from "../lib/types";

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

  const sessionNote = (n: number) => (active || state === "paused" ? `this run ${n}` : undefined);

  const hk = settings?.hotkeys.toggle ?? "F1";

  return (
    <div className="pb-4">
      <div className="px-4 pt-4 pb-3 flex items-center gap-3">
        <StateBadge state={state} />
        <div className="flex-1 min-w-0">
          <div className="text-[15px] font-semibold leading-tight">{STATE_LABEL[state]}</div>
          <div className="text-[12px] text-fg-dim truncate">
            {detail ?? (roblox ? (roblox.is_foreground ? "Roblox focused" : "Roblox in background") : "Roblox not detected")}
          </div>
        </div>
        <div className="flex gap-1.5">
          <Button kind={active ? "default" : "primary"} onClick={() => api.botToggle()} icon={active ? <Pause size={14} /> : <Play size={14} />}>
            {active ? "Pause" : state === "paused" ? "Resume" : "Start"}
          </Button>
          {(active || state === "paused") && <Button kind="danger" onClick={() => api.botStop()} icon={<Square size={13} />} />}
        </div>
      </div>
      <div className="px-4 pb-3 text-[11px] text-fg-mute">
        Press <span className="font-mono text-fg-dim">{hk}</span> anywhere to start or pause.
      </div>

      <Section
        title="Custom Automation"
        action={
          onNavigate && (
            <button
              onClick={() => onNavigate("macros")}
              className="text-[11px] text-accent hover:underline flex items-center gap-1 font-medium cursor-pointer"
            >
              Open Studio →
            </button>
          )
        }
      >
        <QuickMacroRow onNavigate={onNavigate} />
      </Section>

      <Section title="All time">
        <Stat
          label="Fish caught"
          value={total.fish}
          sub={sessionNote(stats.fish)}
          extra={total.last_fish ? <Pill tone="accent">{total.last_fish.slice(0, 40)}</Pill> : undefined}
        />
        <Stat label="Failed reels" value={total.failed} sub={sessionNote(stats.failed)} />
        <Stat
          label="Devil fruits"
          value={total.fruits}
          sub={sessionNote(stats.fruits)}
          extra={total.last_fruit ? <Pill tone="fruit">{total.last_fruit.slice(0, 40)}</Pill> : undefined}
        />
        <Stat
          label="Fruit pity (since last fruit)"
          value={stats.pity_fruit ?? 0}
          extra={<Pill tone="warn">⚡ {stats.pity_fruit ?? 0} fish</Pill>}
        />
        <Stat
          label="Legendary pity"
          value={stats.pity_legendary ?? 0}
          extra={<Pill tone="fruit">🌟 {stats.pity_legendary ?? 0} fish</Pill>}
        />
        <Stat label="Bait bought" value={total.bait_purchased} sub={sessionNote(stats.bait_purchased)} />
        <Stat label="Time fishing" value={fmtRuntime(totalRuntime)} sub={active || state === "paused" ? `this run ${fmtRuntime(runtime)}` : undefined} />
        <Stat label="Sessions" value={total.sessions} />
        <Stat label="Bite rate" value={`${Math.round(stats.success_rate * 100)}%`} extra={<Bar pct={stats.success_rate} />} />
        {stats.restarts > 0 && <Stat label="Recoveries" value={stats.restarts} />}
        {total.last_spawn && <Stat label="Last spawn" value={total.last_spawn} />}
      </Section>

      <Section title="Schedules & Automation">
        <Stat
          label="Bait restock"
          value={
            settings?.features.auto_purchase
              ? `${Math.max(0, (settings?.purchase.every_n_catches ?? 10) - (stats.since_purchase ?? 0))} fish left`
              : "Disabled"
          }
          sub={settings?.features.auto_purchase ? `every ${settings?.purchase.every_n_catches ?? 10} fish` : undefined}
          extra={
            settings?.features.auto_purchase ? (
              <Pill tone="accent">🛒 {stats.since_purchase ?? 0} / {settings?.purchase.every_n_catches ?? 10}</Pill>
            ) : undefined
          }
        />
        <Stat
          label="Progress notification"
          value={
            settings?.webhook.progress
              ? `${Math.max(0, (settings?.webhook.progress_every_n ?? 50) - (stats.since_progress ?? 0))} fish left`
              : "Disabled"
          }
          sub={settings?.webhook.progress ? `every ${settings?.webhook.progress_every_n ?? 50} fish` : undefined}
          extra={
            settings?.webhook.progress ? (
              <Pill tone="warn">📱 {stats.since_progress ?? 0} / {settings?.webhook.progress_every_n ?? 50}</Pill>
            ) : undefined
          }
        />
      </Section>

      <Section title="Economy & Efficiency">
        <Stat
          label="Catch rate"
          value={`${Math.round(stats.fish_per_hour ?? 0)} fish / hr`}
          extra={<Pill tone="accent">⚡ Hourly pace</Pill>}
        />
        <Stat
          label="Estimated earnings"
          value={`~${(total.estimated_peli ?? (total.fish * 95)).toLocaleString()} Peli`}
          sub={sessionNote(stats.estimated_peli ?? (stats.fish * 95))}
          extra={<Pill tone="warn">💰 Fish market value</Pill>}
        />
      </Section>

      <Section title="Live">
        <LiveAreas />
      </Section>

      <Section title="Activity">
        <LogList height={220} />
      </Section>
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

function Bar({ pct }: { pct: number }) {
  return (
    <div className="w-24 h-1.5 rounded-full bg-white/10 overflow-hidden">
      <div className={cx("h-full rounded-full", pct > 0.7 ? "bg-ok" : pct > 0.4 ? "bg-warn" : "bg-bad")} style={{ width: `${pct * 100}%` }} />
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
      .catch(() => undefined);
  };

  useEffect(() => {
    refresh();
    const t = setInterval(
      refresh,
      status?.is_recording || status?.is_playing ? 800 : 2500,
    );
    return () => clearInterval(t);
  }, [status?.is_recording, status?.is_playing]);

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
            { value: "0.75", label: "0.75x" },
            { value: "1.0", label: "1.0x" },
            { value: "1.25", label: "1.25x" },
            { value: "1.5", label: "1.5x" },
            { value: "2.0", label: "2.0x" },
            { value: "3.0", label: "3.0x" },
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
