//! The self-update tier (`par-mux update`): resolve the latest release on
//! a channel, download the platform archive, verify it, replace this
//! binary, and restart the daemon so its sessions ride the stop-and-restore
//! handoff (D3.5: every pane respawns with its screen, scrollback, cwd,
//! and agent identity; D10 records why live handoff is not the contract).
//!
//! Channels: `stable` follows the latest non-prerelease GitHub release;
//! `preview` follows the newest prerelease. The network rides `curl`
//! exactly as the plugin installer rides `git` — par-mux ships no HTTP
//! client dependency, and curl is present everywhere the release archives
//! are consumable. Everything except the actual network calls and the
//! self-replace is pure and unit-tested.

use std::path::{Path, PathBuf};

/// The repository whose releases carry the par-mux archives.
pub const RELEASE_REPO: &str = "paulrobello/par-term-emu-core-rust";

/// This binary's target triple (the archive name embeds it) — the same six
/// targets the release workflow builds.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub const TARGET_TRIPLE: &str = "aarch64-apple-darwin";
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
pub const TARGET_TRIPLE: &str = "x86_64-apple-darwin";
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
pub const TARGET_TRIPLE: &str = "aarch64-unknown-linux-gnu";
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub const TARGET_TRIPLE: &str = "x86_64-unknown-linux-gnu";
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
pub const TARGET_TRIPLE: &str = "x86_64-pc-windows-msvc";
#[cfg(all(target_os = "windows", target_arch = "aarch64"))]
pub const TARGET_TRIPLE: &str = "aarch64-pc-windows-msvc";

/// One downloadable asset of a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    /// The archive's file name.
    pub name: String,
    /// Its browser download URL.
    pub url: String,
}

/// The facts `update --check` reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatePlan {
    /// The channel the plan resolved.
    pub channel: Channel,
    /// The running binary's version.
    pub current_version: String,
    /// The release the channel serves.
    pub latest_version: String,
    /// The asset for this platform, when the release carries one.
    pub asset: Option<Asset>,
}

impl UpdatePlan {
    /// Whether the plan installs anything.
    pub fn installs(&self) -> bool {
        version_gt(&self.latest_version, &self.current_version)
    }
}

/// The update channels: `stable` = latest non-prerelease; `preview` =
/// newest prerelease.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "lower")]
pub enum Channel {
    /// The latest non-prerelease release.
    Stable,
    /// The newest prerelease, falling back to stable when none exists.
    Preview,
}

impl Channel {
    /// The channel's lowercase name.
    pub fn as_str(&self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Preview => "preview",
        }
    }
}

