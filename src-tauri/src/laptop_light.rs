use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyboardLightStatus {
    pub success: bool,
    pub level: i32,
    pub status: String,
    pub error: Option<String>,
    pub updated_at: String,
}

impl Default for KeyboardLightStatus {
    fn default() -> Self {
        Self {
            success: false,
            level: -1,
            status: "Unknown".to_string(),
            error: None,
            updated_at: String::new(),
        }
    }
}

pub fn get_data_dir() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("gpo-autofish")
}

pub fn ensure_worker_scripts() {
    let dir = get_data_dir();
    let _ = fs::create_dir_all(&dir);

    let dir_str = dir.to_string_lossy().to_string();
    let worker_path = dir.join("keyboard_backlight_worker.ps1");
    let worker_script = format!(
        r#"param([string]$Action = "toggle")

$baseDir = "{dir_str}"
if (-not (Test-Path $baseDir)) {{
    New-Item -ItemType Directory -Path $baseDir -Force | Out-Null
}}

$stateFile = "$baseDir\keyboard_light_state.json"
$targetFile = "$baseDir\keyboard_light_target.txt"

if (Test-Path $targetFile) {{
    try {{
        $raw = (Get-Content $targetFile -Raw).Trim()
        if ($raw) {{ $Action = $raw }}
    }} catch {{}}
}}

$cs = @"
using System;
using System.IO;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

public class EnergyDrv {{
    private const uint GENERIC_READ = 0x80000000;
    private const uint GENERIC_WRITE = 0x40000000;
    private const uint FILE_SHARE_READ = 0x00000001;
    private const uint FILE_SHARE_WRITE = 0x00000002;
    private const uint OPEN_EXISTING = 3;
    private const uint FILE_ATTRIBUTE_NORMAL = 0x80;

    public const uint IOCTL_ENERGY_KEYBOARD = 0x83102144;

    [DllImport("kernel32.dll", CharSet = CharSet.Auto, SetLastError = true)]
    public static extern SafeFileHandle CreateFile(
        string lpFileName,
        uint dwDesiredAccess,
        uint dwShareMode,
        IntPtr lpSecurityAttributes,
        uint dwCreationDisposition,
        uint dwFlagsAndAttributes,
        IntPtr hTemplateFile
    );

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool DeviceIoControl(
        SafeFileHandle hDevice,
        uint dwIoControlCode,
        ref uint lpInBuffer,
        uint nInBufferSize,
        out uint lpOutBuffer,
        uint nOutBufferSize,
        out uint lpBytesReturned,
        IntPtr lpOverlapped
    );

    public static SafeFileHandle Open() {{
        return CreateFile(@"\\.\EnergyDrv",
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            IntPtr.Zero,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            IntPtr.Zero);
    }}

    public static int GetState(SafeFileHandle handle) {{
        uint inBuffer = 0x22;
        uint outBuffer = 0;
        uint bytesReturned = 0;
        bool ok = DeviceIoControl(handle, IOCTL_ENERGY_KEYBOARD, ref inBuffer, 4, out outBuffer, 4, out bytesReturned, IntPtr.Zero);
        if (!ok) {{
            int err = Marshal.GetLastWin32Error();
            throw new Exception("DeviceIoControl get failed: " + err);
        }}
        return (int)outBuffer;
    }}

    public static bool SetState(SafeFileHandle handle, int level) {{
        uint inBuffer = 0x00023;
        switch (level) {{
            case 0: inBuffer = 0x00023; break;
            case 1: inBuffer = 0x10023; break;
            case 2: inBuffer = 0x20023; break;
        }}
        uint outBuffer = 0;
        uint bytesReturned = 0;
        return DeviceIoControl(handle, IOCTL_ENERGY_KEYBOARD, ref inBuffer, 4, out outBuffer, 4, out bytesReturned, IntPtr.Zero);
    }}
}}
"@

try {{
    Add-Type -TypeDefinition $cs -ErrorAction SilentlyContinue

    $h = [EnergyDrv]::Open()
    if ($h.IsInvalid) {{
        throw "Failed to open EnergyDrv device (error code $([System.Runtime.InteropServices.Marshal]::GetLastWin32Error()))"
    }}

    $rawState = [EnergyDrv]::GetState($h)
    $currLevel = switch ($rawState) {{
        0x1 {{ 0 }}
        0x3 {{ 1 }}
        0x5 {{ 2 }}
        default {{ 0 }}
    }}

    $newLevel = switch ($Action.ToLower()) {{
        "off"    {{ 0 }}
        "0"      {{ 0 }}
        "low"    {{ 1 }}
        "1"      {{ 1 }}
        "high"   {{ 2 }}
        "2"      {{ 2 }}
        "cycle"  {{ if ($currLevel -ge 2) {{ 0 }} else {{ $currLevel + 1 }} }}
        "toggle" {{ if ($currLevel -ge 2) {{ 0 }} else {{ $currLevel + 1 }} }}
        default  {{ if ($currLevel -ge 2) {{ 0 }} else {{ $currLevel + 1 }} }}
    }}

    [EnergyDrv]::SetState($h, $newLevel) | Out-Null
    $h.Close()

    $statusName = switch ($newLevel) {{ 0 {{ "Off" }} 1 {{ "Low" }} 2 {{ "High" }} default {{ "Unknown" }} }}
    $outObj = @{{
        success = $true
        level = $newLevel
        status = $statusName
        error = $null
        updated_at = (Get-Date).ToString("yyyy-MM-ddTHH:mm:ss")
    }}
    $outObj | ConvertTo-Json -Compress | Set-Content -Path $stateFile -Force
}} catch {{
    $errObj = @{{
        success = $false
        level = -1
        status = "Error"
        error = $_.Exception.Message
        updated_at = (Get-Date).ToString("yyyy-MM-ddTHH:mm:ss")
    }}
    $errObj | ConvertTo-Json -Compress | Set-Content -Path $stateFile -Force
}}
"#
    );
    let _ = fs::write(&worker_path, worker_script);

    let setup_path = dir.join("setup_keyboard_backlight.ps1");
    let setup_script = format!(
        r#"$worker = "{dir_str}\keyboard_backlight_worker.ps1"
$action = "powershell.exe -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File `"$worker`""

$res = schtasks /create /tn "GPO_KeyboardBacklight" /tr "$action" /sc ONCE /st 00:00 /ru "SYSTEM" /f
Write-Host "Task creation result: $res"
& "$worker" -Action "toggle"
"#
    );
    let _ = fs::write(&setup_path, setup_script);
}

pub fn get_keyboard_light_status() -> KeyboardLightStatus {
    let dir = get_data_dir();
    let state_file = dir.join("keyboard_light_state.json");
    if state_file.is_file() {
        if let Ok(content) = fs::read_to_string(&state_file) {
            if let Ok(st) = serde_json::from_str::<KeyboardLightStatus>(&content) {
                return st;
            }
        }
    }

    KeyboardLightStatus::default()
}

pub fn set_keyboard_light(action: &str) -> KeyboardLightStatus {
    ensure_worker_scripts();
    let dir = get_data_dir();
    let target_file = dir.join("keyboard_light_target.txt");
    let state_file = dir.join("keyboard_light_state.json");

    let _ = fs::write(&target_file, action);

    let prev_mtime = fs::metadata(&state_file)
        .and_then(|m| m.modified())
        .ok();

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        // Strategy 1: Trigger elevated scheduled task if configured
        let sch_status = Command::new("schtasks")
            .args(["/run", "/tn", "GPO_KeyboardBacklight"])
            .creation_flags(CREATE_NO_WINDOW)
            .status();

        if let Ok(status) = sch_status {
            if status.success() {
                // Poll for updated state file up to 250ms
                let start = Instant::now();
                while start.elapsed() < Duration::from_millis(250) {
                    thread::sleep(Duration::from_millis(30));
                    if let Ok(m) = fs::metadata(&state_file) {
                        if let Ok(mtime) = m.modified() {
                            if Some(mtime) != prev_mtime {
                                return get_keyboard_light_status();
                            }
                        }
                    }
                }
            }
        }

        // Strategy 2: Direct PowerShell worker execution
        let worker_path = dir.join("keyboard_backlight_worker.ps1");
        let _ = Command::new("powershell")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                &worker_path.to_string_lossy(),
                "-Action",
                action,
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .status();

        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(500) {
            thread::sleep(Duration::from_millis(30));
            if let Ok(m) = fs::metadata(&state_file) {
                if let Ok(mtime) = m.modified() {
                    if Some(mtime) != prev_mtime {
                        break;
                    }
                }
            }
        }
    }

    get_keyboard_light_status()
}

pub fn setup_keyboard_light_task() -> Result<String, String> {
    ensure_worker_scripts();
    let dir = get_data_dir();
    let setup_path = dir.join("setup_keyboard_backlight.ps1");

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let cmd = format!(
            "Start-Process powershell.exe -ArgumentList '-NoProfile -ExecutionPolicy Bypass -File \"{}\"' -Verb RunAs",
            setup_path.to_string_lossy()
        );
        match Command::new("powershell")
            .creation_flags(CREATE_NO_WINDOW)
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &cmd])
            .status()
        {
            Ok(status) if status.success() => Ok(
                "Administrator setup launched. Please click 'Yes' on the Windows permission prompt on your laptop.".to_string(),
            ),
            Ok(_) => Err("Elevation prompt was cancelled or failed to run.".to_string()),
            Err(e) => Err(format!("Failed to launch elevation process: {e}")),
        }
    }

    #[cfg(not(windows))]
    {
        Err("Keyboard light control is only supported on Windows Lenovo laptops.".to_string())
    }
}
