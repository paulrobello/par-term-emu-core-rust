//! The `send_*` broadcast API embedders call, and `shutdown`.

use super::*;

impl StreamingServer {
    /// Send terminal output to all connected clients
    pub fn send_output(&self, data: String) -> Result<()> {
        if let Some(ref session) = self.default_session {
            match session.output_tx.try_send(data) {
                Ok(()) => Ok(()),
                Err(mpsc::error::TrySendError::Full(_)) => {
                    session
                        .metrics
                        .dropped_messages
                        .fetch_add(1, Ordering::Relaxed);
                    Ok(()) // Drop silently under backpressure
                }
                Err(mpsc::error::TrySendError::Closed(_)) => Err(StreamingError::ServerError(
                    "Output channel closed".to_string(),
                )),
            }
        } else {
            Err(StreamingError::ServerError(
                "No default session".to_string(),
            ))
        }
    }

    /// Send a resize event to all clients
    pub fn send_resize(&self, cols: u16, rows: u16) {
        let msg = ServerMessage::resize(cols, rows);
        self.broadcast(msg);
    }

    /// Send a title change event to all clients
    pub fn send_title(&self, title: String) {
        let msg = ServerMessage::title(title);
        self.broadcast(msg);
    }

    /// Send a bell event to all clients
    pub fn send_bell(&self) {
        let msg = ServerMessage::bell();
        self.broadcast(msg);
    }

    /// Send a CWD changed event to all clients
    pub fn send_cwd_changed(
        &self,
        old_cwd: Option<String>,
        new_cwd: String,
        hostname: Option<String>,
        username: Option<String>,
        timestamp: u64,
    ) {
        let msg = ServerMessage::cwd_changed_full(old_cwd, new_cwd, hostname, username, timestamp);
        self.broadcast(msg);
    }

    /// Send a trigger matched event to all clients
    // Public API mirroring `ServerMessage::trigger_matched`; a parameter
    // struct would break embedders.
    #[allow(clippy::too_many_arguments)]
    pub fn send_trigger_matched(
        &self,
        trigger_id: u64,
        row: u16,
        col: u16,
        end_col: u16,
        text: String,
        captures: Vec<String>,
        timestamp: u64,
    ) {
        let msg = ServerMessage::trigger_matched(
            trigger_id, row, col, end_col, text, captures, timestamp,
        );
        self.broadcast(msg);
    }

    /// Send a trigger action notify event to all clients
    pub fn send_action_notify(&self, trigger_id: u64, title: String, message: String) {
        let msg = ServerMessage::action_notify(trigger_id, title, message);
        self.broadcast(msg);
    }

    /// Send a trigger action mark line event to all clients
    pub fn send_action_mark_line(
        &self,
        trigger_id: u64,
        row: u16,
        label: Option<String>,
        color: Option<(u8, u8, u8)>,
    ) {
        let msg = ServerMessage::action_mark_line(trigger_id, row, label, color);
        self.broadcast(msg);
    }

    /// Send a mode changed event to all clients
    pub fn send_mode_changed(&self, mode: String, enabled: bool) {
        let msg = ServerMessage::mode_changed(mode, enabled);
        self.broadcast(msg);
    }

    /// Send a graphics added event to all clients
    pub fn send_graphics_added(&self, row: u16) {
        let msg = ServerMessage::graphics_added(row);
        self.broadcast(msg);
    }

    /// Send a hyperlink added event to all clients
    pub fn send_hyperlink_added(&self, url: String, row: u16, col: u16, id: Option<String>) {
        let msg = match id {
            Some(id) => ServerMessage::hyperlink_added_with_id(url, row, col, id),
            None => ServerMessage::hyperlink_added(url, row, col),
        };
        self.broadcast(msg);
    }

    /// Send a user variable changed event to all clients
    pub fn send_user_var_changed(&self, name: String, value: String, old_value: Option<String>) {
        let msg = ServerMessage::user_var_changed_full(name, value, old_value);
        self.broadcast(msg);
    }

    /// Send a progress bar changed event to all clients
    pub fn send_progress_bar_changed(
        &self,
        action: crate::terminal::ProgressBarAction,
        id: String,
        state: Option<crate::terminal::ProgressState>,
        percent: Option<u8>,
        label: Option<String>,
    ) {
        let msg = ServerMessage::progress_bar_changed(action, id, state, percent, label);
        self.broadcast(msg);
    }

    /// Send a cursor position event to all clients
    pub fn send_cursor_position(&self, col: u16, row: u16, visible: bool) {
        let msg = ServerMessage::cursor(col, row, visible);
        self.broadcast(msg);
    }

    /// Send a badge changed event to all clients
    pub fn send_badge_changed(&self, badge: Option<String>) {
        let msg = ServerMessage::badge_changed(badge);
        self.broadcast(msg);
    }

    /// Shutdown the server and disconnect all clients
    pub fn shutdown(&self, reason: String) {
        crate::debug_info!("STREAMING", "Shutting down server: {}", reason);
        let msg = ServerMessage::shutdown(reason);
        self.broadcast(msg);
        self.shutdown.notify_waiters();
    }
}
