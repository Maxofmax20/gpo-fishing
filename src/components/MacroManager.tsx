import { useEffect, useState } from "react";
import {
  Film,
  Play,
  Square,
  Repeat,
  Trash2,
  ChevronDown,
  ChevronUp,
  MousePointer,
  Keyboard,
  Clock,
  Sparkles,
  Layers,
} from "lucide-react";
import { api } from "../lib/ipc";
import type { CustomMacro, RecorderStatus } from "../lib/types";
import { Button, Pill } from "./primitives";

export function MacroManager() {
  const [macros, setMacros] = useState<CustomMacro[]>([]);
  const [status, setStatus] = useState<RecorderStatus>({
    is_recording: false,
    record_mode: null,
    recorded_steps_count: 0,
    is_playing: false,
    playing_macro_name: null,
    current_loop: 0,
    is_looping: false,
    message: "",
  });
  const [macroName, setMacroName] = useState("Craft Rare Bait");
  const [selectedMacroName, setSelectedMacroName] = useState("");
  const [recordMode, setRecordMode] = useState<"pc" | "web">("pc");
  const [showSteps, setShowSteps] = useState(false);
  const [busy, setBusy] = useState(false);

  const refresh = async () => {
    try {
      const [list, st] = await Promise.all([api.macroList(), api.macroStatus()]);
      setMacros(list);
      setStatus(st);
      if (!selectedMacroName && list.length > 0) {
        setSelectedMacroName(list[0].name);
      }
    } catch (e) {
      console.warn("Failed to poll macros:", e);
    }
  };

  useEffect(() => {
    refresh();
    const interval = setInterval(refresh, status.is_recording || status.is_playing ? 800 : 2000);
    return () => clearInterval(interval);
  }, [status.is_recording, status.is_playing]);

  const activeMacro = macros.find((m) => m.name === selectedMacroName || m.id === selectedMacroName) || macros[0];

  const handleStartRecord = async () => {
    setBusy(true);
    try {
      await api.macroRecord("start", macroName.trim() || "My Macro", recordMode);
      await refresh();
    } catch (e) {
      alert("Failed to start recording: " + e);
    } finally {
      setBusy(false);
    }
  };

  const handleStopRecord = async () => {
    setBusy(true);
    try {
      const res = await api.macroRecord("stop", macroName.trim() || "My Macro");
      if (res.macro) {
        setSelectedMacroName(res.macro.name);
      }
      await refresh();
    } catch (e) {
      alert("Failed to save macro: " + e);
    } finally {
      setBusy(false);
    }
  };

  const handleCancelRecord = async () => {
    setBusy(true);
    try {
      await api.macroRecord("cancel");
      await refresh();
    } catch (e) {
      alert("Failed to cancel: " + e);
    } finally {
      setBusy(false);
    }
  };

  const handlePlay = async (loop: boolean) => {
    if (!activeMacro) {
      alert("Please record or select a macro first!");
      return;
    }
    setBusy(true);
    try {
      await api.macroPlay(loop ? "loop" : "play", activeMacro.name, loop);
      await refresh();
    } catch (e) {
      alert("Failed to start playback: " + e);
    } finally {
      setBusy(false);
    }
  };

  const handleStopPlayback = async () => {
    setBusy(true);
    try {
      await api.macroPlay("stop");
      await refresh();
    } catch (e) {
      alert("Failed to stop: " + e);
    } finally {
      setBusy(false);
    }
  };

  const handleDelete = async () => {
    if (!activeMacro) return;
    if (!confirm(`Delete macro "${activeMacro.name}"?`)) return;
    setBusy(true);
    try {
      await api.macroDelete(activeMacro.name);
      setSelectedMacroName("");
      await refresh();
    } catch (e) {
      alert("Failed to delete: " + e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="bg-white/[0.03] border border-line rounded-xl p-4 flex flex-col gap-4 text-fg">
      {/* Header & Badges */}
      <div className="flex items-center justify-between flex-wrap gap-2">
        <div className="flex items-center gap-2">
          <div className="w-8 h-8 rounded-lg bg-cyan-500/10 text-cyan-400 flex items-center justify-center">
            <Film size={18} />
          </div>
          <div>
            <div className="text-[14px] font-semibold flex items-center gap-2">
              <span>Step Recorder &amp; Macro Player</span>
              <span className="text-[10px] px-1.5 py-0.5 rounded bg-cyan-500/20 text-cyan-300 font-mono">
                v4.2.29
              </span>
            </div>
            <div className="text-[11px] text-fg-dim">
              Record real PC clicks &amp; WASD movements directly in Roblox, then replay or loop!
            </div>
          </div>
        </div>

        <div>
          {status.is_recording ? (
            <Pill tone="warn">
              🔴 RECORDING ({status.recorded_steps_count} steps)
            </Pill>
          ) : status.is_playing ? (
            <Pill tone="fruit">
              ▶️ {status.is_looping ? `LOOPING (#${status.current_loop})` : "PLAYING"}
            </Pill>
          ) : (
            <Pill tone="mute">⏹️ IDLE</Pill>
          )}
        </div>
      </div>

      {/* 1. RECORD SECTION */}
      <div className="bg-black/30 border border-line/60 rounded-lg p-3 flex flex-col gap-2.5">
        <div className="flex items-center justify-between text-[12px] font-semibold text-fg-dim">
          <span className="flex items-center gap-1.5">
            <span>1. RECORD NEW WORKFLOW</span>
          </span>
          <span className="font-mono text-[11px] text-accent">
            {status.is_recording ? `${status.recorded_steps_count} STEPS CAPTURED` : "READY TO RECORD"}
          </span>
        </div>

        <div className="flex flex-wrap gap-2 items-center">
          <input
            type="text"
            className="flex-1 min-w-[170px] bg-black/40 border border-line rounded-lg px-3 py-1.5 text-[13px] outline-none focus:border-cyan-500 transition-colors"
            placeholder="Macro Name (e.g. Craft Rare Bait)"
            value={macroName}
            disabled={status.is_recording}
            onChange={(e) => setMacroName(e.target.value)}
          />

          <select
            className="bg-black/40 border border-line rounded-lg px-2.5 py-1.5 text-[12px] font-medium outline-none cursor-pointer"
            value={recordMode}
            disabled={status.is_recording}
            onChange={(e) => setRecordMode(e.target.value as "pc" | "web")}
            title="Recording input source"
          >
            <option value="pc">🖥️ PC Game (Clicks &amp; Moves)</option>
            <option value="web">🌐 Web Screen Taps</option>
          </select>

          {status.is_recording ? (
            <>
              <Button
                kind="danger"
                disabled={busy}
                onClick={handleStopRecord}
                icon={<Square size={13} />}
              >
                Finish &amp; Save (or F8)
              </Button>
              <Button
                kind="default"
                disabled={busy}
                onClick={handleCancelRecord}
              >
                Cancel
              </Button>
            </>
          ) : (
            <Button
              kind="primary"
              disabled={busy || status.is_playing}
              onClick={handleStartRecord}
              icon={<Sparkles size={13} />}
            >
              ⏺️ Record Actions
            </Button>
          )}
        </div>

        <div className="text-[11px] text-fg-mute flex items-center gap-1">
          {status.is_recording ? (
            <span className="text-warn animate-pulse font-medium">
              👉 Click into Roblox now! Every mouse click and held move (W, A, S, D, E, T, Space) is being recorded. Press <b>F8</b> anytime to save!
            </span>
          ) : (
            <span>
              💡 <b>Tip:</b> Click <b>Record</b>, focus Roblox, walk and click your crafting or merchant steps, then press <b>F8</b> or click Finish.
            </span>
          )}
        </div>
      </div>

      {/* 2. PLAYBACK SECTION */}
      <div className="bg-black/30 border border-line/60 rounded-lg p-3 flex flex-col gap-2.5">
        <div className="flex items-center justify-between text-[12px] font-semibold text-fg-dim">
          <span>2. PLAY OR LOOP SAVED MACRO</span>
          <span className="font-mono text-[11px] text-cyan-400">
            {macros.length} SAVED
          </span>
        </div>

        <div className="flex flex-wrap gap-2 items-center">
          <select
            className="flex-1 min-w-[200px] bg-black/40 border border-line rounded-lg px-3 py-1.5 text-[13px] font-medium outline-none cursor-pointer"
            value={activeMacro?.name ?? ""}
            disabled={status.is_playing || status.is_recording}
            onChange={(e) => setSelectedMacroName(e.target.value)}
          >
            {macros.length === 0 ? (
              <option value="">(No macros recorded yet)</option>
            ) : (
              macros.map((m) => (
                <option key={m.id} value={m.name}>
                  📋 {m.name} ({m.steps.length} steps)
                </option>
              ))
            )}
          </select>

          {status.is_playing ? (
            <Button
              kind="danger"
              disabled={busy}
              onClick={handleStopPlayback}
              icon={<Square size={13} />}
            >
              Stop Playback
            </Button>
          ) : (
            <>
              <Button
                kind="default"
                disabled={busy || !activeMacro || status.is_recording}
                onClick={() => handlePlay(false)}
                icon={<Play size={13} />}
              >
                Play Once
              </Button>
              <Button
                kind="primary"
                disabled={busy || !activeMacro || status.is_recording}
                onClick={() => handlePlay(true)}
                icon={<Repeat size={13} />}
              >
                Loop Play
              </Button>
            </>
          )}

          <span title="Delete selected macro">
            <Button
              kind="ghost"
              disabled={busy || !activeMacro || status.is_playing || status.is_recording}
              onClick={handleDelete}
              icon={<Trash2 size={13} />}
            />
          </span>
        </div>

        {/* Step Inspector Toggle */}
        {activeMacro && activeMacro.steps.length > 0 && (
          <div className="mt-1 pt-2 border-t border-line/40 flex flex-col gap-2">
            <button
              className="flex items-center justify-between text-[11px] text-fg-dim hover:text-fg transition-colors w-full cursor-pointer select-none"
              onClick={() => setShowSteps((s) => !s)}
            >
              <span className="flex items-center gap-1.5 font-medium">
                <Layers size={13} />
                <span>Inspect Steps for &ldquo;{activeMacro.name}&rdquo; ({activeMacro.steps.length} steps)</span>
              </span>
              {showSteps ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
            </button>

            {showSteps && (
              <div className="max-h-[180px] overflow-y-auto pr-1 flex flex-col gap-1 text-[11px] font-mono bg-black/50 p-2.5 rounded border border-line/40">
                {activeMacro.steps.map((step, idx) => (
                  <div
                    key={idx}
                    className="flex items-center justify-between py-1 px-2 rounded hover:bg-white/[0.04] border-b border-white/[0.02]"
                  >
                    <div className="flex items-center gap-2 text-fg">
                      <span className="text-fg-mute font-bold">#{idx + 1}</span>
                      {step.type === "Click" ? (
                        <span className="flex items-center gap-1.5 text-cyan-300">
                          <MousePointer size={12} />
                          <span>
                            {step.button.toUpperCase()} Click at ({(step.rx * 100).toFixed(1)}%, {(step.ry * 100).toFixed(1)}%)
                          </span>
                        </span>
                      ) : step.type === "KeyHold" ? (
                        <span className="flex items-center gap-1.5 text-amber-300">
                          <Keyboard size={12} />
                          <span>
                            HOLD [{step.key.toUpperCase()}] for {step.duration_ms}ms
                          </span>
                        </span>
                      ) : step.type === "KeyTap" ? (
                        <span className="flex items-center gap-1.5 text-purple-300">
                          <Keyboard size={12} />
                          <span>TAP [{step.key.toUpperCase()}]</span>
                        </span>
                      ) : step.type === "MouseMove" ? (
                        <span className="flex items-center gap-1.5 text-blue-300">
                          <MousePointer size={12} />
                          <span>Move to ({(step.rx * 100).toFixed(1)}%, {(step.ry * 100).toFixed(1)}%)</span>
                        </span>
                      ) : (
                        <span className="text-fg-dim">SLEEP {(step as any).ms}ms</span>
                      )}
                    </div>
                    {"delay_ms" in step && (
                      <span className="text-fg-mute text-[10px] flex items-center gap-1">
                        <Clock size={10} />
                        <span>+{step.delay_ms}ms</span>
                      </span>
                    )}
                  </div>
                ))}
              </div>
            )}
          </div>
        )}
      </div>

      {status.message && (
        <div className="text-[11px] text-fg-dim italic px-1">
          {status.message}
        </div>
      )}
    </div>
  );
}
