//! The sidebar git badge's daemon-side resolver: one `git status -sb`
//! read per displayed chain, refreshed by a daemon-wide background
//! thread on the configured cadence and served into the
//! `client-snapshot` reply as `git` rows from the cache alone — a
//! reply path never spawns git.
//!
//! Resolution (the card's contract): the displayed session's workspace →
//! its active window → its active pane → the pane's live cwd (OSC 7
//! first, then the child's kernel cwd); when the cwd is unknown or not a
//! repo, the workspace's launch folder (its [`WorktreeLink`] checkout)
//! stands in; when neither is a repo, no row — the sidebar hides the
//! badge. ONE badge for what's on screen.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// The badge's parsed `## ` header: the branch and its ahead/behind
/// counts against the upstream (`0`/`0` with no upstream).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitBadge {
    /// The checked-out branch (`main`). A detached HEAD has no branch to
    /// badge — `parse_branch_header` answers `None` for it.
    pub branch: String,
    /// Commits ahead of the upstream.
    pub ahead: u32,
    /// Commits behind the upstream.
    pub behind: u32,
}

/// The bound for one `git status` child (the card's 2 s): a wedged
/// filesystem must not stretch the snapshot reply.
const GIT_TIMEOUT: Duration = Duration::from_secs(2);

/// The cache entry: when the value was computed and what it said.
struct CacheEntry {
    at: Instant,
    badge: Option<GitBadge>,
}

/// The daemon-wide cache, keyed by the probed directory.
static CACHE: OnceLock<parking_lot::Mutex<HashMap<PathBuf, CacheEntry>>> = OnceLock::new();

fn cache() -> &'static parking_lot::Mutex<HashMap<PathBuf, CacheEntry>> {
    CACHE.get_or_init(|| parking_lot::Mutex::new(HashMap::new()))
}

/// The refresher's wake cadence: each wake gathers the live badge dirs
/// under a brief tree lock, then runs its bounded git reads outside any
/// lock. A badge's worst-case age is `ttl` plus one wake.
const REFRESH_TICK: Duration = Duration::from_secs(2);

/// How long past its `ttl` a cache entry survives before the prune
/// drops it (a workspace whose pane moved on stops costing map rows).
const PRUNE_TTLS: u32 = 10;

/// The badge for `dir` (the chain's chosen directory), served from the
/// cache ALONE — stale entries included. The refresher thread
/// ([`refresh_known_badges`]) keeps entries fresh on the configured
/// cadence, so a snapshot reply never runs git (the freeze contract:
/// the bounded reads a reply path used to pay serially now run where
/// nothing waits on them). `None` when the directory is `None` or has
/// never been refreshed — the sidebar shows nothing for it until the
/// refresher's next tick.
pub(crate) fn cached(dir: Option<&Path>) -> Option<GitBadge> {
    cache().lock().get(dir?).and_then(|e| e.badge.clone())
}

/// Reset the cache — the tests' isolation seam.
#[cfg(test)]
pub(crate) fn reset_cache() {
    cache().lock().clear();
}

/// The refresher tick's work: gather the live badge dirs from `tree`
/// (a brief lock) and run the bounded git reads for every entry older
/// than `ttl` — the configured poll minus one second, the freshness
/// the inline path used to give — then prune entries the workspaces
/// have left behind. Runs on the refresher thread and in the tests;
/// never on a reply path.
pub(crate) fn refresh_known_badges(tree: &crate::mux::tree::MuxTree, ttl: Duration) {
    let dirs: Vec<PathBuf> = badge_sources(tree)
        .into_iter()
        .filter_map(|(_, cwd, launch)| {
            badge_dir(cwd.as_deref(), launch.as_deref()).map(|dir| dir.to_path_buf())
        })
        .collect();
    for dir in &dirs {
        let fresh = cache()
            .lock()
            .get(dir)
            .is_some_and(|e| e.at.elapsed() < ttl);
        if fresh {
            continue;
        }
        let badge = resolve(dir);
        cache().lock().insert(
            dir.clone(),
            CacheEntry {
                at: Instant::now(),
                badge,
            },
        );
    }
    let max_age = ttl.saturating_mul(PRUNE_TTLS);
    prune(max_age);
}

/// Drop cache entries older than `max_age` — a workspace whose pane
/// moved on stops costing a map row. Its own fn so the unit tests can
/// exercise it without a tree.
fn prune(max_age: Duration) {
    cache().lock().retain(|_, e| e.at.elapsed() <= max_age);
}

