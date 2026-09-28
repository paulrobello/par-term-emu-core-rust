//! Host-side telemetry probe: what the daemon can measure about a pane's
//! cwd that no hook can (card 01a0e3f1205371619309073eb5f803d6) —
//! disk-free percent and git state, the fields HerdDeck's hub probes
//! today (par-remote-herd `status_telemetry.py` `_probe_cwd`).
//!
//! Probed on a bounded cadence by a dedicated thread spawned with the
//! accept loop — never on the roster poll, the expensive mistake the hub
//! avoids by snapshot cadence: each `list-agents` costs zero syscalls
//! beyond reading pane metadata, and the sweep's git invocations (up to
//! three per pane, each under [`GIT_TIMEOUT`]) run where no client waits.
//! Results land in pane metadata as one canonical object with per-field
//! `sampled_at_unix_ms` and `source: "host-probe"`, so clients can tell
//! daemon-probed from hook-reported data (the same manners rule the
//! telemetry endpoint applies). Like hook telemetry it is display-only:
//! the save format's named-key capture never copies it.
//!
//! Trust rule (SEC-115): the probe runs only in a pane child's
//! kernel-reported cwd — never a directory program output picked through
//! OSC 7 — and every git invocation goes through [`git_command`]'s
//! hardened config, so no repo-configured hook, filter or transport
//! executes. A pane whose child cwd cannot be read is simply not probed.

use crate::mux::tree::MuxTree;
use parking_lot::Mutex;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The metadata key holding the pane's host-probed telemetry — sibling of
/// the hook-reported `agent_telemetry`, cleared with the claim likewise.
pub(crate) const HOST_TELEMETRY_KEY: &str = "agent_host_telemetry";

/// How often the sweep re-probes a pane, and how long the probe thread
/// sleeps between shutdown checks (it must never outlive `run` by more
/// than one interval).
pub(crate) const HOST_PROBE_INTERVAL: Duration = Duration::from_secs(30);

/// Deadline for one git invocation. A repo that cannot answer in time
/// serves absent fields rather than stalling the sweep.
const GIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Deadline for one whole sweep across every rostered pane — the bound a
/// shutdown join waits out (one in-flight git plus this check).
const SWEEP_DEADLINE: Duration = Duration::from_secs(10);

/// The branch string cap — the hub's `_bounded_string(branch, 128)`.
const MAX_GIT_BRANCH_LEN: usize = 128;

/// One sweep's measurements for one pane. Every field is optional: a
/// probe that fails serves nothing for that field (absent beats stale).
struct HostProbe {
    disk_free_percent: Option<u64>,
    git_branch: Option<String>,
    git_dirty: Option<bool>,
}

/// Probe a pane's cwd: disk headroom plus git shape, the hub's pair.
fn probe_cwd(cwd: &Path) -> HostProbe {
    HostProbe {
        disk_free_percent: disk_free_percent(cwd),
        git_branch: git_branch(cwd),
        git_dirty: git_dirty(cwd),
    }
}

/// Free-space headroom of the filesystem holding `cwd`, as a percent
/// rounded and clamped to 0-100 — `shutil.disk_usage` semantics (space
/// available to unprivileged callers). Unix reads `statvfs`; Windows
/// serves `None` (graceful — `GetDiskFreeSpaceExW` is future work, and
/// an unavailable field must not fail the sweep).
#[cfg(unix)]
fn disk_free_percent(cwd: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt as _;
    let mut stats = unsafe { std::mem::zeroed::<libc::statvfs>() };
    let path = std::ffi::CString::new(cwd.as_os_str().as_bytes()).ok()?;
    // SAFETY: `stats` is a valid out-pointer for the lifetime of the call
    // and `path` outlives it; the call touches no Rust memory.
    if unsafe { libc::statvfs(path.as_ptr(), &mut stats) } != 0 {
        return None;
    }
    let total = stats.f_blocks;
    if total == 0 {
        return None;
    }
    let percent = (stats.f_bavail as f64) * 100.0 / (total as f64);
    Some(percent.round().clamp(0.0, 100.0) as u64)
}

