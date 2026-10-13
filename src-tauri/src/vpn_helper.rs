//! Fixed Task Scheduler actions installed once with elevation. The macro
//! never runs elevated; ordinary users cannot alter helper code or tasks.
use std::path::PathBuf;

pub fn directory() -> PathBuf {
    PathBuf::from(r"C:\Program Files\GPO German Relay")
}

pub fn available() -> bool {
    cfg!(windows) && directory().join("ready-v1").is_file()
}

pub fn run(connect: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let task = if connect { "GPO_GermanRelayStart_v1" } else { "GPO_GermanRelayStop_v1" };
        let result = std::process::Command::new("schtasks.exe").args(["/Run", "/TN", task]).creation_flags(0x08000000).output().map_err(|e| e.to_string())?;
        if result.status.success() { return Ok(()); }
        Err("Windows refused the German relay helper operation. Run Repair helper on the VPN page; the macro itself needs no Administrator rights.".into())
    }
    #[cfg(not(windows))]
    { let _ = connect; Err("German relay helper is only available on Windows".into()) }
}

pub fn install() -> Result<String, String> {
    static SETUP: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
    let _setup = SETUP.try_lock().ok_or("German helper setup is already running")?;
    #[cfg(windows)]
    {
        use base64::Engine;
        use std::os::windows::process::CommandExt;
        let encode = |script: &str| base64::engine::general_purpose::STANDARD.encode(script.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<_>>());
        let sid = std::process::Command::new("whoami.exe").args(["/user", "/fo", "csv", "/nh"]).creation_flags(0x08000000).output().map_err(|e| e.to_string())?;
        let raw = String::from_utf8_lossy(&sid.stdout);
        let sid = raw.trim().split(',').next_back().unwrap_or("").trim_matches('"');
        if !sid.starts_with("S-1-5-21-") || !sid.bytes().all(|b| b == b'S' || b == b'-' || b.is_ascii_digit()) { return Err("Could not identify the current Windows user".into()); }
        let script = include_str!("vpn_helper_setup.ps1").replace("%%SID%%", sid);
        // Pass embedded code directly, never execute an elevated script from
        // a writable temporary file. Windows displays its one-time UAC prompt.
        let bootstrap = format!("$ErrorActionPreference='Stop'; try {{ $p=Start-Process -FilePath 'C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe' -Verb RunAs -WindowStyle Hidden -ArgumentList '-NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand {}' -Wait -PassThru; exit $p.ExitCode }} catch {{ exit 1 }}", encode(&script));
        let result = std::process::Command::new(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", &bootstrap]).creation_flags(0x08000000).status().map_err(|e| e.to_string())?;
        if !result.success() || !available() { return Err("German helper setup was cancelled or failed. Your macro remains a standard-user app; no tunnel was started. Check the sing-box profile and retry setup.".into()); }
        Ok("German relay helper installed. Connect and disconnect now work without elevating the macro.".into())
    }
    #[cfg(not(windows))]
    { Err("German relay helper is only available on Windows".into()) }
}

#[cfg(test)]
mod tests {
    #[test]
    fn setup_exposes_only_fixed_tasks_and_protects_files() {
        let script = include_str!("vpn_helper_setup.ps1");
        assert!(script.contains("SetAccessRuleProtection($true, $false)"));
        assert!(script.contains("(A;;GRGX;;;$ownerSid)"));
        assert!(!script.contains("(A;;GA;;;$ownerSid)"));
        assert!(script.contains("ReparsePoint"));
        assert!(script.contains("Get-FileHash"));
        assert!(script.contains("ExecutionTimeLimit = 'PT0S'"));
        assert!(!script.contains("Triggers.Create"));
    }
}
