//! Listeners: the HTTP/HTTPS app and the WebSocket-only accept loops.

use super::*;

impl StreamingServer {
    /// Build the full HTTP application (API routes + static frontend)
    /// shared by the plain and TLS listeners.
    ///
    /// API routes get the auth middleware when any auth method is
    /// configured; static file serving stays unprotected so the browser can
    /// load the page before authenticating the WebSocket. Every response —
    /// API and static alike — carries the anti-framing headers (SEC-010)
    /// and the CORS layer.
    #[cfg(feature = "streaming")]
    pub(super) fn build_http_app(self: &Arc<Self>) -> axum::Router {
        use axum::{routing::get, Router};
        use tower_http::services::ServeDir;

        // Build API routes (protected by auth)
        let api_routes = Router::new()
            .route("/ws", get(ws_handler))
            .route("/sessions", get(sessions_handler))
            .route("/stats", get(stats_ws_handler));

        // Apply auth middleware to API routes only if configured
        let auth_config = ApiAuthConfig {
            api_key: self.config.api_key.clone(),
            http_basic_auth: self.config.http_basic_auth.clone(),
            allow_api_key_in_query: self.config.allow_api_key_in_query,
        };
        let api_routes = if auth_config.is_configured() {
            api_routes.layer(axum::middleware::from_fn(move |req, next| {
                let auth_config = auth_config.clone();
                api_auth_middleware(req, next, auth_config)
            }))
        } else {
            api_routes
        };

        // Merge API routes with unprotected static file serving
        api_routes
            .fallback_service(ServeDir::new(&self.config.web_root))
            .with_state(self.clone())
            .layer(build_cors_layer(&self.config.allowed_origins))
            .layer(axum::middleware::from_fn(add_security_headers))
    }

    /// Start server with HTTP static file serving using Axum
    #[cfg(feature = "streaming")]
    pub(super) async fn start_with_http(self: Arc<Self>) -> Result<()> {
        crate::debug_info!("STREAMING", "Server with HTTP listening on {}", self.addr);

        self.spawn_default_broadcaster();
        self.spawn_idle_reaper();

        let app = self.build_http_app();

        // Start server
        let listener = tokio::net::TcpListener::bind(&self.addr)
            .await
            .map_err(|e| StreamingError::ServerError(format!("Failed to bind: {}", e)))?;

        axum::serve(listener, app.into_make_service())
            .await
            .map_err(|e| StreamingError::ServerError(format!("Server error: {}", e)))?;

        Ok(())
    }

    /// Start server with HTTPS/TLS static file serving using Axum
    #[cfg(feature = "streaming")]
    pub(super) async fn start_with_https(self: Arc<Self>) -> Result<()> {
        use axum_server::tls_rustls::RustlsConfig;

        let tls_config = self
            .config
            .tls
            .as_ref()
            .ok_or_else(|| StreamingError::ServerError("TLS config required".to_string()))?;

        crate::debug_info!(
            "STREAMING",
            "Server with HTTPS/TLS listening on {}",
            self.addr
        );

        self.spawn_default_broadcaster();
        self.spawn_idle_reaper();

        let app = self.build_http_app();

        // Build TLS config for axum-server
        let rustls_config = RustlsConfig::from_der(
            tls_config.certs.iter().map(|c| c.to_vec()).collect(),
            tls_config.key.secret_der().to_vec(),
        )
        .await
        .map_err(|e| StreamingError::ServerError(format!("Failed to create TLS config: {}", e)))?;

        // Parse address for axum-server
        let addr: std::net::SocketAddr = self.addr.parse().map_err(|e| {
            StreamingError::ServerError(format!("Invalid address '{}': {}", self.addr, e))
        })?;

        // Start HTTPS server
        axum_server::bind_rustls(addr, rustls_config)
            .serve(app.into_make_service())
            .await
            .map_err(|e| StreamingError::ServerError(format!("Server error: {}", e)))?;

        Ok(())
    }

    /// Start WebSocket-only server (original implementation)
    pub(super) async fn start_websocket_only(self: Arc<Self>) -> Result<()> {
        let listener = TcpListener::bind(&self.addr).await?;
        crate::debug_info!(
            "STREAMING",
            "WebSocket-only server listening on {}",
            self.addr
        );

        self.spawn_default_broadcaster();
        self.spawn_idle_reaper();

        self.accept_loop(listener, |stream| async move { Ok(stream) }, "Client")
            .await
    }

    /// Start WebSocket-only server with TLS (WSS)
    pub(super) async fn start_websocket_only_tls(self: Arc<Self>) -> Result<()> {
        let tls_config = self
            .config
            .tls
            .as_ref()
            .ok_or_else(|| StreamingError::ServerError("TLS config required".to_string()))?;

        let rustls_config = tls_config.build_rustls_config()?;
        let acceptor = TlsAcceptor::from(Arc::new(rustls_config));

        let listener = TcpListener::bind(&self.addr).await?;
        crate::debug_info!(
            "STREAMING",
            "WebSocket-only server with TLS (WSS) listening on {}",
            self.addr
        );

        self.spawn_default_broadcaster();
        self.spawn_idle_reaper();

        self.accept_loop(
            listener,
            move |stream| {
                let acceptor = acceptor.clone();
                async move { acceptor.accept(stream).await }
            },
            "TLS Client",
        )
        .await
    }

