//! Register the native messaging host with every browser found on this computer.

use crate::paths;
use serde_json::json;
use std::path::PathBuf;

pub const HOST_NAME: &str = "org.opentransfermanager.host";
/// Fixed by the "key" in the Chromium manifest; add store IDs here when published.
pub const CHROMIUM_IDS: &[&str] = &["iaaaiogfbkpiddhonhfnekdffilohjjc"];
pub const FIREFOX_IDS: &[&str] = &["otm@opentransfermanager.org"];

pub fn valid_chrome_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|c| (b'a'..=b'p').contains(&c))
}

pub fn chrome_store_id() -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(extension_dir().join("distribution.json")).ok()?,
    )
    .ok()?;
    let id = value["chrome_store_id"].as_str()?;
    valid_chrome_id(id).then(|| id.to_string())
}

/// Opens Chrome's supported store/development flow. Never edits browser profiles.
pub fn open_chrome_setup() -> Result<(), String> {
    let url = chrome_store_id()
        .map(|id| format!("https://chromewebstore.google.com/detail/{id}"))
        .unwrap_or_else(|| "chrome://extensions/".into());
    let folder = extension_dir().join("chromium");
    if !folder.join("manifest.json").is_file() {
        return Err("Bundle Chrome introuvable. Réinstallez OTM avec le setup complet.".into());
    }
    #[cfg(windows)]
    {
        let browser = ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
            .iter()
            .filter_map(std::env::var_os)
            .map(|base| PathBuf::from(base).join("Google/Chrome/Application/chrome.exe"))
            .find(|p| p.is_file())
            .ok_or("Google Chrome est introuvable sur cet ordinateur.")?;
        paths::background_command(browser)
            .args(["--new-tab", &url])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(windows))]
    {
        let _ = open::that_detached(&url);
    }
    if chrome_store_id().is_none() {
        paths::open_path(&folder);
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Chromium,
    Firefox,
}

/// (browser name, family, folder proving it is installed, manifest folder)
fn targets() -> Vec<(&'static str, Family, PathBuf, PathBuf)> {
    let h = paths::home();
    let mut out = Vec::new();
    if cfg!(target_os = "macos") {
        let sup = h.join("Library/Application Support");
        for (n, d) in [
            ("Google Chrome", "Google/Chrome"),
            ("Chromium", "Chromium"),
            ("Brave", "BraveSoftware/Brave-Browser"),
            ("Microsoft Edge", "Microsoft Edge"),
            ("Vivaldi", "Vivaldi"),
            ("Opera", "com.operasoftware.Opera"),
            ("Arc", "Arc/User Data"),
        ] {
            out.push((
                n,
                Family::Chromium,
                sup.join(d),
                sup.join(d).join("NativeMessagingHosts"),
            ));
        }
        out.push((
            "Firefox",
            Family::Firefox,
            sup.join("Firefox"),
            sup.join("Mozilla/NativeMessagingHosts"),
        ));
        out.push((
            "LibreWolf",
            Family::Firefox,
            sup.join("librewolf"),
            sup.join("librewolf/NativeMessagingHosts"),
        ));
    } else if cfg!(target_os = "linux") {
        let cfg = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| h.join(".config"));
        for (n, d) in [
            ("Google Chrome", "google-chrome"),
            ("Chromium", "chromium"),
            ("Brave", "BraveSoftware/Brave-Browser"),
            ("Microsoft Edge", "microsoft-edge"),
            ("Vivaldi", "vivaldi"),
            ("Opera", "opera"),
        ] {
            out.push((
                n,
                Family::Chromium,
                cfg.join(d),
                cfg.join(d).join("NativeMessagingHosts"),
            ));
        }
        for (n, app, d) in [
            ("Chromium (Flatpak)", "org.chromium.Chromium", "chromium"),
            (
                "Brave (Flatpak)",
                "com.brave.Browser",
                "BraveSoftware/Brave-Browser",
            ),
        ] {
            let root = h.join(".var/app").join(app).join("config").join(d);
            out.push((
                n,
                Family::Chromium,
                root.clone(),
                root.join("NativeMessagingHosts"),
            ));
        }
        out.push((
            "Firefox",
            Family::Firefox,
            h.join(".mozilla/firefox"),
            h.join(".mozilla/native-messaging-hosts"),
        ));
        out.push((
            "Firefox (Snap)",
            Family::Firefox,
            h.join("snap/firefox/common/.mozilla/firefox"),
            h.join("snap/firefox/common/.mozilla/native-messaging-hosts"),
        ));
        out.push((
            "LibreWolf",
            Family::Firefox,
            h.join(".librewolf"),
            h.join(".librewolf/native-messaging-hosts"),
        ));
    }
    out
}

