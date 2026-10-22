//! Buffer, capture, environment, and daemon-info handlers: capture-pane,
//! set/show/paste-buffer, set-client-colors, set-environment, version,
//! reload-config, list-commands.

use super::*;
use crate::mux::command::{OutputPattern, COMMAND_SCHEMA_JSON, WAIT_OUTPUT_DEFAULT_HISTORY};
use par_term_emu_core::pty_session::UpdateWaiter;
use parking_lot::RwLock;
use std::time::{Duration, Instant};

/// The buffer/capture/daemon-info group's router; `route_command` sends
/// only this group's variants here.
pub(super) fn route_buffer_command(ctx: &Ctx<'_>, command: MuxCommand) -> Outcome {
    match command {
        MuxCommand::CapturePane {
            pane,
            start_line,
            end_line,
            escape,
        } => cmd_capture_pane(ctx, pane, start_line, end_line, escape),
        MuxCommand::WaitOutput {
            pane,
            pattern,
            timeout_ms,
            start_line,
            end_line,
        } => cmd_wait_output(ctx, pane, pattern, timeout_ms, start_line, end_line),
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
        MuxCommand::ShowNotification { title, body } => {
            Outcome::ok(ctx, "").notifying(TmuxNotification::Notification { title, body })
        }
        MuxCommand::ListCommands { schema } => cmd_list_commands(ctx, schema),
        other => unreachable!("route_command sent a non-buffer command: {other:?}"),
    }
}

pub(super) fn cmd_capture_pane(
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
            let body = capture_text(&terminal.read(), start_line, end_line, escape);
            Outcome::ok(ctx, &body)
        }
        None => Outcome::err(ctx, &format!("no such pane: {pane}")),
    }
}

/// The one content source `capture-pane` replies with and `wait-output`
/// matches against: the visible screen, or with a `-S`/`-E` range the
/// composed scrollback + screen trimmed by [`capture_range`].
pub(super) fn capture_text(
    term: &par_term_emu_core::terminal::Terminal,
    start_line: Option<i64>,
    end_line: Option<i64>,
    escape: bool,
) -> String {
    match (start_line, end_line) {
        // Decision 2 stands: no new Terminal API — the default capture
        // reads the pane's visible screen (the active grid, so an
        // alt-screen TUI captures its TUI screen).
        (None, None) => {
            if escape {
                term.export_visible_screen_styled_lines()
            } else {
                term.content()
            }
        }
        (start, end) => {
            // The tmux -S/-E range trim happens here on the composed
            // buffer, not in Terminal.
            let (scrollback, screen) = if escape {
                (
                    term.export_scrollback(par_term_emu_core::terminal::ExportFormat::Ansi, None),
                    term.export_visible_screen_styled_lines(),
                )
            } else {
                (plain_history_tail(term, start, end), term.content())
            };
            capture_range(&scrollback, &screen, start, end)
        }
    }
}

/// The newest-first plain history rows a `-S/-E` range can reach — the
/// shape `export_scrollback(Plain, None)` emits, cut by its tail limit to
/// the deepest negative bound so a range near the screen does not walk the
/// whole scrollback under the terminal's read lock. Only a `None` start
/// reaches all of history, and every caller passing `None` with an `end`
/// gets the full walk as before.
fn plain_history_tail(
    term: &par_term_emu_core::terminal::Terminal,
    start: Option<i64>,
    end: Option<i64>,
) -> String {
    let needed = start.and_then(|start| {
        let deepest = start.min(end.unwrap_or(0)).min(0);
        usize::try_from(deepest.unsigned_abs()).ok()
    });
    term.export_scrollback(par_term_emu_core::terminal::ExportFormat::Plain, needed)
}

/// How long one `wait-output` park lasts before the waiter re-checks the
/// pane's liveness, identity, and the daemon's shutdown flag under a brief
/// tree lock — the same cadence as the connection eviction poll.
const WAIT_OUTPUT_SLICE: Duration = Duration::from_millis(200);

/// The minimum spacing between two `wait-output` content checks. Each
/// check composes the capture under the terminal's read lock, so a pane
/// streaming output is matched at most this often instead of once per PTY
/// read.
const WAIT_OUTPUT_MIN_CHECK: Duration = Duration::from_millis(50);