#[cfg(windows)]
fn disk_free_percent(_cwd: &Path) -> Option<u64> {
    None
}

/// The checked-out branch: `git symbolic-ref --quiet --short HEAD`, the
/// hub's invocation, bounded to [`MAX_GIT_BRANCH_LEN`]. A detached HEAD
/// (non-zero exit) serves no branch.
fn git_branch(cwd: &Path) -> Option<String> {
    let output = run_git(
        cwd,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        GIT_TIMEOUT,
    )?;
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if branch.is_empty()
        || branch.len() > MAX_GIT_BRANCH_LEN
        || branch.chars().any(char::is_control)
    {
        return None;
    }
    Some(branch)
}

/// Whether the worktree has changes: `diff-index --cached --quiet HEAD
/// --` (staged changes, exit 1) OR `ls-files --others
/// --exclude-standard --directory` (untracked, any output). The verdict
/// map is three-way (0 clean, 1 dirty, anything else — 128 above all,
/// not-a-repo or a git error — no verdict): a failing repo must serve
/// absent, never read as "dirty".
///
/// `--cached` is a deliberate narrowing (SEC-115): the worktree-walking
/// forms (`diff-index` without it, `status`) read file content and so
/// execute the repo's clean filters and fsmonitor hook; `--cached`
/// compares HEAD against the index alone and runs nothing the repo
/// configured. The price: an unstaged-only edit to a tracked file reads
/// clean.
fn git_dirty(cwd: &Path) -> Option<bool> {
    let tracked = run_git(
        cwd,
        &["diff-index", "--cached", "--quiet", "HEAD", "--"],
        GIT_TIMEOUT,
    )?;
    match tracked.status.code() {
        Some(1) => return Some(true),
        Some(0) => {}
        _ => return None,
    }
    let untracked = run_git(
        cwd,
        &["ls-files", "--others", "--exclude-standard", "--directory"],
        GIT_TIMEOUT,
    )?;
    Some(!untracked.stdout.is_empty())
}

/// Every probe git invocation, hardened (SEC-115): the probe runs in a
/// directory pane state chose, so nothing the repo's config names may
/// execute. `core.fsmonitor=false` and `core.hooksPath=/dev/null` close
/// the two config-specified command vectors; `safe.bareRepository=explicit`
/// keeps a stray bare repo from being entered; `protocol.ext.allow=never`
/// blocks the ext remote transport; `--no-optional-locks` (plus
/// `GIT_OPTIONAL_LOCKS=0`) stops the probe writing the repo's index;
/// `GIT_TERMINAL_PROMPT=0` stops it waiting on a prompt; and clearing
/// `GIT_DIR`/`GIT_WORK_TREE` stops an inherited environment from
/// redirecting the repo root away from `-C`.
fn git_command(cwd: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "safe.bareRepository=explicit",
            "-c",
            "protocol.ext.allow=never",
            "--no-optional-locks",
            "-C",
        ])
        .arg(cwd)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE");
    command
}

/// Run one git invocation under a deadline. Returns `None` when git
/// could not run or did not finish in time (the caller serves absent) —
/// a probe failure is data absence, not an error path. stdin is null
/// and stderr is dropped: the probe asks, it never feeds. Every call
/// goes through [`git_command`] — a bare `git -C` here is the SEC-115
/// bug shape.
fn run_git(cwd: &Path, args: &[&str], timeout: Duration) -> Option<std::process::Output> {
    let Ok(mut child) = git_command(cwd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return None;
    };
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = child.stdout.take().map_or_else(Vec::new, |mut pipe| {
                    use std::io::Read as _;
                    let mut out = Vec::new();
                    let _ = pipe.read_to_end(&mut out);
                    out
                });
                return Some(std::process::Output {
                    status,
                    stdout,
                    stderr: Vec::new(),
                });
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return None,
        }
    }
}

