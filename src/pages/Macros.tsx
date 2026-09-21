import { useEffect, useState } from "react";
import {
  Film,
  Play,
  Square,
  Repeat,
  Trash2,
  MousePointer,
  Keyboard,
  Clock,
  MoveRight,
  CheckCircle2,
  Plus,
} from "lucide-react";
import { api } from "../lib/ipc";
import type { CustomMacro, RecorderStatus } from "../lib/types";
import {
  Button,
  Kbd,
  Pill,
  Row,
  Section,
  Segmented,
  TextField,
} from "../components/primitives";

export default function Macros() {
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
  const [selectedMacroName, setSelectedMacroName] = useState("");
  const [recordName, setRecordName] = useState("Craft Rare Bait");
  const [recordMode, setRecordMode] = useState<"pc" | "web">("pc");
  const [speed, setSpeed] = useState("1.0");
  const [openSection, setOpenSection] = useState<string | null>("steps");
  const [busy, setBusy] = useState(false);

  const toggle = (k: string) => setOpenSection((o) => (o === k ? null : k));

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
    const interval = setInterval(
      refresh,
      status.is_recording || status.is_playing ? 700 : 2000,
    );
    return () => clearInterval(interval);
  }, [status.is_recording, status.is_playing]);

  const activeMacro =
    macros.find((m) => m.name === selectedMacroName || m.id === selectedMacroName) ||
    macros[0];

  const handleStartRecord = async () => {
    setBusy(true);
    try {
      const name = recordName.trim() || `Macro #${macros.length + 1}`;
      await api.macroRecord("start", name, recordMode);
      await refresh();
    } catch (e) {
      console.error(e);
    } finally {
      setBusy(false);
    }
  };

  const handleStopRecord = async () => {
    setBusy(true);
    try {
      const name = recordName.trim() || `Macro #${macros.length + 1}`;
      const res = await api.macroRecord("stop", name);
      if (res.macro) {
        setSelectedMacroName(res.macro.name);
      }
      await refresh();
    } catch (e) {
      console.error(e);
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
      console.error(e);
    } finally {
      setBusy(false);
    }
  };

  const handlePlay = async (loop: boolean) => {
    if (!activeMacro) return;
    setBusy(true);
    try {
      const sp = parseFloat(speed) || 1.0;
      await api.macroPlay(loop ? "loop" : "play", activeMacro.name, loop, sp);
      await refresh();
    } catch (e) {
      console.error(e);
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
      console.error(e);
    } finally {
      setBusy(false);
    }
  };

  const handleDelete = async () => {
    if (!activeMacro) return;
    setBusy(true);
    try {
      await api.macroDelete(activeMacro.id || activeMacro.name);
      setSelectedMacroName("");
      await refresh();
    } catch (e) {
      console.error(e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="pb-6 pt-2">
      {/* 1. PLAYBACK & ACTIVE STATUS */}
      <Section
        title="Execution & Controls"
        action={
          status.is_recording ? (
            <Pill tone="bad">🔴 Recording in Progress</Pill>
          ) : status.is_playing ? (
            <Pill tone="fruit">
              ▶️ {status.is_looping ? `Looping (#${status.current_loop})` : "Playing"} ({speed}x)
            </Pill>
          ) : (
            <Pill tone="mute">⏹️ Ready</Pill>
          )
        }
      >
        <Row
          title={
            status.is_recording ? (
              <span className="text-bad font-medium">
                Recording Roblox Actions ({status.recorded_steps_count} steps captured)
              </span>
            ) : status.is_playing ? (
              <span className="text-accent font-medium">
                Active: &ldquo;{status.playing_macro_name}&rdquo;
              </span>
            ) : activeMacro ? (
              <span>Selected Macro: &ldquo;{activeMacro.name}&rdquo;</span>
            ) : (
              <span className="text-fg-dim">No Macros Saved Yet</span>
            )
          }
          sub={
            status.is_recording ? (
              <>Press <Kbd>F8</Kbd> inside Roblox at any time to instantly save.</>
            ) : status.is_playing ? (
              status.message || "Executing sequence with authentic timings and key holds."
            ) : activeMacro ? (
              `${activeMacro.steps.length} steps · Recorded ${activeMacro.created_at || "recently"}`
            ) : (
              "Record your mouse clicks, drag & drops, and movement holds below."
            )
          }
          right={
            status.is_recording ? (
              <div className="flex items-center gap-1.5">
                <Button
                  size="sm"
                  kind="primary"
                  disabled={busy}
                  onClick={handleStopRecord}
                  icon={<CheckCircle2 size={13} />}
                >
                  Save (F8)
                </Button>
                <Button
                  size="sm"
                  kind="danger"
                  disabled={busy}
                  onClick={handleCancelRecord}
                >
                  Cancel
                </Button>
              </div>
            ) : status.is_playing ? (
              <Button
                size="sm"
                kind="danger"
                disabled={busy}
                onClick={handleStopPlayback}
                icon={<Square size={13} />}
              >
                Stop
              </Button>
            ) : activeMacro ? (
              <div className="flex items-center gap-1.5">
                <Button
                  size="sm"
                  kind="primary"
                  disabled={busy}
                  onClick={() => handlePlay(false)}
                  icon={<Play size={13} />}
                >
                  Play ({speed}x)
                </Button>
                <Button
                  size="sm"
                  kind="default"
                  disabled={busy}
                  onClick={() => handlePlay(true)}
                  icon={<Repeat size={13} />}
                >
                  Loop
                </Button>
              </div>
            ) : undefined
          }
        />

        {macros.length > 0 && !status.is_recording && (
          <>
            <Row
              title="Saved Macros"
              sub="Choose which recorded workflow to inspect or replay."
              right={
                <div className="flex items-center gap-2">
                  <select
                    value={activeMacro?.id || activeMacro?.name || ""}
                    onChange={(e) => setSelectedMacroName(e.target.value)}
                    disabled={status.is_playing || busy}
                    className="h-8 px-3 rounded-lg bg-white/[0.06] border border-line-strong text-[12px] text-fg outline-none focus:border-accent"
                  >
                    {macros.map((m) => (
                      <option key={m.id || m.name} value={m.id || m.name} className="bg-bg-elev text-fg">
                        {m.name} ({m.steps.length} steps)
                      </option>
                    ))}
                  </select>
                  <Button
                    size="sm"
                    kind="ghost"
                    disabled={status.is_playing || busy || !activeMacro}
                    onClick={handleDelete}
                    icon={<Trash2 size={13} />}
                  >
                    Delete
                  </Button>
                </div>
              }
            />

            <Row
              title="Playback Speed"
              sub="Scale all delays, movement hold durations, and drag timings."
              right={
                <Segmented
                  value={speed}
                  options={[
                    { value: "0.75", label: "0.75x" },
                    { value: "1.0", label: "1.0x" },
                    { value: "1.25", label: "1.25x" },
                    { value: "1.5", label: "1.5x" },
                    { value: "2.0", label: "2.0x" },
                    { value: "3.0", label: "3.0x" },
                  ]}
                  onChange={setSpeed}
                />
              }
            />

            {activeMacro && activeMacro.steps.length > 0 && (
              <Row
                title={
                  <span className="inline-flex items-center gap-2">
                    <Film size={14} className="text-accent" />
                    Step Inspector ({activeMacro.steps.length} steps)
                  </span>
                }
                sub="Inspect exact click coordinates, drag & drops, key hold durations, and pauses."
                open={openSection === "steps"}
                onToggle={() => toggle("steps")}
              >
                <div className="max-h-[260px] overflow-y-auto pr-1 flex flex-col gap-1.5 text-[11px] font-mono bg-black/40 p-3 rounded-xl border border-line">
                  {activeMacro.steps.map((step, idx) => (
                    <div
                      key={idx}
                      className="flex items-center justify-between py-1.5 px-2.5 rounded-lg hover:bg-white/[0.04] border border-white/[0.03] transition-colors"
                    >
                      <div className="flex items-center gap-2.5 text-fg">
                        <span className="text-fg-mute font-semibold tabular-nums w-5 text-right">
                          #{idx + 1}
                        </span>

                        {step.type === "Drag" ? (
                          <span className="flex items-center gap-1.5 text-emerald-300">
                            <MousePointer size={12} />
                            <span>
                              DRAG ({(step.start_rx * 100).toFixed(1)}%, {(step.start_ry * 100).toFixed(1)}%)
                            </span>
                            <MoveRight size={11} className="text-fg-mute" />
                            <span>
                              ({(step.end_rx * 100).toFixed(1)}%, {(step.end_ry * 100).toFixed(1)}%)
                            </span>
                            <span className="text-emerald-400 font-medium">
                              [{step.duration_ms}ms]
                            </span>
                          </span>
                        ) : step.type === "Click" ? (
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
              </Row>
            )}
          </>
        )}
      </Section>

      {/* 2. RECORD NEW MACRO */}
      <Section title="Action Recorder">
        <Row
          title="Recording Source"
          sub="PC Game captures directly from the live Roblox window. Web Remote captures from mobile/browser stream."
          right={
            <Segmented
              value={recordMode}
              options={[
                { value: "pc", label: "🖥️ PC Game Window" },
                { value: "web", label: "🌐 Web Remote" },
              ]}
              onChange={(v) => setRecordMode(v as "pc" | "web")}
            />
          }
        />

        <Row
          title="New Recording"
          sub={
            <>
              Captures mouse clicks, drag & drop, and movement keys (<Kbd>W</Kbd><Kbd>A</Kbd><Kbd>S</Kbd><Kbd>D</Kbd><Kbd>Space</Kbd><Kbd>Shift</Kbd><Kbd>E</Kbd><Kbd>T</Kbd><Kbd>1-5</Kbd>).
            </>
          }
          right={
            <div className="flex items-center gap-2">
              <div className="w-44">
                <TextField
                  value={recordName}
                  onChange={setRecordName}
                  placeholder="Macro name..."
                />
              </div>
              <Button
                size="sm"
                kind="primary"
                disabled={busy || status.is_recording || status.is_playing}
                onClick={handleStartRecord}
                icon={<Plus size={13} />}
              >
                Record
              </Button>
            </div>
          }
        />
      </Section>
    </div>
  );
}
