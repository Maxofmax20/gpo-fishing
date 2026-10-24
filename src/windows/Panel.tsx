import { lazy, Suspense, useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useUpdater } from "../lib/updater";
import { api } from "../lib/ipc";
import { showToast, useStore } from "../lib/store";
import PageBoundary from "../components/PageBoundary";
import Toasts from "../components/Toasts";
import { cx, Dot } from "../components/primitives";
import { ConnectGate } from "../components/ConnectGate";
import logo from "../assets/logo.png";
import Dashboard from "../pages/Dashboard";
const Journal = lazy(() => import("../pages/Journal"));
const Setup = lazy(() => import("../pages/Setup"));
const Features = lazy(() => import("../pages/Features"));
const SettingsPage = lazy(() => import("../pages/Settings"));
const Macros = lazy(() => import("../pages/Macros"));
const VpnPage = lazy(() => import("../pages/VpnPage"));
const MultiRobloxPage = lazy(() => import("../pages/MultiRobloxPage"));
const TrainingCenter = lazy(() => import("../pages/TrainingCenter"));
import { BookOpen, Download, Film, Gauge, Globe, GraduationCap, Layers, ListChecks, Minus, Settings2, Shield, SlidersHorizontal, Wrench, X } from "lucide-react";

type Tab = "dashboard" | "journal" | "setup" | "features" | "macros" | "vpn" | "multi" | "training" | "settings";

const TABS: { id: Tab; label: string; icon: React.ReactNode }[] = [
  { id: "dashboard", label: "Dashboard", icon: <Gauge size={17} /> },
  { id: "journal", label: "Journal", icon: <BookOpen size={17} /> },
  { id: "setup", label: "Setup", icon: <ListChecks size={17} /> },
  { id: "features", label: "Features", icon: <SlidersHorizontal size={17} /> },
  { id: "macros", label: "Macros", icon: <Film size={17} /> },
  { id: "vpn", label: "VPN & Relay", icon: <Shield size={17} /> },
  { id: "multi", label: "Multi Roblox", icon: <Layers size={17} /> },
  { id: "training", label: "Training", icon: <GraduationCap size={17} /> },
  { id: "settings", label: "Settings", icon: <Settings2 size={17} /> },
];

