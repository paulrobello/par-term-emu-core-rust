//! The shared par-mux attach client connection layer.
//!
//! [`AttachConn`] is the seam both attach phases build on: it owns the
//! [`MuxClient`] (whose reader thread already splits command replies from
//! pushed notifications), runs the attach handshake in the documented
//! client-contract order, and exposes the push stream as events. Neither
//! phase re-opens the wire.

use crate::mux::build_stamp;
use crate::mux::client::{MuxClient, Reply};
use crate::tmux_control::TmuxNotification;
use std::io;
use std::path::Path;
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::time::Duration;

/// Everything one attach session knows about its daemon connection.
pub struct AttachConn {
    client: MuxClient,
    /// The daemon's `version` reply, when it answered one.
    daemon_stamp: Option<String>,
    /// Feature tokens from `list-commands`' `features …` line; empty when
    /// the daemon predates the command (assume no features).
    daemon_features: Vec<String>,
    warnings: HandshakeWarnings,
}

/// Handshake findings for the caller to surface, per the client contract.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HandshakeWarnings {
    /// The daemon's stamp differs from this client's in a way the contract
    /// says to warn about (pre-formatted for eprintln!).
    pub stamp_mismatch: Option<String>,
    /// `list-commands` came back as an unknown-command error: a pre-feature
    /// daemon. Attach proceeds assuming no features.
    pub pre_feature_daemon: bool,
}

/// One event on the client side of a control connection. A channel error of
/// `Disconnected` (recv family) is the connection-ended signal — the daemon
/// closed the socket, whether by `%exit` shutdown, eviction, or process
/// death.
pub type AttachEvent = TmuxNotification;

impl AttachConn {
    /// Connect to the daemon already listening at `path` and run the attach
    /// handshake. Never spawns a daemon: a connect error means no live
    /// server owns the path, and the caller reports it with the standard
    /// "no daemon running on <path>" message.
    pub fn connect(path: &Path) -> io::Result<Self> {
        let client = MuxClient::connect(path)?;
        let mut conn = Self {
            client,
            daemon_stamp: None,
            daemon_features: Vec::new(),
            warnings: HandshakeWarnings::default(),
        };
        conn.run_handshake()?;
        Ok(conn)
    }

    /// The attach handshake, in the documented client-contract order
    /// (docs/MUX.md, `version` / `list-commands` / `set-client-colors` /
    /// `refresh-client -p` bullets):
    ///
    /// 1. `version` — stamp comparison, warn-only on mismatch; quiet when
    ///    the stamps match or only `+unknown` identities are comparable.
    /// 2. `list-commands` — capability discovery; an unknown-command error
    ///    means a pre-feature daemon (assume no features).
    /// 3. `set-client-colors -f/-b` — host theme (dark default for now).
    /// 4. `refresh-client -C WxH -p WxH` — renderer grid and cell pixels.
    ///
    /// The first control command (step 1) also registers this connection,
    /// so the registration replay (`%pane-exited` per held pane,
    /// `%layout-change` per zoomed window) is queued on the push stream by
    /// the time this returns — or lands moments later; it is broadcast, not
    /// reply. The caller drains it through [`Self::drain_pending_events`].
    fn run_handshake(&mut self) -> io::Result<()> {
        // 1. Version stamp. Both sides read crate::mux::build_stamp() — the
        //    daemon serves its own compile's value.
        let served = match self.client.send_checked("version") {
            Ok(reply) if reply.ok => Some(reply.body.first().cloned().unwrap_or_default()),
            // A daemon that refuses `version` is broken in a way the rest
            // of the handshake surfaces; proceed and let it answer.
            _ => None,
        };
        if let Some(served) = &served {
            if !stamp_mismatch_is_quiet(build_stamp(), served) {
                self.warnings.stamp_mismatch = Some(format!(
                    "par-mux: daemon stamp {served} differs from client {}",
                    build_stamp()
                ));
            }
        }
        self.daemon_stamp = served;

        // 2. Capability discovery.
        match self.client.send_checked("list-commands") {
            Ok(reply) if reply.ok => {
                self.daemon_features = parse_feature_tokens(&reply.body);
            }
            // Pre-feature daemon: the unknown-command %error.
            _ => self.warnings.pre_feature_daemon = true,
        }

        // 3. Client theme colors (contract: send on attach and on theme
        //    change; dark default when the host terminal is not probed yet).
        let (fg, bg) = host_colors();
        self.client
            .send_checked(&format!("set-client-colors -f {fg} -b {bg}"))
            .ok();

        // 4. Renderer size + cell pixels. Sent without -t: the daemon-side
        //    parser requires -t today, so current daemons answer %error to
        //    exactly this shape; the size report is still sent (the contract
        //    form) and a failure is tolerated — the pane grid stays at the
        //    daemon's default until the daemon learns the -t-less form.
        let (cols, rows) = terminal_grid();
        let (cw, ch) = cell_pixels();
        let _ = self
            .client
            .send_checked(&format!("refresh-client -C {cols}x{rows} -p {cw}x{ch}"));

        Ok(())
    }

