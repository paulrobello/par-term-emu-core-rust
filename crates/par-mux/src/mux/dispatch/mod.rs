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

// Handlers by command group (ARC-003); `dispatch_command` below routes to
// them by name.
mod buffers;
mod client;
mod panes;
mod sessions;
mod windows;
use buffers::*;
use client::*;
use panes::*;
use sessions::*;
pub(crate) use sessions::{workspace_roster_changed, workspace_roster_fingerprint};
use windows::*;

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

#[cfg(test)]
mod tests;
