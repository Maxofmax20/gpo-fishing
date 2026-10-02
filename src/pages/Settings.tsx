import { useEffect, useState } from "react";
import { check } from "@tauri-apps/plugin-updater";
import { Download, FolderOpen, Globe, RotateCcw, Save, Trash2, Upload } from "lucide-react";
import { api } from "../lib/ipc";
import { showToast, useStore } from "../lib/store";
import { Button, KeyCapture, Pill, Row, Section, Segmented, Slider, Stepper, TextField, Toggle } from "../components/primitives";
import ConfirmModal from "../components/ConfirmModal";

export default function SettingsPage() {
  const s = useStore((st) => st.settings);
  const update = useStore((st) => st.update);
  const setSettings = useStore((st) => st.setSettings);
  const version = useStore((st) => st.version);
  const [open, setOpen] = useState<string | null>(null);
  const [presets, setPresets] = useState<string[]>([]);
  const [presetName, setPresetName] = useState("");
  const [updateMsg, setUpdateMsg] = useState<string | null>(null);
  const [dataDir, setDataDir] = useState("");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [confirmReset, setConfirmReset] = useState(false);
  const [resetting, setResetting] = useState(false);
  const [tokenRevealed, setTokenRevealed] = useState(false);

  useEffect(() => {
    api.presetList().then(setPresets).catch((e) => showToast("warn", `Could not list presets: ${String(e)}`));
    api.dataDir().then(setDataDir).catch(() => undefined);
    api.settingsLoadError().then(setLoadError).catch(() => undefined);
  }, []);

  if (!s) {
    return (
      <div className="pb-4 pt-2">
        <div className="rounded-xl border border-line p-4 text-[12px] text-fg-dim">Loading settings…</div>
      </div>
    );
  }
  const toggle = (k: string) => setOpen((o) => (o === k ? null : k));

  const savePreset = async () => {
    if (!presetName.trim()) return;
    try {
      await api.presetSave(presetName.trim());
      setPresets(await api.presetList());
      setPresetName("");
    } catch (e) {
      showToast("error", `Could not save preset: ${String(e)}`);
    }
  };

  const doReset = async () => {
    setResetting(true);
    try {
      setSettings(await api.settingsReset());
      setConfirmReset(false);
      showToast("info", "Settings reset. A backup of the previous file was kept in the data folder.");
    } catch (e) {
      showToast("error", `Reset failed: ${String(e)}`);
    } finally {
      setResetting(false);
    }
  };

  const checkUpdate = async () => {
    setUpdateMsg("Checking…");
    try {
      const u = await check();
      if (!u) {
        setUpdateMsg("You're on the latest version.");
        return;
      }
      setUpdateMsg(`Downloading ${u.version}…`);
      await u.downloadAndInstall();
      setUpdateMsg("Installing, the app will restart.");
    } catch (e) {
      setUpdateMsg(String(e));
    }
  };

  const importLegacy = async () => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = ".json";
    input.onchange = async () => {
      const f = input.files?.[0];
      if (!f) return;
      try {
        setSettings(await api.legacyImport(await f.text()));
        showToast("info", "Legacy settings imported.");
      } catch (e) {
        showToast("error", `Import failed: ${String(e)}`);
      }
    };
    input.click();
  };

  const openDashboard = async () => {
    try {
      await api.openUrl(await api.webDashboardUrl());
    } catch (e) {
      showToast("error", `Could not open dashboard: ${String(e)}`);
    }
  };

  return (
    <div className="pb-4 pt-2">
      {loadError && (
        <div className="mb-3 rounded-xl border border-bad/50 bg-bad-soft p-3 text-[12px] text-bad">
          <div className="font-semibold mb-1">Settings file was corrupt on load</div>
          <div className="break-words select-text">{loadError}</div>
        </div>
      )}
      <Section title="Tracking">
        <Row title="Tracking" sub="Physics mode models the catch zone's acceleration and input latency. Lookahead is the fallback." open={open === "ctl"} onToggle={() => toggle("ctl")}>
          <Field label="Mode">
            <Segmented
              value={s.fishing.control.mode}
              options={[
                { value: "lookahead", label: "Lookahead" },
                { value: "physics", label: "Physics" },
              ]}
              onChange={(v) => update((x) => void (x.fishing.control.mode = v))}
            />
          </Field>
          {s.fishing.control.mode === "physics" && (
            <div className="text-[11px] text-fg-dim font-mono py-1">
              hold {s.fishing.control.physics.accel_hold.toFixed(2)} · release {s.fishing.control.physics.accel_release.toFixed(2)} · max {s.fishing.control.physics.max_speed.toFixed(2)} · latency {Math.round(s.fishing.control.physics.latency_ms)} ms
              {s.fishing.control.physics.calibrated_at === 0 && <span className="text-warn"> · not calibrated</span>}
            </div>
          )}
          <Field label="Lookahead">
            <Slider value={s.fishing.control.lookahead_ms} min={0} max={400} step={10} format={(v) => `${v} ms`} onChange={(v) => update((x) => void (x.fishing.control.lookahead_ms = v))} />
          </Field>
          <Field label="Dead band">
            <Slider value={s.fishing.control.hysteresis} min={0} max={0.08} step={0.005} format={(v) => v.toFixed(3)} onChange={(v) => update((x) => void (x.fishing.control.hysteresis = v))} />
          </Field>
          <Field label="Velocity smoothing">
            <Slider value={s.fishing.control.velocity_smoothing} min={0} max={0.9} step={0.05} format={(v) => v.toFixed(2)} onChange={(v) => update((x) => void (x.fishing.control.velocity_smoothing = v))} />
          </Field>
          <Field label="Invert hold">
            <Toggle value={s.fishing.control.invert} onChange={(v) => update((x) => void (x.fishing.control.invert = v))} />
          </Field>
          <Field label="Tracking rate">
            <Segmented
              value={String(s.fishing.track_hz)}
              options={[
                { value: "30", label: "30 Hz" },
                { value: "60", label: "60 Hz" },
                { value: "120", label: "120 Hz" },
              ]}
              onChange={(v) => update((x) => void (x.fishing.track_hz = Number(v)))}
            />
          </Field>
        </Row>
        <Row
          title="Record reel logs"
          sub="Every reel writes a small logs/traces/reel-*.json with per-frame numbers and metrics. No screenshots."
          right={<Toggle value={s.fishing.trace} onChange={(v) => update((x) => void (x.fishing.trace = v))} />}
        />
        <Row title="Bite detection" sub="Frames the bar must be seen before reeling starts, and frames it must vanish before the catch counts." open={open === "bite"} onToggle={() => toggle("bite")}>
          <Field label="Confirm frames">
            <Stepper value={s.fishing.bite_confirm_frames} min={1} max={10} onChange={(v) => update((x) => void (x.fishing.bite_confirm_frames = v))} />
          </Field>
          <Field label="Lost frames">
            <Stepper value={s.fishing.lost_frames} min={1} max={20} onChange={(v) => update((x) => void (x.fishing.lost_frames = v))} />
          </Field>
          <Field label="Min reel time">
            <Slider value={s.fishing.min_track_s} min={0} max={5} step={0.1} format={(v) => `${v.toFixed(1)} s`} onChange={(v) => update((x) => void (x.fishing.min_track_s = v))} />
          </Field>
        </Row>
        <Row title="Timeouts" sub="When to give up and recast." open={open === "timeouts"} onToggle={() => toggle("timeouts")}>
          <Field label="Wait for bite">
            <Slider value={s.fishing.scan_timeout_s} min={5} max={60} step={1} format={(v) => `${v} s`} onChange={(v) => update((x) => void (x.fishing.scan_timeout_s = v))} />
          </Field>
          <Field label="Reel timeout">
            <Slider value={s.fishing.track_timeout_s} min={10} max={120} step={5} format={(v) => `${v} s`} onChange={(v) => update((x) => void (x.fishing.track_timeout_s = v))} />
          </Field>
          <Field label="After catch">
            <Slider value={s.fishing.wait_after_catch_s} min={0} max={5} step={0.1} format={(v) => `${v.toFixed(1)} s`} onChange={(v) => update((x) => void (x.fishing.wait_after_catch_s = v))} />
          </Field>
          <Field label="Cast hold">
            <Slider value={s.fishing.cast_hold_ms} min={200} max={3000} step={50} format={(v) => `${v} ms`} onChange={(v) => update((x) => void (x.fishing.cast_hold_ms = v))} />
          </Field>
        </Row>
        <Row title="Bar recognition" sub="Color tolerance and the shape a blue patch must have to count as the fishing bar." open={open === "tol"} onToggle={() => toggle("tol")}>
          <Field label="Color tolerance">
            <Slider value={s.fishing.palette.tolerance} min={0} max={40} step={1} onChange={(v) => update((x) => void (x.fishing.palette.tolerance = v))} />
          </Field>
          <Field label="Min bar height">
            <Slider value={s.fishing.palette.min_bar_height_px} min={10} max={200} step={5} format={(v) => `${v} px`} onChange={(v) => update((x) => void (x.fishing.palette.min_bar_height_px = v))} />
          </Field>
          <Field label="Min tall ratio">
            <Slider value={s.fishing.palette.min_bar_aspect} min={1} max={6} step={0.1} format={(v) => `${v.toFixed(1)}×`} onChange={(v) => update((x) => void (x.fishing.palette.min_bar_aspect = v))} />
          </Field>
          <Field label="Min blue fill">
            <Slider value={s.fishing.palette.min_bar_fill} min={0.1} max={0.9} step={0.05} format={(v) => `${Math.round(v * 100)}%`} onChange={(v) => update((x) => void (x.fishing.palette.min_bar_fill = v))} />
          </Field>
        </Row>
        <Row
          title="Watchdog"
          sub="Restarts the loop if it stops making progress."
          right={<Toggle value={s.watchdog.enabled} onChange={(v) => { update((x) => void (x.watchdog.enabled = v)); if (v) setOpen("wd"); }} />}
          open={open === "wd"}
          onToggle={() => toggle("wd")}
        >
          <Field label="Max restarts">
            <Stepper value={s.watchdog.max_restarts} min={1} max={20} onChange={(v) => update((x) => void (x.watchdog.max_restarts = v))} />
          </Field>
          <Field label="Heartbeat timeout">
            <Slider value={s.watchdog.heartbeat_timeout_s} min={10} max={120} step={5} format={(v) => `${v} s`} onChange={(v) => update((x) => void (x.watchdog.heartbeat_timeout_s = v))} />
          </Field>
        </Row>
      </Section>

      <Section title="Hotkeys">
        <HotkeyRow label="Start / pause" value={s.hotkeys.toggle} onChange={(v) => update((x) => void (x.hotkeys.toggle = v))} />
        <HotkeyRow label="Record macro (start / stop)" value={s.hotkeys.record_toggle || "F7"} onChange={(v) => update((x) => void (x.hotkeys.record_toggle = v))} />
        <HotkeyRow label="Edit areas" value={s.hotkeys.overlay} onChange={(v) => update((x) => void (x.hotkeys.overlay = v))} />
        <HotkeyRow label="Hide HUD" value={s.hotkeys.hide_hud} onChange={(v) => update((x) => void (x.hotkeys.hide_hud = v))} />
        <HotkeyRow label="Quit" value={s.hotkeys.quit} onChange={(v) => update((x) => void (x.hotkeys.quit = v))} />
      </Section>

      <Section title="Presets">
        <Row title="Save current" sub="Snapshot every setting under a name.">
          <div className="flex gap-2">
            <TextField value={presetName} onChange={setPresetName} placeholder="e.g. main account" />
            <Button onClick={savePreset} disabled={!presetName.trim()} icon={<Save size={13} />}>
              Save
            </Button>
          </div>
        </Row>
        {presets.map((p) => (
          <Row
            key={p}
            title={p}
            right={
              <>
                <Button
                  size="sm"
                  onClick={async () => {
                    try {
                      setSettings(await api.presetLoad(p));
                    } catch (e) {
                      showToast("error", `Could not load preset: ${String(e)}`);
                    }
                  }}
                  icon={<Upload size={13} />}
                >
                  Load
                </Button>
                <Button
                  size="sm"
                  kind="ghost"
                  onClick={async () => {
                    try {
                      await api.presetDelete(p);
                      setPresets(await api.presetList());
                    } catch (e) {
                      showToast("error", `Could not delete preset: ${String(e)}`);
                    }
                  }}
                  icon={<Trash2 size={13} />}
                />
              </>
            }
          />
        ))}
        <Row
          title="Import v3 settings"
          sub="Converts an old default_settings.json. Roblox must be open at the same size it was."
          right={
            <Button size="sm" onClick={importLegacy} icon={<FolderOpen size={13} />}>
              Choose file
            </Button>
          }
        />
        <Row
          title="Reset everything"
          sub="Backs up first; you can restore from the data folder."
          right={
            <Button size="sm" kind="danger" onClick={() => setConfirmReset(true)} icon={<RotateCcw size={13} />}>
              Reset
            </Button>
          }
        />
      </Section>

      <ConfirmModal
        open={confirmReset}
        title="Reset all settings?"
        body="Every calibration area, key, purchase point, webhook and AI setting returns to defaults. The current settings.json is backed up first."
        backupNote="Backup is kept as settings.backup-<timestamp>.json in the data folder."
        confirmLabel="Reset everything"
        busy={resetting}
        onConfirm={doReset}
        onCancel={() => !resetting && setConfirmReset(false)}
      />

      <Section title="Web dashboard">
        <Row
          title="Remote dashboard"
          sub="Browser control at 127.0.0.1:3888. Every action needs the per-install token, which the app attaches automatically."
          right={
            <Button size="sm" kind="ghost" onClick={openDashboard} icon={<Globe size={13} />}>
              Open
            </Button>
          }
        />
        <Row
          title="Allow LAN access"
          sub={s.web.allow_lan ? "Exposed to your local network (token still required). Takes effect after restart." : "Loopback only. Enable only on networks you trust; restart required."}
          right={
            <Toggle
              value={s.web.allow_lan}
              onChange={async (v) => {
                try {
                  setSettings(await api.webSetAllowLan(v));
                  if (v) showToast("warn", "LAN access turns on after you restart the app.");
                } catch (e) {
                  showToast("error", `Could not change LAN mode: ${String(e)}`);
                }
              }}
            />
          }
        />
        <Row
          title="Dashboard token"
          sub="Masked by default. Reveal only to type it into a LAN device's browser (?token=), then hide it again. Rotating invalidates saved bookmarks."
          right={
            <Button
              size="sm"
              onClick={async () => {
                try {
                  setSettings(await api.webRegenerateToken());
                  setTokenRevealed(false);
                  showToast("info", "Dashboard token rotated.");
                } catch (e) {
                  showToast("error", `Could not rotate token: ${String(e)}`);
                }
              }}
            >
              Rotate token
            </Button>
          }
        >
          <div className="flex items-center gap-2">
            <span className="font-mono text-[12px] select-text">
              {tokenRevealed ? s.web.token || "(none)" : s.web.token ? `${s.web.token.slice(0, 4)}${"•".repeat(8)}` : "(none)"}
            </span>
            {s.web.token && (
              <>
                <Button size="sm" kind="ghost" onClick={() => setTokenRevealed((v) => !v)}>
                  {tokenRevealed ? "Hide" : "Reveal"}
                </Button>
                <Button
                  size="sm"
                  kind="ghost"
                  onClick={async () => {
                    try {
                      await navigator.clipboard.writeText(`http://127.0.0.1:3888/?token=${s.web.token}`);
                      showToast("info", "Dashboard URL with token copied. It grants full control — share carefully.");
                    } catch (e) {
                      showToast("error", `Copy failed: ${String(e)}`);
                    }
                  }}
                >
                  Copy URL
                </Button>
              </>
            )}
          </div>
        </Row>
      </Section>

      <Section title="App">
        <Row
          title="Updates"
          sub={`Version ${version}`}
          right={
            <>
              <Toggle value={s.auto_update} onChange={(v) => update((x) => void (x.auto_update = v))} />
              <Button size="sm" onClick={checkUpdate} icon={<Download size={13} />}>
                Check
              </Button>
            </>
          }
        >
          {updateMsg && <Pill tone="mute">{updateMsg}</Pill>}
        </Row>
        <Row
          title="Data folder"
          sub={<span className="font-mono text-[11px] select-text">{dataDir}</span>}
          right={
            <Button
              size="sm"
              kind="ghost"
              onClick={() => api.openUrl(dataDir).catch((e) => showToast("error", `Could not open folder: ${String(e)}`))}
              icon={<FolderOpen size={13} />}
            />
          }
        />
        <Row
          title="Discord community"
          right={
            <Button size="sm" kind="ghost" onClick={() => api.openUrl("https://discord.gg/unPZxXAtfb")}>
              Open
            </Button>
          }
        />
      </Section>
    </div>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center min-h-10 py-1">
      <div className="text-fg-dim w-32 shrink-0">{label}</div>
      <div className="ml-auto">{children}</div>
    </div>
  );
}

function HotkeyRow({ label, value, onChange }: { label: string; value: string; onChange: (v: string) => void }) {
  return <Row title={label} right={<KeyCapture value={value} onChange={onChange} />} />;
}
