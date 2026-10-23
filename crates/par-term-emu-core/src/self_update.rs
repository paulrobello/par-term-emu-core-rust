//! Checksum-required release updates shared by the standalone binaries.

use sha2::Digest;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Standalone binary whose release asset is selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binary {
    /// The streaming server.
    Streamer,
    /// The multiplexer.
    Mux,
}

/// Automatic update cadence. Automatic updates themselves default off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Interval {
    /// Every hour.
    Hourly,
    /// Every day (default).
    #[default]
    Daily,
    /// Every week.
    Weekly,
}

impl Interval {
    /// Seconds between checks.
    pub fn seconds(self) -> u64 {
        match self {
            Self::Hourly => 3600,
            Self::Daily => 86400,
            Self::Weekly => 604800,
        }
    }
}

impl std::str::FromStr for Interval {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, String> {
        match value {
            "hourly" => Ok(Self::Hourly),
            "daily" => Ok(Self::Daily),
            "weekly" => Ok(Self::Weekly),
            _ => Err("expected hourly, daily, or weekly".into()),
        }
    }
}

/// Strict semantic-version comparison, ignoring build metadata.
pub fn newer(tag: &str, current: &str) -> Result<bool, String> {
    let parse =
        |v: &str| semver::Version::parse(v.trim_start_matches('v')).map_err(|e| e.to_string());
    Ok(parse(tag)?.cmp_precedence(&parse(current)?).is_gt())
}

/// Exact names produced by deployment.yml and package_mux_release.py.
pub fn asset_name(binary: Binary, version: &str, os: &str, arch: &str) -> Result<String, String> {
    let target = match (os, arch) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => return Err(format!("no release asset for {os}/{arch}")),
    };
    Ok(match binary {
        Binary::Streamer => format!(
            "par-term-streamer-{os}-{arch}{}",
            if os == "windows" { ".exe" } else { "" }
        ),
        Binary::Mux => format!(
            "par-mux-v{version}-{target}.{}",
            if os == "windows" { "zip" } else { "tar.gz" }
        ),
    })
}

/// Refuse replacing an executable inside a signed application bundle.
pub fn guard_bundle(exe: &Path, force: bool) -> Result<(), String> {
    if !force
        && exe.ancestors().any(|p| {
            p.file_name().is_some_and(|n| n == "Contents")
                && p.parent()
                    .is_some_and(|parent| parent.extension().is_some_and(|ext| ext == "app"))
        })
    {
        return Err("refusing to replace a binary inside an .app bundle: this can break its signature and TCC grant; use --force-self-update explicitly".into());
    }
    Ok(())
}

/// Select a strict sha256sum entry for exactly this asset.
pub fn checksum(text: &str, name: &str) -> Result<String, String> {
    let mut found = None;
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let Some(digest) = fields.next() else {
            continue;
        };
        if fields.next().map(|s| s.trim_start_matches('*')) == Some(name) {
            if found.is_some()
                || digest.len() != 64
                || !digest.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("malformed or duplicate SHA-256 entry".into());
            }
            found = Some(digest.to_ascii_lowercase());
        }
    }
    found.ok_or_else(|| format!("checksum sidecar has no entry for {name}"))
}

fn verify(bytes: &[u8], expected: &str) -> Result<(), String> {
    let actual = sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    if actual != expected {
        return Err(format!(
            "SHA-256 mismatch: expected {expected}, got {actual}; installed binary unchanged"
        ));
    }
    Ok(())
}

