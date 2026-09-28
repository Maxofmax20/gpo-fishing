import { useEffect, useState } from "react";
import {
  Layers,
  Play,
  RotateCw,
  Trash2,
  ExternalLink,
  ShieldCheck,
  CheckCircle2,
  Crosshair,
  User,
  AlertTriangle,
  Monitor,
} from "lucide-react";
import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import type { MultiRobloxStatus, RobloxInstanceInfo } from "../lib/types";

export default function MultiRobloxPage() {
  const settings = useStore((s) => s.settings);
  const update = useStore((s) => s.update);

  const [status, setStatus] = useState<MultiRobloxStatus | null>(null);
  const [loading, setLoading] = useState(false);
  const [actionMsg, setActionMsg] = useState<string | null>(null);

  const refreshStatus = async () => {
    try {
      const res = await api.multiRobloxGetStatus();
      setStatus(res);
    } catch (e) {
      console.error("Failed to get multi roblox status:", e);
    }
  };

  useEffect(() => {
    refreshStatus();
    const interval = setInterval(refreshStatus, 2500);
    return () => clearInterval(interval);
  }, []);

  const handleToggle = async (enable: boolean) => {
    setLoading(true);
    setActionMsg(null);
    try {
      const res = await api.multiRobloxSetEnabled(enable);
      setStatus(res);
      update((s) => {
        s.features.multi_roblox = enable;
      });
      setActionMsg(
        enable
          ? "Multi-Roblox active! Singleton locks and Error 773 protection claimed."
          : "Multi-Roblox deactivated. Standard Roblox singleton restored."
      );
    } catch (e) {
      setActionMsg(`Error: ${e}`);
    } finally {
      setLoading(false);
    }
  };

  const handleFocus = async (pid: number) => {
    try {
      await api.multiRobloxFocusInstance(pid);
    } catch (e) {
      setActionMsg(`Failed to focus: ${e}`);
    }
  };

  const handleKill = async (pid: number) => {
    try {
      await api.multiRobloxKillInstance(pid);
      setActionMsg(`Instance (PID ${pid}) closed.`);
      await refreshStatus();
    } catch (e) {
      setActionMsg(`Failed to close: ${e}`);
    }
  };

  const handleKillAll = async () => {
    if (!window.confirm("Close ALL running Roblox instances?")) return;
    try {
      const count = await api.multiRobloxKillAll();
      setActionMsg(`Closed ${count} Roblox instances.`);
      await refreshStatus();
    } catch (e) {
      setActionMsg(`Failed to close all: ${e}`);
    }
  };

  const handleSetTarget = async (pid: number) => {
    try {
      const newTarget = status?.target_pid === pid ? null : pid;
      await api.multiRobloxSetTarget(newTarget);
      await refreshStatus();
    } catch (e) {
      setActionMsg(`Failed to set target: ${e}`);
    }
  };

  const handleLaunch = async (placeId?: number) => {
    try {
      await api.multiRobloxLaunch(placeId);
      setActionMsg(placeId ? "Launching Grand Piece Online..." : "Launching Roblox client...");
    } catch (e) {
      setActionMsg(`Launch error: ${e}`);
    }
  };

  const isEnabled = status?.enabled ?? settings?.features.multi_roblox ?? false;

  return (
    <div className="space-y-6 pb-12 pt-2 text-stone-200">
      {/* Top Banner / Master Toggle */}
      <div className="relative overflow-hidden rounded-xl border border-stone-800 bg-stone-900/80 p-5 shadow-lg backdrop-blur">
        <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
          <div className="space-y-1">
            <div className="flex items-center gap-2.5">
              <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-emerald-500/10 text-emerald-400">
                <Layers className="h-4.5 w-4.5" />
              </div>
              <h2 className="text-lg font-semibold tracking-tight text-white">
                Multiple Roblox Instances
              </h2>
              {isEnabled ? (
                <span className="inline-flex items-center gap-1 rounded-full bg-emerald-500/15 px-2.5 py-0.5 text-xs font-medium text-emerald-400">
                  <CheckCircle2 className="h-3 w-3" />
                  Active
                </span>
              ) : (
                <span className="inline-flex items-center gap-1 rounded-full bg-stone-700/50 px-2.5 py-0.5 text-xs font-medium text-stone-400">
                  Disabled
                </span>
              )}
            </div>
            <p className="text-xs text-stone-400">
              Run multiple Roblox accounts simultaneously without crashes or Error 773 teleports.
            </p>
          </div>

          <div className="flex items-center gap-3">
            <button
              onClick={() => handleToggle(!isEnabled)}
              disabled={loading}
              className={`flex items-center gap-2 rounded-lg px-4 py-2 text-sm font-semibold transition-all ${
                isEnabled
                  ? "bg-rose-500/20 text-rose-300 hover:bg-rose-500/30 border border-rose-500/40"
                  : "bg-emerald-600 text-white hover:bg-emerald-500 shadow-sm"
              }`}
            >
              {isEnabled ? "Disable Multi-Roblox" : "Enable Multi-Roblox"}
            </button>
          </div>
        </div>

        {/* Status Indicators */}
        <div className="mt-4 grid grid-cols-1 gap-2.5 border-t border-stone-800/80 pt-4 sm:grid-cols-3">
          <div className="flex items-center gap-2 rounded-lg bg-stone-950/40 px-3 py-2 text-xs">
            <div
              className={`h-2 w-2 rounded-full ${
                status?.mutex_locked ? "bg-emerald-400 animate-pulse" : "bg-stone-600"
              }`}
            />
            <span className="text-stone-400">Singleton Mutex:</span>
            <span className="font-medium text-stone-200">
              {status?.mutex_locked ? "Claimed (Active)" : "Released"}
            </span>
          </div>

          <div className="flex items-center gap-2 rounded-lg bg-stone-950/40 px-3 py-2 text-xs">
            <ShieldCheck
              className={`h-3.5 w-3.5 ${
                status?.cookie_locked ? "text-sky-400" : "text-stone-500"
              }`}
            />
            <span className="text-stone-400">Error 773 Fix:</span>
            <span className="font-medium text-stone-200">
              {status?.cookie_locked ? "Cookies Protected" : "Unlocked"}
            </span>
          </div>

          <div className="flex items-center gap-2 rounded-lg bg-stone-950/40 px-3 py-2 text-xs">
            <Monitor className="h-3.5 w-3.5 text-stone-400" />
            <span className="text-stone-400">Instances:</span>
            <span className="font-medium text-stone-200">
              {status?.instances_count ?? 0} running
            </span>
          </div>
        </div>
      </div>

      {actionMsg && (
        <div className="rounded-lg border border-stone-700/60 bg-stone-800/60 px-4 py-2.5 text-xs text-stone-300">
          {actionMsg}
        </div>
      )}

      {/* Action Tools Bar */}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <button
            onClick={() => handleLaunch(1730877806)}
            className="flex items-center gap-1.5 rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-1.5 text-xs font-medium text-amber-300 hover:bg-amber-500/20 transition"
          >
            <Play className="h-3.5 w-3.5 fill-current" />
            Launch GPO
          </button>
          <button
            onClick={() => handleLaunch()}
            className="flex items-center gap-1.5 rounded-lg border border-stone-700 bg-stone-800/80 px-3 py-1.5 text-xs font-medium text-stone-200 hover:bg-stone-700 transition"
          >
            <ExternalLink className="h-3.5 w-3.5" />
            Launch Roblox Client
          </button>
        </div>

        <div className="flex items-center gap-2">
          <button
            onClick={refreshStatus}
            className="flex items-center gap-1.5 rounded-lg border border-stone-700 bg-stone-800/80 px-2.5 py-1.5 text-xs font-medium text-stone-300 hover:bg-stone-700 transition"
            title="Refresh instances"
          >
            <RotateCw className="h-3.5 w-3.5" />
            Refresh
          </button>
          {(status?.instances_count ?? 0) > 0 && (
            <button
              onClick={handleKillAll}
              className="flex items-center gap-1.5 rounded-lg border border-rose-500/30 bg-rose-500/10 px-2.5 py-1.5 text-xs font-medium text-rose-300 hover:bg-rose-500/20 transition"
            >
              <Trash2 className="h-3.5 w-3.5" />
              Kill All
            </button>
          )}
        </div>
      </div>

      {/* Instances List */}
      <div className="space-y-3">
        <h3 className="text-xs font-semibold uppercase tracking-wider text-stone-400">
          Running Roblox Accounts & Windows ({status?.instances_count ?? 0})
        </h3>

        {!status?.instances || status.instances.length === 0 ? (
          <div className="rounded-xl border border-dashed border-stone-800 bg-stone-900/30 p-8 text-center">
            <Layers className="mx-auto h-8 w-8 text-stone-600 mb-2" />
            <p className="text-sm font-medium text-stone-300">No Roblox windows detected</p>
            <p className="mt-1 text-xs text-stone-500 max-w-md mx-auto">
              Make sure Multi-Roblox is enabled above, then log into your accounts in your browser
              and launch Roblox. They will run side-by-side simultaneously.
            </p>
            <div className="mt-4 flex justify-center gap-2">
              <button
                onClick={() => handleLaunch(1730877806)}
                className="inline-flex items-center gap-1.5 rounded-lg bg-emerald-600 px-3.5 py-1.5 text-xs font-semibold text-white hover:bg-emerald-500"
              >
                <Play className="h-3 w-3 fill-current" />
                Launch First Instance (GPO)
              </button>
            </div>
          </div>
        ) : (
          <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
            {status.instances.map((inst: RobloxInstanceInfo) => (
              <div
                key={inst.pid}
                className={`relative flex flex-col justify-between rounded-xl border p-4 transition-all ${
                  inst.is_target
                    ? "border-emerald-500/60 bg-emerald-950/20 shadow-md shadow-emerald-950/30"
                    : "border-stone-800 bg-stone-900/60 hover:border-stone-700"
                }`}
              >
                <div className="flex items-start gap-3.5">
                  {/* Avatar */}
                  <div className="relative h-12 w-12 shrink-0 overflow-hidden rounded-xl border border-stone-700 bg-stone-800">
                    {inst.avatar_url ? (
                      <img
                        src={inst.avatar_url}
                        alt={inst.username ?? "Avatar"}
                        className="h-full w-full object-cover"
                      />
                    ) : (
                      <div className="flex h-full w-full items-center justify-center text-stone-500">
                        <User className="h-6 w-6" />
                      </div>
                    )}
                  </div>

                  {/* Account / Window Info */}
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="font-semibold text-white text-sm truncate">
                        {inst.display_name || inst.username || `Roblox (PID ${inst.pid})`}
                      </span>
                      {inst.is_target && (
                        <span className="inline-flex items-center gap-1 rounded bg-emerald-500/20 px-1.5 py-0.5 text-[10px] font-semibold text-emerald-400">
                          <Crosshair className="h-2.5 w-2.5" />
                          Macro Target
                        </span>
                      )}
                    </div>

                    {inst.username && (
                      <p className="text-xs text-stone-400 truncate">@{inst.username}</p>
                    )}

                    <p className="mt-1 text-xs text-stone-300 truncate font-medium">
                      {inst.game_name || "Roblox Experience"}
                    </p>

                    <div className="mt-1 flex items-center gap-3 text-[11px] text-stone-500">
                      <span>PID: {inst.pid}</span>
                      {inst.user_id && <span>User ID: {inst.user_id}</span>}
                    </div>
                  </div>
                </div>

                {/* Instance Control Buttons */}
                <div className="mt-4 flex items-center justify-end gap-2 border-t border-stone-800/80 pt-3">
                  <button
                    onClick={() => handleSetTarget(inst.pid)}
                    className={`rounded-lg px-2.5 py-1 text-xs font-medium transition ${
                      inst.is_target
                        ? "bg-emerald-500/20 text-emerald-300 hover:bg-emerald-500/30"
                        : "bg-stone-800 text-stone-300 hover:bg-stone-700"
                    }`}
                  >
                    {inst.is_target ? "Active Target ✓" : "Hook Macro"}
                  </button>

                  <button
                    onClick={() => handleFocus(inst.pid)}
                    className="rounded-lg bg-stone-800 px-2.5 py-1 text-xs font-medium text-stone-300 hover:bg-stone-700 transition"
                  >
                    Focus
                  </button>

                  <button
                    onClick={() => handleKill(inst.pid)}
                    className="rounded-lg bg-rose-500/10 px-2.5 py-1 text-xs font-medium text-rose-300 hover:bg-rose-500/20 transition"
                  >
                    Close
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Guide Card */}
      <div className="rounded-xl border border-stone-800 bg-stone-900/40 p-4 text-xs text-stone-400 space-y-2">
        <div className="flex items-center gap-2 font-medium text-stone-300">
          <AlertTriangle className="h-4 w-4 text-amber-400" />
          How Multiple Accounts Work:
        </div>
        <ol className="list-decimal list-inside space-y-1 text-stone-400 pl-1 leading-relaxed">
          <li>Ensure <strong className="text-stone-200">Multi-Roblox is enabled</strong> before launching your games.</li>
          <li>Log into your first account on your browser and hit <strong>Play</strong>.</li>
          <li>Open an <strong>Incognito window</strong> (or a second browser profile like Chrome Profile 2) and log into your secondary account, then hit <strong>Play</strong>.</li>
          <li>Both accounts will open in separate windows and appear in the list above with player avatar and game info.</li>
          <li>Click <strong className="text-stone-200">Hook Macro</strong> on whichever account you want the fishing bot to automate!</li>
        </ol>
      </div>
    </div>
  );
}
