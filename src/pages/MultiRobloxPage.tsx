import { useState } from "react";
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
  Plus,
  HelpCircle,
  KeyRound,
  X,
  Sparkles,
  Rocket,
  Monitor,
} from "lucide-react";
import { api } from "../lib/ipc";
import { showToast, useStore } from "../lib/store";
import { useVisiblePoll } from "../lib/useVisiblePoll";
import ConfirmModal from "../components/ConfirmModal";
import type { MultiRobloxStatus, RobloxInstanceInfo, SavedRobloxAccount } from "../lib/types";

export default function MultiRobloxPage() {
  const settings = useStore((s) => s.settings);
  const update = useStore((s) => s.update);

  const [status, setStatus] = useState<MultiRobloxStatus | null>(null);
  const [accounts, setAccounts] = useState<SavedRobloxAccount[]>([]);
  const [loading, setLoading] = useState(false);
  const [actionMsg, setActionMsg] = useState<{ type: "info" | "error" | "success"; text: string } | null>(null);

  // Add Account form state
  const [showAddModal, setShowAddModal] = useState(false);
  const [newCookie, setNewCookie] = useState("");
  const [newNote, setNewNote] = useState("");
  const [addingAccount, setAddingAccount] = useState(false);
  const [showCookieHelp, setShowCookieHelp] = useState(false);
  const [confirmKillAll, setConfirmKillAll] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState<{ id: string; name: string } | null>(null);
  const [busyConfirm, setBusyConfirm] = useState(false);

  const gpoPlaceId = settings?.game.gpo_place_id ?? 1730877806;

  const refreshAll = async () => {
    try {
      const [statusRes, accountsRes] = await Promise.all([
        api.multiRobloxGetStatus(),
        api.multiRobloxListAccounts(),
      ]);
      setStatus(statusRes);
      setAccounts(accountsRes);
    } catch (e) {
      showToast("warn", `Multi-Roblox refresh failed: ${String(e)}`);
    }
  };

  useVisiblePoll(refreshAll, 2500);

  const handleToggle = async (enable: boolean) => {
    setLoading(true);
    setActionMsg(null);
    try {
      const res = await api.multiRobloxSetEnabled(enable);
      setStatus(res);
      update((s) => {
        s.features.multi_roblox = enable;
      });
      setActionMsg({
        type: "success",
        text: enable
          ? "Multi-Roblox active! Singleton locks and Error 773 protection claimed."
          : "Multi-Roblox deactivated. Standard Roblox singleton restored.",
      });
    } catch (e) {
      setActionMsg({ type: "error", text: `Error: ${e}` });
    } finally {
      setLoading(false);
    }
  };

  const handleFocus = async (pid: number) => {
    try {
      await api.multiRobloxFocusInstance(pid);
    } catch (e) {
      setActionMsg({ type: "error", text: `Failed to focus: ${e}` });
    }
  };

  const handleKill = async (pid: number) => {
    try {
      await api.multiRobloxKillInstance(pid);
      setActionMsg({ type: "info", text: `Instance (PID ${pid}) closed.` });
      await refreshAll();
    } catch (e) {
      setActionMsg({ type: "error", text: `Failed to close: ${e}` });
    }
  };

  const handleKillAll = async () => {
    setBusyConfirm(true);
    try {
      const count = await api.multiRobloxKillAll();
      setActionMsg({ type: "info", text: `Closed ${count} Roblox instances.` });
      setConfirmKillAll(false);
      await refreshAll();
    } catch (e) {
      setActionMsg({ type: "error", text: `Failed to close all: ${e}` });
    } finally {
      setBusyConfirm(false);
    }
  };

  const handleSetTarget = async (pid: number) => {
    try {
      const newTarget = status?.target_pid === pid ? null : pid;
      await api.multiRobloxSetTarget(newTarget);
      await refreshAll();
    } catch (e) {
      setActionMsg({ type: "error", text: `Failed to set target: ${e}` });
    }
  };

  const handleAddAccount = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!newCookie.trim()) {
      setActionMsg({ type: "error", text: "Please paste your .ROBLOSECURITY cookie" });
      return;
    }
    setAddingAccount(true);
    setActionMsg(null);
    try {
      const acc = await api.multiRobloxAddAccount(newCookie, newNote.trim() || undefined);
      setActionMsg({
        type: "success",
        text: `Account added: @${acc.username} (${acc.display_name})!`,
      });
      setNewCookie("");
      setNewNote("");
      setShowAddModal(false);
      await refreshAll();
    } catch (e) {
      setActionMsg({ type: "error", text: `Failed to add account: ${e}` });
    } finally {
      setAddingAccount(false);
    }
  };

  const handleRemoveAccount = async () => {
    if (!confirmRemove) return;
    const { id, name } = confirmRemove;
    setBusyConfirm(true);
    try {
      await api.multiRobloxRemoveAccount(id);
      setActionMsg({ type: "info", text: `Account @${name} removed.` });
      setConfirmRemove(null);
      await refreshAll();
    } catch (e) {
      setActionMsg({ type: "error", text: `Failed to remove: ${e}` });
    } finally {
      setBusyConfirm(false);
    }
  };

  const handleLaunchAccount = async (id: string, name: string, placeId?: number) => {
    setActionMsg({
      type: "info",
      text: `Authenticating & launching @${name} into ${placeId ? "Grand Piece Online" : "Roblox"}...`,
    });
    try {
      await api.multiRobloxLaunchAccount(id, placeId);
      setActionMsg({
        type: "success",
        text: `🚀 @${name} launched successfully! Window will open in a moment.`,
      });
      setTimeout(refreshAll, 3000);
    } catch (e) {
      setActionMsg({ type: "error", text: `Launch error: ${e}` });
    }
  };

  const handleLaunchAllAccounts = async () => {
    if (accounts.length === 0) return;
    setActionMsg({ type: "info", text: `Launching all ${accounts.length} accounts into GPO...` });
    for (let i = 0; i < accounts.length; i++) {
      const acc = accounts[i];
      try {
        await api.multiRobloxLaunchAccount(acc.id, gpoPlaceId);
        // Stagger launches by 3 seconds so Roblox doesn't contend for process start
        if (i < accounts.length - 1) {
          await new Promise((r) => setTimeout(r, 3000));
        }
      } catch (err) {
        setActionMsg({ type: "error", text: `Failed to launch @${acc.username}: ${err}` });
      }
    }
    setActionMsg({ type: "success", text: "All accounts dispatched to launch!" });
    setTimeout(refreshAll, 3000);
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
                Multiple Roblox Instances & Account Launcher
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
              Run and launch multiple Roblox accounts directly with 1-click. Bypass singleton mutex & protect against Error 773.
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
              {status?.instances_count ?? 0} running ({accounts.length} saved)
            </span>
          </div>
        </div>
      </div>

      {actionMsg && (
        <div
          className={`rounded-lg border px-4 py-2.5 text-xs ${
            actionMsg.type === "error"
              ? "border-rose-500/40 bg-rose-950/30 text-rose-300"
              : actionMsg.type === "success"
              ? "border-emerald-500/40 bg-emerald-950/30 text-emerald-300"
              : "border-stone-700/60 bg-stone-800/60 text-stone-300"
          }`}
        >
          {actionMsg.text}
        </div>
      )}

      {/* SECTION 1: SAVED ACCOUNTS & DIRECT LAUNCHER */}
      <div className="space-y-3">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="space-y-0.5">
            <div className="flex items-center gap-2">
              <KeyRound className="h-4 w-4 text-emerald-400" />
              <h3 className="text-sm font-semibold text-white">Saved Accounts ({accounts.length})</h3>
            </div>
            <p className="text-xs text-stone-400">
              Launch directly into GPO with your saved accounts in 1-click.
            </p>
          </div>

          <div className="flex items-center gap-2">
            {accounts.length > 1 && (
              <button
                onClick={handleLaunchAllAccounts}
                className="flex items-center gap-1.5 rounded-lg border border-emerald-500/40 bg-emerald-500/10 px-3 py-1.5 text-xs font-semibold text-emerald-300 hover:bg-emerald-500/20 transition"
              >
                <Rocket className="h-3.5 w-3.5" />
                Launch All Accounts
              </button>
            )}

            <button
              onClick={() => setShowAddModal(!showAddModal)}
              className="flex items-center gap-1.5 rounded-lg bg-emerald-600 px-3 py-1.5 text-xs font-semibold text-white hover:bg-emerald-500 transition shadow-sm"
            >
              <Plus className="h-3.5 w-3.5" />
              Add Account
            </button>
          </div>
        </div>

        {/* Add Account Card / Modal */}
        {showAddModal && (
          <form
            onSubmit={handleAddAccount}
            className="rounded-xl border border-emerald-500/30 bg-stone-900/90 p-4 shadow-xl space-y-4"
          >
            <div className="flex items-center justify-between border-b border-stone-800 pb-2.5">
              <div className="flex items-center gap-2">
                <Sparkles className="h-4 w-4 text-emerald-400" />
                <span className="font-semibold text-sm text-white">Add Roblox Account</span>
              </div>
              <button
                type="button"
                onClick={() => setShowAddModal(false)}
                className="text-stone-400 hover:text-white"
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            <div className="space-y-3">
              <div>
                <div className="flex items-center justify-between mb-1">
                  <label className="text-xs font-medium text-stone-300">
                    .ROBLOSECURITY Cookie
                  </label>
                  <button
                    type="button"
                    onClick={() => setShowCookieHelp(!showCookieHelp)}
                    className="flex items-center gap-1 text-[11px] text-emerald-400 hover:underline"
                  >
                    <HelpCircle className="h-3 w-3" />
                    How to get cookie?
                  </button>
                </div>
                <input
                  type="password"
                  value={newCookie}
                  onChange={(e) => setNewCookie(e.target.value)}
                  placeholder="_|WARNING:-DO-NOT-SHARE-THIS.--Sharing-this-will-allow-someone-to-log-in..."
                  className="w-full rounded-lg border border-stone-700 bg-stone-950 px-3 py-2 text-xs font-mono text-white placeholder-stone-600 focus:border-emerald-500 focus:outline-none"
                  autoComplete="off"
                />
              </div>

              {showCookieHelp && (
                <div className="rounded-lg border border-stone-800 bg-stone-950/80 p-3 text-[11px] text-stone-400 space-y-1.5 leading-relaxed">
                  <p className="font-medium text-stone-200">How to copy your cookie in 15 seconds:</p>
                  <ol className="list-decimal list-inside space-y-0.5 pl-1 text-stone-400">
                    <li>Open <strong>roblox.com</strong> in your browser (Chrome/Edge/Brave) and log into your account.</li>
                    <li>Press <kbd className="rounded bg-stone-800 px-1 py-0.5 font-mono text-[10px] text-stone-200">F12</kbd> (or right click -&gt; Inspect).</li>
                    <li>Go to the <strong className="text-stone-200">Application</strong> (or <strong className="text-stone-200">Storage</strong>) tab at the top.</li>
                    <li>In the left sidebar, click <strong className="text-stone-200">Cookies</strong> -&gt; <strong className="text-stone-200">https://www.roblox.com</strong>.</li>
                    <li>Find the cookie named <code className="rounded bg-stone-800 px-1 py-0.5 font-mono text-[10px] text-emerald-300">.ROBLOSECURITY</code>, double-click its value, copy it, and paste it above!</li>
                  </ol>
                  <p className="text-[10px] text-stone-500 pt-1">
                    * The cookie is stored locally only on your PC in <code className="text-stone-400">accounts.json</code> and used exclusively to generate client launch tickets.
                  </p>
                </div>
              )}

              <div>
                <label className="text-xs font-medium text-stone-300 block mb-1">
                  Nickname / Note (optional)
                </label>
                <input
                  type="text"
                  value={newNote}
                  onChange={(e) => setNewNote(e.target.value)}
                  placeholder="e.g. Main Fisher, Fruit Alt 1"
                  className="w-full rounded-lg border border-stone-700 bg-stone-950 px-3 py-2 text-xs text-white placeholder-stone-600 focus:border-emerald-500 focus:outline-none"
                />
              </div>
            </div>

            <div className="flex items-center justify-end gap-2 pt-1 border-t border-stone-800/80">
              <button
                type="button"
                onClick={() => setShowAddModal(false)}
                className="rounded-lg bg-stone-800 px-3 py-1.5 text-xs font-medium text-stone-300 hover:bg-stone-700"
              >
                Cancel
              </button>
              <button
                type="submit"
                disabled={addingAccount}
                className="rounded-lg bg-emerald-600 px-4 py-1.5 text-xs font-semibold text-white hover:bg-emerald-500 transition disabled:opacity-50"
              >
                {addingAccount ? "Validating..." : "Save Account"}
              </button>
            </div>
          </form>
        )}

        {/* Saved Accounts Cards */}
        {accounts.length === 0 ? (
          <div className="rounded-xl border border-dashed border-stone-800 bg-stone-900/30 p-6 text-center">
            <KeyRound className="mx-auto h-7 w-7 text-stone-600 mb-2" />
            <p className="text-sm font-medium text-stone-300">No accounts saved yet</p>
            <p className="mt-1 text-xs text-stone-500 max-w-sm mx-auto">
              Add your accounts once with their security cookie to launch them directly from the macro without signing in and out in browsers!
            </p>
            <button
              onClick={() => setShowAddModal(true)}
              className="mt-3 inline-flex items-center gap-1.5 rounded-lg bg-emerald-600 px-3 py-1.5 text-xs font-semibold text-white hover:bg-emerald-500"
            >
              <Plus className="h-3 w-3" />
              Add Your First Account
            </button>
          </div>
        ) : (
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 md:grid-cols-3">
            {accounts.map((acc: SavedRobloxAccount) => (
              <div
                key={acc.id}
                className={`relative flex flex-col justify-between rounded-xl border p-4 transition-all ${
                  acc.is_running
                    ? "border-emerald-500/50 bg-emerald-950/20 shadow-md"
                    : "border-stone-800 bg-stone-900/70 hover:border-stone-700"
                }`}
              >
                <div className="flex items-start gap-3">
                  <div className="relative h-12 w-12 shrink-0 overflow-hidden rounded-xl border border-stone-700 bg-stone-800">
                    {acc.avatar_url ? (
                      <img
                        src={acc.avatar_url}
                        alt={acc.username}
                        className="h-full w-full object-cover"
                      />
                    ) : (
                      <div className="flex h-full w-full items-center justify-center text-stone-500">
                        <User className="h-6 w-6" />
                      </div>
                    )}
                  </div>

                  <div className="min-w-0 flex-1">
                    <div className="flex items-center justify-between gap-1">
                      <span className="font-semibold text-white text-sm truncate">
                        {acc.display_name}
                      </span>
                      <button
                        onClick={() => setConfirmRemove({ id: acc.id, name: acc.username })}
                        title="Delete account"
                        className="text-stone-500 hover:text-rose-400 p-0.5 transition"
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </button>
                    </div>

                    <p className="text-xs text-stone-400 truncate">@{acc.username}</p>

                    {acc.note && (
                      <span className="inline-block mt-1 rounded bg-stone-800 px-1.5 py-0.5 text-[10px] font-medium text-stone-300 truncate max-w-full">
                        {acc.note}
                      </span>
                    )}

                    <div className="mt-2 flex items-center gap-1.5 text-[11px]">
                      {acc.is_running ? (
                        <span className="inline-flex items-center gap-1 text-emerald-400 font-medium">
                          <span className="h-1.5 w-1.5 rounded-full bg-emerald-400 animate-ping" />
                          Running (PID {acc.running_pid})
                        </span>
                      ) : (
                        <span className="text-stone-500">Offline</span>
                      )}
                    </div>
                  </div>
                </div>

                <div className="mt-3 flex items-center gap-2 border-t border-stone-800/80 pt-2.5">
                  <button
                    onClick={() => handleLaunchAccount(acc.id, acc.username, gpoPlaceId)}
                    className="flex-1 flex items-center justify-center gap-1.5 rounded-lg bg-emerald-600/90 hover:bg-emerald-500 px-2.5 py-1.5 text-xs font-semibold text-white transition shadow-sm"
                  >
                    <Play className="h-3 w-3 fill-current" />
                    Launch GPO
                  </button>

                  <button
                    onClick={() => handleLaunchAccount(acc.id, acc.username)}
                    title="Launch Roblox Player"
                    className="rounded-lg border border-stone-700 bg-stone-800 px-2 py-1.5 text-xs text-stone-300 hover:bg-stone-700 transition"
                  >
                    <ExternalLink className="h-3.5 w-3.5" />
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* SECTION 2: RUNNING INSTANCES & ACTIVE WINDOWS */}
      <div className="space-y-3 pt-4 border-t border-stone-800/80">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="space-y-0.5">
            <h3 className="text-sm font-semibold text-white">
              Running Roblox Windows & Macro Targets ({status?.instances_count ?? 0})
            </h3>
            <p className="text-xs text-stone-400">
              Active Roblox game processes currently detected on your system.
            </p>
          </div>

          <div className="flex items-center gap-2">
            <button
              onClick={refreshAll}
              className="flex items-center gap-1.5 rounded-lg border border-stone-700 bg-stone-800/80 px-2.5 py-1.5 text-xs font-medium text-stone-300 hover:bg-stone-700 transition"
              title="Refresh instances"
            >
              <RotateCw className="h-3.5 w-3.5" />
              Refresh
            </button>
            {(status?.instances_count ?? 0) > 0 && (
              <button
                onClick={() => setConfirmKillAll(true)}
                className="flex items-center gap-1.5 rounded-lg border border-rose-500/30 bg-rose-500/10 px-2.5 py-1.5 text-xs font-medium text-rose-300 hover:bg-rose-500/20 transition"
              >
                <Trash2 className="h-3.5 w-3.5" />
                Kill All
              </button>
            )}
          </div>
        </div>

        {!status?.instances || status.instances.length === 0 ? (
          <div className="rounded-xl border border-dashed border-stone-800 bg-stone-900/30 p-6 text-center">
            <Layers className="mx-auto h-7 w-7 text-stone-600 mb-2" />
            <p className="text-sm font-medium text-stone-300">No active Roblox windows open</p>
            <p className="mt-1 text-xs text-stone-500 max-w-sm mx-auto">
              Click &quot;Launch GPO&quot; on any saved account above or launch through your browser to begin.
            </p>
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
                      {inst.game_name || "Grand Piece Online"}
                    </p>

                    <div className="mt-1 flex items-center gap-3 text-[11px] text-stone-500">
                      <span>PID: {inst.pid}</span>
                      {inst.user_id && <span>User ID: {inst.user_id}</span>}
                    </div>
                  </div>
                </div>

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
      <ConfirmModal
        open={confirmKillAll}
        title="Close all Roblox instances?"
        body="Every running Roblox window will be terminated, including any active fishing session."
        confirmLabel="Close all"
        busy={busyConfirm}
        onConfirm={handleKillAll}
        onCancel={() => !busyConfirm && setConfirmKillAll(false)}
      />
      <ConfirmModal
        open={confirmRemove !== null}
        title={`Remove account @${confirmRemove?.name ?? ""}?`}
        body="The saved cookie for this account is deleted from this PC. You can re-add it later by pasting the cookie again."
        confirmLabel="Remove account"
        busy={busyConfirm}
        onConfirm={handleRemoveAccount}
        onCancel={() => !busyConfirm && setConfirmRemove(null)}
      />
    </div>
  );
}
