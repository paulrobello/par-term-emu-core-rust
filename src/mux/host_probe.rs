//! Host-side telemetry probe: what the daemon can measure about a pane's
//! cwd that no hook can (card 01a0e3f1205371619309073eb5f803d6) —
//! disk-free percent and git state, the fields HerdDeck's hub probes
//! today (par-remote-herd `status_telemetry.py` `_probe_cwd`).
//!
//! Probed on a bounded cadence by a dedicated thread spawned with the
//! accept loop — never on the roster poll, the expensive mistake the hub
//! avoids by snapshot cadence: each `list-agents` costs zero syscalls
//! beyond reading pane metadata, and the probe work runs where no client
//! waits. Every probe is bounded (SEC-133): each pane's probe (`statvfs`
//! plus up to three git runs, each capped by [`GIT_TIMEOUT`] with stdout
//! drained as it arrives) runs on a detached worker the sweep stops
//! waiting for after [`PANE_PROBE_BUDGET`], so a wedged NFS/FUSE mount
//! costs a leaked thread, never a stalled sweep or shutdown.
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

use crate::mux::ids::PaneId;
use crate::mux::tree::MuxTree;
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// One host-probed value and the wall-clock time it was sampled; each
/// field ages out on its own at serve time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Stamped<T> {
    pub(crate) value: T,
    pub(crate) sampled_at_unix_ms: u64,
}

/// A pane's host-probed telemetry (ARC-113), stored typed on
/// [`crate::mux::pane::MuxPane::host_telemetry`] — sibling of the
/// hook-reported telemetry, cleared with the claim likewise. The roster
/// renders it through [`fresh_host_telemetry_b64`] with no JSON parse.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct HostTelemetry {
    pub(crate) disk_free_percent: Option<Stamped<u64>>,
    pub(crate) git_branch: Option<Stamped<String>>,
    pub(crate) git_dirty: Option<Stamped<bool>>,
}

impl HostTelemetry {
    /// Stamp every field a probe produced with the sweep's sample time.
    fn from_probe(probe: &HostProbe, sampled_at_unix_ms: u64) -> Self {
        fn stamp<T: Clone>(value: &Option<T>, at: u64) -> Option<Stamped<T>> {
            value.as_ref().map(|value| Stamped {
                value: value.clone(),
                sampled_at_unix_ms: at,
            })
        }
        Self {
            disk_free_percent: stamp(&probe.disk_free_percent, sampled_at_unix_ms),
            git_branch: stamp(&probe.git_branch, sampled_at_unix_ms),
            git_dirty: stamp(&probe.git_dirty, sampled_at_unix_ms),
        }
    }

    /// The roster's JSON object: the `version`/`source` envelope plus every
    /// field sampled within `max_age_ms` of `now_ms` (all fields when
    /// `max_age_ms` is `None`), each as `{"sampled_at_unix_ms", "value"}`.
    /// `serde_json::Map` sorts keys (no `preserve_order`), so the bytes
    /// match what the stored-JSON representation emitted before ARC-113.
    /// `None` when no field qualifies.
    fn to_json(&self, now_ms: u64, max_age_ms: Option<u64>) -> Option<String> {
        let fresh = |at: u64| max_age_ms.is_none_or(|max| now_ms.saturating_sub(at) <= max);
        let mut object = serde_json::Map::new();
        let mut field = |name: &str, value: serde_json::Value, at: u64| {
            if fresh(at) {
                let mut stamped = serde_json::Map::new();
                stamped.insert("value".to_string(), value);
                stamped.insert(
                    "sampled_at_unix_ms".to_string(),
                    serde_json::Value::from(at),
                );
                object.insert(name.to_string(), serde_json::Value::Object(stamped));
            }
        };
        if let Some(disk) = &self.disk_free_percent {
            field(
                "disk_free_percent",
                serde_json::Value::from(disk.value),
                disk.sampled_at_unix_ms,
            );
        }
        if let Some(branch) = &self.git_branch {
            field(
                "git_branch",
                serde_json::Value::String(branch.value.clone()),
                branch.sampled_at_unix_ms,
            );
        }
        if let Some(dirty) = &self.git_dirty {
            field(
                "git_dirty",
                serde_json::Value::Bool(dirty.value),
                dirty.sampled_at_unix_ms,
            );
        }
        if object.is_empty() {
            return None;
        }
        object.insert("version".to_string(), serde_json::Value::from(1));
        object.insert(
            "source".to_string(),
            serde_json::Value::String("host-probe".to_string()),
        );
        Some(serde_json::Value::Object(object).to_string())
    }
}

