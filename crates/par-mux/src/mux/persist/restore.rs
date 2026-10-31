//! Restoring: reading the state file through its recovery tiers
//! (quarantine, last-good, snapshot history) and rebuilding a tree from it
//! (D3.5: spawn first, restore second).

use super::format::{
    clamp_restored_grid_dims, clamp_restored_window_size, unix_ms, PersistAgentSession,
    PersistError, PersistPane, PersistSession, PersistState, PersistWindow, FORMAT_VERSION,
};
use super::save::{kept_snapshots, lastgood_path, state_has_panes};
use crate::mux::agent_resume::resume_invocation;
use crate::mux::ids::{IdAllocator, PaneId, SessionId, WindowId, WorkspaceId};
use crate::mux::pane::{PaneFactory, SpawnContext};
use crate::mux::tree::{MuxSession, MuxTree, MuxWindow, MuxWorkspace};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

impl MuxTree {
    /// Rebuild a tree from persisted state (D3.5): spawn each live pane's
    /// replacement process first (a pane persisted held-dead gets a
    /// processless pane instead, ARC-114), then restore its terminal from the
    /// snapshot, so process startup bytes never overwrite restored content.
    ///
    /// Held-dead entries are honored against the daemon's CURRENT
    /// `remain-on-exit` setting (`crate::mux::config::daemon_remain_on_exit`):
    /// a daemon with auto-remove on drops them, one with the hold on
    /// restores them held-dead — `respawn-pane` available. The
    /// [`Self::from_persist_state_with_policy`] form pins the decision for
    /// tests.
    ///
    /// The id allocator resumes from the persisted counters, so restored
    /// panes keep their `%N` identities and new panes do not collide.
    pub fn from_persist_state(
        state: &PersistState,
        factory: Box<dyn PaneFactory>,
    ) -> Result<MuxTree, PersistError> {
        Self::from_persist_state_with_policy(
            state,
            factory,
            crate::mux::config::daemon_remain_on_exit(),
            // Test/embedder form: the resume-append table pins blank here;
            // the daemon boots through `_with_policy` with the resolved
            // table, so no test inherits the host's config entries. The
            // resume prompt pins empty the same way: no injected turn.
            &crate::mux::config::AgentAppend::default(),
            "",
        )
    }

