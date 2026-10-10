//! The passthrough `Session`'s pump lifecycle: the screen resync, the
//! status-state re-query, the poll loop, stdin draining, and the
//! one-shot reconnect.

use super::*;
use std::io::Write as _;

impl Session {
    /// Resync the target pane's screen: the replay body goes to stdout
    /// verbatim (each reply line plus its newline — the draw's absolute
    /// CUP re-places the host cursor, so the write side's final newline
    /// is immaterial). The reply body — the daemon's screen-restore byte
    /// stream, ending with the pane's real cursor CUP — seeds the shadow
    /// emulator verbatim; its tracked cursor becomes the pane's truth the
    /// status draw re-places.
    pub(super) fn resync(&mut self) {
        // Size report against the target pane's window, BEFORE the replay:
        // the handshake's target-less -C sizes only the newest session's
        // active window, and every switch re-enters here — a window
        // restored at another size never re-fit, and the pane's child ran
        // at the stale height (the manual-pass htop report; render mode's
        // switch path has always reported). rows-1: the status row stays
        // reserved below the content region — the same pane grid the
        // shadow emulator is sized to.
        let (cols, rows) = conn::terminal_grid();
        if rows >= 2 && cols >= 2 {
            let (pane_cols, pane_rows) = super::status_row::pane_grid(cols, rows);
            let _ = self.conn.send_checked(&format!(
                "refresh-client -t {} -C {}x{}",
                self.pane, pane_cols, pane_rows
            ));
        }
        let reply = match self
            .conn
            .send_checked(&format!("refresh-client -t {}", self.pane))
        {
            Ok(reply) if reply.ok => reply,
            Ok(reply) => {
                let _ = std::io::stderr()
                    .write_all(format!("par-mux: {}\n", reply.body.join("\n")).as_bytes());
                return;
            }
            Err(err) => {
                let _ = std::io::stderr().write_all(format!("par-mux: {err}\n").as_bytes());
                return;
            }
        };
        // The restore stream ends with the pane's real cursor CUP, so the
        // shadow — which must track the PANE, not the host's post-write
        // drift — feeds it verbatim. A trailing newline appended here
        // parked the shadow one row below the prompt, and every status
        // draw's absolute placement with it (manual-pass cursor bug). The
        // host write below keeps its per-line newlines; the draw's
        // absolute CUP re-places the host cursor regardless.
        let bytes = reply.body.join("\n").into_bytes();
        self.emulator.feed(&bytes);
        let mut stdout = std::io::stdout().lock();
        // Clear before the replay: a switch lands here too, and the new
        // pane's screen must not mix with the previous pane's leftovers
        // (rows the last pane never wrote). The guard cleared at attach;
        // this re-clears per pane-show.
        let _ = write_all_blocking(&mut stdout, b"\x1b[2J\x1b[H");
        for line in &reply.body {
            let _ = write_all_blocking(&mut stdout, line.as_bytes());
            let _ = write_all_blocking(&mut stdout, b"\n");
        }
        let _ = stdout.flush();
    }

    /// Re-read the status-line state. Against a daemon advertising
    /// `client-snapshot v1`, one `client-snapshot -t <window>` round trip
    /// (the ENH-043 render-mode shape, card 01a1216d4780); any refusal —
    /// the feature missing, the tracked window unknown, transport
    /// failure, a stale window the daemon no longer knows — runs the
    /// legacy per-fact round, the spelling that also discovers `window`
    /// from pane-info.
    pub(super) fn refresh_status(&mut self) {
        if !self.window.is_empty()
            && self.conn.has_command_feature("client-snapshot", "v1")
            && self.refresh_status_snapshot()
        {
            return;
        }
        self.refresh_status_legacy();
    }

