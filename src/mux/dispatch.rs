//! Per-command dispatch: one handler per [`MuxCommand`] variant plus the
//! shared post-dispatch tail.
//!
//! Each handler is small and shape-identical: take the tree lock, run one
//! tree operation, and report an [`Outcome`] — the reply block, lifecycle
//! notifications, issuer-only notifications, and the window whose layout
//! changed. The tail then emits everything in wire order, saves state for
//! mutating commands (par-mux.md D3.3), and returns the reply. Adding a
//! tmux command is one `parse_<cmd>` in [`crate::mux::command`], one
//! [`MuxCommand`] variant, one `cmd_<name>` handler here, and its match arm
//! in [`dispatch_command`].
//!
//! [`crate::mux::server`] keeps the accept loop, client threads, and the
//! broadcast sinks this module calls into.

use crate::mux::command::{list_commands_body, MuxCommand, ResizeAdjustment, SendKeysPayload};
use crate::mux::emit::{emit, emit_block};
use crate::mux::foreground::ProcessTable;
use crate::mux::ids::{AnyTarget, PaneId, SessionId, Target, WindowId, WorkspaceId};
use crate::mux::layout::SplitDirection;
use crate::mux::pane::{MuxError, OutputSink, PaneFactory};
use crate::mux::persist::{PersistState, SaveOrigin};
use crate::mux::server::{
    broadcast_layout_change, broadcast_notification, capture_range, pane_output_sink,
    replay_pane_exited_lines, Clients,
};
use crate::mux::tree::{MuxTree, SpawnPlan};
use crate::tmux_control::TmuxNotification;
use base64::Engine as _;
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, SyncSender};
use std::sync::Arc;

/// Default pane size for sessions and windows created without an explicit
/// size.
const DEFAULT_COLS: u16 = 80;
/// Default pane size for sessions and windows created without an explicit
/// size.
const DEFAULT_ROWS: u16 = 24;

/// The paste buffer's name (single-buffer, no numbered stack — D3 non-goal).
/// tmux's `set-buffer`/`show-buffer` grammar accepts an explicit `-b <name>`,
/// but Phase 2's client never sends one, so every buffer command targets
/// this one slot under the hood.
const DEFAULT_BUFFER: &str = "default";

// QA-113 test hook: when set on the dispatching thread, `dispatch_command`
// panics before running the command, so the containment tests can drive a
// guaranteed panic. Thread-local (QA-222): the former process-global flag
// panicked commands every concurrently running test dispatched during the
// injection window, forcing the mux::server tests to run serially.
#[cfg(test)]
thread_local! {
    pub(crate) static PANIC_ON_COMMAND: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

/// The per-dispatch context every handler shares.
pub(super) struct Ctx<'a> {
    /// The issuing connection's id ([`CLIENT_SEQ`]'s counter) — the key the
    /// client's sizing contribution is recorded under. `None` for embedders
    /// and tests that dispatch without a running accept loop; their
    /// `refresh-client -C` then keeps the legacy direct-resize behavior.
    pub(super) client_id: Option<u64>,
    /// The whole pane tree behind its lock.
    pub(super) tree: &'a Arc<Mutex<MuxTree>>,
    /// Connected clients' broadcast senders.
    pub(super) clients: &'a Clients,
    /// The issuing client's per-connection command counter — the `%begin`/
    /// `%end` number the reply block carries.
    pub(super) command_number: u32,
    /// The server's shutdown flag, for `kill-server`. `None` for embedders
    /// and tests that dispatch without a running accept loop.
    pub(super) shutdown: Option<&'a std::sync::atomic::AtomicBool>,
    /// The daemon's applied settings copy, behind its lock — what
    /// `reload-config` compares a re-read file against and what it would
    /// update. `None` for embedders and tests that dispatch without a
    /// configured server: `reload-config` then reports every setting
    /// restart-required (there is no applied copy to diff or update).
    pub(super) config: Option<&'a Mutex<crate::mux::config::EffectiveConfig>>,
}

/// What one handler produced: the reply block plus everything the shared
/// tail emits around it.
pub(super) struct Outcome {
    /// The `%begin`…`%end` block answering the command.
    pub(super) reply: String,
    /// Lifecycle notifications broadcast to every connected client.
    pub(super) notifications: Vec<TmuxNotification>,
    /// Notifications sent only to the issuing client (`%session-changed`).
    pub(super) issuer_only: Vec<TmuxNotification>,
    /// The windows a `%layout-change` is broadcast for, in order —
    /// usually one; a pane move touches both its windows.
    pub(super) layout_changed: Vec<WindowId>,
    /// Whether the handler's tree operation landed — gates persistence
    /// alongside [`MuxCommand::mutates`], so a mutating command that failed
    /// (`kill-pane` of an unknown pane) does not save.
    pub(super) succeeded: bool,
}

impl Outcome {
    /// A successful handler's reply block over `body`.
    fn ok(ctx: &Ctx<'_>, body: &str) -> Self {
        Self {
            reply: emit_block(ctx.command_number, body, true),
            notifications: Vec::new(),
            issuer_only: Vec::new(),
            layout_changed: Vec::new(),
            succeeded: true,
        }
    }

    /// A failed handler's error block over `message`.
    fn err(ctx: &Ctx<'_>, message: &str) -> Self {
        Self {
            reply: emit_block(ctx.command_number, message, false),
            notifications: Vec::new(),
            issuer_only: Vec::new(),
            layout_changed: Vec::new(),
            succeeded: false,
        }
    }

    /// Queue a lifecycle broadcast.
    fn notifying(mut self, notification: TmuxNotification) -> Self {
        self.notifications.push(notification);
        self
    }

    /// Queue a notification for the issuing client only.
    fn telling_issuer(mut self, notification: TmuxNotification) -> Self {
        self.issuer_only.push(notification);
        self
    }

    /// Queue a `%layout-change` window.
    fn with_layout(mut self, window_id: WindowId) -> Self {
        self.layout_changed.push(window_id);
        self
    }
}

/// Dispatch one parsed command: run its handler, then the shared tail.
///
/// `persist` (the daemon always passes one): when a mutating command
/// succeeded, the state is captured under the tree lock and handed to the
/// server's persist worker, which writes it off the lock (ARC-003;
/// par-mux.md D3.3 — D3.3 accepts losing the last window on `kill -9`, so
/// per-command synchronous durability is not required). `issuer` is the
/// channel of the client that sent the command, for notifications that
/// concern that client specifically; lifecycle broadcasts go to everyone
/// via `ctx.clients`.
///
/// The tail emits in the pre-decomposition wire order — `%layout-change`
/// first (it carries the geometry the pane-changed notification is read
/// against), then lifecycle broadcasts, then the issuer-only notifications.
pub(super) fn dispatch_command(
    command: MuxCommand,
    ctx: &Ctx<'_>,
    persist: Option<&Sender<(SaveOrigin, PersistState)>>,
    issuer: Option<&SyncSender<String>>,
) -> String {
    // QA-113 test hook: force a dispatcher panic to exercise the client
    // thread's containment (see `dispatch_contained`).
    #[cfg(test)]
    if crate::mux::dispatch::PANIC_ON_COMMAND.with(std::cell::Cell::get) {
        panic!("injected dispatcher panic (QA-113)");
    }
    let mutates = command.mutates();
    let outcome = match command {
        MuxCommand::NewSession {
            name,
            env,
            workspace,
        } => cmd_new_session(ctx, name, env, workspace),
        MuxCommand::ListPanes { window } => cmd_list_panes(ctx, window),
        MuxCommand::ListAgents => cmd_list_agents(ctx),
        MuxCommand::ListCommands => cmd_list_commands(ctx),
        MuxCommand::SendKeys { pane, keys } => cmd_send_keys(ctx, pane, &keys),
        MuxCommand::RefreshClient {
            pane,
            size,
            cell_pixels,
        } => cmd_refresh_client(ctx, pane, size, cell_pixels),
        MuxCommand::KillPane { pane } => cmd_kill_pane(ctx, pane),
        MuxCommand::SplitWindow {
            target,
            direction,
            percent,
            before,
            start_dir,
        } => cmd_split_window(
            ctx,
            target,
            direction,
            percent,
            before,
            start_dir.as_deref(),
        ),
        MuxCommand::SelectPane { pane, title } => cmd_select_pane(ctx, pane, title),
        MuxCommand::PaneTitle { pane } => cmd_pane_title(ctx, pane),
        MuxCommand::PaneInfo { pane } => cmd_pane_info(ctx, pane),
        MuxCommand::PaneExitedReplay => cmd_pane_exited_replay(ctx, issuer),
        MuxCommand::ClearHistory { pane } => cmd_clear_history(ctx, pane),
        MuxCommand::ResizePane { pane, adjustment } => cmd_resize_pane(ctx, pane, adjustment),
        MuxCommand::SwapPanes { target, source } => cmd_swap_panes(ctx, target, source),
        MuxCommand::BreakPane { source, name } => cmd_break_pane(ctx, source, name),
        MuxCommand::JoinPane {
            source,
            target,
            direction,
            percent,
        } => cmd_join_pane(ctx, source, target, direction, percent),
        MuxCommand::MoveWindow { source, index } => cmd_move_window(ctx, source, index),
        MuxCommand::SwapWindows { source, target } => cmd_swap_windows(ctx, source, target),
        MuxCommand::RespawnPane {
            pane,
            kill,
            start_dir,
            command,
        } => cmd_respawn_pane(ctx, pane, kill, start_dir.as_deref(), command),
        MuxCommand::NewWindow {
            target,
            name,
            start_dir,
        } => cmd_new_window(ctx, target, name, start_dir.as_deref()),
        MuxCommand::SelectWindow { window } => cmd_select_window(ctx, window),
        MuxCommand::KillWindow { window } => cmd_kill_window(ctx, window),
        MuxCommand::RenameSession { session, name } => cmd_rename_session(ctx, session, name),
        MuxCommand::KillSession { session } => cmd_kill_session(ctx, session),
        MuxCommand::RenameWindow { window, name } => cmd_rename_window(ctx, window, name),
        MuxCommand::ListWindows { session } => cmd_list_windows(ctx, session),
        MuxCommand::ListSessions { workspace } => cmd_list_sessions(ctx, workspace),
        MuxCommand::NewWorkspace { name } => cmd_new_workspace(ctx, name),
        MuxCommand::ListWorkspaces => cmd_list_workspaces(ctx),
        MuxCommand::SelectWorkspace { workspace } => cmd_select_workspace(ctx, workspace),
        MuxCommand::RenameWorkspace { workspace, name } => {
            cmd_rename_workspace(ctx, workspace, name)
        }
        MuxCommand::KillWorkspace { workspace } => cmd_kill_workspace(ctx, workspace),
        MuxCommand::KillServer => cmd_kill_server(ctx),
        MuxCommand::CapturePane {
            pane,
            start_line,
            end_line,
            escape,
        } => cmd_capture_pane(ctx, pane, start_line, end_line, escape),
        MuxCommand::SetBuffer { content } => cmd_set_buffer(ctx, content),
        MuxCommand::SetClientColors { fg, bg } => cmd_set_client_colors(ctx, fg, bg),
        MuxCommand::SetEnvironment {
            session,
            name,
            value,
        } => cmd_set_environment(ctx, session, &name, value.as_deref()),
        MuxCommand::ShowBuffer => cmd_show_buffer(ctx),
        MuxCommand::PasteBuffer { pane } => cmd_paste_buffer(ctx, pane),
        MuxCommand::Version => cmd_version(ctx),
        MuxCommand::ReloadConfig => cmd_reload_config(ctx),
    };

    for window_id in &outcome.layout_changed {
        broadcast_layout_change(ctx.tree, ctx.clients, *window_id);
    }
    for notification in &outcome.notifications {
        broadcast_notification(ctx.clients, notification);
    }
    for notification in &outcome.issuer_only {
        if let Some(tx) = issuer {
            let _ = tx.send(emit(notification));
        }
    }
    if mutates && outcome.succeeded {
        if let Some(tx) = persist {
            // Collect under the lock (cheap clones and field reads), then
            // capture OFF it — the grid walks and cwd syscalls a cache miss
            // triggers must not stall every client (ARC-032) — and hand off
            // to the worker, which serializes and fsyncs off the lock
            // (ARC-003). A send fails only if the worker is gone; the
            // shutdown save is the durability backstop.
            let capture = ctx.tree.lock().collect_persist_capture();
            let state = capture.capture();
            let _ = tx.send((SaveOrigin::Command, state));
        }
    }
    outcome.reply
}

/// Phases 2 and 3 of a two-phase spawn (ARC-022/ARC-103): spawn OFF the
/// tree lock with the output sink already in the context — the pane's
/// first bytes are forwarded, not lost before a later wire-up — then
/// re-lock to complete, re-install the same sink (for factories that
/// ignore [`crate::mux::pane::SpawnContext::output`]), and write any
/// start-dir note through the geometry-publishing path (QA-195).
///
/// With the sink live from the first byte, `%output %N` can reach clients
/// before the command's reply and before `%window-add`/`%layout-change`;
/// clients drop output for a pane they have not mapped, which loses at
/// most what was lost before, and the daemon grid always has the bytes.
///
/// The sink takes `clients.lock()` from the PTY reader thread, so this
/// never holds the clients lock while taking the tree lock.
fn spawn_and_wire<P: SpawnPlan>(
    ctx: &Ctx<'_>,
    factory: &dyn PaneFactory,
    plan: P,
    command: Option<&str>,
    note: Option<&str>,
) -> Result<P::Done, MuxError> {
    let pane_id = plan.pane_id();
    let (cols, rows) = plan.size();
    let sink = OutputSink(Arc::new(pane_output_sink(ctx.clients, pane_id)));
    let pane = {
        let mut context = plan.spawn_context();
        context.output = Some(&sink);
        factory.create_pane(pane_id, cols, rows, command, &context)?
    };
    let mut guard = ctx.tree.lock();
    let done = plan.complete(&mut guard, pane)?;
    if let Some(pane) = guard.pane_mut(pane_id) {
        pane.on_output_sink(sink);
        if let Some(note) = note {
            // Same visibility rule as a restore's gone cwd: the pane says
            // where it landed instead of silently starting elsewhere.
            pane.write_note(note.as_bytes());
        }
    }
    Ok(done)
}

