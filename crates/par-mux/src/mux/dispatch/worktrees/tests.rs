use super::super::{dispatch_command, Ctx};
use crate::mux::command::parse_command;
use crate::mux::ids::{PaneId, WorkspaceId};
use crate::mux::pane::{MuxError, MuxPane, PaneFactory, ShellPaneFactory, SpawnContext};
use crate::mux::tree::MuxTree;
use crate::mux::worktree::tests::test_repo;
use crate::mux::worktree::{canonical, default_checkout_path};
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Records every spawn's cwd and runs a bounded sleeper instead of a
/// shell, so no pane process keeps a checkout busy past the test.
#[derive(Clone, Default)]
struct CwdFactory(Arc<Mutex<Vec<Spawned>>>);

/// One recorded spawn: the pane and the cwd its context carried.
type Spawned = (PaneId, Option<PathBuf>);

impl PaneFactory for CwdFactory {
    fn create_pane(
        &self,
        id: PaneId,
        cols: u16,
        rows: u16,
        _command: Option<&str>,
        context: &SpawnContext<'_>,
    ) -> Result<MuxPane, MuxError> {
        self.0.lock().push((id, context.cwd.map(Path::to_path_buf)));
        ShellPaneFactory::default().create_pane(id, cols, rows, Some("sleep 60"), context)
    }

    fn create_dead_pane(
        &self,
        id: PaneId,
        cols: u16,
        rows: u16,
        command: Option<&str>,
        exit_code: Option<i32>,
    ) -> Result<MuxPane, MuxError> {
        ShellPaneFactory::default().create_dead_pane(id, cols, rows, command, exit_code)
    }
}

struct Harness {
    tree: Arc<Mutex<MuxTree>>,
    clients: crate::mux::server::Clients,
    spawns: CwdFactory,
}

impl Harness {
    fn new() -> Self {
        let spawns = CwdFactory::default();
        Self {
            tree: Arc::new(Mutex::new(MuxTree::new(Box::new(spawns.clone())))),
            clients: Arc::new(Mutex::new(Vec::new())),
            spawns,
        }
    }

    /// Run one command line; `(ok, body)`.
    fn run(&self, line: &str) -> (bool, String) {
        let __output_epoch: crate::mux::server::OutputEpoch = std::sync::Arc::default();
        let ctx = Ctx {
            output_epoch: &__output_epoch,
            tree: &self.tree,
            clients: &self.clients,
            command_number: 1,
            shutdown: None,
            config: None,
            client_id: None,
        };
        let reply = dispatch_command(parse_command(line).unwrap(), &ctx, None, None);
        let ok = !reply.contains("%error");
        let body: Vec<&str> = reply
            .lines()
            .filter(|l| {
                !l.starts_with("%begin") && !l.starts_with("%end") && !l.starts_with("%error")
            })
            .collect();
        (ok, body.join("\n"))
    }

    fn ok(&self, line: &str) -> String {
        let (ok, body) = self.run(line);
        assert!(ok, "{line} failed: {body}");
        body
    }

    fn workspace_id(body: &str) -> WorkspaceId {
        body.trim().parse().expect("a +N workspace id")
    }

    fn live(&self, id: WorkspaceId) -> bool {
        self.tree.lock().workspace(id).is_some()
    }
}

fn quoted(path: &Path) -> String {
    format!("'{}'", path.display())
}