/// Pick the release a channel serves from GitHub's list (newest first):
/// stable takes the first non-prerelease, preview the first prerelease
/// (falling back to stable when no prerelease exists).
pub fn release_for_channel(releases: &serde_json::Value, channel: Channel) -> Option<String> {
    let releases = releases.as_array()?;
    let pick = |prerelease: bool| {
        releases
            .iter()
            .find(|release| release.get("prerelease").and_then(|v| v.as_bool()) == Some(prerelease))
            .and_then(|release| {
                release
                    .get("tag_name")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
    };
    match channel {
        Channel::Stable => pick(false),
        Channel::Preview => pick(true).or_else(|| pick(false)),
    }
}

/// The platform asset inside a release: `par-mux-v<version>-<target>.tar.gz`
/// (Unix) or `.zip` (Windows).
pub fn asset_for_platform(release: &serde_json::Value, version: &str) -> Option<Asset> {
    let ext = if cfg!(windows) { "zip" } else { "tar.gz" };
    let wanted = format!("par-mux-v{version}-{TARGET_TRIPLE}.{ext}");
    release.get("assets")?.as_array()?.iter().find_map(|asset| {
        let name = asset.get("name")?.as_str()?;
        let url = asset.get("browser_download_url")?.as_str()?;
        (name == wanted).then(|| Asset {
            name: name.to_string(),
            url: url.to_string(),
        })
    })
}

/// Strip a leading `v` from a tag/version string.
pub fn strip_v(version: &str) -> &str {
    version.strip_prefix('v').unwrap_or(version)
}

/// Numeric dotted-version comparison: `version_gt("0.63.0", "0.62.9")`.
/// Non-numeric segments compare lexically (pre-release tags sort BELOW the
/// release they decorate: `1.0.0-rc1` < `1.0.0`).
pub fn version_gt(new: &str, current: &str) -> bool {
    fn segments(version: &str) -> Vec<(u64, String)> {
        strip_v(version)
            .split(['.', '-'])
            .map(|segment| (segment.parse::<u64>().unwrap_or(0), segment.to_string()))
            .collect()
    }
    let (a, b) = (segments(new), segments(current));
    // A bare numeric segment outranks a string one at the same position
    // (1.0.0 > 1.0.0-rc1): a missing segment outranks "-rc1"'s pair (0,"rc1").
    let numeric = |segments: &[(u64, String)], index: usize| -> (u64, String) {
        segments
            .get(index)
            .cloned()
            .unwrap_or((u64::MAX, String::new()))
    };
    let len = a.len().max(b.len());
    for index in 0..len {
        let (left, right) = (numeric(&a, index), numeric(&b, index));
        if left != right {
            return left > right;
        }
    }
    false
}

/// How the update performs network calls and process spawning — a seam so
/// tests drive the whole flow without a network or replacing themselves.
pub trait UpdateIo {
    /// `curl -fsSL <url>` to stdout (or a file path given).
    fn fetch(&self, url: &str, to: &Path) -> Result<(), String>;
    /// Run `<exe> --version` and return its first stdout line.
    fn probe_version(&self, exe: &Path) -> Result<String, String>;
    /// Replace `self_exe` with `new_binary` (rename dance per platform).
    fn replace_binary(&self, self_exe: &Path, new_binary: &Path) -> Result<(), String>;
    /// Detached `par-mux --restart` (the new binary, same socket
    /// resolution) so sessions ride the stop-and-restore handoff.
    fn restart_daemon(&self, new_binary: &Path, socket: &Path) -> Result<(), String>;
}

/// The real I/O: curl, spawn, rename.
pub struct RealIo;

impl UpdateIo for RealIo {
    fn fetch(&self, url: &str, to: &Path) -> Result<(), String> {
        let status = std::process::Command::new("curl")
            .arg("-fsSL")
            .arg(url)
            .arg("-o")
            .arg(to)
            .status()
            .map_err(|err| format!("curl {url}: {err} (is curl installed?)"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("curl {url} failed: {status}"))
        }
    }

    fn probe_version(&self, exe: &Path) -> Result<String, String> {
        let output = std::process::Command::new(exe)
            .arg("--version")
            .output()
            .map_err(|err| format!("{} --version: {err}", exe.display()))?;
        if !output.status.success() {
            return Err(format!(
                "{} --version exited {}",
                exe.display(),
                output.status
            ));
        }
        String::from_utf8(output.stdout)
            .map_err(|err| err.to_string())?
            .lines()
            .next()
            .map(str::to_string)
            .ok_or_else(|| "empty --version output".to_string())
    }

    fn replace_binary(&self, self_exe: &Path, new_binary: &Path) -> Result<(), String> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(new_binary)
                .map_err(|err| err.to_string())?
                .permissions()
                .mode();
            std::fs::set_permissions(new_binary, std::fs::Permissions::from_mode(mode | 0o755))
                .map_err(|err| err.to_string())?;
            // Rename over a running POSIX binary is fine; keep the old
            // image aside so a failed restart still has it.
            let old = self_exe.with_extension("previous");
            let _ = std::fs::remove_file(&old);
            std::fs::rename(self_exe, &old).map_err(|err| err.to_string())?;
            if let Err(err) = std::fs::rename(new_binary, self_exe) {
                let _ = std::fs::rename(&old, self_exe);
                return Err(err.to_string());
            }
            Ok(())
        }
        #[cfg(windows)]
        {
            // A running PE cannot be replaced but CAN be renamed; the
            // running daemon keeps the old image until it restarts.
            let old = self_exe.with_extension("previous.exe");
            let _ = std::fs::remove_file(&old);
            std::fs::rename(self_exe, &old).map_err(|err| err.to_string())?;
            if let Err(err) = std::fs::rename(new_binary, self_exe) {
                let _ = std::fs::rename(&old, self_exe);
                return Err(err.to_string());
            }
            Ok(())
        }
    }

    fn restart_daemon(&self, new_binary: &Path, socket: &Path) -> Result<(), String> {
        std::process::Command::new(new_binary)
            .arg("--socket")
            .arg(socket)
            .arg("--restart")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|err| format!("restart via {}: {err}", new_binary.display()))
    }
}

/// Build the plan for `channel` against `current_version`, from the
/// GitHub releases listing `releases`.
pub fn plan(
    releases: &serde_json::Value,
    channel: Channel,
    current_version: &str,
) -> Result<UpdatePlan, String> {
    let tag = release_for_channel(releases, channel)
        .ok_or_else(|| format!("no release serves the {} channel", channel.as_str()))?;
    let version = strip_v(&tag).to_string();
    let release = releases
        .as_array()
        .and_then(|list| {
            list.iter().find(|release| {
                release
                    .get("tag_name")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    == Some(tag.clone())
            })
        })
        .ok_or_else(|| format!("release {tag} carries no asset list"))?;
    Ok(UpdatePlan {
        channel,
        current_version: current_version.to_string(),
        asset: asset_for_platform(release, &version),
        latest_version: version,
    })
}