    /// [`Self::from_persist_state`] with the held-dead policy pinned: with
    /// `remain_on_exit` false (the product default) every persisted dead
    /// entry is dropped — through the kill-pane cascade, so a window whose
    /// last pane was dead closes and an emptied session (and workspace) go
    /// with it; with it true the entry is restored held-dead exactly as
    /// ARC-114 defined.
    pub fn from_persist_state_with_policy(
        state: &PersistState,
        factory: Box<dyn PaneFactory>,
        remain_on_exit: bool,
        agent_append: &crate::mux::config::AgentAppend,
        agent_resume_prompt: &str,
    ) -> Result<MuxTree, PersistError> {
        if state.format_version != FORMAT_VERSION {
            return Err(PersistError::UnsupportedVersion {
                found: state.format_version,
                supported: FORMAT_VERSION,
            });
        }

        let mut panes = HashMap::new();
        // Each session with its windows in order; linked into the tree
        // through `MuxTree::insert_window` below, which maintains the
        // reverse indexes (ARC-096).
        let mut sessions: Vec<(MuxSession, Vec<MuxWindow>)> = Vec::new();
        // Panes whose persisted cwd was gone at restore — they spawned in
        // home and get a visible note after their content is restored.
        let mut cwd_fallbacks: HashMap<u32, String> = HashMap::new();
        let mut note_batches = Vec::new();

        // Each session's workspace restore policy, read before the pane
        // loop: panes spawn before `restore_workspaces` re-links the
        // workspaces, so the policy travels by session id. An unknown
        // spelling (a hand-edited file) degrades to the default.
        let mut policy_of_session: HashMap<u32, crate::mux::config::RestorePolicy> = HashMap::new();
        for workspace in &state.workspaces {
            let Some(policy) = workspace
                .restore_policy
                .as_deref()
                .and_then(crate::mux::config::RestorePolicy::parse)
            else {
                continue;
            };
            for session_id in &workspace.sessions {
                policy_of_session.insert(*session_id, policy);
            }
        }
        // `none` disables restore of the workspace's panes entirely: the
        // workspace itself still restores (empty), its sessions never do.
        let no_restore: std::collections::HashSet<u32> = policy_of_session
            .iter()
            .filter(|(_, policy)| **policy == crate::mux::config::RestorePolicy::NoRestore)
            .map(|(session_id, _)| *session_id)
            .collect();

        for session in &state.sessions {
            if no_restore.contains(&session.id) {
                continue;
            }
            let mut session_windows = Vec::with_capacity(session.windows.len());
            for window in &session.windows {
                // The state file is untrusted input: the window size feeds
                // the factory's grid allocation, so it is clamped before it
                // reaches one — the same bounds a live client's size
                // request is held to.
                let (cols, rows) = clamp_restored_window_size(window.cols, window.rows);
                for pane in &window.panes {
                    let created = spawn_restored_pane(
                        factory.as_ref(),
                        session,
                        window.id,
                        pane,
                        (cols, rows),
                        policy_of_session.get(&session.id).copied(),
                        agent_append,
                        agent_resume_prompt,
                        &mut cwd_fallbacks,
                    )?;
                    panes.insert(PaneId(pane.id), created);
                }
                for pane in &window.panes {
                    let restored = panes.get(&PaneId(pane.id)).expect("just inserted above");
                    note_batches.extend(restore_pane_screen(restored, pane, &cwd_fallbacks));
                }
                session_windows.push(restored_window(window, cols, rows));
            }
            sessions.push((
                MuxSession {
                    id: SessionId(session.id),
                    name: crate::mux::strip_controls(&session.name).into_owned(),
                    windows: Vec::new(),
                    active: session.active_window_index,
                    env: session.env.clone(),
                },
                session_windows,
            ));
        }

        let mut tree = MuxTree::new(factory);
        for batch in note_batches {
            tree.defer_observer_batch(batch);
        }
        tree.ids = IdAllocator::resume(state.next_ids);
        tree.panes = panes;
        tree.buffers = state.buffers.clone();
        for (session, windows) in sessions {
            let session_id = session.id;
            tree.sessions.insert(session_id, session);
            for window in windows {
                tree.insert_window(session_id, window);
            }
        }
        tree.restore_workspaces(state);
        // Panes were spawned at their window's full extent, but the restored
        // layout divides that extent — re-fit every terminal (and PTY) to
        // its geometry, exactly as a live resize would have, so a restart
        // lands in the same state a running server would be in.
        for window_id in tree.windows.keys().copied().collect::<Vec<_>>() {
            tree.sync_pane_sizes(window_id);
        }
        if !remain_on_exit {
            tree.drop_restored_dead_entries(state);
        }
        // The tree is still owned here, behind no lock, so the notes' and
        // re-fits' observer events go out now rather than riding into the
        // server.
        for batch in tree.take_observer_batches() {
            batch.deliver();
        }
        Ok(tree)
    }

    /// Restore the workspaces after the sessions they reference; a session
    /// id absent from the sessions map (a hand-edited file) is skipped,
    /// and a workspace left referencing nothing still restores as an
    /// empty workspace — the same state `new-workspace` creates.
    fn restore_workspaces(&mut self, state: &PersistState) {
        let mut active_workspace = None;
        for workspace in &state.workspaces {
            let workspace_id = WorkspaceId(workspace.id);
            let sessions: Vec<SessionId> = workspace
                .sessions
                .iter()
                .filter(|id| self.sessions.contains_key(&SessionId(**id)))
                .map(|id| SessionId(*id))
                .collect();
            for session_id in &sessions {
                self.session_workspace.insert(*session_id, workspace_id);
            }
            let active = workspace
                .active_session_index
                .min(sessions.len().saturating_sub(1));
            self.workspaces.insert(
                workspace_id,
                MuxWorkspace {
                    id: workspace_id,
                    name: crate::mux::strip_controls(&workspace.name).into_owned(),
                    sessions,
                    active,
                    respawn_last: workspace.respawn_last,
                    exit_action: workspace
                        .exit_action
                        .as_deref()
                        .and_then(crate::mux::config::ExitAction::parse),
                    restore_policy: workspace
                        .restore_policy
                        .as_deref()
                        .and_then(crate::mux::config::RestorePolicy::parse),
                    worktree: workspace
                        .worktree
                        .as_ref()
                        .map(crate::mux::tree::WorktreeLink::from),
                },
            );
            if state.active_workspace == Some(workspace.id) {
                active_workspace = Some(workspace_id);
            }
        }
        // An active pointer naming a workspace the file never carried
        // falls back to the first restored one.
        // Lowest id, not map order, when the file named no live workspace.
        self.active_workspace = active_workspace.or_else(|| self.workspaces.keys().min().copied());
    }

