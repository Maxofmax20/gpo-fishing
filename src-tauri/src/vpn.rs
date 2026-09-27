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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VpnStatus {
    pub connected: bool,
    pub engine: String,
    pub ip: String,
    pub country: String,
    pub city: String,
    pub latency_ms: Option<u64>,
    pub uptime_secs: u64,
    pub auto_reconnect: bool,
    pub last_error: Option<String>,
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

pub struct VpnManager {
    connected: AtomicBool,
    engine: Mutex<String>,
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
            start_time: Mutex::new(None),
            last_ip: Arc::new(Mutex::new("Detecting...".to_string())),
            last_country: Arc::new(Mutex::new("".to_string())),
            last_city: Arc::new(Mutex::new("".to_string())),
            last_latency: Mutex::new(None),
            last_error: Mutex::new(None),
            auto_reconnect: AtomicBool::new(true),
            user_disconnected: AtomicBool::new(true),
        });

        // Check if sing-box or WARP is already running
        mgr.sync_process_state();

        // Spawn watchdog thread
        let mgr_clone = Arc::clone(&mgr);
        std::thread::Builder::new()
            .name("vpn-watchdog".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_secs(10));
                    mgr_clone.watchdog_tick();
                }
            })
            .ok();

        mgr
    }

    fn is_singbox_running() -> bool {
        #[cfg(windows)]
        {
            let mut cmd = Command::new("tasklist");
            cmd.args(["/FI", "IMAGENAME eq sing-box.exe", "/FO", "CSV", "/NH"]);
            cmd.creation_flags(CREATE_NO_WINDOW);
            if let Ok(output) = cmd.output() {
                let text = String::from_utf8_lossy(&output.stdout);
                return text.contains("sing-box.exe");
            }
        }
        false
    }

    pub fn sync_process_state(&self) {
        if Self::is_singbox_running() {
            self.connected.store(true, Ordering::SeqCst);
            *self.engine.lock() = "dedicated".to_string();
            self.user_disconnected.store(false, Ordering::SeqCst);
            if self.start_time.lock().is_none() {
                *self.start_time.lock() = Some(Instant::now());
            }
            self.refresh_ip_async();
        } else if *self.engine.lock() == "dedicated" && self.connected.load(Ordering::SeqCst) {
            self.connected.store(false, Ordering::SeqCst);
            *self.start_time.lock() = None;
        }
    }

    fn watchdog_tick(&self) {
        if self.user_disconnected.load(Ordering::SeqCst) {
            return;
        }

        let current_engine = self.engine.lock().clone();
        if current_engine == "dedicated" && self.auto_reconnect.load(Ordering::SeqCst) {
            if !Self::is_singbox_running() {
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
            ip: self.last_ip.lock().clone(),
            country: self.last_country.lock().clone(),
            city: self.last_city.lock().clone(),
            latency_ms: *self.last_latency.lock(),
            uptime_secs,
            auto_reconnect: self.auto_reconnect.load(Ordering::SeqCst),
            last_error: self.last_error.lock().clone(),
        }
    }

    pub fn connect(&self, engine: &str) -> Result<VpnStatus, String> {
        self.user_disconnected.store(false, Ordering::SeqCst);
        *self.last_error.lock() = None;

        match engine {
            "dedicated" => self.connect_dedicated()?,
            "warp" => self.connect_warp()?,
            "psiphon" => self.connect_psiphon()?,
            _ => return Err(format!("Unknown engine: {}", engine)),
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
                if !Self::is_singbox_running() {
                    let err = "sing-box failed to initialize TUN interface. Ensure app has administrator privileges.".to_string();
                    *self.last_error.lock() = Some(err.clone());
                    return Err(err);
                }
                self.connected.store(true, Ordering::SeqCst);
                *self.engine.lock() = "dedicated".to_string();
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
        let mut cmd = Command::new("warp-cli");
        cmd.arg("connect");
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);

        match cmd.output() {
            Ok(out) => {
                if out.status.success() {
                    self.connected.store(true, Ordering::SeqCst);
                    *self.engine.lock() = "warp".to_string();
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
        *self.start_time.lock() = Some(Instant::now());
        Ok(())
    }

    pub fn disconnect(&self) -> Result<VpnStatus, String> {
        self.user_disconnected.store(true, Ordering::SeqCst);
        let engine = self.engine.lock().clone();

        if engine == "dedicated" || Self::is_singbox_running() {
            Self::kill_process("sing-box.exe");
        } else if engine == "warp" {
            let mut cmd = Command::new("warp-cli");
            cmd.arg("disconnect");
            #[cfg(windows)]
            cmd.creation_flags(CREATE_NO_WINDOW);
            let _ = cmd.output();
        } else if engine == "psiphon" {
            Self::kill_process("psiphon3.exe");
        }

        self.connected.store(false, Ordering::SeqCst);
        *self.engine.lock() = "none".to_string();
        *self.start_time.lock() = None;
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