/// The canonical host-probe object: per-field `{value, sampled_at_unix_ms}`
/// pairs (fields age independently — git may change while disk holds),
/// with `version` and `source: "host-probe"` so a client can attribute
/// every field. Fields the probe could not measure are omitted outright.
fn host_telemetry_json(probe: &HostProbe, sampled_at_unix_ms: u64) -> String {
    let mut object = serde_json::Map::new();
    object.insert("version".to_string(), serde_json::Value::from(1));
    object.insert(
        "source".to_string(),
        serde_json::Value::String("host-probe".to_string()),
    );
    let mut field = |name: &str, value: serde_json::Value| {
        let mut stamped = serde_json::Map::new();
        stamped.insert("value".to_string(), value);
        stamped.insert(
            "sampled_at_unix_ms".to_string(),
            serde_json::Value::from(sampled_at_unix_ms),
        );
        object.insert(name.to_string(), serde_json::Value::Object(stamped));
    };
    if let Some(disk) = probe.disk_free_percent {
        field("disk_free_percent", serde_json::Value::from(disk));
    }
    if let Some(branch) = &probe.git_branch {
        field("git_branch", serde_json::Value::String(branch.clone()));
    }
    if let Some(dirty) = probe.git_dirty {
        field("git_dirty", serde_json::Value::Bool(dirty));
    }
    serde_json::Value::Object(object).to_string()
}

/// One probe sweep: collect the rostered panes and their cwd inputs under
/// the tree lock (the ARC-032 shape — the cwd syscalls run after it
/// drops), probe each cwd off the lock, then write each result back under
/// a fresh lock. A pane that died mid-sweep simply fails the write-back
/// lookup; a probe failure writes nothing (the previous sample ages out
/// on its own at serve time). No broadcast: the roster poll reads what is
/// there, and `%agent-telemetry-changed` stays the hook's signal.
pub(crate) fn host_probe_sweep(tree: &Arc<Mutex<MuxTree>>) {
    use crate::mux::ids::PaneId;

    let targets: Vec<(PaneId, crate::mux::pane::PaneSnapshotParts)> = {
        let guard = tree.lock();
        guard
            .sessions()
            .iter()
            .filter_map(|s| guard.session(*s))
            .flat_map(|s| s.windows.clone())
            .filter_map(|w| guard.window(w))
            .flat_map(|w| w.panes())
            .filter_map(|p| {
                let pane = guard.pane(p)?;
                // Rostered panes only: the probe exists to enrich agent
                // rows, and every extra pane costs three git runs.
                pane.metadata().get("agent_state")?;
                Some((p, pane.snapshot_capture_parts()))
            })
            .collect()
    };

    let now_ms = unix_now_ms();
    let sweep_deadline = Instant::now() + SWEEP_DEADLINE;
    let mut results = Vec::with_capacity(targets.len());
    for (pane_id, parts) in &targets {
        // The sweep is bounded as a whole: panes past the deadline keep
        // their previous sample (and re-probe next sweep) rather than
        // holding a shutdown join or starving later panes behind one
        // slow repo.
        if Instant::now() >= sweep_deadline {
            break;
        }
        if let Some(cwd) = parts.probe_cwd() {
            results.push((*pane_id, host_telemetry_json(&probe_cwd(&cwd), now_ms)));
        }
    }

    let mut guard = tree.lock();
    for (pane_id, telemetry) in results {
        if let Some(pane) = guard.pane_mut(pane_id) {
            pane.set_metadata(HOST_TELEMETRY_KEY, &telemetry);
        }
    }
}

/// The pane's host-probed telemetry for the roster row: the stored object
/// with every field older than the freshness window dropped (per-field —
/// the sweep rewrites all of them together, but a stopped sweep must not
/// pin aged data), base64-encoded as one whitespace-free token. `None`
/// when nothing fresh remains — the row then carries no host token at
/// all, exactly like a pane that was never probed.
pub(crate) fn fresh_host_telemetry_b64(
    metadata: &std::collections::HashMap<String, String>,
) -> Option<String> {
    use base64::Engine as _;

    let raw = metadata.get(HOST_TELEMETRY_KEY)?;
    let mut object = serde_json::from_str::<serde_json::Value>(raw).ok()?;
    let fields = object.as_object_mut()?;
    let now_ms = unix_now_ms();
    let window = crate::mux::hooks::TELEMETRY_FRESHNESS_MS;
    // The envelope (version, source) rides unconditionally; every stamped
    // field ages on its own clock.
    fields.retain(|name, stamped| {
        if name == "version" || name == "source" {
            return true;
        }
        stamped
            .get("sampled_at_unix_ms")
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|sampled| now_ms.saturating_sub(sampled) <= window)
    });
    if fields.len() <= 2 {
        return None;
    }
    Some(base64::engine::general_purpose::STANDARD.encode(object.to_string()))
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(u64::MAX)
}