    /// Auto-remove (remain-on-exit = false): a persisted dead entry is
    /// dropped, honoring the CURRENT setting at restore time. The
    /// kill-pane cascade does the pruning — window and session go with a
    /// pane that was their last — so the restored tree is exactly what a
    /// live daemon with the setting off would have held: the reaper's
    /// keep-one-tab rule applies here too, so a dead entry that is (or,
    /// after earlier drops, becomes) its workspace's last pane is kept
    /// held dead only when that workspace opted into keep-one-tab
    /// (`respawn_last`); every other last pane cascades closed. No PTY
    /// concern: a dead entry is processless.
    fn drop_restored_dead_entries(&mut self, state: &PersistState) {
        let dead_ids: Vec<PaneId> = state
            .sessions
            .iter()
            .flat_map(|s| s.windows.iter())
            .flat_map(|w| w.panes.iter())
            .filter(|pane| pane.dead)
            .map(|pane| PaneId(pane.id))
            .collect();
        for pane_id in dead_ids {
            if !self.respawns_as_workspace_last_pane(pane_id) {
                let _ = self.kill_pane(pane_id);
            }
        }
    }
}

/// Spawn one persisted pane's replacement: a processless held-dead pane
/// for a dead entry (ARC-114), else a live process — the agent's resume
/// invocation when one builds, the original command otherwise — in the
/// persisted cwd. Then re-attach its agent identity, user title, and (for
/// a dead pane) its OSC 7 cwd. A cwd that vanished records a note in
/// `cwd_fallbacks` for [`restore_pane_screen`] to print.
#[allow(clippy::too_many_arguments)]
fn spawn_restored_pane(
    factory: &dyn PaneFactory,
    session: &PersistSession,
    window_id: u32,
    pane: &PersistPane,
    (cols, rows): (u16, u16),
    restore_policy: Option<crate::mux::config::RestorePolicy>,
    agent_append: &crate::mux::config::AgentAppend,
    agent_resume_prompt: &str,
    cwd_fallbacks: &mut HashMap<u32, String>,
) -> Result<crate::mux::pane::MuxPane, PersistError> {
    // The effective command (D6.3): a resumable agent session rewrites
    // what the pane respawns as. The start-command choice (the pane's own
    // `--restore start-command` flag, or its workspace's
    // `start-command` policy) re-runs the ORIGINAL start command — every
    // flag — exporting the hook-reported session id as
    // PAR_MUX_RESUME_SESSION_ID, so a supervised wrapper passes --resume
    // itself. Otherwise the historical resume chain: the hook-reported
    // invocation first, the per-agent table second, handed to the factory
    // as STRUCTURED argv so Windows can spawn without a cmd.exe string
    // re-parse (the factory's string path POSIX-quotes, which cmd treats
    // as literal characters), with the `[daemon.agent-append]` words
    // riding along, deduped against the tokens the resume shape itself
    // carries. Every failure mode of that chain is an Option degrading to
    // the pane's original `spawn_command`, exactly Phase 3 behavior; no
    // retry and no probe (an agent that accepts a resume flag and starts
    // fresh is 6.4's after-the-fact question, not spawn time's). A chain
    // that BUILDS but fails at runtime (uninstalled binary, rejected
    // session id) renders with a fallback tail so the failure lands the
    // pane on a live shell rather than a reaper deletion.
    // A pane persisted held-dead has no process to resume or to land in a
    // cwd: it is rebuilt processless below, so neither the agent resume
    // invocation nor the cwd spawn logic runs for it (ARC-114).
    let via_start_command = !pane.dead
        && (pane.restore_start_command
            || restore_policy == Some(crate::mux::config::RestorePolicy::StartCommand));
    let resume_argv = if via_start_command {
        None
    } else {
        pane.agent_session
            .as_ref()
            .filter(|_| !pane.dead)
            .map(|agent_session| {
                resume_invocation(agent_session).map(|argv| {
                    let argv = crate::mux::agent_resume::argv_with_agent_append(
                        argv,
                        &agent_session.agent,
                        agent_append,
                        pane.cwd.as_deref(),
                        pane.id,
                    );
                    // The continuation turn: a configured, agent-supported
                    // prompt rides as the trailing positional so the resumed
                    // session starts working instead of idling at its prompt.
                    match crate::mux::agent_resume::resume_prompt(
                        agent_session,
                        agent_resume_prompt,
                    ) {
                        Some(prompt) => {
                            let mut argv = argv;
                            argv.push(prompt);
                            argv
                        }
                        None => argv,
                    }
                })
            })
            .and_then(|argv| argv)
    };
    // The spawn env: the captured agent exec-env snapshot as the base (a
    // resumed agent relaunches with the environment it originally ran
    // with, not the daemon's launch context), explicit session env entries
    // over it (the newer, deliberate mux-side decisions), and the stale
    // keys stripped — a hand-edited state file must not smuggle the
    // previous daemon's socket endpoint into the new process. The factory
    // re-exports the current `PAR_MUX_*` identity set AFTER the session
    // env, so the keys the capture stripped come back fresh; the
    // start-command path adds its resume hook on top of the same base.
    let mut spawn_env = pane
        .agent_session
        .as_ref()
        .and_then(|agent| agent.env.as_ref())
        .map(|env| super::format::strip_agent_env_for_persistence(env.clone()))
        .unwrap_or_default();
    spawn_env.extend(session.env.clone());
    let resume_env = if via_start_command {
        pane.agent_session
            .as_ref()
            .and_then(|agent| agent.session_id.as_deref())
            .map(|id| {
                let mut env = spawn_env.clone();
                env.insert("PAR_MUX_RESUME_SESSION_ID".to_string(), id.to_string());
                env
            })
    } else {
        None
    };
    let cwd = restored_spawn_cwd(pane, cwd_fallbacks);
    let context = SpawnContext {
        session: Some((SessionId(session.id), &session.name)),
        window: Some(WindowId(window_id)),
        env: Some(resume_env.as_ref().unwrap_or(&spawn_env)),
        cwd: cwd.as_deref(),
        // No client can connect before the accept loop starts, so restore
        // wires after the tree is built (`MuxServer::bind_with_tree`); the
        // bytes land in the grid that clients seed from.
        output: None,
    };
    let mut created = if pane.dead {
        factory.create_dead_pane(
            PaneId(pane.id),
            cols,
            rows,
            pane.spawn_command.as_deref(),
            pane.exit_code,
        )?
    } else {
        match resume_argv {
            Some(argv) => factory.create_argv_pane(PaneId(pane.id), cols, rows, &argv, &context)?,
            None => factory.create_pane(
                PaneId(pane.id),
                cols,
                rows,
                pane.spawn_command.as_deref(),
                &context,
            )?,
        }
    };
    if let Some(agent_session) = &pane.agent_session {
        restore_agent_claim(&mut created, agent_session);
    } else if !pane.dead {
        // A scrape-tier claim (adopt-pane, or an agent-start whose agent
        // never reported identity) saves no agent_session, so nothing
        // above restored its label. Re-derive it from what the pane
        // respawns as, under the same gate `adopt-pane` applies — a label
        // the tick cannot classify would sit on the pane inertly — and
        // the 1s tick writes the state and broadcasts from here.
        if let Some(kind) = agent_of_invocation(
            &crate::mux::scrape::ScrapeEngine::bundled(),
            pane.spawn_command.as_deref(),
        ) {
            created.set_metadata("agent", &kind);
        }
    }
    if let Some(title) = &pane.user_title {
        created.set_user_title(title);
    }
    // A re-restored pane keeps a cwd floor too: its spawn cwd is the
    // persisted cwd the restore just used (None only when the state had
    // none and the factory default answered).
    created.set_spawn_cwd(cwd.as_deref());
    if pane.dead {
        // `begin_respawn` reads the pane's cwd from the terminal's OSC 7
        // state (the dead pane has no child to ask), and the next save
        // captures it from the same place — so the persisted cwd is
        // seeded there rather than lost with the process.
        if let Some(dir) = &pane.cwd {
            created.with_terminal_mut(|term| {
                let si = term.shell_integration_mut();
                si.set_cwd(dir.clone());
                si.set_hostname(pane.cwd_host.clone());
            });
        }
    }
    Ok(created)
}