/// The pane a `wait-output` is parked on: its terminal and the waiter
/// signalled by that terminal's reader thread. A respawn swaps the pane's
/// terminal under the same id, which is why both are re-read on it.
struct WaitWatch {
    terminal: Arc<RwLock<par_term_emu_core::terminal::Terminal>>,
    waiter: UpdateWaiter,
    generation: u64,
}

impl WaitWatch {
    fn of(pane: &crate::mux::pane::MuxPane) -> Self {
        // The generation is read before any content check, so output that
        // lands between the check and the park wakes the park at once.
        Self {
            generation: pane.update_generation(),
            terminal: pane.terminal(),
            waiter: pane.update_waiter(),
        }
    }
}

/// The first line of `text` the pattern matches. Lines are compared with
/// trailing whitespace trimmed: history rows carry the grid's blank padding
/// where screen lines do not, and a `$`-anchored regex must match both.
fn first_match(text: &str, pattern: &WaitPattern) -> Option<String> {
    text.lines()
        .map(str::trim_end)
        .find(|line| match pattern {
            WaitPattern::Substring(needle) => line.contains(needle.as_str()),
            WaitPattern::Regex(re) => re.is_match(line),
        })
        .map(str::to_string)
}

/// [`OutputPattern`] with its regex compiled.
enum WaitPattern {
    Substring(String),
    Regex(regex::Regex),
}

/// `wait-output`: park the issuing connection's own thread until a line of
/// the pane's capture matches. Other clients are served by their own
/// threads, and no lock is held while parked — each check takes only the
/// terminal read lock, each liveness re-check the tree lock briefly. The
/// match is checked before every park, so output already on screen
/// matches at once.
pub(super) fn cmd_wait_output(
    ctx: &Ctx<'_>,
    pane: Target<PaneId>,
    pattern: OutputPattern,
    timeout_ms: Option<u64>,
    start_line: Option<i64>,
    end_line: Option<i64>,
) -> Outcome {
    let pattern = match pattern {
        OutputPattern::Substring(needle) => WaitPattern::Substring(needle),
        OutputPattern::Regex(source) => match regex::Regex::new(&source) {
            Ok(re) => WaitPattern::Regex(re),
            Err(err) => return Outcome::err(ctx, &format!("wait-output: invalid -r: {err}")),
        },
    };
    let started = Instant::now();
    let deadline = timeout_ms.and_then(|ms| started.checked_add(Duration::from_millis(ms)));
    let start_line = Some(start_line.unwrap_or(-WAIT_OUTPUT_DEFAULT_HISTORY));
    let (pane, mut watch) = {
        let guard = ctx.tree.lock();
        let id = match guard.resolve_pane_target(pane) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        };
        match guard.pane(id) {
            Some(target) => (id, WaitWatch::of(target)),
            None => return Outcome::err(ctx, &format!("no such pane: {id}")),
        }
    };
    let mut exited = false;
    loop {
        let last_check = Instant::now();
        let text = capture_text(&watch.terminal.read(), start_line, end_line, false);
        if let Some(line) = first_match(&text, &pattern) {
            return Outcome::ok(ctx, &line);
        }
        if exited {
            return Outcome::err(
                ctx,
                &format!("wait-output: pane {pane} exited without a match"),
            );
        }
        let now = Instant::now();
        if deadline.is_some_and(|deadline| now >= deadline) {
            return Outcome::err(
                ctx,
                &format!(
                    "wait-output: timed out after {}ms without a match on {pane}",
                    timeout_ms.unwrap_or_default()
                ),
            );
        }
        if ctx
            .shutdown
            .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed))
        {
            return Outcome::err(ctx, "wait-output: the daemon is shutting down");
        }
        // A client that hung up mid-wait leaves the broadcast registry on
        // the next push that finds its writer gone (or on eviction), so a
        // busy pane's output ends an orphaned wait instead of leaving it
        // re-capturing until a match. Embedders dispatch without an id.
        if ctx
            .client_id
            .is_some_and(|id| !ctx.clients.lock().iter().any(|(c, ..)| *c == id))
        {
            return Outcome::err(ctx, "wait-output: client disconnected");
        }
        let until_deadline =
            deadline.map_or(WAIT_OUTPUT_SLICE, |d| d.saturating_duration_since(now));
        let park = WAIT_OUTPUT_SLICE.min(until_deadline);
        if let Some(next) = watch.waiter.wait_for_update(watch.generation, park) {
            watch.generation = next;
            let settle = WAIT_OUTPUT_MIN_CHECK
                .saturating_sub(last_check.elapsed())
                .min(deadline.map_or(Duration::MAX, |d| {
                    d.saturating_duration_since(Instant::now())
                }));
            std::thread::sleep(settle);
            continue;
        }
        // A park that ended without new output: the slice ran out, or the
        // child exited (the waiter returns at once then). Re-validate the
        // pane under a brief tree lock.
        let guard = ctx.tree.lock();
        let Some(target) = guard.pane(pane) else {
            return Outcome::err(
                ctx,
                &format!("wait-output: pane {pane} closed without a match"),
            );
        };
        if !Arc::ptr_eq(&target.terminal(), &watch.terminal) {
            watch = WaitWatch::of(target);
            continue;
        }
        // `dead()` alone lags the reaper; the reader's running flag drops
        // first, and the waiter returns immediately from then on.
        exited = !target.is_running() || target.dead();
        watch.generation = target.update_generation();
    }
}

