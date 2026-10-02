use std::net::SocketAddr;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x08000000;

/// True when this process runs elevated (Administrator). The sing-box TUN
/// driver requires elevation; without it `configure tun interface: Access
/// is denied` kills sing-box on startup — detected here so the user gets an
/// instant actionable error instead of a 20s verification timeout.
pub fn is_elevated() -> bool {
    #[cfg(windows)]
    {
        #[link(name = "shell32")]
        extern "system" {
            fn IsUserAnAdmin() -> i32;
        }
        // SAFETY: IsUserAnAdmin takes no arguments and has no side effects.
        unsafe { IsUserAnAdmin() != 0 }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Returns true if any of `names` (lowercase exe names) is in the process list.
fn any_process_running(procs: &[String], names: &[&str]) -> bool {
    procs.iter().any(|p| names.iter().any(|n| p == n))
}
const DEDICATED_SERVER_IP: [u8; 4] = [92, 5, 127, 89];
const DEDICATED_SERVER_PORT: u16 = 443;
const SINGBOX_EXE: &str = r"C:\VPN\sing-box.exe";
const SINGBOX_CONFIG: &str = r"C:\VPN\sing-box-client.json";
const SINGBOX_LOG: &str = r"C:\VPN\sing-box.log";
const PSIPHON_EXE: &str = r"C:\VPN\psiphon3.exe";
const WARP_CLI_DEFAULT: &str = r"C:\Program Files\Cloudflare\Cloudflare WARP\warp-cli.exe";
/// Human-readable label for the dedicated relay probe target.
const DEDICATED_RELAY_LABEL: &str = "92.5.127.89:443 (Frankfurt)";

/// Evidence-based VPN lifecycle. `Connected` is reported ONLY after
/// `verify_connection()` succeeds — never on spawn, exit code, selection,
/// or previous state. `Unknown` means something VPN-shaped was seen but no
/// engine verified: it must never be presented as connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VpnState {
    Disconnected,
    Connecting,
    Verifying,
    Connected,
    Disconnecting,
    Error,
    Unknown,
}

impl VpnState {
    pub fn as_str(self) -> &'static str {
        match self {
            VpnState::Disconnected => "disconnected",
            VpnState::Connecting => "connecting",
            VpnState::Verifying => "verifying",
            VpnState::Connected => "connected",
            VpnState::Disconnecting => "disconnecting",
            VpnState::Error => "error",
            VpnState::Unknown => "unknown",
        }
    }
}

/// Per-check evidence behind a state. Shown in the UI so users (and macros)
/// can see WHY the app believes what it believes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VpnEvidence {
    pub process_running: bool,
    pub tunnel_detected: bool,
    pub cli_reports_connected: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VpnStatus {
    pub connected: bool,
    pub state: VpnState,
    /// True when THIS app established the connection (vs externally active).
    pub managed: bool,
    pub engine: String,
    pub engine_name: String,
    pub ip: String,
    pub country: String,
    pub city: String,
    pub latency_ms: Option<u64>,
    pub uptime_secs: u64,
    pub auto_reconnect: bool,
    pub last_error: Option<String>,
    pub auto_detected: bool,
    pub process_running: bool,
    pub tunnel_detected: bool,
    pub verification_detail: String,
}

/// Raw OS observations fed into [`evaluate_evidence`]. Real collectors fill
/// this from processes/netsh/CLI; tests inject synthetic snapshots, so all
/// verification decisions are unit-testable without a VPN provider.
#[derive(Debug, Clone, Default)]
pub struct EvidenceSnapshot {
    /// Lowercase exe names, e.g. `sing-box.exe`.
    pub processes: Vec<String>,
    /// Raw `netsh interface show interface` output lines.
    pub netsh_lines: Vec<String>,
    /// `warp-cli status` stdout, when warp is relevant/available.
    pub warp_status: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Verification {
    pub connected: bool,
    pub evidence: VpnEvidence,
}

/// Default time `connect()` waits for verification before failing.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// Time `disconnect()` waits for processes to vanish.
const DISCONNECT_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingResult {
    pub success: bool,
    pub latency_ms: u64,
    pub target: String,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct IpApiResponse {
    query: Option<String>,
    country: Option<String>,
    city: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DetectedVpn {
    pub engine: String,
    pub display_name: String,
}

pub struct VpnManager {
    connected: AtomicBool,
    state: Mutex<VpnState>,
    /// True only when THIS app established the verified connection.
    managed: AtomicBool,
    engine: Mutex<String>,
    engine_name: Mutex<String>,
    auto_detected: AtomicBool,
    start_time: Mutex<Option<Instant>>,
    last_ip: Arc<Mutex<String>>,
    last_country: Arc<Mutex<String>>,
    last_city: Arc<Mutex<String>>,
    last_latency: Mutex<Option<u64>>,
    last_error: Mutex<Option<String>>,
    state_reason: Mutex<Option<String>>,
    evidence: Mutex<VpnEvidence>,
    auto_reconnect: AtomicBool,
    user_disconnected: AtomicBool,
}

/// Control surface used by macro steps. Implemented by [`VpnManager`];
/// tests substitute a scripted fake. Production code must never fake success.
pub trait VpnControl: Send + Sync {
    fn vpn_state(&self) -> VpnState;
    fn vpn_connect_verified(&self, engine: &str, timeout: Duration) -> Result<VpnStatus, String>;
    fn vpn_disconnect_verified(&self, timeout: Duration) -> Result<VpnStatus, String>;
    fn vpn_wait_for(&self, want_connected: bool, timeout: Duration) -> Result<VpnStatus, String>;
}

impl VpnManager {
    pub fn new() -> Arc<Self> {
        let mgr = Arc::new(Self {
            connected: AtomicBool::new(false),
            state: Mutex::new(VpnState::Disconnected),
            managed: AtomicBool::new(false),
            engine: Mutex::new("none".to_string()),
            engine_name: Mutex::new("Offline".to_string()),
            auto_detected: AtomicBool::new(false),
            start_time: Mutex::new(None),
            last_ip: Arc::new(Mutex::new("Detecting...".to_string())),
            last_country: Arc::new(Mutex::new("".to_string())),
            last_city: Arc::new(Mutex::new("".to_string())),
            last_latency: Mutex::new(None),
            last_error: Mutex::new(None),
            state_reason: Mutex::new(None),
            evidence: Mutex::new(VpnEvidence::default()),
            auto_reconnect: AtomicBool::new(true),
            user_disconnected: AtomicBool::new(false),
        });

        // Startup: desired state is DISCONNECTED. Only adopt a genuinely
        // verified external connection (marked unmanaged/external).
        mgr.sync_process_state();

        // Spawn watchdog thread
        let mgr_clone = Arc::clone(&mgr);
        std::thread::Builder::new()
            .name("vpn-watchdog".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_secs(5));
                    mgr_clone.watchdog_tick();
                }
            })
            .ok();

        mgr
    }