export default function Panel() {
  const init = useStore((s) => s.init);
  const ready = useStore((s) => s.ready);
  const error = useStore((s) => s.error);
  const roblox = useStore((s) => s.roblox);
  const version = useStore((s) => s.version);
  const [tab, setTab] = useState<Tab>("dashboard");
  const [advanced, setAdvanced] = useState(false);
  const [skipGate, setSkipGate] = useState(false);
  useEffect(() => {
    init().finally(() => api.panelShow());
  }, [init]);

  const update = useUpdater((s) => s.available);
  const updating = useUpdater((s) => s.busy);
  const updateMessage = useUpdater((s) => s.message);
  const dismissedVersion = useUpdater((s) => s.dismissedVersion);
  const checkUpdate = useUpdater((s) => s.check);
  const installUpdate = useUpdater((s) => s.install);
  const dismissUpdate = useUpdater((s) => s.dismiss);
  const updateDismissed = update?.version === dismissedVersion;
  const autoUpdate = useStore((s) => s.settings?.auto_update ?? true);
  // Startup check is delayed (never blocks launch) and repeats every 6h.
  // Offline/network failures toast once and never break the bot. Honors the
  // Settings › auto-update toggle (manual Check still works when off).
  useEffect(() => {
    if (!ready || !autoUpdate) return;
    const timer = window.setTimeout(() => { void checkUpdate(true); }, 30_000);
    const interval = window.setInterval(() => { if (!document.hidden) void checkUpdate(true); }, 6 * 3600_000);
    return () => { window.clearTimeout(timer); window.clearInterval(interval); };
  }, [ready, autoUpdate, checkUpdate]);

  useEffect(() => {
    if (roblox) setSkipGate(false);
  }, [roblox]);

  useEffect(() => {
    let t: number | undefined;
    const schedule = async () => {
      window.clearTimeout(t);
      try {
        if (await getCurrentWindow().isMinimized()) return;
      } catch {}
      t = window.setTimeout(async () => {
        try {
          if (await getCurrentWindow().isMinimized()) return;
        } catch {}
        api.panelPlacementChanged();
      }, 400);
    };
    const w = getCurrentWindow();
    const subs = [w.onResized(schedule), w.onMoved(schedule)];
    return () => {
      subs.forEach((p) => p.then((u) => u()));
    };
  }, []);

  const gated = ready && !roblox && !skipGate && !["settings", "journal", "training", "vpn", "multi", "macros"].includes(tab);

  return (
    <div className="h-full w-full bg-bg-glass">
      <div className="h-full w-full flex overflow-hidden">
        <nav className="w-14 shrink-0 flex flex-col items-center py-3 overflow-y-auto border-r border-line bg-black/20">
          <img src={logo} alt="" draggable={false} className="w-9 h-9 rounded-xl mb-4 drag shadow-[0_2px_10px_rgba(0,0,0,0.4)]" />
          {TABS.filter((t) => advanced || !["macros", "vpn", "multi", "training"].includes(t.id)).map((t) => (
            <button
              key={t.id}
              onClick={() => setTab(t.id)}
              title={t.label} aria-label={t.label} aria-current={tab === t.id ? "page" : undefined}
              className={cx(
                "w-10 h-10 shrink-0 rounded-xl grid place-items-center mb-1 transition-colors",
                tab === t.id ? "bg-white/[0.1] text-fg" : "text-fg-mute hover:text-fg hover:bg-white/[0.05]",
              )}
            >
              {t.icon}
            </button>
          ))}
          <button aria-label="Advanced tools" aria-expanded={advanced} title="Advanced tools: recorder, VPN, Multi Roblox and training"
            onClick={() => { if (advanced && ["macros", "vpn", "multi", "training"].includes(tab)) setTab("dashboard"); setAdvanced(!advanced); }}
            className={cx("w-10 h-10 rounded-xl grid place-items-center mt-2 shrink-0", advanced ? "bg-accent-soft text-accent" : "text-fg-mute hover:bg-white/[0.05]")}>
            <Wrench size={17} />
          </button>
          <div className="mt-auto flex flex-col items-center gap-1.5 text-[10px] text-fg-mute">
            <Dot tone={roblox ? (roblox.is_foreground ? "ok" : "accent") : "mute"} />
            <span className="font-mono">{version}</span>
          </div>
        </nav>
        <div className="flex-1 min-w-0 flex flex-col">
          <header className="h-11 flex items-center px-4 border-b border-line drag shrink-0">
            <div className="font-semibold">{gated ? "GPO Autofish" : TABS.find((t) => t.id === tab)?.label}</div>
            <div className="ml-auto flex items-center gap-1 no-drag">
              <button
                className="h-7 px-2.5 mr-1 rounded-lg inline-flex items-center gap-1.5 text-[11px] font-medium bg-white/[0.06] text-fg-dim hover:bg-white/[0.12] hover:text-fg transition-colors"
                onClick={async () => {
                  try {
                    await api.openUrl(await api.webDashboardUrl());
                  } catch (e) {
                    showToast("error", `Could not open Web UI: ${String(e)}`);
                  }
                }}
                title="Open authenticated Web Dashboard (token attached automatically)"
              >
                <Globe size={12} className="text-accent" />
                Web UI
              </button>
              {update && !updateDismissed && (
                <span className="mr-1 inline-flex items-center gap-1">
                  <button
                    className="h-7 max-w-44 truncate px-2.5 rounded-lg inline-flex items-center gap-1.5 text-[11px] font-medium bg-accent-soft text-accent hover:bg-accent/30 disabled:opacity-60"
                    onClick={installUpdate}
                    disabled={!!updating}
                    title={update.body ? `Update ${update.version}\n\n${update.body.slice(0, 400)}` : `Update ${update.version}`}
                  >
                    <Download size={12} />
                    {updating ? updateMessage : `Install ${update.version}`}
                  </button>
                  <button
                    className="h-7 px-2 rounded-lg text-[11px] text-fg-mute hover:text-fg hover:bg-white/[0.06]"
                    onClick={() => api.openUrl("https://github.com/Maxofmax20/gpo-fishing/releases").catch((e) => showToast("error", `Could not open releases: ${String(e)}`))}
                    title="View release notes"
                  >
                    Notes
                  </button>
                  <button
                    className="h-7 w-6 rounded-lg grid place-items-center text-fg-mute hover:text-fg hover:bg-white/[0.06]"
                    onClick={() => dismissUpdate()}
                    title="Later"
                  >
                    <X size={12} />
                  </button>
                </span>
              )}
              <button className="w-8 h-8 rounded-lg grid place-items-center text-fg-mute hover:bg-white/[0.06] hover:text-fg" aria-label="Minimize window" onClick={() => getCurrentWindow().minimize()}>
                <Minus size={15} />
              </button>
              <button className="w-8 h-8 rounded-lg grid place-items-center text-fg-mute hover:bg-bad-soft hover:text-bad" aria-label="Hide window" onClick={() => api.panelHide()}>
                <X size={15} />
              </button>
            </div>
          </header>
          <main className="flex-1 min-h-0 overflow-y-auto relative">
            {!ready && <Booting error={error} />}
            {gated && <ConnectGate onSkip={() => setSkipGate(true)} />}
            {ready && !gated && (
              <PageBoundary key={tab}>
              <Suspense fallback={<div role="status" className="p-4 text-fg-mute">Loading page…</div>}>
                {tab === "dashboard" && <Dashboard onNavigate={(next) => { if (["macros", "vpn", "multi", "training"].includes(next)) setAdvanced(true); setTab(next); }} />}
                {tab === "journal" && <Journal />}
                {tab === "setup" && <Setup />}
                {tab === "features" && <Features />}
                {tab === "macros" && <Macros />}
                {tab === "vpn" && <VpnPage />}
                {tab === "multi" && <MultiRobloxPage />}
                {tab === "training" && <TrainingCenter />}
                {tab === "settings" && <SettingsPage />}
              </Suspense>
              </PageBoundary>
            )}
            <Toasts />
          </main>
        </div>
      </div>
    </div>
  );
}

function Booting({ error }: { error: string | null }) {
  return (
    <div className="h-full flex flex-col items-center justify-center text-center px-8">
      <div className="w-8 h-8 rounded-full border-2 border-white/10 border-t-accent animate-spin mb-4" />
      <div className="text-fg-dim">{error ? "Could not connect to the app" : "Starting…"}</div>
      {error && <>
        <div className="mt-3 text-xs text-warn font-mono break-all select-text max-w-[320px]">{error}</div>
        <button className="mt-4 px-3 py-2 rounded-lg bg-accent-soft text-accent" onClick={() => window.location.reload()}>Reload interface</button>
      </>}
    </div>
  );
}
