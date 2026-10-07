//! Buffer, capture, environment, and daemon-info handlers: capture-pane,
//! set/show/paste-buffer, set-client-colors, set-environment, version,
//! reload-config, list-commands.

use super::*;

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

pub(super) fn cmd_set_buffer(ctx: &Ctx<'_>, content: String) -> Outcome {
    ctx.tree.lock().set_buffer(DEFAULT_BUFFER, content);
    Outcome::ok(ctx, "")
}

pub(super) fn cmd_set_client_colors(
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
pub(super) fn cmd_list_commands(ctx: &Ctx<'_>) -> Outcome {
    Outcome::ok(ctx, &list_commands_body())
}
