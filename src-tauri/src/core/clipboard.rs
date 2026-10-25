//! Windows clipboard get/set over raw Win32 (no new dependencies).
//!
//! Used by the webapp TYPE/CLIPBOARD dialog: push text from a phone to the
//! PC clipboard, or pull the PC clipboard back to the phone. Unicode
//! (`CF_UNICODETEXT`) throughout. The clipboard is a shared system resource
//! that other apps may hold open: every operation retries briefly, then
//! reports busy honestly instead of pretending success.

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_UNICODETEXT;

const BUSY_RETRIES: u32 = 6;
const BUSY_WAIT_MS: u64 = 50;

fn busy_wait() {
    std::thread::sleep(std::time::Duration::from_millis(BUSY_WAIT_MS));
}

/// Read Unicode text from the Windows clipboard.
/// `Ok(None)` = clipboard has no text (not an error).
/// `Err` = clipboard genuinely unavailable (locked/denied after retries).
pub fn get_text() -> Result<Option<String>, String> {
    unsafe {
        let mut last_err = "clipboard unavailable".to_string();
        for _ in 0..BUSY_RETRIES {
            if OpenClipboard(None).is_err() {
                last_err = "clipboard is busy (another app holds it)".to_string();
                busy_wait();
                continue;
            }
            let result = (|| {
                if IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_err() {
                    return Ok(None);
                }
                let h = GetClipboardData(CF_UNICODETEXT.0 as u32)
                    .map_err(|e| format!("clipboard read failed: {e}"))?;
                if h.is_invalid() {
                    return Ok(None);
                }
                let ptr = GlobalLock(HGLOBAL(h.0)) as *const u16;
                if ptr.is_null() {
                    return Err("clipboard lock failed".to_string());
                }
                let mut len = 0usize;
                while *ptr.add(len) != 0 {
                    len += 1;
                }
                let slice = std::slice::from_raw_parts(ptr, len);
                let text = String::from_utf16_lossy(slice);
                let _ = GlobalUnlock(HGLOBAL(h.0));
                Ok(Some(text))
            })();
            let _ = CloseClipboard();
            return result;
        }
        Err(last_err)
    }
}

/// Replace the Windows clipboard text. Returns the char count written.
/// Ownership of the global memory transfers to the system on success.
pub fn set_text(text: &str) -> Result<usize, String> {
    let chars = text.chars().count();
    unsafe {
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes = wide.len() * 2;
        let mut last_err = "clipboard unavailable".to_string();
        for _ in 0..BUSY_RETRIES {
            if OpenClipboard(None).is_err() {
                last_err = "clipboard is busy (another app holds it)".to_string();
                busy_wait();
                continue;
            }
            let result = (|| {
                EmptyClipboard().map_err(|e| format!("clipboard clear failed: {e}"))?;
                let h = GlobalAlloc(GMEM_MOVEABLE, bytes)
                    .map_err(|e| format!("clipboard alloc failed: {e}"))?;
                let dst = GlobalLock(h) as *mut u16;
                if dst.is_null() {
                    let _ = GlobalFree(Some(h));
                    return Err("clipboard lock failed".to_string());
                }
                std::ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
                let _ = GlobalUnlock(h);
                // On success the system owns `h` — never free it.
                // On failure it stays ours — free it to avoid leaking.
                    match SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(h.0))) {
                        Ok(_) => Ok(chars),
                        Err(e) => {
                            let _ = GlobalFree(Some(h));
                            Err(format!("clipboard write failed: {e}"))
                        }
                    }
            })();
            let _ = CloseClipboard();
            return result;
        }
        Err(last_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_roundtrip_preserves_unicode() {
        let saved = get_text().ok().flatten();
        let token = format!("GPO-CLIP-TEST-αβγ-🎣-{}", std::process::id());
        let n = set_text(&token).expect("clipboard must be writable in test");
        assert_eq!(n, token.chars().count());
        assert_eq!(get_text().expect("readable").as_deref(), Some(token.as_str()));
        // Restore whatever was there (or clear when there was no text).
        if let Some(prev) = saved {
            let _ = set_text(&prev);
        } else {
            let _ = set_text("");
        }
    }
}
