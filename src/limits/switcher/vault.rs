//! Encrypts saved credentials at rest with DPAPI, bound to the current Windows user.

use std::io;
use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};
use windows::core::PCWSTR;

/// App-specific entropy: another program running as the same user can't decrypt a slot by
/// passing the blob to `CryptUnprotectData` without also knowing this value.
const ENTROPY: &[u8] = b"claude-codex-taskbar/switcher/v1";

pub fn protect(plain: &[u8]) -> io::Result<Vec<u8>> {
    crypt(plain, true)
}

pub fn unprotect(blob: &[u8]) -> io::Result<Vec<u8>> {
    crypt(blob, false)
}

fn crypt(input: &[u8], encrypt: bool) -> io::Result<Vec<u8>> {
    let len = u32::try_from(input.len()).map_err(io::Error::other)?;
    let data_in = CRYPT_INTEGER_BLOB {
        cbData: len,
        pbData: input.as_ptr() as *mut u8,
    };
    let entropy = CRYPT_INTEGER_BLOB {
        cbData: ENTROPY.len() as u32,
        pbData: ENTROPY.as_ptr() as *mut u8,
    };
    let mut out = CRYPT_INTEGER_BLOB::default();
    // SAFETY: the input blobs point at live slices that DPAPI only reads; `out` is allocated by
    // DPAPI, copied out, and freed with LocalFree exactly once.
    unsafe {
        let result = if encrypt {
            CryptProtectData(
                &data_in,
                PCWSTR::null(),
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        } else {
            CryptUnprotectData(
                &data_in,
                None,
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        result.map_err(|e| io::Error::other(e.message()))?;
        let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(out.pbData.cast())));
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_does_not_store_plaintext() {
        let secret = br#"{"claudeAiOauth":{"refreshToken":"FAKE-REFRESH"}}"#;
        let blob = protect(secret).unwrap();
        assert!(!blob.windows(12).any(|w| w == b"FAKE-REFRESH"));
        assert_eq!(unprotect(&blob).unwrap(), secret);
    }

    #[test]
    fn a_tampered_blob_fails() {
        let mut blob = protect(b"x").unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 0xff;
        assert!(unprotect(&blob).is_err());
    }
}
