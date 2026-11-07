//! The upload engine: rclone (MIT), run as a separate process like yt-dlp.
//!
//! rclone speaks SFTP, FTP, WebDAV, S3 and 70+ other storage systems. OTM gives it the
//! destination through `RCLONE_CONFIG_OTM_*` environment variables (see `connectors`) and
//! reads progress from its JSON log, so the user's own rclone setup is never touched.

use crate::connectors::Connector;
use crate::{paths, secrets};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

pub fn path() -> Option<PathBuf> {
    paths::find_program("rclone")
}

pub fn version() -> Option<String> {
    let out = base_command(&path()?).arg("version").output().ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .map(|l| l.trim().trim_start_matches("rclone ").to_string())
        .filter(|s| !s.is_empty())
}

fn base_command(program: &Path) -> Command {
    let mut c = paths::background_command(program);
    // An empty config of our own: the user's rclone.conf (and its remotes) stay untouched.
    let conf = paths::data_dir().join("rclone.conf");
    if !conf.exists() {
        let _ = std::fs::write(&conf, "");
    }
    c.env("RCLONE_CONFIG", conf)
        .env("RCLONE_ASK_PASSWORD", "false")
        .stdin(Stdio::null());
    c
}

// ------------------------------------------------------------------ install

fn release_os_arch() -> Result<(&'static str, &'static str), String> {
    let os = match std::env::consts::OS {
        "windows" => "windows",
        "macos" => "osx",
        "linux" => "linux",
        "freebsd" => "freebsd",
        o => {
            return Err(format!(
                "No rclone build for {o}; install it with your package manager."
            ));
        }
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "x86" => "386",
        "arm" => "arm-v7",
        a => {
            return Err(format!(
                "No rclone build for {a}; install it with your package manager."
            ));
        }
    };
    Ok((os, arch))
}

fn get_bytes(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let resp = ureq::get(url)
        .call()
        .map_err(|e| format!("Download failed ({url}): {e}"))?;
    let mut buf = Vec::new();
    resp.into_body()
        .into_reader()
        .take(limit)
        .read_to_end(&mut buf)
        .map_err(|e| format!("Download failed ({url}): {e}"))?;
    Ok(buf)
}

/// The line of rclone's SHA256SUMS file that belongs to `file`.
pub fn checksum_for(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        let hash = it.next()?;
        let name = it.next()?.trim_start_matches('*');
        (name == file && hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| hash.to_ascii_lowercase())
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Download the official rclone build into OTM's tools folder (checked against the
/// SHA256SUMS published with it), or say it is already up to date.
pub fn install_or_update() -> Result<String, String> {
    let latest = String::from_utf8_lossy(&get_bytes(
        "https://downloads.rclone.org/version.txt",
        1024,
    )?)
    .trim()
    .trim_start_matches("rclone ")
    .to_string();
    if !latest.starts_with('v') || latest.contains(['/', '\\', ' ']) {
        return Err(format!("Unexpected rclone version: {latest}"));
    }
    if version().as_deref() == Some(latest.as_str()) {
        return Ok(format!("rclone {latest} is up to date."));
    }
    let (os, arch) = release_os_arch()?;
    let file = format!("rclone-{latest}-{os}-{arch}.zip");
    let base = format!("https://downloads.rclone.org/{latest}");
    let sums =
        String::from_utf8_lossy(&get_bytes(&format!("{base}/SHA256SUMS"), 1 << 20)?).to_string();
    let want = checksum_for(&sums, &file).ok_or(format!("No checksum published for {file}"))?;
    let zip_bytes = get_bytes(&format!("{base}/{file}"), 200 << 20)?;
    let got = hex(&Sha256::digest(&zip_bytes));
    if got != want {
        return Err(format!(
            "Checksum mismatch for {file}; nothing was installed."
        ));
    }
    let exe = if cfg!(windows) {
        "rclone.exe"
    } else {
        "rclone"
    };
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|e| e.to_string())?;
    let index = (0..archive.len())
        .find(|&i| {
            archive
                .by_index(i)
                .ok()
                .and_then(|f| f.name().ok().map(|n| n.rsplit('/').next() == Some(exe)))
                .unwrap_or(false)
        })
        .ok_or("rclone program not found in the archive")?;
    let target = paths::tools_dir().join(exe);
    let tmp = target.with_extension("download");
    {
        let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let mut out = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        out.flush().map_err(|e| e.to_string())?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
    }
    std::fs::rename(&tmp, &target).map_err(|e| e.to_string())?;
    Ok(format!("rclone {latest} installed in {}", target.display()))
}

// ------------------------------------------------------------------ connection settings

