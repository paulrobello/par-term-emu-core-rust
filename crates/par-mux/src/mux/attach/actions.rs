//! The passthrough `Session`'s prefix-chord actions: resize mode, pane
//! swap, help dump, respawn, the management chords (split / kill /
//! new-window), the config reload, and the chunked key forward.

use super::*;
use std::io::Write as _;

impl Session {
    /// Enter the sticky resize mode (the `resize` chord): arrows adjust
    /// the focused pane's edges until Enter/Escape/`q` — tmux's resize
    /// step with an explicit mode instead of repeat-time. The flash cue
    /// rides the pump's flash path so the user can see the mode is up.
    pub(super) fn enter_resize_mode(&mut self) {
        self.resize_mode = true;
        self.flash = Some(format!(
            "resize — arrows move the edge by {}, Enter/q exits",
            self.resize_step
        ));
    }

    /// Resize-mode byte routing: arrow CSI sequences (`ESC [ A..D`) send
    /// one resize step for the focused pane (`resize-pane -t <pane>
    /// -L|-R|-U|-D <step>`, the wire's relative form); `q`/Enter exit; any
    /// other byte exits the mode and is reprocessed by the normal router
    /// (the key that cancelled still does its job). Returns true on
    /// detach.
    pub(super) fn route_resize_bytes(&mut self, bytes: &[u8]) -> bool {
        let mut index = 0;
        while index < bytes.len() {
            match &bytes[index..] {
                [0x1b, b'[', dir, ..] if (b'A'..=b'D').contains(dir) => {
                    self.resize_step_cmd(*dir);
                    index += 3;
                }
                _ => {
                    let exits = matches!(bytes[index], b'q' | b'\r');
                    self.resize_mode = false;
                    index += usize::from(exits);
                    // The remainder — including the cancelling byte unless
                    // it was a clean exit key — routes normally.
                    return self.route_bytes(&bytes[index..]);
                }
            }
        }
        false
    }

    /// One resize step in the arrow's direction: the wire's relative form
    /// (`resize-pane -t <pane> -U <step>` etc.). Best-effort — the
    /// daemon's repaint rides the pane's %output stream either way.
    pub(super) fn resize_step_cmd(&mut self, arrow: u8) {
        let flag = match arrow {
            b'A' => "-U",
            b'B' => "-D",
            b'C' => "-R",
            _ => "-L",
        };
        let _ = self.conn.send_checked(&format!(
            "resize-pane -t {} {flag} {}",
            self.pane, self.resize_step
        ));
    }

    /// prefix { / }: swap the focused pane with its layout-order neighbor
    /// (`swap-pane -s <focused> -t <neighbor>`); fewer than two panes is
    /// a no-op. The daemon's %layout-change / output stream carries the
    /// visual swap.
    pub(super) fn swap_pane(&mut self, direction: i32) {
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
        let Some(position) = panes.iter().position(|p| *p == self.pane) else {
            return;
        };
        let next = (position as i32 + direction).rem_euclid(panes.len() as i32) as usize;
        let _ = self
            .conn
            .send_checked(&format!("swap-pane -s {} -t {}", self.pane, panes[next]));
    }

    /// prefix ?: the bindings panel. Passthrough has no overlay surface —
    /// the pane's own output owns the screen — so the panel prints as
    /// plain text (the same category rows the render-mode modal composes;
    /// filter/scroll are modal-only controls) and the pane's next output
    /// redraws over it (the documented passthrough help shape). The dump
    /// leads with a blank line and bolds the category headers — printing
    /// from the cursor's current row put the first header on the prompt
    /// line (manual pass).
    pub(super) fn show_help(&mut self) {
        let rows = help_rows(
            self.prefix,
            self.reload_key,
            self.management,
            self.resize_step,
        );
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(help_dump_text(&rows).as_bytes());
        let _ = stdout.flush();
        // Advance the pane past the dump: the pane's cursor is still on
        // the prompt row, so every later keystroke painted over the help
        // (the manual-pass mix). One Enter at a shell prompt runs an empty
        // command line and a fresh prompt — ~2 rows each — so half the
        // dump's row count lands the shell below the text; the help stays
        // in the pane's scrollback either way.
        let enters = rows.len() / 2 + 1;
        let bytes = vec![b'\r'; enters];
        forward_chunked(&mut self.conn, self.pane.clone(), &bytes);
    }

    /// Prefix r: `respawn-pane` — but ONLY when the pane is held dead
    /// (the card's "respawn-pane when held dead"); a live pane restart is
    /// not a Phase A affordance (the daemon itself refuses without -k, and
    /// killing a live pane from a mistyped chord would be destructive).
    pub(super) fn respawn_if_dead(&mut self) {
        if self.exited.is_none() {
            return;
        }
        if self
            .conn
            .send_checked(&format!("respawn-pane -t {}", self.pane))
            .is_ok_and(|reply| reply.ok)
        {
            self.exited = None;
            // The dead pane's frozen screen is still on the glass and the
            // fresh replay only paints what the new grid holds — wipe the
            // surface first (we are inside the alternate screen) so the
            // new output does not mix over the corpse.
            let mut stdout = std::io::stdout().lock();
            let _ = stdout.write_all(b"\x1b[2J\x1b[H");
            let _ = stdout.flush();
            self.resync();
            self.refresh_status();
            self.draw_status();
        }
    }

