//! Named-layout handlers: save-layout, load-layout, list-layouts,
//! kill-layout. The store itself lives in `mux::persist`; `load-layout` is
//! the one handler that replaces the live tree.

use super::*;
use crate::mux::persist;
use std::path::PathBuf;

/// The layouts store this daemon serves: beside the state file when the
/// server runs persisting, so `--state-dir` relocates the store; the
/// platform default for embedders and tests that dispatch without a
/// persisting server.
fn layouts_dir(ctx: &Ctx<'_>) -> PathBuf {
    ctx.layouts_dir
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(persist::layouts_dir)
}

/// The layout group's router; `route_command` sends only this group's
/// variants here.
pub(super) fn route_layout_command(ctx: &Ctx<'_>, command: MuxCommand) -> Outcome {
    match command {
        MuxCommand::SaveLayout { name } => cmd_save_layout(ctx, &name),
        MuxCommand::LoadLayout { name, force } => cmd_load_layout(ctx, &name, force),
        MuxCommand::ListLayouts => Outcome::ok(
            ctx,
            &persist::list_layouts_in_dir(&layouts_dir(ctx)).join("\n"),
        ),
        MuxCommand::KillLayout { name } => cmd_kill_layout(ctx, &name),
        other => unreachable!("route_command sent a non-layout command: {other:?}"),
    }
}

/// `save-layout <name>`: capture under the tree lock, write with it
/// released (the file write must not stall every other client).
fn cmd_save_layout(ctx: &Ctx<'_>, name: &str) -> Outcome {
    if !persist::valid_layout_name(name) {
        return Outcome::err(ctx, &format!("invalid layout name: {name}"));
    }
    let capture = ctx.tree.lock().collect_persist_capture();
    let state = capture.capture();
    match persist::save_layout_state_in_dir(&layouts_dir(ctx), &state, name) {
        Ok(_) => Outcome::ok(ctx, &format!("layout saved: {name}")),
        Err(err) => Outcome::err(ctx, &err.to_string()),
    }
}

/// A missing layout file is the common mistake (a typo); name the layout
/// instead of surfacing the raw I/O text. A decode failure means the file
/// is not a layout at all, so name it corrupt instead of quoting serde's
/// encoding text. Other errors keep their Display.
fn layout_error_message(err: &persist::PersistError, name: &str) -> String {
    match err {
        persist::PersistError::Io(io) if io.kind() == std::io::ErrorKind::NotFound => {
            format!("no such layout: {name}")
        }
        persist::PersistError::Serialize(_) => format!("corrupt layout: {name}"),
        other => other.to_string(),
    }
}

fn cmd_kill_layout(ctx: &Ctx<'_>, name: &str) -> Outcome {
    match persist::kill_layout_in_dir(&layouts_dir(ctx), name) {
        Ok(()) => Outcome::ok(ctx, &format!("layout killed: {name}")),
        Err(err) => Outcome::err(ctx, &layout_error_message(&err, name)),
    }
}

/// `load-layout [-f] <name>`: replace the live tree with the stored layout.
/// Refuses while any pane is alive unless `force`, because the swap kills
/// their processes.
fn cmd_load_layout(ctx: &Ctx<'_>, name: &str, force: bool) -> Outcome {
    let state = match persist::load_layout_in_dir(&layouts_dir(ctx), name) {
        Ok(state) => state,
        Err(err) => return Outcome::err(ctx, &layout_error_message(&err, name)),
    };
    // Lock order: the config lock is taken and released on its own, then
    // the tree lock. The two are never held together here, so no ordering
    // against the other handlers can invert.
    let (remain_on_exit, agent_append) = match ctx.config {
        Some(config) => {
            let config = config.lock();
            (config.remain_on_exit, config.agent_append.clone())
        }
        None => (false, crate::mux::config::AgentAppend::default()),
    };
    let factory = {
        let guard = ctx.tree.lock();
        let live = guard.panes.values().filter(|pane| !pane.dead()).count();
        if live > 0 && !force {
            return Outcome::err(
                ctx,
                &format!("load-layout: {live} live pane(s) would be killed; pass -f to replace"),
            );
        }
        guard.factory()
    };
    // Spawning the stored panes runs off the tree lock.
    let mut fresh = match MuxTree::from_persist_state_with_policy(
        &state,
        Box::new(SharedFactory(factory)),
        remain_on_exit,
        &agent_append,
    ) {
        Ok(tree) => tree,
        Err(err) => return Outcome::err(ctx, &err.to_string()),
    };
    for pane_id in fresh.panes.keys().copied().collect::<Vec<_>>() {
        if let Some(pane) = fresh.pane_mut(pane_id) {
            pane.on_output_sink(OutputSink(Arc::new(pane_output_sink(
                ctx.clients,
                pane_id,
                ctx.output_epoch,
            ))));
        }
    }
    let old = {
        let mut guard = ctx.tree.lock();
        // Per-connection and per-daemon state the layout file does not
        // carry stays with the daemon.
        fresh.client_cell_pixels = guard.client_cell_pixels;
        fresh.client_fg = guard.client_fg;
        fresh.client_bg = guard.client_bg;
        fresh.client_views = std::mem::take(&mut guard.client_views);
        fresh.streaming_mirrors = std::mem::take(&mut guard.streaming_mirrors);
        fresh.client_focus = std::mem::take(&mut guard.client_focus);
        fresh.scrape_engine = guard.scrape_engine.clone();
        // Silence the outgoing panes so their last bytes cannot reach
        // clients under ids the new tree reuses.
        for pane_id in guard.panes.keys().copied().collect::<Vec<_>>() {
            if let Some(pane) = guard.pane_mut(pane_id) {
                pane.detach_output();
            }
        }
        std::mem::replace(&mut *guard, fresh)
    };
    // The old tree's panes kill their children as they drop. Slow child
    // death (a pane trapping SIGHUP) must not delay the reply past the
    // client's reply timeout, so the drop runs on a detached thread that
    // owns the tree; nothing on this thread references it afterwards.
    std::thread::spawn(move || drop(old));
    Outcome::ok(ctx, &format!("layout loaded: {name}")).notifying(TmuxNotification::SessionsChanged)
}

/// Lets the swap reuse the live tree's `Arc<dyn PaneFactory>` where
/// `from_persist_state_with_policy` wants a `Box`.
struct SharedFactory(Arc<dyn PaneFactory>);

impl PaneFactory for SharedFactory {
    fn create_pane(
        &self,
        id: PaneId,
        cols: u16,
        rows: u16,
        command: Option<&str>,
        context: &crate::mux::pane::SpawnContext<'_>,
    ) -> Result<crate::mux::pane::MuxPane, MuxError> {
        self.0.create_pane(id, cols, rows, command, context)
    }

    fn create_argv_pane(
        &self,
        id: PaneId,
        cols: u16,
        rows: u16,
        argv: &[String],
        context: &crate::mux::pane::SpawnContext<'_>,
    ) -> Result<crate::mux::pane::MuxPane, MuxError> {
        self.0.create_argv_pane(id, cols, rows, argv, context)
    }

    fn create_dead_pane(
        &self,
        id: PaneId,
        cols: u16,
        rows: u16,
        command: Option<&str>,
        exit_code: Option<i32>,
    ) -> Result<crate::mux::pane::MuxPane, MuxError> {
        self.0.create_dead_pane(id, cols, rows, command, exit_code)
    }
}
