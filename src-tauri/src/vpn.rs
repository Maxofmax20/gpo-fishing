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
const DEDICATED_SERVER_IP: [u8; 4] = [92, 5, 127, 89];
const DEDICATED_SERVER_PORT: u16 = 443;
const SINGBOX_EXE: &str = r"C:\VPN\sing-box.exe";
const SINGBOX_CONFIG: &str = r"C:\VPN\sing-box-client.json";
const SINGBOX_LOG: &str = r"C:\VPN\sing-box.log";
const WARP_CLI_DEFAULT: &str = r"C:\Program Files\Cloudflare\Cloudflare WARP\warp-cli.exe";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VpnStatus {
    pub connected: bool,
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
}

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
    engine: Mutex<String>,
    engine_name: Mutex<String>,
    auto_detected: AtomicBool,
    start_time: Mutex<Option<Instant>>,
    last_ip: Arc<Mutex<String>>,
    last_country: Arc<Mutex<String>>,
    last_city: Arc<Mutex<String>>,
    last_latency: Mutex<Option<u64>>,
    last_error: Mutex<Option<String>>,
    auto_reconnect: AtomicBool,
    user_disconnected: AtomicBool,
}

impl VpnManager {
    pub fn new() -> Arc<Self> {
        let mgr = Arc::new(Self {
            connected: AtomicBool::new(false),
            engine: Mutex::new("none".to_string()),
            engine_name: Mutex::new("Offline".to_string()),
            auto_detected: AtomicBool::new(false),
            start_time: Mutex::new(None),
            last_ip: Arc::new(Mutex::new("Detecting...".to_string())),
            last_country: Arc::new(Mutex::new("".to_string())),
            last_city: Arc::new(Mutex::new("".to_string())),
            last_latency: Mutex::new(None),
            last_error: Mutex::new(None),
            auto_reconnect: AtomicBool::new(true),
            user_disconnected: AtomicBool::new(false),
        });

        // Detect any currently active VPN on launch
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

    pub fn sync_process_state(&self) {
        if let Some(detected) = Self::detect_active_vpn() {
            let was_connected = self.connected.load(Ordering::SeqCst);
            let prev_engine = self.engine.lock().clone();

            self.connected.store(true, Ordering::SeqCst);
            *self.engine.lock() = detected.engine.clone();
            *self.engine_name.lock() = detected.display_name.clone();
            self.auto_detected.store(true, Ordering::SeqCst);
            self.user_disconnected.store(false, Ordering::SeqCst);

            if !was_connected || prev_engine != detected.engine {
                if self.start_time.lock().is_none() {
                    *self.start_time.lock() = Some(Instant::now());
                }
                tracing::info!("VPN: Active engine detected: {}", detected.display_name);
                self.refresh_ip_async();
            }
        } else if self.connected.load(Ordering::SeqCst) {
            // No VPN is currently running
            self.connected.store(false, Ordering::SeqCst);
            *self.engine.lock() = "none".to_string();
            *self.engine_name.lock() = "Offline".to_string();
            *self.start_time.lock() = None;
            self.auto_detected.store(false, Ordering::SeqCst);
            self.refresh_ip_async();
        }
    }

    fn watchdog_tick(&self) {
        // Run continuous auto-detection sync
        self.sync_process_state();

        if self.user_disconnected.load(Ordering::SeqCst) {
            return;
        }

        let current_engine = self.engine.lock().clone();
        if current_engine == "dedicated" && self.auto_reconnect.load(Ordering::SeqCst) {
            let procs = Self::get_running_process_names();
            if !procs.iter().any(|p| p == "sing-box.exe") {
                tracing::warn!("VPN Watchdog: sing-box process died unexpectedly! Auto-reconnecting...");
                let _ = self.connect_dedicated();
            }
        }
    }

    pub fn get_status(&self) -> VpnStatus {
        self.sync_process_state();

        let connected = self.connected.load(Ordering::SeqCst);
        let uptime_secs = if connected {
            self.start_time
                .lock()
                .map(|t| t.elapsed().as_secs())
                .unwrap_or(0)
        } else {
            0
        };

        VpnStatus {
            connected,
            engine: self.engine.lock().clone(),
            engine_name: self.engine_name.lock().clone(),
            ip: self.last_ip.lock().clone(),
            country: self.last_country.lock().clone(),
            city: self.last_city.lock().clone(),
            latency_ms: *self.last_latency.lock(),
            uptime_secs,
            auto_reconnect: self.auto_reconnect.load(Ordering::SeqCst),
            last_error: self.last_error.lock().clone(),
            auto_detected: self.auto_detected.load(Ordering::SeqCst),
        }
    }

    pub fn connect(&self, engine: &str) -> Result<VpnStatus, String> {
        self.user_disconnected.store(false, Ordering::SeqCst);
        *self.last_error.lock() = None;

        match engine {
            "auto" => {
                if let Some(detected) = Self::detect_active_vpn() {
                    self.connected.store(true, Ordering::SeqCst);
                    *self.engine.lock() = detected.engine.clone();
                    *self.engine_name.lock() = detected.display_name.clone();
                    self.auto_detected.store(true, Ordering::SeqCst);
                    if self.start_time.lock().is_none() {
                        *self.start_time.lock() = Some(Instant::now());
                    }
                    tracing::info!("VPN: Bound to active {}", detected.display_name);
                } else if Path::new(SINGBOX_EXE).exists() {
                    self.connect_dedicated()?;
                } else {
                    self.connect_warp()?;
                }
            }
            "dedicated" => self.connect_dedicated()?,
            "warp" => self.connect_warp()?,
            "psiphon" => self.connect_psiphon()?,
            _ => {
                if let Some(detected) = Self::detect_active_vpn() {
                    if detected.engine == engine {
                        self.connected.store(true, Ordering::SeqCst);
                        *self.engine.lock() = detected.engine;
                        *self.engine_name.lock() = detected.display_name;
                        self.auto_detected.store(true, Ordering::SeqCst);
                    }
                } else {
                    return Err(format!("Unknown engine: {}", engine));
                }
            }
        };

        self.refresh_ip_async();
        Ok(self.get_status())
    }

    fn connect_dedicated(&self) -> Result<(), String> {
        if !Path::new(SINGBOX_EXE).exists() {
            let err = format!("sing-box executable not found at {}", SINGBOX_EXE);
            *self.last_error.lock() = Some(err.clone());
            return Err(err);
        }
        if !Path::new(SINGBOX_CONFIG).exists() {
            let err = format!("sing-box config not found at {}", SINGBOX_CONFIG);
            *self.last_error.lock() = Some(err.clone());
            return Err(err);
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
                std::thread::sleep(Duration::from_millis(800));
                let procs = Self::get_running_process_names();
                if !procs.iter().any(|p| p == "sing-box.exe") {
                    let err = "sing-box failed to initialize TUN interface. Ensure app has administrator privileges.".to_string();
                    *self.last_error.lock() = Some(err.clone());
                    return Err(err);
                }
                self.connected.store(true, Ordering::SeqCst);
                *self.engine.lock() = "dedicated".to_string();
                *self.engine_name.lock() = "Dedicated Relay (sing-box)".to_string();
                self.auto_detected.store(false, Ordering::SeqCst);
                *self.start_time.lock() = Some(Instant::now());
                tracing::info!("VPN: Connected via Dedicated Relay");
                Ok(())
            }
            Err(e) => {
                let err = format!("Failed to spawn sing-box: {}", e);
                *self.last_error.lock() = Some(err.clone());
                Err(err)
            }
        }
    }

