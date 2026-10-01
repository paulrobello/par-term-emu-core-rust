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

use crate::mux::command::{MuxCommand, ResizeAdjustment, SendKeysPayload};
use crate::mux::emit::{emit, emit_block};
use crate::mux::foreground::ProcessTable;
use crate::mux::ids::{PaneId, SessionId, Target, WindowId};
use crate::mux::layout::SplitDirection;
use crate::mux::pane::{MuxError, OutputSink, PaneFactory};
use crate::mux::persist::{PersistState, SaveOrigin};
use crate::mux::server::{
    broadcast_layout_change, broadcast_notification, capture_range, pane_output_sink, Clients,
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
        MuxCommand::NewSession { name, env } => cmd_new_session(ctx, name, env),
        MuxCommand::ListPanes => cmd_list_panes(ctx),
        MuxCommand::ListAgents => cmd_list_agents(ctx),
        MuxCommand::SendKeys { pane, keys } => cmd_send_keys(ctx, pane, &keys),
        MuxCommand::RefreshClient {
            pane,
            size,
            cell_pixels,
        } => cmd_refresh_client(ctx, pane, size, cell_pixels),
        MuxCommand::KillPane { pane } => cmd_kill_pane(ctx, pane),
        MuxCommand::SplitWindow {
            pane,
            direction,
            percent,
            before,
            start_dir,
        } => cmd_split_window(ctx, pane, direction, percent, before, start_dir.as_deref()),
        MuxCommand::SelectPane { pane, title } => cmd_select_pane(ctx, pane, title),
        MuxCommand::PaneTitle { pane } => cmd_pane_title(ctx, pane),
        MuxCommand::PaneInfo { pane } => cmd_pane_info(ctx, pane),
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
            session,
            name,
            start_dir,
        } => cmd_new_window(ctx, session, name, start_dir.as_deref()),
        MuxCommand::SelectWindow { window } => cmd_select_window(ctx, window),
        MuxCommand::KillWindow { window } => cmd_kill_window(ctx, window),
        MuxCommand::RenameSession { session, name } => cmd_rename_session(ctx, session, name),
        MuxCommand::KillSession { session } => cmd_kill_session(ctx, session),
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
        MuxCommand::SetClientColors { fg, bg } => cmd_set_client_colors(ctx, fg, bg),
        MuxCommand::SetEnvironment {
            session,
            name,
            value,
        } => cmd_set_environment(ctx, session, &name, value.as_deref()),
        MuxCommand::ShowBuffer => cmd_show_buffer(ctx),
        MuxCommand::PasteBuffer { pane } => cmd_paste_buffer(ctx, pane),
        MuxCommand::Version => cmd_version(ctx),
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

fn cmd_new_session(ctx: &Ctx<'_>, name: Option<String>, env: Vec<(String, String)>) -> Outcome {
    let name = name.unwrap_or_else(|| "0".to_string());
    let env: std::collections::BTreeMap<String, String> = env.into_iter().collect();
    // Two-phase spawn (ARC-022): reserve ids under the lock, run the
    // fork/exec OFF it — a slow spawn must not stall every other client —
    // then re-lock to insert and wire. The in-flight session is invisible
    // until the insert lands.
    let (plan, factory) = {
        let mut guard = ctx.tree.lock();
        (
            guard.begin_session(&name, DEFAULT_COLS, DEFAULT_ROWS, &env),
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
                result = result.notifying(TmuxNotification::WindowAdd {
                    window_id: window.to_string(),
                });
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
    pane: Target<PaneId>,
    size: Option<(u16, u16)>,
    cell_pixels: Option<(u16, u16)>,
) -> Outcome {
    let pane = {
        let guard = ctx.tree.lock();
        match guard.resolve_pane_target(pane) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    // `-p` is applied first and independently of `-C`: the cell size is
    // daemon-wide state every later re-fit reads (sync_pane_sizes), so a
    // combined report lands the pixels even when the grid resize below
    // fails on a dead pane, and a pixels-only report re-fits at the
    // current grid through the same sync path.
    if let Some((cell_w, cell_h)) = cell_pixels {
        ctx.tree.lock().set_client_cell_pixels(cell_w, cell_h);
    }
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
        Ok((window_id, removed_session)) => {
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
                let mut outcome = Outcome::ok(ctx, "").notifying(TmuxNotification::WindowClose {
                    window_id: window_id.to_string(),
                });
                if removed_session.is_some() {
                    // The window's closure emptied the session: the set of
                    // sessions changed, and tmux says so argument-less.
                    outcome = outcome.notifying(TmuxNotification::SessionsChanged);
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
    pane: Target<PaneId>,
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
        let pane = match guard.resolve_pane_target(pane) {
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
    match ctx.tree.lock().break_pane(source, &name) {
        Ok((window_id, source_window, source_closed)) => {
            // The new window is announced like new-window's; the source's
            // frame only exists while the source does.
            let mut outcome =
                Outcome::ok(ctx, &window_id.to_string()).notifying(TmuxNotification::WindowAdd {
                    window_id: window_id.to_string(),
                });
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
    match ctx
        .tree
        .lock()
        .join_pane(source, target, direction, percent as f32 / 100.0)
    {
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
    session: Option<Target<SessionId>>,
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
        let Some(session) = session.or_else(|| guard.sessions().last().copied().map(Target::Id))
        else {
            return Outcome::err(ctx, "no sessions exist");
        };
        let session = match guard.resolve_session_target(session) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        };
        match guard.begin_window(session, &name, DEFAULT_COLS, DEFAULT_ROWS, cwd.as_deref()) {
            Ok(plan) => (plan, guard.factory()),
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    match spawn_and_wire(ctx, &*factory, plan, None, note.as_deref()) {
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
        Ok(removed_session) => {
            let mut outcome = Outcome::ok(ctx, "").notifying(TmuxNotification::WindowClose {
                window_id: window.to_string(),
            });
            if removed_session.is_some() {
                // The cascade reached the session — same argument-less
                // cue kill-pane's cascade sends, so one handler covers both.
                outcome = outcome.notifying(TmuxNotification::SessionsChanged);
            }
            outcome
        }
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
    let session = {
        let guard = ctx.tree.lock();
        match guard.resolve_session_target(session) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        }
    };
    match ctx.tree.lock().kill_session(session) {
        Ok(killed_windows) => {
            // The same line order kill-window's cascade produces: one
            // %window-close per killed window, then the session-set cue.
            let mut outcome = Outcome::ok(ctx, "");
            for window in &killed_windows {
                outcome = outcome.notifying(TmuxNotification::WindowClose {
                    window_id: window.to_string(),
                });
            }
            outcome.notifying(TmuxNotification::SessionsChanged)
        }
        Err(err) => Outcome::err(ctx, &err.to_string()),
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
        let Some(content) = guard.get_buffer(DEFAULT_BUFFER).map(str::to_string) else {
            return Outcome::err(ctx, "no buffers");
        };
        let Some(target) = guard.pane(pane) else {
            return Outcome::err(ctx, &format!("no such pane: {pane}"));
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
}
