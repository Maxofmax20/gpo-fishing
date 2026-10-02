//! Secret storage helpers: Windows DPAPI (user scope) + log redaction.
//!
//! Plaintext secrets (Telegram tokens, Discord webhook URLs, Gemini API keys,
//! `.ROBLOSECURITY` cookies, web-dashboard tokens) must never be written to
//! disk or logs in the clear. This module provides:
//!
//! * [`protect`] / [`unprotect`] — DPAPI round-trip. Protected values are
//!   serialized as `ENC1:<base64>` so legacy plaintext remains readable for
//!   migration (`maybe_unprotect`).
//! * [`redact`] — replaces known secret values inside log/error strings.
//! * [`strip_token_from_uri`] — removes `?token=` / `&token=` values from URIs
//!   before logging.
//! * [`generate_token_hex`] — cryptographically random hex token for the
//!   local web dashboard.
//!
//! On non-Windows targets (tests only — the app ships Windows-only) DPAPI is
//! unavailable; a clearly-marked `ENC0` fallback encoding is used so unit
//! tests still exercise the migration/redaction paths. `ENC0` is NOT secure
//! and must never be relied upon in production.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

pub const ENC_PREFIX: &str = "ENC1:";
/// Non-secure test fallback prefix (non-Windows only). Never use in prod.
pub const ENC_FALLBACK_PREFIX: &str = "ENC0:";

/// Prefix actually written by [`protect`] on this platform: real DPAPI on
/// Windows, the clearly-marked insecure fallback elsewhere (tests only).
#[cfg(windows)]
const ACTIVE_PREFIX: &str = ENC_PREFIX;
#[cfg(not(windows))]
const ACTIVE_PREFIX: &str = ENC_FALLBACK_PREFIX;

/// Returns true if `s` looks like a protected value (either prefix).
pub fn is_protected(s: &str) -> bool {
    s.starts_with(ENC_PREFIX) || s.starts_with(ENC_FALLBACK_PREFIX)
}

/// Encrypt `plaintext` with user-scoped DPAPI. Empty input stays empty
/// (avoids persisting noise for unset optional secrets).
pub fn protect(plaintext: &str) -> Result<String, String> {
    if plaintext.is_empty() {
        return Ok(String::new());
    }
    let cipher = protect_bytes(plaintext.as_bytes())?;
    Ok(format!("{ACTIVE_PREFIX}{}", B64.encode(cipher)))
}

/// Constant-time equality for token comparison (no short-circuit oracle).
pub fn tokens_equal(a: &str, b: &str) -> bool {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    if ab.len() != bb.len() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..ab.len() {
        diff |= ab[i] ^ bb[i];
    }
    diff == 0
}

/// Decrypt a value produced by [`protect`]. Fails for plaintext input —
/// use [`maybe_unprotect`] when legacy plaintext must keep working.
pub fn unprotect(enc: &str) -> Result<String, String> {
    if let Some(b64) = enc.strip_prefix(ENC_PREFIX) {
        let cipher = B64.decode(b64).map_err(|e| format!("secret base64: {e}"))?;
        let plain = unprotect_bytes(&cipher)?;
        return String::from_utf8(plain).map_err(|e| format!("secret utf8: {e}"));
    }
    if let Some(b64) = enc.strip_prefix(ENC_FALLBACK_PREFIX) {
        let plain = B64.decode(b64).map_err(|e| format!("secret base64: {e}"))?;
        return String::from_utf8(plain).map_err(|e| format!("secret utf8: {e}"));
    }
    Err("value is not protected (legacy plaintext)".into())
}

/// Migration helper: decrypt `ENC1:`/`ENC0:` values, pass plaintext through
/// unchanged. Used on load so old configs keep working; callers re-encrypt
/// on the next save.
///
/// Fail-safe: if decryption fails (e.g. Windows user profile changed), the
/// ORIGINAL ciphertext is preserved — never replaced with empty — so a later
/// save cannot destroy the secret. Callers must treat an `is_protected`
/// value in memory as unusable and prompt to re-enter it.
pub fn maybe_unprotect(s: &str) -> String {
    if is_protected(s) {
        match unprotect(s) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("secret decrypt failed ({}); preserving ciphertext", redact_token(&e));
                s.to_string()
            }
        }
    } else {
        s.to_string()
    }
}