/// Verify before staging and atomically replace on Unix, preserving a backup.
/// On Windows the running image must first be renamed, with rollback on failure.
pub fn install_verified(exe: &Path, bytes: &[u8], expected: &str) -> Result<PathBuf, String> {
    verify(bytes, expected)?;
    let parent = exe.parent().ok_or("executable has no parent")?;
    let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    staged.write_all(bytes).map_err(|e| e.to_string())?;
    let permissions = std::fs::metadata(exe)
        .map_err(|e| e.to_string())?
        .permissions();
    staged
        .as_file()
        .set_permissions(permissions)
        .map_err(|e| e.to_string())?;
    staged.as_file().sync_all().map_err(|e| e.to_string())?;
    let backup = exe.with_file_name(format!(
        "{}.previous",
        exe.file_name()
            .ok_or("executable has no name")?
            .to_string_lossy()
    ));
    #[cfg(unix)]
    {
        let mut old = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        std::io::copy(
            &mut std::fs::File::open(exe).map_err(|e| e.to_string())?,
            old.as_file_mut(),
        )
        .map_err(|e| e.to_string())?;
        old.as_file()
            .set_permissions(
                std::fs::metadata(exe)
                    .map_err(|e| e.to_string())?
                    .permissions(),
            )
            .map_err(|e| e.to_string())?;
        old.as_file().sync_all().map_err(|e| e.to_string())?;
        old.persist(&backup).map_err(|e| e.to_string())?;
        staged.persist(exe).map_err(|e| e.to_string())?;
    }
    #[cfg(windows)]
    {
        if backup.exists() {
            std::fs::remove_file(&backup).map_err(|e| e.to_string())?;
        }
        std::fs::rename(exe, &backup).map_err(|e| e.to_string())?;
        if let Err(e) = staged.persist(exe) {
            std::fs::rename(&backup, exe)
                .map_err(|rollback| format!("{e}; rollback failed: {rollback}"))?;
            return Err(e.to_string());
        }
    }
    Ok(backup)
}

/// Fetch an HTTPS resource using the existing mux curl convention, with bounds.
/// Redirects must remain HTTPS, including GitHub's release CDN redirects.
fn fetch(url: &str, cap: u64, directory: Option<&Path>) -> Result<Vec<u8>, String> {
    if !url.starts_with("https://") {
        return Err("update URLs must use HTTPS".into());
    }
    let file = match directory {
        Some(dir) => tempfile::NamedTempFile::new_in(dir),
        None => tempfile::NamedTempFile::new(),
    }
    .map_err(|e| e.to_string())?
    .into_temp_path();
    let status = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--connect-timeout",
            "15",
            "--max-time",
            "180",
            "--max-filesize",
            &cap.to_string(),
            "--user-agent",
            "par-term-release-updater",
            "--output",
        ])
        .arg(&file)
        .arg(url)
        .status()
        .map_err(|e| format!("curl: {e}"))?;
    if !status.success() {
        return Err(format!("release download failed: {status}"));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&file)
        .map_err(|e| e.to_string())?
        .take(cap + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > cap {
        return Err("release response exceeds download cap".into());
    }
    Ok(bytes)
}

/// Result of a manual or automatic update check.
#[derive(Debug)]
pub struct Outcome {
    /// Human-readable result.
    pub report: String,
    /// Whether the installed executable changed and needs restarting.
    pub installed: bool,
}

/// Check latest stable GitHub release and optionally install it. A missing
/// sidecar always fails closed, including releases predating this updater.
pub fn update(
    binary: Binary,
    current: &str,
    exe: &Path,
    check: bool,
    force: bool,
) -> Result<Outcome, String> {
    update_channel(binary, current, exe, check, force, false)
}

/// As [`update`], optionally selecting the newest preview release.
pub fn update_channel(
    binary: Binary,
    current: &str,
    exe: &Path,
    check: bool,
    force: bool,
    preview: bool,
) -> Result<Outcome, String> {
    let endpoint = if preview {
        "https://api.github.com/repos/paulrobello/par-term-emu-core-rust/releases"
    } else {
        "https://api.github.com/repos/paulrobello/par-term-emu-core-rust/releases/latest"
    };
    let bytes = fetch(endpoint, 2 * 1024 * 1024, None)?;
    let release: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let selected = if preview {
        let list = release
            .as_array()
            .ok_or("release listing is not an array")?;
        list.iter()
            .find(|r| r["prerelease"] == true && r["draft"] != true)
            .or_else(|| list.iter().find(|r| r["draft"] != true))
            .ok_or("no releases found")?
    } else {
        &release
    };
    update_release(binary, current, exe, check, force, selected, |url, cap| {
        fetch(url, cap, exe.parent())
    })
}