/// The daemon-wide badge refresher: the one thread that runs the
/// bounded `git status` reads behind the sidebar badges, so no reply
/// path ever spawns git. Follows the host-probe worker's lifecycle —
/// 2 s wakes, exits within one wake of shutdown. The daemon-side
/// `[client] git-status` gate and `git-poll-secs` clock are read fresh
/// each wake (live on reload).
pub(crate) fn spawn_badge_refresher(
    tree: std::sync::Arc<parking_lot::Mutex<crate::mux::tree::MuxTree>>,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
    config: Option<std::sync::Arc<parking_lot::Mutex<crate::mux::config::EffectiveConfig>>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || loop {
        std::thread::sleep(REFRESH_TICK);
        if shutdown.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        let Some((enabled, ttl)) = config.as_ref().map(|config| {
            let cfg = config.lock();
            (cfg.git_status, badge_ttl(cfg.git_poll_secs))
        }) else {
            continue;
        };
        if !enabled {
            continue;
        }
        refresh_known_badges(&tree.lock(), ttl);
    })
}

/// The badge TTL from the configured poll (`git-poll-secs`): one second
/// under the poll, floored at one second — the freshness the inline
/// path's cache gave.
fn badge_ttl(git_poll_secs: u64) -> Duration {
    Duration::from_secs(git_poll_secs.saturating_sub(1).max(1))
}

/// Per-workspace badge source directories in workspace id order: the
/// workspace's active session's active window's active pane's live cwd,
/// else the workspace's launch folder (its worktree checkout). The
/// snapshot's `git` rows and the refresher tick enumerate the same
/// chain.
pub(crate) fn badge_sources(
    tree: &crate::mux::tree::MuxTree,
) -> Vec<(
    crate::mux::ids::WorkspaceId,
    Option<PathBuf>,
    Option<PathBuf>,
)> {
    let mut workspaces = tree.workspaces();
    workspaces.sort();
    workspaces
        .iter()
        .filter_map(|id| {
            let ws = tree.workspace(*id)?;
            let cwd = ws
                .sessions
                .get(ws.active)
                .or_else(|| ws.sessions.first())
                .and_then(|sid| tree.session(*sid))
                .and_then(|s| s.windows.get(s.active).copied())
                .and_then(|w| tree.window(w))
                .map(|w| w.active)
                .and_then(|p| tree.pane(p))
                .and_then(|pane| pane.persistence_cwd());
            let launch = ws.worktree.as_ref().map(|link| link.checkout.clone());
            Some((*id, cwd, launch))
        })
        .collect()
}

/// The chain's tail: the pane's live cwd first, the workspace's launch
/// folder when the pane never reported one (no OSC 7, no readable child
/// cwd, or a dead pane).
pub(crate) fn badge_dir<'a>(
    pane_cwd: Option<&'a Path>,
    launch_folder: Option<&'a Path>,
) -> Option<&'a Path> {
    pane_cwd.or(launch_folder)
}

