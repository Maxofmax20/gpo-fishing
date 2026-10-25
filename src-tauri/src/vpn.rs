use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

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
    pub germany_verified: bool,
    pub warp_verified: bool,
    pub exit_age_secs: Option<u64>,
    pub exit_error: Option<String>,
    pub administrator: bool,
    pub helper_ready: bool,
    pub recovery_active: bool,
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

#[derive(Debug, Clone, Default)]
struct ExitObservation {
    ip: String,
    country: String,
    warp: bool,
    checked: Option<Instant>,
    error: Option<String>,
}

impl ExitObservation {
    fn germany_verified(&self) -> bool {
        self.country == "DE"
            && self
                .checked
                .is_some_and(|at| at.elapsed() <= Duration::from_secs(30))
            && self.error.is_none()
    }
}

fn parse_exit_trace(raw: &str) -> Result<ExitObservation, String> {
    let field = |key: &str| raw.lines().find_map(|line| line.trim().strip_prefix(key));
    let ip = field("ip=")
        .filter(|ip| ip.parse::<std::net::IpAddr>().is_ok())
        .ok_or("Exit probe did not return a valid IP address")?;
    let country = field("loc=")
        .filter(|s| s.len() == 2 && s.bytes().all(|c| c.is_ascii_uppercase()))
        .ok_or("Exit probe did not return a country")?;
    Ok(ExitObservation {
        ip: ip.into(),
        country: country.into(),
        warp: matches!(field("warp="), Some("on" | "plus")),
        checked: Some(Instant::now()),
        error: None,
    })
}

fn probe_exit(timeout: Duration) -> Result<ExitObservation, String> {
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(timeout)
        .build()
        .map_err(|e| format!("Exit probe unavailable: {e}"))?;
    let response = client
        .get("https://www.cloudflare.com/cdn-cgi/trace")
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Exit verification failed: {e}"))?;
    parse_exit_trace(
        &response
            .text()
            .map_err(|e| format!("Exit response unreadable: {e}"))?,
    )
}

fn publish_exit(
    target: &Mutex<ExitObservation>,
    epoch: &AtomicU64,
    expected: u64,
    observed: ExitObservation,
) -> bool {
    let mut exit = target.lock();
    if epoch.load(Ordering::SeqCst) != expected {
        return false;
    }
    *exit = observed;
    true
}

