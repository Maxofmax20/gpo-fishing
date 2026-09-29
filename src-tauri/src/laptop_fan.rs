use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

const GUID_QUIET: &str = "16edbccd-dee9-4ec4-ace5-2f0b5f2a8975";
const GUID_BALANCE: &str = "85d583c5-cf2e-4197-80fd-3789a227a72c";
const GUID_PERFORMANCE: &str = "52521609-efc9-4268-b9ba-67dea73f18b2";

static AUTO_TURBO_ENABLED: AtomicBool = AtomicBool::new(true);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FanStatus {
    pub success: bool,
    pub current_mode: String,        // "quiet", "balance", "performance", "turbo"
    pub mode_label: String,          // "Quiet (Silent)", "Balance (Auto)", "Performance (High)", "Turbo (100% Max)"
    pub fan_level_pct: u32,          // 25, 50, 75, 100
    pub cpu_temp: Option<f32>,       // in Celsius
    pub gpu_temp: Option<u32>,       // in Celsius
    pub gpu_power: Option<f32>,      // in Watts
    pub gpu_util: Option<u32>,       // in %
    pub est_fan_rpm: String,         // e.g. "~1,800 RPM", "~2,800 RPM", "~4,200 RPM", "~5,200 RPM (Max)"
    pub auto_turbo: bool,            // auto turbo during macro active
    pub is_turbo_active: bool,
    pub error: Option<String>,
    pub updated_at: String,
}

