//! Per-message client handlers: refresh/snapshot builders, input, resize,
//! subscribe, mouse, focus, paste, selection, clipboard.

use super::*;

impl StreamingServer {
    /// Build a refresh message from the session's current visible terminal
    /// state. Pure (no side effects, no async) so the tungstenite and axum
    /// WebSocket handlers share one implementation.
    pub(super) fn build_refresh_message(
        terminal_for_refresh: &Arc<RwLock<Terminal>>,
    ) -> Option<ServerMessage> {
        let terminal = terminal_for_refresh.read();
        let content = terminal.export_visible_screen_styled();
        let (cols, rows) = terminal.size();
        Some(ServerMessage::refresh(cols as u16, rows as u16, content))
    }

    /// Build a semantic-snapshot message (or an error message for an invalid
    /// scope). Pure (no side effects, no async) so the tungstenite and axum
    /// WebSocket handlers share one implementation.
    ///
    /// Returns `Some(msg)` for a valid scope (including the snapshot payload),
    /// `Some(error_msg)` for an invalid scope string, and `None` only when a
    /// valid scope produced no payload (currently never).
    pub(super) fn build_snapshot_message(
        terminal_for_refresh: &Arc<RwLock<Terminal>>,
        scope: &str,
        max_commands: Option<u32>,
    ) -> ServerMessage {
        use crate::terminal::semantic_snapshot::SnapshotScope;
        match scope {
            "visible" => {
                let terminal = terminal_for_refresh.read();
                ServerMessage::semantic_snapshot(
                    terminal.get_semantic_snapshot_json(SnapshotScope::Visible),
                )
            }
            "recent" => {
                let n = max_commands.unwrap_or(10) as usize;
                let terminal = terminal_for_refresh.read();
                ServerMessage::semantic_snapshot(
                    terminal.get_semantic_snapshot_json(SnapshotScope::Recent(n)),
                )
            }
            "full" => {
                let terminal = terminal_for_refresh.read();
                ServerMessage::semantic_snapshot(
                    terminal.get_semantic_snapshot_json(SnapshotScope::Full),
                )
            }
            other => ServerMessage::error(format!(
                "Invalid snapshot scope '{}': must be 'visible', 'recent', or 'full'",
                other
            )),
        }
    }

