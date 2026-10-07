//! Client admission counting, broadcasts, roster snapshots, session
//! lookup/resolution, the idle reaper, and broadcaster health.

use super::*;

impl StreamingServer {
    /// Check if the server can accept more clients
    pub(super) fn can_accept_client(&self) -> bool {
        self.client_count.load(Ordering::Relaxed) < self.config.max_clients
    }

    /// Increment the client count. Returns false if max_clients would be exceeded.
    pub(super) fn try_add_client(&self) -> bool {
        loop {
            let current = self.client_count.load(Ordering::Relaxed);
            if current >= self.config.max_clients {
                return false;
            }
            match self.client_count.compare_exchange(
                current,
                current + 1,
                Ordering::SeqCst,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(_) => continue,
            }
        }
    }

    /// Decrement the client count
    pub(super) fn remove_client(&self) {
        self.client_count.fetch_sub(1, Ordering::SeqCst);
    }

    /// Broadcast a message to all clients in the default session
    pub fn broadcast(&self, msg: ServerMessage) {
        if let Some(ref session) = self.default_session {
            session.broadcast(msg);
        }
    }

    /// Broadcast a message to the clients of every session
    pub fn broadcast_all(&self, msg: ServerMessage) {
        for session in self.sessions.all() {
            session.broadcast(msg.clone());
        }
    }

    /// Attach the par-mux roster watcher whose snapshot every connecting
    /// client receives. First call wins.
    #[cfg(feature = "mux")]
    pub fn set_roster_watcher(&self, watcher: Arc<crate::streaming::RosterWatcher>) {
        let _ = self.roster.set(watcher);
    }

    /// The roster snapshot to send a freshly connected client, if a watcher is attached.
    pub(super) fn roster_snapshot(&self) -> Option<ServerMessage> {
        #[cfg(feature = "mux")]
        {
            self.roster.get().map(|w| w.snapshot())
        }
        #[cfg(not(feature = "mux"))]
        {
            None
        }
    }

    /// Send a message to a specific session
    pub fn send_to_session(&self, session_id: &str, msg: ServerMessage) {
        if let Some(session) = self.sessions.get(session_id) {
            session.broadcast(msg);
        }
    }

    /// Broadcast a message to all clients of a specific session
    pub fn broadcast_to_session(&self, session_id: &str, msg: ServerMessage) {
        if let Some(session) = self.sessions.get(session_id) {
            let _ = session.broadcast_tx.send(msg);
        } else if let Some(ref session) = self.default_session {
            let _ = session.broadcast_tx.send(msg);
        }
    }

    /// Get a session by ID from the registry
    pub fn get_session(&self, session_id: &str) -> Option<Arc<StreamSessionState>> {
        self.sessions.get(session_id)
    }

