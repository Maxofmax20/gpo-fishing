import { useRef, useState } from "react";
import { Activity, AlertCircle, Clock, Globe2, RefreshCw, Shield, ShieldCheck, Terminal, Zap } from "lucide-react";
import { api } from "../lib/ipc";
import { useVisiblePoll } from "../lib/useVisiblePoll";
import type { PingResult, VpnEngine, VpnStatus } from "../lib/types";
import { Button, Pill, Row, Section, Toggle, cx } from "../components/primitives";

const initialStatus: VpnStatus = {
  connected: false, state: "disconnected", managed: false, engine: "none", engine_name: "Checking connection",
  ip: "", country: "", city: "", latency_ms: null, uptime_secs: 0, auto_reconnect: true, last_error: null,
  auto_detected: false, process_running: false, tunnel_detected: false, verification_detail: "",
};
const stateLabels: Record<VpnStatus["state"], string> = {
  disconnected: "Disconnected", connecting: "Connecting", verifying: "Verifying connection",
  connected: "Tunnel connected", disconnecting: "Disconnecting", error: "Connection needs attention", unknown: "Connection unverified",
};
const countryName = (country: string) => ({ DE: "Germany", EG: "Egypt" }[country] || country);
const engines: { id: VpnEngine; title: string; description: string }[] = [
  { id: "dedicated", title: "Germany · Frankfurt relay", description: "Recommended for your German exit. Uses TLS/WebSocket and supports TCP and UDP. Install the helper once, then connect from the macro without Administrator mode." },
  { id: "warp", title: "Cloudflare WARP", description: "Alternative Cloudflare tunnel on this PC. WARP chooses its exit automatically; selecting it does not guarantee Germany." },
  { id: "psiphon", title: "Psiphon fallback", description: "Alternative for restrictive networks. Requires Psiphon in C:\\VPN. Its exit country is checked separately." },
  { id: "auto", title: "Use an existing VPN", description: "Detect an active connection. Supported local engines can be adopted for recovery; other VPN apps remain externally managed." },
];

