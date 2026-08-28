//! Session state and registry for the streaming server (ARC-004).
//!
//! Split out of `server.rs`: per-session state (terminal handle, broadcast
//! channels, metrics), the session snapshot type behind `/sessions`, and the
//! thread-safe session registry. `SessionState` is renamed to
//! `StreamSessionState` (ARC-010) to stop colliding with the terminal
//! multiplexing `SessionState` in the Python bindings and `par-term`.

use crate::mouse::{MouseEncoding, MouseMode};
use crate::streaming::error::{Result, StreamingError};
use crate::streaming::protocol::{ServerMessage, ThemeInfo};
use crate::terminal::Terminal;
use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};

/// Get current time as epoch milliseconds
pub(crate) fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// =============================================================================
// Session Metrics
// =============================================================================

/// Per-session metrics for observability
pub struct SessionMetrics {
    /// Total messages sent to clients
    pub messages_sent: AtomicUsize,
    /// Total output bytes sent to clients
    pub bytes_sent: AtomicUsize,
    /// Total input bytes received from clients
    pub input_bytes: AtomicUsize,
    /// Total errors encountered
    pub errors: AtomicUsize,
    /// Total messages dropped (e.g., no receivers)
    pub dropped_messages: AtomicUsize,
    /// Last broadcast time (epoch millis)
    pub last_broadcast_time: AtomicU64,
}

impl SessionMetrics {
    /// Create new zeroed metrics
    fn new() -> Self {
        Self {
            messages_sent: AtomicUsize::new(0),
            bytes_sent: AtomicUsize::new(0),
            input_bytes: AtomicUsize::new(0),
            errors: AtomicUsize::new(0),
            dropped_messages: AtomicUsize::new(0),
            last_broadcast_time: AtomicU64::new(0),
        }
    }
}

// =============================================================================
// Session State
// =============================================================================

/// Per-session state extracted from StreamingServer for multi-session support
pub struct StreamSessionState {
    /// Unique session identifier
    pub id: String,
    /// Terminal instance for this session
    pub terminal: Arc<RwLock<Terminal>>,
    /// Broadcast channel for sending output to all clients in this session
    pub(crate) broadcast_tx: broadcast::Sender<ServerMessage>,
    /// Channel for sending output data into the broadcaster loop (bounded for backpressure)
    pub(crate) output_tx: mpsc::Sender<String>,
    /// Receiver end of the output channel (consumed by broadcaster loop)
    pub(crate) output_rx: Arc<tokio::sync::Mutex<mpsc::Receiver<String>>>,
    /// PTY writer for sending client input (optional, only set if PTY is available)
    #[allow(clippy::type_complexity)]
    pub(crate) pty_writer: std::sync::RwLock<Option<Arc<Mutex<Box<dyn std::io::Write + Send>>>>>,
    /// Channel for sending resize requests
    pub(crate) resize_tx: mpsc::UnboundedSender<(u16, u16)>,
    /// Receiver for resize requests
    pub(crate) resize_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<(u16, u16)>>>,
    /// Number of clients connected to this session
    pub(crate) client_count: AtomicUsize,
    /// When the last client disconnected (for idle timeout)
    pub(crate) last_client_disconnect: parking_lot::RwLock<Option<tokio::time::Instant>>,
    /// When this session was created (Unix epoch seconds)
    created_at: u64,
    /// Shutdown signal for this session's broadcaster loop
    pub(crate) shutdown: Arc<tokio::sync::Notify>,
    /// Optional theme for this session
    pub(crate) theme: Option<ThemeInfo>,
    /// Whether to send initial screen content on connect
    pub(crate) send_initial_screen: bool,
    /// Per-session metrics
    pub metrics: SessionMetrics,
}