impl Default for FanStatus {
    fn default() -> Self {
        Self {
            success: true,
            current_mode: "balance".to_string(),
            mode_label: "Balance (Auto)".to_string(),
            fan_level_pct: 50,
            cpu_temp: None,
            gpu_temp: None,
            gpu_power: None,
            gpu_util: None,
            est_fan_rpm: "~2,800 RPM".to_string(),
            auto_turbo: true,
            is_turbo_active: false,
            error: None,
            updated_at: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct FanPersistState {
    pub auto_turbo: bool,
    pub last_mode: String,
    pub prev_mode_before_bot: Option<String>,
}

pub fn get_data_dir() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("gpo-autofish")
}

fn config_path() -> PathBuf {
    get_data_dir().join("fan_control_config.json")
}

fn load_persist_state() -> FanPersistState {
    let p = config_path();
    if p.is_file() {
        if let Ok(c) = fs::read_to_string(&p) {
            if let Ok(s) = serde_json::from_str::<FanPersistState>(&c) {
                return s;
            }
        }
    }
    FanPersistState {
        auto_turbo: true,
        last_mode: "balance".to_string(),
        prev_mode_before_bot: None,
    }
}

fn save_persist_state(s: &FanPersistState) {
    let dir = get_data_dir();
    let _ = fs::create_dir_all(&dir);
    if let Ok(json) = serde_json::to_string_pretty(s) {
        let _ = fs::write(config_path(), json);
    }
}

struct CachedTelemetry {
    status: FanStatus,
    last_fetched: Instant,
}

static TELEMETRY_CACHE: RwLock<Option<CachedTelemetry>> = RwLock::new(None);
static IS_TURBO_FLAG: AtomicBool = AtomicBool::new(false);

fn query_gpu_stats() -> (Option<u32>, Option<f32>, Option<u32>) {
    #[cfg(windows)]
    {
        let out = Command::new("nvidia-smi")
            .args(["--query-gpu=temperature.gpu,power.draw,utilization.gpu", "--format=csv,noheader,nounits"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();

        if let Ok(res) = out {
            if res.status.success() {
                let s = String::from_utf8_lossy(&res.stdout);
                let parts: Vec<&str> = s.split(',').map(|p| p.trim()).collect();
                let temp = parts.first().and_then(|t| t.parse::<u32>().ok());
                let power = parts.get(1).and_then(|p| p.parse::<f32>().ok());
                let util = parts.get(2).and_then(|u| u.parse::<u32>().ok());
                return (temp, power, util);
            }
        }
    }
    (None, None, None)
}

fn query_cpu_temp() -> Option<f32> {
    #[cfg(windows)]
    {
        let out = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "$t = (Get-CimInstance -ClassName Win32_PerfFormattedData_Counters_ThermalZoneInformation -ErrorAction SilentlyContinue | Select-Object -First 1).Temperature; if ($t -and $t -gt 250) { [math]::Round($t - 273.15) }",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output();

        if let Ok(res) = out {
            if res.status.success() {
                let s = String::from_utf8_lossy(&res.stdout).trim().to_string();
                if let Ok(celsius) = s.parse::<f32>() {
                    if celsius >= 15.0 && celsius <= 115.0 {
                        return Some(celsius);
                    }
                }
            }
        }
    }
    None
}

fn query_active_mode() -> String {
    if IS_TURBO_FLAG.load(Ordering::Relaxed) {
        return "turbo".to_string();
    }

    #[cfg(windows)]
    {
        let out = Command::new("powercfg")
            .arg("/getactivescheme")
            .creation_flags(CREATE_NO_WINDOW)
            .output();

        if let Ok(res) = out {
            let s = String::from_utf8_lossy(&res.stdout).to_lowercase();
            if s.contains("16edbccd") || s.contains("quiet") {
                return "quiet".to_string();
            } else if s.contains("52521609") || s.contains("performance") {
                return "performance".to_string();
            } else if s.contains("85d583c5") || s.contains("balance") {
                return "balance".to_string();
            }
        }
    }
    "balance".to_string()
}

pub fn get_fan_status() -> FanStatus {
    {
        let cache = TELEMETRY_CACHE.read();
        if let Some(ref c) = *cache {
            if c.last_fetched.elapsed() < Duration::from_millis(2000) {
                return c.status.clone();
            }
        }
    }

    let mode = query_active_mode();
    let (gpu_temp, gpu_power, gpu_util) = query_gpu_stats();
    let cpu_temp = query_cpu_temp();
    let persist = load_persist_state();
    let is_turbo = mode == "turbo" || IS_TURBO_FLAG.load(Ordering::Relaxed);

    let (mode_label, fan_pct, est_rpm) = match mode.as_str() {
        "quiet" => ("Quiet (Silent)", 25, "~1,800 RPM"),
        "performance" => ("Performance (High)", 75, "~4,200 RPM"),
        "turbo" => ("Turbo (100% Max Fan)", 100, "~5,200 RPM (Full Speed)"),
        _ => ("Balance (Auto)", 50, "~2,800 RPM"),
    };

    let status = FanStatus {
        success: true,
        current_mode: mode,
        mode_label: mode_label.to_string(),
        fan_level_pct: fan_pct,
        cpu_temp,
        gpu_temp,
        gpu_power,
        gpu_util,
        est_fan_rpm: est_rpm.to_string(),
        auto_turbo: persist.auto_turbo,
        is_turbo_active: is_turbo,
        error: None,
        updated_at: chrono_now_iso(),
    };

    *TELEMETRY_CACHE.write() = Some(CachedTelemetry {
        status: status.clone(),
        last_fetched: Instant::now(),
    });

    status
}

pub fn set_fan_mode(target: &str) -> FanStatus {
    let mode = match target.to_lowercase().as_str() {
        "quiet" | "silent" | "low" => "quiet",
        "performance" | "perf" | "high" => "performance",
        "turbo" | "max" | "extreme" => "turbo",
        _ => "balance",
    };

    let is_turbo = mode == "turbo";
    IS_TURBO_FLAG.store(is_turbo, Ordering::SeqCst);

    let guid = match mode {
        "quiet" => GUID_QUIET,
        "performance" | "turbo" => GUID_PERFORMANCE,
        _ => GUID_BALANCE,
    };

    #[cfg(windows)]
    {
        // 1. Activate Windows Power Scheme for Lenovo EC
        let _ = Command::new("powercfg")
            .args(["/setactive", guid])
            .creation_flags(CREATE_NO_WINDOW)
            .status();

        // 2. Dispatch background WMI command for Lenovo Gamezone Cooling
        let turbo_val = if is_turbo { 1 } else { 0 };
        let smart_fan_val = match mode {
            "quiet" => 1,
            "performance" | "turbo" => 3,
            _ => 2,
        };

        std::thread::spawn(move || {
            let cmd = format!(
                r#"try {{
                    Invoke-CimMethod -Namespace 'root\wmi' -ClassName 'LENOVO_GAMEZONE_DATA' -MethodName 'SetFanCooling' -Arguments @{{ Data = {turbo_val} }} -ErrorAction SilentlyContinue | Out-Null
                }} catch {{}}
                try {{
                    Invoke-CimMethod -Namespace 'root\wmi' -ClassName 'LENOVO_GAMEZONE_DATA' -MethodName 'SetSmartFanMode' -Arguments @{{ Data = {smart_fan_val} }} -ErrorAction SilentlyContinue | Out-Null
                }} catch {{}}"#
            );
            let _ = Command::new("powershell")
                .args(["-NoProfile", "-Command", &cmd])
                .creation_flags(CREATE_NO_WINDOW)
                .status();
        });
    }

    let mut persist = load_persist_state();
    persist.last_mode = mode.to_string();
    save_persist_state(&persist);

    // Invalidate cache immediately so new mode takes effect
    *TELEMETRY_CACHE.write() = None;
    get_fan_status()
}

pub fn set_fan_percentage(pct: u32) -> FanStatus {
    let mode = if pct <= 33 {
        "quiet"
    } else if pct <= 66 {
        "balance"
    } else if pct <= 89 {
        "performance"
    } else {
        "turbo"
    };
    set_fan_mode(mode)
}

pub fn set_auto_turbo(enabled: bool) -> FanStatus {
    AUTO_TURBO_ENABLED.store(enabled, Ordering::SeqCst);
    let mut persist = load_persist_state();
    persist.auto_turbo = enabled;
    save_persist_state(&persist);

    *TELEMETRY_CACHE.write() = None;
    get_fan_status()
}

pub fn on_bot_state_changed(is_active: bool) {
    let persist = load_persist_state();
    if !persist.auto_turbo {
        return;
    }

    if is_active {
        // Macro started: save current mode and ramp up to Turbo/Performance
        let current = query_active_mode();
        let mut p = persist.clone();
        if current != "turbo" && current != "performance" {
            p.prev_mode_before_bot = Some(current);
            save_persist_state(&p);
        }
        let _ = set_fan_mode("turbo");
    } else {
        // Macro stopped: restore previous mode if saved, else return to Balance
        let prev = persist.prev_mode_before_bot.as_deref().unwrap_or("balance");
        let _ = set_fan_mode(prev);
        let mut p = persist.clone();
        p.prev_mode_before_bot = None;
        save_persist_state(&p);
    }
}

fn chrono_now_iso() -> String {
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{now}")
}