    /// Close a session: remove from registry, shut it down, and tear down factory resources.
    /// Factory teardown is delayed 500ms so clients receive the shutdown message.
    pub fn close_session(&self, session_id: &str, reason: String) -> bool {
        if let Some(session) = self.sessions.remove(session_id) {
            session.shutdown(reason);
            if let Some(ref factory) = self.session_factory {
                let factory = Arc::clone(factory);
                let id = session_id.to_string();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    factory.teardown_session(&id);
                });
            }
            crate::debug_info!("STREAMING", "Closed session: {}", session_id);
            true
        } else {
            false
        }
    }

    /// Resolve a session from connection parameters
    ///
    /// 1. If session already exists in registry, return it
    /// 2. If factory is available, create a new session
    /// 3. If no factory and id == "default", return default session
    /// 4. Otherwise, error
    pub fn resolve_session(
        self: &Arc<Self>,
        params: &ConnectionParams,
    ) -> Result<Arc<StreamSessionState>> {
        let session_id = &params.session_id;

        // Check if session already exists
        if let Some(session) = self.sessions.get(session_id) {
            return Ok(session);
        }

        // Try to create via factory
        if let Some(ref factory) = self.session_factory {
            // Resolve shell command from preset if specified
            let shell_command = if let Some(ref preset_name) = params.preset {
                let cmd = self
                    .config
                    .presets
                    .get(preset_name)
                    .ok_or_else(|| StreamingError::InvalidPreset(preset_name.clone()))?;
                Some(cmd.as_str())
            } else {
                None
            };

            // Get terminal size from config or defaults
            let cols = if self.config.initial_cols > 0 {
                self.config.initial_cols
            } else {
                80
            };
            let rows = if self.config.initial_rows > 0 {
                self.config.initial_rows
            } else {
                24
            };

            let (cols, rows) = validate_terminal_size(cols, rows)?;

            let result = factory.create_session(session_id, cols, rows, shell_command)?;

            // Apply the configured Kitty file-media gate to every session
            // terminal the server creates (SEC-101): PTY output is untrusted
            // input, so `t=t`/`t=f` APCs must not read/delete arbitrary files.
            result
                .terminal
                .write()
                .set_allow_file_media(self.config.kitty_file_media);

            let session = Arc::new(StreamSessionState::new(
                session_id.clone(),
                result.terminal,
                self.theme.clone(),
                self.config.send_initial_screen,
            ));

            if let Some(writer) = result.pty_writer {
                session.set_pty_writer(writer);
            }

            // Insert into registry
            self.sessions
                .insert(session_id.clone(), Arc::clone(&session))?;

            // Setup session (spawn background tasks, etc.)
            factory.setup_session(session_id, &session)?;

            // Spawn broadcaster loop for this session
            let session_clone = Arc::clone(&session);
            tokio::spawn(async move {
                session_clone.output_broadcaster_loop().await;
            });

            return Ok(session);
        }

        // No factory - check if asking for default
        if session_id == "default" {
            if let Some(ref default) = self.default_session {
                return Ok(Arc::clone(default));
            }
        }

        Err(StreamingError::SessionNotFound(session_id.clone()))
    }

    /// [`Self::resolve_session`] for async callers.
    ///
    /// Creating a session runs the factory, which blocks on process spawns
    /// or daemon round trips (`MuxSessionFactory`); that work runs on the
    /// blocking pool so a slow or wedged backend cannot park runtime
    /// workers. Resolving an existing session, or the default session of a
    /// factory-less server, does no I/O and stays inline.
    pub(super) async fn resolve_session_off_runtime(
        self: &Arc<Self>,
        params: &ConnectionParams,
    ) -> Result<Arc<StreamSessionState>> {
        if self.session_factory.is_none() || self.sessions.get(&params.session_id).is_some() {
            return self.resolve_session(params);
        }
        let this = Arc::clone(self);
        let params = params.clone();
        tokio::task::spawn_blocking(move || this.resolve_session(&params))
            .await
            .map_err(|e| {
                StreamingError::ServerError(format!("session creation task failed: {e}"))
            })?
    }

    /// Spawn the session reaper task (always runs for dead session cleanup)
    pub(super) fn spawn_idle_reaper(self: &Arc<Self>) {
        let server = Arc::clone(self);
        tokio::spawn(async move {
            server.session_reaper().await;
        });
    }

    /// Session reaper - periodically checks for idle and dead sessions
    pub(super) async fn session_reaper(self: Arc<Self>) {
        let idle_timeout = if self.config.session_idle_timeout > 0 {
            Some(Duration::from_secs(self.config.session_idle_timeout))
        } else {
            None
        };
        let mut interval = tokio::time::interval(Duration::from_secs(30));

        loop {
            interval.tick().await;

            // Idle timeout reaping (if configured)
            if let Some(timeout) = idle_timeout {
                let idle_ids = self.sessions.idle_sessions(timeout);
                for id in idle_ids {
                    // Allow reaping default in factory mode only
                    if id == "default" && self.session_factory.is_none() {
                        continue;
                    }
                    if self.close_session(&id, "Session idle timeout".to_string()) {
                        crate::debug_info!("STREAMING", "Reaped idle session: {}", id);
                    }
                }
            }

            // Dead session reaping (always)
            self.reap_dead_sessions();

            // Broadcaster health check
            self.check_broadcaster_health();
        }
    }

    /// Reap sessions whose PTY process has exited and have no clients
    pub(super) fn reap_dead_sessions(&self) {
        if let Some(ref factory) = self.session_factory {
            let session_ids: Vec<String> = self
                .sessions
                .list_sessions()
                .iter()
                .filter(|s| s.clients == 0)
                .map(|s| s.id.clone())
                .collect();
            for id in session_ids {
                if !factory.is_session_alive(&id)
                    && self.close_session(&id, "Dead session (PTY exited)".to_string())
                {
                    crate::debug_info!("STREAMING", "Reaped dead session: {}", id);
                }
            }
        }
    }

    /// Check broadcaster health — warn if no broadcasts for 30s with active clients
    pub(super) fn check_broadcaster_health(&self) {
        let now = now_millis();
        for info in self.sessions.list_sessions() {
            if info.clients > 0 {
                if let Some(session) = self.sessions.get(&info.id) {
                    let last = session.metrics.last_broadcast_time.load(Ordering::Relaxed);
                    if last > 0 && now.saturating_sub(last) > 30_000 {
                        crate::debug_error!(
                            "STREAMING",
                            "Session {} broadcaster may be stalled ({}s since last broadcast, {} clients)",
                            info.id,
                            (now - last) / 1000,
                            info.clients
                        );
                    }
                }
            }
        }
    }

    /// Spawn broadcaster loop for the default session
    pub(super) fn spawn_default_broadcaster(self: &Arc<Self>) {
        if let Some(ref session) = self.default_session {
            let session = Arc::clone(session);
            tokio::spawn(async move {
                session.output_broadcaster_loop().await;
            });
        }
    }
}
