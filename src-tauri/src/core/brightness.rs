use std::sync::atomic::AtomicU32;

static CACHED_BRIGHTNESS: AtomicU32 = AtomicU32::new(80);

#[cfg(windows)]
pub mod windows_brightness {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    use std::sync::atomic::Ordering;
    use super::CACHED_BRIGHTNESS;

    const CREATE_NO_WINDOW: u32 = 0x08000000;

    pub fn get_brightness() -> Result<u32, String> {
        let output = Command::new("powershell")
            .creation_flags(CREATE_NO_WINDOW)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "(Get-CimInstance -Namespace root/wmi -ClassName WmiMonitorBrightness -ErrorAction SilentlyContinue).CurrentBrightness",
            ])
            .output()
            .map_err(|e| format!("Failed to execute PowerShell: {e}"))?;

        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if let Ok(val) = text.parse::<u32>() {
            let clamped = val.clamp(0, 100);
            CACHED_BRIGHTNESS.store(clamped, Ordering::SeqCst);
            return Ok(clamped);
        }

        // Fallback to cached value
        Ok(CACHED_BRIGHTNESS.load(Ordering::SeqCst))
    }

    pub fn set_brightness(level: u32) -> Result<u32, String> {
        let clamped = level.clamp(0, 100);
        let script = format!(
            "(Get-WmiObject -Namespace root/wmi -ClassName WmiMonitorBrightnessMethods -ErrorAction SilentlyContinue).WmiSetBrightness(1, {clamped})"
        );

        let output = Command::new("powershell")
            .creation_flags(CREATE_NO_WINDOW)
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .map_err(|e| format!("Failed to execute PowerShell: {e}"))?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            tracing::warn!("WmiSetBrightness returned non-zero: {err}");
        }

        CACHED_BRIGHTNESS.store(clamped, Ordering::SeqCst);
        Ok(clamped)
    }
}

#[cfg(windows)]
pub use windows_brightness::*;

#[cfg(not(windows))]
pub fn get_brightness() -> Result<u32, String> {
    Ok(CACHED_BRIGHTNESS.load(Ordering::SeqCst))
}

#[cfg(not(windows))]
pub fn set_brightness(level: u32) -> Result<u32, String> {
    let clamped = level.clamp(0, 100);
    CACHED_BRIGHTNESS.store(clamped, Ordering::SeqCst);
    Ok(clamped)
}