/// Execute `plan`: download the archive, extract the binary, verify it,
/// replace, restart. `check` stops after printing the plan.
pub fn execute(
    plan: &UpdatePlan,
    io: &dyn UpdateIo,
    self_exe: &Path,
    socket: &Path,
    check: bool,
) -> Result<String, String> {
    let mut report = format!(
        "par-mux {} channel: current v{}, latest v{}\n",
        plan.channel.as_str(),
        plan.current_version,
        plan.latest_version
    );
    if !plan.installs() {
        report.push_str("already up to date\n");
        return Ok(report);
    }
    if plan.asset.is_none() {
        return Err(format!(
            "release v{} carries no archive for {TARGET_TRIPLE}",
            plan.latest_version
        ));
    }
    if check {
        report.push_str("update available (re-run without --check to install)\n");
        return Ok(report);
    }
    let asset = plan.asset.clone().expect("checked above");
    let work = tempfile_dir()?;
    let archive = work.join(&asset.name);
    io.fetch(&asset.url, &archive)?;
    let binary = extract_binary(&archive, &work)?;
    let version_line = io.probe_version(&binary)?;
    if !version_line.contains(&plan.latest_version) {
        return Err(format!(
            "the downloaded binary reports {version_line:?}, expected v{}",
            plan.latest_version
        ));
    }
    io.replace_binary(self_exe, &binary)?;
    match io.restart_daemon(self_exe, socket) {
        Ok(()) => report.push_str(&format!(
            "updated to v{}; the daemon restarts and restores its sessions\n",
            plan.latest_version
        )),
        Err(err) => report.push_str(&format!(
            "updated to v{}, but the daemon restart failed: {err}\n(run par-mux --restart yourself)\n",
            plan.latest_version
        )),
    }
    Ok(report)
}

/// Extract the par-mux binary from the release archive into `dir`
/// (tar.gz on Unix through `tar`; zip on Windows through `tar` too —
/// bsdtar reads zip and ships with Windows).
fn extract_binary(archive: &Path, dir: &Path) -> Result<PathBuf, String> {
    let status = std::process::Command::new("tar")
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(dir)
        .status()
        .map_err(|err| format!("tar: {err}"))?;
    if !status.success() {
        return Err(format!("tar -xf {} failed: {status}", archive.display()));
    }
    let binary_name = if cfg!(windows) {
        "par-mux.exe"
    } else {
        "par-mux"
    };
    let binary = dir.join(binary_name);
    binary
        .is_file()
        .then_some(binary)
        .ok_or_else(|| format!("{} carries no {binary_name} at its root", archive.display()))
}