    /// Handle one decoded `ClientMessage` from a connected client.
    ///
    /// This is the single arm-set for client-message dispatch, shared by every
    /// WebSocket transport (the tungstenite `run_ws_session` loop and the
    /// axum HTTP-served loop). Each arm delegates to one per-variant handler
    /// below (`handle_input`, `handle_resize`, …; `RequestRefresh` and
    /// `SnapshotRequest` already live in their pure `build_*` builders). The
    /// match is exhaustive over `ClientMessage` with no wildcard arm on
    /// purpose: adding a variant must be handled here — once — and forgetting
    /// an arm is a compile error instead of a silently dropped message.
    /// (ARC-001: the axum path previously dropped
    /// Mouse/FocusChange/Paste/SelectionRequest/ClipboardRequest via a
    /// `_ => {}` wildcard.)
    ///
    /// `ctx` is the connection's identity and write permission.
    /// `subscriptions` and `rate_limiter` are the caller's per-connection
    /// state, mutated in place. Returns the direct replies to send back to
    /// this client; messages that only write to the PTY or broadcast to the
    /// session produce no direct reply.
    pub(super) fn handle_client_message(
        self: &Arc<Self>,
        session: &Arc<StreamSessionState>,
        ctx: ConnCtx<'_>,
        subscriptions: &mut Option<
            std::collections::HashSet<crate::streaming::protocol::EventType>,
        >,
        rate_limiter: &mut Option<InputRateLimiter>,
        msg: crate::streaming::protocol::ClientMessage,
    ) -> Vec<ServerMessage> {
        let ConnCtx {
            transport_label,
            client_id,
            read_only,
        } = ctx;
        // Input on a session whose PTY writer is detached (never attached,
        // or detached by shutdown) has nowhere to go: it is dropped, logged
        // and counted in dropped_messages, and the connection stays open.
        // Closing it would not help: the session is still writer-less when
        // the client reconnects, so a reconnecting client loops.
        if !read_only
            && !Self::session_has_writer(session)
            && matches!(
                msg,
                crate::streaming::protocol::ClientMessage::Input { .. }
                    | crate::streaming::protocol::ClientMessage::Paste { .. }
                    | crate::streaming::protocol::ClientMessage::Mouse { .. }
                    | crate::streaming::protocol::ClientMessage::FocusChange { .. }
            )
        {
            crate::debug_error!(
                "STREAMING",
                "Dropping input from {} {} on session {} with no PTY writer",
                transport_label,
                client_id,
                session.id
            );
            session
                .metrics
                .dropped_messages
                .fetch_add(1, Ordering::Relaxed);
            return Vec::new();
        }
        let mut replies = Vec::new();
        match msg {
            crate::streaming::protocol::ClientMessage::Input { data } => {
                Self::handle_input(
                    session,
                    transport_label,
                    client_id,
                    read_only,
                    rate_limiter,
                    data,
                );
            }
            crate::streaming::protocol::ClientMessage::Resize { cols, rows } => {
                Self::handle_resize(session, transport_label, client_id, read_only, cols, rows);
            }
            crate::streaming::protocol::ClientMessage::Ping => {
                replies.push(Self::handle_ping());
            }
            crate::streaming::protocol::ClientMessage::RequestRefresh => {
                if let Some(msg) = Self::build_refresh_message(&session.terminal) {
                    replies.push(msg);
                }
            }
            crate::streaming::protocol::ClientMessage::Subscribe { events } => {
                Self::handle_subscribe(subscriptions, events);
            }
            crate::streaming::protocol::ClientMessage::Mouse {
                col,
                row,
                button,
                shift,
                ctrl,
                alt,
                event_type,
            } => {
                Self::handle_mouse(
                    session,
                    read_only,
                    MouseInput {
                        col,
                        row,
                        button,
                        shift,
                        ctrl,
                        alt,
                        event_type,
                    },
                );
            }
            crate::streaming::protocol::ClientMessage::FocusChange { focused } => {
                Self::handle_focus_change(session, read_only, focused);
            }
            crate::streaming::protocol::ClientMessage::Paste { content } => {
                Self::handle_paste(
                    session,
                    transport_label,
                    client_id,
                    read_only,
                    rate_limiter,
                    content,
                );
            }
            crate::streaming::protocol::ClientMessage::SelectionRequest {
                start_col,
                start_row,
                end_col,
                end_row,
                mode,
            } => {
                self.handle_selection_request(
                    session,
                    ctx,
                    (start_col, start_row),
                    (end_col, end_row),
                    mode,
                );
            }
            crate::streaming::protocol::ClientMessage::ClipboardRequest {
                operation,
                content,
                target,
            } => {
                replies.extend(
                    self.handle_clipboard_request(session, ctx, operation, content, target),
                );
            }
            crate::streaming::protocol::ClientMessage::SnapshotRequest {
                scope,
                max_commands,
            } => {
                let msg = Self::build_snapshot_message(&session.terminal, &scope, max_commands);
                replies.push(msg);
            }
        }
        replies
    }

    /// True when a PTY writer is attached to the session. Input-bearing
    /// handlers check this before touching terminal state so a session whose
    /// process has exited never mutates the terminal (e.g. mouse click
    /// tracking) or counts bytes for a PTY nobody will read.
    pub(super) fn session_has_writer(session: &Arc<StreamSessionState>) -> bool {
        session.pty_writer.read().is_some()
    }

    /// The one client→PTY write path (QA-110), shared by every
    /// input-bearing handler: enqueue onto the session's serialized input
    /// path. One blocking drain task owns the PTY writes, so arrival order
    /// is write order, and no async worker can be stalled by a blocking PTY
    /// write (SEC-005). No handler may reach the session channel directly —
    /// a second write site is how the Mouse/FocusChange arms drifted before
    /// QA-110.
    pub(super) fn enqueue_input(session: &Arc<StreamSessionState>, bytes: Vec<u8>) {
        session.enqueue_pty_input(bytes);
    }

