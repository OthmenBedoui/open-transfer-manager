//! Where things live on each OS, and how to open files there.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const APP_NAME: &str = "Open Transfer Manager";
pub const APP_ID: &str = "open-transfer-manager";

/// Background tools must never create a console on Windows, including debug builds.
pub fn background_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = Command::new(program);
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW (keeps native messaging pipes)
        cmd
    }
    #[cfg(not(windows))]
    {
        Command::new(program)
    }
}

pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// Settings, download list and runtime files.
pub fn data_dir() -> PathBuf {
    let dir = if let Ok(over) = std::env::var("OTM_DATA_DIR") {
        PathBuf::from(over)
    } else if cfg!(target_os = "macos") {
        home()
            .join("Library")
            .join("Application Support")
            .join(APP_NAME)
    } else if cfg!(windows) {
        dirs::data_dir()
            .unwrap_or_else(home)
            .join("OpenTransferManager")
    } else {
        dirs::data_dir()
            .unwrap_or_else(|| home().join(".local/share"))
            .join(APP_ID)
    };
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn downloads_dir() -> PathBuf {
    dirs::download_dir().unwrap_or_else(|| home().join("Downloads"))
}

/// Folder with helper programs the app downloads itself (yt-dlp).
pub fn tools_dir() -> PathBuf {
    let d = data_dir().join("tools");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn spawn(cmd: &mut Command) {
    let _ = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

pub fn open_path(path: &Path) {
    let _ = open::that_detached(path);
}

/// Show a file selected in the file manager (or open its folder).
/// Explorer argument that opens the file's folder with the file selected.
/// Only the path may be quoted: `"/select,C:\a b\c.mp3"` (what `Command::arg` produces
/// for a path with spaces) makes Explorer open the Documents folder instead.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn explorer_select_arg(path: &Path) -> String {
    format!(
        "/select,\"{}\"",
        path.display().to_string().replace('/', "\\")
    )
}

pub fn reveal(path: &Path) {
    if path.is_file() {
        if cfg!(target_os = "macos") {
            spawn(Command::new("open").arg("-R").arg(path));
            return;
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            spawn(Command::new("explorer").raw_arg(explorer_select_arg(path)));
            return;
        }
    }
    let folder = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().map(Path::to_path_buf).unwrap_or_default()
    };
    open_path(&folder);
}

/// Desktop notification without extra libraries. Never fails loudly.
pub fn notify(title: &str, message: &str) {
    if cfg!(target_os = "macos") {
        let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            esc(message),
            esc(title)
        );
        spawn(Command::new("osascript").arg("-e").arg(script));
    } else if cfg!(target_os = "linux") {
        spawn(Command::new("notify-send").args(["-a", APP_NAME, title, message]));
    }
}

/// Look for a program next to us, in our tools folder, then in PATH.
pub fn find_program(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    let mut candidates = vec![tools_dir().join(&exe)];
    if let Ok(me) = std::env::current_exe()
        && let Some(dir) = me.parent()
    {
        candidates.push(dir.join(&exe));
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            candidates.push(dir.join(&exe));
        }
    }
    for extra in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
        candidates.push(Path::new(extra).join(&exe));
    }
    candidates.into_iter().find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explorer_gets_the_path_quoted_alone() {
        let p = Path::new(r"C:\Users\x\Downloads\Music\AURORA - Runaway [d_HlPboLRL8] 320k.mp3");
        assert_eq!(
            explorer_select_arg(p),
            r#"/select,"C:\Users\x\Downloads\Music\AURORA - Runaway [d_HlPboLRL8] 320k.mp3""#
        );
        // forward slashes from tools are turned into Windows separators
        assert_eq!(
            explorer_select_arg(Path::new("C:/a b/c.zip")),
            r#"/select,"C:\a b\c.zip""#
        );
    }
}