/// The cadence thread (see module docs): one sweep per
/// [`HOST_PROBE_INTERVAL`], the first after one full interval, exiting
/// within half a second of the shutdown flag so a slow in-flight git
/// never blocks daemon exit — the sweep is skipped, not awaited, past
/// the deadline.
pub(crate) fn spawn_host_probe_worker(
    tree: Arc<Mutex<MuxTree>>,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut last_probe = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(500));
            if shutdown.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            if last_probe.elapsed() >= HOST_PROBE_INTERVAL {
                host_probe_sweep(&tree);
                last_probe = Instant::now();
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::mux::pane::ShellPaneFactory;

    /// Unix-only: the sweep test that uses this needs a PTY-bearing pane,
    /// and a cfg-gated helper keeps the Windows build warning-free.
    #[cfg(unix)]
    fn tree_with_pane() -> (Arc<Mutex<MuxTree>>, crate::mux::ids::PaneId) {
        let mut tree = MuxTree::new(Box::new(ShellPaneFactory::default()));
        let session = tree
            .new_session("probe", 80, 24)
            .expect("test session spawns");
        let pane_id = tree
            .session(session)
            .expect("session exists")
            .windows
            .iter()
            .filter_map(|window| tree.window(*window))
            .flat_map(|window| window.panes())
            .next()
            .expect("a new session has a pane");
        (Arc::new(Mutex::new(tree)), pane_id)
    }

    /// A git repo fixture answering the probe's three questions.
    fn git_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            let status = Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .expect("git runs");
            assert!(status.success(), "git {args:?} in fixture");
        };
        run(&["init", "--quiet", "--initial-branch=main"]);
        std::fs::write(dir.path().join("tracked.txt"), "one\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "--quiet", "--message", "fixture"]);
        dir
    }

    #[cfg(unix)]
    #[test]
    fn disk_probe_measures_a_real_filesystem() {
        // Positive control: a real tmpdir reports a plausible percent.
        let dir = tempfile::tempdir().unwrap();
        let percent = disk_free_percent(dir.path()).expect("tmpdir probes");
        assert!(percent <= 100, "clamped: {percent}");
        // A missing path serves nothing.
        assert_eq!(disk_free_percent(Path::new("/nonexistent-xyz")), None);
    }

    #[test]
    fn git_probe_reads_branch_and_dirty() {
        let repo = git_repo();
        let probe = probe_cwd(repo.path());
        assert_eq!(probe.git_branch.as_deref(), Some("main"));
        assert_eq!(probe.git_dirty, Some(false), "clean fixture");

        // An untracked file flips dirty — the hub's ls-files half.
        std::fs::write(repo.path().join("untracked.txt"), "x\n").unwrap();
        let probe = probe_cwd(repo.path());
        assert_eq!(probe.git_dirty, Some(true));

        // A fresh repo pins the --cached narrowing (SEC-115): an
        // unstaged-only edit to a tracked file reads clean (detecting it
        // would read worktree content and run the repo's clean filter),
        // the same edit staged reads dirty.
        let repo = git_repo();
        std::fs::write(repo.path().join("tracked.txt"), "two\n").unwrap();
        let probe = probe_cwd(repo.path());
        assert_eq!(probe.git_dirty, Some(false), "unstaged-only edit is clean");
        let staged = Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["add", "--all"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        assert!(staged.expect("git runs").success(), "git add in fixture");
        let probe = probe_cwd(repo.path());
        assert_eq!(probe.git_dirty, Some(true), "staged edit is dirty");

        // Not a repo at all: every git field absent, disk still measured.
        let plain = tempfile::tempdir().unwrap();
        let probe = probe_cwd(plain.path());
        assert_eq!(probe.git_branch, None);
        assert_eq!(probe.git_dirty, None);
        assert!(probe.disk_free_percent.is_some() || cfg!(windows));
    }

    /// SEC-115: a repo whose config names an fsmonitor hook must not run
    /// it — the probe's git calls run with every config-specified
    /// execution vector disabled.
    #[cfg(unix)]
    #[test]
    fn fsmonitor_hook_does_not_run() {
        if !Command::new("git")
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return; // no git on PATH — the probe serves absent anyway
        }
        let repo = git_repo();
        let marker = tempfile::tempdir().unwrap();
        let marker_path = marker.path().join("PWNED");
        let mut config = std::fs::read_to_string(repo.path().join(".git/config")).unwrap();
        config.push_str(&format!(
            "\n[core]\n\tfsmonitor = \"sh -c 'touch {}'\"\n",
            marker_path.display()
        ));
        std::fs::write(repo.path().join(".git/config"), config).unwrap();

        let probe = probe_cwd(repo.path());
        assert_eq!(probe.git_branch.as_deref(), Some("main"));
        let _ = probe.git_dirty; // exercised for the hook, verdict irrelevant
        assert!(
            !marker_path.exists(),
            "the repo-configured fsmonitor hook must never run"
        );
    }

    /// SEC-115: the sweep's git target is the child's kernel-reported cwd,
    /// never the OSC 7 value pane output controls. The pane's shell lives
    /// in a git repo whose config names an fsmonitor hook while its
    /// terminal reports a hostile OSC 7 path — telemetry must carry the
    /// repo's state (the probe ran where the kernel says the child is)
    /// and the hook must never run.
    #[cfg(unix)]
    #[test]
    fn sweep_probes_child_cwd_not_osc7() {
        let repo = git_repo();
        let marker = tempfile::tempdir().unwrap();
        let marker_path = marker.path().join("PWNED");
        let mut config = std::fs::read_to_string(repo.path().join(".git/config")).unwrap();
        config.push_str(&format!(
            "\n[core]\n\tfsmonitor = \"sh -c 'touch {}'\"\n",
            marker_path.display()
        ));
        std::fs::write(repo.path().join(".git/config"), config).unwrap();
        let mut tree = MuxTree::new(Box::new(ShellPaneFactory {
            cwd: Some(repo.path().to_path_buf()),
            ..ShellPaneFactory::default()
        }));
        let session = tree
            .new_session("probe", 80, 24)
            .expect("test session spawns");
        let pane_id = tree
            .session(session)
            .expect("session exists")
            .windows
            .iter()
            .filter_map(|window| tree.window(*window))
            .flat_map(|window| window.panes())
            .next()
            .expect("a new session has a pane");
        let tree = Arc::new(Mutex::new(tree));
        {
            let mut guard = tree.lock();
            let pane = guard.pane_mut(pane_id).unwrap();
            pane.set_metadata("agent", "kimi");
            pane.set_metadata("agent_state", "working");
            // The attacker's write: pane output picks the probe directory.
            pane.terminal()
                .write()
                .process(b"\x1b]7;file:///nonexistent-osc7\x07");
        }
        host_probe_sweep(&tree);
        let guard = tree.lock();
        let pane = guard.pane(pane_id).unwrap();
        // The seam: persistence semantics keep the OSC 7 value, the probe
        // target is the kernel-reported child cwd.
        let parts = pane.snapshot_capture_parts();
        assert_eq!(
            parts.cwd(),
            Some(std::path::PathBuf::from("/nonexistent-osc7")),
            "persistence cwd still prefers OSC 7"
        );
        let probed = parts.probe_cwd().expect("the pane has a live child");
        assert_ne!(probed, std::path::PathBuf::from("/nonexistent-osc7"));
        assert_eq!(
            std::fs::canonicalize(&probed).ok(),
            std::fs::canonicalize(repo.path()).ok(),
            "probe_cwd is the child's actual cwd"
        );
        let raw = pane
            .metadata()
            .get(HOST_TELEMETRY_KEY)
            .expect("rostered pane probed");
        let parsed: serde_json::Value = serde_json::from_str(raw).unwrap();
        assert_eq!(
            parsed["git_branch"]["value"], "main",
            "the probe ran in the child's cwd, not the OSC 7 path"
        );
        assert_eq!(parsed["git_dirty"]["value"], false);
        assert!(
            !marker_path.exists(),
            "the sweep never executed the repo-configured fsmonitor hook"
        );
    }

    #[test]
    fn canonical_object_is_stamped_and_attributed() {
        let probe = HostProbe {
            disk_free_percent: Some(37),
            git_branch: Some("main".to_string()),
            git_dirty: Some(true),
        };
        let parsed: serde_json::Value =
            serde_json::from_str(&host_telemetry_json(&probe, 1_000)).unwrap();
        assert_eq!(parsed["version"], 1);
        assert_eq!(parsed["source"], "host-probe");
        assert_eq!(parsed["disk_free_percent"]["value"], 37);
        assert_eq!(parsed["disk_free_percent"]["sampled_at_unix_ms"], 1_000);
        assert_eq!(parsed["git_branch"]["value"], "main");
        assert_eq!(parsed["git_dirty"]["value"], true);
    }

    #[test]
    fn serving_drops_aged_fields_and_keeps_fresh_ones() {
        let now = unix_now_ms();
        let mut metadata = std::collections::HashMap::new();
        metadata.insert(
            HOST_TELEMETRY_KEY.to_string(),
            format!(
                r#"{{"version":1,"source":"host-probe","disk_free_percent":{{"value":37,"sampled_at_unix_ms":{}}},"git_branch":{{"value":"main","sampled_at_unix_ms":{}}}}}"#,
                now - 1_000,
                now - 2 * 3_600_000
            ),
        );
        use base64::Engine as _;
        let encoded = fresh_host_telemetry_b64(&metadata).expect("fresh fields remain");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&decoded).unwrap();
        assert!(
            parsed.get("disk_free_percent").is_some(),
            "the fresh field survives: {parsed}"
        );
        assert!(
            parsed.get("git_branch").is_none(),
            "the aged field is dropped per-field: {parsed}"
        );
    }

    #[test]
    fn serving_serves_nothing_when_all_fields_aged() {
        let mut metadata = std::collections::HashMap::new();
        metadata.insert(
            HOST_TELEMETRY_KEY.to_string(),
            format!(
                r#"{{"version":1,"source":"host-probe","git_dirty":{{"value":true,"sampled_at_unix_ms":{}}}}}"#,
                unix_now_ms() - 2 * 3_600_000
            ),
        );
        assert_eq!(fresh_host_telemetry_b64(&metadata), None);
    }

    /// The sweep writes host telemetry for a rostered pane and skips an
    /// unrostered one — the bound on probe work. Unix-only: the fixture
    /// needs a git repo and a probe-able cwd.
    #[cfg(unix)]
    #[test]
    fn sweep_writes_rostered_panes_only() {
        let (tree, pane_id) = tree_with_pane();
        {
            let mut guard = tree.lock();
            let pane = guard.pane_mut(pane_id).unwrap();
            pane.set_metadata("agent", "kimi");
            pane.set_metadata("agent_state", "working");
        }
        host_probe_sweep(&tree);
        {
            let mut guard = tree.lock();
            let pane = guard.pane_mut(pane_id).unwrap();
            assert!(
                pane.metadata().contains_key(HOST_TELEMETRY_KEY),
                "the rostered pane carries host telemetry"
            );
            pane.clear_metadata(&["agent", "agent_state", HOST_TELEMETRY_KEY]);
        }
        host_probe_sweep(&tree);
        let guard = tree.lock();
        let pane = guard.pane(pane_id).unwrap();
        assert!(
            !pane.metadata().contains_key(HOST_TELEMETRY_KEY),
            "the unrostered pane is never probed"
        );
    }
}