/// How often the sweep re-probes a pane, and how long the probe thread
/// sleeps between shutdown checks (it must never outlive `run` by more
/// than one interval).
pub(crate) const HOST_PROBE_INTERVAL: Duration = Duration::from_secs(30);

/// Deadline for one git invocation. A repo that cannot answer in time
/// serves absent fields rather than stalling the sweep.
const GIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Wall-clock bound on one whole sweep across every rostered pane. Each
/// pane's probe runs on a detached worker capped by [`PANE_PROBE_BUDGET`],
/// so no single pane can hold the sweep past this.
const SWEEP_DEADLINE: Duration = Duration::from_secs(10);

/// Per-pane wall-clock budget; the git runs inside it are each capped by
/// [`GIT_TIMEOUT`] and by what remains of this budget.
const PANE_PROBE_BUDGET: Duration = Duration::from_secs(6);

/// Probe threads still running (pane workers the sweep stopped waiting
/// for, and reapers of killed git children still in `wait`) before the
/// sweep stops starting new probes. Only a wedged filesystem keeps them
/// alive, so this caps what one leaks.
const MAX_ABANDONED_PROBES: usize = 4;

/// Margin between a pane probe's git deadline and the end of the sweep's
/// wait for it, so a slow but responsive repo returns (fields absent)
/// instead of reading as wedged.
const PROBE_SETTLE: Duration = Duration::from_millis(500);

/// How often a waiting sweep re-checks the shutdown flag.
const PROBE_POLL: Duration = Duration::from_millis(100);

/// How long a finished child's output may take to arrive from its reader
/// thread (a grandchild still holding the pipe open is not waited for).
const OUTPUT_GRACE: Duration = Duration::from_millis(250);

/// The branch string cap — the hub's `_bounded_string(branch, 128)`.
/// cap: Bytes of git branch name the host probe serves for one pane cwd.
const MAX_GIT_BRANCH_LEN: usize = 128;

/// One sweep's measurements for one pane. Every field is optional: a
/// probe that fails serves nothing for that field (absent beats stale).
struct HostProbe {
    disk_free_percent: Option<u64>,
    git_branch: Option<String>,
    git_dirty: Option<bool>,
}

/// What bounds one pane's probe (SEC-133): when its git runs must end, the
/// shutdown flag that ends them early, and the count of probe threads that
/// have not returned.
struct ProbeBounds {
    deadline: Instant,
    shutdown: Arc<AtomicBool>,
    outstanding: Arc<AtomicUsize>,
}

/// One unit of an outstanding-thread count, released on drop, so a thread
/// that panics still releases it.
struct Outstanding(Arc<AtomicUsize>);

impl Outstanding {
    fn enter(counter: &Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self(Arc::clone(counter))
    }
}