impl StreamSessionState {
    /// Create a new session state
    pub fn new(
        id: String,
        terminal: Arc<RwLock<Terminal>>,
        theme: Option<ThemeInfo>,
        send_initial_screen: bool,
    ) -> Self {
        let (output_tx, output_rx) = mpsc::channel(1000);
        let (broadcast_tx, _) = broadcast::channel(100);
        let (resize_tx, resize_rx) = mpsc::unbounded_channel();

        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Self {
            id,
            terminal,
            broadcast_tx,
            output_tx,
            output_rx: Arc::new(tokio::sync::Mutex::new(output_rx)),
            pty_writer: std::sync::RwLock::new(None),
            resize_tx,
            resize_rx: Arc::new(tokio::sync::Mutex::new(resize_rx)),
            client_count: AtomicUsize::new(0),
            last_client_disconnect: parking_lot::RwLock::new(None),
            created_at,
            shutdown: Arc::new(tokio::sync::Notify::new()),
            theme,
            send_initial_screen,
            metrics: SessionMetrics::new(),
        }
    }

    /// Try to add a client to this session. Returns true if successful.
    /// When `max_per_session > 0`, uses CAS loop to enforce the limit atomically.
    pub fn try_add_client(&self, max_per_session: usize) -> bool {
        if max_per_session == 0 {
            self.client_count.fetch_add(1, Ordering::SeqCst);
            return true;
        }
        loop {
            let current = self.client_count.load(Ordering::Relaxed);
            if current >= max_per_session {
                return false;
            }
            if self
                .client_count
                .compare_exchange(current, current + 1, Ordering::SeqCst, Ordering::Relaxed)
                .is_ok()
            {
                return true;
            }
        }
    }

    /// Remove a client from this session.
    pub fn remove_client(&self) {
        let prev = self.client_count.fetch_sub(1, Ordering::SeqCst);
        if prev == 1 {
            // Was the last client - record disconnect time
            *self.last_client_disconnect.write() = Some(tokio::time::Instant::now());
        }
    }

    /// Build a Connected message from current terminal state
    pub fn build_connect_message(&self, client_id: &str, readonly: bool) -> ServerMessage {
        let terminal = self.terminal.write();
        let (cols, rows) = terminal.size();

        let initial_screen = if self.send_initial_screen {
            Some(terminal.export_visible_screen_styled())
        } else {
            None
        };

        let badge = terminal.evaluate_badge();
        let faint_alpha = Some(terminal.faint_text_alpha());
        let cwd = terminal.current_directory().map(|s| s.to_string());
        let mok_mode = Some(terminal.modify_other_keys_mode() as u32);

        ServerMessage::connected_builder(cols as u16, rows as u16, self.id.clone())
            .initial_screen(initial_screen)
            .theme(self.theme.clone())
            .badge(badge)
            .faint_text_alpha(faint_alpha)
            .cwd(cwd)
            .modify_other_keys(mok_mode)
            .client_id(Some(client_id.to_string()))
            .readonly(Some(readonly))
            .build()
    }