/// Build the `%window-add` for a window this dispatch just created, with
/// its layout triple formatted exactly as [`broadcast_layout_change`]
/// renders a `%layout-change` (card 01a0ef3b) — clients lay the new
/// window out without a follow-up notification. A window that vanished
/// between creation and this read degrades to the bare-id form.
fn window_add_notification(tree: &Arc<Mutex<MuxTree>>, window_id: WindowId) -> TmuxNotification {
    let guard = tree.lock();
    let Some(window) = guard.window(window_id) else {
        return TmuxNotification::WindowAdd {
            window_id: window_id.to_string(),
            window_layout: String::new(),
            window_visible_layout: String::new(),
            window_raw_flags: String::new(),
        };
    };
    let layout = window
        .layout
        .render(0, 0, window.cols as usize, window.rows as usize);
    let (visible_layout, raw_flags) = match window.zoomed {
        Some(pane) => (
            // The single-pane form render_node produces for a leaf.
            format!("0000,{}x{},0,0,{}", window.cols, window.rows, pane.0),
            "Z".to_string(),
        ),
        None => (layout.clone(), String::new()),
    };
    TmuxNotification::WindowAdd {
        window_id: window_id.to_string(),
        window_layout: layout,
        window_visible_layout: visible_layout,
        window_raw_flags: raw_flags,
    }
}

