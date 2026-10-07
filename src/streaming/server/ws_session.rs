//! One WebSocket client session: handshake follow-up, session binding,
//! and the bidirectional run loop (tungstenite and axum transports).

use super::*;

impl StreamingServer {
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
        let session = self.resolve_session_off_runtime(params).await?;
        if !session.try_add_client(self.config.max_clients_per_session) {
            return Err(StreamingError::MaxClientsReached);
        }
        let session_guard = SessionClientGuard {
            session: Arc::clone(&session),
        };
        let read_only = params.readonly || self.config.default_read_only;
        Ok((session, global_guard, session_guard, read_only))
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

        // Send initial connection message
        let connect_msg = session.build_connect_message(&client_id.to_string(), read_only);
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

        // Roster snapshot. Ordering: the subscription above precedes the
        // snapshot read. The watcher updates its cache before it broadcasts
        // a delta, so any delta this snapshot misses is already queued on
        // `output_rx`, and any delta it includes is at worst replayed from
        // the queue. Deltas carry a whole entry, so apply-then-replace
        // converges on the same state.
        if let Some(roster) = self.roster_snapshot() {
            client.send(roster).await?;
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
                            let replies = self.handle_client_message(
                                &session,
                                ConnCtx {
                                    transport_label,
                                    client_id,
                                    read_only,
                                },
                                &mut subscriptions,
                                &mut rate_limiter,
                                client_msg,
                            );
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
                            // Dropped messages may include roster deltas: resync.
                            match self.roster_snapshot() {
                                Some(roster) => roster,
                                None => continue,
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    };
                    if should_send(&msg, &subscriptions)
                        && client.send(msg).await.is_err() {
                            break;
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