    /// Build ModeChanged messages for all active (non-default) terminal modes.
    ///
    /// Used to sync terminal mode state to clients connecting to existing sessions.
    /// Returns a list of `ServerMessage::ModeChanged` for each mode that differs
    /// from its default value.
    pub fn build_mode_sync_messages(&self) -> Vec<ServerMessage> {
        let terminal = self.terminal.write();
        let mut messages = Vec::new();

        // Mouse tracking mode
        let mouse_mode = terminal.mouse_mode();
        if mouse_mode != MouseMode::Off {
            let mode_name = match mouse_mode {
                MouseMode::X10 => "mouse_x10",
                MouseMode::Normal => "mouse_normal",
                MouseMode::ButtonEvent => "mouse_button_event",
                MouseMode::AnyEvent => "mouse_any_event",
                MouseMode::Off => unreachable!(),
            };
            messages.push(ServerMessage::mode_changed(mode_name.to_string(), true));
        }

        // Mouse encoding (if not default)
        let mouse_encoding = terminal.mouse_encoding();
        if mouse_encoding != MouseEncoding::Default {
            let encoding_name = match mouse_encoding {
                MouseEncoding::Utf8 => "mouse_utf8",
                MouseEncoding::Sgr => "mouse_sgr",
                MouseEncoding::Urxvt => "mouse_urxvt",
                MouseEncoding::Default => unreachable!(),
            };
            messages.push(ServerMessage::mode_changed(encoding_name.to_string(), true));
        }

        // Bracketed paste mode (DECSET 2004)
        if terminal.bracketed_paste() {
            messages.push(ServerMessage::mode_changed(
                "bracketed_paste".to_string(),
                true,
            ));
        }

        // Application cursor keys (DECCKM)
        if terminal.application_cursor() {
            messages.push(ServerMessage::mode_changed(
                "application_cursor".to_string(),
                true,
            ));
        }

        // Focus tracking (DECSET 1004)
        if terminal.focus_tracking() {
            messages.push(ServerMessage::mode_changed(
                "focus_tracking".to_string(),
                true,
            ));
        }

        // Cursor visibility (DECTCEM) - default is visible, so send if hidden
        if !terminal.cursor().visible {
            messages.push(ServerMessage::mode_changed(
                "cursor_visible".to_string(),
                false,
            ));
        }

        // Alternate screen buffer
        if terminal.is_alt_screen_active() {
            messages.push(ServerMessage::mode_changed(
                "alternate_screen".to_string(),
                true,
            ));
        }

        // Origin mode (DECOM)
        if terminal.origin_mode() {
            messages.push(ServerMessage::mode_changed("origin_mode".to_string(), true));
        }

        // Insert mode (IRM)
        if terminal.insert_mode() {
            messages.push(ServerMessage::mode_changed("insert_mode".to_string(), true));
        }

        // Auto-wrap mode (DECAWM) - default is true, so send if disabled
        if !terminal.auto_wrap_mode() {
            messages.push(ServerMessage::mode_changed("auto_wrap".to_string(), false));
        }

        messages
    }

    /// Set the PTY writer for handling client input
    pub fn set_pty_writer(&self, writer: Arc<Mutex<Box<dyn std::io::Write + Send>>>) {
        if let Ok(mut guard) = self.pty_writer.write() {
            *guard = Some(writer);
        }
    }

    /// Get a clone of the output sender channel
    pub fn get_output_sender(&self) -> mpsc::Sender<String> {
        self.output_tx.clone()
    }

    /// Get a clone of the resize receiver
    pub fn get_resize_receiver(
        &self,
    ) -> Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<(u16, u16)>>> {
        Arc::clone(&self.resize_rx)
    }