fn validate_german_profile(config: &serde_json::Value) -> Result<(), String> {
    let relay = config["outbounds"]
        .as_array()
        .and_then(|outbounds| {
            outbounds.iter().find(|outbound| {
                outbound["type"] == "vless"
                    && outbound["server"] == "92.5.127.89"
                    && outbound["server_port"] == 443
            })
        })
        .ok_or("The profile must use the Frankfurt relay at 92.5.127.89:443")?;
    if relay["tls"]["enabled"] != true
        || relay["transport"]["type"] != "ws"
        || relay["transport"]["path"] != "/rbx-tunnel-9f2a"
    {
        return Err(
            "The German profile needs TLS and the working WebSocket route /rbx-tunnel-9f2a".into(),
        );
    }
    if config["route"]["final"] != relay["tag"] || relay["tag"].as_str().map_or(true, str::is_empty)
    {
        return Err("The German relay must be the profile's default outbound route".into());
    }
    let tun = config["inbounds"].as_array().is_some_and(|inbounds| {
        inbounds.iter().any(|inbound| {
            inbound["type"] == "tun"
                && inbound["auto_route"] == true
                && inbound["strict_route"] == true
        })
    });
    if !tun {
        return Err(
            "The German profile needs a TUN interface with automatic and strict routing enabled"
                .into(),
        );
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct DetectedVpn {
    pub engine: String,
    pub display_name: String,
}

pub struct VpnManager {
    operation: Mutex<()>,
    connected: AtomicBool,
    state: Mutex<VpnState>,
    /// True only when THIS app established the verified connection.
    managed: AtomicBool,
    engine: Mutex<String>,
    engine_name: Mutex<String>,
    auto_detected: AtomicBool,
    start_time: Mutex<Option<Instant>>,
    exit: Arc<Mutex<ExitObservation>>,
    exit_epoch: Arc<AtomicU64>,
    exit_in_flight: Arc<AtomicBool>,
    exit_requested: Mutex<Option<Instant>>,
    last_latency: Mutex<Option<u64>>,
    last_error: Mutex<Option<String>>,
    state_reason: Mutex<Option<String>>,
    evidence: Mutex<VpnEvidence>,
    auto_reconnect: AtomicBool,
    user_disconnected: AtomicBool,
    reconnect_engine: Mutex<Option<String>>,
    retry_at: Mutex<Option<Instant>>,
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
            operation: Mutex::new(()),
            connected: AtomicBool::new(false),
            state: Mutex::new(VpnState::Disconnected),
            managed: AtomicBool::new(false),
            engine: Mutex::new("none".to_string()),
            engine_name: Mutex::new("Offline".to_string()),
            auto_detected: AtomicBool::new(false),
            start_time: Mutex::new(None),
            exit: Arc::new(Mutex::new(ExitObservation::default())),
            exit_epoch: Arc::new(AtomicU64::new(0)),
            exit_in_flight: Arc::new(AtomicBool::new(false)),
            exit_requested: Mutex::new(None),
            last_latency: Mutex::new(None),
            last_error: Mutex::new(None),
            state_reason: Mutex::new(None),
            evidence: Mutex::new(VpnEvidence::default()),
            auto_reconnect: AtomicBool::new(true),
            user_disconnected: AtomicBool::new(false),
            reconnect_engine: Mutex::new(None),
            retry_at: Mutex::new(None),
        });

        // Startup: desired state is DISCONNECTED. Only adopt a genuinely
        // verified external connection (marked unmanaged/external).

        // Spawn watchdog thread
        let mgr_clone = Arc::clone(&mgr);
        std::thread::Builder::new()
            .name("vpn-watchdog".into())
            .spawn(move || loop {
                mgr_clone.watchdog_tick();
                std::thread::sleep(Duration::from_secs(5));
            })
            .ok();

        mgr
    }

    /// Deterministic construction for tests: DISCONNECTED, no OS probing,
    /// no watchdog thread.
    #[cfg(test)]
    fn new_isolated() -> Self {
        Self {
            operation: Mutex::new(()),
            connected: AtomicBool::new(false),
            state: Mutex::new(VpnState::Disconnected),
            managed: AtomicBool::new(false),
            engine: Mutex::new("none".to_string()),
            engine_name: Mutex::new("Offline".to_string()),
            auto_detected: AtomicBool::new(false),
            start_time: Mutex::new(None),
            exit: Arc::new(Mutex::new(ExitObservation::default())),
            exit_epoch: Arc::new(AtomicU64::new(0)),
            exit_in_flight: Arc::new(AtomicBool::new(false)),
            exit_requested: Mutex::new(None),
            last_latency: Mutex::new(None),
            last_error: Mutex::new(None),
            state_reason: Mutex::new(None),
            evidence: Mutex::new(VpnEvidence::default()),
            auto_reconnect: AtomicBool::new(true),
            user_disconnected: AtomicBool::new(false),
            reconnect_engine: Mutex::new(None),
            retry_at: Mutex::new(None),
        }
    }

    fn set_state(&self, state: VpnState, reason: Option<String>) {
        if self.vpn_state() == VpnState::Connected && state != VpnState::Connected {
            self.invalidate_exit();
            *self.start_time.lock() = None;
            *self.last_latency.lock() = None;
        }
        *self.state.lock() = state;
        *self.state_reason.lock() = reason;
        self.connected
            .store(state == VpnState::Connected, Ordering::SeqCst);
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

            if procs
                .iter()
                .any(|p| p == "warp-svc.exe" || p == "cloudflare warp.exe")
            {
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
                    if line.contains(target)
                        && (line.contains("Connected") || line.contains("connected"))
                    {
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
                        if let Some(idx) =
                            line.rfind("Dedicated").or_else(|| line.rfind("Loopback"))
                        {
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
            let connected_like = lower.contains("connected")
                || lower.contains("connesso")
                || lower.contains("verbunden");
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

    fn dedicated_tunnel(snap: &EvidenceSnapshot) -> bool {
        let filtered = EvidenceSnapshot {
            netsh_lines: snap
                .netsh_lines
                .iter()
                .filter(|line| {
                    let lower = line.to_ascii_lowercase();
                    lower.contains("sing-box") || lower.split_whitespace().last() == Some("tun0")
                })
                .cloned()
                .collect(),
            ..Default::default()
        };
        Self::snap_has_tunnel(&filtered, None)
    }

    fn verified_engine(snap: &EvidenceSnapshot) -> Option<DetectedVpn> {
        [
            "dedicated",
            "warp",
            "psiphon",
            "wireguard",
            "proton",
            "windscribe",
            "openvpn",
            "tailscale",
            "mullvad",
            "nord",
            "clash",
            "nekobox",
            "v2ray",
        ]
        .iter()
        .find(|engine| Self::evaluate_evidence(engine, snap).connected)
        .map(|engine| DetectedVpn {
            engine: (*engine).into(),
            display_name: if *engine == "dedicated" {
                "Germany · Frankfurt relay".into()
            } else {
                engine.to_uppercase()
            },
        })
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
            (
                "dedicated",
                "Dedicated Relay (sing-box)",
                &["sing-box.exe"],
                None,
            ),
            (
                "warp",
                "Cloudflare WARP",
                &["warp-svc.exe", "cloudflare warp.exe"],
                Some("warp"),
            ),
            (
                "psiphon",
                "Psiphon Tunnel",
                &["psiphon3.exe", "psiphond.exe"],
                Some("psiphon"),
            ),
            (
                "wireguard",
                "WireGuard Tunnel",
                &["wireguard.exe"],
                Some("wireguard"),
            ),
            (
                "proton",
                "Proton VPN",
                &["protonvpn.exe", "protonvpn.service.exe"],
                Some("proton"),
            ),
            (
                "windscribe",
                "Windscribe VPN",
                &["windscribe.exe", "windscribe-service.exe"],
                Some("windscribe"),
            ),
            (
                "openvpn",
                "OpenVPN Tunnel",
                &["openvpn.exe", "openvpnserv.exe"],
                Some("openvpn"),
            ),
            (
                "tailscale",
                "Tailscale Mesh",
                &["tailscale-ipn.exe", "tailscaled.exe"],
                Some("tailscale"),
            ),
            (
                "mullvad",
                "Mullvad VPN",
                &["mullvad-vpn.exe", "mullvad-daemon.exe"],
                Some("mullvad"),
            ),
            (
                "nord",
                "NordVPN",
                &["nordvpn.exe", "nordvpn-service.exe"],
                Some("nord"),
            ),
            (
                "clash",
                "Clash / Mihomo",
                &["clash.exe", "clash-meta.exe", "mihomo.exe"],
                None,
            ),
            (
                "nekobox",
                "NekoBox / NekoRay",
                &["nekobox.exe", "nekoray.exe"],
                None,
            ),
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
                            "warp-cli reports Connected and CloudflareWARP tunnel interface is up."
                                .into()
                        } else if cli && !tunnel {
                            "warp-cli reports Connected but no WARP tunnel interface detected."
                                .into()
                        } else if tunnel && !cli {
                            "WARP tunnel interface present but warp-cli does not report Connected."
                                .into()
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
                        detail:
                            "warp-cli reports Connected and CloudflareWARP tunnel interface is up."
                                .into(),
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
            let tunnel = if *id == "dedicated" {
                Self::dedicated_tunnel(snap)
            } else {
                Self::snap_has_tunnel(snap, *keyword)
            };
            if tunnel {
                return Verification {
                    connected: true,
                    evidence: VpnEvidence {
                        process_running: true,
                        tunnel_detected: true,
                        cli_reports_connected: false,
                        detail: format!(
                            "Process {0} running and matching tunnel interface detected.",
                            procs.join("/")
                        ),
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
        if procs
            .iter()
            .any(|p| p == "psiphon3.exe" || p == "psiphond.exe")
        {
            return Some(DetectedVpn {
                engine: "psiphon".to_string(),
                display_name: "Psiphon Tunnel".to_string(),
            });
        }

        // 4. WireGuard
        if procs.iter().any(|p| p == "wireguard.exe")
            || Self::check_netsh_interface_connected("WireGuard")
        {
            return Some(DetectedVpn {
                engine: "wireguard".to_string(),
                display_name: "WireGuard Tunnel".to_string(),
            });
        }

        // 5. Proton VPN
        if procs
            .iter()
            .any(|p| p == "protonvpn.exe" || p == "protonvpn.service.exe")
        {
            return Some(DetectedVpn {
                engine: "proton".to_string(),
                display_name: "Proton VPN".to_string(),
            });
        }

        // 6. Windscribe
        if procs
            .iter()
            .any(|p| p == "windscribe.exe" || p == "windscribe-service.exe")
        {
            return Some(DetectedVpn {
                engine: "windscribe".to_string(),
                display_name: "Windscribe VPN".to_string(),
            });
        }

        // 7. OpenVPN
        if procs
            .iter()
            .any(|p| p == "openvpn.exe" || p == "openvpnserv.exe")
        {
            return Some(DetectedVpn {
                engine: "openvpn".to_string(),
                display_name: "OpenVPN Tunnel".to_string(),
            });
        }

        // 8. Tailscale
        if procs
            .iter()
            .any(|p| p == "tailscale-ipn.exe" || p == "tailscaled.exe")
        {
            return Some(DetectedVpn {
                engine: "tailscale".to_string(),
                display_name: "Tailscale Mesh".to_string(),
            });
        }

        // 9. Mullvad
        if procs
            .iter()
            .any(|p| p == "mullvad-vpn.exe" || p == "mullvad-daemon.exe")
        {
            return Some(DetectedVpn {
                engine: "mullvad".to_string(),
                display_name: "Mullvad VPN".to_string(),
            });
        }

        // 10. NordVPN
        if procs
            .iter()
            .any(|p| p == "nordvpn.exe" || p == "nordvpn-service.exe")
        {
            return Some(DetectedVpn {
                engine: "nord".to_string(),
                display_name: "NordVPN".to_string(),
            });
        }

        // 11. Clash / Mihomo
        if procs
            .iter()
            .any(|p| p == "clash.exe" || p == "clash-meta.exe" || p == "mihomo.exe")
        {
            return Some(DetectedVpn {
                engine: "clash".to_string(),
                display_name: "Clash / Mihomo".to_string(),
            });
        }

        // 12. NekoBox / NekoRay
        if procs
            .iter()
            .any(|p| p == "nekobox.exe" || p == "nekoray.exe")
        {
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
        let Some(_operation) = self.operation.try_lock() else {
            return;
        };
        match *self.state.lock() {
            VpnState::Connecting | VpnState::Verifying | VpnState::Disconnecting => return,
            _ => {}
        }

        let managed = self.managed.load(Ordering::SeqCst);
        if managed {
            // A connection WE established: re-verify the same engine.
            let engine = self.engine.lock().clone();
            let v = Self::evaluate_evidence(&engine, &Self::collect_snapshot());
            self.set_evidence(v.evidence.clone());
            if v.connected {
                if engine == "dedicated" && !self.exit.lock().germany_verified() {
                    self.set_state(
                        VpnState::Unknown,
                        Some(
                            "Relay interface detected; German exit is not currently verified"
                                .into(),
                        ),
                    );
                    self.refresh_ip_async();
                    return;
                }
                if self.vpn_state() != VpnState::Connected {
                    self.set_state(VpnState::Connected, None);
                    *self.last_error.lock() = None;
                    if self.start_time.lock().is_none() {
                        *self.start_time.lock() = Some(Instant::now());
                    }
                }
            } else if self.vpn_state() == VpnState::Connected {
                let reason = format!("Managed '{engine}' connection lost: {}", v.evidence.detail);
                self.set_state(VpnState::Error, Some(reason.clone()));
                *self.last_error.lock() = Some(reason);
                tracing::warn!(
                    "VPN: {}",
                    self.last_error.lock().as_deref().unwrap_or("lost")
                );
            }
            return;
        }

        // Unmanaged: adopt a genuinely verified external connection, else
        // Unknown (never Connected) when something VPN-shaped is visible.
        let snap = Self::collect_snapshot();
        let v = Self::evaluate_evidence("auto", &snap);
        self.set_evidence(v.evidence.clone());
        if v.connected {
            let detected = Self::verified_engine(&snap);
            let (engine, name) = detected
                .map(|d| (d.engine, d.display_name))
                .unwrap_or_else(|| ("generic".to_string(), "VPN (external)".to_string()));
            let prev = self.engine.lock().clone();
            let was_connected = self.vpn_state() == VpnState::Connected;
            if prev != engine {
                self.invalidate_exit();
            }
            self.set_evidence(Self::evaluate_evidence(&engine, &snap).evidence);
            *self.engine.lock() = engine;
            *self.engine_name.lock() = name.clone();
            self.auto_detected.store(true, Ordering::SeqCst);
            let german_pending =
                self.engine.lock().as_str() == "dedicated" && !self.exit.lock().germany_verified();
            self.set_state(
                if german_pending {
                    VpnState::Unknown
                } else {
                    VpnState::Connected
                },
                Some(
                    if german_pending {
                        "Relay detected; checking German exit"
                    } else {
                        "externally managed connection"
                    }
                    .into(),
                ),
            );
            if !self.user_disconnected.load(Ordering::SeqCst) {
                *self.last_error.lock() = None;
            }
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
        self.refresh_ip_async();
        if let Some(engine) = self.recovery_due(is_elevated() || crate::vpn_helper::available()) {
            tracing::warn!("VPN recovery: {engine} needs verification");
            let _ = self.connect_verified(&engine, Duration::from_secs(10));
        }
    }

    fn recovery_due(&self, administrator: bool) -> Option<String> {
        self.reconnect_engine.lock().clone().filter(|engine| {
            self.auto_reconnect.load(Ordering::SeqCst)
                && !self.user_disconnected.load(Ordering::SeqCst)
                && (engine != "dedicated" || administrator)
                && matches!(
                    self.vpn_state(),
                    VpnState::Disconnected | VpnState::Error | VpnState::Unknown
                )
                && self.retry_at.lock().map_or(true, |at| Instant::now() >= at)
        })
    }

    pub fn get_status(&self) -> VpnStatus {
        let exit = self.exit.lock().clone();
        let engine = self.engine.lock().clone();
        let state = if self.vpn_state() == VpnState::Connected
            && engine == "dedicated"
            && !exit.germany_verified()
        {
            VpnState::Unknown
        } else {
            self.vpn_state()
        };
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
        let fresh = exit
            .checked
            .is_some_and(|at| at.elapsed() <= Duration::from_secs(30));
        let administrator = is_elevated();

        VpnStatus {
            connected,
            state,
            managed: self.managed.load(Ordering::SeqCst),
            engine,
            engine_name: self.engine_name.lock().clone(),
            ip: if fresh {
                exit.ip.clone()
            } else {
                String::new()
            },
            country: if fresh {
                exit.country.clone()
            } else {
                String::new()
            },
            city: String::new(),
            latency_ms: *self.last_latency.lock(),
            uptime_secs,
            auto_reconnect: self.auto_reconnect.load(Ordering::SeqCst),
            last_error: self.last_error.lock().clone().or_else(|| {
                if state == VpnState::Error {
                    self.state_reason.lock().clone()
                } else {
                    None
                }
            }),
            auto_detected: self.auto_detected.load(Ordering::SeqCst),
            process_running: ev.process_running,
            tunnel_detected: ev.tunnel_detected,
            verification_detail: ev.detail,
            germany_verified: connected && exit.germany_verified(),
            warp_verified: connected && fresh && exit.warp && exit.error.is_none(),
            exit_age_secs: exit.checked.map(|at| at.elapsed().as_secs()),
            exit_error: exit.error,
            administrator,
            helper_ready: crate::vpn_helper::available(),
            recovery_active: self.reconnect_engine.lock().as_ref().is_some_and(|engine| {
                engine == self.engine.lock().as_str()
                    && (engine != "dedicated" || administrator || crate::vpn_helper::available())
            }) && self.auto_reconnect.load(Ordering::SeqCst)
                && !self.user_disconnected.load(Ordering::SeqCst),
        }
    }

    /// Request a connection and return success ONLY after verification.
    /// `CONNECTING → VERIFYING → CONNECTED`, else `ERROR` (+ `Err`).
    /// A selected engine or a sent command is never reported as connected.
    pub fn connect(&self, engine: &str) -> Result<VpnStatus, String> {
        self.connect_verified(engine, DEFAULT_CONNECT_TIMEOUT)
    }

    pub fn connect_verified(&self, engine: &str, timeout: Duration) -> Result<VpnStatus, String> {
        let _operation = self
            .operation
            .try_lock()
            .ok_or("Another VPN operation is already running")?;
        let result = self.connect_inner(engine, timeout);
        *self.retry_at.lock() = if result.is_err() {
            Some(Instant::now() + Duration::from_secs(30))
        } else {
            None
        };
        result
    }

    fn connect_inner(&self, engine: &str, timeout: Duration) -> Result<VpnStatus, String> {
        let engine = engine.trim().to_ascii_lowercase();
        if !matches!(engine.as_str(), "auto" | "dedicated" | "warp" | "psiphon") {
            return Err(format!("Engine '{engine}' is read-only. Connect supported engines: dedicated, warp or psiphon."));
        }
        self.user_disconnected.store(false, Ordering::SeqCst);
        self.invalidate_exit();
        *self.last_error.lock() = None;
        self.set_state(
            VpnState::Connecting,
            Some(format!("connecting via {engine}")),
        );
        *self.engine.lock() = engine.clone();

        // "auto": adopt a verified external connection if present, else fall
        // through to the preferred local engines.
        if engine == "auto" {
            *self.reconnect_engine.lock() = None;
            let snapshot = Self::collect_snapshot();
            let v = Self::evaluate_evidence("auto", &snapshot);
            self.set_evidence(v.evidence.clone());
            if v.connected {
                if let Some(detected) = Self::verified_engine(&snapshot) {
                    if matches!(detected.engine.as_str(), "dedicated" | "warp" | "psiphon")
                        && Self::evaluate_evidence(&detected.engine, &snapshot).connected
                    {
                        return self.connect_inner(&detected.engine, timeout);
                    }
                    *self.engine.lock() = detected.engine.clone();
                    *self.engine_name.lock() = detected.display_name.clone();
                    self.auto_detected.store(true, Ordering::SeqCst);
                    self.managed.store(false, Ordering::SeqCst);
                    self.set_state(
                        VpnState::Connected,
                        Some("verified external connection".into()),
                    );
                    *self.start_time.lock() = Some(Instant::now());
                    tracing::info!("VPN: Bound to verified {}", self.engine_name.lock());
                    self.refresh_ip_async();
                    return Ok(self.get_status());
                }
            }
            if Path::new(SINGBOX_EXE).exists() {
                return self.connect_inner("dedicated", timeout);
            }
            return self.connect_inner("warp", timeout);
        }

        // Display name up front (state shows Connecting with engine context).
        *self.engine_name.lock() = match engine.as_str() {
            "dedicated" => "Germany · Frankfurt relay".to_string(),
            "warp" => "Cloudflare WARP".to_string(),
            "psiphon" => "Psiphon Tunnel".to_string(),
            other => format!("VPN ({other})"),
        };

        *self.reconnect_engine.lock() = Some(engine.clone());
        let already_running = Self::evaluate_evidence(&engine, &Self::collect_snapshot()).connected;
        if engine == "dedicated" {
            if let Err(error) = Self::validate_relay_profile() {
                *self.last_error.lock() = Some(error.clone());
                self.set_state(VpnState::Error, Some(error.clone()));
                *self.reconnect_engine.lock() = None;
                return Err(error);
            }
        }
        // Adopting a verified tunnel never kills or restarts it.
        let spawn_result = if already_running {
            Ok(())
        } else {
            match engine.as_str() {
                "dedicated" => Self::spawn_dedicated(),
                "warp" => self.spawn_warp(),
                "psiphon" => self.spawn_psiphon(),
                other => Err(format!(
                    "Unknown engine '{other}'. Supported: auto, dedicated, warp, psiphon."
                )),
            }
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
        if !already_running && matches!(engine.as_str(), "dedicated" | "psiphon") {
            let procs = match engine.as_str() {
                "dedicated" => &["sing-box.exe"][..],
                _ => &["psiphon3.exe", "psiphond.exe"][..],
            };
            std::thread::sleep(Duration::from_millis(1500));
            let mut alive = any_process_running(&Self::get_running_process_names(), procs);
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
        self.set_state(
            VpnState::Verifying,
            Some(format!("verifying {engine} tunnel")),
        );
        let deadline = Instant::now() + timeout;
        let mut last_detail = String::from("no verification result yet");
        while Instant::now() < deadline {
            let v = Self::evaluate_evidence(&engine, &Self::collect_snapshot());
            self.set_evidence(v.evidence.clone());
            last_detail = v.evidence.detail.clone();
            if v.connected {
                if engine == "dedicated" {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    match probe_exit(
                        remaining
                            .min(Duration::from_secs(5))
                            .max(Duration::from_millis(1)),
                    ) {
                        Ok(exit) if exit.germany_verified() => {
                            *self.exit.lock() = exit;
                        }
                        Ok(exit) => {
                            last_detail = format!("Relay interface is up, but the observed exit country is {} instead of Germany", exit.country);
                            *self.exit.lock() = exit;
                            break;
                        }
                        Err(error) => {
                            last_detail = error;
                            break;
                        }
                    }
                }
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

        let reason = format!("Could not verify {engine}: {last_detail}");
        *self.last_error.lock() = Some(reason.clone());
        self.set_state(VpnState::Error, Some(reason.clone()));
        self.managed.store(false, Ordering::SeqCst);
        // Best-effort cleanup of a half-started tunnel.
        if !already_running {
            Self::kill_engine_processes(&engine);
        }
        Err(reason)
    }

    fn validate_relay_profile() -> Result<(), String> {
        let path = if crate::vpn_helper::available() {
            crate::vpn_helper::directory().join("profile.json")
        } else {
            SINGBOX_CONFIG.into()
        };
        let bytes = std::fs::read(&path).map_err(|_| "German relay profile missing".to_string())?;
        let config: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| "German relay profile is not valid JSON")?;
        validate_german_profile(&config)
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
                    let line = detail
                        .lines()
                        .find(|l| !l.trim().is_empty())
                        .unwrap_or(&detail);
                    format!(
                        "sing-box config invalid: {}",
                        line.chars().take(220).collect::<String>()
                    )
                })
            }
            Err(e) => Err(format!("Failed to run sing-box config check: {e}")),
        }
    }

    /// Spawn-only: sends the connect command. Success means the command was
    /// accepted — it says NOTHING about connectivity (verified later).
    fn spawn_dedicated() -> Result<(), String> {
        if cfg!(test) {
            return Err(
                "German relay helper not found in test mode; live network operations are disabled"
                    .into(),
            );
        }
        if crate::vpn_helper::available() {
            Self::validate_relay_profile()?;
            if Self::snap_warp_cli_connected(&Self::collect_snapshot()) {
                return Err("Disconnect local WARP before connecting the German relay to avoid conflicting routes.".into());
            }
            return crate::vpn_helper::run(true);
        }
        if !Path::new(SINGBOX_EXE).exists() {
            return Err(format!("sing-box executable not found at {SINGBOX_EXE}"));
        }
        if !Path::new(SINGBOX_CONFIG).exists() {
            return Err(format!("sing-box config not found at {SINGBOX_CONFIG}"));
        }
        if !is_elevated() {
            return Err(
                "Install the German relay helper on the VPN page once. Windows needs one setup approval for the TUN driver; normal connections do not elevate the macro.".to_string(),
            );
        }
        Self::check_singbox_config()?;
        if Self::snap_warp_cli_connected(&Self::collect_snapshot()) {
            return Err("Local WARP is active. Disconnect it before starting the German TUN relay to avoid competing default routes.".into());
        }

        // Clean up any stale instances first
        Self::stop_dedicated()?;
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
        cmd.spawn()
            .map_err(|e| format!("Failed to run Psiphon: {e}"))?;
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
            "dedicated" => {
                let _ = Self::stop_dedicated();
            }
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
            _ => {}
        }
    }

    pub fn disconnect(&self) -> Result<VpnStatus, String> {
        self.disconnect_verified(DISCONNECT_TIMEOUT)
    }

    /// `DISCONNECTING → DISCONNECTED`, verified by evidence absence. Fails to
    /// `Error` if engine processes persist past the timeout.
    pub fn disconnect_verified(&self, timeout: Duration) -> Result<VpnStatus, String> {
        let _operation = self
            .operation
            .try_lock()
            .ok_or("Another VPN operation is already running")?;
        let engine = self.engine.lock().clone();
        if !matches!(engine.as_str(), "dedicated" | "warp" | "psiphon" | "none") {
            return Err("This VPN is externally managed. Disconnect it using its own app.".into());
        }
        if engine == "dedicated" && !is_elevated() && !crate::vpn_helper::available() {
            return Err("Install the German relay helper to control it without elevating the macro. The current tunnel has been left running.".into());
        }
        self.user_disconnected.store(true, Ordering::SeqCst);
        *self.reconnect_engine.lock() = None;
        *self.retry_at.lock() = None;
        self.set_state(
            VpnState::Disconnecting,
            Some(format!("disconnecting {engine}")),
        );
        if engine == "dedicated" {
            Self::stop_dedicated()?;
        } else {
            Self::kill_engine_processes(&engine);
        }

        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let snap = Self::collect_snapshot();
            // Disconnected when NEITHER this engine's processes NOR any
            // tunnel interface remain.
            if Self::disconnected_evidence(&engine, &snap) {
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

        let reason = format!(
            "Disconnect timeout: '{engine}' processes persist after {}s.",
            timeout.as_secs()
        );
        *self.last_error.lock() = Some(reason.clone());
        self.set_state(VpnState::Error, Some(reason.clone()));
        Err(reason)
    }

    fn disconnected_evidence(engine: &str, snap: &EvidenceSnapshot) -> bool {
        if engine == "dedicated" {
            return !Self::snap_has_proc(snap, &["sing-box.exe"]) && !Self::dedicated_tunnel(snap);
        }
        let (processes, keyword): (&[&str], Option<&str>) = match engine {
            "dedicated" => (&["sing-box.exe"], Some("sing-box")),
            "warp" => (&[], Some("warp")), // WARP's service remains alive when disconnected.
            "psiphon" => (&["psiphon3.exe", "psiphond.exe"], Some("psiphon")),
            _ => (&[], None),
        };
        let cli_disconnected = engine != "warp"
            || snap
                .warp_status
                .as_ref()
                .is_some_and(|s| s.to_ascii_lowercase().contains("disconnected"));
        !Self::snap_has_proc(snap, processes)
            && !Self::snap_has_tunnel(snap, keyword)
            && cli_disconnected
    }

    /// Block until `want_connected` matches verified reality, or timeout.
    /// Side-effect free (never connects/disconnects by itself).
    pub fn wait_for(&self, want_connected: bool, timeout: Duration) -> Result<VpnStatus, String> {
        let deadline = Instant::now() + timeout;
        loop {
            let st = self.get_status();
            if (want_connected && st.state == VpnState::Connected)
                || (!want_connected && st.state == VpnState::Disconnected)
            {
                return Ok(st);
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "VPN wait timeout after {}s: expected {}, observed {} (engine {}, {}).",
                    timeout.as_secs(),
                    if want_connected {
                        "connected"
                    } else {
                        "disconnected"
                    },
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

    fn stop_dedicated() -> Result<(), String> {
        if crate::vpn_helper::available() {
            return crate::vpn_helper::run(false);
        }
        #[cfg(windows)]
        {
            let script = format!(
                "$ErrorActionPreference='Stop'; Get-CimInstance Win32_Process -Filter \"Name='sing-box.exe'\" | Where-Object {{ $_.ExecutablePath -eq '{SINGBOX_EXE}' -and (($_.CommandLine -split '\\s+' | ForEach-Object {{ $_.Trim('\"') }}) -contains '{SINGBOX_CONFIG}') }} | ForEach-Object {{ Stop-Process -Id $_.ProcessId -ErrorAction Stop }}"
            );
            let output = Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", &script])
                .creation_flags(CREATE_NO_WINDOW)
                .output()
                .map_err(|error| format!("Could not stop the German relay: {error}"))?;
            if !output.status.success() {
                return Err("Could not stop the German relay. Administrator rights may be required; unrelated tunnels were left alone.".into());
            }
        }
        Ok(())
    }

    pub fn test_ping(&self) -> PingResult {
        let addr = SocketAddr::from((DEDICATED_SERVER_IP, DEDICATED_SERVER_PORT));
        let mut samples = Vec::new();
        let mut last_error = None;
        for _ in 0..3 {
            let start = Instant::now();
            match std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(1500)) {
                Ok(_) => samples.push(start.elapsed().as_millis() as u64),
                Err(error) => {
                    last_error = Some(error.to_string());
                    break;
                }
            }
        }
        if samples.len() == 3 {
            samples.sort_unstable();
            let ms = samples[1];
            *self.last_latency.lock() = Some(ms);
            PingResult {
                success: true,
                latency_ms: ms,
                target: DEDICATED_RELAY_LABEL.into(),
                error: None,
            }
        } else {
            *self.last_latency.lock() = None;
            PingResult {
                success: false,
                latency_ms: 0,
                target: DEDICATED_RELAY_LABEL.into(),
                error: last_error,
            }
        }
    }

    pub fn reset_network(&self) -> Result<String, String> {
        #[cfg(windows)]
        {
            let mut cmd = Command::new("ipconfig");
            cmd.arg("/flushdns");
            cmd.creation_flags(CREATE_NO_WINDOW);
            let output = cmd
                .output()
                .map_err(|error| format!("DNS flush failed: {error}"))?;
            if !output.status.success() {
                return Err(
                    "Windows refused to flush DNS. Administrator rights may be required.".into(),
                );
            }
        }
        Ok("DNS cache flushed. The active VPN routes were left in place.".to_string())
    }

    pub fn get_logs(&self, max_lines: usize) -> Vec<String> {
        let log = if crate::vpn_helper::available() {
            crate::vpn_helper::directory().join("relay.log")
        } else {
            SINGBOX_LOG.into()
        };
        if let Ok(content) = std::fs::read_to_string(log) {
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
        if self
            .exit_requested
            .lock()
            .is_some_and(|at| at.elapsed() < Duration::from_secs(10))
        {
            return;
        }
        if self.exit_in_flight.swap(true, Ordering::SeqCst) {
            return;
        }
        *self.exit_requested.lock() = Some(Instant::now());
        let target = Arc::clone(&self.exit);
        let epoch = Arc::clone(&self.exit_epoch);
        let expected = epoch.load(Ordering::SeqCst);
        let in_flight = Arc::clone(&self.exit_in_flight);
        std::thread::spawn(move || {
            let observed =
                probe_exit(Duration::from_secs(3)).unwrap_or_else(|error| ExitObservation {
                    error: Some(error),
                    ..Default::default()
                });
            publish_exit(&target, &epoch, expected, observed);
            in_flight.store(false, Ordering::SeqCst);
        });
    }

    fn invalidate_exit(&self) {
        let mut exit = self.exit.lock();
        self.exit_epoch.fetch_add(1, Ordering::SeqCst);
        *exit = ExitObservation::default();
        *self.exit_requested.lock() = None;
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
    fn a_different_vpn_interface_cannot_verify_the_german_relay() {
        let snapshot = snap(
            &["sing-box.exe", "warp-svc.exe"],
            &[WARP_LINE],
            Some("Connected"),
        );
        assert!(!VpnManager::evaluate_evidence("dedicated", &snapshot).connected);
        assert_eq!(
            VpnManager::verified_engine(&snapshot).unwrap().engine,
            "warp"
        );
        let tun0 = snap(&["sing-box.exe"], &["Connected Dedicated tun0"], None);
        assert!(VpnManager::evaluate_evidence("dedicated", &tun0).connected);
        assert!(!VpnManager::disconnected_evidence("dedicated", &tun0));
    }

    #[test]
    fn german_exit_needs_fresh_complete_https_evidence() {
        let mut exit = parse_exit_trace("ip=203.0.113.7\nloc=DE\nwarp=on\n").unwrap();
        assert!(exit.germany_verified());
        assert!(exit.warp);
        exit.checked = Some(Instant::now() - Duration::from_secs(31));
        assert!(!exit.germany_verified());
        assert!(!parse_exit_trace("ip=203.0.113.7\nloc=EG\nwarp=off")
            .unwrap()
            .germany_verified());
        for malformed in [
            "loc=DE",
            "ip=broken\nloc=DE",
            "ip=203.0.113.7",
            "ip=203.0.113.7\nloc=Germany",
        ] {
            assert!(parse_exit_trace(malformed).is_err());
        }
    }

    #[test]
    fn replies_from_a_previous_route_cannot_restore_old_country_evidence() {
        let mgr = VpnManager::new_isolated();
        let previous = mgr.exit_epoch.load(Ordering::SeqCst);
        mgr.invalidate_exit();
        let stale = parse_exit_trace("ip=203.0.113.7\nloc=DE\nwarp=off").unwrap();
        assert!(!publish_exit(&mgr.exit, &mgr.exit_epoch, previous, stale));
        assert!(mgr.get_status().country.is_empty());
        assert!(!mgr.get_status().germany_verified);
    }

    #[test]
    fn a_lost_tunnel_clears_exit_country_latency_and_uptime() {
        let mgr = VpnManager::new_isolated();
        mgr.set_state(VpnState::Connected, None);
        *mgr.exit.lock() = parse_exit_trace("ip=203.0.113.7\nloc=DE\nwarp=off").unwrap();
        *mgr.last_latency.lock() = Some(52);
        *mgr.start_time.lock() = Some(Instant::now());
        assert!(mgr.get_status().germany_verified);
        mgr.set_state(VpnState::Error, Some("Tunnel lost".into()));
        let status = mgr.get_status();
        assert!(!status.connected);
        assert!(!status.germany_verified);
        assert!(status.ip.is_empty());
        assert!(status.country.is_empty());
        assert_eq!(status.latency_ms, None);
        assert_eq!(status.uptime_secs, 0);
    }

    #[test]
    fn an_expired_german_exit_is_not_a_connected_macro_wait_result() {
        let mgr = VpnManager::new_isolated();
        *mgr.engine.lock() = "dedicated".into();
        mgr.set_state(
            VpnState::Connected,
            Some("externally managed connection".into()),
        );
        *mgr.exit.lock() = parse_exit_trace("ip=203.0.113.7\nloc=DE\nwarp=off").unwrap();
        assert!(mgr.get_status().connected);
        assert!(
            mgr.get_status().last_error.is_none(),
            "healthy connection notes are not error alerts"
        );
        mgr.exit.lock().checked = Some(Instant::now() - Duration::from_secs(31));
        let status = mgr.get_status();
        assert_eq!(status.state, VpnState::Unknown);
        assert!(!status.connected);
        assert!(!status.germany_verified);
        assert!(status.country.is_empty());
    }

    #[test]
    fn recovery_preserves_the_request_across_failures_and_respects_cancellation() {
        let mgr = VpnManager::new_isolated();
        *mgr.reconnect_engine.lock() = Some("warp".into());
        mgr.set_state(VpnState::Error, None);
        mgr.managed.store(false, Ordering::SeqCst);
        assert_eq!(mgr.recovery_due(false).as_deref(), Some("warp"));
        *mgr.retry_at.lock() = Some(Instant::now() + Duration::from_secs(30));
        assert_eq!(mgr.recovery_due(false), None);
        *mgr.retry_at.lock() = None;
        mgr.auto_reconnect.store(false, Ordering::SeqCst);
        assert_eq!(mgr.recovery_due(false), None);
        mgr.auto_reconnect.store(true, Ordering::SeqCst);
        mgr.user_disconnected.store(true, Ordering::SeqCst);
        assert_eq!(mgr.recovery_due(false), None);
        mgr.user_disconnected.store(false, Ordering::SeqCst);
        mgr.set_state(VpnState::Connecting, None);
        assert_eq!(mgr.recovery_due(false), None);
        mgr.set_state(VpnState::Error, None);
        *mgr.reconnect_engine.lock() = Some("dedicated".into());
        assert_eq!(mgr.recovery_due(false), None);
        assert_eq!(mgr.recovery_due(true).as_deref(), Some("dedicated"));
    }

    #[test]
    fn overlapping_operations_are_rejected_before_os_input_or_state_changes() {
        let mgr = VpnManager::new_isolated();
        let _busy = mgr.operation.lock();
        assert!(mgr
            .connect_verified("dedicated", Duration::ZERO)
            .unwrap_err()
            .contains("already running"));
        assert!(mgr
            .disconnect_verified(Duration::ZERO)
            .unwrap_err()
            .contains("already running"));
        assert_eq!(mgr.vpn_state(), VpnState::Disconnected);
        assert!(!mgr.user_disconnected.load(Ordering::SeqCst));
    }

    #[test]
    fn the_german_profile_requires_the_working_route_and_tls() {
        let valid = serde_json::json!({
            "inbounds": [{"type":"tun","auto_route":true,"strict_route":true}],
            "outbounds": [{"type":"vless","tag":"relay","server":"92.5.127.89","server_port":443,
                "tls":{"enabled":true},"transport":{"type":"ws","path":"/rbx-tunnel-9f2a"}}],
            "route":{"final":"relay"}
        });
        assert!(validate_german_profile(&valid).is_ok());
        for pointer in ["/outbounds/0/tls/enabled", "/inbounds/0/strict_route"] {
            let mut invalid = valid.clone();
            *invalid.pointer_mut(pointer).unwrap() = serde_json::json!(false);
            assert!(validate_german_profile(&invalid).is_err());
        }
        let mut wrong_route = valid.clone();
        wrong_route["route"]["final"] = serde_json::json!("direct");
        assert!(validate_german_profile(&wrong_route).is_err());
        let mut wrong_path = valid;
        wrong_path["outbounds"][0]["transport"]["path"] = serde_json::json!("/old-path");
        assert!(validate_german_profile(&wrong_path).is_err());
    }

    #[test]
    fn warp_disconnect_uses_cli_and_tunnel_not_service_lifetime() {
        let disconnected = snap(&["warp-svc.exe"], &[], Some("Status update: Disconnected"));
        assert!(VpnManager::disconnected_evidence("warp", &disconnected));
        let connected = snap(
            &["warp-svc.exe"],
            &[WARP_LINE],
            Some("Status update: Connected"),
        );
        assert!(!VpnManager::disconnected_evidence("warp", &connected));
        let external = snap(&[], &[WARP_LINE], None);
        assert!(!VpnManager::disconnected_evidence("external", &external));
    }

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
        let v =
            VpnManager::evaluate_evidence("dedicated", &snap(&["sing-box.exe"], &[TUN_LINE], None));
        assert!(v.connected);
        assert!(v.evidence.process_running);
        assert!(v.evidence.tunnel_detected);
    }

    #[test]
    fn dedicated_process_without_tunnel_is_not_connected() {
        // The core false-Connected bug: process alone must NOT verify.
        let v = VpnManager::evaluate_evidence(
            "dedicated",
            &snap(&["sing-box.exe"], &[WIFI_LINE], None),
        );
        assert!(!v.connected);
        assert!(v.evidence.process_running);
        assert!(!v.evidence.tunnel_detected);
        assert!(v.evidence.detail.contains("no matching tunnel"));
    }

    #[test]
    fn disconnected_interface_line_does_not_count() {
        let v =
            VpnManager::evaluate_evidence("dedicated", &snap(&["sing-box.exe"], &[DIS_LINE], None));
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
        let tun_only = snap(
            &["warp-svc.exe"],
            &[WARP_LINE],
            Some("Status update: Disconnected"),
        );
        let v3 = VpnManager::evaluate_evidence("warp", &tun_only);
        assert!(!v3.connected);
    }

    #[test]
    fn auto_accepts_first_verifiable_engine() {
        let s = snap(
            &["sing-box.exe", "warp-svc.exe"],
            &[TUN_LINE],
            Some("Disconnected"),
        );
        let v = VpnManager::evaluate_evidence("auto", &s);
        assert!(
            v.connected,
            "dedicated verifies even with warp present-but-down"
        );

        let none = snap(&["warp-svc.exe"], &[WIFI_LINE], Some("Disconnected"));
        let v2 = VpnManager::evaluate_evidence("auto", &none);
        assert!(!v2.connected);
    }

    #[test]
    fn unknown_engine_never_verifies() {
        let v =
            VpnManager::evaluate_evidence("fictional", &snap(&["sing-box.exe"], &[TUN_LINE], None));
        assert!(!v.connected);
    }

    #[test]
    fn generic_tunnel_keyword_matches_vpn_adapters() {
        let line = "Connected  Dedicated  VPN Adapter (ProtonVPN)";
        let v = VpnManager::evaluate_evidence("proton", &snap(&["protonvpn.exe"], &[line], None));
        assert!(v.connected);
    }

    #[test]
    fn state_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&VpnState::Connected).unwrap(),
            "\"connected\""
        );
        assert_eq!(
            serde_json::to_string(&VpnState::Verifying).unwrap(),
            "\"verifying\""
        );
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
        for st in [
            VpnState::Disconnected,
            VpnState::Connecting,
            VpnState::Verifying,
            VpnState::Disconnecting,
            VpnState::Error,
            VpnState::Unknown,
        ] {
            assert_ne!(
                st,
                VpnState::Connected,
                "{st:?} must never read as connected"
            );
        }
    }
}
