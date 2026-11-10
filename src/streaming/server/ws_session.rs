//! One WebSocket client session: handshake follow-up, session binding,
//! and the bidirectional run loop (tungstenite and axum transports).

use super::*;

impl StreamingServer {
    #[cfg(test)]
    // Only the mux-gated mirror tests call this; the streaming-only build
    // otherwise lints the helper as dead (same shape as parse_titles_output).
    #[cfg_attr(not(feature = "mux"), allow(dead_code))]
    pub(crate) async fn handle_test_ws<S>(
        self: &Arc<Self>,
        stream: tokio_tungstenite::WebSocketStream<S>,
        params: &ConnectionParams,
    ) -> Result<()>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
    {
        assert!(self.try_add_client());
        self.handle_ws_connection(stream, params, GlobalClientGuard { server: self }, "test")
            .await
    }

    /// Run a session over an upgraded tungstenite WebSocket (plain or TLS).
    pub(super) async fn handle_ws_connection<S>(
        self: &Arc<Self>,
        ws_stream: tokio_tungstenite::WebSocketStream<S>,
        params: &ConnectionParams,
        global_guard: GlobalClientGuard<'_>,
        label: &'static str,
    ) -> Result<()>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
    {
        let (session, _global_guard, _session_guard, read_only) =
            self.prepare_ws_session(params, global_guard).await?;
        let client = Client::new(ws_stream, read_only);
        self.run_ws_session(client, session, read_only, label).await
    }