    /// Which management chord (if any) `key` is. Matched by byte BEFORE
    /// the fixed command table — the chords are configurable, so they
    /// cannot be static table arms.
    pub(super) fn management_command(&self, key: u8) -> Option<ManagementKey> {
        let m = self.management;
        match key {
            k if k == m.split_right => Some(ManagementKey::SplitRight),
            k if k == m.split_down => Some(ManagementKey::SplitDown),
            k if k == m.kill_pane => Some(ManagementKey::KillPane),
            k if k == m.new_window => Some(ManagementKey::NewWindow),
            k if k == m.swap_prev => Some(ManagementKey::SwapPrev),
            k if k == m.swap_next => Some(ManagementKey::SwapNext),
            k if k == m.workspace_next => Some(ManagementKey::WorkspaceNext),
            k if k == m.workspace_prev => Some(ManagementKey::WorkspacePrev),
            _ => None,
        }
    }

    /// prefix % / ": `split-window -t <focused> [-h]` — the daemon
    /// focuses the new pane and replies with its id; land the pump on it
    /// (the same switch-then-refresh contract `cycle_pane` follows, so
    /// the redraw is the fresh pane's authoritative screen). tmux lands
    /// you on the new split; this is the same follow.
    pub(super) fn split_pane(&mut self, right: bool) {
        let flag = if right { " -h" } else { "" };
        let Ok(reply) = self
            .conn
            .send_checked(&format!("split-window -t {}{flag}", self.pane))
        else {
            return;
        };
        if !reply.ok {
            return;
        }
        if let Some(new_pane) = reply.body.first() {
            self.switch_to_pane(new_pane);
        }
    }

    /// prefix x: `kill-pane -t <focused>`. Killing the focused pane
    /// removes it — the window survives (a survivor is announced via
    /// %window-pane-changed) or the window itself closes. The pump
    /// follows the successor: the window's active pane when one remains
    /// (the same land-on-survivor move the switch chords make), the
    /// dead-pane cue when the whole window closed is NOT survivable from
    /// here — that path ends through %sessions-changed's existing
    /// contract (passthrough: the next pane-info fails, the reconnect
    /// gate reports the pane gone; the user sees the cue and detaches or
    /// respawns per the standing contract). Best-effort either way: a
    /// failed kill (the pane already gone) changes nothing client-side.
    pub(super) fn kill_focused_pane(&mut self) {
        let window = self.window.clone();
        let Ok(reply) = self
            .conn
            .send_checked(&format!("kill-pane -t {}", self.pane))
        else {
            return;
        };
        if !reply.ok {
            return;
        }
        // Does the window survive? Its new active pane is the land site.
        if let Ok(list) = self.conn.send_checked(&format!("list-panes -t {window}")) {
            if list.ok {
                if let Ok(survivor) = marked_pane(&list.body, &window) {
                    self.switch_to_pane(&survivor);
                    return;
                }
            }
        }
        // The window is gone (the focused pane was its last): the pane
        // this pump was showing no longer exists anywhere. Mirror the
        // held-dead guard's silence — take no further bytes, show the
        // exit cue — and let the user detach. `pane-info` on the dead id
        // fails on the next status refresh, which is fine: the guard
        // keeps stdin dropped and the chord table live.
        self.exited = Some(None);
        self.refresh_status();
        self.draw_status();
    }

    /// prefix c: `new-window -t <session>` — the daemon replies with the
    /// new window's id; select it and attach to its active pane (the
    /// reply ordering in tmux puts you on the fresh window; this follows).
    pub(super) fn new_window_in_session(&mut self) {
        let Some(session) = self.session_id.clone() else {
            return;
        };
        let Ok(reply) = self.conn.send_checked(&format!("new-window -t {session}")) else {
            return;
        };
        if !reply.ok {
            return;
        }
        let Some(window) = reply.body.first() else {
            return;
        };
        let window = window.trim();
        if self
            .conn
            .send_checked(&format!("select-window -t {window}"))
            .is_ok_and(|reply| reply.ok)
        {
            self.window = window.to_string();
            self.attach_window_active_pane(window);
        }
    }

    /// The reload chord: re-read the config file, rebind the prefix and
    /// the reload chord live (the reload key rebinding includes itself —
    /// the NEXT reload follows the new chord), queue the status cue, and
    /// send `reload-config` to the daemon so its settings follow. A
    /// parse error in the re-read file shows on the status row instead
    /// of detaching.
    pub(super) fn reload_config(&mut self) {
        match reload_client_chords(crate::mux::config::Chords {
            prefix: self.prefix,
            reload: self.reload_key,
            management: self.management,
            resize_step: self.resize_step,
            pane_borders: false,
            show_label_in_border: false,
            pane_gaps: 0,
            scrollbar_gutter: false,
            sidebar_width: 20,
            drag_cursor_shape: false,
            border_lines: "unicode".to_string(),
        }) {
            Ok(new_chords) => {
                self.prefix = new_chords.prefix;
                self.reload_key = new_chords.reload;
                self.management = new_chords.management;
                self.resize_step = new_chords.resize_step;
                self.flash = Some("config reloaded".to_string());
            }
            Err(err) => {
                self.flash = Some(format!("reload failed: {err}"));
            }
        }
        // Daemon-side: best-effort — the daemon reports its per-setting
        // outcome in its own reply; nothing here parses it (the status
        // cue above is the client-side truth).
        let _ = self.conn.send_checked("reload-config");
    }

    /// Forward bytes to the pane in ~512-byte `send-keys -H` chunks. A
    /// send failure means the daemon is gone; the pump's next poll
    /// observes the close.
    pub(super) fn send_chunked(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(CHUNK) {
            if self
                .conn
                .send_checked(&format!(
                    "send-keys -t {} -H {}",
                    self.pane,
                    hex_byte_list(chunk)
                ))
                .is_err()
            {
                return;
            }
        }
    }
}