export default function VpnPage() {
  const [status, setStatus] = useState(initialStatus);
  const [selectedEngine, setSelectedEngine] = useState<VpnEngine>("dedicated");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [ping, setPing] = useState<PingResult | null>(null);
  const [pinging, setPinging] = useState(false);
  const [resetting, setResetting] = useState(false);
  const [resetMessage, setResetMessage] = useState<string | null>(null);
  const [showLogs, setShowLogs] = useState(false);
  const [logs, setLogs] = useState<string[]>([]);
  const operation = useRef(0);
  const working = useRef(false);
  const transitioning = ["connecting", "verifying", "disconnecting"].includes(status.state);
  const germanVerified = status.connected && status.state === "connected" && status.germany_verified === true;
  const canManage = ["dedicated", "warp", "psiphon"].includes(status.engine);
  const needsAdmin = status.engine === "dedicated" && !status.administrator && !status.helper_ready;
  const [setupMessage, setSetupMessage] = useState<string | null>(null);

  const fetchStatus = async () => {
    if (working.current) return;
    const generation = operation.current;
    try {
      const next = await api.vpnGetStatus();
      if (generation === operation.current) { setStatus(next); setStatusError(null); }
    } catch (failure) {
      if (generation === operation.current) {
        setStatusError(`VPN status unavailable: ${String(failure)}`);
        setStatus(previous => ({ ...previous, connected: false, state: "unknown", germany_verified: false, warp_verified: false, country: "", ip: "", process_running: false, tunnel_detected: false, verification_detail: "Connection status could not be refreshed." }));
      }
    }
  };
  useVisiblePoll(fetchStatus, 2000);
  useVisiblePoll(async () => {
    try { setLogs(await api.vpnGetLogs(40)); }
    catch (failure) { setError(`VPN logs unavailable: ${String(failure)}`); }
  }, 3000, showLogs);

  const testPing = async () => {
    setPinging(true);
    try { setPing(await api.vpnTestPing()); }
    catch (failure) { setPing({ success: false, latency_ms: 0, target: "92.5.127.89:443", error: String(failure) }); }
    finally { setPinging(false); }
  };
  const run = async (action: "connect" | "disconnect" | "adopt") => {
    operation.current++;
    working.current = true;
    setBusy(true);
    setError(null);
    setPing(null);
    try {
      const next = action === "disconnect" ? await api.vpnDisconnect() : await api.vpnConnect(action === "adopt" ? status.engine : selectedEngine);
      setStatus(next);
      if (next.connected && action !== "disconnect") void testPing();
    } catch (failure) { setError(String(failure)); }
    finally { working.current = false; setBusy(false); await fetchStatus(); }
  };
  const flushDns = async () => {
    setResetting(true);
    try { setResetMessage(await api.vpnResetNetwork()); }
    catch (failure) { setResetMessage(String(failure)); }
    finally { setResetting(false); }
  };
  const toggleRecovery = async (enabled: boolean) => {
    try { await api.vpnSetAutoReconnect(enabled); await fetchStatus(); }
    catch (failure) { setError(`Could not change recovery: ${String(failure)}`); }
  };
  const setupHelper = async () => {
    if (working.current) return;
    working.current = true;
    setBusy(true);
    setError(null);
    try { setSetupMessage(await api.vpnSetupHelper()); }
    catch (failure) { setError(String(failure)); }
    finally { working.current = false; setBusy(false); await fetchStatus(); }
  };

  return <div className="pb-6 pt-2">
    <div className="mx-4 mb-4 rounded-xl border border-line bg-bg-elev p-4">
      <div className="flex items-start gap-3">
        <div className={cx("h-11 w-11 shrink-0 rounded-xl grid place-items-center", germanVerified ? "bg-ok-soft text-ok" : "bg-accent-soft text-accent")}>
          {germanVerified ? <ShieldCheck size={23} /> : <Shield size={23} />}
        </div>
        <div className="min-w-0 flex-1">
          <div className="text-[15px] font-semibold">{germanVerified ? "German exit verified" : stateLabels[status.state]}</div>
          <div className="mt-1 text-[12px] text-fg-dim break-words">{status.engine_name || "Germany · Frankfurt relay"}</div>
          <div className="mt-2 flex flex-wrap gap-1.5">
            <Pill tone={germanVerified ? "ok" : "warn"}>{germanVerified ? "Germany" : "German exit not verified"}</Pill>
            {status.warp_verified && status.connected && <Pill tone="accent">WARP observed</Pill>}
            {status.connected && !status.managed && <Pill tone="mute">Externally managed</Pill>}
          </div>
        </div>
      </div>
      <div className="mt-3 grid grid-cols-2 gap-2 text-[11px] text-fg-dim">
        <span className="flex items-center gap-1.5"><Globe2 size={13} />{status.country ? countryName(status.country) : "Exit check pending"}</span>
        <span className="flex items-center gap-1.5"><Clock size={13} />{status.connected ? `${Math.floor(status.uptime_secs / 60)}m ${status.uptime_secs % 60}s connected` : "No verified uptime"}</span>
        <span>Process: {status.process_running ? "running" : "not detected"}</span>
        <span>Tunnel: {status.tunnel_detected ? "detected" : "not detected"}</span>
      </div>
      <div className="mt-3 flex flex-wrap gap-2 border-t border-line pt-3">
        <Button size="sm" kind="primary" disabled={busy || transitioning || status.connected} icon={<Zap size={14} />} onClick={() => void run("connect")}>
          {busy || transitioning ? "Working…" : "Connect selected route"}
        </Button>
        {(status.connected || status.tunnel_detected || (status.engine === "dedicated" && status.process_running)) && <Button size="sm" kind="danger" disabled={busy || transitioning || !canManage || needsAdmin} onClick={() => void run("disconnect")}>Disconnect</Button>}
        {status.connected && !status.managed && canManage && <Button size="sm" disabled={busy || transitioning} onClick={() => void run("adopt")}>Use existing tunnel</Button>}
        <Button size="sm" kind="ghost" disabled={busy} icon={<RefreshCw size={13} />} onClick={() => void fetchStatus()}>Refresh</Button>
      </div>
      {status.verification_detail && <div className="mt-2 text-[11px] text-fg-mute break-words">{status.verification_detail}</div>}
      {needsAdmin && <p className="mt-2 text-[11px] text-warn">Set up the relay helper below. The macro can stay in normal user mode.</p>}
    </div>

    {(error || statusError || status.last_error) && <div role="alert" className="mx-4 mb-4 flex items-start gap-2 rounded-lg border border-bad/30 bg-bad-soft px-3 py-2 text-[12px] text-bad">
      <AlertCircle size={15} className="mt-0.5 shrink-0" /><span className="break-words min-w-0">{error || statusError || status.last_error}</span>
    </div>}

    <Section title="German relay setup">
      <Row title={status.helper_ready ? "Helper installed" : "One-time Windows setup"}
        sub={setupMessage || "Windows asks once to install fixed connect and disconnect tasks. Later connections and recovery run without elevating the macro. Setup does not connect the VPN."}
        right={<Button size="sm" disabled={busy || transitioning || status.connected || status.tunnel_detected} onClick={() => void setupHelper()}>{status.helper_ready ? "Repair helper" : "Install helper"}</Button>} />
    </Section>
    <Section title="Choose a connection">
      {engines.map(engine => <Row key={engine.id} title={engine.title} sub={engine.description} right={
        <input type="radio" name="vpn-engine" aria-label={engine.title} checked={selectedEngine === engine.id}
          onChange={() => setSelectedEngine(engine.id)} disabled={busy || transitioning || status.connected} className="h-4 w-4 accent-accent" />
      } />)}
    </Section>

    <Section title="Exit & connection health">
      <Row title="Observed public IP" sub={status.exit_error || (status.exit_age_secs != null ? `HTTPS exit check ${status.exit_age_secs}s ago. Old checks expire; country selection alone is not proof.` : "Waiting for a live HTTPS exit check.")}
        right={<span className="max-w-40 font-mono text-[12px] break-all">{status.ip || "Not verified"}</span>} />
      {status.country === "EG" && <Row title="Egyptian exit detected" sub="This exit does not meet your German routing goal. Use the Frankfurt relay and wait for German exit verification." right={<AlertCircle size={17} className="text-warn" />} />}
      <Row title="Relay latency" sub={ping ? (ping.success ? `Median TCP connection time to Frankfurt: ${ping.latency_ms} ms. This is not Roblox game-server ping.` : `Relay unreachable: ${ping.error}`) : "Measure three TCP connections to Frankfurt. Game ping also depends on the Roblox server and your ISP."}
        right={<Button size="sm" disabled={pinging} icon={<Activity size={13} className={cx(pinging && "animate-spin")} />} onClick={() => void testPing()}>{pinging ? "Testing…" : "Test latency"}</Button>} />
      <Row title="Automatic recovery" sub={status.recovery_active ? "Active while this macro stays open. Checks every 5 seconds; failed attempts wait 30 seconds before retrying. Manual disconnect stops recovery." : "Standby. Connect or adopt a supported tunnel to enable recovery. Install the German helper once for recovery in normal user mode."}
        right={<Toggle value={status.auto_reconnect} onChange={(enabled) => void toggleRecovery(enabled)} />} />
      <Row title="Recovery status" sub="Internet outages can interrupt a game session. The recovery switch does not install a Windows startup service or a Roblox firewall block."
        right={<Pill tone={status.recovery_active ? "ok" : "mute"}>{status.recovery_active ? "Monitoring" : "Standby"}</Pill>} />
    </Section>

    <Section title="WARP & German routing">
      <Row title="Keep game traffic on the German relay" sub="The Frankfurt relay carries TCP and UDP. The WARP proxy on the VPS supports TCP only, so it cannot carry UDP game traffic. Ordinary WARP on this PC has no Germany selector." />
    </Section>

    <Section title="Diagnostics">
      <Row title="Flush DNS" sub={resetMessage || "Clear the Windows DNS cache while leaving the active VPN routes in place."}
        right={<Button size="sm" disabled={resetting || busy || transitioning} icon={<RefreshCw size={13} />} onClick={() => void flushDns()}>{resetting ? "Flushing…" : "Flush DNS"}</Button>} />
      <Row title="Relay logs" sub="Read the local relay log. If a connection fails, the full error remains visible above."
        right={<Button size="sm" icon={<Terminal size={13} />} onClick={() => setShowLogs(!showLogs)}>{showLogs ? "Hide logs" : "View logs"}</Button>}>
        {showLogs && <div className="mt-2 max-h-52 overflow-y-auto rounded-lg border border-line bg-black/20 p-2.5 font-mono text-[11px] text-fg-dim">
          {logs.length ? logs.map((line, index) => <div key={index} className="whitespace-pre-wrap break-all">{line}</div>) : "No local relay log entries."}
        </div>}
      </Row>
    </Section>
  </div>;
}