/// Generate `num_bytes` of randomness as lowercase hex (web-dashboard token).
pub fn generate_token_hex(num_bytes: usize) -> String {
    let bytes = random_bytes(num_bytes.clamp(16, 64));
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Replace every occurrence of each known secret inside `text` with `***`.
/// Also masks `Bearer <value>` and `token=<value>` patterns generically.
pub fn redact(text: &str, secrets: &[&str]) -> String {
    let mut out = text.to_string();
    for s in secrets {
        // Skip tiny fragments (single chars) to avoid mangling ordinary log
        // text; anything token-shaped (>= 4 chars) is masked.
        if s.len() < 4 {
            continue;
        }
        out = out.replace(s, "***");
    }
    out = mask_query_token(&out);
    mask_bearer(&out)
}

/// Redact just the token-shaped part of a short string (for warn! paths).
/// Char-boundary safe (never panics on multi-byte input).
pub fn redact_token(s: &str) -> String {
    let head: String = s.chars().take(4).collect();
    if s.chars().count() <= 8 {
        "***".into()
    } else {
        format!("{head}***")
    }
}

/// Strip `token=...` query values from a URI/path before logging.
pub fn strip_token_from_uri(uri: &str) -> String {
    mask_query_param(uri, "token=")
}

/// Strip Gemini-style `key=...` values from error strings. reqwest errors
/// echo the request URL, which carries the API key as `?key=`.
pub fn strip_key_param(s: &str) -> String {
    mask_query_param(s, "key=")
}

fn mask_query_token(s: &str) -> String {
    mask_query_param(s, "token=")
}

/// Byte-scan for an ASCII `param`, copying everything else char-by-char so
/// multi-byte UTF-8 (OCR text, chat messages) is never corrupted. Param
/// matching on bytes is sound: UTF-8 continuation bytes are >= 0x80 and can
/// never falsely match ASCII.
fn mask_query_param(s: &str, param: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i..].starts_with(param) {
            out.push_str(param);
            out.push_str("***");
            i += param.len();
            while i < s.len() && !matches!(s.as_bytes()[i], b'&' | b' ' | b'"' | b'\'' | b'\r' | b'\n') {
                // Values are tokens/keys (ASCII); advance by char for safety.
                let ch = s[i..].chars().next().unwrap_or('\u{FFFD}');
                i += ch.len_utf8();
            }
        } else {
            let ch = s[i..].chars().next().unwrap_or('\u{FFFD}');
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn mask_bearer(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i..].starts_with("Bearer ") {
            out.push_str("Bearer ***");
            i += 7;
            while i < s.len()
                && !matches!(s.as_bytes()[i], b' ' | b'"' | b'\'' | b'\r' | b'\n' | b',' | b';')
            {
                let ch = s[i..].chars().next().unwrap_or('\u{FFFD}');
                i += ch.len_utf8();
            }
        } else {
            let ch = s[i..].chars().next().unwrap_or('\u{FFFD}');
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Platform backends
// ---------------------------------------------------------------------------

/// Raw FFI to stable Win32 C APIs (crypt32/bcrypt/kernel32). Used instead of
/// the `windows` crate so DPAPI does not depend on that crate's feature
/// selection; these signatures have been stable since Windows 2000.
#[cfg(windows)]
mod win32 {
    use std::ffi::c_void;

    #[repr(C)]
    pub struct DataBlob {
        pub cb_data: u32,
        pub pb_data: *mut u8,
    }

    pub const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;
    pub const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x00000002;

    #[link(name = "crypt32")]
    extern "system" {
        pub fn CryptProtectData(
            p_data_in: *const DataBlob,
            sz_data_descr: *const u16,
            p_optional_entropy: *const DataBlob,
            pv_reserved: *const c_void,
            p_prompt_struct: *const c_void,
            dw_flags: u32,
            p_data_out: *mut DataBlob,
        ) -> i32;
        pub fn CryptUnprotectData(
            p_data_in: *const DataBlob,
            ppsz_data_descr: *mut *mut u16,
            p_optional_entropy: *const DataBlob,
            pv_reserved: *const c_void,
            p_prompt_struct: *const c_void,
            dw_flags: u32,
            p_data_out: *mut DataBlob,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        pub fn LocalFree(hmem: *mut c_void) -> *mut c_void;
    }

    #[link(name = "bcrypt")]
    extern "system" {
        pub fn BCryptGenRandom(
            h_algorithm: *mut c_void,
            pb_buffer: *mut u8,
            cb_buffer: u32,
            dw_flags: u32,
        ) -> i32;
    }
}

#[cfg(windows)]
fn dpapi_roundtrip(protect: bool, input: &[u8]) -> Result<Vec<u8>, String> {
    use win32::*;
    use std::ptr::null;

    let in_blob = DataBlob { cb_data: input.len() as u32, pb_data: input.as_ptr() as *mut u8 };
    let mut out_blob = DataBlob { cb_data: 0, pb_data: null::<u8>() as *mut u8 };
    // SAFETY: in_blob borrows `input` which outlives the call; out_blob is
    // allocated by DPAPI with LocalAlloc and freed below. User scope is
    // selected by passing no LOCAL_MACHINE flag, so only this Windows user
    // can decrypt.
    let ok = unsafe {
        if protect {
            CryptProtectData(
                &in_blob,
                null(),
                null(),
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out_blob,
            )
        } else {
            CryptUnprotectData(
                &in_blob,
                null::<*mut u16>() as *mut *mut u16,
                null(),
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out_blob,
            )
        }
    };
    if ok == 0 || out_blob.pb_data.is_null() || out_blob.cb_data == 0 {
        return Err(if protect {
            "CryptProtectData failed".into()
        } else {
            "CryptUnprotectData failed (key changed user/machine?)".into()
        });
    }
    // SAFETY: DPAPI guarantees `cb_data` readable bytes at `pb_data`.
    let bytes = unsafe {
        std::slice::from_raw_parts(out_blob.pb_data as *const u8, out_blob.cb_data as usize).to_vec()
    };
    // SAFETY: DPAPI blobs are freed with LocalFree.
    unsafe {
        LocalFree(out_blob.pb_data as *mut std::ffi::c_void);
    }
    Ok(bytes)
}

#[cfg(windows)]
fn protect_bytes(plain: &[u8]) -> Result<Vec<u8>, String> {
    dpapi_roundtrip(true, plain)
}

#[cfg(windows)]
fn unprotect_bytes(cipher: &[u8]) -> Result<Vec<u8>, String> {
    dpapi_roundtrip(false, cipher)
}

#[cfg(windows)]
fn random_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    // SAFETY: buffer is valid for `n` bytes; system-preferred RNG (null
    // algorithm handle) is the documented CNG one-shot pattern.
    let status = unsafe {
        win32::BCryptGenRandom(
            null_mut_handle(),
            buf.as_mut_ptr(),
            n as u32,
            win32::BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status >= 0 {
        return buf;
    }
    fallback_random_bytes(n)
}

#[cfg(windows)]
fn null_mut_handle() -> *mut std::ffi::c_void {
    std::ptr::null::<std::ffi::c_void>() as *mut std::ffi::c_void
}

#[cfg(not(windows))]
fn protect_bytes(plain: &[u8]) -> Result<Vec<u8>, String> {
    // Test-only fallback: NOT secure. Production ships Windows-only.
    Ok(plain.to_vec())
}

#[cfg(not(windows))]
fn unprotect_bytes(cipher: &[u8]) -> Result<Vec<u8>, String> {
    Ok(cipher.to_vec())
}

#[cfg(not(windows))]
fn random_bytes(n: usize) -> Vec<u8> {
    fallback_random_bytes(n)
}

fn fallback_random_bytes(n: usize) -> Vec<u8> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};
    let mut out = Vec::with_capacity(n);
    let mut counter = 0u64;
    while out.len() < n {
        let mut h = DefaultHasher::new();
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
            .hash(&mut h);
        std::process::id().hash(&mut h);
        std::thread::current().id().hash(&mut h);
        counter.hash(&mut h);
        counter += 1;
        out.extend_from_slice(&h.finish().to_le_bytes());
    }
    out.truncate(n);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_protect_unprotect() {
        let token = "synthetic-test-token-abc123XYZ";
        let enc = protect(token).expect("protect");
        assert!(enc.starts_with(ENC_PREFIX) || enc.starts_with(ENC_FALLBACK_PREFIX));
        assert!(!enc.contains(token));
        let back = unprotect(&enc).expect("unprotect");
        assert_eq!(back, token);
    }

    #[test]
    fn empty_stays_empty() {
        assert_eq!(protect("").unwrap(), "");
        assert_eq!(maybe_unprotect(""), "");
    }

    #[test]
    fn legacy_plaintext_passes_through() {
        assert_eq!(maybe_unprotect("plain-value"), "plain-value");
        assert!(unprotect("plain-value").is_err());
    }

    #[test]
    fn undecryptable_ciphertext_is_preserved_never_wiped() {
        // Corrupt/key-mismatched blobs must survive a load→save cycle so a
        // later save cannot destroy the secret.
        let bad = "ENC1:!!!not-valid-base64!!!";
        assert!(unprotect(bad).is_err());
        assert_eq!(maybe_unprotect(bad), bad);
    }

    #[test]
    fn serialized_form_never_contains_plaintext() {
        let secret = "synthetic-discord-webhook-secret-999";
        let enc = protect(secret).unwrap();
        let json = serde_json::json!({ "url": enc }).to_string();
        assert!(!json.contains(secret));
    }

    #[test]
    fn redact_masks_known_secrets_and_bearer() {
        let secret = "synthetic-secret-value-12345678";
        let text = format!("failed with {secret} and Bearer {secret} end");
        let out = redact(&text, &[secret]);
        assert!(!out.contains(secret));
        assert!(out.contains("***"));
    }

    #[test]
    fn strip_token_from_uri_masks_query() {
        let uri = "/api/stream?fps=5&token=supersecretvalue123&scale=720";
        let out = strip_token_from_uri(uri);
        assert!(!out.contains("supersecretvalue123"));
        assert!(out.contains("token=***"));
        assert!(out.contains("fps=5"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_emits_real_dpapi_prefix() {
        let enc = protect("synthetic-value-123").unwrap();
        assert!(enc.starts_with(ENC_PREFIX), "Windows must emit ENC1 (real DPAPI), got {enc}");
    }

    #[test]
    fn token_compare_is_exact() {
        assert!(tokens_equal("abc123", "abc123"));
        assert!(!tokens_equal("abc123", "abc124"));
        assert!(!tokens_equal("abc123", "abc1234"));
        assert!(!tokens_equal("", "abc123"));
        assert!(tokens_equal("", ""));
    }

    #[test]
    fn masking_preserves_multibyte_text() {
        let text = "OCR 掉落：suna-suna 果实 token=supersecretvalue123 done 🎣";
        let out = strip_token_from_uri(text);
        assert!(!out.contains("supersecretvalue123"));
        // Non-ASCII content must survive byte-oriented masking intact.
        assert!(out.contains("掉落"));
        assert!(out.contains("🎣"));
        assert!(out.contains("suna-suna"));
    }

    #[test]
    fn redact_token_never_panics_on_multibyte() {
        assert_eq!(redact_token("掉落果实测试文本多"), "掉落果实***");
        assert_eq!(redact_token("掉落果实测试文本"), "***");
        assert_eq!(redact_token("ab"), "***");
    }

    #[test]
    fn generated_tokens_are_unique_hex() {
        let a = generate_token_hex(32);
        let b = generate_token_hex(32);
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
