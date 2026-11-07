//! `otm install-tools`: everything the app needs besides itself, in one step.
//!
//! - yt-dlp (videos, MP3) — official build, see `media::install_or_update`
//! - rclone (Send, Transfer) — official build checked against its SHA256SUMS
//! - ffmpeg + ffprobe (MP3, best video quality) — kept if already installed (package
//!   manager, Homebrew, winget); otherwise on Windows and Linux the static build of
//!   BtbN/FFmpeg-Builds, checked against its published checksums.
//!
//! The installers (Windows setup, install.sh) run this, so one command gives every feature.

use crate::{media, paths, rclone};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const FFMPEG_RELEASE: &str = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest";

pub struct Report {
    pub tool: &'static str,
    pub result: Result<String, String>,
}

/// Install or update every helper program. `ffmpeg: false` leaves ffmpeg alone.
pub fn install_all(ffmpeg: bool, mut on_step: impl FnMut(&str)) -> Vec<Report> {
    let mut out = Vec::new();
    on_step("yt-dlp (videos and MP3)");
    out.push(Report {
        tool: "yt-dlp",
        result: media::install_or_update(),
    });
    on_step("rclone (Send and Transfer)");
    out.push(Report {
        tool: "rclone",
        result: rclone::install_or_update(),
    });
    if ffmpeg {
        on_step("ffmpeg (MP3 and best video quality)");
        out.push(Report {
            tool: "ffmpeg",
            result: install_ffmpeg(),
        });
    }
    out
}

/// The BtbN build for this system, if there is one.
pub fn ffmpeg_asset() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("ffmpeg-master-latest-win64-gpl.zip"),
        ("windows", "aarch64") => Some("ffmpeg-master-latest-winarm64-gpl.zip"),
        ("linux", "x86_64") => Some("ffmpeg-master-latest-linux64-gpl.tar.xz"),
        ("linux", "aarch64") => Some("ffmpeg-master-latest-linuxarm64-gpl.tar.xz"),
        _ => None,
    }
}

pub fn install_ffmpeg() -> Result<String, String> {
    if let Some(p) = media::ffmpeg_path()
        && paths::find_program("ffprobe").is_some()
    {
        return Ok(format!("ffmpeg already installed ({})", p.display()));
    }
    let Some(asset) = ffmpeg_asset() else {
        return Err(if cfg!(target_os = "macos") {
            "Install ffmpeg with Homebrew: brew install ffmpeg".into()
        } else {
            "Install ffmpeg with your package manager.".into()
        });
    };
    let sums = String::from_utf8_lossy(&get_small(&format!("{FFMPEG_RELEASE}/checksums.sha256"))?)
        .to_string();
    let want =
        rclone::checksum_for(&sums, asset).ok_or(format!("No checksum published for {asset}"))?;
    let work = paths::tools_dir().join("ffmpeg-download");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let archive = work.join(asset);
    let got = download_hashed(&format!("{FFMPEG_RELEASE}/{asset}"), &archive)?;
    if got != want {
        let _ = std::fs::remove_dir_all(&work);
        return Err(format!(
            "Checksum mismatch for {asset}; nothing was installed."
        ));
    }
    let exe = |n: &str| {
        if cfg!(windows) {
            format!("{n}.exe")
        } else {
            n.to_string()
        }
    };
    let names = [exe("ffmpeg"), exe("ffprobe")];
    let result = if asset.ends_with(".zip") {
        extract_zip(&archive, &names)
    } else {
        extract_tar_xz(&archive, &work, &names)
    };
    let _ = std::fs::remove_dir_all(&work);
    result?;
    Ok(format!(
        "ffmpeg installed in {}",
        paths::tools_dir().display()
    ))
}

fn get_small(url: &str) -> Result<Vec<u8>, String> {
    let resp = ureq::get(url)
        .call()
        .map_err(|e| format!("Download failed ({url}): {e}"))?;
    let mut buf = Vec::new();
    resp.into_body()
        .into_reader()
        .take(1 << 20)
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;
    Ok(buf)
}

/// Stream `url` to `file` (no large buffer in memory) and return its SHA-256 (hex).
fn download_hashed(url: &str, file: &Path) -> Result<String, String> {
    let resp = ureq::get(url)
        .call()
        .map_err(|e| format!("Download failed ({url}): {e}"))?;
    let mut reader = resp.into_body().into_reader();
    let mut out = std::fs::File::create(file).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("Download failed: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
    }
    out.flush().map_err(|e| e.to_string())?;
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn place(from: &mut impl Read, name: &str) -> Result<(), String> {
    let target = paths::tools_dir().join(name);
    let tmp = target.with_extension("part");
    {
        let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        std::io::copy(from, &mut f).map_err(|e| e.to_string())?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
    }
    std::fs::rename(&tmp, &target).map_err(|e| e.to_string())
}

fn extract_zip(archive: &Path, names: &[String]) -> Result<(), String> {
    let f = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
    for name in names {
        let index = (0..zip.len())
            .find(|&i| {
                zip.by_index(i)
                    .ok()
                    .and_then(|e| e.name().ok().map(|n| n.ends_with(&format!("/bin/{name}"))))
                    .unwrap_or(false)
            })
            .ok_or(format!("{name} not found in the archive"))?;
        let mut entry = zip.by_index(index).map_err(|e| e.to_string())?;
        place(&mut entry, name)?;
    }
    Ok(())
}

/// Linux builds are .tar.xz: the system `tar` (with xz) unpacks them.
fn extract_tar_xz(archive: &Path, work: &Path, names: &[String]) -> Result<(), String> {
    let status = std::process::Command::new("tar")
        .arg("-xJf")
        .arg(archive)
        .arg("-C")
        .arg(work)
        .status()
        .map_err(|e| format!("tar is needed to unpack ffmpeg: {e}"))?;
    if !status.success() {
        return Err("Could not unpack ffmpeg (is xz installed?).".into());
    }
    for name in names {
        let found = find_file(work, name).ok_or(format!("{name} not found in the archive"))?;
        let mut f = std::fs::File::open(found).map_err(|e| e.to_string())?;
        place(&mut f, name)?;
    }
    Ok(())
}

fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).ok()?.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().is_some_and(|n| n == name)
                && p.parent().is_some_and(|parent| parent.ends_with("bin"))
            {
                return Some(p);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffmpeg_build_for_this_system() {
        let a = ffmpeg_asset();
        if cfg!(all(windows, target_arch = "x86_64")) {
            assert_eq!(a, Some("ffmpeg-master-latest-win64-gpl.zip"));
        }
        if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            assert_eq!(a, Some("ffmpeg-master-latest-linux64-gpl.tar.xz"));
        }
        if cfg!(target_os = "macos") {
            assert_eq!(a, None, "macOS uses Homebrew");
        }
    }

    #[test]
    fn finds_binaries_only_in_bin_folders() {
        let d = std::env::temp_dir().join(format!("otm-tools-{}", std::process::id()));
        std::fs::create_dir_all(d.join("ffmpeg-x/bin")).unwrap();
        std::fs::create_dir_all(d.join("ffmpeg-x/doc")).unwrap();
        std::fs::write(d.join("ffmpeg-x/doc/ffmpeg"), "doc").unwrap();
        std::fs::write(d.join("ffmpeg-x/bin/ffmpeg"), "bin").unwrap();
        let f = find_file(&d, "ffmpeg").unwrap();
        assert_eq!(std::fs::read_to_string(f).unwrap(), "bin");
        let _ = std::fs::remove_dir_all(d);
    }
}
