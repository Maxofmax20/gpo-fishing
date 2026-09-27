import { useEffect, useState, useRef } from "react";
import {
  Activity,
  AlertCircle,
  Clock,
  RefreshCw,
  Shield,
  ShieldCheck,
  Terminal,
  Zap,
} from "lucide-react";
import { api } from "../lib/ipc";
import type { PingResult, VpnEngine, VpnStatus } from "../lib/types";
import { Button, Pill, Row, Section, Toggle, cx } from "../components/primitives";

export default function VpnPage() {
  const [status, setStatus] = useState<VpnStatus>({
    connected: false,
    engine: "dedicated",
    ip: "Checking...",
    country: "",
    city: "",
    latency_ms: null,
    uptime_secs: 0,
    auto_reconnect: true,
    last_error: null,
  });

  const [selectedEngine, setSelectedEngine] = useState<VpnEngine>("dedicated");
  const [busy, setBusy] = useState(false);
  const [pingResult, setPingResult] = useState<PingResult | null>(null);
  const [pinging, setPinging] = useState(false);
  const [resetting, setResetting] = useState(false);
  const [resetMsg, setResetMsg] = useState<string | null>(null);
  const [showLogs, setShowLogs] = useState(false);
  const [logs, setLogs] = useState<string[]>([]);
  const logsEndRef = useRef<HTMLDivElement>(null);

  const fetchStatus = async () => {
    try {
      const s = await api.vpnGetStatus();
      setStatus(s);
      if (s.engine && s.engine !== "none") {
        setSelectedEngine(s.engine);
      }
    } catch {
      // Ignore initial IPC blips
    }
  };

  useEffect(() => {
    fetchStatus();
    const interval = setInterval(fetchStatus, 3000);
    return () => clearInterval(interval);
  }, []);

  useEffect(() => {
    if (!showLogs) return;
    const fetchLogs = () => {
      api.vpnGetLogs(40).then((l) => {
        setLogs(l);
        logsEndRef.current?.scrollIntoView({ behavior: "smooth" });
      });
    };
    fetchLogs();
    const t = setInterval(fetchLogs, 2500);
    return () => clearInterval(t);
  }, [showLogs]);

  const handleToggleConnect = async () => {
    setBusy(true);
    try {
      if (status.connected) {
        const next = await api.vpnDisconnect();
        setStatus(next);
      } else {
        const next = await api.vpnConnect(selectedEngine);
        setStatus(next);
        // Automatically probe latency after connecting
        setTimeout(handleTestPing, 1500);
      }
    } catch (e) {
      setStatus((prev) => ({ ...prev, last_error: String(e) }));
    } finally {
      setBusy(false);
      fetchStatus();
    }
  };

  const handleTestPing = async () => {
    setPinging(true);
    try {
      const res = await api.vpnTestPing();
      setPingResult(res);
      if (res.success) {
        setStatus((prev) => ({ ...prev, latency_ms: res.latency_ms }));
      }
    } catch (e) {
      setPingResult({
        success: false,
        latency_ms: 0,
        target: "92.5.127.89:443",
        error: String(e),
      });
    } finally {
      setPinging(false);
    }
  };

  const handleResetNetwork = async () => {
    setResetting(true);
    setResetMsg("Flushing DNS & routes...");
    try {
      const res = await api.vpnResetNetwork();
      setResetMsg(res);
      setTimeout(() => setResetMsg(null), 3000);
    } catch (e) {
      setResetMsg(String(e));
    } finally {
      setResetting(false);
    }
  };

  const handleToggleAutoReconnect = (val: boolean) => {
    setStatus((prev) => ({ ...prev, auto_reconnect: val }));
    api.vpnSetAutoReconnect(val);
  };

  const formatUptime = (secs: number) => {
    if (!secs) return "0s";
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    const s = secs % 60;
    if (h > 0) return `${h}h ${m}m ${s}s`;
    if (m > 0) return `${m}m ${s}s`;
    return `${s}s`;
  };

  return (
    <div className="pb-6 pt-2">
      {/* Hero Banner / Status */}
      <div className="px-4 py-3 mx-4 mb-4 rounded-xl bg-gradient-to-r from-bg-elev to-bg border border-line flex items-center justify-between shadow-lg">
        <div className="flex items-center gap-3">
          <div
            className={cx(
              "w-12 h-12 rounded-xl grid place-items-center transition-colors shadow-[0_0_15px_rgba(0,0,0,0.5)]",
              status.connected
                ? "bg-ok-soft text-ok border border-ok/30"
                : "bg-white/[0.05] text-fg-dim border border-line-strong",
            )}
          >
            {status.connected ? <ShieldCheck size={24} /> : <Shield size={24} />}
          </div>
          <div>
            <div className="flex items-center gap-2">
              <span className="font-semibold text-[15px]">
                {status.connected ? "Relay Active" : "Disconnected"}
              </span>
              <Pill tone={status.connected ? "ok" : "mute"}>
                {status.connected
                  ? status.engine === "dedicated"
                    ? "Frankfurt Dedicated"
                    : status.engine.toUpperCase()
                  : "Offline"}
              </Pill>
            </div>
            <div className="text-[12px] text-fg-dim mt-0.5 flex items-center gap-2 font-mono">
              <Clock size={12} className="text-fg-mute" />
              <span>Uptime: {formatUptime(status.uptime_secs)}</span>
              {status.latency_ms != null && (
                <>
                  <span className="text-line-strong">|</span>
                  <Zap size={12} className={status.latency_ms < 100 ? "text-ok" : "text-warn"} />
                  <span>{status.latency_ms} ms</span>
                </>
              )}
            </div>
          </div>
        </div>

        <Button
          kind={status.connected ? "danger" : "primary"}
          size="md"
          disabled={busy}
          onClick={handleToggleConnect}
          icon={<RefreshCw size={14} className={cx(busy && "animate-spin")} />}
        >
          {status.connected ? "Disconnect" : "Connect Now"}
        </Button>
      </div>

      {/* Error notification if any */}
      {status.last_error && (
        <div className="mx-4 mb-4 px-3 py-2 rounded-lg bg-bad-soft border border-bad/30 flex items-center gap-2 text-[12px] text-bad">
          <AlertCircle size={15} className="shrink-0" />
          <span className="truncate">{status.last_error}</span>
        </div>
      )}

      {/* Network Engine Selection */}
      <Section title="Tunnel Routing Engine">
        <Row
          title="German Dedicated Relay (Recommended)"
          sub="Private VLESS-WS-TLS over Port 443 to Frankfurt (92.5.127.89). Zero throttling, low latency, full Roblox UDP pass-through."
          right={
            <input
              type="radio"
              name="vpn_engine"
              checked={selectedEngine === "dedicated"}
              onChange={() => setSelectedEngine("dedicated")}
              disabled={status.connected}
              className="accent-accent w-4 h-4 cursor-pointer"
            />
          }
        />
        <Row
          title="Cloudflare WARP (Edge Fallback)"
          sub="Official Cloudflare WARP client tunnel for resilient routing when dedicated relay is in maintenance."
          right={
            <input
              type="radio"
              name="vpn_engine"
              checked={selectedEngine === "warp"}
              onChange={() => setSelectedEngine("warp")}
              disabled={status.connected}
              className="accent-accent w-4 h-4 cursor-pointer"
            />
          }
        />
        <Row
          title="Psiphon Tunnel (Anti-Censorship Fallback)"
          sub="Multi-hop fallback for severe ISP throttling or restrictive ISP firewalls."
          right={
            <input
              type="radio"
              name="vpn_engine"
              checked={selectedEngine === "psiphon"}
              onChange={() => setSelectedEngine("psiphon")}
              disabled={status.connected}
              className="accent-accent w-4 h-4 cursor-pointer"
            />
          }
        />
      </Section>

      {/* Telemetry & Live Diagnostics */}
      <Section title="Live Telemetry">
        <Row
          title="Public IP & Geolocation"
          sub={status.country ? `${status.city ? status.city + ", " : ""}${status.country}` : "Global Network"}
          right={
            <span className="font-mono text-[13px] font-medium text-fg">
              {status.ip || "Detecting..."}
            </span>
          }
        />
        <Row
          title="Frankfurt Relay Latency (TCP 443)"
          sub={pingResult ? (pingResult.success ? `Reachable in ${pingResult.latency_ms} ms` : `Unreachable: ${pingResult.error}`) : "Measure ping to 92.5.127.89:443"}
          right={
            <div className="flex items-center gap-2">
              {status.latency_ms != null && (
                <span className="font-mono text-[12px] font-medium text-fg-dim">
                  {status.latency_ms} ms
                </span>
              )}
              <Button
                kind="default"
                size="sm"
                disabled={pinging}
                onClick={handleTestPing}
                icon={<Activity size={13} className={cx(pinging && "animate-spin")} />}
              >
                {pinging ? "Testing…" : "Test Ping"}
              </Button>
            </div>
          }
        />
        <Row
          title="Auto-Reconnect Watchdog"
          sub="Background daemon monitors connection health every 10s and instantly revives tunnel if interrupted."
          right={
            <Toggle
              value={status.auto_reconnect}
              onChange={handleToggleAutoReconnect}
            />
          }
        />
      </Section>

      {/* Network Tools & Diagnostics */}
      <Section title="Quick Actions & Tools">
        <Row
          title="Flush DNS & Refresh Network Routes"
          sub={resetMsg ?? "Clears local Windows DNS resolver cache and cleans up stale network routing tables."}
          right={
            <Button
              kind="default"
              size="sm"
              disabled={resetting}
              onClick={handleResetNetwork}
              icon={<RefreshCw size={13} className={cx(resetting && "animate-spin")} />}
            >
              {resetting ? "Refreshing…" : "Flush DNS"}
            </Button>
          }
        />
        <Row
          title="Tunnel Diagnostic Logs"
          sub="Inspect silent sing-box background engine logs, TLS handshake status, and TUN interface."
          right={
            <Button
              kind="default"
              size="sm"
              onClick={() => setShowLogs(!showLogs)}
              icon={<Terminal size={13} />}
            >
              {showLogs ? "Hide Logs" : "View Logs"}
            </Button>
          }
        >
          {showLogs && (
            <div className="mt-2 p-2.5 rounded-lg bg-black/50 border border-line-strong font-mono text-[11px] text-fg-dim max-h-56 overflow-y-auto leading-relaxed">
              {logs.length === 0 ? (
                <div className="text-fg-mute py-4 text-center">No tunnel log entries yet. Connect to start logging.</div>
              ) : (
                logs.map((line, i) => (
                  <div key={i} className="hover:text-fg transition-colors whitespace-pre-wrap break-all">
                    {line}
                  </div>
                ))
              )}
              <div ref={logsEndRef} />
            </div>
          )}
        </Row>
      </Section>
    </div>
  );
}