    /// Broadcast a message to all clients in this session
    pub fn broadcast(&self, msg: ServerMessage) {
        match self.broadcast_tx.send(msg) {
            Ok(_) => {
                self.metrics.messages_sent.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => {
                self.metrics
                    .dropped_messages
                    .fetch_add(1, Ordering::Relaxed);
                // No receivers — normal when 0 clients connected
            }
        }
    }

    /// Run the output broadcaster loop for this session
    pub async fn output_broadcaster_loop(&self) {
        let mut rx = self.output_rx.lock().await;
        let mut buffer = String::new();
        let mut last_flush = tokio::time::Instant::now();

        const BATCH_WINDOW: Duration = Duration::from_millis(16);
        const MAX_BATCH_SIZE: usize = 8192;

        loop {
            tokio::select! {
                _ = self.shutdown.notified() => {
                    crate::debug_info!("STREAMING", "Session {} broadcaster received shutdown signal", self.id);
                    if !buffer.is_empty() {
                        let data_len = buffer.len();
                        let msg = ServerMessage::output(buffer);
                        self.broadcast(msg);
                        self.metrics.bytes_sent.fetch_add(data_len, Ordering::Relaxed);
                    }
                    break;
                }
                msg = rx.recv() => {
                    match msg {
                        Some(data) => {
                            if !data.is_empty() {
                                buffer.push_str(&data);
                                if buffer.len() > MAX_BATCH_SIZE {
                                    let data_len = buffer.len();
                                    let msg = ServerMessage::output(std::mem::take(&mut buffer));
                                    self.broadcast(msg);
                                    self.metrics.bytes_sent.fetch_add(data_len, Ordering::Relaxed);
                                    self.metrics.last_broadcast_time.store(now_millis(), Ordering::Relaxed);
                                    last_flush = tokio::time::Instant::now();
                                }
                            }
                        }
                        None => {
                            if !buffer.is_empty() {
                                let data_len = buffer.len();
                                let msg = ServerMessage::output(buffer);
                                self.broadcast(msg);
                                self.metrics.bytes_sent.fetch_add(data_len, Ordering::Relaxed);
                            }
                            break;
                        }
                    }
                }
                _ = tokio::time::sleep_until(last_flush + BATCH_WINDOW), if !buffer.is_empty() => {
                    let data_len = buffer.len();
                    let msg = ServerMessage::output(std::mem::take(&mut buffer));
                    self.broadcast(msg);
                    self.metrics.bytes_sent.fetch_add(data_len, Ordering::Relaxed);
                    self.metrics.last_broadcast_time.store(now_millis(), Ordering::Relaxed);
                    last_flush = tokio::time::Instant::now();
                }
            }
        }
    }

    /// Signal this session to shut down
    pub fn shutdown(&self, reason: String) {
        crate::debug_info!("STREAMING", "Shutting down session {}: {}", self.id, reason);
        let msg = ServerMessage::shutdown(reason);
        self.broadcast(msg);
        self.shutdown.notify_waiters();
    }

    /// Get the number of clients connected to this session
    pub fn client_count(&self) -> usize {
        self.client_count.load(Ordering::Relaxed)
    }

    /// Check if this session is idle (no clients and past timeout)
    pub fn is_idle(&self, timeout: Duration) -> bool {
        if self.client_count() > 0 {
            return false;
        }
        if let Some(last_disconnect) = *self.last_client_disconnect.read() {
            last_disconnect.elapsed() >= timeout
        } else {
            false
        }
    }

    /// Get session info for the /sessions endpoint
    pub fn session_info(&self) -> SessionInfo {
        let terminal = self.terminal.write();
        let (cols, rows) = terminal.size();
        let cwd = terminal.current_directory().map(|s| s.to_string());

        let idle_seconds = if self.client_count() == 0 {
            self.last_client_disconnect
                .read()
                .map(|t| t.elapsed().as_secs())
                .unwrap_or(0)
        } else {
            0
        };

        SessionInfo {
            id: self.id.clone(),
            created: self.created_at,
            clients: self.client_count(),
            idle_seconds,
            cols: cols as u16,
            rows: rows as u16,
            cwd,
            messages_sent: self.metrics.messages_sent.load(Ordering::Relaxed),
            bytes_sent: self.metrics.bytes_sent.load(Ordering::Relaxed),
            input_bytes: self.metrics.input_bytes.load(Ordering::Relaxed),
            errors: self.metrics.errors.load(Ordering::Relaxed),
            dropped_messages: self.metrics.dropped_messages.load(Ordering::Relaxed),
        }
    }
}

impl std::fmt::Debug for StreamSessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamSessionState")
            .field("id", &self.id)
            .field("client_count", &self.client_count())
            .field("created_at", &self.created_at)
            .field("send_initial_screen", &self.send_initial_screen)
            .finish()
    }
}

/// Session information returned by the /sessions endpoint
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionInfo {
    /// Session identifier
    pub id: String,
    /// Creation timestamp (Unix epoch seconds)
    pub created: u64,
    /// Number of connected clients
    pub clients: usize,
    /// Seconds since last client disconnected (0 if clients are connected)
    pub idle_seconds: u64,
    /// Terminal columns
    pub cols: u16,
    /// Terminal rows
    pub rows: u16,
    /// Current working directory
    pub cwd: Option<String>,
    /// Total messages sent to clients
    pub messages_sent: usize,
    /// Total output bytes sent
    pub bytes_sent: usize,
    /// Total input bytes received
    pub input_bytes: usize,
    /// Total errors encountered
    pub errors: usize,
    /// Total messages dropped
    pub dropped_messages: usize,
}

// =============================================================================
// Session Registry
// =============================================================================

/// Thread-safe registry of active sessions
pub struct SessionRegistry {
    sessions: parking_lot::RwLock<HashMap<String, Arc<StreamSessionState>>>,
    max_sessions: usize,
}

impl SessionRegistry {
    /// Create a new session registry
    pub fn new(max_sessions: usize) -> Self {
        Self {
            sessions: parking_lot::RwLock::new(HashMap::new()),
            max_sessions,
        }
    }