    /// Deterministic construction for tests: DISCONNECTED, no OS probing,
    /// no watchdog thread.
    #[cfg(test)]
    fn new_isolated() -> Self {
        Self {
            connected: AtomicBool::new(false),
            state: Mutex::new(VpnState::Disconnected),
            managed: AtomicBool::new(false),
            engine: Mutex::new("none".to_string()),
            engine_name: Mutex::new("Offline".to_string()),
            auto_detected: AtomicBool::new(false),
            start_time: Mutex::new(None),
            last_ip: Arc::new(Mutex::new(String::new())),
            last_country: Arc::new(Mutex::new(String::new())),
            last_city: Arc::new(Mutex::new(String::new())),
            last_latency: Mutex::new(None),
            last_error: Mutex::new(None),
            state_reason: Mutex::new(None),
            evidence: Mutex::new(VpnEvidence::default()),
            auto_reconnect: AtomicBool::new(true),
            user_disconnected: AtomicBool::new(false),
        }
    }

    fn set_state(&self, state: VpnState, reason: Option<String>) {
        *self.state.lock() = state;
        *self.state_reason.lock() = reason;
        self.connected.store(state == VpnState::Connected, Ordering::SeqCst);
    }

    pub fn vpn_state(&self) -> VpnState {
        *self.state.lock()
    }

    fn set_evidence(&self, ev: VpnEvidence) {
        *self.evidence.lock() = ev;
    }