/// A tiny launcher the browser runs: `otm native-host` with this exact binary.
pub fn launcher() -> std::io::Result<PathBuf> {
    let dir = paths::data_dir().join("bin");
    std::fs::create_dir_all(&dir)?;
    let me = std::env::current_exe()?;
    if cfg!(windows) {
        // Chromium supplies its origin; Firefox supplies the manifest path.
        // main.rs recognizes both and enters native-host mode directly.
        Ok(me)
    } else {
        let p = dir.join("otm-native-host");
        let quoted = format!("'{}'", me.display().to_string().replace('\'', "'\\''"));
        std::fs::write(&p, format!("#!/bin/sh\nexec {quoted} native-host \"$@\"\n"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755))?;
        }
        Ok(p)
    }
}

pub fn manifest(family: Family, launcher: &std::path::Path) -> serde_json::Value {
    let mut m = json!({"name": HOST_NAME, "description": "Open Transfer Manager browser bridge",
                       "path": launcher.to_string_lossy(), "type": "stdio"});
    match family {
        Family::Chromium => {
            let mut ids: Vec<String> = CHROMIUM_IDS.iter().map(|id| id.to_string()).collect();
            if let Some(id) = chrome_store_id()
                && !ids.contains(&id)
            {
                ids.push(id);
            }
            m["allowed_origins"] = json!(
                ids.iter()
                    .map(|i| format!("chrome-extension://{i}/"))
                    .collect::<Vec<_>>()
            )
        }
        Family::Firefox => m["allowed_extensions"] = json!(FIREFOX_IDS),
    }
    m
}

/// Write the bridge file for installed browsers. Returns (browser, file).
pub fn register(force: bool) -> std::io::Result<Vec<(String, PathBuf)>> {
    let l = launcher()?;
    #[cfg(windows)]
    {
        return register_windows(&l, force);
    }
    #[allow(unreachable_code)]
    {
        let mut done = Vec::new();
        for (name, fam, root, folder) in targets() {
            if !force && !root.is_dir() {
                continue;
            }
            std::fs::create_dir_all(&folder)?;
            let path = folder.join(format!("{HOST_NAME}.json"));
            std::fs::write(
                &path,
                serde_json::to_string_pretty(&manifest(fam, &l)).unwrap_or_default(),
            )?;
            done.push((name.to_string(), path));
        }
        Ok(done)
    }
}

pub fn unregister() -> Vec<String> {
    let mut removed = Vec::new();
    for (name, _, _, folder) in targets() {
        let path = folder.join(format!("{HOST_NAME}.json"));
        if std::fs::remove_file(&path).is_ok() {
            removed.push(name.to_string());
        }
    }
    #[cfg(windows)]
    for (name, _, key) in WIN_KEYS {
        let _ = paths::background_command("reg.exe")
            .args(["delete", &format!("HKCU\\{key}\\{HOST_NAME}"), "/f"])
            .output();
        removed.push(name.to_string());
    }
    for suffix in ["chromium", "firefox"] {
        let _ = std::fs::remove_file(paths::data_dir().join(format!("{HOST_NAME}.{suffix}.json")));
    }
    let _ = std::fs::remove_file(paths::data_dir().join("browser-registration.json"));
    removed
}

/// (browser, installed, registered)
pub fn status() -> Vec<(String, bool, bool)> {
    let mut out = Vec::new();
    for (name, _, root, folder) in targets() {
        let installed = root.is_dir();
        let registered = folder.join(format!("{HOST_NAME}.json")).exists();
        if installed || registered {
            out.push((name.to_string(), installed, registered));
        }
    }
    #[cfg(windows)]
    for (name, _, key) in WIN_KEYS {
        if !windows_installed(name) {
            continue;
        }
        let suffix = if *name == "Firefox" {
            "firefox"
        } else {
            "chromium"
        };
        let path = paths::data_dir().join(format!("{HOST_NAME}.{suffix}.json"));
        // No shell/process polling on the UI thread. Verify the manifest on disk;
        // actual extension connection is intentionally not claimed here.
        let ok = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .is_some_and(|m| {
                m["path"]
                    .as_str()
                    .is_some_and(|p| std::path::Path::new(p).is_file())
            });
        let _ = key;
        out.push((name.to_string(), true, ok));
    }
    out
}

#[cfg(windows)]
const WIN_KEYS: &[(&str, Family, &str)] = &[
    (
        "Google Chrome",
        Family::Chromium,
        r"Software\Google\Chrome\NativeMessagingHosts",
    ),
    (
        "Microsoft Edge",
        Family::Chromium,
        r"Software\Microsoft\Edge\NativeMessagingHosts",
    ),
    (
        "Brave",
        Family::Chromium,
        r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts",
    ),
    (
        "Firefox",
        Family::Firefox,
        r"Software\Mozilla\NativeMessagingHosts",
    ),
];

#[cfg(windows)]
fn register_windows(l: &std::path::Path, force: bool) -> std::io::Result<Vec<(String, PathBuf)>> {
    register_windows_scope(l, force, false)
}

