//! Small secret store (`secrets.dat` next to the exe): GitHub token, later
//! webhook URL / private server link. On Windows every value is encrypted
//! with DPAPI, so the file is useless on another PC or Windows user.

use anyhow::{Context, Result};
use base64::Engine;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub struct Secrets {
    path: PathBuf,
    map: BTreeMap<String, String>,
}

impl Secrets {
    pub fn default_path() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("secrets.dat")))
            .unwrap_or_else(|| PathBuf::from("secrets.dat"))
    }

    pub fn load(path: &Path) -> Self {
        let map = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Self {
            path: path.to_path_buf(),
            map,
        }
    }

    pub fn get(&self, key: &str) -> Option<String> {
        let b = base64::engine::general_purpose::STANDARD
            .decode(self.map.get(key)?)
            .ok()?;
        let plain = unprotect(&b).ok()?;
        String::from_utf8(plain).ok().filter(|s| !s.is_empty())
    }

    pub fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        if value.trim().is_empty() {
            self.map.remove(key);
        } else {
            let enc = protect(value.trim().as_bytes())?;
            self.map.insert(
                key.into(),
                base64::engine::general_purpose::STANDARD.encode(enc),
            );
        }
        std::fs::write(&self.path, serde_json::to_string_pretty(&self.map)?)
            .context("writing secrets.dat")
    }
}

#[cfg(windows)]
fn protect(data: &[u8]) -> Result<Vec<u8>> {
    dpapi(data, true)
}
#[cfg(windows)]
fn unprotect(data: &[u8]) -> Result<Vec<u8>> {
    dpapi(data, false)
}

#[cfg(windows)]
fn dpapi(data: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB,
    };
    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe {
        if encrypt {
            CryptProtectData(&input, PCWSTR::null(), None, None, None, 0, &mut out)?;
        } else {
            CryptUnprotectData(&input, None, None, None, None, 0, &mut out)?;
        }
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(out.pbData as *mut core::ffi::c_void)));
        Ok(v)
    }
}

// Non-Windows (tests / dev): no OS keystore, values are only obfuscated.
#[cfg(not(windows))]
fn protect(data: &[u8]) -> Result<Vec<u8>> {
    Ok(data.iter().map(|b| b ^ 0x5A).collect())
}
#[cfg(not(windows))]
fn unprotect(data: &[u8]) -> Result<Vec<u8>> {
    protect(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_clear() {
        let p = std::env::temp_dir().join(format!("aa-secrets-{}.dat", std::process::id()));
        let mut s = Secrets::load(&p);
        assert!(!s.has("github_token"));
        s.set("github_token", " ghp_abc ").unwrap();
        let again = Secrets::load(&p);
        assert_eq!(again.get("github_token").as_deref(), Some("ghp_abc"));
        assert!(!std::fs::read_to_string(&p).unwrap().contains("ghp_abc"));
        s.set("github_token", "").unwrap();
        assert!(!Secrets::load(&p).has("github_token"));
        std::fs::remove_file(p).ok();
    }
}