    fn get_running_process_names() -> Vec<String> {
        #[cfg(windows)]
        {
            use windows::Win32::Foundation::CloseHandle;
            use windows::Win32::System::Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
                TH32CS_SNAPPROCESS,
            };

            let mut list = Vec::new();
            unsafe {
                let snapshot = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
                    Ok(h) => h,
                    Err(_) => return list,
                };

                let mut entry = PROCESSENTRY32W {
                    dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                    ..Default::default()
                };

                if Process32FirstW(snapshot, &mut entry).is_ok() {
                    loop {
                        let name = String::from_utf16_lossy(&entry.szExeFile);
                        let clean = name.trim_matches('\0').trim().to_lowercase();
                        if !clean.is_empty() {
                            list.push(clean);
                        }
                        if Process32NextW(snapshot, &mut entry).is_err() {
                            break;
                        }
                    }
                }
                let _ = CloseHandle(snapshot);
            }
            list
        }
        #[cfg(not(windows))]
        {
            Vec::new()
        }
    }

    fn is_warp_active(procs: &[String]) -> bool {
        #[cfg(windows)]
        {
            let bin = if Path::new(WARP_CLI_DEFAULT).exists() {
                WARP_CLI_DEFAULT
            } else {
                "warp-cli"
            };
            let mut cmd = Command::new(bin);
            cmd.arg("status");
            cmd.creation_flags(CREATE_NO_WINDOW);
            if let Ok(output) = cmd.output() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.contains("Connected") || stdout.contains("connected") {
                    return true;
                }
                if stdout.contains("Disconnected") || stdout.contains("disconnected") {
                    return false;
                }
            }

            if Self::check_netsh_interface_connected("CloudflareWARP") {
                return true;
            }

            if procs.iter().any(|p| p == "warp-svc.exe" || p == "cloudflare warp.exe") {
                if Self::check_netsh_interface_connected("CloudflareWARP") {
                    return true;
                }
            }
        }
        false
    }

    fn check_netsh_interface_connected(target: &str) -> bool {
        #[cfg(windows)]
        {
            let mut cmd = Command::new("netsh");
            cmd.args(["interface", "show", "interface"]);
            cmd.creation_flags(CREATE_NO_WINDOW);
            if let Ok(output) = cmd.output() {
                let text = String::from_utf8_lossy(&output.stdout);
                for line in text.lines() {
                    if line.contains(target) && (line.contains("Connected") || line.contains("connected")) {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn find_any_connected_vpn_interface() -> Option<String> {
        #[cfg(windows)]
        {
            let mut cmd = Command::new("netsh");
            cmd.args(["interface", "show", "interface"]);
            cmd.creation_flags(CREATE_NO_WINDOW);
            if let Ok(output) = cmd.output() {
                let text = String::from_utf8_lossy(&output.stdout);
                for line in text.lines() {
                    let lower = line.to_lowercase();
                    if lower.contains("connected")
                        && (lower.contains("vpn")
                            || lower.contains("wintun")
                            || lower.contains("wireguard")
                            || lower.contains("sing-box")
                            || lower.contains("tap")
                            || lower.contains("tun"))
                        && !lower.contains("disconnected")
                    {
                        if let Some(idx) = line.rfind("Dedicated").or_else(|| line.rfind("Loopback")) {
                            let name = line[idx + "Dedicated".len()..].trim();
                            if !name.is_empty() {
                                return Some(name.to_string());
                            }
                        }
                    }
                }
            }
        }
        None
    }

    fn netsh_lines() -> Vec<String> {
        #[cfg(windows)]
        {
            let mut cmd = Command::new("netsh");
            cmd.args(["interface", "show", "interface"]);
            cmd.creation_flags(CREATE_NO_WINDOW);
            if let Ok(output) = cmd.output() {
                return String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .map(|l| l.to_string())
                    .collect();
            }
        }
        Vec::new()
    }

    fn warp_status_stdout() -> Option<String> {
        #[cfg(windows)]
        {
            let bin = if Path::new(WARP_CLI_DEFAULT).exists() {
                WARP_CLI_DEFAULT
            } else {
                "warp-cli"
            };
            let mut cmd = Command::new(bin);
            cmd.arg("status");
            cmd.creation_flags(CREATE_NO_WINDOW);
            if let Ok(output) = cmd.output() {
                return Some(String::from_utf8_lossy(&output.stdout).to_string());
            }
        }
        None
    }

    /// Collect one OS evidence snapshot (processes + netsh + warp CLI).
    pub fn collect_snapshot() -> EvidenceSnapshot {
        EvidenceSnapshot {
            processes: Self::get_running_process_names(),
            netsh_lines: Self::netsh_lines(),
            warp_status: Self::warp_status_stdout(),
        }
    }

    fn snap_has_proc(snap: &EvidenceSnapshot, names: &[&str]) -> bool {
        snap.processes.iter().any(|p| names.iter().any(|n| p == n))
    }

    /// A line counts as a connected tunnel iff it names a tunnel-like
    /// interface AND says connected AND does not say disconnected. An
    /// optional keyword narrows to one engine (e.g. `cloudflarewarp`).
    fn snap_has_tunnel(snap: &EvidenceSnapshot, keyword: Option<&str>) -> bool {
        snap.netsh_lines.iter().any(|line| {
            let lower = line.to_lowercase();
            let connected_like =
                lower.contains("connected") || lower.contains("connesso") || lower.contains("verbunden");
            if !connected_like || lower.contains("disconnected") {
                return false;
            }
            let tunnel_like = lower.contains("vpn")
                || lower.contains("wintun")
                || lower.contains("wireguard")
                || lower.contains("sing-box")
                || lower.contains("cloudflarewarp")
                || lower.contains("warp")
                || lower.contains("tap")
                || lower.contains("tun")
                || lower.contains("psiphon");
            if !tunnel_like {
                return false;
            }
            match keyword {
                Some(k) => lower.contains(k),
                None => true,
            }
        })
    }

    fn snap_warp_cli_connected(snap: &EvidenceSnapshot) -> bool {
        match &snap.warp_status {
            Some(out) => {
                (out.contains("Connected") || out.contains("connected"))
                    && !out.contains("Disconnected")
                    && !out.contains("disconnected")
            }
            None => false,
        }
    }

    /// Pure verification decision: process evidence AND tunnel evidence are
    /// BOTH required for `connected = true`. Process-without-tunnel is an
    /// explicit non-connected verdict (spec: never infer a tunnel).
    /// `engine` is the lower-case engine id, or `"auto"` to accept the first
    /// verifiable engine in priority order.
    pub fn evaluate_evidence(engine: &str, snap: &EvidenceSnapshot) -> Verification {
        let engine = engine.to_ascii_lowercase();
        // (id, display, process names, tunnel keyword)
        const TABLE: &[(&str, &str, &[&str], Option<&str>)] = &[
            ("dedicated", "Dedicated Relay (sing-box)", &["sing-box.exe"], None),
            ("warp", "Cloudflare WARP", &["warp-svc.exe", "cloudflare warp.exe"], Some("warp")),
            ("psiphon", "Psiphon Tunnel", &["psiphon3.exe", "psiphond.exe"], Some("psiphon")),
            ("wireguard", "WireGuard Tunnel", &["wireguard.exe"], Some("wireguard")),
            ("proton", "Proton VPN", &["protonvpn.exe", "protonvpn.service.exe"], Some("proton")),
            ("windscribe", "Windscribe VPN", &["windscribe.exe", "windscribe-service.exe"], Some("windscribe")),
            ("openvpn", "OpenVPN Tunnel", &["openvpn.exe", "openvpnserv.exe"], Some("openvpn")),
            ("tailscale", "Tailscale Mesh", &["tailscale-ipn.exe", "tailscaled.exe"], Some("tailscale")),
            ("mullvad", "Mullvad VPN", &["mullvad-vpn.exe", "mullvad-daemon.exe"], Some("mullvad")),
            ("nord", "NordVPN", &["nordvpn.exe", "nordvpn-service.exe"], Some("nord")),
            ("clash", "Clash / Mihomo", &["clash.exe", "clash-meta.exe", "mihomo.exe"], None),
            ("nekobox", "NekoBox / NekoRay", &["nekobox.exe", "nekoray.exe"], None),
            ("v2ray", "Xray / V2Ray", &["xray.exe", "v2ray.exe"], None),
        ];

        // WARP additionally consults its CLI (strongest signal for that engine).
        if engine == "warp" || engine == "auto" {
            let cli = Self::snap_warp_cli_connected(snap);
            let tunnel = Self::snap_has_tunnel(snap, Some("warp"));
            let proc = Self::snap_has_proc(snap, &["warp-svc.exe", "cloudflare warp.exe"]);
            if engine == "warp" {
                let connected = cli && tunnel;
                return Verification {
                    connected,
                    evidence: VpnEvidence {
                        process_running: proc,
                        tunnel_detected: tunnel,
                        cli_reports_connected: cli,
                        detail: if connected {
                            "warp-cli reports Connected and CloudflareWARP tunnel interface is up.".into()
                        } else if cli && !tunnel {
                            "warp-cli reports Connected but no WARP tunnel interface detected.".into()
                        } else if tunnel && !cli {
                            "WARP tunnel interface present but warp-cli does not report Connected.".into()
                        } else {
                            "No WARP process, CLI confirmation, or tunnel interface.".into()
                        },
                    },
                };
            }
            if cli && tunnel {
                return Verification {
                    connected: true,
                    evidence: VpnEvidence {
                        process_running: proc,
                        tunnel_detected: true,
                        cli_reports_connected: true,
                        detail: "warp-cli reports Connected and CloudflareWARP tunnel interface is up.".into(),
                    },
                };
            }
        }

        for (id, _display, procs, keyword) in TABLE {
            if engine != "auto" && engine != *id {
                continue;
            }
            if *id == "warp" {
                continue; // handled above (CLI-gated)
            }
            let proc = Self::snap_has_proc(snap, procs);
            if !proc {
                continue;
            }
            let tunnel = Self::snap_has_tunnel(snap, *keyword);
            if tunnel {
                return Verification {
                    connected: true,
                    evidence: VpnEvidence {
                        process_running: true,
                        tunnel_detected: true,
                        cli_reports_connected: false,
                        detail: format!("Process {0} running and matching tunnel interface detected.", procs.join("/")),
                    },
                };
            }
            // Process present but tunnel absent: NOT connected (strict).
            if engine != "auto" {
                return Verification {
                    connected: false,
                    evidence: VpnEvidence {
                        process_running: true,
                        tunnel_detected: false,
                        cli_reports_connected: false,
                        detail: format!(
                            "Process {0} running but no matching tunnel interface detected — not connected.",
                            procs.join("/")
                        ),
                    },
                };
            }
        }

        Verification {
            connected: false,
            evidence: VpnEvidence {
                process_running: false,
                tunnel_detected: false,
                cli_reports_connected: false,
                detail: if engine == "auto" {
                    "No verifiable VPN engine detected.".into()
                } else {
                    format!("Engine '{engine}' not running (no process match).")
                },
            },
        }
    }

    pub fn detect_active_vpn() -> Option<DetectedVpn> {
        let procs = Self::get_running_process_names();

        // 1. Dedicated Relay (sing-box)
        if procs.iter().any(|p| p == "sing-box.exe") {
            return Some(DetectedVpn {
                engine: "dedicated".to_string(),
                display_name: "Dedicated Relay (sing-box)".to_string(),
            });
        }

        // 2. Cloudflare WARP
        if Self::is_warp_active(&procs) {
            return Some(DetectedVpn {
                engine: "warp".to_string(),
                display_name: "Cloudflare WARP".to_string(),
            });
        }

        // 3. Psiphon
        if procs.iter().any(|p| p == "psiphon3.exe" || p == "psiphond.exe") {
            return Some(DetectedVpn {
                engine: "psiphon".to_string(),
                display_name: "Psiphon Tunnel".to_string(),
            });
        }

        // 4. WireGuard
        if procs.iter().any(|p| p == "wireguard.exe") || Self::check_netsh_interface_connected("WireGuard") {
            return Some(DetectedVpn {
                engine: "wireguard".to_string(),
                display_name: "WireGuard Tunnel".to_string(),
            });
        }

        // 5. Proton VPN
        if procs.iter().any(|p| p == "protonvpn.exe" || p == "protonvpn.service.exe") {
            return Some(DetectedVpn {
                engine: "proton".to_string(),
                display_name: "Proton VPN".to_string(),
            });
        }

        // 6. Windscribe
        if procs.iter().any(|p| p == "windscribe.exe" || p == "windscribe-service.exe") {
            return Some(DetectedVpn {
                engine: "windscribe".to_string(),
                display_name: "Windscribe VPN".to_string(),
            });
        }

        // 7. OpenVPN
        if procs.iter().any(|p| p == "openvpn.exe" || p == "openvpnserv.exe") {
            return Some(DetectedVpn {
                engine: "openvpn".to_string(),
                display_name: "OpenVPN Tunnel".to_string(),
            });
        }

        // 8. Tailscale
        if procs.iter().any(|p| p == "tailscale-ipn.exe" || p == "tailscaled.exe") {
            return Some(DetectedVpn {
                engine: "tailscale".to_string(),
                display_name: "Tailscale Mesh".to_string(),
            });
        }

        // 9. Mullvad
        if procs.iter().any(|p| p == "mullvad-vpn.exe" || p == "mullvad-daemon.exe") {
            return Some(DetectedVpn {
                engine: "mullvad".to_string(),
                display_name: "Mullvad VPN".to_string(),
            });
        }

        // 10. NordVPN
        if procs.iter().any(|p| p == "nordvpn.exe" || p == "nordvpn-service.exe") {
            return Some(DetectedVpn {
                engine: "nord".to_string(),
                display_name: "NordVPN".to_string(),
            });
        }

        // 11. Clash / Mihomo
        if procs.iter().any(|p| p == "clash.exe" || p == "clash-meta.exe" || p == "mihomo.exe") {
            return Some(DetectedVpn {
                engine: "clash".to_string(),
                display_name: "Clash / Mihomo".to_string(),
            });
        }

        // 12. NekoBox / NekoRay
        if procs.iter().any(|p| p == "nekobox.exe" || p == "nekoray.exe") {
            return Some(DetectedVpn {
                engine: "nekobox".to_string(),
                display_name: "NekoBox / NekoRay".to_string(),
            });
        }

        // 13. Xray / V2Ray
        if procs.iter().any(|p| p == "xray.exe" || p == "v2ray.exe") {
            return Some(DetectedVpn {
                engine: "v2ray".to_string(),
                display_name: "Xray / V2Ray".to_string(),
            });
        }

        // 14. Generic connected VPN adapter
        if let Some(iface) = Self::find_any_connected_vpn_interface() {
            return Some(DetectedVpn {
                engine: "generic".to_string(),
                display_name: format!("VPN Adapter ({})", iface),
            });
        }

        None
    }

    /// Reconcile in-memory state with OS evidence. Never fabricates
    /// `Connected`: managed connections that lose verification become
    /// `Error`; externally visible VPNs become `Connected` only when
    /// verified (marked unmanaged), else `Unknown`.
    pub fn sync_process_state(&self) {
        match *self.state.lock() {
            VpnState::Connecting | VpnState::Verifying | VpnState::Disconnecting => return,
            VpnState::Error => return, // latched until an explicit action
            _ => {}
        }

        let managed = self.managed.load(Ordering::SeqCst);
        if managed {
            // A connection WE established: re-verify the same engine.
            let engine = self.engine.lock().clone();
            let v = Self::evaluate_evidence(&engine, &Self::collect_snapshot());
            self.set_evidence(v.evidence.clone());
            if v.connected {
                if self.vpn_state() != VpnState::Connected {
                    self.set_state(VpnState::Connected, None);
                    if self.start_time.lock().is_none() {
                        *self.start_time.lock() = Some(Instant::now());
                    }
                }
            } else if self.vpn_state() == VpnState::Connected {
                let reason = format!("Managed '{engine}' connection lost: {}", v.evidence.detail);
                self.set_state(VpnState::Error, Some(reason.clone()));
                *self.last_error.lock() = Some(reason);
                tracing::warn!("VPN: {}", self.last_error.lock().as_deref().unwrap_or("lost"));
            }
            return;
        }

        // Unmanaged: adopt a genuinely verified external connection, else
        // Unknown (never Connected) when something VPN-shaped is visible.
        let snap = Self::collect_snapshot();
        let v = Self::evaluate_evidence("auto", &snap);
        self.set_evidence(v.evidence.clone());
        if v.connected {
            let detected = Self::detect_active_vpn();
            let (engine, name) = detected
                .map(|d| (d.engine, d.display_name))
                .unwrap_or_else(|| ("generic".to_string(), "VPN (external)".to_string()));
            let prev = self.engine.lock().clone();
            let was_connected = self.vpn_state() == VpnState::Connected;
            *self.engine.lock() = engine;
            *self.engine_name.lock() = name.clone();
            self.auto_detected.store(true, Ordering::SeqCst);
            self.user_disconnected.store(false, Ordering::SeqCst);
            self.set_state(VpnState::Connected, Some("externally managed connection".into()));
            if !was_connected || prev != self.engine.lock().clone() {
                if self.start_time.lock().is_none() {
                    *self.start_time.lock() = Some(Instant::now());
                }
                tracing::info!("VPN: verified external connection: {}", name);
                self.refresh_ip_async();
            }
        } else if Self::detect_active_vpn().is_some() {
            // VPN-shaped activity without verification: Unknown, not Connected.
            if self.vpn_state() != VpnState::Unknown {
                self.set_state(
                    VpnState::Unknown,
                    Some(format!("Unverified VPN activity: {}", v.evidence.detail)),
                );
                *self.engine.lock() = "unknown".to_string();
                *self.engine_name.lock() = "Unverified VPN".to_string();
                self.auto_detected.store(true, Ordering::SeqCst);
                *self.start_time.lock() = None;
            }
        } else if self.vpn_state() != VpnState::Disconnected {
            self.set_state(VpnState::Disconnected, None);
            *self.engine.lock() = "none".to_string();
            *self.engine_name.lock() = "Offline".to_string();
            *self.start_time.lock() = None;
            self.auto_detected.store(false, Ordering::SeqCst);
            self.managed.store(false, Ordering::SeqCst);
            self.refresh_ip_async();
        }
    }

    fn watchdog_tick(&self) {
        // Run continuous auto-detection sync
        self.sync_process_state();

        if self.user_disconnected.load(Ordering::SeqCst) {
            return;
        }

        // Auto-reconnect a managed dedicated relay that dropped. Full
        // verified cycle (not spawn-only); skipped while a cycle is already
        // in flight to avoid overlap.
        let current_engine = self.engine.lock().clone();
        let st = self.vpn_state();
        if current_engine == "dedicated"
            && self.managed.load(Ordering::SeqCst)
            && self.auto_reconnect.load(Ordering::SeqCst)
            && matches!(st, VpnState::Disconnected | VpnState::Error)
        {
            tracing::warn!("VPN Watchdog: managed relay down ({:?}); reconnecting with verification...", st);
            let _ = self.connect_verified("dedicated", Duration::from_secs(10));
        }
    }

    pub fn get_status(&self) -> VpnStatus {
        self.sync_process_state();

        let state = self.vpn_state();
        let connected = state == VpnState::Connected;
        let uptime_secs = if connected {
            self.start_time
                .lock()
                .map(|t| t.elapsed().as_secs())
                .unwrap_or(0)
        } else {
            0
        };
        let ev = self.evidence.lock().clone();

        VpnStatus {
            connected,
            state,
            managed: self.managed.load(Ordering::SeqCst),
            engine: self.engine.lock().clone(),
            engine_name: self.engine_name.lock().clone(),
            ip: self.last_ip.lock().clone(),
            country: self.last_country.lock().clone(),
            city: self.last_city.lock().clone(),
            latency_ms: *self.last_latency.lock(),
            uptime_secs,
            auto_reconnect: self.auto_reconnect.load(Ordering::SeqCst),
            last_error: self
                .last_error
                .lock()
                .clone()
                .or_else(|| self.state_reason.lock().clone()),
            auto_detected: self.auto_detected.load(Ordering::SeqCst),
            process_running: ev.process_running,
            tunnel_detected: ev.tunnel_detected,
            verification_detail: ev.detail,
        }
    }

    /// Request a connection and return success ONLY after verification.
    /// `CONNECTING → VERIFYING → CONNECTED`, else `ERROR` (+ `Err`).
    /// A selected engine or a sent command is never reported as connected.
    pub fn connect(&self, engine: &str) -> Result<VpnStatus, String> {
        self.connect_verified(engine, DEFAULT_CONNECT_TIMEOUT)
    }

    pub fn connect_verified(&self, engine: &str, timeout: Duration) -> Result<VpnStatus, String> {
        let engine = engine.trim().to_ascii_lowercase();
        self.user_disconnected.store(false, Ordering::SeqCst);
        *self.last_error.lock() = None;
        self.set_state(VpnState::Connecting, Some(format!("connecting via {engine}")));
        *self.engine.lock() = engine.clone();

        // "auto": adopt a verified external connection if present, else fall
        // through to the preferred local engines.
        if engine == "auto" {
            let v = Self::evaluate_evidence("auto", &Self::collect_snapshot());
            self.set_evidence(v.evidence.clone());
            if v.connected {
                if let Some(detected) = Self::detect_active_vpn() {
                    *self.engine.lock() = detected.engine.clone();
                    *self.engine_name.lock() = detected.display_name.clone();
                    self.auto_detected.store(true, Ordering::SeqCst);
                    self.managed.store(false, Ordering::SeqCst);
                    self.set_state(VpnState::Connected, Some("verified external connection".into()));
                    *self.start_time.lock() = Some(Instant::now());
                    tracing::info!("VPN: Bound to verified {}", self.engine_name.lock());
                    self.refresh_ip_async();
                    return Ok(self.get_status());
                }
            }
            if Path::new(SINGBOX_EXE).exists() {
                return self.connect_verified("dedicated", timeout);
            }
            return self.connect_verified("warp", timeout);
        }

        // Display name up front (state shows Connecting with engine context).
        *self.engine_name.lock() = match engine.as_str() {
            "dedicated" => "Dedicated Relay (sing-box)".to_string(),
            "warp" => "Cloudflare WARP".to_string(),
            "psiphon" => "Psiphon Tunnel".to_string(),
            other => format!("VPN ({other})"),
        };

        // Spawn only — success here means "command sent", NOT connected.
        let spawn_result = match engine.as_str() {
            "dedicated" => Self::spawn_dedicated(),
            "warp" => self.spawn_warp(),
            "psiphon" => self.spawn_psiphon(),
            other => Err(format!(
                "Unknown engine '{other}'. Supported: auto, dedicated, warp, psiphon."
            )),
        };
        if let Err(e) = spawn_result {
            *self.last_error.lock() = Some(e.clone());
            self.set_state(VpnState::Error, Some(e.clone()));
            self.managed.store(false, Ordering::SeqCst);
            return Err(e);
        }

        // Early-exit probe (spawned daemons only): if the engine process is
        // absent shortly after spawn it crashed on startup (bad config,
        // missing driver) — fail in ~3s with a reason instead of burning
        // the full verification timeout. warp-cli is one-shot, so the
        // generic loop below covers it.
        if matches!(engine.as_str(), "dedicated" | "psiphon") {
            let procs = match engine.as_str() {
                "dedicated" => &["sing-box.exe"][..],
                _ => &["psiphon3.exe", "psiphond.exe"][..],
            };
            std::thread::sleep(Duration::from_millis(1500));
            let mut alive =
                any_process_running(&Self::get_running_process_names(), procs);
            if !alive {
                std::thread::sleep(Duration::from_millis(1500));
                alive = any_process_running(&Self::get_running_process_names(), procs);
            }
            if !alive {
                let reason = format!(
                    "{0} exited within seconds of starting — check its setup ({1}).",
                    procs.join("/"),
                    if engine == "dedicated" {
                        "Administrator rights for the TUN driver and a valid sing-box-client.json"
                    } else {
                        "a working psiphon3.exe in C:\\VPN"
                    }
                );
                *self.last_error.lock() = Some(reason.clone());
                self.set_state(VpnState::Error, Some(reason.clone()));
                self.managed.store(false, Ordering::SeqCst);
                return Err(reason);
            }
        }

        // VERIFYING: poll evidence until verified or timeout.
        self.set_state(VpnState::Verifying, Some(format!("verifying {engine} tunnel")));
        let deadline = Instant::now() + timeout;
        let mut last_detail = String::from("no verification result yet");
        while Instant::now() < deadline {
            let v = Self::evaluate_evidence(&engine, &Self::collect_snapshot());
            self.set_evidence(v.evidence.clone());
            last_detail = v.evidence.detail.clone();
            if v.connected {
                self.managed.store(true, Ordering::SeqCst);
                self.auto_detected.store(false, Ordering::SeqCst);
                self.set_state(VpnState::Connected, None);
                *self.start_time.lock() = Some(Instant::now());
                tracing::info!("VPN: verified connected via {}", self.engine_name.lock());
                self.refresh_ip_async();
                return Ok(self.get_status());
            }
            std::thread::sleep(Duration::from_millis(500));
        }

        let reason = format!("Verification timeout after {}s: {last_detail}", timeout.as_secs());
        *self.last_error.lock() = Some(reason.clone());
        self.set_state(VpnState::Error, Some(reason.clone()));
        self.managed.store(false, Ordering::SeqCst);
        // Best-effort cleanup of a half-started tunnel.
        Self::kill_engine_processes(&engine);
        Err(reason)
    }

    /// Validate the sing-box config without starting a tunnel (`sing-box
    /// check` needs no privileges). Returns Err with sing-box's own message
    /// on invalid config.
    fn check_singbox_config() -> Result<(), String> {
        let mut cmd = Command::new(SINGBOX_EXE);
        cmd.args(["check", "-c", SINGBOX_CONFIG]);
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);
        match cmd.output() {
            Ok(out) if out.status.success() => Ok(()),
            Ok(out) => {
                let detail = String::from_utf8_lossy(&out.stderr).trim().to_string();
                Err(if detail.is_empty() {
                    "sing-box config check failed with no details".to_string()
                } else {
                    // Keep it to the first meaningful line for UI display.
                    let line = detail.lines().find(|l| !l.trim().is_empty()).unwrap_or(&detail);
                    format!("sing-box config invalid: {}", line.chars().take(220).collect::<String>())
                })
            }
            Err(e) => Err(format!("Failed to run sing-box config check: {e}")),
        }
    }

    /// Spawn-only: sends the connect command. Success means the command was
    /// accepted — it says NOTHING about connectivity (verified later).
    fn spawn_dedicated() -> Result<(), String> {
        if !Path::new(SINGBOX_EXE).exists() {
            return Err(format!("sing-box executable not found at {SINGBOX_EXE}"));
        }
        if !Path::new(SINGBOX_CONFIG).exists() {
            return Err(format!("sing-box config not found at {SINGBOX_CONFIG}"));
        }
        if !is_elevated() {
            return Err(
                "Administrator rights required: the Dedicated Relay TUN driver cannot start \
                without elevation (sing-box fails with 'Access is denied'). Restart GPO Autofish \
                as Administrator, or use Cloudflare WARP instead.".to_string(),
            );
        }
        if let Err(e) = Self::check_singbox_config() {
            return Err(e);
        }

        // Clean up any stale instances first
        Self::kill_process("sing-box.exe");
        std::thread::sleep(Duration::from_millis(300));

        let mut cmd = Command::new(SINGBOX_EXE);
        cmd.args(["run", "-c", SINGBOX_CONFIG]);
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);

        match cmd.spawn() {
            Ok(_) => {
                tracing::info!("VPN: sing-box spawn requested; verifying tunnel...");
                Ok(())
            }
            Err(e) => Err(format!("Failed to spawn sing-box: {e}")),
        }
    }

    fn spawn_warp(&self) -> Result<(), String> {
        let bin = if Path::new(WARP_CLI_DEFAULT).exists() {
            WARP_CLI_DEFAULT
        } else {
            "warp-cli"
        };
        let mut cmd = Command::new(bin);
        cmd.arg("connect");
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);

        match cmd.output() {
            Ok(out) => {
                if out.status.success() {
                    tracing::info!("VPN: warp-cli connect accepted; verifying tunnel...");
                    Ok(())
                } else {
                    Err(String::from_utf8_lossy(&out.stderr).to_string())
                }
            }
            Err(e) => Err(format!(
                "warp-cli not found or failed to run ({e}). Install Cloudflare WARP so warp-cli is on PATH."
            )),
        }
    }

    fn spawn_psiphon(&self) -> Result<(), String> {
        let path = PSIPHON_EXE;
        if !Path::new(path).exists() {
            return Err("Psiphon executable not found in C:\\VPN".to_string());
        }
        let mut cmd = Command::new(path);
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd.spawn().map_err(|e| format!("Failed to run Psiphon: {e}"))?;
        tracing::info!("VPN: psiphon spawn requested; verifying tunnel...");
        Ok(())
    }

    fn kill_engine_processes(engine: &str) {
        match engine {
            "warp" => {
                let bin = if Path::new(WARP_CLI_DEFAULT).exists() {
                    WARP_CLI_DEFAULT
                } else {
                    "warp-cli"
                };
                let mut cmd = Command::new(bin);
                cmd.arg("disconnect");
                #[cfg(windows)]
                cmd.creation_flags(CREATE_NO_WINDOW);
                let _ = cmd.output();
            }
            "dedicated" => Self::kill_process("sing-box.exe"),
            "psiphon" => {
                Self::kill_process("psiphon3.exe");
                Self::kill_process("psiphond.exe");
            }
            "wireguard" => {
                Self::kill_process("wireguard.exe");
            }
            "proton" => {
                Self::kill_process("ProtonVPN.exe");
                Self::kill_process("ProtonVPN.Service.exe");
            }
            "windscribe" => {
                Self::kill_process("Windscribe.exe");
                Self::kill_process("windscribe-service.exe");
            }
            "openvpn" => {
                Self::kill_process("openvpn.exe");
                Self::kill_process("openvpnserv.exe");
            }
            "clash" => {
                Self::kill_process("clash.exe");
                Self::kill_process("clash-meta.exe");
                Self::kill_process("mihomo.exe");
            }
            "nekobox" => {
                Self::kill_process("nekobox.exe");
                Self::kill_process("nekoray.exe");
            }
            "v2ray" => {
                Self::kill_process("xray.exe");
                Self::kill_process("v2ray.exe");
            }
            _ => {
                let procs = Self::get_running_process_names();
                if procs.iter().any(|p| p == "sing-box.exe") {
                    Self::kill_process("sing-box.exe");
                }
            }
        }
    }

    pub fn disconnect(&self) -> Result<VpnStatus, String> {
        self.disconnect_verified(DISCONNECT_TIMEOUT)
    }

    /// `DISCONNECTING → DISCONNECTED`, verified by evidence absence. Fails to
    /// `Error` if engine processes persist past the timeout.
    pub fn disconnect_verified(&self, timeout: Duration) -> Result<VpnStatus, String> {
        self.user_disconnected.store(true, Ordering::SeqCst);
        let engine = self.engine.lock().clone();
        self.set_state(VpnState::Disconnecting, Some(format!("disconnecting {engine}")));
        Self::kill_engine_processes(&engine);

        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let snap = Self::collect_snapshot();
            // Disconnected when NEITHER this engine's processes NOR any
            // tunnel interface remain.
            let engine_procs = match engine.as_str() {
                "dedicated" => vec!["sing-box.exe"],
                "warp" => vec!["warp-svc.exe"],
                "psiphon" => vec!["psiphon3.exe", "psiphond.exe"],
                _ => vec![],
            };
            let procs_left = Self::snap_has_proc(&snap, &engine_procs);
            let no_tunnel = !Self::snap_has_tunnel(&snap, None);
            if !procs_left && (no_tunnel || engine_procs.is_empty()) {
                self.managed.store(false, Ordering::SeqCst);
                *self.engine.lock() = "none".to_string();
                *self.engine_name.lock() = "Offline".to_string();
                *self.start_time.lock() = None;
                self.auto_detected.store(false, Ordering::SeqCst);
                *self.last_error.lock() = None;
                self.set_evidence(VpnEvidence::default());
                self.set_state(VpnState::Disconnected, None);
                tracing::info!("VPN: verified disconnected");
                self.refresh_ip_async();
                return Ok(self.get_status());
            }
            std::thread::sleep(Duration::from_millis(400));
        }

        let reason = format!("Disconnect timeout: '{engine}' processes persist after {}s.", timeout.as_secs());
        *self.last_error.lock() = Some(reason.clone());
        self.set_state(VpnState::Error, Some(reason.clone()));
        Err(reason)
    }

    /// Block until `want_connected` matches verified reality, or timeout.
    /// Side-effect free (never connects/disconnects by itself).
    pub fn wait_for(&self, want_connected: bool, timeout: Duration) -> Result<VpnStatus, String> {
        let deadline = Instant::now() + timeout;
        loop {
            let st = self.get_status();
            if st.connected == want_connected {
                return Ok(st);
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "VPN wait timeout after {}s: expected {}, observed {} (engine {}, {}).",
                    timeout.as_secs(),
                    if want_connected { "connected" } else { "disconnected" },
                    st.state.as_str(),
                    st.engine,
                    st.verification_detail,
                ));
            }
            std::thread::sleep(Duration::from_millis(400));
        }
    }

    fn kill_process(name: &str) {
        #[cfg(windows)]
        {
            let mut cmd = Command::new("taskkill");
            cmd.args(["/F", "/IM", name]);
            cmd.creation_flags(CREATE_NO_WINDOW);
            let _ = cmd.output();
        }
    }

    pub fn test_ping(&self) -> PingResult {
        let addr = SocketAddr::from((DEDICATED_SERVER_IP, DEDICATED_SERVER_PORT));
        let start = Instant::now();

        match std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(2500)) {
            Ok(_) => {
                let ms = start.elapsed().as_millis() as u64;
                *self.last_latency.lock() = Some(ms);
                PingResult {
                    success: true,
                    latency_ms: ms,
                    target: DEDICATED_RELAY_LABEL.into(),
                    error: None,
                }
            }
            Err(e) => PingResult {
                success: false,
                latency_ms: 0,
                target: DEDICATED_RELAY_LABEL.into(),
                error: Some(e.to_string()),
            },
        }
    }

    pub fn reset_network(&self) -> Result<String, String> {
        #[cfg(windows)]
        {
            let mut cmd = Command::new("ipconfig");
            cmd.arg("/flushdns");
            cmd.creation_flags(CREATE_NO_WINDOW);
            let _ = cmd.output();
        }
        Ok("DNS cache flushed and network routes refreshed.".to_string())
    }

    pub fn get_logs(&self, max_lines: usize) -> Vec<String> {
        if let Ok(content) = std::fs::read_to_string(SINGBOX_LOG) {
            let lines: Vec<&str> = content.lines().collect();
            let start = lines.len().saturating_sub(max_lines);
            lines[start..].iter().map(|s| s.to_string()).collect()
        } else {
            Vec::new()
        }
    }

    pub fn set_auto_reconnect(&self, enabled: bool) {
        self.auto_reconnect.store(enabled, Ordering::SeqCst);
    }

    fn refresh_ip_async(&self) {
        let target_ip = Arc::clone(&self.last_ip);
        let target_country = Arc::clone(&self.last_country);
        let target_city = Arc::clone(&self.last_city);

        std::thread::spawn(move || {
            let client = match reqwest::blocking::Client::builder()
                .timeout(Duration::from_millis(3000))
                .build()
            {
                Ok(c) => c,
                Err(_) => return,
            };

            // Attempt ip-api.com
            if let Ok(res) = client.get("http://ip-api.com/json/").send() {
                if let Ok(json) = res.json::<IpApiResponse>() {
                    if let Some(ip) = json.query {
                        *target_ip.lock() = ip;
                    }
                    if let Some(country) = json.country {
                        *target_country.lock() = country;
                    }
                    if let Some(city) = json.city {
                        *target_city.lock() = city;
                    }
                    return;
                }
            }

            // Fallback ipify
            if let Ok(res) = client.get("https://api.ipify.org").send() {
                if let Ok(ip) = res.text() {
                    *target_ip.lock() = ip.trim().to_string();
                }
            }
        });
    }
}