pub(super) fn cmd_set_buffer(ctx: &Ctx<'_>, content: String) -> Outcome {
    ctx.tree.lock().set_buffer(DEFAULT_BUFFER, content);
    Outcome::ok(ctx, "")
}

pub(super) fn cmd_set_client_colors(
    ctx: &Ctx<'_>,
    fg: Option<(u8, u8, u8)>,
    bg: Option<(u8, u8, u8)>,
) -> Outcome {
    let to_color = |(r, g, b)| par_term_emu_core::color::Color::Rgb(r, g, b);
    ctx.tree
        .lock()
        .set_client_colors(fg.map(to_color), bg.map(to_color));
    Outcome::ok(ctx, "")
}

pub(super) fn cmd_set_environment(
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

pub(super) fn cmd_show_buffer(ctx: &Ctx<'_>) -> Outcome {
    let guard = ctx.tree.lock();
    match guard.get_buffer(DEFAULT_BUFFER) {
        Some(content) => Outcome::ok(ctx, content),
        None => Outcome::err(ctx, "no buffers"),
    }
}

pub(super) fn cmd_paste_buffer(ctx: &Ctx<'_>, pane: Target<PaneId>) -> Outcome {
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
pub(super) fn cmd_version(ctx: &Ctx<'_>) -> Outcome {
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
pub(super) fn cmd_reload_config(ctx: &Ctx<'_>) -> Outcome {
    use crate::mux::config::{load_file, reload_report};
    let Some(applied) = ctx.config else {
        // No applied copy: an embedded server without config support, or
        // the dispatch test shim. Honest report, not a fake success.
        return Outcome::err(
            ctx,
            "reload-config: this server has no applied config to reload",
        );
    };
    // The reload re-reads the file the applied copy was resolved from —
    // the path the daemon stamped on it at startup — so an env change
    // mid-run cannot move the file under a running daemon. A hand-built
    // copy (embedders, tests) falls back to the canonical lookup. A
    // PRESENT file that does not parse is an error, not defaults: the
    // user's edit must surface, not vanish. Absent = nothing changed.
    let path = applied
        .lock()
        .config_path
        .clone()
        .or_else(crate::mux::config::config_file_path);
    let Some(path) = path else {
        return Outcome::err(ctx, "reload-config: no config path on this platform");
    };
    let file = match load_file(&path) {
        Ok(Some(file)) => Some(file),
        Ok(None) => None,
        Err(err) => return Outcome::err(ctx, &format!("reload-config: {err}")),
    };
    let report = reload_report(&mut applied.lock(), file.as_ref());
    // Plugin installs and removals take effect with the reload, the same
    // fresh-read the config gets (plugins.rs).
    crate::mux::plugins::reload_engine();
    // Every attached client re-applies its `[client]` settings from the
    // file on this cue, so a settings-dialog change in one client reaches
    // the others. Errors above broadcast nothing: no file was re-read.
    Outcome::ok(ctx, &report).notifying(TmuxNotification::ConfigChanged)
}

/// ENH-037: capability discovery — the sorted command roster with feature
/// tokens, generated from the same `COMMANDS` table the parser dispatches
/// from, so a new command is discoverable the moment its row lands.
pub(super) fn cmd_list_commands(ctx: &Ctx<'_>, schema: bool) -> Outcome {
    if schema {
        Outcome::ok(ctx, COMMAND_SCHEMA_JSON)
    } else {
        Outcome::ok(ctx, &list_commands_body())
    }
}
