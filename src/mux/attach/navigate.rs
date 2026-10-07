//! The passthrough `Session`'s focus moves: pane cycling and the
//! window / session / workspace switches, all through the
//! select-then-refresh contract.

use super::*;

impl Session {
    /// prefix + arrow: every arrow cycles panes in Phase A (tmux's o).
    pub(super) fn prefix_arrow(&mut self, seq: &[u8]) {
        if matches!(seq, [b'[', b'A'..=b'D']) {
            self.cycle_pane();
        }
    }

    /// prefix o / arrows: select the next pane in the window's layout-leaf
    /// order, then resync — select-then-refresh, so the redraw is the
    /// daemon's authoritative screen.
    pub(super) fn cycle_pane(&mut self) {
        let Ok(reply) = self
            .conn
            .send_checked(&format!("list-panes -t {}", self.window))
        else {
            return;
        };
        if !reply.ok {
            return;
        }
        let panes: Vec<String> = reply
            .body
            .iter()
            .filter_map(|l| l.split_whitespace().next())
            .filter(|p| p.starts_with('%'))
            .map(str::to_string)
            .collect();
        if panes.len() < 2 {
            return;
        }
        let current = panes.iter().position(|p| *p == self.pane).unwrap_or(0);
        let next = (current + 1) % panes.len();
        self.switch_to_pane(&panes[next]);
    }

    /// prefix n/p: move to the next/previous window of the session and
    /// attach to its active pane, select-then-refresh.
    pub(super) fn switch_window(&mut self, direction: i32) {
        let Some(session) = self.session_id.clone() else {
            return;
        };
        let Ok(reply) = self
            .conn
            .send_checked(&format!("list-windows -t {session}"))
        else {
            return;
        };
        if !reply.ok {
            return;
        }
        let windows: Vec<String> = reply
            .body
            .iter()
            .filter_map(|l| l.split_whitespace().next())
            .filter(|w| w.starts_with('@'))
            .map(str::to_string)
            .collect();
        let Some(position) = windows.iter().position(|w| *w == self.window) else {
            return;
        };
        let next = (position as i32 + direction).rem_euclid(windows.len() as i32) as usize;
        let window = &windows[next];
        if self
            .conn
            .send_checked(&format!("select-window -t {window}"))
            .is_ok_and(|reply| reply.ok)
        {
            self.window = window.clone();
            self.attach_window_active_pane(window);
        }
    }

    /// prefix ( / ): the previous/next session in list-sessions order;
    /// attach to its active window's active pane, select-then-refresh.
    pub(super) fn switch_session(&mut self, direction: i32) {
        let Ok(reply) = self.conn.send_checked("list-sessions") else {
            return;
        };
        if !reply.ok {
            return;
        }
        let sessions: Vec<String> = reply
            .body
            .iter()
            .filter_map(|l| parse_session_line(l).map(|(id, _)| id))
            .collect();
        let Some(current) = sessions
            .iter()
            .position(|s| Some(s) == self.session_id.as_ref())
        else {
            return;
        };
        let next = (current as i32 + direction).rem_euclid(sessions.len() as i32) as usize;
        let session = &sessions[next];
        let Ok(windows) = self
            .conn
            .send_checked(&format!("list-windows -t {session}"))
        else {
            return;
        };
        if !windows.ok {
            return;
        }
        // The marked `*` window is the session's active one.
        let window = windows
            .body
            .iter()
            .find(|l| l.split_whitespace().nth(1) == Some("*"))
            .or_else(|| windows.body.first())
            .and_then(|l| l.split_whitespace().next());
        let Some(window) = window else {
            return;
        };
        let _ = self
            .conn
            .send_checked(&format!("select-window -t {window}"));
        self.window = window.to_string();
        self.session_id = Some(session.clone());
        self.attach_window_active_pane(window);
    }

    /// prefix W / C-w: the next/previous workspace in id order —
    /// `select-workspace -t +N`, then land the view on the workspace's
    /// session (its active window's active pane) through the same
    /// select-then-refresh contract every switch follows. A workspace
    /// with no sessions cannot be landed on: the selection still moves
    /// and the status refresh carries the new active marker.
    pub(super) fn switch_workspace(&mut self, direction: i32) {
        let Ok(reply) = self.conn.send_checked("list-workspaces") else {
            return;
        };
        if !reply.ok {
            return;
        }
        let rows: Vec<(String, String, bool)> = reply
            .body
            .iter()
            .filter_map(|l| parse_workspace_line(l))
            .collect();
        if rows.is_empty() {
            return;
        }
        let Some(current) = rows.iter().position(|(_, _, active)| *active) else {
            return;
        };
        let next = (current as i32 + direction).rem_euclid(rows.len() as i32) as usize;
        let (ws_id, _, _) = &rows[next];
        let _ = self
            .conn
            .send_checked(&format!("select-workspace -t {ws_id}"));
        self.land_in_workspace(ws_id);
    }

    /// After a workspace select, land the pump on the workspace's
    /// session: its first listed session's active window's active pane.
    /// A workspace with no sessions cannot be landed on — the status
    /// refresh carries the moved active marker instead.
    pub(super) fn land_in_workspace(&mut self, ws_id: &str) {
        let Ok(reply) = self.conn.send_checked(&format!("list-sessions -t {ws_id}")) else {
            return;
        };
        if !reply.ok {
            return;
        }
        let Some((session, _)) = reply
            .body
            .iter()
            .filter_map(|l| parse_session_line(l))
            .next()
        else {
            self.refresh_status();
            self.draw_status();
            return;
        };
        let Ok(windows) = self
            .conn
            .send_checked(&format!("list-windows -t {session}"))
        else {
            return;
        };
        if !windows.ok {
            return;
        }
        let window = windows
            .body
            .iter()
            .find(|l| l.split_whitespace().nth(1) == Some("*"))
            .or_else(|| windows.body.first())
            .and_then(|l| l.split_whitespace().next());
        let Some(window) = window else {
            return;
        };
        let _ = self
            .conn
            .send_checked(&format!("select-window -t {window}"));
        self.window = window.to_string();
        self.session_id = Some(session);
        self.attach_window_active_pane(window);
    }

    /// Make `window`'s active pane (its `*` marker in `list-panes -t`) the
    /// pumped pane, select-then-refresh.
    pub(super) fn attach_window_active_pane(&mut self, window: &str) {
        let Ok(reply) = self.conn.send_checked(&format!("list-panes -t {window}")) else {
            return;
        };
        if !reply.ok {
            return;
        }
        if let Ok(pane) = marked_pane(&reply.body, window) {
            self.switch_to_pane(&pane);
        }
    }

    /// Point the pump at `pane`, select it daemon-side, and resync the
    /// redraw (the select-then-refresh contract every switch follows).
    pub(super) fn switch_to_pane(&mut self, pane: &str) {
        let _ = self.conn.send_checked(&format!("select-pane -t {pane}"));
        self.pane = pane.to_string();
        self.exited = None;
        self.resync();
        self.refresh_status();
        self.draw_status();
    }
}