#[cfg(windows)]
fn register_windows_scope(
    l: &std::path::Path,
    force: bool,
    machine: bool,
) -> std::io::Result<Vec<(String, PathBuf)>> {
    let mut done = Vec::new();
    let hive = if machine { "HKLM" } else { "HKCU" };
    let dir = if machine {
        l.parent()
            .ok_or_else(|| std::io::Error::other("Executable folder missing"))?
            .join("native-hosts")
    } else {
        paths::data_dir()
    };
    std::fs::create_dir_all(&dir)?;
    for (name, fam, key) in WIN_KEYS {
        if !force && !windows_installed(name) {
            continue;
        }
        let suffix = if *fam == Family::Firefox {
            "firefox"
        } else {
            "chromium"
        };
        let path = dir.join(format!("{HOST_NAME}.{suffix}.json"));
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&manifest(*fam, l)).unwrap_or_default(),
        )?;
        let result = paths::background_command("reg.exe")
            .args([
                "add",
                &format!("{hive}\\{key}\\{HOST_NAME}"),
                "/ve",
                "/t",
                "REG_SZ",
                "/d",
                &path.to_string_lossy(),
                "/f",
            ])
            .output()?;
        if !result.status.success() {
            return Err(std::io::Error::other(format!(
                "Could not register {name}: {}",
                String::from_utf8_lossy(&result.stderr)
            )));
        }
        done.push((name.to_string(), path));
    }
    Ok(done)
}

pub fn register_machine() -> std::io::Result<Vec<(String, PathBuf)>> {
    #[cfg(windows)]
    {
        register_windows_scope(&std::env::current_exe()?, true, true)
    }
    #[cfg(not(windows))]
    {
        Err(std::io::Error::other(
            "Machine registration is Windows-only",
        ))
    }
}

pub fn unregister_machine() -> std::io::Result<()> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe()?;
        let dir = exe
            .parent()
            .ok_or_else(|| std::io::Error::other("Executable folder missing"))?
            .join("native-hosts");
        for (_, _, key) in WIN_KEYS {
            let _ = paths::background_command("reg.exe")
                .args(["delete", &format!("HKLM\\{key}\\{HOST_NAME}"), "/f"])
                .output()?;
        }
        for suffix in ["chromium", "firefox"] {
            let _ = std::fs::remove_file(dir.join(format!("{HOST_NAME}.{suffix}.json")));
        }
        // Only remove this known generated directory when empty.
        let _ = std::fs::remove_dir(dir);
    }
    Ok(())
}

#[cfg(windows)]
fn windows_installed(name: &str) -> bool {
    let relative = match name {
        "Google Chrome" => "Google/Chrome/User Data",
        "Microsoft Edge" => "Microsoft/Edge/User Data",
        "Brave" => "BraveSoftware/Brave-Browser/User Data",
        "Firefox" => "Mozilla/Firefox",
        _ => return false,
    };
    let env = if name == "Firefox" {
        "APPDATA"
    } else {
        "LOCALAPPDATA"
    };
    std::env::var_os(env).is_some_and(|base| PathBuf::from(base).join(relative).is_dir())
}

/// Installed bundles live beside the executable; source runs use dist/extension.
pub fn extension_dir() -> PathBuf {
    let near = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join("extensions")));
    near.filter(|p| p.is_dir())
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("dist/extension"))
}

/// Register once per executable/browser set, and repair missing manifests at startup.
pub fn ensure_registered() -> std::io::Result<()> {
    let marker = paths::data_dir().join("browser-registration.json");
    let fingerprint = format!(
        "{}|{}|{:?}",
        std::env::current_exe()?.display(),
        env!("CARGO_PKG_VERSION"),
        status()
            .iter()
            .map(|(n, i, r)| (n.clone(), *i, *r))
            .collect::<Vec<_>>()
    );
    if std::fs::read_to_string(&marker).ok().as_deref() != Some(&fingerprint) {
        register(false)?;
        let fingerprint = format!(
            "{}|{}|{:?}",
            std::env::current_exe()?.display(),
            env!("CARGO_PKG_VERSION"),
            status()
                .iter()
                .map(|(n, i, r)| (n.clone(), *i, *r))
                .collect::<Vec<_>>()
        );
        std::fs::write(marker, fingerprint)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chrome_distribution_ids_reject_paths_and_invalid_identifiers() {
        assert!(valid_chrome_id(CHROMIUM_IDS[0]));
        for bad in [
            "",
            "../chromium",
            "https://example.com",
            "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
            "IAAAIOGFBKPIDDHONHFNEKDFFILOHJJC",
        ] {
            assert!(!valid_chrome_id(bad));
        }
    }

    #[test]
    fn manifests() {
        let m = manifest(Family::Chromium, std::path::Path::new("/x/otm-native-host"));
        assert!(
            m["allowed_origins"][0]
                .as_str()
                .unwrap()
                .starts_with("chrome-extension://")
        );
        let f = manifest(Family::Firefox, std::path::Path::new("/x/otm-native-host"));
        assert_eq!(f["allowed_extensions"][0], "otm@opentransfermanager.org");
        assert_eq!(f["type"], "stdio");
    }
}