fn tempfile_dir() -> Result<PathBuf, String> {
    std::env::temp_dir()
        .join(format!(
            "par-mux-update-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        ))
        .pipe(|dir| {
            std::fs::create_dir_all(&dir).map_err(|err| format!("{}: {err}", dir.display()))?;
            Ok(dir)
        })
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}

#[cfg(test)]
mod tests {
    use super::*;

    fn releases_json() -> serde_json::Value {
        let ours = format!("par-mux-v0.63.0-{TARGET_TRIPLE}.tar.gz");
        let preview = "par-mux-v0.64.0-rc1-x86_64-unknown-linux-gnu.tar.gz";
        serde_json::json!([
            {
                "tag_name": "v0.64.0-rc1",
                "prerelease": true,
                "assets": [{"name": preview,
                            "browser_download_url": "https://x/1"}]
            },
            {
                "tag_name": "v0.63.0",
                "prerelease": false,
                "assets": [{"name": ours,
                            "browser_download_url": "https://x/2"},
                           {"name": "par-mux-v0.63.0-other-target.tar.gz",
                            "browser_download_url": "https://x/3"}]
            },
            {
                "tag_name": "v0.62.0",
                "prerelease": false,
                "assets": []
            }
        ])
    }

    #[test]
    fn channels_pick_their_release() {
        assert_eq!(
            release_for_channel(&releases_json(), Channel::Stable).as_deref(),
            Some("v0.63.0")
        );
        assert_eq!(
            release_for_channel(&releases_json(), Channel::Preview).as_deref(),
            Some("v0.64.0-rc1")
        );
        // A preview channel with no prerelease falls back to stable.
        let stable_only =
            serde_json::json!([{"tag_name": "v1.0.0", "prerelease": false, "assets": []}]);
        assert_eq!(
            release_for_channel(&stable_only, Channel::Preview).as_deref(),
            Some("v1.0.0")
        );
    }

    #[test]
    fn the_plan_selects_this_platforms_asset() {
        let plan = plan(&releases_json(), Channel::Stable, "0.62.0").unwrap();
        assert!(plan.installs());
        let asset = plan.asset.expect("the fixture carries the target");
        assert_eq!(asset.url, "https://x/2");
        assert_eq!(
            asset.name,
            format!("par-mux-v0.63.0-{TARGET_TRIPLE}.tar.gz")
        );
    }

    #[test]
    fn version_comparison_orders_prereleases_below_releases() {
        assert!(version_gt("0.63.0", "0.62.9"));
        assert!(version_gt("v0.63.0", "0.63.0-rc1"));
        assert!(!version_gt("0.63.0", "0.63.0"));
        assert!(!version_gt("0.62.0", "0.63.0"));
        assert!(version_gt("1.0.0", "1.0.0-rc2"));
    }

    #[test]
    fn an_up_to_date_plan_installs_nothing() {
        let plan = plan(&releases_json(), Channel::Stable, "0.63.0").unwrap();
        assert!(!plan.installs());
    }

    struct FakeIo {
        fetched: std::sync::Mutex<Vec<String>>,
    }

    impl UpdateIo for FakeIo {
        fn fetch(&self, url: &str, _to: &Path) -> Result<(), String> {
            self.fetched.lock().unwrap().push(format!("fetch:{url}"));
            Ok(())
        }
        fn probe_version(&self, _exe: &Path) -> Result<String, String> {
            Ok("par-mux 0.63.0+abc".into())
        }
        fn replace_binary(&self, _self_exe: &Path, _new: &Path) -> Result<(), String> {
            self.fetched.lock().unwrap().push("replace".into());
            Ok(())
        }
        fn restart_daemon(&self, _new: &Path, socket: &Path) -> Result<(), String> {
            self.fetched
                .lock()
                .unwrap()
                .push(format!("restart:{}", socket.display()));
            Ok(())
        }
    }

    #[test]
    fn check_reports_without_touching_the_machine() {
        let plan = plan(&releases_json(), Channel::Stable, "0.62.0").unwrap();
        let io = FakeIo {
            fetched: std::sync::Mutex::new(Vec::new()),
        };
        let report = execute(
            &plan,
            &io,
            Path::new("/bin/par-mux"),
            Path::new("/sock"),
            true,
        )
        .expect("check never installs");
        assert!(report.contains("update available"));
        assert!(io.fetched.lock().unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn execute_downloads_extracts_verifies_replaces_and_restarts() {
        // A real archive with a real (script) binary, fetched by a fake
        // that writes the archive where curl would.
        let work = tempfile::tempdir().unwrap();
        let payload = work.path().join("payload");
        std::fs::create_dir_all(&payload).unwrap();
        std::fs::write(
            payload.join("par-mux"),
            "#!/bin/sh\necho 'par-mux 0.63.0+test'\n",
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(
                payload.join("par-mux"),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
        let archive = work.path().join("fake.tar.gz");
        let tar_status = std::process::Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&payload)
            .arg("par-mux")
            .status()
            .unwrap();
        assert!(tar_status.success());

        struct FetchingIo {
            archive: PathBuf,
            calls: std::sync::Mutex<Vec<String>>,
        }
        impl UpdateIo for FetchingIo {
            fn fetch(&self, _url: &str, to: &Path) -> Result<(), String> {
                std::fs::copy(&self.archive, to).map_err(|err| err.to_string())?;
                Ok(())
            }
            fn probe_version(&self, exe: &Path) -> Result<String, String> {
                super::RealIo.probe_version(exe)
            }
            fn replace_binary(&self, _a: &Path, b: &Path) -> Result<(), String> {
                self.calls.lock().unwrap().push("replace".into());
                assert!(b.is_file());
                Ok(())
            }
            fn restart_daemon(&self, _new: &Path, socket: &Path) -> Result<(), String> {
                self.calls
                    .lock()
                    .unwrap()
                    .push(format!("restart:{}", socket.display()));
                Ok(())
            }
        }

        let io = FetchingIo {
            archive: archive.clone(),
            calls: std::sync::Mutex::new(Vec::new()),
        };
        let plan = plan(&releases_json(), Channel::Stable, "0.62.0").unwrap();
        let self_exe = work.path().join("self-par-mux");
        std::fs::write(&self_exe, "#!/bin/sh\n").unwrap();
        let report = execute(&plan, &io, &self_exe, Path::new("/sock/s1"), false).unwrap();
        assert!(report.contains("updated to v0.63.0"), "{report}");
        assert!(report.contains("restores its sessions"));
        let calls = io.calls.lock().unwrap();
        assert_eq!(
            *calls,
            vec!["replace".to_string(), "restart:/sock/s1".to_string()]
        );
    }
}