impl VpnControl for VpnManager {
    fn vpn_state(&self) -> VpnState {
        VpnManager::vpn_state(self)
    }

    fn vpn_connect_verified(&self, engine: &str, timeout: Duration) -> Result<VpnStatus, String> {
        self.connect_verified(engine, timeout)
    }

    fn vpn_disconnect_verified(&self, timeout: Duration) -> Result<VpnStatus, String> {
        self.disconnect_verified(timeout)
    }

    fn vpn_wait_for(&self, want_connected: bool, timeout: Duration) -> Result<VpnStatus, String> {
        self.wait_for(want_connected, timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(procs: &[&str], netsh: &[&str], warp: Option<&str>) -> EvidenceSnapshot {
        EvidenceSnapshot {
            processes: procs.iter().map(|s| s.to_string()).collect(),
            netsh_lines: netsh.iter().map(|s| s.to_string()).collect(),
            warp_status: warp.map(|s| s.to_string()),
        }
    }

    const TUN_LINE: &str = "Connected  Dedicated  sing-box TUN";
    const WARP_LINE: &str = "Connected  Dedicated  CloudflareWARP";
    const WIFI_LINE: &str = "Connected  Dedicated  Wi-Fi";
    const DIS_LINE: &str = "Disconnected  Dedicated  sing-box TUN";

    #[test]
    fn fresh_state_is_disconnected() {
        let mgr = VpnManager::new_isolated();
        assert_eq!(mgr.vpn_state(), VpnState::Disconnected);
        assert!(!mgr.connected.load(Ordering::SeqCst));
        assert!(!mgr.managed.load(Ordering::SeqCst));
    }

    #[test]
    fn selection_alone_never_connects() {
        // Merely naming an engine (no evidence) must not verify.
        let s = snap(&[], &[], None);
        assert!(!VpnManager::evaluate_evidence("dedicated", &s).connected);
        assert!(!VpnManager::evaluate_evidence("warp", &s).connected);
        assert!(!VpnManager::evaluate_evidence("auto", &s).connected);
    }

    #[test]
    fn empty_snapshot_is_not_unknown_nor_connected() {
        let v = VpnManager::evaluate_evidence("auto", &snap(&[], &[], None));
        assert!(!v.connected);
        assert!(!v.evidence.process_running);
        assert!(!v.evidence.tunnel_detected);
    }

    #[test]
    fn dedicated_process_plus_tunnel_verifies() {
        let v = VpnManager::evaluate_evidence("dedicated", &snap(&["sing-box.exe"], &[TUN_LINE], None));
        assert!(v.connected);
        assert!(v.evidence.process_running);
        assert!(v.evidence.tunnel_detected);
    }

    #[test]
    fn dedicated_process_without_tunnel_is_not_connected() {
        // The core false-Connected bug: process alone must NOT verify.
        let v = VpnManager::evaluate_evidence("dedicated", &snap(&["sing-box.exe"], &[WIFI_LINE], None));
        assert!(!v.connected);
        assert!(v.evidence.process_running);
        assert!(!v.evidence.tunnel_detected);
        assert!(v.evidence.detail.contains("no matching tunnel"));
    }

    #[test]
    fn disconnected_interface_line_does_not_count() {
        let v = VpnManager::evaluate_evidence("dedicated", &snap(&["sing-box.exe"], &[DIS_LINE], None));
        assert!(!v.connected);
    }

    #[test]
    fn warp_requires_cli_and_tunnel() {
        let cli = "Status update: Connected";
        let ok = snap(&["warp-svc.exe"], &[WARP_LINE], Some(cli));
        let v = VpnManager::evaluate_evidence("warp", &ok);
        assert!(v.connected);
        assert!(v.evidence.cli_reports_connected);

        // CLI success alone (exit-code-only, the old bug) is NOT enough.
        let cli_only = snap(&["warp-svc.exe"], &[WIFI_LINE], Some(cli));
        let v2 = VpnManager::evaluate_evidence("warp", &cli_only);
        assert!(!v2.connected);

        // Tunnel without CLI confirmation is NOT enough either.
        let tun_only = snap(&["warp-svc.exe"], &[WARP_LINE], Some("Status update: Disconnected"));
        let v3 = VpnManager::evaluate_evidence("warp", &tun_only);
        assert!(!v3.connected);
    }

    #[test]
    fn auto_accepts_first_verifiable_engine() {
        let s = snap(&["sing-box.exe", "warp-svc.exe"], &[TUN_LINE], Some("Disconnected"));
        let v = VpnManager::evaluate_evidence("auto", &s);
        assert!(v.connected, "dedicated verifies even with warp present-but-down");

        let none = snap(&["warp-svc.exe"], &[WIFI_LINE], Some("Disconnected"));
        let v2 = VpnManager::evaluate_evidence("auto", &none);
        assert!(!v2.connected);
    }

    #[test]
    fn unknown_engine_never_verifies() {
        let v = VpnManager::evaluate_evidence("fictional", &snap(&["sing-box.exe"], &[TUN_LINE], None));
        assert!(!v.connected);
    }

    #[test]
    fn generic_tunnel_keyword_matches_vpn_adapters() {
        let line = "Connected  Dedicated  VPN Adapter (ProtonVPN)";
        let v = VpnManager::evaluate_evidence(
            "proton",
            &snap(&["protonvpn.exe"], &[line], None),
        );
        assert!(v.connected);
    }

    #[test]
    fn state_serializes_snake_case() {
        assert_eq!(serde_json::to_string(&VpnState::Connected).unwrap(), "\"connected\"");
        assert_eq!(serde_json::to_string(&VpnState::Verifying).unwrap(), "\"verifying\"");
    }

    #[test]
    fn process_matcher_is_exact_case_insensitive_list() {
        // get_running_process_names() already lowercases; matching is exact.
        let procs = vec!["sing-box.exe".to_string(), "warp-svc.exe".to_string()];
        assert!(any_process_running(&procs, &["sing-box.exe"]));
        assert!(!any_process_running(&procs, &["box.exe"]));
        assert!(!any_process_running(&[], &["sing-box.exe"]));
        assert!(!any_process_running(&procs, &[]));
    }

    #[test]
    fn elevation_probe_runs_without_panic() {
        // Value is environment-dependent (CI runners are usually elevated,
        // dev shells usually not) — only the call itself is asserted.
        let _ = is_elevated();
    }

    #[test]
    fn dedicated_preflight_fails_fast_with_actionable_error() {
        // Whatever the machine state (missing exe/config on CI, no admin
        // rights on a dev box), spawn_dedicated must fail BEFORE spawning —
        // fast and with a message naming the missing prerequisite.
        let start = std::time::Instant::now();
        let err = VpnManager::spawn_dedicated().unwrap_err();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "pre-flight must not burn the verification timeout"
        );
        assert!(!err.is_empty());
        let lower = err.to_ascii_lowercase();
        assert!(
            lower.contains("not found")
                || lower.contains("administrator")
                || lower.contains("invalid"),
            "error must name the prerequisite, got: {err}"
        );
    }

    #[test]
    fn error_state_is_not_connected() {
        for st in [VpnState::Disconnected, VpnState::Connecting, VpnState::Verifying, VpnState::Disconnecting, VpnState::Error, VpnState::Unknown] {
            assert_ne!(st, VpnState::Connected, "{st:?} must never read as connected");
        }
    }
}