    /// The single-round-trip refresh. False when the daemon refused the
    /// snapshot (transport failure, unknown/stale tracked window, unknown
    /// version) — the caller falls back to the legacy round, which
    /// re-derives the tracked window. The pane's exit state is
    /// event-driven (`%pane-exited` / `%pane-respawned`); the snapshot
    /// body carries no exit fact.
    fn refresh_status_snapshot(&mut self) -> bool {
        let reply = match self.conn.send_checked_timeout(
            &format!("client-snapshot -t {}", self.window),
            super::status::STATUS_TIMEOUT,
        ) {
            Ok(reply) => reply,
            Err(_) => return false,
        };
        if !reply.ok {
            return false;
        }
        let mut body = reply.body.iter();
        if body.next().map(|l| l.trim_end()) != Some("snapshot 1") {
            return false;
        }
        self.session_id = None;
        self.session_name.clear();
        self.workspaces.clear();
        self.active_workspace = None;
        self.pane_title.clear();
        let mut agents = 0usize;
        for line in body {
            let Some((kind, rest)) = line.split_once(' ') else {
                continue;
            };
            match kind {
                "workspace" => {
                    let mut parts = rest.splitn(3, ' ');
                    let (Some(id), Some(marker)) = (parts.next(), parts.next()) else {
                        continue;
                    };
                    let name = parts.next().unwrap_or_default();
                    if marker == "*" {
                        self.active_workspace = Some(id.to_string());
                    }
                    self.workspaces.push((id.to_string(), name.to_string()));
                }
                "session" => {
                    let mut parts = rest.splitn(4, ' ');
                    let (Some(id), Some(marker)) = (parts.next(), parts.next()) else {
                        continue;
                    };
                    if marker == "*" {
                        let name = parts.nth(1).unwrap_or_default();
                        self.session_id = Some(id.to_string());
                        self.session_name = name.to_string();
                    }
                }
                "pane" => {
                    let (pane, title) = rest.split_once(' ').unwrap_or((rest, ""));
                    if pane == self.pane.as_str() {
                        self.pane_title = title.to_string();
                    }
                }
                "agent" => match rest.split_whitespace().next() {
                    Some(pane) if pane == self.pane.as_str() => agents += 1,
                    _ => {}
                },
                _ => {}
            }
        }
        self.agents = agents;
        true
    }

    /// The legacy per-fact re-read: pane-info, the session owner scan,
    /// the workspace roster, pane-title, list-agents.
    pub(super) fn refresh_status_legacy(&mut self) {
        if let Some(line) = self
            .conn
            .send_checked(&format!("pane-info -t {}", self.pane))
            .ok()
            .filter(|reply| reply.ok)
            .and_then(|reply| reply.body.first().cloned())
        {
            let mut fields = line.split_whitespace();
            if let (Some(_pane), Some(window)) = (fields.next(), fields.next()) {
                self.window = window.to_string();
            }
            self.exited = line
                .split_whitespace()
                .find_map(|tok| tok.strip_prefix("exited="))
                .map(|code| code.parse::<i32>().ok());
        }
        self.session_name.clear();
        self.session_id = None;
        if let Ok(reply) = self.conn.send_checked("list-sessions") {
            if reply.ok {
                for line in &reply.body {
                    let Some((sid, name)) = parse_session_line(line) else {
                        continue;
                    };
                    if let Ok(windows) = self.conn.send_checked(&format!("list-windows -t {sid}")) {
                        if windows
                            .body
                            .iter()
                            .any(|l| l.split_whitespace().next() == Some(self.window.as_str()))
                        {
                            self.session_id = Some(sid);
                            self.session_name = name;
                            break;
                        }
                        if !windows.ok {
                            break;
                        }
                    }
                }
            }
        }
        // The workspace roster for the status line's workspaces segment.
        // Ids sort as `+N` strings here; the daemon already lists in id
        // order, so the reply order IS id order.
        if let Ok(reply) = self.conn.send_checked("list-workspaces") {
            if reply.ok {
                let rows: Vec<(String, String, bool)> = reply
                    .body
                    .iter()
                    .filter_map(|l| parse_workspace_line(l))
                    .collect();
                self.workspaces = rows
                    .iter()
                    .map(|(id, name, _)| (id.clone(), name.clone()))
                    .collect();
                self.active_workspace = rows
                    .iter()
                    .find(|(_, _, active)| *active)
                    .map(|(id, _, _)| id.clone());
            }
        }
        if let Some(title) = self
            .conn
            .send_checked(&format!("pane-title -t {}", self.pane))
            .ok()
            .filter(|reply| reply.ok)
            .map(|reply| reply.body.join(" "))
        {
            self.pane_title = title;
        }
        self.agents = self
            .conn
            .send_checked("list-agents")
            .ok()
            .filter(|reply| reply.ok)
            .map(|reply| {
                reply
                    .body
                    .iter()
                    .filter(|l| l.split_whitespace().next() == Some(self.pane.as_str()))
                    .count()
            })
            .unwrap_or(0);
    }

    /// The pump: forward daemon pushes to stdout, stdin bytes to the pane,
    /// route the prefix, keep the status row current.
    pub(super) fn pump(&mut self) -> PumpOutcome {
        let outcome = self.pump_loop();
        // The status row's DECSTBM reservation must not outlive the
        // session: reset the scroll region and unhide whatever the pane
        // left hidden on EVERY exit path (detach, %exit, socket close).
        // Raw mode itself is the guard's business; these are the bytes the
        // client is responsible for.
        self.restore_region();
        outcome
    }