/// The agent kind a restored pane's invocation names, when the scrape
/// engine covers it. Reads only the first whitespace token's basename —
/// the persisted `spawn_command` is a shell-parsed string, and a fuller
/// parse would second-guess quoting for no roster gain. `None` leaves the
/// pane unlabeled: a plain shell, an editor editing `claude.md`, anything
/// the tier cannot classify stays off the roster exactly as a
/// hand-launched pane does until `adopt-pane` names it. Restore runs
/// before the server assigns the tree its engine, so tests and callers
/// pass the bundled set (what the daemon's engine degrades to without
/// `agent-patterns` overrides).
pub(super) fn agent_of_invocation(
    engine: &crate::mux::scrape::ScrapeEngine,
    invocation: Option<&str>,
) -> Option<String> {
    let first = invocation?.split_whitespace().next()?;
    let lowered = first.rsplit(['/', '\\']).next()?.to_ascii_lowercase();
    let program = lowered.strip_suffix(".exe").unwrap_or(&lowered);
    engine.set_for(program).map(|_| program.to_string())
}

/// The persisted cwd re-lands the pane where it left off. A directory
/// that vanished between save and restore would fail the spawn, so it
/// degrades to home — the spawn's success may not depend on a directory
/// this process cannot control — and the pane says so after its content
/// is restored (the note recorded in `cwd_fallbacks`). A dead pane spawns
/// nothing, so it gets no cwd.
fn restored_spawn_cwd(
    pane: &PersistPane,
    cwd_fallbacks: &mut HashMap<u32, String>,
) -> Option<PathBuf> {
    match pane.cwd.as_deref().map(Path::new).filter(|_| !pane.dead) {
        Some(dir) if dir.is_dir() => Some(dir.to_path_buf()),
        Some(dir) => {
            let home = dirs::home_dir().unwrap_or_default();
            cwd_fallbacks.insert(
                pane.id,
                format!(
                    "\r\npar-mux: {} is gone; pane restored in {}\r\n",
                    dir.display(),
                    home.display()
                ),
            );
            home.is_dir().then_some(home)
        }
        None => None,
    }
}