fn update_release(
    binary: Binary,
    current: &str,
    exe: &Path,
    check: bool,
    force: bool,
    release: &serde_json::Value,
    download: impl Fn(&str, u64) -> Result<Vec<u8>, String>,
) -> Result<Outcome, String> {
    let tag = release["tag_name"]
        .as_str()
        .ok_or("release has no tag_name")?;
    if !newer(tag, current)? {
        return Ok(Outcome {
            report: format!("up to date (v{current})"),
            installed: false,
        });
    }
    if check {
        return Ok(Outcome {
            report: format!("update available: {current} -> {tag}"),
            installed: false,
        });
    }
    if binary == Binary::Mux {
        guard_bundle(exe, force)?;
    }
    let version = tag.trim_start_matches('v');
    let name = asset_name(
        binary,
        version,
        std::env::consts::OS,
        std::env::consts::ARCH,
    )?;
    let sidecar = match binary {
        Binary::Streamer => format!("{name}.sha256"),
        Binary::Mux => format!("par-mux-v{version}-SHA256SUMS.txt"),
    };
    let assets = release["assets"]
        .as_array()
        .ok_or("release has no assets")?;
    let url = |wanted: &str| -> Result<&str, String> {
        assets
            .iter()
            .find(|a| a["name"].as_str() == Some(wanted))
            .and_then(|a| a["browser_download_url"].as_str())
            .ok_or_else(|| format!("release {tag} has no {wanted}; refusing unverified update"))
    };
    let expected = download(url(&sidecar)?, 64 * 1024)?;
    let expected = checksum(
        std::str::from_utf8(&expected).map_err(|e| e.to_string())?,
        &name,
    )?;
    let bytes = download(url(&name)?, 256 * 1024 * 1024)?;
    verify(&bytes, &expected)?;
    let payload = if binary == Binary::Mux {
        let dir = tempfile::tempdir_in(exe.parent().ok_or("executable has no parent")?)
            .map_err(|e| e.to_string())?;
        let archive = dir.path().join(&name);
        std::fs::write(&archive, bytes).map_err(|e| e.to_string())?;
        let top = name
            .strip_suffix(".tar.gz")
            .or_else(|| name.strip_suffix(".zip"))
            .ok_or("unknown archive format")?;
        let member = format!("{top}/par-mux{}", if cfg!(windows) { ".exe" } else { "" });
        // Extract only the known binary to stdout, never archive paths onto disk.
        let output = Command::new("tar")
            .arg("-xOf")
            .arg(&archive)
            .arg(&member)
            .output()
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!("could not extract {member}"));
        }
        output.stdout
    } else {
        bytes
    };
    if payload.is_empty() || payload.len() > 256 * 1024 * 1024 {
        return Err("invalid executable size".into());
    }
    let digest = sha2::Sha256::digest(&payload)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let backup = install_verified(exe, &payload, &digest)?;
    Ok(Outcome {
        report: format!("updated to {tag}; backup: {}", backup.display()),
        installed: true,
    })
}

/// Restart a mux daemon through its final-save/restore path on the same socket.
pub fn restart_mux(exe: &Path, socket: &Path, state_dir: Option<&Path>) -> Result<(), String> {
    mux_restart_command(exe, socket, state_dir)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn mux_restart_command(exe: &Path, socket: &Path, state_dir: Option<&Path>) -> Command {
    let mut command = Command::new(exe);
    command
        .arg("--socket")
        .arg(socket)
        .args(["--restart", "--no-attach"]);
    if let Some(dir) = state_dir {
        command.arg("--state-dir").arg(dir);
    }
    command
}

/// Re-execute the new binary without update one-shot arguments.
pub fn reexec(exe: &Path) -> Result<(), String> {
    let args: Vec<_> = std::env::args_os()
        .skip(1)
        .filter(|a| a != "--self-update" && a != "--check-update" && a != "--force-self-update")
        .collect();
    let mut command = Command::new(exe);
    command.args(args);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(command.exec().to_string())
    }
    #[cfg(not(unix))]
    {
        command.spawn().map_err(|e| e.to_string())?;
        std::process::exit(0);
    }
}