    /// The accept loop shared by the plain and TLS WebSocket-only
    /// listeners. `upgrade` is the transport handshake: identity for plain
    /// TCP, the TLS accept for WSS. `label` names the transport in logs.
    pub(super) async fn accept_loop<U, Fut, S>(
        self: Arc<Self>,
        listener: TcpListener,
        upgrade: U,
        label: &'static str,
    ) -> Result<()>
    where
        U: Fn(TcpStream) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = std::io::Result<S>> + Send + 'static,
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let upgrade = Arc::new(upgrade);
        loop {
            let (stream, addr) = match listener.accept().await {
                Ok(accepted) => accepted,
                Err(e) => {
                    crate::debug_error!("STREAMING", "Failed to accept connection: {}", e);
                    continue;
                }
            };
            if !self.can_accept_client() {
                crate::debug_error!(
                    "STREAMING",
                    "Max clients reached ({}), rejecting {} connection from {}",
                    self.config.max_clients,
                    label,
                    addr
                );
                continue;
            }
            if let Err(e) = stream.set_nodelay(true) {
                crate::debug_error!("STREAMING", "Failed to set TCP_NODELAY: {}", e);
            }
            crate::debug_info!("STREAMING", "New {} connection from {}", label, addr);
            let server = Arc::clone(&self);
            let upgrade = Arc::clone(&upgrade);
            tokio::spawn(async move {
                server
                    .serve_connection(stream, addr, upgrade.as_ref(), label)
                    .await;
            });
        }
    }

    /// One accepted TCP connection, from slot reservation to session end:
    /// the transport handshake and the WebSocket handshake (with
    /// header-callback auth), each under `WS_HANDSHAKE_TIMEOUT`, then the
    /// session.
    pub(super) async fn serve_connection<U, Fut, S>(
        self: &Arc<Self>,
        stream: TcpStream,
        addr: std::net::SocketAddr,
        upgrade: &U,
        label: &'static str,
    ) where
        U: Fn(TcpStream) -> Fut,
        Fut: std::future::Future<Output = std::io::Result<S>>,
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
    {
        // Reserve a global client slot for the whole handshake so
        // unauthenticated pre-upgrade connections cannot occupy tasks
        // indefinitely, uncapped by max_clients (SEC-004). The guard
        // releases on drop: handshake failure, timeout, or session end.
        if !self.try_add_client() {
            crate::debug_error!(
                "STREAMING",
                "Max clients reached ({}), rejecting {} connection from {}",
                self.config.max_clients,
                label,
                addr
            );
            return;
        }
        let global_guard = GlobalClientGuard { server: self };

        let stream = match tokio::time::timeout(WS_HANDSHAKE_TIMEOUT, upgrade(stream)).await {
            Ok(Ok(stream)) => stream,
            Ok(Err(e)) => {
                crate::debug_error!(
                    "STREAMING",
                    "{} transport handshake failed from {}: {}",
                    label,
                    addr,
                    e
                );
                return;
            }
            Err(_) => {
                crate::debug_error!(
                    "STREAMING",
                    "{} transport handshake timed out from {} after {}s",
                    label,
                    addr,
                    WS_HANDSHAKE_TIMEOUT.as_secs()
                );
                return;
            }
        };

        // Accept WebSocket with header callback to capture URI query and validate auth
        let (header_callback, uri_query) = build_ws_header_callback(
            self.config.api_key.clone(),
            self.config.http_basic_auth.clone(),
            self.config.allowed_origins.clone(),
            self.config.allow_api_key_in_query,
        );
        // The tungstenite `Callback` trait fixes `ErrorResponse` as
        // `HttpResponse<Option<String>>` — we cannot box or shrink it
        // without violating the external API contract.
        let ws_stream = match tokio::time::timeout(
            WS_HANDSHAKE_TIMEOUT,
            accept_hdr_async_with_config(stream, header_callback, ws_accept_config()),
        )
        .await
        {
            Ok(Ok(ws_stream)) => ws_stream,
            Ok(Err(e)) => {
                crate::debug_error!(
                    "STREAMING",
                    "{} WebSocket handshake failed from {}: {}",
                    label,
                    addr,
                    e
                );
                return;
            }
            Err(_) => {
                crate::debug_error!(
                    "STREAMING",
                    "{} WebSocket handshake timed out from {} after {}s",
                    label,
                    addr,
                    WS_HANDSHAKE_TIMEOUT.as_secs()
                );
                return;
            }
        };

        let query_str = uri_query.lock().take();
        let params = ConnectionParams::from_uri_query(query_str.as_deref());
        if let Err(e) = self
            .handle_ws_connection(ws_stream, &params, global_guard, label)
            .await
        {
            crate::debug_error!(
                "STREAMING",
                "{} connection error from {}: {}",
                label,
                addr,
                e
            );
        }
    }
}