/// Identity comes back through the typed claim (ARC-113c) so the format
/// round-trips and task 6.3's hook-first lookup reads it from the same
/// place it reads a live pane's. The last-known state rides back too, so
/// the restored pane is on the roster before its agent's next hook report;
/// the seq and the sweep's miss counters do not (the restored pane reports
/// its own from scratch).
fn restore_agent_claim(
    created: &mut crate::mux::pane::MuxPane,
    agent_session: &PersistAgentSession,
) {
    let mut claim = crate::mux::hooks::AgentClaim {
        agent: agent_session.agent.clone(),
        state: agent_session.state.clone(),
        state_source: agent_session.state_source.clone(),
        message: agent_session.message.clone(),
        session_id: agent_session.session_id.clone(),
        session_path: agent_session.session_path.clone(),
        source: agent_session.source.clone(),
        ..Default::default()
    };
    // Verbatim argv passthrough: a malformed (hand-edited) argv string in
    // the state file cannot parse into the typed argv, so it is stored raw
    // rather than silently dropped — loading an old file never loses what
    // the file said.
    let mut raw_argv: Option<&str> = None;
    match agent_session
        .resume_argv
        .as_deref()
        .map(|raw| (raw, serde_json::from_str::<Vec<String>>(raw)))
    {
        Some((_, Ok(argv))) => claim.resume_argv = Some(argv),
        Some((raw, Err(_))) => raw_argv = Some(raw),
        None => {}
    }
    created.set_agent_claim(&claim);
    // Seed the agent-env cache with what the state file carried, keyed to
    // the same session identity the save side keys its fallback to — the
    // next save's env read covers a process that cannot be read then.
    if let Some(env) = &agent_session.env {
        created.set_agent_env_snapshot(crate::mux::pane::AgentEnvSnapshot {
            env: super::format::strip_agent_env_for_persistence(env.clone()),
            agent: agent_session.agent.clone(),
            session: agent_session
                .session_id
                .clone()
                .or_else(|| agent_session.session_path.clone()),
        });
    }
    if let Some(raw) = raw_argv {
        created.set_metadata("agent_resume_argv", raw);
    }
}