#[test]
fn worktree_create_opens_a_workspace_grouped_under_the_repo_workspace() {
    let (_dir, repo) = test_repo();
    let h = Harness::new();
    let body = h.ok(&format!("worktree-create -c {} -b feat/one", quoted(&repo)));
    let linked = Harness::workspace_id(&body);

    let checkout = canonical(&default_checkout_path(&repo, "feat/one"));
    assert!(checkout.is_dir(), "git worktree add ran");

    let guard = h.tree.lock();
    let ws = guard.workspace(linked).unwrap();
    let link = ws.worktree.as_ref().expect("linked workspace");
    assert!(link.is_linked());
    assert_eq!(link.repo, repo);
    assert_eq!(link.checkout, checkout);
    assert_eq!(link.branch.as_deref(), Some("feat/one"));
    assert_eq!(ws.name, "repo/feat/one");
    assert_eq!(
        guard.active_workspace(),
        Some(linked),
        "the new workspace is focused"
    );

    let parent = guard.repo_workspace(&repo).expect("repo workspace opened");
    assert_ne!(parent, linked);
    assert_eq!(guard.workspace(parent).unwrap().name, "repo");
    assert_eq!(guard.linked_worktrees_of(parent), vec![linked]);
    drop(guard);

    let cwds: Vec<Option<PathBuf>> = h.spawns.0.lock().iter().map(|(_, c)| c.clone()).collect();
    assert_eq!(cwds, vec![Some(repo.clone()), Some(checkout.clone())]);

    let list = h.ok("list-worktrees");
    assert_eq!(
        list,
        format!(
            "{parent} repo - main {}\n{linked} worktree {parent} feat/one {}",
            repo.display(),
            checkout.display()
        )
    );
    // client-snapshot carries the same rows, `worktree `-prefixed, after
    // its agent rows: the side panel's grouping source.
    let first_pane = h.spawns.0.lock()[0].0;
    let window = h.tree.lock().window_of_pane(first_pane).unwrap();
    // The badge comes from the refresher's cache; tick it inline the way
    // the daemon's thread does — a reply path no longer resolves.
    crate::mux::git_status::refresh_known_badges(&h.tree.lock(), std::time::Duration::from_secs(9));
    let snapshot = h.ok(&format!("client-snapshot -t {window}"));
    let worktree_rows: Vec<&str> = snapshot
        .lines()
        .filter_map(|l| l.strip_prefix("worktree "))
        .collect();
    assert_eq!(worktree_rows.join("\n"), list);
    assert!(
        snapshot.lines().last().unwrap().starts_with("git +"),
        "the git badge row appends after the worktree rows: {snapshot}"
    );

    // A second create reuses the open repo workspace.
    let second =
        Harness::workspace_id(&h.ok(&format!("worktree-create -c {} -b two", quoted(&repo))));
    assert_eq!(
        h.tree.lock().linked_worktrees_of(parent),
        vec![linked, second]
    );
}

#[test]
fn worktree_create_refuses_bad_input_before_running_git() {
    let (_dir, repo) = test_repo();
    let h = Harness::new();
    let (ok, err) = h.run(&format!("worktree-create -c {} -b -bad", quoted(&repo)));
    assert!(!ok && err.contains("invalid branch"), "{err}");
    let (ok, err) = h.run("worktree-create -c relative/dir -b x");
    assert!(!ok && err.contains("absolute"), "{err}");
    let (ok, err) = h.run(&format!(
        "worktree-create -c {} -b x --base --orphan",
        quoted(&repo)
    ));
    assert!(!ok && err.contains("invalid base"), "{err}");
    let (ok, err) = h.run(&format!(
        "worktree-create -c {} -b x -p {}",
        quoted(&repo),
        quoted(&repo)
    ));
    assert!(!ok && err.contains("already exists"), "{err}");
    assert!(h.tree.lock().workspaces().is_empty(), "nothing opened");
}

#[test]
fn worktree_open_focuses_an_open_checkout_and_opens_a_closed_one() {
    let (_dir, repo) = test_repo();
    let h = Harness::new();
    let linked =
        Harness::workspace_id(&h.ok(&format!("worktree-create -c {} -b topic", quoted(&repo))));
    let checkout = canonical(&default_checkout_path(&repo, "topic"));
    let parent = h.tree.lock().repo_workspace(&repo).unwrap();

    // Focus: a subdirectory of the open checkout resolves to it.
    std::fs::create_dir(checkout.join("sub")).unwrap();
    h.ok(&format!("select-workspace -t {parent}"));
    let again = Harness::workspace_id(&h.ok(&format!(
        "worktree-open -c {}",
        quoted(&checkout.join("sub"))
    )));
    assert_eq!(again, linked);
    assert_eq!(h.tree.lock().active_workspace(), Some(linked));

    // Closed checkout: reopened, regrouped under the open repo workspace.
    h.ok(&format!("kill-workspace -t {linked}"));
    let reopened = Harness::workspace_id(&h.ok(&format!("worktree-open -c {}", quoted(&checkout))));
    assert_ne!(reopened, linked);
    assert_eq!(h.tree.lock().linked_worktrees_of(parent), vec![reopened]);

    // Opening the main checkout focuses the repo workspace.
    assert_eq!(
        Harness::workspace_id(&h.ok(&format!("worktree-open -c {}", quoted(&repo)))),
        parent
    );
    assert_eq!(h.tree.lock().active_workspace(), Some(parent));
}

/// The repo workspace died (its only session was killed) while a linked
/// worktree workspace stayed open: reopening the checkout reopens the
/// repo workspace for the group, yet focuses and replies the linked one.
#[test]
fn worktree_open_of_an_orphaned_checkout_reopens_the_parent_and_focuses_the_target() {
    let (_dir, repo) = test_repo();
    let h = Harness::new();
    let linked =
        Harness::workspace_id(&h.ok(&format!("worktree-create -c {} -b orphan", quoted(&repo))));
    let checkout = canonical(&default_checkout_path(&repo, "orphan"));
    let parent = h.tree.lock().repo_workspace(&repo).unwrap();
    let parent_session = h.tree.lock().workspace(parent).unwrap().sessions[0];
    h.ok(&format!("kill-session -t {parent_session}"));
    assert!(!h.live(parent) && h.live(linked));

    let reply = Harness::workspace_id(&h.ok(&format!("worktree-open -c {}", quoted(&checkout))));
    assert_eq!(reply, linked);
    let guard = h.tree.lock();
    assert_eq!(
        guard.active_workspace(),
        Some(linked),
        "the target is focused"
    );
    let new_parent = guard
        .repo_workspace(&repo)
        .expect("the group's parent reopened");
    assert_eq!(guard.linked_worktrees_of(new_parent), vec![linked]);
    assert_eq!(guard.workspaces().len(), 2);
}