    /// Common pre-loop setup shared by both tungstenite WebSocket handlers.
    ///
    /// The global client slot must already be reserved by the caller
    /// (`try_add_client` before the handshake, SEC-004) — the guard is
    /// passed in and held for the connection's lifetime. This resolves the
    /// session (off the runtime when it must be created), reserves the
    /// per-session slot (returning an RAII guard whose `Drop` releases it),
    /// and computes the read-only flag. The caller wraps the accepted stream
    /// in a `Client<S>` and hands it to `run_ws_session`.
    pub(super) async fn prepare_ws_session<'s>(
        self: &'s Arc<Self>,
        params: &ConnectionParams,
        global_guard: GlobalClientGuard<'s>,
    ) -> Result<(
        Arc<StreamSessionState>,
        GlobalClientGuard<'s>,
        SessionClientGuard,
        bool,
    )> {
        let read_only = params.readonly || params.metadata || self.config.default_read_only;
        let session = if params.metadata {
            self.metadata_session_off_runtime().await?
        } else {
            self.resolve_session_off_runtime(params).await?
        };
        let session_guard = SessionClientGuard {
            session: Arc::clone(&session),
            server: Arc::clone(self),
        };
        Ok((session, global_guard, session_guard, read_only))
    }

    /// The `?metadata=1` connection state: a factory-free session under
    /// the reserved registry id `metadata` — no pane mirror is created
    /// (and none of the daemon's sizing involves this connection). It
    /// exists only to route the metadata broadcasts (`mux_tree_snapshot`,
    /// the roster) to the client and to give the shared dispatch loop a
    /// write-refusing target; the write paths are refused by the forced
    /// read-only flag, and the tree reply reads the watcher cache
    /// directly. The state disappears with its last client
    /// (`close_on_last_disconnect`), so `/sessions` lists it only while a
    /// metadata listener is actually connected.
    async fn metadata_session_off_runtime(self: &Arc<Self>) -> Result<Arc<StreamSessionState>> {
        let this = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            let _lifecycle = this.session_lifecycle.lock();
            let session = match this.sessions.get("metadata") {
                Some(session) => session,
                None => {
                    // A fresh in-memory terminal: nothing mirrors a pane,
                    // no PTY writer exists, and the idle/dead reapers
                    // treat the state like any other client-less session.
                    let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
                    let session = Arc::new(StreamSessionState::new(
                        "metadata".to_string(),
                        terminal,
                        None,
                        false,
                    ));
                    let _ = this
                        .sessions
                        .insert("metadata".to_string(), Arc::clone(&session));
                    session
                }
            };
            // Same admission accounting the factory path does: the
            // SessionClientGuard's drop pairs with it.
            if !session.try_add_client(this.config.max_clients_per_session) {
                return Err(StreamingError::MaxClientsReached);
            }
            Ok(session)
        })
        .await
        .map_err(|e| StreamingError::ServerError(format!("metadata session failed: {e}")))?
    }

    /// The one session loop for every WebSocket transport (tungstenite
    /// plain and TLS, and the axum HTTP-served route): connect and mode-sync
    /// messages, then client messages, session broadcasts and keepalive
    /// until the client leaves, then the closing handshake. Frame encoding
    /// and policy live in the [`WsTransport`]. `transport_label` is used
    /// only in debug logs so the transports remain distinguishable.
    ///
    /// Client messages are dispatched via [`Self::handle_client_message`].
    pub(super) async fn run_ws_session<T: WsTransport>(
        self: &Arc<Self>,
        mut client: T,
        session: Arc<StreamSessionState>,
        read_only: bool,
        transport_label: &'static str,
    ) -> Result<()> {
        let client_id = client.id();
        struct ViewerGuard(Arc<StreamSessionState>, uuid::Uuid);
        impl Drop for ViewerGuard {
            fn drop(&mut self) {
                self.0
                    .metrics
                    .last_disconnect_time
                    .store(now_millis(), Ordering::Relaxed);
                self.0.viewer_leave(self.1);
                crate::streaming::metrics::Metrics::global().client_disconnected(&self.0.id);
            }
        }
        session.viewer_join(client_id, "web");
        let _viewer = ViewerGuard(Arc::clone(&session), client_id);

        // Reconnect heuristic: the session's previous client left within the
        // reconnect window (kanban 01a18331). The wire has no stable client
        // identity, so the session is the identity.
        let last_leave = session.metrics.last_disconnect_time.load(Ordering::Relaxed);
        let reconnect = last_leave > 0
            && now_millis().saturating_sub(last_leave)
                <= crate::streaming::metrics::RECONNECT_WINDOW_MS;
        crate::streaming::metrics::Metrics::global().client_connected(
            &session.id,
            transport_label,
            reconnect,
        );

        // Send initial connection message. The pane-create signal mirrors
        // the `handle_pane_create` refusal ladder's first two rungs: the
        // allow flag AND the writable requirement (read-only forces false).
        let connect_msg = session.build_connect_message(
            &client_id.to_string(),
            read_only,
            self.config.allow_pane_create,
        );
        client.send(connect_msg).await?;

        // Sync terminal mode state for existing sessions
        for mode_msg in session.build_mode_sync_messages() {
            client.send(mode_msg).await?;
        }

        crate::debug_info!(
            "STREAMING",
            "{} {} connected to session {} (total: {})",
            transport_label,
            client_id,
            session.id,
            self.client_count()
        );

        // Subscribe to session broadcasts
        let mut output_rx = session.broadcast_tx.subscribe();
        if let Some(status) = session.pane_size_status() {
            client.send(status).await?;
        }
        // The mirrored pane's process lifecycle, same stored-snapshot
        // ordering argument as the size status: the mirror updates its
        // state before it broadcasts a transition, so a transition this
        // send races is queued on the subscription above and corrects it.
        if let Some(process) = session.pane_process_status() {
            client.send(process).await?;
        }
        client.send(session.viewer_presence()).await?;

        // Roster snapshot. Ordering: the subscription above precedes the
        // snapshot read. The watcher updates its cache before it broadcasts
        // a delta, so any delta this snapshot misses is already queued on
        // `output_rx`, and any delta it includes is at worst replayed from
        // the queue. Deltas carry a whole entry, so apply-then-replace
        // converges on the same state.
        if let Some(roster) = self.roster_snapshot() {
            client.send(roster).await?;
        }

        // Tree snapshot, same ordering argument as the roster: the
        // subscription precedes the read, the watcher updates its cache
        // before it broadcasts, and every push is a whole replacement —
        // so a missed or replayed push converges on the same state.
        if let Some(tree) = self.tree_snapshot() {
            client.send(tree).await?;
        }

        // Setup keepalive timer
        let keepalive_interval = if self.config.keepalive_interval > 0 {
            Some(Duration::from_secs(self.config.keepalive_interval))
        } else {
            None
        };
        let mut keepalive_timer = keepalive_interval.map(|d| tokio::time::interval(d));
        let mut subscriptions: Option<
            std::collections::HashSet<crate::streaming::protocol::EventType>,
        > = None;
        let mut rate_limiter = if self.config.input_rate_limit_bytes_per_sec > 0 {
            Some(InputRateLimiter::new(
                self.config.input_rate_limit_bytes_per_sec,
            ))
        } else {
            None
        };

        loop {
            tokio::select! {
                msg = client.recv() => {
                    match msg {
                        Err(e) => {
                            crate::debug_error!("STREAMING", "{} {} error: {}", transport_label, client_id, e);
                            break;
                        }
                        Ok(msg_opt) => match msg_opt {
                        Some(client_msg) => {
                            let replies = self
                                .handle_client_message(
                                    &session,
                                    ConnCtx {
                                        transport_label,
                                        client_id,
                                        read_only,
                                    },
                                    &mut subscriptions,
                                    &mut rate_limiter,
                                    client_msg,
                                )
                                .await;
                            for reply in replies {
                                if let Err(e) = client.send(reply).await {
                                    crate::debug_error!(
                                        "STREAMING",
                                        "Failed to send reply to {} {}: {}",
                                        transport_label,
                                        client_id,
                                        e
                                    );
                                }
                            }
                        }
                        None => {
                            crate::debug_info!("STREAMING", "{} {} disconnected from session {}", transport_label, client_id, session.id);
                            break;
                        }
                        }
                    }
                }

                output_msg = output_rx.recv() => {
                    let msg = match output_msg {
                        Ok(msg) => msg,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            // Dropped frames may include pane output, and a
                            // lost output frame is the bare-ESC corruption:
                            // an escape byte riding the dropped chunk never
                            // reaches the client's parser, so the sequence
                            // tail paints as literal text. Push the current
                            // screen as an in-band resync (the same
                            // screen-restore vehicle a workspace retarget
                            // uses); frames still queued behind the lag
                            // replay over it, which is the convergence shape
                            // the roster/tree resync below already relies on.
                            let resync = session.screen_resync_frame();
                            if client.send(resync).await.is_err() {
                                break;
                            }
                            if let Some(status) = session.pane_size_status() {
                                client.send(status).await?;
                            }
                            client.send(session.viewer_presence()).await?;
                            // Dropped messages may include roster deltas or
                            // tree pushes: resync both. Every snapshot is a
                            // whole replacement, so send-then-overwrite
                            // converges on the current state.
                            if let Some(roster) = self.roster_snapshot() {
                                if client.send(roster).await.is_err() {
                                    break;
                                }
                            }
                            match self.tree_snapshot() {
                                Some(tree) => tree,
                                None => continue,
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    };
                    if should_send(&msg, &subscriptions) {
                        let send_started = std::time::Instant::now();
                        if client.send(msg).await.is_err() {
                            break;
                        }
                        crate::streaming::metrics::Metrics::global().record_send_latency_ms(
                            send_started.elapsed().as_secs_f64() * 1000.0,
                        );
                    }
                }

                _ = async {
                    if let Some(ref mut timer) = keepalive_timer {
                        timer.tick().await
                    } else {
                        std::future::pending::<tokio::time::Instant>().await
                    }
                } => {
                    if let Err(e) = client.ping().await {
                        crate::debug_error!("STREAMING", "Failed to ping {} {}: {}", transport_label, client_id, e);
                        break;
                    }
                }
            }
        }

        crate::debug_info!(
            "STREAMING",
            "{} {} cleanup (remaining: {})",
            transport_label,
            client_id,
            self.client_count() - 1
        );

        // Complete the closing handshake: reply with a Close frame before the
        // stream drops. Without it the TCP connection just FINs, and clients
        // with a full receive queue (websockets stops reading once its queue
        // backs up) never see the close and run out their full close timeout.
        // Best-effort: the connection may already be dead on error break paths.
        if let Err(e) = client.close().await {
            crate::debug_error!(
                "STREAMING",
                "Failed to send close reply to {} {}: {}",
                transport_label,
                client_id,
                e
            );
        }

        Ok(())
    }

    /// Handle Axum WebSocket connection
    pub(super) async fn handle_axum_websocket(
        self: &Arc<Self>,
        socket: axum::extract::ws::WebSocket,
        params: ConnectionParams,
    ) -> Result<()> {
        // Reserve the global client slot BEFORE resolving or creating a
        // session, so max_clients bounds session spawns too (SEC-011). The
        // guard releases on any early return below (guard → resolve →
        // per-session slot, dropped in reverse order).
        if !self.try_add_client() {
            return Err(StreamingError::MaxClientsReached);
        }
        let (session, _global_guard, _session_guard, read_only) = self
            .prepare_ws_session(&params, GlobalClientGuard { server: self })
            .await?;
        let transport = AxumTransport {
            id: uuid::Uuid::new_v4(),
            socket,
        };
        self.run_ws_session(transport, session, read_only, "Axum WebSocket")
            .await
    }
}