fn obscure(program: &Path, plain: &str) -> Result<String, String> {
    let mut child = base_command(program)
        .args(["obscure", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(plain.as_bytes())
            .map_err(|e| e.to_string())?;
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() && !s.is_empty() {
        Ok(s)
    } else {
        Err("rclone could not prepare the password".into())
    }
}

/// A ready-to-run rclone and the environment that defines the `otm:` remote.
pub struct Remote {
    program: PathBuf,
    env: Vec<(String, String)>,
}

impl Remote {
    /// `typed_secret`: what the user just typed (testing before saving). `None` reads
    /// the saved one from the keychain.
    pub fn new(c: &Connector, typed_secret: Option<&str>) -> Result<Self, String> {
        let program = path().ok_or("The upload engine (rclone) is not installed.")?;
        let secret = match typed_secret {
            Some(s) => Some(s.to_string()).filter(|s| !s.is_empty()),
            None => secrets::get(&c.secret_key()),
        };
        c.validate(secret.is_some())?;
        let secret = match secret {
            Some(s) if c.needs_obscured_secret() => Some(obscure(&program, &s)?),
            s => s,
        };
        Ok(Remote {
            env: c.rclone_env(secret.as_deref()),
            program,
        })
    }

    fn command(&self) -> Command {
        let mut c = base_command(&self.program);
        c.envs(self.env.iter().map(|(k, v)| (k, v))).args([
            "--contimeout",
            "20s",
            "--timeout",
            "2m",
            "--retries",
            "2",
        ]);
        c
    }

    /// List the destination folder; then, where the server reports it, the free space.
    pub fn test(&self, c: &Connector) -> Result<String, String> {
        let folder = c.remote_folder();
        let out = self
            .command()
            .args([
                "lsf",
                "--max-depth",
                "1",
                "--low-level-retries",
                "1",
                "--retries",
                "1",
            ])
            .arg(&folder)
            .output()
            .map_err(|e| e.to_string())?;
        let stderr = String::from_utf8_lossy(&out.stderr);
        let mut text = if out.status.success() {
            let n = String::from_utf8_lossy(&out.stdout).lines().count();
            format!(
                "Connected · {n} item{} in the folder",
                if n == 1 { "" } else { "s" }
            )
        } else if stderr.contains("directory not found") {
            "Connected · the folder will be created on the first upload".to_string()
        } else {
            return Err(readable_error(&stderr));
        };
        if let Ok(o) = self
            .command()
            .args(["about", "--json"])
            .arg(&folder)
            .output()
            && o.status.success()
            && let Ok(v) = serde_json::from_slice::<Value>(&o.stdout)
            && let Some(free) = v["free"].as_u64()
        {
            text += &format!(" · {} free", crate::model::fmt_size(Some(free)));
        }
        Ok(text)
    }

    /// Copy a file (or a folder, with its content) to `remote`, reporting progress.
    /// Returns when done, failed, or `stop` is set (then the process is killed).
    pub fn upload(
        &self,
        local: &Path,
        remote: &str,
        stop: &AtomicBool,
        mut progress: impl FnMut(Stats),
    ) -> Result<(), String> {
        let mut cmd = self.command();
        cmd.arg(if local.is_dir() { "copy" } else { "copyto" })
            .arg(local)
            .arg(remote)
            .args([
                "--use-json-log",
                "--stats",
                "1s",
                "--stats-log-level",
                "NOTICE",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Could not start rclone: {e}"))?;
        let stderr = child.stderr.take().ok_or("no rclone output")?;
        let (tx, rx) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut last_error = None;
        loop {
            if stop.load(Ordering::SeqCst) {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Stopped".into());
            }
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(line) => {
                    if let Some(s) = parse_stats(&line) {
                        progress(s);
                    } else if let Some(e) = parse_error(&line) {
                        last_error = Some(e);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        let status = child.wait().map_err(|e| e.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(readable_error(
                &last_error.unwrap_or_else(|| format!("rclone exited with {status}")),
            ))
        }
    }

    /// A public / shareable link, where the destination can make one (S3 presigned
    /// links, WebDAV shares on Nextcloud…). `None` when it can't.
    pub fn link(&self, remote: &str) -> Option<String> {
        let out = self
            .command()
            .args(["link", "--expire", "7d"])
            .arg(remote)
            .output()
            .ok()?;
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (out.status.success() && s.starts_with("http")).then_some(s)
    }
}

// ------------------------------------------------------------------ log parsing

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub bytes: u64,
    pub total: u64,
    pub speed: f64,
    pub eta: Option<f64>,
}

/// Progress from one line of `--use-json-log` output.
pub fn parse_stats(line: &str) -> Option<Stats> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    let s = v.get("stats")?;
    Some(Stats {
        bytes: s["bytes"].as_u64().unwrap_or(0),
        total: s["totalBytes"].as_u64().unwrap_or(0),
        speed: s["speed"].as_f64().unwrap_or(0.0),
        eta: s["eta"].as_f64(),
    })
}

/// Error message from a log line, if the line reports one.
pub fn parse_error(line: &str) -> Option<String> {
    let line = line.trim();
    if let Ok(v) = serde_json::from_str::<Value>(line) {
        let level = v["level"].as_str().unwrap_or("");
        return matches!(level, "error" | "critical" | "fatal")
            .then(|| v["msg"].as_str().unwrap_or("").trim().to_string())
            .filter(|m| !m.is_empty());
    }
    // before JSON logging starts (bad flags, config problems) rclone prints plain text
    (line.contains("ERROR") || line.contains("CRITICAL") || line.starts_with("Failed"))
        .then(|| line.to_string())
}

/// Turn rclone / server errors into one short sentence.
pub fn readable_error(raw: &str) -> String {
    let last = raw
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("Upload failed");
    let low = raw.to_lowercase();
    let hint = if low.contains("unable to authenticate")
        || low.contains("authentication failed")
        || low.contains("530 ")
        || low.contains("401 unauthorized")
        || low.contains("invalidaccesskeyid")
        || low.contains("signaturedoesnotmatch")
        || low.contains("login incorrect")
    {
        Some("Sign-in refused: check the user name and password or key.")
    } else if low.contains("no such host") || low.contains("name resolution") {
        Some("Server not found: check the host name.")
    } else if low.contains("connection refused") {
        Some("The server refused the connection: check the port.")
    } else if low.contains("i/o timeout")
        || low.contains("timed out")
        || low.contains("deadline exceeded")
    {
        Some("The server did not answer in time.")
    } else if low.contains("permission denied")
        || low.contains("403 forbidden")
        || low.contains("accessdenied")
    {
        Some("Permission denied on the server folder.")
    } else if low.contains("quota")
        || low.contains("insufficient storage")
        || low.contains("no space")
    {
        Some("Not enough space on the destination.")
    } else if low.contains("nosuchbucket") {
        Some("Bucket not found: check the bucket name.")
    } else if low.contains("certificate") || low.contains("x509") {
        Some("The server's TLS certificate is not trusted.")
    } else {
        None
    };
    match hint {
        Some(h) => h.to_string(),
        None => {
            let l = last
                .trim_start_matches("Failed to copy:")
                .trim_start_matches("Failed to copyto:")
                .trim();
            let mut s: String = l.chars().take(220).collect();
            if l.chars().count() > 220 {
                s.push('…');
            }
            s
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_and_errors_from_json_log() {
        let l = r#"{"level":"notice","msg":"\nTransferred: 1 MiB / 5 MiB","source":"accounting/stats.go:482","stats":{"bytes":1048576,"checks":0,"elapsedTime":1.0,"errors":0,"eta":4,"speed":1048576.5,"totalBytes":5242880,"transfers":0},"time":"2026-11-07T15:00:00Z"}"#;
        assert_eq!(
            parse_stats(l),
            Some(Stats {
                bytes: 1048576,
                total: 5242880,
                speed: 1048576.5,
                eta: Some(4.0)
            })
        );
        assert_eq!(parse_error(l), None);
        let e = r#"{"level":"error","msg":"Failed to copy: sftp: \"permission denied\" (SSH_FX_PERMISSION_DENIED)","object":"a.zip","time":"x"}"#;
        assert!(parse_error(e).unwrap().contains("permission denied"));
        assert_eq!(parse_stats(e), None);
        assert!(parse_error("2026/11/07 ERROR : bad flag").is_some());
        assert_eq!(parse_error("some info"), None);
    }

    #[test]
    fn errors_become_short_sentences() {
        assert_eq!(
            readable_error(
                "Failed to copy: couldn't connect SSH: ssh: handshake failed: ssh: unable to authenticate"
            ),
            "Sign-in refused: check the user name and password or key."
        );
        assert_eq!(
            readable_error("dial tcp: lookup nope.invalid: no such host"),
            "Server not found: check the host name."
        );
        assert_eq!(
            readable_error("Failed to copy: something odd"),
            "something odd"
        );
    }

    #[test]
    fn checksum_lines() {
        let sums = "-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA1\n\n\
            0a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f9  rclone-v1.71.2-windows-amd64.zip\n\
            ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff  rclone-v1.71.2-linux-amd64.zip\n";
        assert_eq!(
            checksum_for(sums, "rclone-v1.71.2-windows-amd64.zip").as_deref(),
            Some("0a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f9")
        );
        assert_eq!(checksum_for(sums, "rclone-v1.71.2-osx-arm64.zip"), None);
        assert_eq!(hex(&Sha256::digest(b"abc"))[..8], *"ba7816bf");
    }
}
