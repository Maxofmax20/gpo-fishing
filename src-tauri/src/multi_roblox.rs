use std::collections::HashMap;
use std::fs::File;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use windows::core::{BOOL, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, OpenProcess, TerminateProcess, PROCESS_TERMINATE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible,
    SetForegroundWindow, ShowWindow, SW_RESTORE,
};

const FILE_SHARE_READ: u32 = 1;
const FILE_SHARE_WRITE: u32 = 2;
const FILE_SHARE_DELETE: u32 = 4;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RobloxInstanceInfo {
    pub pid: u32,
    pub hwnd: Option<isize>,
    pub user_id: Option<String>,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub universe_id: Option<String>,
    pub game_name: Option<String>,
    pub is_target: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiRobloxStatus {
    pub enabled: bool,
    pub mutex_locked: bool,
    pub cookie_locked: bool,
    pub instances_count: usize,
    pub instances: Vec<RobloxInstanceInfo>,
    pub target_pid: Option<u32>,
}

#[derive(Default)]
struct UserCacheEntry {
    username: String,
    display_name: String,
    avatar_url: Option<String>,
}

#[derive(Default)]
struct GameCacheEntry {
    name: String,
}

pub struct MultiRobloxManager {
    enabled: AtomicBool,
    mutex_handle: Mutex<Option<usize>>,
    event_handle: Mutex<Option<usize>>,
    cookie_lock: Mutex<Option<File>>,
    target_pid: Mutex<Option<u32>>,
    user_cache: Mutex<HashMap<String, UserCacheEntry>>,
    game_cache: Mutex<HashMap<String, GameCacheEntry>>,
    last_instances: Mutex<Vec<RobloxInstanceInfo>>,
    last_scan: Mutex<Instant>,
}

impl MultiRobloxManager {
    pub fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            mutex_handle: Mutex::new(None),
            event_handle: Mutex::new(None),
            cookie_lock: Mutex::new(None),
            target_pid: Mutex::new(None),
            user_cache: Mutex::new(HashMap::new()),
            game_cache: Mutex::new(HashMap::new()),
            last_instances: Mutex::new(Vec::new()),
            last_scan: Mutex::new(Instant::now() - Duration::from_secs(10)),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    pub fn set_enabled(&self, enable: bool) -> Result<MultiRobloxStatus, String> {
        if enable {
            self.acquire_locks()?;
            self.enabled.store(true, Ordering::SeqCst);
            tracing::info!("Multi-Roblox enabled: claimed singleton locks and cookie protections");
        } else {
            self.release_locks();
            self.enabled.store(false, Ordering::SeqCst);
            tracing::info!("Multi-Roblox disabled: released singleton locks");
        }
        Ok(self.get_status())
    }

    fn acquire_locks(&self) -> Result<(), String> {
        unsafe {
            // 1. Claim ROBLOX_singletonMutex
            let mut m_guard = self.mutex_handle.lock();
            if m_guard.is_none() {
                let m_name = wide("ROBLOX_singletonMutex");
                match CreateMutexW(None, true, PCWSTR(m_name.as_ptr())) {
                    Ok(handle) => {
                        *m_guard = Some(handle.0 as usize);
                        tracing::info!("Acquired ROBLOX_singletonMutex handle");
                    }
                    Err(e) => {
                        tracing::warn!("Failed to create ROBLOX_singletonMutex: {e}");
                    }
                }
            }

            // 2. Claim ROBLOX_singletonEvent
            let mut e_guard = self.event_handle.lock();
            if e_guard.is_none() {
                let e_name = wide("ROBLOX_singletonEvent");
                match CreateEventW(None, true, false, PCWSTR(e_name.as_ptr())) {
                    Ok(handle) => {
                        *e_guard = Some(handle.0 as usize);
                        tracing::info!("Acquired ROBLOX_singletonEvent handle");
                    }
                    Err(e) => {
                        tracing::warn!("Failed to create ROBLOX_singletonEvent: {e}");
                    }
                }
            }
        }

        // 3. Lock RobloxCookies.dat (Fix error 773 / multi-account cookie collision)
        let mut c_guard = self.cookie_lock.lock();
        if c_guard.is_none() {
            if let Some(local_appdata) = dirs::data_local_dir() {
                let cookie_path = local_appdata.join("Roblox").join("LocalStorage").join("RobloxCookies.dat");
                if cookie_path.exists() {
                    // Open with read-only share mode to prevent Roblox from overwriting cookies when switching instances
                    match std::fs::OpenOptions::new()
                        .read(true)
                        .share_mode(FILE_SHARE_READ)
                        .open(&cookie_path)
                    {
                        Ok(file) => {
                            *c_guard = Some(file);
                            tracing::info!("Applied Error 773 cookie lock on {:?}", cookie_path);
                        }
                        Err(err) => {
                            tracing::warn!("Could not lock RobloxCookies.dat (might be open): {err}");
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn release_locks(&self) {
        unsafe {
            let mut m_guard = self.mutex_handle.lock();
            if let Some(h) = m_guard.take() {
                let _ = CloseHandle(HANDLE(h as *mut _));
            }

            let mut e_guard = self.event_handle.lock();
            if let Some(h) = e_guard.take() {
                let _ = CloseHandle(HANDLE(h as *mut _));
            }
        }

        let mut c_guard = self.cookie_lock.lock();
        *c_guard = None;
    }

    pub fn get_status(&self) -> MultiRobloxStatus {
        let enabled = self.enabled.load(Ordering::SeqCst);
        let mutex_locked = self.mutex_handle.lock().is_some();
        let cookie_locked = self.cookie_lock.lock().is_some();
        let instances = self.list_instances();
        let target_pid = *self.target_pid.lock();

        MultiRobloxStatus {
            enabled,
            mutex_locked,
            cookie_locked,
            instances_count: instances.len(),
            instances,
            target_pid,
        }
    }

    pub fn list_instances(&self) -> Vec<RobloxInstanceInfo> {
        let mut last_scan = self.last_scan.lock();
        let mut last_instances = self.last_instances.lock();

        // Throttle scans to every 1.5 seconds unless empty
        if !last_instances.is_empty() && last_scan.elapsed() < Duration::from_millis(1500) {
            let current_target = *self.target_pid.lock();
            return last_instances
                .iter()
                .map(|inst| {
                    let mut i = inst.clone();
                    i.is_target = current_target == Some(i.pid);
                    i
                })
                .collect();
        }

        *last_scan = Instant::now();
        let pids = enumerate_roblox_pids();
        let target = *self.target_pid.lock();

        // If target pid exited, reset it
        if let Some(t) = target {
            if !pids.contains(&t) {
                *self.target_pid.lock() = None;
            }
        }

        let mut instances = Vec::new();
        for pid in pids {
            let hwnd = find_hwnd_for_pid(pid);
            let mut info = RobloxInstanceInfo {
                pid,
                hwnd: hwnd.map(|h| h.0 as isize),
                user_id: None,
                username: None,
                display_name: None,
                avatar_url: None,
                universe_id: None,
                game_name: None,
                is_target: target == Some(pid),
            };

            // Read log details if available
            if let Some((user_id, universe_id)) = find_user_and_universe_from_logs(pid) {
                info.user_id = Some(user_id.clone());
                info.universe_id = Some(universe_id.clone());

                // Resolve user info from cache or API
                if let Some(user_entry) = self.resolve_user(&user_id) {
                    info.username = Some(user_entry.username);
                    info.display_name = Some(user_entry.display_name);
                    info.avatar_url = user_entry.avatar_url;
                }

                // Resolve universe info from cache or API
                if let Some(game_name) = self.resolve_game(&universe_id) {
                    info.game_name = Some(game_name);
                }
            }

            instances.push(info);
        }

        *last_instances = instances.clone();
        instances
    }

    pub fn set_target_pid(&self, pid: Option<u32>) {
        *self.target_pid.lock() = pid;
    }

    pub fn get_target_pid(&self) -> Option<u32> {
        *self.target_pid.lock()
    }

    pub fn focus_instance(&self, pid: u32) -> Result<(), String> {
        let hwnd = find_hwnd_for_pid(pid).ok_or_else(|| format!("No window found for PID {pid}"))?;
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(hwnd);
        }
        Ok(())
    }

    pub fn kill_instance(&self, pid: u32) -> Result<(), String> {
        unsafe {
            let handle = OpenProcess(PROCESS_TERMINATE, false, pid)
                .map_err(|e| format!("Could not open process {pid}: {e}"))?;
            let res = TerminateProcess(handle, 1);
            let _ = CloseHandle(handle);
            res.map_err(|e| format!("Could not terminate process {pid}: {e}"))?;
        }
        // Force refresh cache
        let mut last_scan = self.last_scan.lock();
        *last_scan = Instant::now() - Duration::from_secs(10);
        Ok(())
    }

    pub fn kill_all(&self) -> Result<usize, String> {
        let pids = enumerate_roblox_pids();
        let mut killed = 0;
        for pid in pids {
            if self.kill_instance(pid).is_ok() {
                killed += 1;
            }
        }
        Ok(killed)
    }

    fn resolve_user(&self, user_id: &str) -> Option<UserCacheEntry> {
        {
            let cache = self.user_cache.lock();
            if let Some(entry) = cache.get(user_id) {
                return Some(UserCacheEntry {
                    username: entry.username.clone(),
                    display_name: entry.display_name.clone(),
                    avatar_url: entry.avatar_url.clone(),
                });
            }
        }

        // Fetch from Roblox API
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .ok()?;

        let user_url = format!("https://users.roblox.com/v1/users/{user_id}");
        let user_resp = client.get(&user_url).send().ok()?.json::<serde_json::Value>().ok()?;

        let username = user_resp.get("name")?.as_str()?.to_string();
        let display_name = user_resp
            .get("displayName")
            .and_then(|v| v.as_str())
            .unwrap_or(&username)
            .to_string();

        let avatar_url = {
            let thumb_url = format!(
                "https://thumbnails.roblox.com/v1/users/avatar-headshot?size=150x150&format=png&userIds={user_id}"
            );
            client
                .get(&thumb_url)
                .send()
                .ok()
                .and_then(|r| r.json::<serde_json::Value>().ok())
                .and_then(|json| {
                    json.get("data")?
                        .get(0)?
                        .get("imageUrl")?
                        .as_str()
                        .map(|s| s.to_string())
                })
        };

        let entry = UserCacheEntry {
            username,
            display_name,
            avatar_url,
        };

        let res = UserCacheEntry {
            username: entry.username.clone(),
            display_name: entry.display_name.clone(),
            avatar_url: entry.avatar_url.clone(),
        };

        self.user_cache.lock().insert(user_id.to_string(), entry);
        Some(res)
    }

    fn resolve_game(&self, universe_id: &str) -> Option<String> {
        {
            let cache = self.game_cache.lock();
            if let Some(entry) = cache.get(universe_id) {
                return Some(entry.name.clone());
            }
        }

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .ok()?;

        let url = format!("https://games.roblox.com/v1/games?universeIds={universe_id}");
        let resp = client.get(&url).send().ok()?.json::<serde_json::Value>().ok()?;

        let name = resp
            .get("data")?
            .get(0)?
            .get("name")?
            .as_str()?
            .to_string();

        self.game_cache
            .lock()
            .insert(universe_id.to_string(), GameCacheEntry { name: name.clone() });

        Some(name)
    }
}

pub fn enumerate_roblox_pids() -> Vec<u32> {
    let mut pids = Vec::new();
    unsafe {
        let snapshot = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            Ok(h) => h,
            Err(_) => return pids,
        };

        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };

        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let name = String::from_utf16_lossy(&entry.szExeFile);
                let clean_name = name.trim_matches('\0');
                if clean_name.eq_ignore_ascii_case("RobloxPlayerBeta.exe") {
                    pids.push(entry.th32ProcessID);
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    pids
}

struct EnumHwndData {
    target_pid: u32,
    found_hwnd: Option<HWND>,
}

unsafe extern "system" fn enum_windows_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let data = &mut *(lparam.0 as *mut EnumHwndData);
    if !IsWindow(Some(hwnd)).as_bool() || !IsWindowVisible(hwnd).as_bool() {
        return BOOL(1);
    }

    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == data.target_pid {
        data.found_hwnd = Some(hwnd);
        return BOOL(0); // Stop enumeration
    }
    BOOL(1)
}

pub fn find_hwnd_for_pid(pid: u32) -> Option<HWND> {
    let mut data = EnumHwndData {
        target_pid: pid,
        found_hwnd: None,
    };
    unsafe {
        let _ = EnumWindows(Some(enum_windows_callback), LPARAM(&mut data as *mut _ as isize));
    }
    data.found_hwnd
}

fn find_user_and_universe_from_logs(_pid: u32) -> Option<(String, String)> {
    let local_appdata = dirs::data_local_dir()?;
    let logs_dir = local_appdata.join("Roblox").join("logs");
    if !logs_dir.is_dir() {
        return None;
    }

    // Read top recent logs
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&logs_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().map_or(false, |ext| ext == "log"))
        .collect();

    entries.sort_by_key(|p| p.metadata().and_then(|m| m.modified()).ok());
    entries.reverse();

    // Check top 6 most recent logs
    for path in entries.into_iter().take(6) {
        if let Some(ids) = parse_log_for_user_and_universe(&path) {
            return Some(ids);
        }
    }
    None
}

fn parse_log_for_user_and_universe(path: &Path) -> Option<(String, String)> {
    // Open with non-blocking read/write sharing
    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(path)
        .ok()?;

    use std::io::{BufRead, BufReader};
    let reader = BufReader::new(file);

    // Look for: "Report game_join_loadtime: ... universeid:648454481, ... userid:2006823624,"
    for line in reader.lines().flatten() {
        if line.contains("Report game_join_loadtime:") {
            let mut universe_id = None;
            let mut user_id = None;

            if let Some(u_idx) = line.find("universeid:") {
                let rest = &line[u_idx + "universeid:".len()..];
                let end = rest.find([',', ' ', '\r', '\n']).unwrap_or(rest.len());
                let val = rest[..end].trim();
                if !val.is_empty() {
                    universe_id = Some(val.to_string());
                }
            }

            if let Some(uid_idx) = line.find("userid:") {
                let rest = &line[uid_idx + "userid:".len()..];
                let end = rest.find([',', ' ', '\r', '\n']).unwrap_or(rest.len());
                let val = rest[..end].trim();
                if !val.is_empty() {
                    user_id = Some(val.to_string());
                }
            }

            if let (Some(uid), Some(univid)) = (user_id, universe_id) {
                return Some((uid, univid));
            }
        }
    }

    None
}