impl Drop for Outstanding {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Probe a pane's cwd: disk headroom plus git shape, the hub's pair. Every
/// git run ends by the bounds' deadline, and a raised shutdown flag skips
/// whatever has not started yet.
fn probe_cwd(cwd: &Path, bounds: &ProbeBounds) -> HostProbe {
    let mut probe = HostProbe {
        disk_free_percent: None,
        git_branch: None,
        git_dirty: None,
    };
    if bounds.shutdown.load(Ordering::Relaxed) {
        return probe;
    }
    probe.disk_free_percent = disk_free_percent(cwd);
    probe.git_branch = git_branch(cwd, bounds);
    probe.git_dirty = git_dirty(cwd, bounds);
    probe
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
/// (non-zero exit) serves no branch. stdout is capped just past the limit:
/// anything longer is rejected anyway.
fn git_branch(cwd: &Path, bounds: &ProbeBounds) -> Option<String> {
    let output = run_git(
        cwd,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        MAX_GIT_BRANCH_LEN + 2,
        bounds,
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
///
/// Only the emptiness of `ls-files` matters, so its stdout is capped at one
/// byte (SEC-133): the read end closes after it, and git dies on the next
/// write instead of blocking on a full pipe. One byte therefore reads dirty
/// whatever the exit status; empty output is trusted only from a clean
/// exit.
fn git_dirty(cwd: &Path, bounds: &ProbeBounds) -> Option<bool> {
    let tracked = run_git(
        cwd,
        &["diff-index", "--cached", "--quiet", "HEAD", "--"],
        0,
        bounds,
    )?;
    match tracked.status.code() {
        Some(1) => return Some(true),
        Some(0) => {}
        _ => return None,
    }
    let untracked = run_git(
        cwd,
        &["ls-files", "--others", "--exclude-standard", "--directory"],
        1,
        bounds,
    )?;
    if !untracked.stdout.is_empty() {
        return Some(true);
    }
    untracked.status.success().then_some(false)
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

/// Run one git invocation, ending by the bounds' deadline and within
/// [`GIT_TIMEOUT`], with at most `stdout_cap` bytes of stdout kept. Returns
/// `None` when git could not run or did not finish in time (the caller
/// serves absent) — a probe failure is data absence, not an error path.
/// Every call goes through [`git_command`] — a bare `git -C` here is the
/// SEC-115 bug shape.
fn run_git(cwd: &Path, args: &[&str], stdout_cap: usize, bounds: &ProbeBounds) -> Option<Output> {
    let mut command = git_command(cwd);
    command.args(args);
    let deadline = bounds.deadline.min(Instant::now() + GIT_TIMEOUT);
    run_bounded(command, stdout_cap, deadline, bounds)
}

/// Run `command` bounded three ways (SEC-133). stdout is read on its own
/// thread as it arrives, up to `stdout_cap` bytes, and the read end then
/// closes, so a chatty child fails its next write instead of blocking on a
/// full pipe. The child is killed at `deadline` or once the shutdown flag
/// is raised. A killed child is reaped on a detached thread, because `wait`
/// on a child stuck in uninterruptible I/O never returns. stdin is null and
/// stderr is dropped: the probe asks, it never feeds.
fn run_bounded(
    mut command: Command,
    stdout_cap: usize,
    deadline: Instant,
    bounds: &ProbeBounds,
) -> Option<Output> {
    use std::io::Read as _;

    if bounds.shutdown.load(Ordering::Relaxed) || Instant::now() >= deadline {
        return None;
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let Some(pipe) = child.stdout.take() else {
        kill_detached(child, &bounds.outstanding);
        return None;
    };
    let (tx, rx) = channel();
    let reader = std::thread::Builder::new()
        .name("par-mux-probe-stdout".to_string())
        .spawn(move || {
            let mut out = Vec::new();
            // `take` owns the pipe, so the read end closes at the end of
            // this statement, however much was read.
            let _ = pipe.take(stdout_cap as u64).read_to_end(&mut out);
            let _ = tx.send(out);
        });
    if reader.is_err() {
        kill_detached(child, &bounds.outstanding);
        return None;
    }
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline && !bounds.shutdown.load(Ordering::Relaxed) => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) | Err(_) => {
                kill_detached(child, &bounds.outstanding);
                return None;
            }
        }
    };
    let stdout = rx.recv_timeout(OUTPUT_GRACE).ok()?;
    Some(Output {
        status,
        stdout,
        stderr: Vec::new(),
    })
}

/// Kill `child` and reap it on a detached thread, never on the caller's.
/// The reaper counts as outstanding until its `wait` returns, which a
/// child stuck on a wedged filesystem delays indefinitely.
fn kill_detached(mut child: Child, outstanding: &Arc<AtomicUsize>) {
    let _ = child.kill();
    let guard = Outstanding::enter(outstanding);
    let _ = std::thread::Builder::new()
        .name("par-mux-probe-reaper".to_string())
        .spawn(move || {
            let _guard = guard;
            let _ = child.wait();
        });
}

/// Cross-sweep probe state (SEC-133), owned by the probe worker.
#[derive(Default)]
pub(crate) struct ProbeState {
    /// Panes whose last probe outran its budget, keyed to the cwd that
    /// wedged; skipped until the pane's cwd changes.
    timed_out: HashMap<PaneId, PathBuf>,
    /// Probe threads that have not returned (see [`MAX_ABANDONED_PROBES`]).
    outstanding: Arc<AtomicUsize>,
}

/// Why a bounded pane probe produced no result.
#[derive(Debug, PartialEq, Eq)]
enum ProbeMiss {
    /// The budget ran out; the worker is left running, detached.
    TimedOut,
    /// Shutdown was raised while waiting.
    Shutdown,
    /// The worker could not start, or ended without a result.
    Failed,
}

/// Run `probe` on a detached worker and wait for it until `wait_until` or
/// shutdown. The worker counts as outstanding from before it starts until
/// it returns (not merely until the wait gives up), so the count can never
/// underflow and a wedged worker keeps counting.
fn probe_pane_bounded<T, F>(
    probe: F,
    wait_until: Instant,
    shutdown: &AtomicBool,
    outstanding: &Arc<AtomicUsize>,
) -> Result<T, ProbeMiss>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = channel();
    let guard = Outstanding::enter(outstanding);
    std::thread::Builder::new()
        .name("par-mux-pane-probe".to_string())
        .spawn(move || {
            let _guard = guard;
            let _ = tx.send(probe());
        })
        .map_err(|_| ProbeMiss::Failed)?;
    loop {
        if shutdown.load(Ordering::Relaxed) {
            return Err(ProbeMiss::Shutdown);
        }
        let now = Instant::now();
        if now >= wait_until {
            return Err(ProbeMiss::TimedOut);
        }
        match rx.recv_timeout(PROBE_POLL.min(wait_until - now)) {
            Ok(result) => return Ok(result),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Err(ProbeMiss::Failed),
        }
    }
}

/// One probe sweep: collect the rostered panes and their cwd inputs under
/// the tree lock (the ARC-032 shape — the cwd syscalls run after it
/// drops), probe each cwd off the lock, then write each result back under
/// a fresh lock. A pane that died mid-sweep simply fails the write-back
/// lookup; a probe failure writes nothing (the previous sample ages out
/// on its own at serve time). No broadcast: the roster poll reads what is
/// there, and `%agent-telemetry-changed` stays the hook's signal.
///
/// Bounded (SEC-133): each pane's probe runs through
/// [`probe_pane_bounded`], so a wedged filesystem costs that pane's
/// budget once. The pane is then skipped until its cwd changes, and
/// shutdown is checked before every pane.
pub(crate) fn host_probe_sweep(
    tree: &Arc<Mutex<MuxTree>>,
    shutdown: &Arc<AtomicBool>,
    state: &mut ProbeState,
) {
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

    // A pane that left the roster (or died) forgets its wedged cwd.
    let live: HashSet<PaneId> = targets.iter().map(|(pane_id, _)| *pane_id).collect();
    state.timed_out.retain(|pane_id, _| live.contains(pane_id));

    let now_ms = unix_now_ms();
    let sweep_deadline = Instant::now() + SWEEP_DEADLINE;
    let mut results = Vec::with_capacity(targets.len());
    for (pane_id, parts) in &targets {
        // The sweep is bounded as a whole: panes past the deadline keep
        // their previous sample (and re-probe next sweep) rather than
        // holding a shutdown join or starving later panes behind one
        // slow repo. Inside the last PROBE_SETTLE a probe would get no
        // git time at all and overwrite a good sample with disk only.
        if shutdown.load(Ordering::Relaxed) || Instant::now() + PROBE_SETTLE >= sweep_deadline {
            break;
        }
        if state.outstanding.load(Ordering::SeqCst) >= MAX_ABANDONED_PROBES {
            crate::debug_error!(
                "MUX",
                "host probe: {MAX_ABANDONED_PROBES} probe threads still running; sweep stopped"
            );
            break;
        }
        let Some(cwd) = parts.probe_cwd() else {
            continue;
        };
        if state.timed_out.get(pane_id) == Some(&cwd) {
            continue;
        }
        let wait_until = sweep_deadline.min(Instant::now() + PANE_PROBE_BUDGET);
        let bounds = ProbeBounds {
            deadline: wait_until - PROBE_SETTLE,
            shutdown: Arc::clone(shutdown),
            outstanding: Arc::clone(&state.outstanding),
        };
        let probe_dir = cwd.clone();
        match probe_pane_bounded(
            move || probe_cwd(&probe_dir, &bounds),
            wait_until,
            shutdown,
            &state.outstanding,
        ) {
            Ok(probe) => {
                state.timed_out.remove(pane_id);
                results.push((*pane_id, HostTelemetry::from_probe(&probe, now_ms)));
            }
            Err(ProbeMiss::TimedOut) => {
                crate::debug_error!(
                    "MUX",
                    "host probe of pane {pane_id} timed out; skipped until its cwd changes"
                );
                state.timed_out.insert(*pane_id, cwd);
            }
            Err(ProbeMiss::Shutdown) => break,
            Err(ProbeMiss::Failed) => {}
        }
    }

    let mut guard = tree.lock();
    for (pane_id, telemetry) in results {
        if let Some(pane) = guard.pane_mut(pane_id) {
            pane.host_telemetry = Some(telemetry);
        }
    }
}

/// The pane's host-probed telemetry for the roster row: the stored sample
/// with every field older than the freshness window dropped (per-field —
/// the sweep rewrites all of them together, but a stopped sweep must not
/// pin aged data), rendered as the canonical object and base64-encoded as
/// one whitespace-free token. `None` when nothing fresh remains — the row
/// then carries no host token at all, exactly like a pane that was never
/// probed.
pub(crate) fn fresh_host_telemetry_b64(telemetry: Option<&HostTelemetry>) -> Option<String> {
    use base64::Engine as _;

    let json = telemetry?.to_json(
        unix_now_ms(),
        Some(crate::mux::hooks::TELEMETRY_FRESHNESS_MS),
    )?;
    Some(base64::engine::general_purpose::STANDARD.encode(json))
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(u64::MAX)
}

/// The cadence thread (see module docs): one sweep per
/// [`HOST_PROBE_INTERVAL`], the first after one full interval. Once the
/// shutdown flag is raised it exits within one 500 ms idle poll, or, mid
/// sweep, within one [`PROBE_POLL`] wait: in-flight git children are
/// killed and pane workers are left detached, never awaited.
pub(crate) fn spawn_host_probe_worker(
    tree: Arc<Mutex<MuxTree>>,
    shutdown: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut state = ProbeState::default();
        let mut last_probe = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(500));
            if shutdown.load(Ordering::Relaxed) {
                break;
            }
            if last_probe.elapsed() >= HOST_PROBE_INTERVAL {
                host_probe_sweep(&tree, &shutdown, &mut state);
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

    /// Bounds for a test probe: a generous deadline, no shutdown.
    fn test_bounds() -> ProbeBounds {
        ProbeBounds {
            deadline: Instant::now() + PANE_PROBE_BUDGET,
            shutdown: Arc::new(AtomicBool::new(false)),
            outstanding: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn probe_dir(cwd: &Path) -> HostProbe {
        probe_cwd(cwd, &test_bounds())
    }

    /// Unix-only, like the PTY-bearing sweep tests that call it.
    #[cfg(unix)]
    fn sweep(tree: &Arc<Mutex<MuxTree>>) {
        host_probe_sweep(
            tree,
            &Arc::new(AtomicBool::new(false)),
            &mut ProbeState::default(),
        );
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
        let probe = probe_dir(repo.path());
        assert_eq!(probe.git_branch.as_deref(), Some("main"));
        assert_eq!(probe.git_dirty, Some(false), "clean fixture");

        // An untracked file flips dirty — the hub's ls-files half.
        std::fs::write(repo.path().join("untracked.txt"), "x\n").unwrap();
        let probe = probe_dir(repo.path());
        assert_eq!(probe.git_dirty, Some(true));

        // A fresh repo pins the --cached narrowing (SEC-115): an
        // unstaged-only edit to a tracked file reads clean (detecting it
        // would read worktree content and run the repo's clean filter),
        // the same edit staged reads dirty.
        let repo = git_repo();
        std::fs::write(repo.path().join("tracked.txt"), "two\n").unwrap();
        let probe = probe_dir(repo.path());
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
        let probe = probe_dir(repo.path());
        assert_eq!(probe.git_dirty, Some(true), "staged edit is dirty");

        // Not a repo at all: every git field absent, disk still measured.
        let plain = tempfile::tempdir().unwrap();
        let probe = probe_dir(plain.path());
        assert_eq!(probe.git_branch, None);
        assert_eq!(probe.git_dirty, None);
        assert!(probe.disk_free_percent.is_some() || cfg!(windows));
    }

    /// SEC-133: `ls-files` output larger than the pipe buffer must not
    /// deadlock git until the timeout kill. The files sit at the repo root:
    /// `--directory` collapses an untracked subdirectory into one line.
    #[test]
    fn git_dirty_survives_output_larger_than_the_pipe_buffer() {
        let repo = git_repo();
        for i in 0..3_000 {
            let name = format!("untracked-{i:05}-{}", "x".repeat(24));
            std::fs::write(repo.path().join(name), "x").unwrap();
        }
        let started = Instant::now();
        let dirty = git_dirty(repo.path(), &test_bounds());
        let elapsed = started.elapsed();
        assert_eq!(dirty, Some(true), "untracked files read dirty");
        assert!(
            elapsed < Duration::from_secs(2),
            "the check finished without the timeout kill: {elapsed:?}"
        );
    }

    /// SEC-133: a raised shutdown flag kills an in-flight child promptly
    /// rather than waiting out its deadline.
    #[cfg(unix)]
    #[test]
    fn run_bounded_returns_promptly_on_shutdown() {
        let bounds = test_bounds();
        let flag = Arc::clone(&bounds.shutdown);
        let raiser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            flag.store(true, Ordering::Relaxed);
        });
        let mut command = Command::new("sleep");
        command.arg("30");
        let started = Instant::now();
        let output = run_bounded(command, 16, Instant::now() + GIT_TIMEOUT, &bounds);
        let elapsed = started.elapsed();
        raiser.join().unwrap();
        assert!(output.is_none(), "a shutdown-killed child serves nothing");
        assert!(
            elapsed < Duration::from_secs(3),
            "shutdown ended the run promptly: {elapsed:?}"
        );
    }

    /// SEC-133: the deadline kill never blocks on the child's `wait`.
    #[cfg(unix)]
    #[test]
    fn run_bounded_times_out_without_blocking_on_wait() {
        let bounds = test_bounds();
        let mut command = Command::new("sleep");
        command.arg("30");
        let started = Instant::now();
        let output = run_bounded(
            command,
            16,
            Instant::now() + Duration::from_millis(200),
            &bounds,
        );
        let elapsed = started.elapsed();
        assert!(output.is_none(), "a timed-out child serves nothing");
        assert!(
            elapsed < Duration::from_secs(3),
            "the deadline ended the run promptly: {elapsed:?}"
        );
    }

    /// SEC-133: stdout past the cap is not read, and the capped child
    /// is not waited out.
    #[cfg(unix)]
    #[test]
    fn run_bounded_caps_stdout_without_deadlocking() {
        let bounds = test_bounds();
        let mut command = Command::new("sh");
        command.args(["-c", "yes par-mux | head -c 1000000"]);
        let started = Instant::now();
        let output = run_bounded(command, 8, Instant::now() + GIT_TIMEOUT, &bounds)
            .expect("the capped child exits on its own");
        assert_eq!(output.stdout, b"par-mux\n");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the capped child did not wait for the deadline"
        );
    }

    /// SEC-133: a probe that outruns its budget is abandoned promptly, the
    /// outstanding count holds it until it really returns, and a sweep
    /// skips the wedged (pane, cwd) pair until the cwd changes.
    #[test]
    fn a_timed_out_pane_is_skipped_until_its_cwd_changes() {
        let shutdown = AtomicBool::new(false);
        let outstanding = Arc::new(AtomicUsize::new(0));
        let started = Instant::now();
        let result = probe_pane_bounded(
            || std::thread::sleep(Duration::from_secs(2)),
            Instant::now() + Duration::from_millis(100),
            &shutdown,
            &outstanding,
        );
        assert_eq!(result, Err(ProbeMiss::TimedOut));
        assert!(
            started.elapsed() < Duration::from_millis(1500),
            "the wait gave up at its budget: {:?}",
            started.elapsed()
        );
        assert_eq!(
            outstanding.load(Ordering::SeqCst),
            1,
            "the abandoned worker still counts"
        );

        let quick = probe_pane_bounded(
            || 7,
            Instant::now() + Duration::from_secs(1),
            &shutdown,
            &outstanding,
        );
        assert_eq!(quick, Ok(7));
        let deadline = Instant::now() + Duration::from_secs(5);
        while outstanding.load(Ordering::SeqCst) != 0 {
            assert!(Instant::now() < deadline, "workers release their count");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// SEC-133: the sweep's skip rule. A (pane, cwd) pair recorded as
    /// timed out is skipped; the same pane under a new cwd is probed, which
    /// clears the record.
    #[cfg(unix)]
    #[test]
    fn sweep_skips_a_wedged_cwd_and_resumes_on_a_new_one() {
        let (tree, pane_id) = tree_with_pane();
        let cwd = {
            let mut guard = tree.lock();
            let pane = guard.pane_mut(pane_id).unwrap();
            pane.set_metadata("agent", "kimi");
            pane.set_metadata("agent_state", "working");
            pane.snapshot_capture_parts()
                .probe_cwd()
                .expect("the pane has a live child")
        };
        let shutdown = Arc::new(AtomicBool::new(false));

        let mut state = ProbeState::default();
        state.timed_out.insert(pane_id, cwd.clone());
        host_probe_sweep(&tree, &shutdown, &mut state);
        assert!(
            tree.lock().pane(pane_id).unwrap().host_telemetry.is_none(),
            "the wedged (pane, cwd) pair is not probed"
        );
        assert_eq!(state.timed_out.get(&pane_id), Some(&cwd));

        state
            .timed_out
            .insert(pane_id, PathBuf::from("/a-cwd-the-pane-left"));
        host_probe_sweep(&tree, &shutdown, &mut state);
        assert!(
            tree.lock().pane(pane_id).unwrap().host_telemetry.is_some(),
            "a changed cwd is probed again"
        );
        assert!(state.timed_out.is_empty(), "success clears the record");

        state.timed_out.insert(crate::mux::ids::PaneId(9_999), cwd);
        host_probe_sweep(&tree, &shutdown, &mut state);
        assert!(
            state.timed_out.is_empty(),
            "a pane gone from the roster is forgotten"
        );
    }

    /// SEC-133: a sweep started under shutdown probes nothing.
    #[cfg(unix)]
    #[test]
    fn sweep_does_nothing_once_shutdown_is_raised() {
        let (tree, pane_id) = tree_with_pane();
        {
            let mut guard = tree.lock();
            let pane = guard.pane_mut(pane_id).unwrap();
            pane.set_metadata("agent", "kimi");
            pane.set_metadata("agent_state", "working");
        }
        host_probe_sweep(
            &tree,
            &Arc::new(AtomicBool::new(true)),
            &mut ProbeState::default(),
        );
        assert!(tree.lock().pane(pane_id).unwrap().host_telemetry.is_none());
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

        let probe = probe_dir(repo.path());
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
        sweep(&tree);
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
        let telemetry = pane.host_telemetry.as_ref().expect("rostered pane probed");
        assert_eq!(
            telemetry.git_branch.as_ref().map(|b| b.value.as_str()),
            Some("main"),
            "the probe ran in the child's cwd, not the OSC 7 path"
        );
        assert_eq!(telemetry.git_dirty.as_ref().map(|d| d.value), Some(false));
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
        let parsed: serde_json::Value = serde_json::from_str(
            &HostTelemetry::from_probe(&probe, 1_000)
                .to_json(1_000, None)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(parsed["version"], 1);
        assert_eq!(parsed["source"], "host-probe");
        assert_eq!(parsed["disk_free_percent"]["value"], 37);
        assert_eq!(parsed["disk_free_percent"]["sampled_at_unix_ms"], 1_000);
        assert_eq!(parsed["git_branch"]["value"], "main");
        assert_eq!(parsed["git_dirty"]["value"], true);
    }

    /// ARC-113: the typed sample renders the exact bytes the stored-JSON
    /// representation emitted (captured from the pre-change
    /// `host_telemetry_json`), so the roster's `host_telemetry=` token is
    /// unchanged on the wire.
    #[test]
    fn host_telemetry_json_is_byte_identical_to_the_stored_form() {
        let full = HostProbe {
            disk_free_percent: Some(37),
            git_branch: Some("main".to_string()),
            git_dirty: Some(true),
        };
        assert_eq!(
            HostTelemetry::from_probe(&full, 1_000)
                .to_json(1_000, None)
                .unwrap(),
            r#"{"disk_free_percent":{"sampled_at_unix_ms":1000,"value":37},"git_branch":{"sampled_at_unix_ms":1000,"value":"main"},"git_dirty":{"sampled_at_unix_ms":1000,"value":true},"source":"host-probe","version":1}"#
        );
        let partial = HostProbe {
            disk_free_percent: None,
            git_branch: Some("feat x".to_string()),
            git_dirty: None,
        };
        assert_eq!(
            HostTelemetry::from_probe(&partial, 1_000)
                .to_json(1_000, None)
                .unwrap(),
            r#"{"git_branch":{"sampled_at_unix_ms":1000,"value":"feat x"},"source":"host-probe","version":1}"#
        );
    }

    #[test]
    fn serving_drops_aged_fields_and_keeps_fresh_ones() {
        let now = unix_now_ms();
        let telemetry = HostTelemetry {
            disk_free_percent: Some(Stamped {
                value: 37,
                sampled_at_unix_ms: now - 1_000,
            }),
            git_branch: Some(Stamped {
                value: "main".to_string(),
                sampled_at_unix_ms: now - 2 * 3_600_000,
            }),
            git_dirty: None,
        };
        use base64::Engine as _;
        let encoded = fresh_host_telemetry_b64(Some(&telemetry)).expect("fresh fields remain");
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
        let telemetry = HostTelemetry {
            git_dirty: Some(Stamped {
                value: true,
                sampled_at_unix_ms: unix_now_ms() - 2 * 3_600_000,
            }),
            ..HostTelemetry::default()
        };
        assert_eq!(fresh_host_telemetry_b64(Some(&telemetry)), None);
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
        sweep(&tree);
        {
            let mut guard = tree.lock();
            let pane = guard.pane_mut(pane_id).unwrap();
            assert!(
                pane.host_telemetry.is_some(),
                "the rostered pane carries host telemetry"
            );
            pane.clear_metadata(&["agent", "agent_state"]);
            pane.host_telemetry = None;
        }
        sweep(&tree);
        let guard = tree.lock();
        let pane = guard.pane(pane_id).unwrap();
        assert!(
            pane.host_telemetry.is_none(),
            "the unrostered pane is never probed"
        );
    }
}