/// One bounded `git status` read in `dir`: the card's exact invocation
/// (no other flags) with the repo's env hygiene. `None` when git cannot
/// run, the directory is not a repo, or the read outruns
/// [`GIT_TIMEOUT`] — a probe failure is data absence, not an error path.
///
/// fsmonitor: bypassed. The command is the contract's
/// `git -c core.fsmonitor=false --no-optional-locks -C <dir> status -sb
/// --porcelain=v1 --branch --no-renames`: a filesystem monitor answers
/// "nothing changed" for an event it has not received yet, which would
/// pin a stale badge for up to the monitor's own latency; and
/// `--no-optional-locks` because a status run may refresh the index,
/// which must not write a user's checkout from a snapshot request.
fn resolve(dir: &Path) -> Option<GitBadge> {
    let mut command = Command::new("git");
    command
        .args(["-c", "core.fsmonitor=false", "--no-optional-locks"])
        .arg("-C")
        .arg(dir)
        .args([
            "status",
            "-sb",
            "--porcelain=v1",
            "--branch",
            "--no-renames",
        ])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().ok()?;
    let Some(mut pipe) = child.stdout.take() else {
        reap_detached(child);
        return None;
    };
    let deadline = Instant::now() + GIT_TIMEOUT;
    let mut timed_out = false;
    let output = std::thread::scope(|scope| {
        let reader = scope.spawn(move || {
            use std::io::Read as _;
            let mut out = String::new();
            let _ = pipe.read_to_string(&mut out);
            out
        });
        let mut status = None;
        while status.is_none() {
            match child.try_wait() {
                Ok(Some(exit)) => status = Some(exit),
                Ok(None) => {
                    if Instant::now() >= deadline {
                        timed_out = true;
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => {
                    timed_out = true;
                    break;
                }
            }
        }
        // The scope joins the reader at its end, so a still-running child
        // (the deadline hit, or a wait error) must die FIRST — its held
        // stdout pipe would block `read_to_string` forever.
        if timed_out {
            let _ = child.kill();
        }
        reader.join().ok().map(|stdout| (status, stdout))
    })?;
    if timed_out {
        // The kill already closed the pipe (that is what unblocked the
        // reader); the reap goes to a detached thread — `wait` on a
        // child stuck in uninterruptible I/O never returns, and the
        // caller must not block on it.
        reap_detached(child);
        return None;
    }
    let (status, stdout) = output;
    if !status.is_some_and(|exit| exit.success()) {
        return None;
    }
    let header = stdout.lines().find(|line| line.starts_with("## "))?;
    parse_branch_header(header)
}

/// Reap a child whose stdout pipe could not be taken on a detached
/// thread — `wait` on a child stuck in uninterruptible I/O never
/// returns, and the caller must not block on it.
fn reap_detached(mut child: std::process::Child) {
    let _ = std::thread::Builder::new()
        .name("par-mux-gitbadge-reaper".to_string())
        .spawn(move || {
            let _ = child.wait();
        });
}

/// Parse the `## ` branch header of `git status -sb --porcelain=v1
/// --branch`:
///
/// - `## main` — local branch, no upstream → ahead/behind 0/0
/// - `## main...origin/main` — upstream, in sync
/// - `## main...origin/main [ahead 2]` / `[behind 1]` / `[ahead 2, behind 1]`
/// - `## No commits yet on main` — an unborn branch (a fresh `git init`)
/// - `## HEAD (no branch)` — detached HEAD → `None` (no branch to badge)
pub(crate) fn parse_branch_header(line: &str) -> Option<GitBadge> {
    let rest = line.strip_prefix("## ")?;
    let rest = rest.trim_end();
    if let Some(branch) = rest.strip_prefix("No commits yet on ") {
        return Some(GitBadge {
            branch: branch.to_string(),
            ahead: 0,
            behind: 0,
        });
    }
    if rest.contains("(no branch)") {
        return None;
    }
    let mut parts = rest.splitn(2, " [");
    let refs = parts.next().unwrap_or_default();
    let counts = parts.next();
    let (branch, _upstream) = match refs.split_once("...") {
        Some((branch, upstream)) => (branch, Some(upstream)),
        None => (refs, None),
    };
    let counts = counts.unwrap_or_default();
    let counts = counts.strip_prefix('[').unwrap_or(counts);
    let counts = counts.strip_suffix(']').unwrap_or(counts);
    let mut ahead = 0;
    let mut behind = 0;
    for part in counts.split(',') {
        let part = part.trim();
        if let Some(n) = part.strip_prefix("ahead ") {
            ahead = n.parse().ok()?;
        } else if let Some(n) = part.strip_prefix("behind ") {
            behind = n.parse().ok()?;
        }
    }
    if branch.is_empty() {
        return None;
    }
    Some(GitBadge {
        branch: branch.to_string(),
        ahead,
        behind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tests' stand-in for the dispatch-passed poll-minus-one TTL.
    const TEST_TTL: Duration = Duration::from_secs(3);

    /// The header shapes the badge parses, including the unborn-branch
    /// and detached forms `git status -sb` really emits.
    #[test]
    fn branch_header_parse_cases() {
        let badge = |line: &str| parse_branch_header(line).map(|b| (b.branch, b.ahead, b.behind));
        assert_eq!(badge("## main"), Some(("main".into(), 0, 0)));
        assert_eq!(badge("## main...origin/main"), Some(("main".into(), 0, 0)));
        assert_eq!(
            badge("## main...origin/main [ahead 2]"),
            Some(("main".into(), 2, 0))
        );
        assert_eq!(
            badge("## main...origin/main [behind 3]"),
            Some(("main".into(), 0, 3))
        );
        assert_eq!(
            badge("## main...origin/main [ahead 2, behind 1]"),
            Some(("main".into(), 2, 1))
        );
        assert_eq!(
            badge("## No commits yet on main"),
            Some(("main".into(), 0, 0)),
            "an unborn branch still badges by name"
        );
        assert_eq!(
            badge("## HEAD (no branch)"),
            None,
            "a detached HEAD has no branch to badge"
        );
        assert_eq!(badge("## "), None, "an empty header is no badge");
        assert_eq!(parse_branch_header("main"), None, "no `## ` prefix");
    }

    /// The chain's tail: the pane's cwd wins; the workspace's launch
    /// folder stands in; neither, no badge.
    #[test]
    fn badge_dir_prefers_the_pane_cwd_over_the_launch_folder() {
        let cwd = Path::new("/repo/sub");
        let launch = Path::new("/repo");
        assert_eq!(badge_dir(Some(cwd), Some(launch)), Some(cwd));
        assert_eq!(badge_dir(None, Some(launch)), Some(launch));
        assert_eq!(badge_dir(Some(cwd), None), Some(cwd));
        assert_eq!(badge_dir(None, None), None);
    }

    /// A real checkout: the badge resolves the branch; a plain directory
    /// is no repo and no badge. (Skipped where git is unavailable — the
    /// git-dependent integration suites already require it.)
    #[test]
    fn resolve_reads_a_real_repo_and_none_outside_one() {
        let ok = std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success());
        if !ok {
            return;
        }
        let tmp = std::env::temp_dir().join(format!(
            "par-mux-gitbadge-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(tmp.join("repo")).unwrap();
        let git = |args: &[&str], cwd: &Path| {
            std::process::Command::new("git")
                .args(["-c", "core.fsmonitor=false", "-C"])
                .arg(cwd)
                .args(args)
                .env("GIT_TERMINAL_PROMPT", "0")
                .output()
                .unwrap()
        };
        let repo = tmp.join("repo");
        // fsmonitor: bypassed in the test setup, same discipline.
        git(&["init", "-b", "feature/gitbadge"], &repo);
        std::fs::write(repo.join("f.txt"), "x\n").unwrap();
        git(&["add", "."], &repo);
        git(
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-m",
                "c",
            ],
            &repo,
        );
        reset_cache();
        let badge = resolve(&repo).expect("a committed repo badges");
        assert_eq!(badge.branch, "feature/gitbadge");
        assert_eq!((badge.ahead, badge.behind), (0, 0));
        // The plain directory next door is not a repo.
        reset_cache();
        let plain = tmp.join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        assert_eq!(resolve(&plain), None);
        reset_cache();
        assert_eq!(cached(None), None, "no directory, no badge");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The snapshot's contract: the cache serves stale entries too
    /// (stale-while-revalidate — a reply path never resolves), and the
    /// prune drops what nothing refreshes any more.
    #[test]
    fn cache_serves_stale_entries_and_prunes_expired_ones() {
        let key: PathBuf = std::env::temp_dir().join("par-mux-gitbadge-stale");
        {
            let mut entries = cache().lock();
            entries.insert(
                key.clone(),
                CacheEntry {
                    at: Instant::now(),
                    badge: Some(GitBadge {
                        branch: "fresh".to_string(),
                        ahead: 0,
                        behind: 0,
                    }),
                },
            );
        }
        assert_eq!(
            cached(Some(&key)).map(|b| b.branch),
            Some("fresh".to_string()),
            "a fresh entry serves"
        );
        {
            let mut entries = cache().lock();
            entries.insert(
                key.clone(),
                CacheEntry {
                    at: Instant::now() - TEST_TTL,
                    badge: Some(GitBadge {
                        branch: "stale".to_string(),
                        ahead: 0,
                        behind: 0,
                    }),
                },
            );
        }
        assert_eq!(
            cached(Some(&key)).map(|b| b.branch),
            Some("stale".to_string()),
            "a stale entry still serves — the refresher owns the recompute"
        );
        // The prune drops what nothing has refreshed within
        // PRUNE_TTLS × ttl; a fresh-enough entry survives it.
        prune(TEST_TTL.saturating_mul(PRUNE_TTLS));
        assert_eq!(
            cached(Some(&key)).map(|b| b.branch),
            Some("stale".to_string()),
            "inside the prune bound the entry survives"
        );
        prune(Duration::ZERO);
        assert_eq!(cached(Some(&key)), None, "past the bound the entry drops");
        let _ = std::fs::remove_file(&key);
    }
}