    /// Get a session by ID
    pub fn get(&self, id: &str) -> Option<Arc<StreamSessionState>> {
        self.sessions.read().get(id).cloned()
    }

    /// Insert a session. Returns error if max_sessions would be exceeded.
    pub fn insert(&self, id: String, session: Arc<StreamSessionState>) -> Result<()> {
        let mut sessions = self.sessions.write();
        if sessions.len() >= self.max_sessions && !sessions.contains_key(&id) {
            return Err(StreamingError::MaxSessionsReached);
        }
        sessions.insert(id, session);
        Ok(())
    }

    /// Remove a session by ID
    pub fn remove(&self, id: &str) -> Option<Arc<StreamSessionState>> {
        self.sessions.write().remove(id)
    }

    /// Get the number of active sessions
    pub fn session_count(&self) -> usize {
        self.sessions.read().len()
    }

    /// Get IDs of sessions that are idle past the given timeout
    pub fn idle_sessions(&self, timeout: Duration) -> Vec<String> {
        self.sessions
            .read()
            .iter()
            .filter(|(_, s)| s.is_idle(timeout))
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// List all sessions for the /sessions endpoint
    pub fn list_sessions(&self) -> Vec<SessionInfo> {
        self.sessions
            .read()
            .values()
            .map(|s| s.session_info())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_session_state_creation() {
        let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
        let session = StreamSessionState::new("test-session".to_string(), terminal, None, true);
        assert_eq!(session.id, "test-session");
        assert_eq!(session.client_count(), 0);
        assert!(session.created_at > 0);
    }
    #[tokio::test]
    async fn test_session_state_client_count() {
        let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
        let session = StreamSessionState::new("sess".to_string(), terminal, None, true);

        assert_eq!(session.client_count(), 0);
        assert!(session.try_add_client(0)); // 0 = unlimited
        assert_eq!(session.client_count(), 1);
        assert!(session.try_add_client(0));
        assert_eq!(session.client_count(), 2);
        session.remove_client();
        assert_eq!(session.client_count(), 1);
        session.remove_client();
        assert_eq!(session.client_count(), 0);
    }
    #[tokio::test]
    async fn test_session_state_idle_detection() {
        let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
        let session = StreamSessionState::new("sess".to_string(), terminal, None, true);

        // No clients, no disconnect time yet → not idle
        assert!(!session.is_idle(Duration::from_secs(1)));

        // Add and remove a client to set disconnect time
        session.try_add_client(0);
        session.remove_client();

        // Just disconnected, should not be idle with long timeout
        assert!(!session.is_idle(Duration::from_secs(3600)));

        // Should be idle with zero timeout
        assert!(session.is_idle(Duration::from_secs(0)));
    }
    #[tokio::test]
    async fn test_session_registry_basic() {
        let registry = SessionRegistry::new(10);
        assert_eq!(registry.session_count(), 0);

        let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
        let session = Arc::new(StreamSessionState::new(
            "s1".to_string(),
            terminal,
            None,
            true,
        ));

        registry
            .insert("s1".to_string(), Arc::clone(&session))
            .unwrap();
        assert_eq!(registry.session_count(), 1);

        let retrieved = registry.get("s1");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, "s1");

        assert!(registry.get("s2").is_none());

        let removed = registry.remove("s1");
        assert!(removed.is_some());
        assert_eq!(registry.session_count(), 0);
    }
    #[tokio::test]
    async fn test_session_registry_max_sessions() {
        let registry = SessionRegistry::new(2);

        for i in 0..2 {
            let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
            let session = Arc::new(StreamSessionState::new(
                format!("s{}", i),
                terminal,
                None,
                true,
            ));
            registry.insert(format!("s{}", i), session).unwrap();
        }

        // Third insert should fail
        let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
        let session = Arc::new(StreamSessionState::new(
            "s2".to_string(),
            terminal,
            None,
            true,
        ));
        let result = registry.insert("s2".to_string(), session);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            StreamingError::MaxSessionsReached
        ));
    }
    #[tokio::test]
    async fn test_session_registry_list_sessions() {
        let registry = SessionRegistry::new(10);

        let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
        let session = Arc::new(StreamSessionState::new(
            "s1".to_string(),
            terminal,
            None,
            true,
        ));
        registry.insert("s1".to_string(), session).unwrap();

        let sessions = registry.list_sessions();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "s1");
        assert_eq!(sessions[0].cols, 80);
        assert_eq!(sessions[0].rows, 24);
    }
    #[tokio::test]
    async fn test_session_info_serialization() {
        let info = SessionInfo {
            id: "test".to_string(),
            created: 1234567890,
            clients: 2,
            idle_seconds: 0,
            cols: 80,
            rows: 24,
            cwd: Some("/home/user".to_string()),
            messages_sent: 0,
            bytes_sent: 0,
            input_bytes: 0,
            errors: 0,
            dropped_messages: 0,
        };

        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"id\":\"test\""));
        assert!(json.contains("\"clients\":2"));
        assert!(json.contains("\"cols\":80"));
    }
    #[tokio::test]
    async fn test_session_registry_get_nonexistent() {
        let registry = SessionRegistry::new(10);
        assert!(registry.get("nonexistent").is_none());
    }
    #[tokio::test]
    async fn test_session_registry_remove_existing() {
        let registry = SessionRegistry::new(10);
        let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
        let session = Arc::new(StreamSessionState::new(
            "test".to_string(),
            terminal,
            None,
            true,
        ));

        registry
            .insert("test".to_string(), Arc::clone(&session))
            .unwrap();
        assert_eq!(registry.session_count(), 1);

        let removed = registry.remove("test");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().id, "test");
        assert_eq!(registry.session_count(), 0);
    }
    #[tokio::test]
    async fn test_session_registry_remove_nonexistent() {
        let registry = SessionRegistry::new(10);
        assert!(registry.remove("nonexistent").is_none());
    }
    #[tokio::test]
    async fn test_session_registry_replace_existing() {
        let registry = SessionRegistry::new(2);
        let terminal1 = Arc::new(RwLock::new(Terminal::new(80, 24)));
        let session1 = Arc::new(StreamSessionState::new(
            "test".to_string(),
            terminal1,
            None,
            true,
        ));

        registry
            .insert("test".to_string(), Arc::clone(&session1))
            .unwrap();
        assert_eq!(registry.session_count(), 1);

        // Replace with new session (same ID, should not count toward limit)
        let terminal2 = Arc::new(RwLock::new(Terminal::new(100, 30)));
        let session2 = Arc::new(StreamSessionState::new(
            "test".to_string(),
            terminal2,
            None,
            true,
        ));
        let result = registry.insert("test".to_string(), session2);
        assert!(result.is_ok());
        assert_eq!(registry.session_count(), 1);

        let retrieved = registry.get("test").unwrap();
        assert_eq!(retrieved.terminal.write().grid.cols(), 100);
    }
    #[tokio::test]
    async fn test_session_registry_multiple_sessions() {
        let registry = SessionRegistry::new(10);

        for i in 0..5 {
            let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
            let session = Arc::new(StreamSessionState::new(
                format!("s{}", i),
                terminal,
                None,
                true,
            ));
            registry.insert(format!("s{}", i), session).unwrap();
        }
        assert_eq!(registry.session_count(), 5);

        // Verify all sessions can be retrieved
        for i in 0..5 {
            assert!(registry.get(&format!("s{}", i)).is_some());
        }

        // Remove some sessions
        registry.remove("s1");
        registry.remove("s3");
        assert_eq!(registry.session_count(), 3);

        assert!(registry.get("s0").is_some());
        assert!(registry.get("s1").is_none());
        assert!(registry.get("s2").is_some());
        assert!(registry.get("s3").is_none());
        assert!(registry.get("s4").is_some());
    }
    #[tokio::test]
    async fn test_session_registry_zero_capacity() {
        let registry = SessionRegistry::new(0);
        let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
        let session = Arc::new(StreamSessionState::new(
            "test".to_string(),
            terminal,
            None,
            true,
        ));

        let result = registry.insert("test".to_string(), session);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            StreamingError::MaxSessionsReached
        ));
    }
}