#[test]
fn kill_workspace_of_a_parent_with_linked_worktrees_requires_g() {
    let (_dir, repo) = test_repo();
    let h = Harness::new();
    let a = Harness::workspace_id(&h.ok(&format!("worktree-create -c {} -b a", quoted(&repo))));
    let b = Harness::workspace_id(&h.ok(&format!("worktree-create -c {} -b b", quoted(&repo))));
    let parent = h.tree.lock().repo_workspace(&repo).unwrap();

    let (ok, err) = h.run(&format!("kill-workspace -t {parent}"));
    assert!(
        !ok,
        "a parent with linked worktrees must not close without -g"
    );
    assert!(err.contains("-g") && err.contains(&a.to_string()), "{err}");
    assert!(h.live(parent) && h.live(a) && h.live(b), "nothing closed");

    // A linked member closes alone without -g.
    h.ok(&format!("kill-workspace -t {b}"));
    assert!(!h.live(b) && h.live(parent));

    h.ok(&format!("kill-workspace -t {parent} -g"));
    assert!(!h.live(parent) && !h.live(a), "the group closed");
    assert!(
        default_checkout_path(&repo, "a").is_dir(),
        "closing workspaces leaves checkouts on disk"
    );
}

#[test]
fn worktree_remove_refuses_dirty_and_running_agents_unless_forced() {
    let (_dir, repo) = test_repo();
    let h = Harness::new();
    let linked =
        Harness::workspace_id(&h.ok(&format!("worktree-create -c {} -b wip", quoted(&repo))));
    let checkout = canonical(&default_checkout_path(&repo, "wip"));
    let parent = h.tree.lock().repo_workspace(&repo).unwrap();

    let (ok, err) = h.run(&format!("worktree-remove -t {parent}"));
    assert!(!ok && err.contains("not a linked worktree"), "{err}");

    // An agent claim refuses — any roster state, idle included: the
    // agent process still holds the checkout.
    let pane = {
        let guard = h.tree.lock();
        let session = guard.workspace(linked).unwrap().sessions[0];
        let window = guard.session(session).unwrap().windows[0];
        guard.window(window).unwrap().panes()[0]
    };
    h.tree
        .lock()
        .pane_mut(pane)
        .unwrap()
        .set_metadata("agent", "claude");
    for state in ["working", "idle"] {
        h.tree
            .lock()
            .pane_mut(pane)
            .unwrap()
            .set_metadata("agent_state", state);
        let (ok, err) = h.run(&format!("worktree-remove -t {linked}"));
        assert!(!ok && err.contains("agent claim"), "{state}: {err}");
    }
    h.tree
        .lock()
        .pane_mut(pane)
        .unwrap()
        .clear_metadata(&["agent", "agent_state"]);

    // A dirty checkout refuses.
    std::fs::write(checkout.join("scratch"), "x").unwrap();
    let (ok, err) = h.run(&format!("worktree-remove -t {linked}"));
    assert!(!ok && err.contains("modified or untracked"), "{err}");
    assert!(
        h.live(linked) && checkout.is_dir(),
        "a refusal changes nothing"
    );

    h.ok(&format!("worktree-remove -t {linked} -f"));
    assert!(!h.live(linked), "the workspace closed");
    assert!(!checkout.exists(), "the checkout is gone");
    assert!(h.live(parent), "the repo workspace stays");
    assert!(h.tree.lock().linked_worktrees_of(parent).is_empty());
}

#[test]
fn worktree_links_survive_a_persist_round_trip() {
    let (_dir, repo) = test_repo();
    let h = Harness::new();
    let linked =
        Harness::workspace_id(&h.ok(&format!("worktree-create -c {} -b keep", quoted(&repo))));
    let parent = h.tree.lock().repo_workspace(&repo).unwrap();
    let state = h.tree.lock().to_persist_state();
    let restored = MuxTree::from_persist_state(&state, Box::new(CwdFactory::default())).unwrap();
    assert_eq!(restored.repo_workspace(&repo), Some(parent));
    assert_eq!(restored.linked_worktrees_of(parent), vec![linked]);
    assert_eq!(
        restored.workspace(linked).unwrap().worktree,
        h.tree.lock().workspace(linked).unwrap().worktree
    );
}