/// Restore one pane's screen from its snapshot, then print its
/// cwd-fallback note when it has one; returns the note's deferred
/// observer batch.
fn restore_pane_screen(
    restored_pane: &crate::mux::pane::MuxPane,
    pane: &PersistPane,
    cwd_fallbacks: &HashMap<u32, String>,
) -> Option<par_term_emu_core::terminal::ObserverDispatchBatch> {
    // Through the geometry-publishing path (QA-195): the restored cursor
    // and note are what `cursor_position()` serves before the new process
    // prints anything.
    restored_pane.with_terminal_mut(|restored| {
        // ARC-113c2: the snapshot's grid dims are state-file input exactly
        // like the window size, so clamp them before
        // `restore_for_new_process` adopts them.
        let mut snapshot = pane.terminal.clone();
        clamp_restored_grid_dims(&mut snapshot);
        if pane.dead {
            // No new process to protect: the frozen screen returns exactly
            // as the client saw it, modes and alt screen included.
            restored.restore_from_snapshot(snapshot);
        } else {
            restored.restore_for_new_process(snapshot);
        }
        // After the snapshot re-hangs, so the note is the last thing on
        // screen rather than scrolled away. Its observer events wait for
        // the terminal lock to drop and go out with the re-fits'.
        cwd_fallbacks
            .get(&pane.id)
            .map(|note| restored.process_deferred(note.as_bytes()))
    })
}

/// A persisted window as a tree window at the clamped `cols` x `rows`.
fn restored_window(window: &PersistWindow, cols: u16, rows: u16) -> MuxWindow {
    MuxWindow {
        id: WindowId(window.id),
        name: crate::mux::strip_controls(&window.name).into_owned(),
        layout: window.layout.clone(),
        active: PaneId(window.active_pane),
        cols,
        rows,
        // Zoom is session state, not layout — restored windows start
        // unzoomed (tmux's behavior).
        zoomed: None,
        chrome: Default::default(),
    }
}

/// The newest snapshot that parses to a current pane-bearing state —
/// persistence's third recovery tier, after the state file and `.lastgood`.
fn load_newest_snapshot(target: &Path) -> Option<PersistState> {
    for path in kept_snapshots(target).into_iter().rev() {
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        let Ok(state) = serde_json::from_slice::<PersistState>(&bytes) else {
            continue;
        };
        if state.format_version == FORMAT_VERSION && state_has_panes(&state) {
            log::info!(
                "par-mux: restoring the newest history snapshot {}",
                path.display()
            );
            return Some(state);
        }
    }
    None
}

/// What daemon startup found in the state file.
#[derive(Debug)]
pub enum Loaded {
    /// No state file exists — a fresh start.
    Fresh,
    /// A readable, current-version state a rebuild can start from.
    State(Box<PersistState>),
    /// A file existed but was corrupt or carried an unknown version; it has
    /// been renamed aside and the daemon starts fresh (D3.2: unreadable
    /// state never blocks startup — match tmux).
    Quarantined {
        /// The original state-file path.
        from: PathBuf,
        /// The path the file was renamed to.
        to: PathBuf,
    },
}