    /// Reset the scroll region and cursor state the status line borrowed.
    pub(super) fn restore_region(&self) {
        let mut stdout = std::io::stdout().lock();
        let _ = write_all_blocking(&mut stdout, b"\x1b[r\x1b[?25h");
        let _ = stdout.flush();
    }

    pub(super) fn pump_loop(&mut self) -> PumpOutcome {
        let mut stdin = Stdin::new();
        let mut status_dirty = true;
        // The flash cue's lifetime in loop polls (~1 s at POLL = 16 ms).
        const FLASH_POLLS: u32 = 60;
        let mut flash_polls = 0u32;
        // Periodic status redraw (~0.5 s): a full-screen pane app can wipe
        // the reserved bottom row or reset the host's scroll margins (the
        // manual-pass htop report) — one small absolute-CUP run restores
        // the bar without tracking the app's terminal writes.
        const STATUS_REDRAW_POLLS: u32 = 32;
        let mut status_polls = 0u32;
        loop {
            // 1. Drain daemon pushes.
            loop {
                match self.conn.try_recv() {
                    Ok(event) => {
                        if !self.handle_event(&event, &mut status_dirty) {
                            return PumpOutcome::DaemonExited;
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        // Socket closed: %exit delivered, eviction, or
                        // process death. One reconnect-and-requery; a
                        // client that cannot reattach reports the close.
                        if !self.reconnect() {
                            return PumpOutcome::ConnectionClosed;
                        }
                        break;
                    }
                }
            }

            // 2. Drain stdin (prefix routing, chunked forward).
            if self.pump_stdin(&mut stdin) {
                return PumpOutcome::Detached;
            }

            // 3. Status line redraw when something marked it dirty — or
            //    the host grid changed since the last draw (a resize, or
            //    ConPTY's geometry settling late). A reload's flash cue
            //    rides this path (route_bytes set it) and clears after
            //    about a second of polls, so the next normal redraw
            //    restores the plain status line.
            if status_dirty || self.size_changed() || self.flash.is_some() {
                if self.flash.is_some() {
                    flash_polls += 1;
                    if flash_polls > FLASH_POLLS {
                        self.flash = None;
                        flash_polls = 0;
                    }
                }
                status_dirty = false;
                status_polls = 0;
                self.refresh_status();
                self.draw_status();
            } else {
                status_polls += 1;
                if status_polls >= STATUS_REDRAW_POLLS {
                    status_polls = 0;
                    self.draw_status();
                }
            }

            // 4. Wait for the next push — and HANDLE it: an event that
            //    arrived during the wait is consumed here, and discarding
            //    it would drop the pane's %output bytes. Route it through
            //    the same handler; a Disconnected surfaces on the next
            //    pass's drain.
            if let Ok(event) = self.conn.recv_timeout(POLL) {
                if !self.handle_event(&event, &mut status_dirty) {
                    return PumpOutcome::DaemonExited;
                }
            }
        }
    }

    /// One reconnect-and-requery after a connection loss: re-handshake on
    /// the same socket, verify the pane still exists (a held pane keeps
    /// its id), and resync. False when the daemon is really gone.
    pub(super) fn reconnect(&mut self) -> bool {
        let Ok(conn) = conn::AttachConn::connect(&self.socket_path) else {
            return false;
        };
        self.conn = conn;
        let _replay = self.conn.drain_pending_events();
        if self
            .conn
            .send_checked(&format!("pane-info -t {}", self.pane))
            .is_ok_and(|reply| reply.ok)
        {
            self.resync();
            self.refresh_status();
            self.draw_status();
            true
        } else {
            false
        }
    }

    /// Read available stdin bytes and route them. Returns true on detach
    /// or stdin EOF.
    pub(super) fn pump_stdin(&mut self, stdin: &mut Stdin) -> bool {
        loop {
            match stdin.read_available() {
                None => return false,
                Some(Ok(bytes)) if bytes.is_empty() => return true, // EOF
                Some(Ok(bytes)) => {
                    if self.route_bytes(&bytes) {
                        return true;
                    }
                }
                Some(Err(err)) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    // A nonblocking host terminal's idle signal is not a
                    // detach (card 01a11d9687d4); the reader retries it,
                    // this arm keeps a stray one from detaching too.
                    return false;
                }
                Some(Err(_)) => return true,
            }
        }
    }
}
