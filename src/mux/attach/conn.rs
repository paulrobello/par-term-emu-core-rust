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

/// The host terminal theme as `(fg, bg)` six-hex-digit strings, dark
/// default per channel (contract: "dark default"). The renderer refines
/// the background right after raw mode comes up — [`probe_background`]
/// needs raw mode (a cooked tty line-buffers the OSC reply away), so the
/// handshake keeps the default and the session setup follows up with a
/// corrected `set-client-colors`.
fn host_colors() -> (&'static str, &'static str) {
    ("ffffff", "000000")
}

/// Probe the host terminal's background color with OSC 11
/// (`ESC ] 11 ; ? ST`): write the query and wait up to 150 ms for the
/// `rgb:RRRR/GGGG/BBBB` reply on stdin, poll(2)-based so a silent host
/// costs only the deadline. Unix only. Must run while raw mode is up and
/// BEFORE the pump's stdin reader thread starts (the probe is, briefly,
/// the tty's only reader).
///
/// The SECOND tuple element is every stdin byte the probe consumed
/// without finding the reply — keystrokes that landed inside the probe's
/// window. The caller MUST prime them back into the pump's stdin stream
/// (`Stdin::new_with_primer`); dropping them eats the user's first
/// keystrokes.
///
/// Race audit (round 3): the probe and the pump's stdin reader never run
/// concurrently — the reader thread is born in `Stdin::new_with_primer`,
/// after `probe_background` returned — so the reply cannot be stolen
/// mid-window. The two shapes that miss the reply are timing, not racing:
/// a host answering after the 150 ms deadline (the reply bytes then reach
/// the Stdin reader, whose parser drops unknown OSC/CSI spellings — the
/// bytes are consumed and discarded, never forwarded to a pane), and a
/// reply the parser cannot read (non-UTF-8). Both degrade to the `None`
/// result, whose fallback renders terminal-default cells — see
/// `PaneRenderer::set_background` — so a missed probe can never paint a
/// wrong color.
#[cfg(unix)]
pub(crate) fn probe_background() -> (Option<(u8, u8, u8)>, Vec<u8>) {
    use nix::poll;
    use std::io::{Read as _, Write as _};
    use std::os::fd::AsFd as _;
    {
        let mut out = std::io::stdout().lock();
        if out.write_all(b"\x1b]11;?\x1b\\").is_err() || out.flush().is_err() {
            return (None, Vec::new());
        }
    }
    let deadline = std::time::Instant::now() + Duration::from_millis(150);
    let mut acc: Vec<u8> = Vec::new();
    let mut buf = [0u8; 32];
    let stdin = std::io::stdin();
    let mut handle = stdin.lock();
    loop {
        if std::time::Instant::now() >= deadline {
            return (None, acc);
        }
        let remaining = deadline
            .saturating_duration_since(std::time::Instant::now())
            .as_millis()
            .min(150) as u16;
        let mut fds = [poll::PollFd::new(handle.as_fd(), poll::PollFlags::POLLIN)];
        let Ok(ready) = poll::poll(&mut fds, Some(remaining)) else {
            return (None, acc);
        };
        if ready == 0 {
            return (None, acc); // deadline hit with nothing to read
        }
        let Ok(n) = handle.read(&mut buf) else {
            return (None, acc);
        };
        if n == 0 {
            return (None, acc); // EOF: not a tty / host closed
        }
        acc.extend_from_slice(&buf[..n]);
        if let Some(color) = parse_osc_color(&acc) {
            // Bytes landing after the reply in the same read chunk are
            // dropped — the reply path is the success shape.
            return (Some(color), Vec::new());
        }
        if acc.len() > 128 {
            return (None, acc); // runaway reply; hand back what arrived
        }
    }
}

/// Parse an OSC 10/11 color report (`ESC ] 11 ; rgb:RR/GG/BB ST` — each
/// component 1-4 hex digits, scaled to 8 bits) from a raw byte stream.
#[cfg(unix)]
fn parse_osc_color(bytes: &[u8]) -> Option<(u8, u8, u8)> {
    let text = core::str::from_utf8(bytes).ok()?;
    let rest = text.split_once(";rgb:")?.1;
    let end = rest.find(['\x1b', '\x07'])?;
    let body = &rest[..end];
    let mut parts = body.split('/');
    let component = |raw: &str| -> Option<u8> {
        // Scale a 1-4 hex-digit component to 8 bits. The max must be
        // computed in u32: a 4-digit component's max is 0xFFFF, and
        // `1u16 << 16` would shift-overflow — the field defect that
        // turned Ghostty's 16-bit replies (`rgb:1e1e/…`) into garbage.
        let value = u32::from_str_radix(raw, 16).ok()?;
        let digits = raw.len().clamp(1, 4) as u32;
        let max = (1u32 << (4 * digits)) - 1;
        Some((value.min(max) * 255 / max) as u8)
    };
    let r = component(parts.next()?)?;
    let g = component(parts.next()?)?;
    let b = component(parts.next()?)?;
    Some((r, g, b))
}

#[cfg(not(unix))]
pub(crate) fn probe_background() -> (Option<(u8, u8, u8)>, Vec<u8>) {
    (None, Vec::new())
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

// The only test here exercises the unix-only OSC 11 parse; on Windows the
// module would be empty and its imports unused.
#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// The OSC 11 reply parse: xterm's 16-bit and Ghostty's wide
    /// spellings, the BEL terminator, and graceful None on garbage and
    /// on a partial stream (the probe keeps polling until the deadline
    /// when the reply splits across reads).
    #[cfg(unix)] // parse_osc_color backs the unix-only probe
    #[test]
    fn osc_color_report_parses_and_yields_gracefully() {
        // xterm: rgb:ffff/ffff/ffff -> white.
        assert_eq!(
            parse_osc_color(b"\x1b]11;rgb:ffff/ffff/ffff\x1b\\"),
            Some((255, 255, 255))
        );
        // Ghostty's 16-bit-per-component reply for #1e1e1e.
        assert_eq!(
            parse_osc_color(b"\x1b]11;rgb:1e1e/1e1e/1e1e\x1b\\"),
            Some((0x1e, 0x1e, 0x1e))
        );
        // BEL-terminated short form.
        assert_eq!(
            parse_osc_color(b"\x1b]11;rgb:1e/1e/1e\x07"),
            Some((0x1e, 0x1e, 0x1e))
        );
        // A partial reply (split read) parses to None - the probe's loop
        // keeps the bytes and polls again.
        assert_eq!(parse_osc_color(b"\x1b]11;rgb:1e"), None);
        // Non-reply bytes: graceful None, never a color.
        assert_eq!(parse_osc_color(b"hello"), None);
    }
}
