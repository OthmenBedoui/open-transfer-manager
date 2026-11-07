//! Passwords and keys for upload destinations.
//!
//! They are kept out of OTM's own files: Windows Credential Manager, the macOS Keychain,
//! or the Secret Service on Linux (through `secret-tool`). Only when none of these is
//! available (a headless server, for example) do they go to a private file readable by
//! the current user alone; `backend()` says which one is in use so the UI can tell.

use crate::paths;
use std::collections::BTreeMap;
use std::path::PathBuf;

const SERVICE: &str = "Open Transfer Manager";

pub fn set(key: &str, value: &str) -> Result<(), String> {
    if file_only() {
        return file_set(key, Some(value));
    }
    match native::set(key, value) {
        Ok(()) => {
            let _ = file_set(key, None); // drop an older copy from the fallback file
            Ok(())
        }
        Err(_) if native::unavailable() => file_set(key, Some(value)),
        Err(e) => Err(e),
    }
}

pub fn get(key: &str) -> Option<String> {
    if !file_only()
        && let Some(v) = native::get(key)
    {
        return Some(v);
    }
    file_read().remove(key)
}

pub fn delete(key: &str) {
    if !file_only() {
        native::delete(key);
    }
    let _ = file_set(key, None);
}

/// Where secrets are stored, in words for the user.
pub fn backend() -> &'static str {
    if file_only() || native::unavailable() {
        "a private file in OTM's data folder"
    } else if cfg!(windows) {
        "Windows Credential Manager"
    } else if cfg!(target_os = "macos") {
        "the macOS Keychain"
    } else {
        "the system keyring (Secret Service)"
    }
}

/// Tests and portable setups keep everything in the data folder.
fn file_only() -> bool {
    (cfg!(test) && std::env::var_os("OTM_TEST_KEYCHAIN").is_none())
        || std::env::var_os("OTM_SECRETS_FILE").is_some()
}

// ------------------------------------------------------------------ fallback file

static FILE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn file_path() -> PathBuf {
    if cfg!(test) {
        return std::env::temp_dir().join(format!("otm-test-secrets-{}.json", std::process::id()));
    }
    std::env::var_os("OTM_SECRETS_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| paths::data_dir().join("secrets.json"))
}

fn file_read() -> BTreeMap<String, String> {
    std::fs::read_to_string(file_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn file_set(key: &str, value: Option<&str>) -> Result<(), String> {
    let _guard = FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = file_read();
    let changed = match value {
        Some(v) => all.insert(key.to_string(), v.to_string()).as_deref() != Some(v),
        None => all.remove(key).is_some(),
    };
    if !changed {
        return Ok(());
    }
    let path = file_path();
    if all.is_empty() {
        let _ = std::fs::remove_file(&path);
        return Ok(());
    }
    let text = serde_json::to_string_pretty(&all).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    write_private(&tmp, &text).map_err(|e| format!("Could not save the secret: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("Could not save the secret: {e}"))
}

fn write_private(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(text.as_bytes())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, text)
    }
}

// ------------------------------------------------------------------ system stores

#[cfg(any(windows, target_os = "macos"))]
mod native {
    use super::SERVICE;

    fn entry(key: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(SERVICE, key).map_err(|e| e.to_string())
    }
    pub fn set(key: &str, value: &str) -> Result<(), String> {
        entry(key)?
            .set_password(value)
            .map_err(|e| format!("Could not save to the keychain: {e}"))
    }
    pub fn get(key: &str) -> Option<String> {
        entry(key).ok()?.get_password().ok()
    }
    pub fn delete(key: &str) {
        if let Ok(e) = entry(key) {
            let _ = e.delete_credential();
        }
    }
    pub fn unavailable() -> bool {
        false
    }
}

/// Linux and BSD: the Secret Service (GNOME Keyring, KWallet…) through `secret-tool`,
/// which avoids linking D-Bus into the binary.
#[cfg(not(any(windows, target_os = "macos")))]
mod native {
    use super::SERVICE;
    use crate::paths;
    use std::io::Write;
    use std::process::Stdio;
    use std::sync::OnceLock;

    fn tool() -> Option<std::path::PathBuf> {
        paths::find_program("secret-tool")
    }

    /// No `secret-tool`, or no keyring daemon answering (headless machines).
    pub fn unavailable() -> bool {
        static AVAILABLE: OnceLock<bool> = OnceLock::new();
        !*AVAILABLE.get_or_init(|| {
            let Some(t) = tool() else { return false };
            // A lookup of a missing item exits 1 quickly when the service runs; it fails
            // with a D-Bus error message on stderr when there is no service.
            paths::background_command(t)
                .args(["lookup", "service", SERVICE, "key", "otm-probe"])
                .stdin(Stdio::null())
                .output()
                .map(|o| o.stderr.is_empty())
                .unwrap_or(false)
        })
    }

    pub fn set(key: &str, value: &str) -> Result<(), String> {
        let t = tool().ok_or("secret-tool not found")?;
        let mut child = paths::background_command(t)
            .args([
                "store",
                "--label",
                &format!("{SERVICE}: {key}"),
                "service",
                SERVICE,
                "key",
                key,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(value.as_bytes())
                .map_err(|e| e.to_string())?;
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!(
                "Could not save to the keyring: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
    }

    pub fn get(key: &str) -> Option<String> {
        let out = paths::background_command(tool()?)
            .args(["lookup", "service", SERVICE, "key", key])
            .stdin(Stdio::null())
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).to_string()).filter(|s| !s.is_empty())
    }

    pub fn delete(key: &str) {
        if let Some(t) = tool() {
            let _ = paths::background_command(t)
                .args(["clear", "service", SERVICE, "key", key])
                .stdin(Stdio::null())
                .output();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_file_round_trip() {
        set("test-a", "pa$$ \"word\"").unwrap();
        set("test-b", "x").unwrap();
        assert_eq!(get("test-a").as_deref(), Some("pa$$ \"word\""));
        delete("test-a");
        assert_eq!(get("test-a"), None);
        assert_eq!(get("test-b").as_deref(), Some("x"));
        delete("test-b");
        assert_eq!(get("test-b"), None);
    }

    /// The real system store. Run with: OTM_TEST_KEYCHAIN=1 cargo test keychain -- --ignored
    #[test]
    #[ignore]
    fn keychain_round_trip() {
        assert!(!file_only());
        println!("backend: {}", backend());
        set("otm-selftest", "s3cret ✓").unwrap();
        assert_eq!(get("otm-selftest").as_deref(), Some("s3cret ✓"));
        assert!(
            !file_read().contains_key("otm-selftest"),
            "not in the fallback file"
        );
        delete("otm-selftest");
        assert_eq!(get("otm-selftest"), None);
    }
}
