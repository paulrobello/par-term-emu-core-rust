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

use crate::mux::command::{MuxCommand, ResizeAdjustment};
use crate::mux::emit::{emit, emit_block};
use crate::mux::ids::{PaneId, SessionId, Target, WindowId};
use crate::mux::layout::SplitDirection;
use crate::mux::pane::MuxError;
use crate::mux::persist::{PersistState, SaveOrigin};
use crate::mux::server::{
    broadcast_layout_change, broadcast_notification, capture_range, pane_output_sink, Clients,
};
use crate::mux::tree::MuxTree;
use crate::tmux_control::TmuxNotification;
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

/// QA-113 test hook: when set, `dispatch_command` panics before running the
/// command, so the containment tests can drive a guaranteed panic.
#[cfg(test)]
pub(crate) static PANIC_ON_COMMAND: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// The per-dispatch context every handler shares.
pub(super) struct Ctx<'a> {
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
    /// The window a `%layout-change` is broadcast for.
    pub(super) layout_changed: Option<WindowId>,
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
            layout_changed: None,
            succeeded: true,
        }
    }

    /// A failed handler's error block over `message`.
    fn err(ctx: &Ctx<'_>, message: &str) -> Self {
        Self {
            reply: emit_block(ctx.command_number, message, false),
            notifications: Vec::new(),
            issuer_only: Vec::new(),
            layout_changed: None,
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

    /// Queue the `%layout-change` window.
    fn with_layout(mut self, window_id: WindowId) -> Self {
        self.layout_changed = Some(window_id);
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
    if crate::mux::dispatch::PANIC_ON_COMMAND.load(std::sync::atomic::Ordering::Relaxed) {
        panic!("injected dispatcher panic (QA-113)");
    }
    let mutates = command.mutates();
    let outcome = match command {
        MuxCommand::NewSession { name, env } => cmd_new_session(ctx, name, env),
        MuxCommand::ListPanes => cmd_list_panes(ctx),
        MuxCommand::ListAgents => cmd_list_agents(ctx),
        MuxCommand::SendKeys { pane, keys } => cmd_send_keys(ctx, pane, &keys),
        MuxCommand::RefreshClient { pane, size } => cmd_refresh_client(ctx, pane, size),
        MuxCommand::KillPane { pane } => cmd_kill_pane(ctx, pane),
        MuxCommand::SplitWindow {
            pane,
            direction,
            percent,
            start_dir,
        } => cmd_split_window(ctx, pane, direction, percent, start_dir.as_deref()),
        MuxCommand::SelectPane { pane, title } => cmd_select_pane(ctx, pane, title),
        MuxCommand::PaneTitle { pane } => cmd_pane_title(ctx, pane),
        MuxCommand::PaneInfo { pane } => cmd_pane_info(ctx, pane),
        MuxCommand::ResizePane { pane, adjustment } => cmd_resize_pane(ctx, pane, adjustment),
        MuxCommand::SwapPanes { target, source } => cmd_swap_panes(ctx, target, source),
        MuxCommand::NewWindow {
            session,
            name,
            start_dir,
        } => cmd_new_window(ctx, session, name, start_dir.as_deref()),
        MuxCommand::SelectWindow { window } => cmd_select_window(ctx, window),
        MuxCommand::KillWindow { window } => cmd_kill_window(ctx, window),
        MuxCommand::RenameWindow { window, name } => cmd_rename_window(ctx, window, name),
        MuxCommand::ListWindows => cmd_list_windows(ctx),
        MuxCommand::ListSessions => cmd_list_sessions(ctx),
        MuxCommand::KillServer => cmd_kill_server(ctx),
        MuxCommand::CapturePane {
            pane,
            start_line,
            end_line,
            escape,
        } => cmd_capture_pane(ctx, pane, start_line, end_line, escape),
        MuxCommand::SetBuffer { content } => cmd_set_buffer(ctx, content),
        MuxCommand::SetEnvironment {
            session,
            name,
            value,
        } => cmd_set_environment(ctx, session, &name, value.as_deref()),
        MuxCommand::ShowBuffer => cmd_show_buffer(ctx),
        MuxCommand::PasteBuffer { pane } => cmd_paste_buffer(ctx, pane),
        MuxCommand::Version => cmd_version(ctx),
    };

    if let Some(window_id) = outcome.layout_changed {
        broadcast_layout_change(ctx.tree, ctx.clients, window_id);
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
            // Capture under the lock (cheap clones of already-materialized
            // state), then hand off — the worker serializes and fsyncs off
            // the lock (ARC-003). A send fails only if the worker is gone;
            // the shutdown save is the durability backstop.
            let state = ctx.tree.lock().to_persist_state();
            let _ = tx.send((SaveOrigin::Command, state));
        }
    }
    outcome.reply
}

fn cmd_new_session(ctx: &Ctx<'_>, name: Option<String>, env: Vec<(String, String)>) -> Outcome {
    let name = name.unwrap_or_else(|| "0".to_string());
    let outcome = {
        let mut guard = ctx.tree.lock();
        match guard.new_session_with_env(
            &name,
            DEFAULT_COLS,
            DEFAULT_ROWS,
            env.into_iter().collect(),
        ) {
            Ok(session_id) => {
                // Wire every pane in the new session to push its output.
                let window_ids = guard
                    .session(session_id)
                    .map(|s| s.windows.clone())
                    .unwrap_or_default();
                let pane_ids: Vec<_> = window_ids
                    .iter()
                    .filter_map(|w| guard.window(*w))
                    .flat_map(|w| w.panes())
                    .collect();
                for pane_id in pane_ids {
                    if let Some(pane) = guard.pane_mut(pane_id) {
                        pane.on_output(pane_output_sink(ctx.clients, pane_id));
                    }
                }
                Ok((session_id, window_ids))
            }
            Err(err) => Err(err),
        }
    };
    match outcome {
        Ok((session_id, window_ids)) => {
            let mut result = Outcome::ok(ctx, &session_id.to_string());
            for window in window_ids {
                result = result.notifying(TmuxNotification::WindowAdd {
                    window_id: window.to_string(),
                });
            }
            result.telling_issuer(TmuxNotification::SessionChanged {
                session_id: session_id.to_string(),
                name,
            })
        }
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

fn cmd_list_panes(ctx: &Ctx<'_>) -> Outcome {
    // Wire contract: list-panes replies one line per pane, globally, each
    // just the pane id (`%N`). Geometry arrives via %layout-change
    // pushes; there is no -F (Phase 4 T4.E decision — push covers what
    // the -F polling fallback existed for).
    let guard = ctx.tree.lock();
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
    Outcome::ok(ctx, &body)
}

fn cmd_list_agents(ctx: &Ctx<'_>) -> Outcome {
    // Wire contract: list-agents is the roster — one line per pane a
    // hook has CLAIMED or a pattern has MATCHED, `%N <agent> <state>
    // <source>` with source `hook` or `scrape` (T5.4 + the scrape
    // tier's provenance rule: a consumer must tell a claim from a
    // guess), plus an optional trailing blocked-reason column (the
    // rest of the line; whitespace-collapsed at the endpoint). Panes
    // without either are absent outright: `unknown` means no hook ever
    // reported and no rule ever matched, never "idle" (the Phase 5
    // ruling). Fixed shape, no -F — the T4.E decision.
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
            // The blocked reason rides as the rest of the line —
            // already whitespace-collapsed by the endpoint, so it
            // cannot break the one-line-per-pane shape. Absent
            // message, four tokens exactly.
            let reason = pane.metadata().get("agent_message");
            let entry = match reason {
                Some(reason) => format!("{agent} {state} {source} {reason}"),
                None => format!("{agent} {state} {source}"),
            };
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

fn cmd_send_keys(ctx: &Ctx<'_>, pane: Target<PaneId>, keys: &[u8]) -> Outcome {
    let mut guard = ctx.tree.lock();
    let pane = match guard.resolve_pane_target(pane) {
        Ok(id) => id,
        Err(err) => return Outcome::err(ctx, &err.to_string()),
    };
    match guard.pane_mut(pane) {
        Some(target) => match target.write(keys) {
            Ok(()) => Outcome::ok(ctx, ""),
            Err(err) => Outcome::err(ctx, &err.to_string()),
        },
        None => Outcome::err(ctx, &format!("no such pane: {pane}")),
    }
}

fn cmd_refresh_client(ctx: &Ctx<'_>, pane: Target<PaneId>, size: Option<(u16, u16)>) -> Outcome {
    let pane = {
        let guard = ctx.tree.lock();
        match guard.resolve_pane_target(pane) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    match size {
        // The window-size policy's input (T4.C): a client's renderer
        // reports its grid size, the pane's window is resized to it, and
        // every pane terminal re-fits to the re-divided geometry —
        // followed by a %layout-change so clients re-render.
        // Latest report wins (par-mux.md Phase 4 decision).
        Some((cols, rows)) => {
            let outcome = {
                let mut guard = ctx.tree.lock();
                match guard.window_of_pane(pane) {
                    Some(window_id) => guard
                        .resize_window(window_id, cols, rows)
                        .map(|()| window_id),
                    None => Err(MuxError::NoSuchPane(pane)),
                }
            };
            match outcome {
                Ok(window_id) => Outcome::ok(ctx, "").with_layout(window_id),
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
    let outcome = ctx.tree.lock().kill_pane(pane);
    match outcome {
        Ok(window_id) => {
            // A window whose last pane was killed is REMOVED by
            // tree.kill_pane (an emptied session goes with it) — its
            // clients learn that through %window-close, the same
            // notification kill-window sends. A surviving window keeps
            // an active pane to name, and gets the layout push.
            if let Some(active) = ctx.tree.lock().window(window_id).map(|w| w.active) {
                Outcome::ok(ctx, "").with_layout(window_id).notifying(
                    TmuxNotification::WindowPaneChanged {
                        window_id: window_id.to_string(),
                        pane_id: active.to_string(),
                    },
                )
            } else {
                Outcome::ok(ctx, "").notifying(TmuxNotification::WindowClose {
                    window_id: window_id.to_string(),
                })
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
    pane: Target<PaneId>,
    direction: SplitDirection,
    percent: u32,
    start_dir: Option<&str>,
) -> Outcome {
    let (cwd, note) = resolve_start_dir(start_dir);
    let outcome = {
        let mut guard = ctx.tree.lock();
        let pane = match guard.resolve_pane_target(pane) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        };
        let split = guard.split_pane_in_window(
            pane,
            direction,
            percent as f32 / 100.0,
            None,
            cwd.as_deref(),
        );
        // Wire the new pane's output to the clients, as new-session and
        // new-window do for theirs. Without it the pane's PTY still feeds
        // the daemon grid (capture-pane shows it) but no %output line ever
        // leaves, so every client renders a blank split pane.
        if let Ok((new_pane, _)) = &split {
            if let Some(created) = guard.pane_mut(*new_pane) {
                created.on_output(pane_output_sink(ctx.clients, *new_pane));
                if let Some(note) = &note {
                    // Same visibility rule as a restore's gone cwd: the
                    // pane says where it landed instead of silently
                    // starting elsewhere.
                    created.terminal().write().process(note.as_bytes());
                }
            }
        }
        split
    };
    match outcome {
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
    // Wire contract: one line, `%N @W COLSxROWS` — the pane's window and
    // its terminal's current grid size.
    let guard = ctx.tree.lock();
    let pane = match guard.resolve_pane_target(pane) {
        Ok(id) => id,
        Err(err) => return Outcome::err(ctx, &err.to_string()),
    };
    let (Some(target), Some(window)) = (guard.pane(pane), guard.window_of_pane(pane)) else {
        return Outcome::err(ctx, &MuxError::NoSuchPane(pane).to_string());
    };
    let (cols, rows) = target.terminal().read().size();
    Outcome::ok(ctx, &format!("{pane} {window} {cols}x{rows}"))
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

fn cmd_new_window(
    ctx: &Ctx<'_>,
    session: Option<Target<SessionId>>,
    name: Option<String>,
    start_dir: Option<&str>,
) -> Outcome {
    let name = name.unwrap_or_else(|| "0".to_string());
    let (cwd, note) = resolve_start_dir(start_dir);
    let outcome = {
        let mut guard = ctx.tree.lock();
        // Bare `new-window` targets the most-recently-created
        // session — ids are monotonic and the registry keeps
        // insertion order, so the last entry is the newest.
        let Some(session) = session.or_else(|| guard.sessions().last().copied().map(Target::Id))
        else {
            return Outcome::err(ctx, "no sessions exist");
        };
        let session = match guard.resolve_session_target(session) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        };
        guard
            .new_window_with_cwd(session, &name, DEFAULT_COLS, DEFAULT_ROWS, cwd.as_deref())
            .inspect(|&window_id| {
                // Wire the new window's pane the same way new-session does.
                let pane_ids = guard
                    .window(window_id)
                    .map(|w| w.panes())
                    .unwrap_or_default();
                for pane_id in pane_ids {
                    if let Some(pane) = guard.pane_mut(pane_id) {
                        pane.on_output(pane_output_sink(ctx.clients, pane_id));
                        if let Some(note) = &note {
                            // Same visibility rule as a restore's gone cwd.
                            pane.terminal().write().process(note.as_bytes());
                        }
                    }
                }
            })
    };
    match outcome {
        Ok(window_id) => {
            Outcome::ok(ctx, &window_id.to_string()).notifying(TmuxNotification::WindowAdd {
                window_id: window_id.to_string(),
            })
        }
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

fn cmd_select_window(ctx: &Ctx<'_>, window: Target<WindowId>) -> Outcome {
    let outcome = {
        let mut guard = ctx.tree.lock();
        let window = match guard.resolve_window_target(window) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        };
        guard
            .select_window(window)
            .map(|()| guard.window(window).map(|w| w.active))
            .map(|active| (window, active))
    };
    match outcome {
        Ok((window, active)) => {
            let mut result = Outcome::ok(ctx, "");
            if let Some(pane) = active {
                result = result.notifying(TmuxNotification::WindowPaneChanged {
                    window_id: window.to_string(),
                    pane_id: pane.to_string(),
                });
            }
            result
        }
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

fn cmd_kill_window(ctx: &Ctx<'_>, window: Target<WindowId>) -> Outcome {
    let window = {
        let guard = ctx.tree.lock();
        match guard.resolve_window_target(window) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    match ctx.tree.lock().kill_window(window) {
        Ok(()) => Outcome::ok(ctx, "").notifying(TmuxNotification::WindowClose {
            window_id: window.to_string(),
        }),
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
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

fn cmd_list_windows(ctx: &Ctx<'_>) -> Outcome {
    // Wire contract: list-windows replies one line per window,
    // globally, as `@N: name`.
    let guard = ctx.tree.lock();
    let body = guard
        .sessions()
        .iter()
        .filter_map(|s| guard.session(*s))
        .flat_map(|s| s.windows.clone())
        .filter_map(|w| guard.window(w))
        .map(|w| format!("{}: {}", w.id, w.name))
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

fn cmd_list_sessions(ctx: &Ctx<'_>) -> Outcome {
    // Wire contract: list-sessions replies one line per session as
    // `$N: name`.
    let guard = ctx.tree.lock();
    let body = guard
        .sessions()
        .iter()
        .filter_map(|s| guard.session(*s))
        .map(|s| format!("{}: {}", s.id, s.name))
        .collect::<Vec<_>>()
        .join("\n");
    Outcome::ok(ctx, &body)
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
    let mut guard = ctx.tree.lock();
    let pane = match guard.resolve_pane_target(pane) {
        Ok(id) => id,
        Err(err) => return Outcome::err(ctx, &err.to_string()),
    };
    let Some(content) = guard.get_buffer(DEFAULT_BUFFER).map(str::to_string) else {
        return Outcome::err(ctx, "no buffers");
    };
    match guard.pane_mut(pane) {
        Some(target) => match target.write(content.as_bytes()) {
            Ok(()) => Outcome::ok(ctx, ""),
            Err(err) => Outcome::err(ctx, &err.to_string()),
        },
        None => Outcome::err(ctx, &format!("no such pane: {pane}")),
    }
}

/// Wire contract: the reply body is exactly one line — the daemon's
/// [`build_stamp`](crate::mux::build_stamp). Tree-free by design; a stale
/// daemon must still answer it, so nothing here may depend on session
/// state that a long-lived daemon could have torn down.
fn cmd_version(ctx: &Ctx<'_>) -> Outcome {
    Outcome::ok(ctx, crate::mux::build_stamp())
}

#[cfg(test)]
mod tests {
    use super::resolve_start_dir;

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
}