/// Restart the streamer through launchd when running as a launchd job, otherwise
/// replace the current process. The production label can be overridden.
pub fn restart_streamer(exe: &Path, label: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if std::env::var_os("XPC_SERVICE_NAME").is_some_and(|s| s == label)
        || std::env::var_os("LAUNCH_JOB_LABEL").is_some_and(|s| s == label)
    {
        // SAFETY: getuid has no preconditions and only reads the real user ID.
        let uid = unsafe { libc::getuid() };
        let status = Command::new("launchctl")
            .args(["kickstart", "-k", &format!("gui/{uid}/{label}")])
            .status()
            .map_err(|e| e.to_string())?;
        return if status.success() {
            Ok(())
        } else {
            Err(format!("launchctl failed: {status}"))
        };
    }
    let _ = label;
    reexec(exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mux_restart_preserves_socket_and_state_directory() {
        let command = mux_restart_command(
            Path::new("new-mux"),
            Path::new("named.sock"),
            Some(Path::new("state-dir")),
        );
        assert_eq!(command.get_program(), "new-mux");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                "--socket",
                "named.sock",
                "--restart",
                "--no-attach",
                "--state-dir",
                "state-dir"
            ]
        );
    }

    #[test]
    fn release_download_corruption_never_touches_installation() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("streamer");
        std::fs::write(&exe, b"original").unwrap();
        let name = asset_name(
            Binary::Streamer,
            "99.0.0",
            std::env::consts::OS,
            std::env::consts::ARCH,
        )
        .unwrap();
        let release = serde_json::json!({"tag_name": "v99.0.0", "assets": [
            {"name": name, "browser_download_url": "https://example.test/binary"},
            {"name": format!("{name}.sha256"), "browser_download_url": "https://example.test/checksum"}
        ]});
        let download = |url: &str, _cap| {
            Ok(if url.ends_with("checksum") {
                format!(
                    "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  {name}\n"
                )
                .into_bytes()
            } else {
                b"corrupted".to_vec()
            })
        };
        let err = update_release(
            Binary::Streamer,
            "1.0.0",
            &exe,
            false,
            false,
            &release,
            download,
        )
        .unwrap_err();
        assert!(err.contains("SHA-256 mismatch"));
        assert_eq!(std::fs::read(&exe).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        let good = |url: &str, cap| {
            if url.ends_with("binary") {
                Ok(b"abc".to_vec())
            } else {
                download(url, cap)
            }
        };
        assert!(
            update_release(
                Binary::Streamer,
                "1.0.0",
                &exe,
                false,
                false,
                &release,
                good
            )
            .unwrap()
            .installed
        );
        assert_eq!(std::fs::read(&exe).unwrap(), b"abc");
        assert_eq!(
            std::fs::read(dir.path().join("streamer.previous")).unwrap(),
            b"original"
        );
    }

    #[cfg(unix)]
    #[test]
    fn mux_archive_uses_packaged_top_directory() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("installed");
        std::fs::write(&exe, b"old").unwrap();
        let name = asset_name(
            Binary::Mux,
            "99.0.0",
            std::env::consts::OS,
            std::env::consts::ARCH,
        )
        .unwrap();
        let top = name.strip_suffix(".tar.gz").unwrap();
        let source = dir.path().join(top);
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("par-mux"), b"new mux binary").unwrap();
        let archive = dir.path().join(&name);
        assert!(Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(dir.path())
            .arg(top)
            .status()
            .unwrap()
            .success());
        let bytes = std::fs::read(&archive).unwrap();
        let digest = sha2::Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let release = serde_json::json!({"tag_name": "v99.0.0", "assets": [
            {"name": name, "browser_download_url": "https://example.test/archive"},
            {"name": "par-mux-v99.0.0-SHA256SUMS.txt", "browser_download_url": "https://example.test/sums"}
        ]});
        let result = update_release(
            Binary::Mux,
            "1.0.0",
            &exe,
            false,
            false,
            &release,
            |url, _| {
                Ok(if url.ends_with("sums") {
                    format!("{digest}  {name}\n").into_bytes()
                } else {
                    bytes.clone()
                })
            },
        )
        .unwrap();
        assert!(result.installed);
        assert_eq!(std::fs::read(&exe).unwrap(), b"new mux binary");
        assert_eq!(
            std::fs::read(dir.path().join("installed.previous")).unwrap(),
            b"old"
        );
    }

    #[test]
    fn check_only_never_downloads_or_writes() {
        let release = serde_json::json!({"tag_name": "v99.0.0"});
        let outcome = update_release(
            Binary::Mux,
            "1.0.0",
            Path::new("missing"),
            true,
            false,
            &release,
            |_, _| panic!("check must not download assets"),
        )
        .unwrap();
        assert!(!outcome.installed);
        assert!(outcome.report.contains("update available"));
    }

    #[test]
    fn semver_comparison() {
        assert!(newer("v1.10.0", "1.9.0").unwrap());
        assert!(!newer("v1.0.0-rc.1", "1.0.0").unwrap());
        assert!(!newer("v1.0.0+build", "1.0.0").unwrap());
        assert!(newer("garbage", "1.0.0").is_err());
    }

    #[test]
    fn all_release_platforms_match_workflow() {
        let workflow = include_str!("../../../.github/workflows/deployment.yml");
        for (os, arch, target) in [
            ("macos", "aarch64", "aarch64-apple-darwin"),
            ("macos", "x86_64", "x86_64-apple-darwin"),
            ("linux", "aarch64", "aarch64-unknown-linux-gnu"),
            ("linux", "x86_64", "x86_64-unknown-linux-gnu"),
            ("windows", "x86_64", "x86_64-pc-windows-msvc"),
        ] {
            let streamer = asset_name(Binary::Streamer, "1.2.3", os, arch).unwrap();
            assert!(workflow.contains(&format!("asset_name: {streamer}")));
            assert!(workflow.contains(&format!("target: {target}")));
            let ext = if os == "windows" { "zip" } else { "tar.gz" };
            assert_eq!(
                asset_name(Binary::Mux, "1.2.3", os, arch).unwrap(),
                format!("par-mux-v1.2.3-{target}.{ext}")
            );
        }
        assert!(asset_name(Binary::Mux, "1.0.0", "windows", "aarch64").is_err());
    }

    #[test]
    fn checksum_refusal_preserves_installed_binary_and_swap_keeps_backup() {
        let dir = tempfile::tempdir().unwrap();
        let installed = dir.path().join("program");
        std::fs::write(&installed, b"old").unwrap();
        let digest = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(install_verified(&installed, b"corrupted", digest).is_err());
        assert_eq!(std::fs::read(&installed).unwrap(), b"old");
        let backup = install_verified(&installed, b"abc", digest).unwrap();
        assert_eq!(std::fs::read(&installed).unwrap(), b"abc");
        assert_eq!(std::fs::read(backup).unwrap(), b"old");
    }

    #[test]
    fn bundle_guard_requires_explicit_force() {
        let exe = Path::new("/Applications/Par-mux.app/Contents/MacOS/par-mux");
        assert!(guard_bundle(exe, false).is_err());
        assert!(guard_bundle(exe, true).is_ok());
        assert!(guard_bundle(Path::new("/usr/local/bin/par-mux"), false).is_ok());
    }

    #[test]
    fn interval_defaults_daily_and_rejects_unknown() {
        assert_eq!(Interval::default().seconds(), 86400);
        for (text, seconds) in [("hourly", 3600), ("daily", 86400), ("weekly", 604800)] {
            let value: Interval = serde_json::from_str(&format!("\"{text}\"")).unwrap();
            assert_eq!(value.seconds(), seconds);
        }
        assert!(serde_json::from_str::<Interval>("\"sometimes\"").is_err());
    }

    #[test]
    fn checksum_selects_exact_filename() {
        let digest = "a".repeat(64);
        assert_eq!(
            checksum(
                &format!("{}  other\n{digest}  wanted\n", "b".repeat(64)),
                "wanted"
            )
            .unwrap(),
            digest
        );
        assert!(checksum(&format!("{digest}  other"), "wanted").is_err());
        assert!(checksum("bad  wanted", "wanted").is_err());
    }
}