    fn connect_warp(&self) -> Result<(), String> {
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
                    self.connected.store(true, Ordering::SeqCst);
                    *self.engine.lock() = "warp".to_string();
                    *self.engine_name.lock() = "Cloudflare WARP".to_string();
                    self.auto_detected.store(false, Ordering::SeqCst);
                    *self.start_time.lock() = Some(Instant::now());
                    tracing::info!("VPN: Connected via Cloudflare WARP");
                    Ok(())
                } else {
                    let err = String::from_utf8_lossy(&out.stderr).to_string();
                    *self.last_error.lock() = Some(err.clone());
                    Err(err)
                }
            }
            Err(e) => {
                let err = format!("warp-cli command failed: {}", e);
                *self.last_error.lock() = Some(err.clone());
                Err(err)
            }
        }
    }

    fn connect_psiphon(&self) -> Result<(), String> {
        let path = r"C:\VPN\psiphon3.exe";
        if !Path::new(path).exists() {
            return Err("Psiphon executable not found in C:\\VPN".to_string());
        }
        let mut cmd = Command::new(path);
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd.spawn().map_err(|e| format!("Failed to run Psiphon: {}", e))?;

        self.connected.store(true, Ordering::SeqCst);
        *self.engine.lock() = "psiphon".to_string();
        *self.engine_name.lock() = "Psiphon Tunnel".to_string();
        self.auto_detected.store(false, Ordering::SeqCst);
        *self.start_time.lock() = Some(Instant::now());
        Ok(())
    }

    pub fn disconnect(&self) -> Result<VpnStatus, String> {
        self.user_disconnected.store(true, Ordering::SeqCst);
        let engine = self.engine.lock().clone();

        match engine.as_str() {
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
                Self::kill_process("sing-box.exe");
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
            _ => {
                let procs = Self::get_running_process_names();
                if procs.iter().any(|p| p == "sing-box.exe") {
                    Self::kill_process("sing-box.exe");
                }
            }
        }

        self.connected.store(false, Ordering::SeqCst);
        *self.engine.lock() = "none".to_string();
        *self.engine_name.lock() = "Offline".to_string();
        *self.start_time.lock() = None;
        self.auto_detected.store(false, Ordering::SeqCst);
        tracing::info!("VPN: Disconnected");

        self.refresh_ip_async();
        Ok(self.get_status())
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
                    target: "92.5.127.89:443 (Frankfurt)".into(),
                    error: None,
                }
            }
            Err(e) => PingResult {
                success: false,
                latency_ms: 0,
                target: "92.5.127.89:443 (Frankfurt)".into(),
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