    /// `Input`: cap, rate-limit, and write keystrokes to the PTY through the
    /// shared input path.
    pub(super) fn handle_input(
        session: &Arc<StreamSessionState>,
        transport_label: &str,
        client_id: uuid::Uuid,
        read_only: bool,
        rate_limiter: &mut Option<InputRateLimiter>,
        data: String,
    ) {
        if read_only {
            return;
        }
        if data.len() > MAX_INPUT_PAYLOAD_BYTES {
            crate::debug_error!(
                "STREAMING",
                "Dropping oversize Input from {} {} ({} bytes > {} cap)",
                transport_label,
                client_id,
                data.len(),
                MAX_INPUT_PAYLOAD_BYTES
            );
            session
                .metrics
                .dropped_messages
                .fetch_add(1, Ordering::Relaxed);
            return;
        }
        if let Some(ref mut limiter) = rate_limiter {
            if !limiter.try_consume(data.len()) {
                crate::debug_error!(
                    "STREAMING",
                    "Rate limit exceeded for {} {}",
                    transport_label,
                    client_id
                );
                session
                    .metrics
                    .dropped_messages
                    .fetch_add(1, Ordering::Relaxed);
                return;
            }
        }
        if Self::session_has_writer(session) {
            session
                .metrics
                .input_bytes
                .fetch_add(data.len(), Ordering::Relaxed);
            Self::enqueue_input(session, data.into_bytes());
        } else {
            // The no-writer guard in `handle_client_message` drops input
            // before dispatch; reaching here means the writer detached
            // between that check and this one. Count it so the metric never
            // reports a drop-free session the logs disagree with.
            crate::debug_error!(
                "STREAMING",
                "Input from {} {} dropped: session {} lost its PTY writer mid-dispatch",
                transport_label,
                client_id,
                session.id
            );
            session
                .metrics
                .dropped_messages
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    /// `Resize`: validate the requested size and forward it to the session's
    /// resize channel. Invalid sizes are logged and dropped.
    pub(super) fn handle_resize(
        session: &Arc<StreamSessionState>,
        transport_label: &str,
        client_id: uuid::Uuid,
        read_only: bool,
        cols: u16,
        rows: u16,
    ) {
        if read_only {
            return;
        }
        if let Err(e) = validate_terminal_size(cols, rows) {
            crate::debug_error!(
                "STREAMING",
                "{} {} sent invalid resize: {}",
                transport_label,
                client_id,
                e
            );
        } else {
            let _ = session.resize_tx.send((cols, rows));
        }
    }

    /// `Ping`: keepalive, answered with a single `Pong` reply.
    pub(super) fn handle_ping() -> ServerMessage {
        ServerMessage::pong()
    }

    /// `Subscribe`: replace the caller's per-connection event set.
    pub(super) fn handle_subscribe(
        subscriptions: &mut Option<
            std::collections::HashSet<crate::streaming::protocol::EventType>,
        >,
        events: Vec<crate::streaming::protocol::EventType>,
    ) {
        *subscriptions = Some(events.into_iter().collect());
    }

    /// `Mouse`: translate the event into a mouse-report escape sequence and
    /// send it through the shared input path.
    pub(super) fn handle_mouse(
        session: &Arc<StreamSessionState>,
        read_only: bool,
        input: MouseInput,
    ) {
        let MouseInput {
            col,
            row,
            button,
            shift,
            ctrl,
            alt,
            event_type,
        } = input;
        if read_only {
            return;
        }
        if Self::session_has_writer(session) {
            let bytes = {
                let mut terminal = session.terminal.write();
                // Build modifiers bitmask: shift=1, meta/alt=2, ctrl=4
                let mods = if shift { 1u8 } else { 0 }
                    | if alt { 2 } else { 0 }
                    | if ctrl { 4 } else { 0 };
                let pressed = event_type != crate::streaming::protocol::MouseEventType::Release;
                let mouse_event = crate::mouse::MouseEvent::new(
                    button,
                    col as usize,
                    row as usize,
                    pressed,
                    mods,
                );
                terminal.report_mouse(mouse_event)
            };
            if !bytes.is_empty() {
                session
                    .metrics
                    .input_bytes
                    .fetch_add(bytes.len(), Ordering::Relaxed);
                // QA-110: same serialized input path as keystrokes —
                // a mouse report may not overtake or trail the
                // keystrokes around it.
                Self::enqueue_input(session, bytes);
            }
        }
    }

    /// `FocusChange`: emit the focus in/out report when the terminal has
    /// focus tracking enabled, through the shared input path.
    pub(super) fn handle_focus_change(
        session: &Arc<StreamSessionState>,
        read_only: bool,
        focused: bool,
    ) {
        if read_only {
            return;
        }
        if Self::session_has_writer(session) {
            let bytes = {
                let terminal = session.terminal.write();
                if terminal.focus_tracking() {
                    if focused {
                        terminal.report_focus_in()
                    } else {
                        terminal.report_focus_out()
                    }
                } else {
                    Vec::new()
                }
            };
            if !bytes.is_empty() {
                session
                    .metrics
                    .input_bytes
                    .fetch_add(bytes.len(), Ordering::Relaxed);
                // QA-110: same serialized input path as keystrokes.
                Self::enqueue_input(session, bytes);
            }
        }
    }

    /// `Paste`: cap, rate-limit, then send the pasted text — wrapped in
    /// bracketed-paste markers when enabled — as one frame through the
    /// shared input path.
    pub(super) fn handle_paste(
        session: &Arc<StreamSessionState>,
        transport_label: &str,
        client_id: uuid::Uuid,
        read_only: bool,
        rate_limiter: &mut Option<InputRateLimiter>,
        content: String,
    ) {
        if read_only {
            return;
        }
        if content.len() > MAX_PASTE_PAYLOAD_BYTES {
            crate::debug_error!(
                "STREAMING",
                "Dropping oversize Paste from {} {} ({} bytes > {} cap)",
                transport_label,
                client_id,
                content.len(),
                MAX_PASTE_PAYLOAD_BYTES
            );
            session
                .metrics
                .dropped_messages
                .fetch_add(1, Ordering::Relaxed);
            return;
        }
        if let Some(ref mut limiter) = rate_limiter {
            if !limiter.try_consume(content.len()) {
                crate::debug_error!(
                    "STREAMING",
                    "Rate limit exceeded for {} {}",
                    transport_label,
                    client_id
                );
                session
                    .metrics
                    .dropped_messages
                    .fetch_add(1, Ordering::Relaxed);
                return;
            }
        }
        if Self::session_has_writer(session) {
            // Copy the bracketed-paste markers and payload out under
            // the terminal guard, then drop the guard before the
            // (blocking) PTY write (SEC-005).
            let (start, end, payload) = {
                let terminal = session.terminal.write();
                if terminal.bracketed_paste() {
                    (
                        terminal.bracketed_paste_start().to_vec(),
                        terminal.bracketed_paste_end().to_vec(),
                        content.into_bytes(),
                    )
                } else {
                    (Vec::new(), Vec::new(), content.into_bytes())
                }
            };
            // input_bytes counts the pasted payload only; the bracket
            // markers are transport framing, not user input.
            session
                .metrics
                .input_bytes
                .fetch_add(payload.len(), Ordering::Relaxed);
            // QA-110: one frame through the serialized input path, so
            // the bracket markers can never split from their payload
            // and the paste keeps its place among other input.
            let mut frame = Vec::with_capacity(start.len() + payload.len() + end.len());
            frame.extend_from_slice(&start);
            frame.extend_from_slice(&payload);
            frame.extend_from_slice(&end);
            Self::enqueue_input(session, frame);
        } else {
            // Same mid-dispatch detach race as `handle_input`'s else arm.
            crate::debug_error!(
                "STREAMING",
                "Paste from {} {} dropped: session {} lost its PTY writer mid-dispatch",
                transport_label,
                client_id,
                session.id
            );
            session
                .metrics
                .dropped_messages
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    /// `SelectionRequest`: apply the selection to the terminal and broadcast
    /// the resulting selection change to the session (no direct reply).
    /// `start` and `end` are `(col, row)` cell positions.
    pub(super) fn handle_selection_request(
        self: &Arc<Self>,
        session: &Arc<StreamSessionState>,
        ctx: ConnCtx<'_>,
        start: (u16, u16),
        end: (u16, u16),
        mode: String,
    ) {
        let ConnCtx {
            transport_label,
            client_id,
            read_only,
        } = ctx;
        let ((start_col, start_row), (end_col, end_row)) = (start, end);
        if read_only {
            return;
        }
        let (term_cols, term_rows) = {
            let terminal = session.terminal.read();
            terminal.size()
        };
        if usize::from(start_row) >= term_rows
            || usize::from(end_row) >= term_rows
            || usize::from(start_col) > term_cols
            || usize::from(end_col) > term_cols
        {
            crate::debug_error!(
                "STREAMING",
                "{} {} sent out-of-range selection {},{}-{},{}",
                transport_label,
                client_id,
                start_col,
                start_row,
                end_col,
                end_row
            );
            return;
        }
        let selection_msg = {
            let mut terminal = session.terminal.write();
            if mode == "clear" {
                terminal.clear_selection();
                Some(ServerMessage::selection_cleared())
            } else if mode == "word" {
                terminal.select_word_at(start_col as usize, start_row as usize);
                if let Some(sel) = terminal.get_selection() {
                    let text = terminal.get_selected_text();
                    Some(ServerMessage::selection_changed(
                        Some(sel.start.0 as u16),
                        Some(sel.start.1 as u16),
                        Some(sel.end.0 as u16),
                        Some(sel.end.1 as u16),
                        text,
                        "chars".to_string(),
                        false,
                    ))
                } else {
                    None
                }
            } else if mode == "line" {
                terminal.select_line(start_row as usize);
                if let Some(sel) = terminal.get_selection() {
                    let text = terminal.get_selected_text();
                    Some(ServerMessage::selection_changed(
                        Some(sel.start.0 as u16),
                        Some(sel.start.1 as u16),
                        Some(sel.end.0 as u16),
                        Some(sel.end.1 as u16),
                        text,
                        "line".to_string(),
                        false,
                    ))
                } else {
                    None
                }
            } else {
                let sel_mode = match mode.as_str() {
                    "block" => SelectionMode::Block,
                    "line" => SelectionMode::Line,
                    _ => SelectionMode::Character,
                };
                terminal.set_selection(
                    (start_col as usize, start_row as usize),
                    (end_col as usize, end_row as usize),
                    sel_mode,
                );
                let text = terminal.get_selected_text();
                Some(ServerMessage::selection_changed(
                    Some(start_col),
                    Some(start_row),
                    Some(end_col),
                    Some(end_row),
                    text,
                    mode,
                    false,
                ))
            }
        };
        if let Some(msg) = selection_msg {
            self.broadcast_to_session(&session.id, msg);
        }
    }

    /// `ClipboardRequest`: "set" stores the text and broadcasts the sync to
    /// the session; "get" replies with the clipboard contents when reads
    /// are allowed.
    pub(super) fn handle_clipboard_request(
        self: &Arc<Self>,
        session: &Arc<StreamSessionState>,
        ctx: ConnCtx<'_>,
        operation: String,
        content: Option<String>,
        target: Option<String>,
    ) -> Vec<ServerMessage> {
        let ConnCtx {
            transport_label,
            client_id,
            read_only,
        } = ctx;
        let mut replies = Vec::new();
        if read_only {
            return replies;
        }
        match operation.as_str() {
            "set" => {
                if let Some(ref text) = content {
                    let mut terminal = session.terminal.write();
                    terminal.set_clipboard(Some(text.clone()));
                    self.broadcast_to_session(
                        &session.id,
                        ServerMessage::clipboard_sync("set".to_string(), text.clone(), target),
                    );
                }
            }
            "get" => {
                let clipboard = {
                    let terminal = session.terminal.write();
                    if !terminal.allow_clipboard_read() {
                        crate::debug_error!(
                            "STREAMING",
                            "Clipboard read denied for {} {}: allow_clipboard_read is off",
                            transport_label,
                            client_id
                        );
                        None
                    } else {
                        Some(terminal.clipboard().unwrap_or_default().to_string())
                    }
                };
                let Some(clipboard) = clipboard else {
                    return replies;
                };
                replies.push(ServerMessage::clipboard_sync(
                    "get_response".to_string(),
                    clipboard,
                    target,
                ));
            }
            _ => {}
        }
        replies
    }
}