fn cmd_new_session(
    ctx: &Ctx<'_>,
    name: Option<String>,
    env: Vec<(String, String)>,
    workspace: Option<Target<WorkspaceId>>,
) -> Outcome {
    let name = name.unwrap_or_else(|| "0".to_string());
    let env: std::collections::BTreeMap<String, String> = env.into_iter().collect();
    // Two-phase spawn (ARC-022): reserve ids under the lock, run the
    // fork/exec OFF it — a slow spawn must not stall every other client —
    // then re-lock to insert and wire. The in-flight session is invisible
    // until the insert lands. The workspace target resolves here (under
    // the lock, against the tree): an unknown one fails the command before
    // anything spawns.
    let (plan, factory) = {
        let mut guard = ctx.tree.lock();
        let workspace_id = match guard.resolve_new_session_workspace(workspace) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        };
        (
            guard.begin_session_at(workspace_id, &name, DEFAULT_COLS, DEFAULT_ROWS, &env),
            guard.factory(),
        )
    };
    let window_id = plan.window_id;
    let outcome = spawn_and_wire(ctx, &*factory, plan, None, None)
        .map(|session_id| (session_id, vec![window_id]));
    match outcome {
        Ok((session_id, window_ids)) => {
            let mut result = Outcome::ok(ctx, &session_id.to_string());
            for window in window_ids {
                result = result.notifying(window_add_notification(ctx.tree, window));
            }
            // The session set changed on the create side too — one cue for
            // both directions, so a client's "re-query list-sessions"
            // handling is written once.
            result = result.notifying(TmuxNotification::SessionsChanged);
            result.telling_issuer(TmuxNotification::SessionChanged {
                session_id: session_id.to_string(),
                name,
            })
        }
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

fn cmd_list_panes(ctx: &Ctx<'_>, window: Option<Target<WindowId>>) -> Outcome {
    // Wire contract: the bare form replies one line per pane, globally,
    // each just the pane id (`%N`) — the shape every existing client
    // parses. Geometry arrives via %layout-change pushes; there is no -F
    // (Phase 4 T4.E decision — push covers what the -F polling fallback
    // existed for).
    //
    // `list-panes -t <window>` (the attach card's criterion 2): one line
    // per pane of THAT window, in layout leaf order — the same
    // left-to-right/top-to-bottom order `LayoutTree::render` emits leaves
    // in, so `leaf` is the index a client maps tmux layout-string leaves
    // to pane ids by. Fixed positional shape (T4.E): `%N <leaf> <marker>`
    // where marker `*` is the window's active pane and `-` is every other
    // pane — the roster's fixed-prefix rule, and no -F.
    let guard = ctx.tree.lock();
    let Some(window_id) = window.map(|target| guard.resolve_window_target(target)) else {
        let body = guard
            .sessions()
            .iter()
            .filter_map(|s| guard.session(*s))
            .flat_map(|s| s.windows.clone())
            .filter_map(|w| guard.window(w))
            .flat_map(|w| w.panes())
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        return Outcome::ok(ctx, &body);
    };
    let window_id = match window_id {
        Ok(id) => id,
        Err(err) => return Outcome::err(ctx, &err.to_string()),
    };
    let Some(window) = guard.window(window_id) else {
        return Outcome::err(ctx, &MuxError::NoSuchWindow(window_id).to_string());
    };
    let body = window
        .panes()
        .into_iter()
        .enumerate()
        .map(|(leaf, pane)| {
            let marker = if pane == window.active { '*' } else { '-' };
            format!("{pane} {leaf} {marker}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    Outcome::ok(ctx, &body)
}

/// Format one roster row's entry (everything after the pane id): agent,
/// state, source, then zero or more whitespace-free `key=value` tokens —
/// `reason=<base64>` (blocked reason), `telemetry=<base64>`,
/// `host_telemetry=<base64>`. Every token after `source` is key=value,
/// so a consumer parses positions 1-4 then splits each remaining token
/// on its first `=` (ARC-060: a free-text reason column was ambiguous).
fn roster_row_entry(
    agent: &str,
    state: &str,
    source: &str,
    reason: Option<&str>,
    telemetry_b64: Option<&str>,
    host_telemetry_b64: Option<&str>,
) -> String {
    let mut entry = format!("{agent} {state} {source}");
    if let Some(reason) = reason {
        let encoded = base64::engine::general_purpose::STANDARD.encode(reason.as_bytes());
        entry.push_str(&format!(" reason={encoded}"));
    }
    if let Some(telemetry) = telemetry_b64 {
        entry.push_str(&format!(" telemetry={telemetry}"));
    }
    if let Some(host) = host_telemetry_b64 {
        entry.push_str(&format!(" host_telemetry={host}"));
    }
    entry
}

fn cmd_list_agents(ctx: &Ctx<'_>) -> Outcome {
    // Wire contract: list-agents is the roster — one line per pane a
    // hook has CLAIMED or a pattern has MATCHED, `%N <agent> <state>
    // <source>` with source `hook` or `scrape` (T5.4 + the scrape
    // tier's provenance rule: a consumer must tell a claim from a
    // guess), then zero or more whitespace-free `key=value` tokens:
    // `reason=<base64>` (the blocked reason, standard base64 of the
    // whitespace-collapsed message) and `telemetry=<base64>` /
    // `host_telemetry=<base64>` (fresh samples only). Every token
    // after `source` is key=value, so a consumer parses positions 1-4
    // then splits each remaining token on its first `=` — a free-text
    // reason column after `source` was ambiguous (ARC-060: a reason
    // reading `hook` or containing `telemetry=` defeated positional
    // parsers). Panes without state are absent outright: `unknown`
    // means no hook ever reported and no rule ever matched, never
    // "idle" (the Phase 5 ruling). Fixed shape, no -F — the T4.E
    // decision.
    let guard = ctx.tree.lock();
    let mut roster: Vec<(PaneId, String)> = guard
        .sessions()
        .iter()
        .filter_map(|s| guard.session(*s))
        .flat_map(|s| s.windows.clone())
        .filter_map(|w| guard.window(w))
        .flat_map(|w| w.panes())
        .filter_map(|p| {
            let pane = guard.pane(p)?;
            let state = pane.metadata().get("agent_state")?;
            let agent = pane.metadata().get("agent")?;
            let source = pane
                .metadata()
                .get("agent_state_source")
                .map(String::as_str)
                .unwrap_or("hook");
            // The blocked reason rides as one whitespace-free
            // `reason=<base64>` token (same standard engine as the
            // telemetry tokens). Absent message, no token.
            let reason = pane.metadata().get("agent_message").map(String::as_str);
            // Fresh telemetry rides as one final whitespace-free token
            // (base64 of the canonical JSON — string values carry
            // spaces). Stale or absent telemetry adds nothing, so a
            // pane without it keeps the exact pre-telemetry row. The
            // host probe's sibling token follows the same rule, aged
            // per field — and neither ever triggers a probe: the roster
            // reads only what the cadence thread already wrote.
            let telemetry = crate::mux::hooks::fresh_telemetry_b64(pane.telemetry.as_ref());
            let host =
                crate::mux::host_probe::fresh_host_telemetry_b64(pane.host_telemetry.as_ref());
            let entry = roster_row_entry(agent, state, source, reason, telemetry, host.as_deref());
            Some((p, entry))
        })
        .collect();
    roster.sort_by_key(|(pane, _)| *pane);
    let body = roster
        .iter()
        .map(|(pane, entry)| format!("{pane} {entry}"))
        .collect::<Vec<_>>()
        .join("\n");
    Outcome::ok(ctx, &body)
}

fn cmd_send_keys(ctx: &Ctx<'_>, pane: Target<PaneId>, keys: &SendKeysPayload) -> Outcome {
    let mut guard = ctx.tree.lock();
    let pane = match guard.resolve_pane_target(pane) {
        Ok(id) => id,
        Err(err) => return Outcome::err(ctx, &err.to_string()),
    };
    match guard.pane_mut(pane) {
        Some(target) => {
            // Navigation/function keys encode against the pane's live modes
            // (DECCKM, kitty flags); the read guard drops before the write.
            let terminal = target.terminal();
            let bytes = keys.encode(|| terminal.read());
            match target.write(&bytes) {
                Ok(()) => Outcome::ok(ctx, ""),
                Err(err) => Outcome::err(ctx, &err.to_string()),
            }
        }
        None => Outcome::err(ctx, &format!("no such pane: {pane}")),
    }
}

fn cmd_refresh_client(
    ctx: &Ctx<'_>,
    pane: Option<Target<PaneId>>,
    size: Option<(u16, u16)>,
    cell_pixels: Option<(u16, u16)>,
) -> Outcome {
    // `-p` is applied first and independently of `-C` and of the target:
    // the cell size is daemon-wide state every later re-fit reads
    // (sync_pane_sizes), so a report lands the pixels even when the grid
    // resize below fails, and a pixels-only report re-fits at the current
    // grid through the same sync path. This runs before pane resolution so
    // the attach handshake's target-less report takes effect.
    if let Some((cell_w, cell_h)) = cell_pixels {
        ctx.tree.lock().set_client_cell_pixels(cell_w, cell_h);
    }
    match pane {
        None => {
            // The target-less size-report form (the attach handshake's
            // shape): never a replay — replay is what the -t form exists
            // for, and resyncing a pane the client never named would push
            // it screen bytes of the wrong pane. `-C` records the report as
            // the connection's sizing contribution against the window it
            // displays (the bare form's documented stand-in: the newest
            // session's active window) and re-fits that window to the
            // smallest-attached-client minimum.
            match size {
                Some((cols, rows)) => {
                    let outcome: Result<(WindowId, Vec<WindowId>), String> = {
                        let mut guard = ctx.tree.lock();
                        // Newest = highest id (ids are monotonic). The
                        // map has no insertion order, so this is the
                        // deterministic spelling of bare new-window's
                        // documented stand-in.
                        let session = guard.sessions().into_iter().max();
                        match session {
                            Some(session) => {
                                let window = guard
                                    .session(session)
                                    .and_then(|s| s.windows.get(s.active).copied());
                                match window {
                                    Some(window_id) => {
                                        let resized = match ctx.client_id {
                                            Some(client_id) => guard
                                                .set_client_view(client_id, window_id, cols, rows),
                                            // No connection identity (embedder/test
                                            // dispatch): keep the legacy direct resize.
                                            None => {
                                                match guard.resize_window(window_id, cols, rows) {
                                                    Ok(()) => vec![window_id],
                                                    Err(err) => {
                                                        return Outcome::err(ctx, &err.to_string())
                                                    }
                                                }
                                            }
                                        };
                                        Ok((window_id, resized))
                                    }
                                    None => Err(format!("no such session: {session}")),
                                }
                            }
                            None => Err("no sessions exist to size".to_string()),
                        }
                    };
                    match outcome {
                        Ok((window_id, resized)) => {
                            let mut outcome = Outcome::ok(ctx, "").with_layout(window_id);
                            for extra in resized {
                                if extra != window_id {
                                    outcome = outcome.with_layout(extra);
                                }
                            }
                            outcome
                        }
                        Err(err) => Outcome::err(ctx, &err),
                    }
                }
                // Sizeless and targetless is meaningless (the parser
                // rejects it), but dispatch stays total: an empty ok.
                None => Outcome::ok(ctx, ""),
            }
        }
        Some(pane) => {
            let pane = {
                let guard = ctx.tree.lock();
                match guard.resolve_pane_target(pane) {
                    Ok(id) => id,
                    Err(err) => return Outcome::err(ctx, &err.to_string()),
                }
            };
            match size {
                // The window-size policy's input: a client's renderer reports
                // its grid against one of the window's panes; the report is
                // recorded as that connection's sizing contribution and the
                // window re-fits to the smallest-attached-client minimum.
                // Every pane terminal re-fits to the re-divided geometry —
                // followed by a %layout-change so clients re-render (the
                // seed path requires one even when the minimum did not move
                // the grid).
                Some((cols, rows)) => {
                    let outcome = {
                        let mut guard = ctx.tree.lock();
                        match guard.window_of_pane(pane) {
                            Some(window_id) => {
                                let resized = match ctx.client_id {
                                    Some(client_id) => {
                                        guard.set_client_view(client_id, window_id, cols, rows)
                                    }
                                    None => match guard.resize_window(window_id, cols, rows) {
                                        Ok(()) => vec![window_id],
                                        Err(err) => return Outcome::err(ctx, &err.to_string()),
                                    },
                                };
                                Ok((window_id, resized))
                            }
                            None => Err(MuxError::NoSuchPane(pane)),
                        }
                    };
                    match outcome {
                        Ok((window_id, resized)) => {
                            let mut outcome = Outcome::ok(ctx, "").with_layout(window_id);
                            for extra in resized {
                                if extra != window_id {
                                    outcome = outcome.with_layout(extra);
                                }
                            }
                            outcome
                        }
                        Err(err) => Outcome::err(ctx, &err.to_string()),
                    }
                }
                // Resync (D5.4): replay the pane's state as the screen-restore
                // encoder's byte stream so a reattached client's emulator
                // reproduces it exactly — the main screen's scrollback first (a
                // reattached pane can scroll back), alt-screen selection (a TUI
                // replays its TUI screen), then the styled content with
                // absolute row addressing (`\x1b[R;1H`; a `\n`-joined reply
                // staircases: LF preserves the column), attributes via SGR
                // (a plain reply loses every color), trailing background-styled
                // cells (plain text trims them), and finally the cursor
                // position/visibility/style and the input modes a full-screen
                // app's next %output deltas assume.
                None => {
                    let guard = ctx.tree.lock();
                    match guard.pane(pane) {
                        Some(target) => {
                            let screen = target.terminal().read().export_screen_restore_sequence();
                            Outcome::ok(ctx, &screen)
                        }
                        None => Outcome::err(ctx, &format!("no such pane: {pane}")),
                    }
                }
            }
        }
    }
}

fn cmd_kill_pane(ctx: &Ctx<'_>, pane: Target<PaneId>) -> Outcome {
    let pane = {
        let guard = ctx.tree.lock();
        match guard.resolve_pane_target(pane) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    // kill_pane resolves and returns the owning window: afterwards the
    // pane (and its window membership) is gone and cannot be looked up.
    // The lock guard is let-bound so it is gone before the successor
    // lookup below takes the tree again (parking_lot is not reentrant).
    let workspace_fingerprint = workspace_roster_fingerprint(ctx.tree);
    let outcome = ctx.tree.lock().kill_pane(pane);
    match outcome {
        Ok((window_id, removed_session)) => {
            // A window whose last pane was killed is REMOVED by
            // tree.kill_pane (an emptied session goes with it) — its
            // clients learn that through %window-close, the same
            // notification kill-window sends. A surviving window keeps
            // an active pane to name, and gets the layout push.
            // Let-bound (not an `if let` scrutinee) so the lock guard is
            // gone before the else branch's workspace re-lock — the
            // scrutinee temporary would otherwise live through the whole
            // if/else (parking_lot is not reentrant).
            let active = ctx.tree.lock().window(window_id).map(|w| w.active);
            if let Some(active) = active {
                Outcome::ok(ctx, "").with_layout(window_id).notifying(
                    TmuxNotification::WindowPaneChanged {
                        window_id: window_id.to_string(),
                        pane_id: active.to_string(),
                    },
                )
            } else {
                let mut outcome = Outcome::ok(ctx, "").notifying(TmuxNotification::WindowClose {
                    window_id: window_id.to_string(),
                });
                if removed_session.is_some() {
                    // The window's closure emptied the session: the set of
                    // sessions changed, and tmux says so argument-less.
                    outcome = outcome.notifying(TmuxNotification::SessionsChanged);
                    if workspace_roster_changed(ctx.tree, &workspace_fingerprint) {
                        outcome = outcome.notifying(TmuxNotification::WorkspacesChanged);
                    }
                }
                outcome
            }
        }
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

/// Resolve a `split-window -c` / `new-window -c` start directory: an
/// existing directory is used as-is; one that does not exist degrades to
/// home — the command's success may not depend on a directory this
/// process does not control (the restore path's rule for a gone persisted
/// cwd) — and the returned note names both so the pane says where it
/// landed instead of silently starting elsewhere.
fn resolve_start_dir(start_dir: Option<&str>) -> (Option<PathBuf>, Option<String>) {
    let Some(raw) = start_dir else {
        return (None, None);
    };
    let dir = Path::new(raw);
    if dir.is_dir() {
        return (Some(dir.to_path_buf()), None);
    }
    let home = dirs::home_dir().unwrap_or_default();
    let note = home.is_dir().then(|| {
        format!(
            "\r\npar-mux: {raw} is gone; pane started in {}\r\n",
            home.display()
        )
    });
    (home.is_dir().then_some(home), note)
}

fn cmd_split_window(
    ctx: &Ctx<'_>,
    target: AnyTarget,
    direction: SplitDirection,
    percent: u32,
    before: bool,
    start_dir: Option<&str>,
) -> Outcome {
    let (cwd, note) = resolve_start_dir(start_dir);
    // Two-phase spawn (ARC-022): resolve + reserve under the lock, fork/exec
    // off it, re-lock to insert, wire, and shape the layout. A target killed
    // while the pane spawned fails the insert (the pane is killed tree-side).
    let (plan, factory) = {
        let mut guard = ctx.tree.lock();
        // %N splits that pane, @N its window's active pane, $N the
        // session's active window's active pane, a name the pane-title match.
        let pane = match guard.resolve_split_target(target) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        };
        match guard.begin_split(
            pane,
            direction,
            percent as f32 / 100.0,
            cwd.as_deref(),
            before,
        ) {
            Ok(plan) => (plan, guard.factory()),
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    match spawn_and_wire(ctx, &*factory, plan, None, note.as_deref()) {
        Ok((new_pane, window_id)) => {
            // split-window focuses the new pane (tmux semantics).
            Outcome::ok(ctx, &new_pane.to_string())
                .with_layout(window_id)
                .notifying(TmuxNotification::WindowPaneChanged {
                    window_id: window_id.to_string(),
                    pane_id: new_pane.to_string(),
                })
        }
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

fn cmd_select_pane(ctx: &Ctx<'_>, pane: Target<PaneId>, title: Option<String>) -> Outcome {
    let mut guard = ctx.tree.lock();
    let pane = match guard.resolve_pane_target(pane) {
        Ok(id) => id,
        Err(err) => return Outcome::err(ctx, &err.to_string()),
    };
    // The title lands first under the same lock: a failed select reports
    // the pane error and nothing else changed.
    let mut title_notification = None;
    if let Some(title) = title {
        let pane_mut = match guard.pane_mut(pane) {
            Some(p) => p,
            None => return Outcome::err(ctx, &MuxError::NoSuchPane(pane).to_string()),
        };
        if pane_mut.set_user_title(&title) {
            // The notification carries the new USER title (empty = cleared);
            // the display title remains user-or-OSC per the precedence rule.
            title_notification = Some(TmuxNotification::PaneTitleChanged {
                pane_id: pane.to_string(),
                title,
            });
        }
    }
    match guard.select_pane(pane) {
        Ok(window_id) => {
            let mut outcome = Outcome::ok(ctx, "").with_layout(window_id).notifying(
                TmuxNotification::WindowPaneChanged {
                    window_id: window_id.to_string(),
                    pane_id: pane.to_string(),
                },
            );
            if let Some(notification) = title_notification {
                outcome = outcome.notifying(notification);
            }
            outcome
        }
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

fn cmd_pane_title(ctx: &Ctx<'_>, pane: Target<PaneId>) -> Outcome {
    // Wire contract: the reply body is exactly one line — the effective
    // title (user `-T` title when set, else the pane terminal's current
    // OSC 0/2 title). An empty body means neither is set.
    let guard = ctx.tree.lock();
    let pane = match guard.resolve_pane_target(pane) {
        Ok(id) => id,
        Err(err) => return Outcome::err(ctx, &err.to_string()),
    };
    match guard.pane(pane) {
        Some(pane) => Outcome::ok(ctx, &pane.effective_title()),
        None => Outcome::err(ctx, &MuxError::NoSuchPane(pane).to_string()),
    }
}

fn cmd_pane_info(ctx: &Ctx<'_>, pane: Target<PaneId>) -> Outcome {
    // Wire contract: one line, `%N @W COLSxROWS [cmd=<base64>]
    // [exited=<code|?>]` — the pane's window, its terminal's current grid
    // size, and, when knowable, the pane's foreground command name
    // (deepest descendant of its child process) for close-confirmation
    // prompts. The cmd token may be absent (Windows table, unreadable
    // argv, a name with control characters or over the cap, or a reaped
    // child). `exited=` rides only for a held-dead pane — `?` when the exit
    // code was unreadable — so a client that connects after `%pane-exited`
    // can still render the remain-on-exit state (ARC-095). Both are
    // whitespace-free `key=value` tail tokens, so older clients keep
    // parsing the fixed prefix.
    let (pane, window, cols, rows, child_pid, dead, exit_code) = {
        let guard = ctx.tree.lock();
        let pane = match guard.resolve_pane_target(pane) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        };
        let (Some(target), Some(window)) = (guard.pane(pane), guard.window_of_pane(pane)) else {
            return Outcome::err(ctx, &MuxError::NoSuchPane(pane).to_string());
        };
        let (cols, rows) = target.terminal().read().size();
        (
            pane,
            window,
            cols,
            rows,
            target.child_pid(),
            target.dead(),
            target.exit_code(),
        )
    };
    // The process table snapshot reads the whole OS process list; taking
    // it outside the tree lock keeps a slow read from stalling clients.
    let mut line = format!("{pane} {window} {cols}x{rows}");
    if let Some(name) = child_pid
        .and_then(|pid| ProcessTable::snapshot().and_then(|table| table.foreground_command(pid)))
    {
        line.push_str(" cmd=");
        line.push_str(&base64::engine::general_purpose::STANDARD.encode(name));
    }
    if dead {
        line.push_str(" exited=");
        match exit_code {
            Some(code) => line.push_str(&code.to_string()),
            None => line.push('?'),
        }
    }
    Outcome::ok(ctx, &line)
}

/// ENH-042: queue the held panes' `%pane-exited` lines on the issuing
/// client's own channel — ahead of this command's (empty) reply, the same
/// framing registration replay uses. Nothing is broadcast.
fn cmd_pane_exited_replay(ctx: &Ctx<'_>, issuer: Option<&SyncSender<String>>) -> Outcome {
    let lines = replay_pane_exited_lines(&ctx.tree.lock());
    if let Some(tx) = issuer {
        for line in lines {
            if tx.send(line).is_err() {
                break;
            }
        }
    }
    Outcome::ok(ctx, "")
}

fn cmd_clear_history(ctx: &Ctx<'_>, pane: Target<PaneId>) -> Outcome {
    let guard = ctx.tree.lock();
    let pane = match guard.resolve_pane_target(pane) {
        Ok(id) => id,
        Err(err) => return Outcome::err(ctx, &err.to_string()),
    };
    match guard.pane(pane) {
        Some(target) => {
            // The clear rides the emulator's own ED 3 path, so graphics
            // teardown and the ScreenCleared event stay in sync with the
            // core instead of a parallel grid-only reimplementation.
            target.with_terminal_mut(|term| term.process(b"\x1b[3J"));
            Outcome::ok(ctx, "")
        }
        None => Outcome::err(ctx, &MuxError::NoSuchPane(pane).to_string()),
    }
}

fn cmd_resize_pane(ctx: &Ctx<'_>, pane: Target<PaneId>, adjustment: ResizeAdjustment) -> Outcome {
    let outcome = {
        let mut guard = ctx.tree.lock();
        let pane = match guard.resolve_pane_target(pane) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        };
        match adjustment {
            ResizeAdjustment::Relative { direction, cells } => {
                guard.resize_pane(pane, direction, cells)
            }
            ResizeAdjustment::Absolute { cols, rows } => {
                guard.resize_pane_absolute(pane, cols, rows)
            }
            ResizeAdjustment::Zoom => guard.zoom_pane(pane),
        }
    };
    match outcome {
        Ok(window_id) => Outcome::ok(ctx, "").with_layout(window_id),
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

fn cmd_swap_panes(ctx: &Ctx<'_>, target: Target<PaneId>, source: Target<PaneId>) -> Outcome {
    let (target, source) = {
        let guard = ctx.tree.lock();
        match (
            guard.resolve_pane_target(target),
            guard.resolve_pane_target(source),
        ) {
            (Ok(target), Ok(source)) => (target, source),
            (Err(err), _) | (_, Err(err)) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    match ctx.tree.lock().swap_panes(target, source) {
        Ok(window_id) => Outcome::ok(ctx, "").with_layout(window_id),
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

fn cmd_break_pane(ctx: &Ctx<'_>, source: Target<PaneId>, name: Option<String>) -> Outcome {
    let source = {
        let guard = ctx.tree.lock();
        match guard.resolve_pane_target(source) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    let name = name.unwrap_or_else(|| "0".to_string());
    // Bound to a local so the scrutinee's tree-lock temporary drops here:
    // the match arm below re-locks to read the new window's layout triple.
    let broken = ctx.tree.lock().break_pane(source, &name);
    match broken {
        Ok((window_id, source_window, source_closed)) => {
            // The new window is announced like new-window's; the source's
            // frame only exists while the source does.
            let mut outcome = Outcome::ok(ctx, &window_id.to_string())
                .notifying(window_add_notification(ctx.tree, window_id));
            if source_closed {
                outcome = outcome.notifying(TmuxNotification::WindowClose {
                    window_id: source_window.to_string(),
                });
            } else {
                outcome = outcome.with_layout(source_window);
            }
            outcome
        }
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

fn cmd_join_pane(
    ctx: &Ctx<'_>,
    source: Target<PaneId>,
    target: Target<PaneId>,
    direction: SplitDirection,
    percent: u32,
) -> Outcome {
    let (source, target) = {
        let guard = ctx.tree.lock();
        match (
            guard.resolve_pane_target(source),
            guard.resolve_pane_target(target),
        ) {
            (Ok(source), Ok(target)) => (source, target),
            (Err(err), _) | (_, Err(err)) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    let workspace_fingerprint = workspace_roster_fingerprint(ctx.tree);
    // Let-bound so the lock guard drops before the arm's re-lock (the
    // scrutinee temporary would otherwise live through the match —
    // parking_lot is not reentrant).
    let joined = ctx
        .tree
        .lock()
        .join_pane(source, target, direction, percent as f32 / 100.0);
    match joined {
        Ok((target_window, source_window, source_closed, removed_session)) => {
            // Both windows' layouts changed — the destination grew a pane
            // and the source lost one (or closed outright).
            let mut outcome = Outcome::ok(ctx, "").with_layout(target_window);
            if source_closed {
                outcome = outcome.notifying(TmuxNotification::WindowClose {
                    window_id: source_window.to_string(),
                });
            } else {
                outcome = outcome.with_layout(source_window);
            }
            if removed_session.is_some() {
                // The same argument-less cue kill-pane's cascade sends.
                outcome = outcome.notifying(TmuxNotification::SessionsChanged);
                if workspace_roster_changed(ctx.tree, &workspace_fingerprint) {
                    outcome = outcome.notifying(TmuxNotification::WorkspacesChanged);
                }
            }
            outcome
        }
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

/// `respawn-pane -t %N [-k] [-c dir] [command]`: restart the pane's
/// process in place — same pane id, window, and layout; a fresh
/// terminal. Two-phase (ARC-022): resolve and refuse a live pane without
/// `-k` under the lock, spawn off it, re-lock to swap. `%pane-respawned`
/// is the cue clients clear their exited-state chrome on.
fn cmd_respawn_pane(
    ctx: &Ctx<'_>,
    pane: Target<PaneId>,
    kill: bool,
    start_dir: Option<&str>,
    command: Option<String>,
) -> Outcome {
    let (cwd, note) = resolve_start_dir(start_dir);
    // Phase 1: resolve, refuse a live pane, snapshot the restart plan.
    let (plan, factory) = {
        let mut guard = ctx.tree.lock();
        let pane = match guard.resolve_pane_target(pane) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        };
        match guard.begin_respawn(pane, kill, command, cwd.as_deref()) {
            Ok(plan) => {
                // `-k` of a live pane: the old process keeps running and
                // emitting under this id until phase 3 kills it, so stop
                // forwarding it now — its output must not interleave with
                // the replacement's (ARC-089). A failed respawn re-attaches.
                if let Some(old) = guard.pane_mut(plan.pane_id) {
                    old.detach_output();
                }
                (plan, guard.factory())
            }
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    let pane_id = plan.pane_id;
    let command = plan.command.clone();
    // Phases 2 and 3: spawn the replacement off the tree lock with its sink
    // live, then swap it in.
    let outcome = spawn_and_wire(ctx, &*factory, plan, command.as_deref(), note.as_deref());
    if outcome.is_err() {
        // The spawn or the swap failed: the old pane (if it still exists)
        // is the live pane again, so it resumes forwarding as before the
        // command.
        if let Some(pane) = ctx.tree.lock().pane_mut(pane_id) {
            pane.on_output_sink(OutputSink(Arc::new(pane_output_sink(ctx.clients, pane_id))));
        }
    }
    match outcome {
        Ok(pane_id) => Outcome::ok(ctx, "").notifying(TmuxNotification::PaneRespawned {
            pane_id: pane_id.to_string(),
        }),
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

fn cmd_move_window(ctx: &Ctx<'_>, source: Target<WindowId>, index: usize) -> Outcome {
    let source = {
        let guard = ctx.tree.lock();
        match guard.resolve_window_target(source) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    match ctx.tree.lock().move_window(source, index) {
        // No layout changed — the reorder cue is the argument-less
        // sessions-changed; clients re-query list-windows.
        Ok(()) => Outcome::ok(ctx, "").notifying(TmuxNotification::SessionsChanged),
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

fn cmd_swap_windows(ctx: &Ctx<'_>, source: Target<WindowId>, target: Target<WindowId>) -> Outcome {
    let (source, target) = {
        let guard = ctx.tree.lock();
        match (
            guard.resolve_window_target(source),
            guard.resolve_window_target(target),
        ) {
            (Ok(source), Ok(target)) => (source, target),
            (Err(err), _) | (_, Err(err)) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    match ctx.tree.lock().swap_windows(source, target) {
        Ok(()) => Outcome::ok(ctx, "").notifying(TmuxNotification::SessionsChanged),
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

fn cmd_new_window(
    ctx: &Ctx<'_>,
    target: Option<AnyTarget>,
    name: Option<String>,
    start_dir: Option<&str>,
) -> Outcome {
    let name = name.unwrap_or_else(|| "0".to_string());
    let (cwd, note) = resolve_start_dir(start_dir);
    // Two-phase spawn (ARC-022): resolve + reserve under the lock, fork/exec
    // off it, re-lock to insert and wire. A session killed while the pane
    // spawned fails the insert (the pane is killed tree-side).
    let (plan, factory) = {
        let mut guard = ctx.tree.lock();
        // Bare `new-window` targets the most-recently-created
        // session — ids are monotonic and the registry keeps
        // insertion order, so the last entry is the newest.
        let Some(target) =
            target.or_else(|| guard.sessions().last().copied().map(AnyTarget::Session))
        else {
            return Outcome::err(ctx, "no sessions exist");
        };
        // $N (and names) append; @N/%N insert right after that window.
        let (session, insert_after) = match guard.resolve_new_window_target(target) {
            Ok(resolved) => resolved,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        };
        match guard.begin_window(
            session,
            &name,
            DEFAULT_COLS,
            DEFAULT_ROWS,
            cwd.as_deref(),
            insert_after,
        ) {
            Ok(plan) => (plan, guard.factory()),
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    match spawn_and_wire(ctx, &*factory, plan, None, note.as_deref()) {
        Ok(window_id) => Outcome::ok(ctx, &window_id.to_string())
            .notifying(window_add_notification(ctx.tree, window_id)),
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

fn cmd_select_window(ctx: &Ctx<'_>, window: Target<WindowId>) -> Outcome {
    let (window, active, session_id, selection_moved, resized) = {
        let mut guard = ctx.tree.lock();
        let window = match guard.resolve_window_target(window) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        };
        let session_id = guard.session_of_window(window);
        let was_active = session_id
            .and_then(|s| guard.session(s))
            .and_then(|s| s.windows.get(s.active).copied());
        if let Err(err) = guard.select_window(window) {
            return Outcome::err(ctx, &err.to_string());
        }
        // Shared-selection sync: when the move touches the displayed
        // session, every render client attached to it follows to the new
        // active window, and `%session-window-changed` tells them so. A
        // background session's pointer moves silently — clients displaying
        // it are not yanked.
        let displayed = guard.active_session();
        let selection_moved = was_active != Some(window) && displayed == session_id;
        let resized = if selection_moved {
            let Some(session_id) = session_id else {
                return Outcome::err(ctx, "window has no session");
            };
            guard.follow_session_window(session_id, window)
        } else {
            Vec::new()
        };
        (
            window,
            guard.window(window).map(|w| w.active),
            session_id,
            selection_moved,
            resized,
        )
    };
    let mut result = Outcome::ok(ctx, "");
    if selection_moved {
        if let Some(session_id) = session_id {
            result = result.notifying(TmuxNotification::SessionWindowChanged {
                session_id: session_id.to_string(),
                window_id: window.to_string(),
            });
        }
    }
    for window_id in resized {
        result = result.with_layout(window_id);
    }
    if let Some(pane) = active {
        result = result.notifying(TmuxNotification::WindowPaneChanged {
            window_id: window.to_string(),
            pane_id: pane.to_string(),
        });
    }
    result
}

fn cmd_kill_window(ctx: &Ctx<'_>, window: Target<WindowId>) -> Outcome {
    let killed = match kill_target(
        ctx,
        window,
        MuxTree::resolve_window_target,
        MuxTree::kill_window,
    ) {
        Ok(killed) => killed,
        Err(outcome) => return outcome,
    };
    let mut outcome = Outcome::ok(ctx, "").notifying(TmuxNotification::WindowClose {
        window_id: killed.id.to_string(),
    });
    if killed.removed.is_some() {
        // The cascade reached the session — same argument-less
        // cue kill-pane's cascade sends, so one handler covers both.
        outcome = outcome.notifying(TmuxNotification::SessionsChanged);
        if killed.workspaces_changed {
            outcome = outcome.notifying(TmuxNotification::WorkspacesChanged);
        }
    }
    outcome
}

fn cmd_rename_window(ctx: &Ctx<'_>, window: Target<WindowId>, name: String) -> Outcome {
    let window = {
        let guard = ctx.tree.lock();
        match guard.resolve_window_target(window) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    match ctx.tree.lock().rename_window(window, &name) {
        Ok(()) => Outcome::ok(ctx, "").notifying(TmuxNotification::WindowRenamed {
            window_id: window.to_string(),
            name,
        }),
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

fn cmd_list_windows(ctx: &Ctx<'_>, session: Option<Target<SessionId>>) -> Outcome {
    // Wire contract: the bare form replies one line per window, globally,
    // as `@N: name` — the shape every existing client parses.
    //
    // `list-windows -t <session>` (the attach card's criterion 1): one
    // line per window of THAT session, in the session's window order (the
    // order `%sessions-changed` asks clients to re-query), as a fixed
    // positional shape (T4.E): `@N <marker> <name>` — marker `*` for the
    // session's active window, `-` otherwise; the name is the line
    // remainder so a spaced name survives (same rule as the roster's
    // entry and `@N: name`'s own split-on-first-colon). No -F.
    let guard = ctx.tree.lock();
    let Some(session_id) = session.map(|target| guard.resolve_session_target(target)) else {
        let body = guard
            .sessions()
            .iter()
            .filter_map(|s| guard.session(*s))
            .flat_map(|s| s.windows.clone())
            .filter_map(|w| guard.window(w))
            .map(|w| format!("{}: {}", w.id, w.name))
            .collect::<Vec<_>>()
            .join("\n");
        return Outcome::ok(ctx, &body);
    };
    let session_id = match session_id {
        Ok(id) => id,
        Err(err) => return Outcome::err(ctx, &err.to_string()),
    };
    let Some(session) = guard.session(session_id) else {
        return Outcome::err(ctx, &MuxError::NoSuchSession(session_id).to_string());
    };
    let body = session
        .windows
        .iter()
        .enumerate()
        .map(|(index, window_id)| {
            let marker = if index == session.active { '*' } else { '-' };
            let name = guard
                .window(*window_id)
                .map(|w| w.name.as_str())
                .unwrap_or_default();
            format!("{window_id} {marker} {name}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    Outcome::ok(ctx, &body)
}

fn cmd_kill_server(ctx: &Ctx<'_>) -> Outcome {
    // The same path SIGTERM takes: raise the flag and let the accept loop
    // notice on its next tick, which runs the final state save and sends
    // `%exit` to every client. The reply goes out before the loop exits.
    match ctx.shutdown {
        Some(flag) => {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
            Outcome::ok(ctx, "")
        }
        None => Outcome::err(ctx, "kill-server: no running server to stop"),
    }
}

fn cmd_rename_session(ctx: &Ctx<'_>, session: Target<SessionId>, name: String) -> Outcome {
    let session = {
        let guard = ctx.tree.lock();
        match guard.resolve_session_target(session) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    match ctx.tree.lock().rename_session(session, &name) {
        Ok(()) => Outcome::ok(ctx, "").notifying(TmuxNotification::SessionRenamed {
            session_id: session.to_string(),
            name,
        }),
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

fn cmd_kill_session(ctx: &Ctx<'_>, session: Target<SessionId>) -> Outcome {
    let killed = match kill_target(
        ctx,
        session,
        MuxTree::resolve_session_target,
        MuxTree::kill_session,
    ) {
        Ok(killed) => killed,
        Err(outcome) => return outcome,
    };
    // The same line order kill-window's cascade produces: one
    // %window-close per killed window, then the session-set cue —
    // plus the workspace cue when the session's death emptied its
    // workspace away.
    let mut outcome = notify_window_closes(Outcome::ok(ctx, ""), &killed.removed)
        .notifying(TmuxNotification::SessionsChanged);
    if killed.workspaces_changed {
        outcome = outcome.notifying(TmuxNotification::WorkspacesChanged);
    }
    outcome
}

fn cmd_list_sessions(ctx: &Ctx<'_>, workspace: Option<Target<WorkspaceId>>) -> Outcome {
    // Wire contract: bare `list-sessions` lists EVERY workspace's sessions,
    // one line each, extended from the old `$N: name` shape with a
    // workspace prefix: `+W: wname: $N: name`. A consumer finds the
    // session id at the LAST ` $<digits>:` marker; the workspace fields
    // precede it. `list-sessions -t <workspace>` (id or name) restricts
    // the listing to that one workspace, same line shape. Sessions are
    // listed in workspace order, sessions in workspace-list order.
    let guard = ctx.tree.lock();
    let workspace_id = match workspace {
        Some(target) => match guard.resolve_workspace_target(target) {
            Ok(id) if guard.workspace(id).is_some() => Some(id),
            Ok(id) => return Outcome::err(ctx, &MuxError::NoSuchWorkspace(id).to_string()),
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        },
        None => None,
    };
    let mut lines: Vec<String> = Vec::new();
    let mut workspaces = guard.workspaces();
    workspaces.sort();
    for ws_id in workspaces {
        let Some(ws) = guard.workspace(ws_id) else {
            continue;
        };
        if let Some(filter) = workspace_id {
            if filter != ws_id {
                continue;
            }
        }
        for session_id in &ws.sessions {
            let Some(session) = guard.session(*session_id) else {
                continue;
            };
            lines.push(format!(
                "{}: {}: {}: {}",
                ws.id, ws.name, session.id, session.name
            ));
        }
    }
    Outcome::ok(ctx, &lines.join("\n"))
}

/// `new-workspace [-n name]`: create the workspace, select it, and reply
/// with its `+N` id. `%workspaces-changed` is the roster cue — one
/// argument-less broadcast covering add, close, select, and rename, the
/// same re-query convention `%sessions-changed` set (clients re-run
/// `list-workspaces` rather than parse a diff).
fn cmd_new_workspace(ctx: &Ctx<'_>, name: Option<String>) -> Outcome {
    let name = name.unwrap_or_else(|| "main".to_string());
    // A workspace with no sessions cannot be landed on — the panel's
    // ` new ` chip must create a USABLE one: the workspace plus its
    // first session/window spawning the default shell, so the
    // select+land the client runs after the create has something to
    // land on (the manual-pass report: a created workspace highlighted
    // in the side panel while the view stayed on the old one's tabs).
    let (id, plan, factory) = {
        let mut guard = ctx.tree.lock();
        let id = guard.new_workspace(&name);
        (
            id,
            guard
                .begin_session_at(id, &name, DEFAULT_COLS, DEFAULT_ROWS, &Default::default())
                .with_window_name("1"),
            guard.factory(),
        )
    };
    let window_id = plan.window_id;
    let outcome = spawn_and_wire(ctx, &*factory, plan, None, None)
        .map(|session_id| (session_id, vec![window_id]));
    match outcome {
        Ok((_session_id, window_ids)) => {
            // The reply stays the WORKSPACE id (the wire contract the
            // client's landing parses); the session/window cues ride the
            // notifications.
            let mut result = Outcome::ok(ctx, &id.to_string())
                .notifying(TmuxNotification::WorkspacesChanged)
                .notifying(TmuxNotification::SessionsChanged);
            for window in window_ids {
                result = result.notifying(window_add_notification(ctx.tree, window));
            }
            result
        }
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

fn cmd_list_workspaces(ctx: &Ctx<'_>) -> Outcome {
    // Wire contract: one `+N: name` line per workspace, in id order; the
    // daemon's active workspace's line ends with a space and `active`.
    let guard = ctx.tree.lock();
    let active = guard.active_workspace();
    let mut ids = guard.workspaces();
    ids.sort();
    let body = ids
        .iter()
        .filter_map(|id| guard.workspace(*id))
        .map(|ws| {
            if Some(ws.id) == active {
                format!("{}: {} active", ws.id, ws.name)
            } else {
                format!("{}: {}", ws.id, ws.name)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    Outcome::ok(ctx, &body)
}

fn cmd_select_workspace(ctx: &Ctx<'_>, workspace: Target<WorkspaceId>) -> Outcome {
    let (workspace, session, window, moved, resized) = {
        let mut guard = ctx.tree.lock();
        let workspace = match guard.resolve_workspace_target(workspace) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        };
        let previous = guard.active_workspace();
        let was_session = guard.active_session();
        let was_window = was_session
            .and_then(|s| guard.session(s))
            .and_then(|s| s.windows.get(s.active).copied());
        if let Err(err) = guard.select_workspace(workspace) {
            return Outcome::err(ctx, &err.to_string());
        }
        let session = guard.active_session();
        let window = session
            .and_then(|s| guard.session(s))
            .and_then(|s| s.windows.get(s.active).copied());
        // Shared-selection sync: when the switch moves the displayed view,
        // render clients attached to the left workspace's sessions follow
        // to the new displayed window and `%client-session-changed` tells
        // them so — the `client` field carries the workspace the display
        // moved to, since par-mux has no per-client names. Re-selecting
        // the active workspace (same displayed session and window) is a
        // no-op.
        let moved = was_window != window;
        let resized = match (moved, window) {
            (true, Some(window)) => match previous {
                Some(previous) => guard.follow_workspace_views(previous, window),
                None => guard.follow_workspace_views(workspace, window),
            },
            _ => Vec::new(),
        };
        (workspace, session, window, moved, resized)
    };
    let mut result = Outcome::ok(ctx, "").notifying(TmuxNotification::WorkspacesChanged);
    if moved {
        if let (Some(session), Some(_)) = (session, window) {
            result = result.notifying(TmuxNotification::ClientSessionChanged {
                client: workspace.to_string(),
                session_id: session.to_string(),
                name: ctx
                    .tree
                    .lock()
                    .session(session)
                    .map(|s| s.name.clone())
                    .unwrap_or_default(),
            });
        }
    }
    for window_id in resized {
        result = result.with_layout(window_id);
    }
    result
}

fn cmd_rename_workspace(ctx: &Ctx<'_>, workspace: Target<WorkspaceId>, name: String) -> Outcome {
    let workspace = {
        let guard = ctx.tree.lock();
        match guard.resolve_workspace_target(workspace) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    match ctx.tree.lock().rename_workspace(workspace, &name) {
        // Renames change what list-workspaces renders; same cue.
        Ok(()) => Outcome::ok(ctx, "").notifying(TmuxNotification::WorkspacesChanged),
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

fn cmd_kill_workspace(ctx: &Ctx<'_>, workspace: Target<WorkspaceId>) -> Outcome {
    let killed = match kill_target(
        ctx,
        workspace,
        MuxTree::resolve_workspace_target,
        MuxTree::kill_workspace,
    ) {
        Ok(killed) => killed,
        Err(outcome) => return outcome,
    };
    // kill-session's line order, workspace-flavored: one
    // %window-close per killed window, then the session-set cue,
    // then the workspace-roster cue — unconditional, since the killed
    // workspace itself left the roster.
    notify_window_closes(Outcome::ok(ctx, ""), &killed.removed)
        .notifying(TmuxNotification::SessionsChanged)
        .notifying(TmuxNotification::WorkspacesChanged)
}

/// What [`kill_target`] reports for a kill that landed.
struct Killed<I, T> {
    /// The resolved id of the killed target.
    id: I,
    /// The tree's kill result: the cascaded-away session for kill-window,
    /// the killed windows for kill-session and kill-workspace.
    removed: T,
    /// Whether the workspace roster (set or selection) differs from its
    /// pre-kill snapshot.
    workspaces_changed: bool,
}

/// The resolve-then-kill step shared by the kill-window, kill-session, and
/// kill-workspace handlers: resolve the target under its own lock (released
/// before the kill), snapshot the workspace roster, run the tree's kill, and
/// report the roster delta. A resolve failure, then a kill failure, becomes
/// the command's error reply; each handler keeps only its notifications.
fn kill_target<I: Copy, T>(
    ctx: &Ctx<'_>,
    target: Target<I>,
    resolve: impl FnOnce(&MuxTree, Target<I>) -> Result<I, MuxError>,
    kill: impl FnOnce(&mut MuxTree, I) -> Result<T, MuxError>,
) -> Result<Killed<I, T>, Outcome> {
    let resolved = resolve(&ctx.tree.lock(), target);
    let id = resolved.map_err(|err| Outcome::err(ctx, &err.to_string()))?;
    let fingerprint = workspace_roster_fingerprint(ctx.tree);
    let killed = kill(&mut ctx.tree.lock(), id);
    let removed = killed.map_err(|err| Outcome::err(ctx, &err.to_string()))?;
    Ok(Killed {
        id,
        removed,
        workspaces_changed: workspace_roster_changed(ctx.tree, &fingerprint),
    })
}

/// Queue one `%window-close` per killed window, in kill order.
fn notify_window_closes(mut outcome: Outcome, windows: &[WindowId]) -> Outcome {
    for window in windows {
        outcome = outcome.notifying(TmuxNotification::WindowClose {
            window_id: window.to_string(),
        });
    }
    outcome
}

/// Snapshot the workspace roster for [`workspace_roster_changed`]: the
/// sorted id set plus the active pointer, enough to tell "the workspace
/// set (or its selection) changed" from "it did not".
pub(crate) fn workspace_roster_fingerprint(
    tree: &Arc<Mutex<MuxTree>>,
) -> (Vec<WorkspaceId>, Option<WorkspaceId>) {
    let guard = tree.lock();
    let mut ids = guard.workspaces();
    ids.sort();
    (ids, guard.active_workspace())
}

/// True when the workspace roster's fingerprint changed across a kill
/// cascade — a session's death can empty and remove its workspace, and
/// that removal is the workspace-roster cue clients re-query on.
pub(crate) fn workspace_roster_changed(
    tree: &Arc<Mutex<MuxTree>>,
    before: &(Vec<WorkspaceId>, Option<WorkspaceId>),
) -> bool {
    &workspace_roster_fingerprint(tree) != before
}

fn cmd_capture_pane(
    ctx: &Ctx<'_>,
    pane: Target<PaneId>,
    start_line: Option<i64>,
    end_line: Option<i64>,
    escape: bool,
) -> Outcome {
    let guard = ctx.tree.lock();
    let pane = match guard.resolve_pane_target(pane) {
        Ok(id) => id,
        Err(err) => return Outcome::err(ctx, &err.to_string()),
    };
    match guard.pane(pane) {
        Some(target) => {
            let terminal = target.terminal();
            let term = terminal.read();
            let body = match (start_line, end_line) {
                // Decision 2 stands: no new Terminal API — the default
                // capture reads the pane's visible screen (the active
                // grid, so an alt-screen TUI captures its TUI screen).
                (None, None) => {
                    if escape {
                        term.export_visible_screen_styled_lines()
                    } else {
                        term.content()
                    }
                }
                (start, end) => {
                    // export_scrollback only takes a tail count, so
                    // the tmux -S/-E range trim happens here on the
                    // composed buffer, not in Terminal.
                    let format = if escape {
                        crate::terminal::ExportFormat::Ansi
                    } else {
                        crate::terminal::ExportFormat::Plain
                    };
                    let scrollback = term.export_scrollback(format, None);
                    let screen = if escape {
                        term.export_visible_screen_styled_lines()
                    } else {
                        term.content()
                    };
                    capture_range(&scrollback, &screen, start, end)
                }
            };
            Outcome::ok(ctx, &body)
        }
        None => Outcome::err(ctx, &format!("no such pane: {pane}")),
    }
}

fn cmd_set_buffer(ctx: &Ctx<'_>, content: String) -> Outcome {
    ctx.tree.lock().set_buffer(DEFAULT_BUFFER, content);
    Outcome::ok(ctx, "")
}

fn cmd_set_client_colors(
    ctx: &Ctx<'_>,
    fg: Option<(u8, u8, u8)>,
    bg: Option<(u8, u8, u8)>,
) -> Outcome {
    let to_color = |(r, g, b)| crate::color::Color::Rgb(r, g, b);
    ctx.tree
        .lock()
        .set_client_colors(fg.map(to_color), bg.map(to_color));
    Outcome::ok(ctx, "")
}

fn cmd_set_environment(
    ctx: &Ctx<'_>,
    session: Target<SessionId>,
    name: &str,
    value: Option<&str>,
) -> Outcome {
    let session = {
        let guard = ctx.tree.lock();
        match guard.resolve_session_target(session) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    match ctx.tree.lock().set_session_env(session, name, value) {
        Ok(()) => Outcome::ok(ctx, ""),
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

fn cmd_show_buffer(ctx: &Ctx<'_>) -> Outcome {
    let guard = ctx.tree.lock();
    match guard.get_buffer(DEFAULT_BUFFER) {
        Some(content) => Outcome::ok(ctx, content),
        None => Outcome::err(ctx, "no buffers"),
    }
}

fn cmd_paste_buffer(ctx: &Ctx<'_>, pane: Target<PaneId>) -> Outcome {
    // QA-225: the PTY write can block on a full kernel buffer, so it must
    // not run under the tree mutex. Snapshot the target pane, the buffer
    // content, and the pane's input handle under the lock, then write with
    // the lock released (the QA-221 snapshot-then-write shape).
    let (input, content) = {
        let guard = ctx.tree.lock();
        let pane = match guard.resolve_pane_target(pane) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        };
        let Some(target) = guard.pane(pane) else {
            return Outcome::err(ctx, &format!("no such pane: {pane}"));
        };
        let Some(content) = guard.get_buffer(DEFAULT_BUFFER).map(str::to_string) else {
            return Outcome::err(ctx, "no buffers");
        };
        match target.input_handle() {
            Ok(handle) => (handle, content),
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    match input.write(content.as_bytes()) {
        Ok(()) => Outcome::ok(ctx, ""),
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

/// Wire contract: the reply body is exactly one line — the daemon's
/// [`build_stamp`](crate::mux::build_stamp). Tree-free by design; a stale
/// daemon must still answer it, so nothing here may depend on session
/// state that a long-lived daemon could have torn down.
fn cmd_version(ctx: &Ctx<'_>) -> Outcome {
    Outcome::ok(ctx, crate::mux::build_stamp())
}

/// `reload-config`: re-read the canonical config file and diff its
/// `[daemon]` section against the applied copy. The file speaks only for
/// settings it actually states — a setting absent from the file is
/// `unchanged` (the tier did not move, which is what keeps a daemon
/// started with `--socket`/`--state-dir` flags quiet: the flag is a
/// one-shot override the file cannot express, so it never reads back as
/// a change). Per `[daemon]` setting, one reply line:
/// - `restart-required: <name>` when the file states a value that
///   differs from the applied copy — in v1 every daemon setting is
///   startup-resolved (the socket and the persist path are fixed at
///   bind; see [`crate::mux::config`]'s module docs), so nothing can be
///   applied live;
/// - `unchanged: <name>` otherwise.
///
/// A file that fails to load is a `%error` naming the problem — a
/// silently ignored config change would look exactly like a reload that
/// did nothing.
fn cmd_reload_config(ctx: &Ctx<'_>) -> Outcome {
    use crate::mux::config::{load_file, reload_report};
    let Some(applied) = ctx.config else {
        // No applied copy: an embedded server without config support, or
        // the dispatch test shim. Honest report, not a fake success.
        return Outcome::err(
            ctx,
            "reload-config: this server has no applied config to reload",
        );
    };
    // A PRESENT file that does not parse is an error, not defaults: the
    // user's edit must surface, not vanish. Absent = nothing changed.
    let Some(path) = crate::mux::config::config_file_path() else {
        return Outcome::err(ctx, "reload-config: no config path on this platform");
    };
    let file = match load_file(&path) {
        Ok(Some(file)) => Some(file),
        Ok(None) => None,
        Err(err) => return Outcome::err(ctx, &format!("reload-config: {err}")),
    };
    let report = reload_report(&mut applied.lock(), file.as_ref());
    Outcome::ok(ctx, &report)
}

/// ENH-037: capability discovery — the sorted command roster with feature
/// tokens, generated from the same `COMMANDS` table the parser dispatches
/// from, so a new command is discoverable the moment its row lands.
fn cmd_list_commands(ctx: &Ctx<'_>) -> Outcome {
    Outcome::ok(ctx, &list_commands_body())
}

#[cfg(test)]
mod tests {
    use super::{dispatch_command, resolve_start_dir, roster_row_entry, Ctx};
    use crate::mux::command::parse_command;
    use crate::mux::ids::PaneId;
    use crate::mux::pane::{MuxError, MuxPane, PaneFactory, ShellPaneFactory, SpawnContext};
    use crate::mux::tree::MuxTree;
    use base64::Engine as _;
    use parking_lot::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    /// `clear-history` wipes scrollback and the visible screen through the
    /// pane's own emulator (ED 3), so neither a grid-length check nor the
    /// daemon's own capture path finds the prefilled content afterward.
    #[test]
    fn clear_history_wipes_scrollback_and_screen() {
        let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(
            ShellPaneFactory::default(),
        ))));
        let clients = Arc::new(Mutex::new(Vec::new()));
        let ctx = Ctx {
            tree: &tree,
            clients: &clients,
            command_number: 1,
            shutdown: None,
            config: None,
            client_id: None,
        };
        let run = |line: &str| dispatch_command(parse_command(line).unwrap(), &ctx, None, None);
        run("new-session -s main");
        let pane = {
            let guard = tree.lock();
            let session = guard.sessions()[0];
            let window = guard.session(session).unwrap().windows[0];
            guard.window(window).unwrap().panes()[0]
        };
        {
            let terminal = tree.lock().pane(pane).unwrap().terminal();
            let mut term = terminal.write();
            for i in 0..60 {
                term.process(format!("line-{i}\r\n").as_bytes());
            }
        }
        assert!(
            tree.lock()
                .pane(pane)
                .unwrap()
                .terminal()
                .read()
                .grid()
                .scrollback_len()
                > 0,
            "the prefill must produce scrollback"
        );

        let reply = run(&format!("clear-history -t {pane}"));
        assert!(!reply.contains("%error"), "clear failed: {reply}");

        let term = tree.lock().pane(pane).unwrap().terminal();
        assert_eq!(
            term.read().grid().scrollback_len(),
            0,
            "scrollback must be gone"
        );
        let capture = run(&format!("capture-pane -t {pane}"));
        assert!(
            !capture.contains("line-"),
            "the visible screen must be wiped: {capture}"
        );
    }

    /// The attach handshake's target-less `refresh-client -C WxH -p WxH`:
    /// a pure size report. `-p` lands daemon-wide, `-C` resizes the newest
    /// session's active window (the same stand-in bare `new-window` uses),
    /// `%layout-change` broadcasts, and — the gap this form exists to close
    /// — the command never errors and never replays a pane's screen.
    #[test]
    fn target_less_refresh_client_sizes_the_newest_active_window() {
        let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(
            ShellPaneFactory::default(),
        ))));
        let clients = Arc::new(Mutex::new(Vec::new()));
        let ctx = Ctx {
            tree: &tree,
            clients: &clients,
            command_number: 1,
            shutdown: None,
            config: None,
            client_id: None,
        };
        let run = |line: &str| dispatch_command(parse_command(line).unwrap(), &ctx, None, None);
        // Two sessions: newest = second. Its active window is what -C hits.
        run("new-session -s first");
        run("new-session -s second");
        // A registered broadcast sink, so the resize's %layout-change has
        // somewhere to land (the a_failed_respawn test's client shape).
        let (sink_tx, sink_rx) = std::sync::mpsc::sync_channel(4096);
        clients.lock().push((
            u64::MAX,
            sink_tx,
            Arc::new(AtomicBool::new(false)),
            crate::mux::ipc::ConnectionAbort::none(),
        ));
        let (second_window, pane) = {
            let guard = tree.lock();
            let session = *guard
                .sessions()
                .iter()
                .find(|s| guard.session(**s).unwrap().name == "second")
                .expect("the second session exists");
            let window = guard.session(session).unwrap().windows[0];
            let pane = guard.window(window).unwrap().panes()[0];
            (window, pane)
        };
        {
            let guard = tree.lock();
            assert_eq!(
                guard.window(second_window).unwrap().cols,
                80,
                "the default grid starts at 80 columns"
            );
        }

        let reply = run("refresh-client -C 100x40 -p 12x24");
        assert!(
            !reply.contains("%error"),
            "the target-less size report must land, not error: {reply}"
        );
        // The resize broadcasts %layout-change to registered clients — here
        // the one sink the test pushed into the registry. A ConPTY pane's
        // init bytes (the ESC[6n probe, Windows) ride the same queue and
        // can land first when the spawn is slow; skip anything that is not
        // the layout change.
        let broadcast = loop {
            let item = sink_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("the resize broadcast a layout change");
            if item.starts_with("%layout-change") {
                break item;
            }
        };
        assert!(
            broadcast.contains(&format!("%layout-change {second_window} ")),
            "the resize broadcasts geometry: {broadcast}"
        );
        let guard = tree.lock();
        let window = guard.window(second_window).unwrap();
        assert_eq!(
            (window.cols, window.rows),
            (100, 40),
            "the newest session's active window takes the reported size"
        );
        // -p is daemon-wide.
        assert_eq!(
            guard
                .pane(pane)
                .unwrap()
                .terminal()
                .read()
                .graphics
                .cell_dimensions,
            (12, 24),
            "the pixel report lands daemon-wide"
        );
    }

    /// Target-less with no sessions at all: an error naming the absence,
    /// not a panic — the same outcome bare `new-window` gives.
    #[test]
    fn target_less_refresh_client_with_no_sessions_errors_cleanly() {
        let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(
            ShellPaneFactory::default(),
        ))));
        let clients = Arc::new(Mutex::new(Vec::new()));
        let ctx = Ctx {
            tree: &tree,
            clients: &clients,
            command_number: 1,
            shutdown: None,
            config: None,
            client_id: None,
        };
        let reply = dispatch_command(
            parse_command("refresh-client -C 100x40").unwrap(),
            &ctx,
            None,
            None,
        );
        assert!(
            reply.contains("%error") && reply.contains("no sessions"),
            "an empty tree rejects the size report: {reply}"
        );
    }

    /// The -t form keeps its contract: a screen-restore replay only ever
    /// happens for it. The target-less size report of the same daemon
    /// carries no pane content.
    #[test]
    fn target_less_refresh_client_never_replays_a_screen() {
        let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(
            ShellPaneFactory::default(),
        ))));
        let clients = Arc::new(Mutex::new(Vec::new()));
        let ctx = Ctx {
            tree: &tree,
            clients: &clients,
            command_number: 1,
            shutdown: None,
            config: None,
            client_id: None,
        };
        let run = |line: &str| dispatch_command(parse_command(line).unwrap(), &ctx, None, None);
        run("new-session -s main");
        let pane = {
            let guard = tree.lock();
            let session = guard.sessions()[0];
            let window = guard.session(session).unwrap().windows[0];
            guard.window(window).unwrap().panes()[0]
        };
        {
            let terminal = tree.lock().pane(pane).unwrap().terminal();
            let mut term = terminal.write();
            term.process(b"PANE-MARKER");
        }
        let reply = run("refresh-client -C 120x40");
        assert!(
            !reply.contains("PANE-MARKER"),
            "a size report must never replay a pane screen: {reply}"
        );
    }

    /// Spawns the first pane as a shell; every later spawn fails.
    struct FirstSpawnOnly(AtomicBool);

    impl PaneFactory for FirstSpawnOnly {
        fn create_pane(
            &self,
            id: PaneId,
            cols: u16,
            rows: u16,
            _command: Option<&str>,
            context: &SpawnContext<'_>,
        ) -> Result<MuxPane, MuxError> {
            if self.0.swap(true, Ordering::SeqCst) {
                return Err(MuxError::NoSuchPane(id));
            }
            ShellPaneFactory::default().create_pane(id, cols, rows, None, context)
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

    /// ARC-089: `respawn-pane -k` stops forwarding the old process before
    /// the off-lock spawn; when that spawn fails the old pane is still
    /// the live pane, so its output must forward again.
    #[test]
    fn a_failed_respawn_keeps_the_live_panes_output_forwarding() {
        let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(FirstSpawnOnly(
            AtomicBool::new(false),
        )))));
        let clients = Arc::new(Mutex::new(Vec::new()));
        let ctx = Ctx {
            tree: &tree,
            clients: &clients,
            command_number: 1,
            shutdown: None,
            config: None,
            client_id: None,
        };
        let run = |line: &str| dispatch_command(parse_command(line).unwrap(), &ctx, None, None);
        run("new-session -s main");
        let pane = {
            let guard = tree.lock();
            let session = guard.sessions()[0];
            let window = guard.session(session).unwrap().windows[0];
            guard.window(window).unwrap().panes()[0]
        };
        let (tx, rx) = std::sync::mpsc::sync_channel(4096);
        clients.lock().push((
            u64::MAX,
            tx,
            Arc::new(AtomicBool::new(false)),
            crate::mux::ipc::ConnectionAbort::none(),
        ));

        let reply = run(&format!("respawn-pane -k -t {pane}"));
        assert!(reply.contains("%error"), "the spawn failed: {reply}");

        tree.lock()
            .pane_mut(pane)
            .unwrap()
            .write(b"echo ARC089-STILL-WIRED\r")
            .unwrap();
        let wanted = format!("%output {pane} ");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut seen = String::new();
        while !seen.contains("ARC089-STILL-WIRED") {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match rx.recv_timeout(left) {
                // Payload only, so a marker split across two PTY reads
                // still joins up.
                Ok(line) => {
                    if let Some(data) = line.strip_prefix(&wanted) {
                        seen.push_str(data.trim_end_matches('\n'));
                    }
                }
                Err(_) => panic!("the live pane's output stopped forwarding: {seen:?}"),
            }
        }
        let _ = tree.lock().pane_mut(pane).unwrap().kill();
    }

    /// The `-c` degrade rule (card 01a0d9b2fb02): an existing directory
    /// passes through untouched; a missing one falls back to home with a
    /// note naming both — the command must not fail because a directory
    /// this process does not control vanished.
    #[test]
    fn a_gone_start_directory_degrades_to_home_with_a_note() {
        let dir = tempfile::tempdir().unwrap();
        let (cwd, note) = resolve_start_dir(dir.path().to_str());
        assert_eq!(cwd.as_deref(), Some(dir.path()));
        assert!(note.is_none(), "an existing dir needs no note");

        let (cwd, note) = resolve_start_dir(Some("/par-mux-test-no-such-dir"));
        let home = dirs::home_dir().unwrap();
        assert_eq!(cwd.as_deref(), Some(home.as_path()));
        let note = note.expect("the fallback is visible");
        assert!(
            note.contains("/par-mux-test-no-such-dir is gone") && note.contains("pane started in"),
            "note names the gone dir and the landing dir: {note}"
        );
    }

    #[test]
    fn roster_row_entry_encodes_the_reason_as_one_unambiguous_token() {
        // ARC-060: a reason ending in `hook` or containing `telemetry=`
        // cannot be confused with the source column or the key=value
        // tail — every token after source is key=value and base64.
        let entry = roster_row_entry(
            "pi",
            "blocked",
            "hook",
            Some("waiting on telemetry=x hook"),
            Some("dGVsZW1ldHJ5"),
            Some("aG9zdA=="),
        );
        let mut tokens = entry.split_whitespace();
        assert_eq!(tokens.next(), Some("pi"), "agent is positional 1");
        assert_eq!(tokens.next(), Some("blocked"), "state is positional 2");
        assert_eq!(tokens.next(), Some("hook"), "source is positional 3");
        let rest: Vec<&str> = tokens.collect();
        assert_eq!(rest.len(), 3, "one token per optional field: {rest:?}");
        assert_eq!(
            rest[0].split_once('=').map(|(k, _)| k),
            Some("reason"),
            "reason precedes the telemetry tokens"
        );
        for token in &rest {
            let (key, value) = token.split_once('=').expect("key=value token");
            assert!(
                !value.contains(char::is_whitespace),
                "{key} token is whitespace-free"
            );
            assert!(
                base64::engine::general_purpose::STANDARD
                    .decode(value)
                    .is_ok(),
                "{key} token is standard base64"
            );
        }
        let reason_b64 = rest[0].strip_prefix("reason=").unwrap();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(reason_b64)
            .unwrap();
        assert_eq!(
            decoded, b"waiting on telemetry=x hook",
            "the reason round-trips with both ambiguity shapes intact"
        );
        assert!(
            !rest.contains(&"hook"),
            "no token can be mistaken for the source"
        );

        // Absent fields add nothing: the row keeps its exact shorter shape.
        assert_eq!(
            roster_row_entry("pi", "working", "hook", None, None, None),
            "pi working hook"
        );
    }

    /// `reload-config` reports restart-required per changed daemon setting,
    /// diffing the re-read file against the applied copy the server
    /// published.
    #[test]
    fn reload_config_reports_restart_required_per_changed_setting() {
        let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(
            ShellPaneFactory::default(),
        ))));
        let clients = Arc::new(Mutex::new(Vec::new()));
        // The applied copy: what this (fake) daemon started with.
        let applied = Arc::new(Mutex::new(crate::mux::config::EffectiveConfig {
            state_dir: "/tmp/old-state".to_string(),
            ..crate::mux::config::EffectiveConfig::default()
        }));
        let ctx = Ctx {
            client_id: None,
            tree: &tree,
            clients: &clients,
            command_number: 1,
            shutdown: None,
            config: Some(&applied),
        };
        // No canonical config file is readable in the test harness (and
        // QA-196 forbids env mutation to pin one): the dispatch with an
        // absent file is the all-unchanged reply shape.
        let reply = dispatch_command(parse_command("reload-config").unwrap(), &ctx, None, None);
        let unchanged = reply.lines().filter(|l| l.contains("unchanged:")).count();
        assert_eq!(
            unchanged, 6,
            "an absent file leaves every setting unchanged: {reply}"
        );
        // The pure report over a written file: the moved/flipped settings
        // are restart-required, the unstated one stays unchanged.
        let file: crate::mux::config::ConfigFile =
            toml::from_str("[daemon]\nstate-dir = \"/tmp/new-state\"\npane-endpoints = true\n")
                .unwrap();
        let mut applied_copy = applied.lock().clone();
        let report = crate::mux::config::reload_report(&mut applied_copy, Some(&file));
        assert!(
            report.contains("restart-required: daemon.state-dir"),
            "the moved state-dir is reported: {report}"
        );
        assert!(
            report.contains("restart-required: daemon.pane-endpoints"),
            "the flipped bool is reported: {report}"
        );
        assert!(
            report.contains("unchanged: daemon.socket"),
            "an unstated setting says unchanged: {report}"
        );
    }

    /// A server with no applied config (the embedder/test shape) answers
    /// reload-config with an explicit error, not a fake success.
    #[test]
    fn reload_config_without_an_applied_copy_errors() {
        let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(
            ShellPaneFactory::default(),
        ))));
        let clients = Arc::new(Mutex::new(Vec::new()));
        let ctx = Ctx {
            tree: &tree,
            clients: &clients,
            command_number: 1,
            shutdown: None,
            config: None,
            client_id: None,
        };
        let reply = dispatch_command(parse_command("reload-config").unwrap(), &ctx, None, None);
        assert!(
            reply.contains("%error") && reply.contains("no applied config"),
            "honest refusal: {reply}"
        );
    }

    /// reload-config takes no arguments (the reject_positionals rule the
    /// other no-start-command commands follow).
    #[test]
    fn reload_config_rejects_positionals() {
        assert!(parse_command("reload-config now").is_err());
        assert!(parse_command("reload-config").is_ok());
    }

    /// remain-on-exit is the one LIVE daemon setting: a reload-config
    /// dispatch whose file states a different value applies it to the
    /// server's applied copy on the spot — the next observed death honors
    /// the new value with no restart.
    #[test]
    fn reload_config_applies_remain_on_exit_to_the_applied_copy() {
        let applied = Arc::new(Mutex::new(crate::mux::config::EffectiveConfig::default()));
        // No canonical config file is readable in the harness (QA-196
        // forbids env mutation to pin one), so the live-apply is driven
        // through the pure report the dispatch calls.
        let file: crate::mux::config::ConfigFile =
            toml::from_str("[daemon]\nremain-on-exit = true").unwrap();
        let report = crate::mux::config::reload_report(&mut applied.lock(), Some(&file));
        assert!(
            report.contains("applied: daemon.remain-on-exit"),
            "the live setting applies: {report}"
        );
        assert!(
            applied.lock().remain_on_exit,
            "the applied copy now holds the dead panes"
        );
    }

    /// Typed ids resolve without an existence check, so each handler's
    /// tree operation must reject an unknown one itself. These four did
    /// not: an unknown window swapped with itself replied `%end` and
    /// broadcast `%sessions-changed`; an unknown workspace listed as an
    /// empty success; an unknown swap-pane source was reported as "in
    /// different windows"; an unknown pane joined onto itself was
    /// reported as "cannot be moved onto itself".
    #[test]
    fn unknown_typed_ids_are_rejected_as_missing_not_misreported() {
        let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(
            ShellPaneFactory::default(),
        ))));
        let clients = Arc::new(Mutex::new(Vec::new()));
        let ctx = Ctx {
            tree: &tree,
            clients: &clients,
            command_number: 1,
            shutdown: None,
            config: None,
            client_id: None,
        };
        let run = |line: &str| dispatch_command(parse_command(line).unwrap(), &ctx, None, None);
        run("new-session -s main");
        let pane = {
            let guard = tree.lock();
            let session = guard.sessions()[0];
            let window = guard.session(session).unwrap().windows[0];
            guard.window(window).unwrap().panes()[0]
        };
        for (line, expected) in [
            ("swap-window -s @99 -t @99", "no such window: @99"),
            ("list-sessions -t +99", "no such workspace: +99"),
            (&*format!("swap-pane -s %99 -t {pane}"), "no such pane: %99"),
            ("join-pane -s %99 -t %99", "no such pane: %99"),
        ] {
            let reply = run(line);
            let lines: Vec<&str> = reply.lines().collect();
            assert_eq!(lines.len(), 3, "`{line}`: {reply:?}");
            assert_eq!(lines[1], expected, "`{line}`: {reply:?}");
            assert!(lines[2].starts_with("%error "), "`{line}`: {reply:?}");
        }
        let _ = tree.lock().pane_mut(pane).unwrap().kill();
    }

    /// Creates every pane dead (no process): the tree mechanics run for
    /// real, but no shell starts, so no `%output` races the assertions.
    struct DeadPaneFactory;

    impl PaneFactory for DeadPaneFactory {
        fn create_pane(
            &self,
            id: PaneId,
            cols: u16,
            rows: u16,
            command: Option<&str>,
            _context: &SpawnContext<'_>,
        ) -> Result<MuxPane, MuxError> {
            self.create_dead_pane(id, cols, rows, command, Some(0))
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

    /// A dispatch harness over a process-free tree with one registered
    /// broadcast sink.
    struct Harness {
        tree: Arc<Mutex<MuxTree>>,
        clients: crate::mux::server::Clients,
        sink: std::sync::mpsc::Receiver<String>,
    }

    impl Harness {
        fn new() -> Self {
            let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(DeadPaneFactory))));
            let clients: crate::mux::server::Clients = Arc::new(Mutex::new(Vec::new()));
            let (sink_tx, sink) = std::sync::mpsc::sync_channel(4096);
            clients.lock().push((
                u64::MAX,
                sink_tx,
                Arc::new(AtomicBool::new(false)),
                crate::mux::ipc::ConnectionAbort::none(),
            ));
            Self {
                tree,
                clients,
                sink,
            }
        }

        fn run_as(&self, client_id: Option<u64>, line: &str) -> String {
            let ctx = Ctx {
                tree: &self.tree,
                clients: &self.clients,
                command_number: 7,
                shutdown: None,
                config: None,
                client_id,
            };
            dispatch_command(parse_command(line).unwrap(), &ctx, None, None)
        }

        fn run(&self, line: &str) -> String {
            self.run_as(None, line)
        }

        /// Everything broadcast since the last drain, one entry per send.
        fn drain(&self) -> Vec<String> {
            self.sink.try_iter().collect()
        }

        /// A stable picture of the tree: workspaces, sessions, windows in
        /// order, and each window's panes and extent.
        fn snapshot(&self) -> String {
            let guard = self.tree.lock();
            let mut out = String::new();
            let mut sessions = guard.sessions();
            sessions.sort();
            for ws in guard.workspaces() {
                let ws = guard.workspace(ws).unwrap();
                out.push_str(&format!("ws {} {} {:?};", ws.id, ws.name, ws.sessions));
            }
            for s in sessions {
                let session = guard.session(s).unwrap();
                out.push_str(&format!(
                    "s {} {} active={} ",
                    session.id, session.name, session.active
                ));
                for w in &session.windows {
                    let window = guard.window(*w).unwrap();
                    out.push_str(&format!(
                        "[{} {} {}x{} {:?}]",
                        window.id,
                        window.name,
                        window.cols,
                        window.rows,
                        window.panes()
                    ));
                }
                out.push(';');
            }
            out
        }

        fn session_named(&self, name: &str) -> crate::mux::ids::SessionId {
            let guard = self.tree.lock();
            guard
                .sessions()
                .into_iter()
                .find(|s| guard.session(*s).unwrap().name == name)
                .expect("session exists")
        }

        fn windows_of(
            &self,
            session: crate::mux::ids::SessionId,
        ) -> Vec<crate::mux::ids::WindowId> {
            self.tree.lock().session(session).unwrap().windows.clone()
        }

        fn panes_of(&self, window: crate::mux::ids::WindowId) -> Vec<PaneId> {
            self.tree.lock().window(window).unwrap().panes()
        }
    }

    /// Splits a reply block into (body lines, closed with `%end`).
    fn reply_parts(reply: &str) -> (Vec<&str>, bool) {
        let lines: Vec<&str> = reply.lines().collect();
        assert!(
            lines.first().is_some_and(|l| l.starts_with("%begin ")),
            "a reply opens with %begin: {reply:?}"
        );
        let last = lines.last().copied().unwrap_or_default();
        let ok = if last.starts_with("%end ") {
            true
        } else if last.starts_with("%error ") {
            false
        } else {
            panic!("a reply closes with %end or %error: {reply:?}")
        };
        (lines[1..lines.len() - 1].to_vec(), ok)
    }

    fn assert_error(reply: &str, expected: &str) {
        let (body, ok) = reply_parts(reply);
        assert!(!ok, "expected an %error block: {reply:?}");
        assert_eq!(body, vec![expected], "error text for: {reply:?}");
    }

    fn assert_ok(reply: &str) -> Vec<String> {
        let (body, ok) = reply_parts(reply);
        assert!(ok, "expected an %end block: {reply:?}");
        body.into_iter().map(str::to_string).collect()
    }

    /// Every targeted command answers an unknown id or name with the exact
    /// `no such …` text inside an `%error` block, and leaves the tree and
    /// the broadcast stream untouched — a typo'd target must never land on
    /// some other object or announce a change that did not happen.
    #[test]
    fn unknown_targets_error_exactly_and_change_nothing() {
        let h = Harness::new();
        assert_ok(&h.run("new-session -s main"));
        assert_ok(&h.run("set-buffer clip"));
        h.drain();
        let before = h.snapshot();
        let cases: &[(&str, &str)] = &[
            ("send-keys -t %99 x", "no such pane: %99"),
            ("refresh-client -t %99 -C 100x40", "no such pane: %99"),
            ("kill-pane -t %99", "no such pane: %99"),
            ("split-window -t %99", "no such pane: %99"),
            ("select-pane -t %99", "no such pane: %99"),
            ("select-pane -t %99 -T title", "no such pane: %99"),
            ("pane-title -t %99", "no such pane: %99"),
            ("clear-history -t %99", "no such pane: %99"),
            ("resize-pane -t %99 -L 2", "no such pane: %99"),
            ("swap-pane -s %99 -t %0", "no such pane: %99"),
            ("swap-pane -s %0 -t %99", "no such pane: %99"),
            ("break-pane -s %99", "no such pane: %99"),
            ("join-pane -s %99 -t %0", "no such pane: %99"),
            ("join-pane -s %0 -t %99", "no such pane: %99"),
            ("respawn-pane -k -t %99", "no such pane: %99"),
            ("capture-pane -t %99", "no such pane: %99"),
            ("paste-buffer -t %99", "no such pane: %99"),
            ("select-pane -t ghost", "no such pane: ghost"),
            ("list-panes -t @99", "no such window: @99"),
            ("select-window -t @99", "no such window: @99"),
            ("kill-window -t @99", "no such window: @99"),
            ("kill-window -t nope", "no such window: nope"),
            ("rename-window -t @99 x", "no such window: @99"),
            ("move-window -s @99 -t 0", "no such window: @99"),
            ("move-window -s nope -t 0", "no such window: nope"),
            ("swap-window -s nope -t @0", "no such window: nope"),
            ("swap-window -s @99 -t @99", "no such window: @99"),
            ("swap-pane -s %99 -t %99", "no such pane: %99"),
            ("join-pane -s %99 -t %99", "no such pane: %99"),
            ("new-session -s extra -t +99", "no such workspace: +99"),
            ("new-session -s extra -t nope", "no such workspace: nope"),
            ("new-window -t @99", "no such window: @99"),
            ("list-windows -t $99", "no such session: $99"),
            ("rename-session -t $99 x", "no such session: $99"),
            ("rename-session -t nope x", "no such session: nope"),
            ("kill-session -t $99", "no such session: $99"),
            ("kill-session -t nope", "no such session: nope"),
            ("set-environment -t $99 K V", "no such session: $99"),
            ("list-sessions -t +99", "no such workspace: +99"),
            ("select-workspace -t +99", "no such workspace: +99"),
            ("select-workspace -t nope", "no such workspace: nope"),
            ("rename-workspace -t +99 x", "no such workspace: +99"),
            ("rename-workspace -t nope x", "no such workspace: nope"),
            ("kill-workspace -t +99", "no such workspace: +99"),
            ("kill-workspace -t nope", "no such workspace: nope"),
        ];
        let mut mismatches = Vec::new();
        for (line, expected) in cases {
            let reply = h.run(line);
            let (body, ok) = reply_parts(&reply);
            if ok || body != vec![*expected] {
                mismatches.push(format!("`{line}`: want {expected:?}, got {reply:?}"));
            }
            assert_eq!(h.snapshot(), before, "`{line}` changed the tree");
            assert_eq!(h.drain(), Vec::<String>::new(), "`{line}` broadcast");
        }
        assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    }

    /// A bad target is reported before an empty buffer: `paste-buffer -t
    /// %99` with nothing buffered names the missing pane, not the buffer.
    #[test]
    fn paste_buffer_reports_the_bad_target_before_the_empty_buffer() {
        let h = Harness::new();
        assert_ok(&h.run("new-session -s main"));
        let reply = h.run("paste-buffer -t %99");
        let (body, ok) = reply_parts(&reply);
        assert!(!ok, "{reply:?}");
        assert_eq!(body, vec!["no such pane: %99"], "{reply:?}");
        let reply = h.run("paste-buffer -t %0");
        let (body, ok) = reply_parts(&reply);
        assert!(!ok && body == vec!["no buffers"], "{reply:?}");
    }

    /// A name shared by two objects is refused with every candidate listed,
    /// and neither object is touched — guessing one would kill the wrong
    /// window.
    #[test]
    fn ambiguous_names_list_the_candidates_and_touch_nothing() {
        let h = Harness::new();
        assert_ok(&h.run("new-session -s main"));
        let s = h.session_named("main");
        assert_ok(&h.run(&format!("new-window -t {s}")));
        let windows = h.windows_of(s);
        assert_eq!(windows.len(), 2);
        for w in &windows {
            assert_ok(&h.run(&format!("rename-window -t {w} dup")));
        }
        let panes: Vec<PaneId> = windows.iter().map(|w| h.panes_of(*w)[0]).collect();
        for p in &panes {
            assert_ok(&h.run(&format!("select-pane -t {p} -T twin")));
        }
        h.drain();
        let before = h.snapshot();

        assert_error(
            &h.run("kill-window -t dup"),
            &format!(
                "ambiguous window target: dup (matching: {}, {})",
                windows[0], windows[1]
            ),
        );
        assert_error(
            &h.run("kill-pane -t twin"),
            &format!(
                "ambiguous pane target: twin (matching: {}, {})",
                panes[0], panes[1]
            ),
        );
        assert_eq!(
            h.snapshot(),
            before,
            "the ambiguous refusals touched nothing"
        );
        assert!(h.drain().is_empty());

        assert_ok(&h.run("new-session -s main"));
        let before = h.snapshot();
        let mut sessions = h.tree.lock().sessions();
        sessions.sort();
        assert_error(
            &h.run("kill-session -t main"),
            &format!(
                "ambiguous session target: main (matching: {}, {})",
                sessions[0], sessions[1]
            ),
        );
        assert_eq!(
            h.snapshot(),
            before,
            "neither same-named session was killed"
        );
    }

    /// `move-window -s @N -t <index>` reorders the session's window list,
    /// keeps the active window active by identity, and cues clients with
    /// `%sessions-changed`; an index past the end clamps to the end.
    #[test]
    fn move_window_reorders_keeps_active_and_cues_sessions_changed() {
        let h = Harness::new();
        assert_ok(&h.run("new-session -s main"));
        let s = h.session_named("main");
        assert_ok(&h.run(&format!("new-window -t {s}")));
        assert_ok(&h.run(&format!("new-window -t {s}")));
        let [w0, w1, w2] = <[_; 3]>::try_from(h.windows_of(s)).unwrap();
        assert_ok(&h.run(&format!("select-window -t {w1}")));
        h.drain();

        assert_ok(&h.run(&format!("move-window -s {w2} -t 0")));
        assert_eq!(h.windows_of(s), vec![w2, w0, w1]);
        let active = {
            let guard = h.tree.lock();
            let session = guard.session(s).unwrap();
            session.windows[session.active]
        };
        assert_eq!(active, w1, "the active window stays active through a move");
        assert_eq!(h.drain(), vec!["%sessions-changed\n".to_string()]);

        assert_ok(&h.run(&format!("move-window -s {w2} -t 99")));
        assert_eq!(
            h.windows_of(s),
            vec![w0, w1, w2],
            "an index past the end clamps"
        );
    }

    /// Windows of two different sessions cannot swap — that would move
    /// ownership, not order — and the refusal names both windows.
    #[test]
    fn swap_window_across_sessions_is_refused_by_name() {
        let h = Harness::new();
        assert_ok(&h.run("new-session -s a"));
        assert_ok(&h.run("new-session -s b"));
        let wa = h.windows_of(h.session_named("a"))[0];
        let wb = h.windows_of(h.session_named("b"))[0];
        h.drain();
        let before = h.snapshot();
        assert_error(
            &h.run(&format!("swap-window -s {wa} -t {wb}")),
            &format!("windows {wa} and {wb} are in different sessions"),
        );
        assert_eq!(h.snapshot(), before);
        assert!(h.drain().is_empty());
    }

    /// Breaking a window's only pane out closes the source window
    /// (`%window-close`) and announces the new one (`%window-add`); the
    /// reply names the new window.
    #[test]
    fn break_pane_of_a_lone_pane_closes_the_source_window() {
        let h = Harness::new();
        assert_ok(&h.run("new-session -s main"));
        let s = h.session_named("main");
        let source = h.windows_of(s)[0];
        let pane = h.panes_of(source)[0];
        h.drain();

        let body = assert_ok(&h.run(&format!("break-pane -s {pane} -n solo")));
        let new_window = h.windows_of(s)[0];
        assert_ne!(new_window, source);
        assert_eq!(body, vec![new_window.to_string()]);
        assert_eq!(h.panes_of(new_window), vec![pane]);
        let sent = h.drain();
        assert!(
            sent.iter()
                .any(|l| l.starts_with(&format!("%window-add {new_window}"))),
            "the new window is announced: {sent:?}"
        );
        assert!(
            sent.contains(&format!("%window-close {source}\n")),
            "the emptied source window closes: {sent:?}"
        );
        assert!(h.tree.lock().window(source).is_none());
    }

    /// join-pane out of a two-pane window keeps the source window alive and
    /// re-lays out both windows; joining a session's last pane away removes
    /// that session and cues `%sessions-changed`.
    #[test]
    fn join_pane_relayouts_both_windows_and_reaps_an_emptied_session() {
        let h = Harness::new();
        assert_ok(&h.run("new-session -s a"));
        assert_ok(&h.run("new-session -s b"));
        let wa = h.windows_of(h.session_named("a"))[0];
        let wb = h.windows_of(h.session_named("b"))[0];
        let a0 = h.panes_of(wa)[0];
        let b0 = h.panes_of(wb)[0];
        assert_ok(&h.run(&format!("split-window -t {a0}")));
        let a1 = *h.panes_of(wa).iter().find(|p| **p != a0).unwrap();
        h.drain();

        assert_ok(&h.run(&format!("join-pane -s {a1} -t {b0}")));
        assert_eq!(h.panes_of(wa), vec![a0], "the source keeps its other pane");
        assert!(h.panes_of(wb).contains(&a1));
        let sent = h.drain();
        for w in [wa, wb] {
            assert!(
                sent.iter()
                    .any(|l| l.starts_with(&format!("%layout-change {w} "))),
                "both windows re-lay out ({w}): {sent:?}"
            );
        }
        assert!(!sent.iter().any(|l| l.starts_with("%window-close")));

        // Now move session a's last pane away: its window and session go.
        assert_ok(&h.run(&format!("join-pane -s {a0} -t {b0}")));
        let sent = h.drain();
        assert!(sent.contains(&format!("%window-close {wa}\n")), "{sent:?}");
        assert!(
            sent.contains(&"%sessions-changed\n".to_string()),
            "{sent:?}"
        );
        let names: Vec<String> = {
            let guard = h.tree.lock();
            guard
                .sessions()
                .into_iter()
                .map(|s| guard.session(s).unwrap().name.clone())
                .collect()
        };
        assert_eq!(names, vec!["b".to_string()]);
        assert_eq!(h.panes_of(wb).len(), 3);
    }

    /// `resize-pane -L`/`-D` shrink a pane's width and grow its height by
    /// the requested cells, bounded by the window.
    #[test]
    fn resize_pane_left_and_down_move_the_split_by_the_requested_cells() {
        let h = Harness::new();
        assert_ok(&h.run("new-session -s main"));
        let w = h.windows_of(h.session_named("main"))[0];
        let p0 = h.panes_of(w)[0];
        let rect = |pane: PaneId| {
            let guard = h.tree.lock();
            let window = guard.window(w).unwrap();
            window
                .layout
                .geometry(0, 0, window.cols as usize, window.rows as usize)
                .into_iter()
                .find(|g| g.pane == pane)
                .map(|g| (g.width, g.height))
                .unwrap()
        };
        // A horizontal split (side by side), then a vertical one in p0.
        assert_ok(&h.run(&format!("split-window -h -t {p0}")));
        assert_ok(&h.run(&format!("split-window -v -t {p0}")));
        let (w0, h0) = rect(p0);
        h.drain();

        assert_ok(&h.run(&format!("resize-pane -t {p0} -L 5")));
        assert_eq!(rect(p0), (w0 - 5, h0), "-L 5 takes five columns");
        assert_ok(&h.run(&format!("resize-pane -t {p0} -D 3")));
        assert_eq!(rect(p0), (w0 - 5, h0 + 3), "-D 3 adds three rows");
        let sent = h.drain();
        assert_eq!(
            sent.iter()
                .filter(|l| l.starts_with(&format!("%layout-change {w} ")))
                .count(),
            2,
            "each resize re-lays out the window once: {sent:?}"
        );
    }

    /// `capture-pane -e` keeps SGR styling; the plain form strips it.
    #[test]
    fn capture_pane_escape_flag_controls_styling() {
        let h = Harness::new();
        assert_ok(&h.run("new-session -s main"));
        let p = h.panes_of(h.windows_of(h.session_named("main"))[0])[0];
        {
            let term = h.tree.lock().pane(p).unwrap().terminal();
            term.write().process(b"\x1b[31mRED-TEXT\x1b[0m\r\n");
        }
        let plain = assert_ok(&h.run(&format!("capture-pane -t {p}"))).join("\n");
        let styled = assert_ok(&h.run(&format!("capture-pane -e -t {p}"))).join("\n");
        assert!(
            plain.contains("RED-TEXT") && !plain.contains('\x1b'),
            "{plain:?}"
        );
        assert!(
            styled.contains("RED-TEXT") && styled.contains("\x1b[") && styled.contains("31"),
            "the -e capture keeps the red SGR: {styled:?}"
        );
    }

    /// Without a server's shutdown flag (an embedder dispatch), kill-server
    /// has nothing to stop and says so rather than pretending.
    #[test]
    fn kill_server_without_a_running_server_errors() {
        let h = Harness::new();
        assert_error(
            &h.run("kill-server"),
            "kill-server: no running server to stop",
        );
    }

    /// select-workspace moves every client view shown in the workspace it
    /// leaves onto the newly displayed window, re-fits that window to the
    /// smallest reporting client, and tells clients which session they now
    /// show.
    #[test]
    fn select_workspace_moves_reporting_client_views_and_refits() {
        let h = Harness::new();
        assert_ok(&h.run("new-session -s first"));
        // A client reports 100x30 against the newest (first) session.
        assert_ok(&h.run_as(Some(1), "refresh-client -C 100x30"));
        let first_window = h.windows_of(h.session_named("first"))[0];
        {
            let guard = h.tree.lock();
            let w = guard.window(first_window).unwrap();
            assert_eq!(
                (w.cols, w.rows),
                (100, 30),
                "the report sizes the shown window"
            );
        }
        let original_ws = h.tree.lock().active_workspace().unwrap();
        assert_ok(&h.run("new-workspace -n other"));
        let other_ws = h.tree.lock().active_workspace().unwrap();
        assert_ne!(other_ws, original_ws);
        h.drain();

        // Switching back: the client's view follows from the left
        // workspace (`other`) only if it was shown there — it was not, so
        // switch to `other` first and then back to observe the follow.
        assert_ok(&h.run(&format!("select-workspace -t {original_ws}")));
        h.drain();
        assert_ok(&h.run(&format!("select-workspace -t {other_ws}")));
        let other_window = {
            let guard = h.tree.lock();
            let s = guard.active_session().unwrap();
            let session = guard.session(s).unwrap();
            session.windows[session.active]
        };
        let sent = h.drain();
        assert!(
            sent.contains(&"%workspaces-changed\n".to_string()),
            "{sent:?}"
        );
        assert!(
            sent.iter()
                .any(|l| l.starts_with(&format!("%client-session-changed {other_ws} "))),
            "clients learn the session the display moved to: {sent:?}"
        );
        let guard = h.tree.lock();
        let w = guard.window(other_window).unwrap();
        assert_eq!(
            (w.cols, w.rows),
            (100, 30),
            "the followed view re-fits the new window to the client's report"
        );
    }
}
