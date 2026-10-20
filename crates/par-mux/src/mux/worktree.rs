//! Git plumbing for the `worktree-*` control commands: every git run goes
//! through [`git`] with argv-only arguments (no shell), and every error
//! comes back as one line fit for a `%error` body.
//!
//! Checkout layout (the default when `worktree-create` gets no `-p`): a
//! sibling of the repo's MAIN checkout, `<parent>/<repo>-worktrees/<slug>`,
//! where `<slug>` is the branch lowercased with every non-alphanumeric run
//! collapsed to `-` (herdr's `branch_to_path_slug`). herdr defaults to one
//! global root (`~/.herdr/worktrees/<repo>/<slug>`); par-mux keeps the
//! checkouts next to the repo instead, so they share its filesystem and
//! never sit inside the main checkout's own `git status`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// One git invocation rooted at `dir`. The daemon runs git on a user's
/// explicit request, so the repo's hooks and filters stay live (an LFS
/// checkout needs its post-checkout hook) — unlike the host probe's
/// SEC-115 hardening for unrequested reads. What is closed: an inherited
/// `GIT_DIR`/`GIT_WORK_TREE`/`GIT_INDEX_FILE` redirecting the repo away
/// from `-C`, a credential prompt waiting on a terminal nobody sees
/// (`GIT_TERMINAL_PROMPT=0`, null stdin), and localized diagnostics
/// (`LC_ALL=C`) in the error text.
///
/// `core.fsmonitor=false`: a filesystem monitor answers "nothing changed"
/// for a write whose event it has not received yet (macOS FSEvents defers
/// delivery under load), so status reported a dirty checkout clean and
/// `worktree remove` without `--force` deleted its untracked files. `-c`
/// reaches the child status `worktree remove` runs, too.
fn git(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .args(["-c", "core.fsmonitor=false"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdin(Stdio::null());
    command
}

/// Collapse `text` to one line: whitespace runs become single spaces and
/// control characters go, so git's multi-line diagnostics fit one
/// `%error` line.
fn one_line(text: &str) -> String {
    crate::mux::strip_controls(&text.split_whitespace().collect::<Vec<_>>().join(" ")).into_owned()
}

/// Run git in `dir` with `args`; stdout on success, git's diagnostic
/// (stderr, else stdout, else the status) as one line on failure.
fn run(dir: &Path, args: &[&std::ffi::OsStr]) -> Result<String, String> {
    let output = git(dir)
        .args(args)
        .output()
        .map_err(|err| format!("cannot run git: {err}"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let stderr = one_line(&String::from_utf8_lossy(&output.stderr));
    let stdout = one_line(&String::from_utf8_lossy(&output.stdout));
    Err(if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        format!("git failed with {}", output.status)
    })
}

/// [`run`] over string arguments.
fn run_str(dir: &Path, args: &[&str]) -> Result<String, String> {
    let args: Vec<&std::ffi::OsStr> = args.iter().map(std::ffi::OsStr::new).collect();
    run(dir, &args)
}

/// `path` resolved for comparison: canonicalized when it exists (macOS
/// temp dirs come back from git as `/private/var/…` for a `/var/…`
/// input), with Windows' verbatim `\\?\` prefix dropped so it compares
/// equal to the `C:\…` form git's own paths canonicalize to.
pub(crate) fn canonical(path: &Path) -> PathBuf {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    #[cfg(windows)]
    if let Some(rest) = resolved.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        if !rest.starts_with("UNC\\") {
            return PathBuf::from(rest);
        }
    }
    resolved
}

/// A user-supplied path: absolute, or `~/…` under the daemon user's home.
/// A relative path is refused — it would resolve against the DAEMON's
/// cwd, not the client's.
pub(crate) fn user_path(raw: &str) -> Result<PathBuf, String> {
    let expanded = match raw.strip_prefix("~/") {
        Some(rest) => dirs::home_dir()
            .ok_or_else(|| format!("cannot expand {raw}: no home directory"))?
            .join(rest),
        None if raw == "~" => {
            dirs::home_dir().ok_or_else(|| format!("cannot expand {raw}: no home directory"))?
        }
        None => PathBuf::from(raw),
    };
    if !expanded.is_absolute() {
        return Err(format!("path must be absolute or start with ~/: {raw}"));
    }
    Ok(expanded)
}

/// The branch's directory name: lowercased ASCII alphanumerics, every
/// other run collapsed to one `-`, trimmed; `worktree` when nothing is
/// left.
pub(crate) fn branch_slug(branch: &str) -> String {
    let mut slug = String::new();
    for ch in branch.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "worktree".to_string()
    } else {
        slug.to_string()
    }
}

/// The default checkout for `branch` (see the module docs):
/// `<parent of main>/<main's name>-worktrees/<slug>`.
pub(crate) fn default_checkout_path(main: &Path, branch: &str) -> PathBuf {
    let repo = main
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".to_string());
    main.parent()
        .unwrap_or(main)
        .join(format!("{repo}-worktrees"))
        .join(branch_slug(branch))
}

/// The repo's main checkout, from any directory inside it or one of its
/// linked worktrees: the first `git worktree list --porcelain` entry. A
/// bare repository has no checkout to anchor a workspace on and is
/// refused.
pub(crate) fn main_checkout(dir: &Path) -> Result<PathBuf, String> {
    let listing = run_str(dir, &["worktree", "list", "--porcelain"])?;
    let mut lines = listing.lines();
    let first = lines
        .next()
        .and_then(|line| line.strip_prefix("worktree "))
        .ok_or_else(|| format!("{}: not a git repository", dir.display()))?;
    let bare = lines
        .take_while(|line| !line.is_empty())
        .any(|line| line == "bare");
    if bare {
        return Err(format!("{first}: bare repositories are not supported"));
    }
    Ok(canonical(Path::new(first)))
}

/// The work tree containing `dir` (`git rev-parse --show-toplevel`).
pub(crate) fn toplevel(dir: &Path) -> Result<PathBuf, String> {
    let out = run_str(dir, &["rev-parse", "--show-toplevel"])?;
    Ok(canonical(Path::new(out.trim())))
}

/// The branch checked out in `checkout`, `None` when HEAD is detached.
pub(crate) fn current_branch(checkout: &Path) -> Option<String> {
    run_str(checkout, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .ok()
        .map(|out| out.trim().to_string())
        .filter(|branch| !branch.is_empty())
}

/// Refuse a branch name git would not create. The leading-`-` check comes
/// first: such a value would read as an option to the commands it is
/// later passed to.
pub(crate) fn validate_branch(repo: &Path, branch: &str) -> Result<(), String> {
    if branch.starts_with('-') {
        return Err(format!("invalid branch name: {branch}"));
    }
    run_str(repo, &["check-ref-format", "--branch", branch])
        .map(|_| ())
        .map_err(|_| format!("invalid branch name: {branch}"))
}

/// `git worktree add` a checkout of `branch` at `path`: an existing local
/// branch is checked out as-is, otherwise it is created from `base`.
pub(crate) fn add(repo: &Path, path: &Path, branch: &str, base: &str) -> Result<(), String> {
    if base.starts_with('-') {
        return Err(format!("invalid base: {base}"));
    }
    let local = format!("refs/heads/{branch}");
    let exists = git(repo)
        .args(["show-ref", "--verify", "--quiet", &local])
        .status()
        .map_err(|err| format!("cannot run git: {err}"))?
        .success();
    let path = path.as_os_str();
    let os = std::ffi::OsStr::new;
    if exists {
        run(repo, &[os("worktree"), os("add"), path, os(branch)])
    } else {
        run(
            repo,
            &[
                os("worktree"),
                os("add"),
                os("-b"),
                os(branch),
                path,
                os(base),
            ],
        )
    }
    .map(|_| ())
}

/// Whether `checkout` has modified, staged, or untracked files.
/// `--no-optional-locks` keeps this read from rewriting the user's index
/// (which, with fsmonitor off, would drop its fsmonitor extension).
pub(crate) fn is_dirty(checkout: &Path) -> Result<bool, String> {
    run_str(
        checkout,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain",
            "--untracked-files=all",
        ],
    )
    .map(|out| !out.trim().is_empty())
}

/// `git worktree remove [--force] <checkout>` from the main checkout. A
/// checkout whose directory is already gone is pruned instead — git
/// refuses to remove what it cannot find.
pub(crate) fn remove(repo: &Path, checkout: &Path, force: bool) -> Result<(), String> {
    if !checkout.exists() {
        return run_str(repo, &["worktree", "prune"]).map(|_| ());
    }
    let os = std::ffi::OsStr::new;
    let mut args = vec![os("worktree"), os("remove")];
    if force {
        args.push(os("--force"));
    }
    args.push(checkout.as_os_str());
    run(repo, &args).map(|_| ())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A git command isolated from the developer's global and system
    /// config (signing, hooks, templates), with a fixed identity.
    pub(crate) fn test_git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=par-mux test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("run git");
        assert!(status.success(), "git {args:?} failed in {}", dir.display());
    }

    /// A one-commit repo at `<tempdir>/repo` (a subdirectory, so the
    /// sibling `repo-worktrees/` lands inside the tempdir too).
    pub(crate) fn test_repo() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        test_git(&repo, &["init", "-q"]);
        std::fs::write(repo.join("README"), "hi\n").unwrap();
        test_git(&repo, &["add", "README"]);
        test_git(&repo, &["commit", "-q", "-m", "init"]);
        let repo = canonical(&repo);
        (dir, repo)
    }

    #[test]
    fn branch_slug_collapses_separators() {
        assert_eq!(branch_slug("feature/Login-Fix"), "feature-login-fix");
        assert_eq!(branch_slug("a//b__c"), "a-b-c");
        assert_eq!(branch_slug("///"), "worktree");
    }

    #[test]
    fn default_checkout_is_a_sibling_of_the_main_checkout() {
        assert_eq!(
            default_checkout_path(Path::new("/src/app"), "feat/x"),
            Path::new("/src/app-worktrees/feat-x")
        );
    }

    #[test]
    fn user_path_refuses_relative_paths() {
        assert!(user_path("relative/dir").is_err());
        assert!(user_path("/abs/dir").is_ok());
        if let Some(home) = dirs::home_dir() {
            assert_eq!(user_path("~/x").unwrap(), home.join("x"));
        }
    }

    #[test]
    fn add_list_dirty_and_remove_round_trip() {
        let (_dir, repo) = test_repo();
        assert_eq!(main_checkout(&repo).unwrap(), repo);
        validate_branch(&repo, "topic").unwrap();
        assert!(validate_branch(&repo, "-x").is_err());
        assert!(validate_branch(&repo, "bad..name").is_err());

        let checkout = default_checkout_path(&repo, "topic");
        add(&repo, &checkout, "topic", "HEAD").unwrap();
        let checkout = canonical(&checkout);
        assert_eq!(toplevel(&checkout).unwrap(), checkout);
        assert_eq!(main_checkout(&checkout).unwrap(), repo);
        assert_eq!(current_branch(&checkout).as_deref(), Some("topic"));
        assert!(!is_dirty(&checkout).unwrap());

        std::fs::write(checkout.join("scratch"), "x").unwrap();
        assert!(is_dirty(&checkout).unwrap());
        let err = remove(&repo, &checkout, false).unwrap_err();
        assert!(!err.contains('\n'), "one-line error: {err}");
        assert!(checkout.exists());
        remove(&repo, &checkout, true).unwrap();
        assert!(!checkout.exists());

        // An existing branch is checked out, not re-created.
        let again = default_checkout_path(&repo, "topic");
        add(&repo, &again, "topic", "HEAD").unwrap();
        assert_eq!(current_branch(&again).as_deref(), Some("topic"));
    }

    /// Point `repo`'s config at an fsmonitor hook that always answers "no
    /// changes" — an FSEvents stream that has not delivered the write yet —
    /// with the untracked cache on, then prime `checkout`'s index with
    /// status runs that store the hook's token. The untracked mode is
    /// pinned to `all` because the cache resets when a run's mode differs
    /// from the one it was built with.
    #[cfg(unix)]
    fn lagging_fsmonitor(dir: &Path, repo: &Path, checkout: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let hook = dir.join("fsmonitor-hook");
        std::fs::write(&hook, "#!/bin/sh\nprintf 'tok\\0'\n").unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        test_git(repo, &["config", "core.fsmonitor", hook.to_str().unwrap()]);
        test_git(repo, &["config", "core.untrackedcache", "true"]);
        test_git(repo, &["config", "status.showUntrackedFiles", "all"]);
        for _ in 0..2 {
            test_git(
                checkout,
                &["status", "--porcelain", "--untracked-files=all"],
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn is_dirty_ignores_a_lagging_fsmonitor() {
        let (dir, repo) = test_repo();
        lagging_fsmonitor(dir.path(), &repo, &repo);
        std::fs::write(repo.join("scratch"), "x").unwrap();
        assert!(is_dirty(&repo).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn remove_refuses_a_dirty_checkout_despite_a_lagging_fsmonitor() {
        let (dir, repo) = test_repo();
        let checkout = default_checkout_path(&repo, "topic");
        add(&repo, &checkout, "topic", "HEAD").unwrap();
        lagging_fsmonitor(dir.path(), &repo, &checkout);
        std::fs::write(checkout.join("scratch"), "x").unwrap();
        assert!(remove(&repo, &checkout, false).is_err());
        assert!(checkout.join("scratch").exists());
    }
}