/// Read the state file at `target`, quarantining a corrupt or
/// unknown-version file aside so the next save cannot overwrite the
/// evidence (D3.2). Every failure degrades to a fresh start; nothing here
/// can block daemon startup.
///
/// A readable but pane-less state falls back to the last-good snapshot
/// when one holds panes: a reboot/logout race kills the panes before the
/// daemon's stop, and the empty tree those deaths persisted is not the
/// layout the user should lose.
pub fn load_or_quarantine(target: &Path) -> Loaded {
    let bytes = match fs::read(target) {
        Ok(bytes) => bytes,
        Err(_) => return Loaded::Fresh,
    };

    let reason = match serde_json::from_slice::<PersistState>(&bytes) {
        Ok(state) if state.format_version == FORMAT_VERSION => {
            if !state_has_panes(&state) {
                if let Some(good) = load_lastgood(target) {
                    log::info!(
                        "par-mux: state {} held no panes; restoring the last-good snapshot",
                        target.display()
                    );
                    return Loaded::State(Box::new(good));
                }
                if let Some(snapshot) = load_newest_snapshot(target) {
                    return Loaded::State(Box::new(snapshot));
                }
            }
            return Loaded::State(Box::new(state));
        }
        Ok(state) => PersistError::UnsupportedVersion {
            found: state.format_version,
            supported: FORMAT_VERSION,
        },
        Err(err) => PersistError::Serialize(err),
    };

    let mut name = target.as_os_str().to_os_string();
    name.push(format!(".quarantine-{}", unix_ms()));
    let to = PathBuf::from(name);
    match fs::rename(target, &to) {
        Ok(()) => {
            log::warn!(
                "par-mux: state {} was unreadable ({reason}); quarantined as {}",
                target.display(),
                to.display()
            );
            // The recovery tiers below the file itself: the last-good
            // sibling, then the newest history snapshot. Both were written
            // by earlier saves of this same daemon, so restoring from one
            // loses at most the interval since its last snapshot — far
            // less than a fresh start.
            if let Some(good) = load_lastgood(target) {
                log::info!("par-mux: restoring the last-good snapshot after the quarantine");
                return Loaded::State(Box::new(good));
            }
            if let Some(snapshot) = load_newest_snapshot(target) {
                return Loaded::State(Box::new(snapshot));
            }
            Loaded::Quarantined {
                from: target.to_path_buf(),
                to,
            }
        }
        Err(err) => {
            log::warn!(
                "par-mux: state {} was unreadable ({reason}) and could not be quarantined: {err}",
                target.display()
            );
            Loaded::Fresh
        }
    }
}

/// The pane-bearing snapshot beside `target`, if a usable one exists. A
/// derived cache, not evidence: corrupt or missing degrades to `None`
/// silently rather than quarantining.
fn load_lastgood(target: &Path) -> Option<PersistState> {
    let bytes = fs::read(lastgood_path(target)).ok()?;
    let state = serde_json::from_slice::<PersistState>(&bytes).ok()?;
    (state.format_version == FORMAT_VERSION && state_has_panes(&state)).then_some(state)
}

/// What a read-only peek at the state file predicts the next daemon start
/// will restore. Mirrors [`load_or_quarantine`]'s conclusions (including
/// the last-good fallback) without any of its mutations — the `--restart`
/// pre-flight uses it to tell the caller, on the still-open terminal, what
/// the detached fresh daemon is about to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatePeek {
    /// No state file — the next daemon starts fresh.
    Missing,
    /// The state file exists but is not parseable current-version state —
    /// the next daemon quarantines it aside and starts fresh.
    Unreadable,
    /// Parseable but pane-less with no usable last-good snapshot — the next
    /// daemon restores nothing.
    Empty,
    /// Panes would be restored (from the state file itself or the
    /// last-good snapshot fallback).
    Populated,
}

/// The read-only half of [`load_or_quarantine`]: classify the state file
/// without quarantining or otherwise mutating it. A read or parse failure
/// here stays a classification, never an error.
pub fn peek_state(target: &Path) -> StatePeek {
    let bytes = match fs::read(target) {
        Ok(bytes) => bytes,
        Err(_) => return StatePeek::Missing,
    };
    match serde_json::from_slice::<PersistState>(&bytes) {
        Ok(state) if state.format_version == FORMAT_VERSION => {
            if state_has_panes(&state) || load_lastgood(target).is_some() {
                StatePeek::Populated
            } else {
                StatePeek::Empty
            }
        }
        _ => StatePeek::Unreadable,
    }
}
