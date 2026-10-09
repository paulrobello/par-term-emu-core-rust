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
        let windows = list_window_ids(&mut self.conn, &session);
        let Some(window) = next_in_cycle(&windows, &self.window, direction > 0).cloned() else {
            return;
        };
        if !self.land(&window) {
            return;
        }
        self.window = window.clone();
        self.attach_window_active_pane(&window);
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
        let Some(current) = self.session_id.clone() else {
            return;
        };
        let Some(session) = next_in_cycle(&sessions, &current, direction > 0).cloned() else {
            return;
        };
        self.land_in_session(&session);
    }

    /// Land the pump on `session`'s active window's active pane. Local
    /// state moves only once the daemon accepted the landing.
    fn land_in_session(&mut self, session: &str) {
        let Some(window) = session_active_window(&mut self.conn, session) else {
            return;
        };
        if !self.land(&window) {
            return;
        }
        self.window = window.clone();
        self.session_id = Some(session.to_string());
        self.attach_window_active_pane(&window);
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
        let Some(ws_id) = next_workspace(&reply.body, direction > 0) else {
            return;
        };
        let _ = self
            .conn
            .send_checked(&format!("select-workspace -t {ws_id}"));
        self.land_in_workspace(&ws_id);
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
        self.land_in_session(&session);
    }

    /// A user-initiated landing on `window`: `switch-client -t <window>`
    /// selects it AND makes its session the daemon's displayed one (the
    /// persisted pointer the next no-target attach and a restore read —
    /// card 01a11bd1), falling back to `select-window` for a daemon
    /// without `switch-client`. Returns whether either landed; callers
    /// mutate local state only on `true`, so a refused landing never
    /// desyncs the view. The render mode's `land_window` follows the same
    /// rule.
    pub(super) fn land(&mut self, window: &str) -> bool {
        let landed =
            |reply: std::io::Result<crate::mux::client::Reply>| reply.is_ok_and(|reply| reply.ok);
        landed(
            self.conn
                .send_checked(&format!("switch-client -t {window}")),
        ) || landed(
            self.conn
                .send_checked(&format!("select-window -t {window}")),
        )
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
