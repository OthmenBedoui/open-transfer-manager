//! Small native launcher for Inno Setup's generated uninstaller. No shell/console.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::{Path, PathBuf};

fn uninstaller_path(executable: &Path) -> Result<PathBuf, String> {
    let dir = executable
        .parent()
        .ok_or("Dossier d’installation introuvable.")?;
    let path = dir.join("unins000.exe");
    if !path.is_file() || !dir.join("otm.exe").is_file() {
        return Err("Désinstalleur introuvable. Utilisez Paramètres Windows → Applications → Open Transfer Manager.".into());
    }
    Ok(path)
}

#[cfg(windows)]
fn registered_install_dir(hive: isize) -> Option<PathBuf> {
    use std::ffi::{OsString, c_void};
    use std::os::windows::ffi::OsStringExt;
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegGetValueW(
            hive: *mut c_void,
            subkey: *const u16,
            value: *const u16,
            flags: u32,
            kind: *mut u32,
            data: *mut c_void,
            bytes: *mut u32,
        ) -> i32;
    }
    let key: Vec<u16> = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{F390B4CB-96AC-4D1E-9302-4CC6F06443BB}_is1"
        .encode_utf16().chain(Some(0)).collect();
    let name: Vec<u16> = "InstallLocation".encode_utf16().chain(Some(0)).collect();
    let mut buffer = vec![0u16; 32768];
    let mut bytes = (buffer.len() * 2) as u32;
    // SAFETY: predefined HKEY, nul-terminated inputs and correctly sized writable
    // UTF-16 buffer. Restrict to REG_SZ and the installer's 64-bit registry view.
    let result = unsafe {
        RegGetValueW(
            hive as *mut c_void,
            key.as_ptr(),
            name.as_ptr(),
            0x0001_0002,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if result != 0 {
        return None;
    }
    let end = buffer.iter().position(|c| *c == 0)?;
    Some(PathBuf::from(OsString::from_wide(&buffer[..end])))
}

fn resolve_uninstaller(executable: &Path) -> Result<PathBuf, String> {
    if let Ok(path) = uninstaller_path(executable) {
        return Ok(path);
    }
    #[cfg(windows)]
    for hive in [-2_147_483_647isize, -2_147_483_646isize] {
        // HKCU, HKLM
        if let Some(dir) = registered_install_dir(hive)
            && let Ok(path) = uninstaller_path(&dir.join("otm.exe"))
        {
            return Ok(path);
        }
    }
    Err("Installation OTM introuvable. Utilisez Paramètres Windows → Applications.".into())
}

#[cfg(windows)]
fn show_error(message: &str) {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(
            window: *mut std::ffi::c_void,
            text: *const u16,
            caption: *const u16,
            flags: u32,
        ) -> i32;
    }
    let text: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    let title: Vec<u16> = "Désinstaller Open Transfer Manager"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    // SAFETY: null owner and valid nul-terminated UTF-16 buffers live through this call.
    unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), 0x10);
    }
}

#[cfg(not(windows))]
fn show_error(message: &str) {
    eprintln!("{message}");
}

fn main() {
    let result = std::env::current_exe()
        .map_err(|e| e.to_string())
        .and_then(|exe| resolve_uninstaller(&exe))
        .and_then(|path| {
            if std::env::args().any(|arg| arg == "--print-path") {
                println!("{}", path.display());
                return Ok(());
            }
            std::process::Command::new(path)
                .args(std::env::args_os().skip(1))
                .spawn()
                .map(|_| ())
                .map_err(|e| e.to_string())
        });
    match result {
        // Exit immediately so the uninstaller can delete this launcher too.
        Ok(()) => {}
        Err(error) => {
            show_error(&error);
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolves_only_the_installed_uninstaller_next_to_launcher() {
        let dir = std::env::temp_dir().join(format!("otm-uninstall-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let launcher = dir.join("Desinstaller OTM.exe");
        assert!(uninstaller_path(&launcher).is_err());
        std::fs::write(dir.join("unins000.exe"), b"test").unwrap();
        std::fs::write(dir.join("otm.exe"), b"test").unwrap();
        assert_eq!(
            uninstaller_path(&launcher).unwrap(),
            dir.join("unins000.exe")
        );
        std::fs::remove_file(dir.join("unins000.exe")).unwrap();
        std::fs::remove_file(dir.join("otm.exe")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
