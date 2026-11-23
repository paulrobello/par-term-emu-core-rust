//! The render-mode attach entry: connect, seed, pump, and the terminal-state
//! guard that restores the host on every exit path.

use super::sidebar;
use super::{FlushSink as _, StdoutSink, WindowSession};

/// The Phase B render-mode attach session: connect, seed the window's
/// panes from the daemon (layout via a size-report `%layout-change`,
/// per-pane state via the `refresh-client -t` replays), then pump
/// `%output` into the per-pane emulators at frame cadence until detach or
/// the connection ends.
pub(crate) fn run_render_session(
    options: &crate::mux::attach::AttachOptions,
    client: &crate::mux::attach::ResolvedClient,
) -> std::process::ExitCode {
    match render_session_inner(options, client) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("par-mux: attach failed: {err}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn render_session_inner(
    options: &crate::mux::attach::AttachOptions,
    client: &crate::mux::attach::ResolvedClient,
) -> Result<(), String> {
    let path = options.socket_path();
    let no_daemon = || format!("no daemon running on {}", path.display());
    // The session picker runs before the attach connection and the
    // terminal guards exist.
    let target = crate::mux::attach::picker::picked_target_on_host(
        &path,
        options.target.as_deref(),
        client.chords.session_picker,
        client.chords.prefer_default_workspace,
    )
    .map_err(|err| err.unwrap_or_else(no_daemon))?;
    let mut conn = crate::mux::attach::conn::AttachConn::connect(&path).map_err(|_| no_daemon())?;
    if let Some(warning) = &conn.warnings().stamp_mismatch {
        eprintln!("{warning}");
    }

    // The host grid — the renderer's window extent (the window-size
    // policy resizes the daemon's window to it). Raw mode + alternate
    // screen + mouse capture: unlike passthrough, the renderer owns the
    // whole screen and routes mouse events itself. SGR+motion capture
    // (1002 + 1006) is what the host can report; panes that negotiate
    // any-event tracking get drags through the same path. Both guards
    // restore on every exit path, `?` and unwind included. Locals drop in
    // reverse declaration order, so the screen guard (declared second)
    // leaves the alternate screen BEFORE the terminal guard leaves raw
    // mode.
    let (cols, rows) = crate::mux::attach::conn::terminal_grid();
    let terminal_guard = crate::mux::attach::TerminalGuard::enter();
    let _screen_guard = RenderScreenGuard::enter(terminal_guard.entered());

    let mut session = WindowSession::new(cols, rows);
    // The client configuration was resolved once for both modes
    // (`resolve_client`: CLI over file over defaults) before the
    // terminal was touched; the session seeds from it.
    let chords = &client.chords;
    session.render_opts.sidebar_width = chords.sidebar_width;
    // The panel's persisted state (the dragged width and split, the
    // chevron's collapse) overrides the config's defaults; the session
    // writes it back on every drag release and chevron press.
    if let Some(path) = crate::mux::attach::prefs::prefs_path() {
        let prefs = crate::mux::attach::prefs::load(&path);
        if let Some(width) = prefs.sidebar_width {
            session.render_opts.sidebar_width =
                width.clamp(sidebar::SIDEBAR_WIDTH_MIN, sidebar::SIDEBAR_WIDTH_MAX);
        }
        if let Some(split) = prefs.sidebar_split.filter(|s| s.is_finite()) {
            session.renderer.set_sidebar_split(split.clamp(0.1, 0.9));
        }
        let collapsed = prefs.sidebar_collapsed.unwrap_or(false);
        session.sidebar.set_collapsed(collapsed);
        session.renderer.set_sidebar_collapsed(collapsed);
        session.sidebar.set_prefs_path(path);
    }
    session.settings_path = crate::mux::config::config_file_path();
    session.socket = Some(path.clone());
    session.applied_client = client.file.client.clone();
    // `sidebar-on-launch`: the panel width lands on the renderer BEFORE
    // the seed, so the seed's first `refresh-client -C` already reports
    // the reduced grid and the daemon's first division reserves it.
    session.sidebar.set(chords.sidebar_on_launch);
    session.chrome_geometry_changed();
    session.renderer.set_geometry(session.geometry);
    if !session.apply_chords(chords, &client.file) {
        eprintln!(
            "par-mux: [client] border-lines {:?} unknown — valid: unicode, double, heavy, ascii, herdr",
            chords.border_lines
        );
    }
    for warning in crate::mux::config::display_option_warnings(chords) {
        eprintln!("par-mux: {warning}");
    }

    // The host probe: OSC 11 background + XTWINOPS CSI 16 t cell pixels,
    // run with raw mode up and before the pump's stdin reader starts, so
    // the probe is briefly the tty's only reader. The probed bg fills
    // every frame cell; a probe FAILURE leaves the fill terminal-default
    // (no assumed color). A corrected set-client-colors rides the probe
    // so the daemon's theme record follows the host, and probed cell
    // pixels follow as their own `refresh-client -C … -p …` — the
    // handshake already reported the 10x20 construction default and the
    // daemon's client cell-size record is latest-report-wins.
    // With `kitty-graphics` on, the probe also asks the host whether it
    // speaks the kitty graphics protocol; nothing is drawn unless it
    // answers OK (a late answer arrives through the stdin parser).
    let probe = crate::mux::attach::conn::probe_host_terminal(chords.kitty_graphics);
    if let Some(ok) = probe.kitty_graphics {
        session.set_kitty_supported(ok);
    }
    if let Some((r, g, b)) = probe.background {
        session.observe_host_background((r, g, b));
        let _ = conn.send_checked(&format!(
            "set-client-colors -f ffffff -b {r:02x}{g:02x}{b:02x}"
        ));
    }
    if let Some((cw, ch)) = probe.cell_pixels {
        let (cols, rows) = crate::mux::attach::conn::terminal_grid();
        let _ = conn.send_checked(&format!("refresh-client -C {cols}x{rows} -p {cw}x{ch}"));
        session.renderer.set_cell_px((u32::from(cw), u32::from(ch)));
    }
    // Keystrokes the probe consumed get back into the stream instead of
    // being eaten.
    session.stdin_primer = probe.primer;

    let result = session.run(&mut conn, target.as_deref(), &mut StdoutSink);
    session.popup_teardown(&mut conn);
    // Host-side images outlive the alternate screen on some terminals:
    // delete them while the screen guard still holds the screen.
    let teardown = session.kitty.clear();
    if !teardown.is_empty() {
        StdoutSink.write_raw(&teardown);
    }
    result
}

/// RAII owner of the render client's alternate screen, mouse capture,
/// and bracketed paste: entering switches to the alternate screen and
/// enables SGR mouse capture plus DECSET 2004; dropping releases the
/// mouse, leaves the alternate screen, disables 2004, and shows the
/// cursor, ignoring errors. Without a tty (raw mode never came up) it
/// emits nothing. The 2004 request is what makes a tracking host
/// (par-term) deliver every paste as one framed `ESC[200~…ESC[201~` run
/// the scanner can route — without it a multi-line paste reaches panes
/// as raw newlines and pane apps like claude code submit line-by-line
/// (tmux enables 2004 on its outer terminal for the same reason).
pub(crate) struct RenderScreenGuard {
    active: bool,
}

impl RenderScreenGuard {
    /// The bytes `enter` writes on a tty host.
    pub(super) fn enter_sequence() -> &'static str {
        // Mode 2031: the host reports light/dark switches as
        // `CSI ? 997 ; 1|2 n` (the `auto` theme's live signal); a host
        // without it ignores the private mode. Mode 2004: the host
        // frames every paste; a host without it sends pastes raw, as
        // before.
        "\x1b[?2004h\x1b[?2031h"
    }

    /// The bytes [`Drop`] writes on a tty host.
    pub(super) fn exit_sequence() -> &'static str {
        // Restore the pointer before the screen flips back: a shaped
        // pointer (OSC 22) survives the alternate-screen exit on
        // terminals that scope it to the window, and the host shell
        // should not inherit the theme or paste cues.
        "\x1b]22;text\x1b\\\x1b[?2031l\x1b[?2004l"
    }

    pub(super) fn enter(tty: bool) -> Self {
        if tty {
            let _ = crossterm::execute!(
                std::io::stdout(),
                crossterm::terminal::EnterAlternateScreen,
                crossterm::event::EnableMouseCapture,
                crossterm::style::Print(Self::enter_sequence())
            );
        }
        Self { active: tty }
    }
}

impl Drop for RenderScreenGuard {
    fn drop(&mut self) {
        #[cfg(test)]
        screen_guard_drops::record();
        if self.active {
            let _ = crossterm::execute!(
                std::io::stdout(),
                crossterm::style::Print(Self::exit_sequence()),
                crossterm::event::DisableMouseCapture,
                crossterm::terminal::LeaveAlternateScreen,
                crossterm::cursor::Show
            );
        }
    }
}

/// Per-thread count of [`RenderScreenGuard`] drops, so a test can prove
/// the guard restores on an early error return.
#[cfg(test)]
pub(super) mod screen_guard_drops {
    use std::cell::Cell;

    thread_local! {
        static DROPS: Cell<usize> = const { Cell::new(0) };
    }

    pub(crate) fn record() {
        DROPS.with(|d| d.set(d.get() + 1));
    }

    pub(crate) fn count() -> usize {
        DROPS.with(Cell::get)
    }
}