    /// The handshake's findings, for the caller to surface.
    pub fn warnings(&self) -> &HandshakeWarnings {
        &self.warnings
    }

    /// The daemon's build stamp, when the handshake received one.
    pub fn daemon_stamp(&self) -> Option<&str> {
        self.daemon_stamp.as_deref()
    }

    /// Feature tokens the daemon announced (empty for a pre-feature daemon).
    pub fn daemon_features(&self) -> &[String] {
        &self.daemon_features
    }

    /// True when the daemon advertises `token` among its features.
    pub fn has_feature(&self, token: &str) -> bool {
        self.daemon_features.iter().any(|f| f == token)
    }

    /// Send one control command and take its reply block — the query/mutate
    /// path both attach phases use beyond the handshake.
    pub fn send_checked(&mut self, command: &str) -> io::Result<Reply> {
        self.client.send_checked(command)
    }

    /// The push stream: everything the daemon sends that is not a command
    /// reply, already parsed. `Err(Disconnected)` (recv family) is the
    /// connection-ended signal.
    pub fn notifications(&self) -> &Receiver<AttachEvent> {
        self.client.notifications()
    }

    /// Non-blocking snapshot: next pushed notification, or an error that is
    /// `Disconnected` once the daemon closed the connection.
    pub fn try_recv(&self) -> Result<AttachEvent, TryRecvError> {
        self.client.notifications().try_recv()
    }

    /// Drain everything currently queued — the registration replay arrives
    /// this way right after the handshake — newest last.
    pub fn drain_pending_events(&self) -> Vec<AttachEvent> {
        let mut out = Vec::new();
        while let Ok(event) = self.client.notifications().try_recv() {
            out.push(event);
        }
        out
    }

    /// Block for the next pushed notification up to `timeout`, so an idle
    /// loop can poll other sources (stdin, resize) between daemon pushes.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<AttachEvent, RecvTimeoutError> {
        self.client.notifications().recv_timeout(timeout)
    }

    /// Block for the next pushed notification. `None` once the daemon
    /// closed the connection.
    pub fn recv(&self) -> Option<AttachEvent> {
        self.client.notifications().recv().ok()
    }
}

/// The version-stamp warn/quiet rule (docs/MUX.md `version` bullet): quiet
/// when the stamps are equal, and quiet when the version prefixes match and
/// either side is `+unknown` — a same-version mismatch is then unprovable,
/// so clients stay quiet rather than cry wolf. Any other difference warns.
pub(crate) fn stamp_mismatch_is_quiet(mine: &str, served: &str) -> bool {
    if mine == served {
        return true;
    }
    let my_version = mine.split('+').next().unwrap_or_default();
    let served_version = served.split('+').next().unwrap_or_default();
    my_version == served_version && (mine.ends_with("+unknown") || served.ends_with("+unknown"))
}

/// `list-commands` reply -> the daemon-level feature tokens: each
/// `features <token> …` line's tokens. Unknown line shapes are ignored per
/// the contract ("a client must ignore unknown tokens and lines"). A
/// pre-feature daemon never reaches this — its reply is an `%error` — and
/// an empty feature list is a legitimate outcome.
pub(crate) fn parse_feature_tokens(body: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for line in body {
        if let Some(rest) = line.strip_prefix("features ") {
            out.extend(rest.split_whitespace().map(str::to_owned));
        }
    }
    out
}

/// The host terminal theme as `(fg, bg)` six-hex-digit strings, dark default
/// per channel (contract: "dark default"). TODO(attach Phase A): probe with
/// OSC 10/11 (`]10;?BEL` / `]11;?BEL`) in raw mode and parse the
/// `rgb:RRRR/GGGG/BBBB` replies; the probe needs the same raw-mode stdin
/// reader Phase A's key pump owns, so it lands with that card.
fn host_colors() -> (&'static str, &'static str) {
    ("ffffff", "000000")
}

/// The host terminal's per-cell pixel size. TODO(attach Phase A): XTWINOPS
/// `CSI 16 t` probe with the same raw reader; until then the 10x20
/// construction default the daemon itself assumes.
fn cell_pixels() -> (u16, u16) {
    (10, 20)
}

/// The host terminal's character-cell grid size, `(cols, rows)`, falling
/// back to 80x24 when stdin is not a tty (tests, pipes). A tty can still
/// report a zero axis before its first resize event (ConPTY does until the
/// init query is answered), which the daemon's size grammar rejects — the
/// fallback covers that too.
pub(crate) fn terminal_grid() -> (u16, u16) {
    match crossterm::terminal::size() {
        Ok((cols, rows)) if cols > 0 && rows > 0 => (cols, rows),
        _ => (80, 24),
    }
}
