//! Hook-only per-pane endpoints (ENH-039): naming, binding, remnant
//! sweeping, and serving one endpoint connection.

use super::*;

/// Max pane endpoints one daemon binds. Past the cap a new pane gets NO
/// endpoint: `PAR_MUX_SOCKET` stays unset for it rather than falling back
/// to the full control socket, which would silently restore the
/// least-privilege hole the mode exists to close. The count is the
/// endpoint socket files already beside the control socket at bind time
/// (the startup sweep has removed the stale ones). Documented in
/// docs/MUX.md "Agent Hook Reports".
pub(crate) const MAX_PANE_ENDPOINTS: usize = 256;

/// The file-name prefix of a pane endpoint beside `control_socket`:
/// `<control stem>.pane-<N>.sock`.
pub(super) fn pane_endpoint_prefix(control_socket: &Path) -> String {
    let name = control_socket
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = name.strip_suffix(".sock").unwrap_or(&name);
    format!("{stem}.pane-")
}

/// The socket path of `pane_id`'s endpoint: beside the control socket, in
/// the same guarded runtime directory, so the directory guard and the 0600
/// bind mode apply unchanged.
pub(super) fn pane_endpoint_path(control_socket: &Path, pane_id: PaneId) -> PathBuf {
    control_socket.with_file_name(format!(
        "{}{pane_id}.sock",
        pane_endpoint_prefix(control_socket)
    ))
}

/// Whether a directory entry name is one of `control_socket`'s pane
/// endpoints (the sweep's candidate test).
pub(super) fn is_pane_endpoint_name(control_socket: &Path, name: &str) -> bool {
    name.starts_with(&pane_endpoint_prefix(control_socket)) && name.ends_with(".sock")
}

/// One pane's hook-only endpoint (ENH-039): a socket beside the control
/// socket that accepts ONLY hook reports bound to its pane — a pane process
/// handed this path as `PAR_MUX_SOCKET` can report its agent state but
/// cannot drive other panes or the server. Bound by the pane factory when
/// the daemon runs with `--pane-endpoints`; dropped with the pane, which
/// unlinks the socket file (a crash or SIGKILL leaves the remnant for
/// [`sweep_pane_endpoint_remnants`]).
///
/// The accept thread holds no tree reference: accepted connections are
/// forwarded through the [`PaneEndpointTx`] channel and served by the
/// server's accept loop. A pane owning an endpoint therefore cannot keep
/// the tree — and with it the pane itself — alive; that reference cycle
/// would leak.
pub(crate) struct PaneEndpoint {
    pub(super) socket_path: PathBuf,
    accept_thread: Option<std::thread::JoinHandle<()>>,
    /// Set on drop: the accept thread polls it between nonblocking accepts
    /// and exits, so the join below is bounded by one poll interval.
    closed: Arc<AtomicBool>,
}

impl PaneEndpoint {
    /// Bind `pane_id`'s endpoint beside `control_socket`.
    ///
    /// Errors past [`MAX_PANE_ENDPOINTS`] live endpoints, when the path
    /// exceeds the platform socket-address limit, or on a bind failure —
    /// the factory answers any of these the same way: the pane's env
    /// contract exports NO socket, never a fallback to the full control
    /// socket.
    pub(crate) fn bind(
        control_socket: &Path,
        pane_id: PaneId,
        sink: PaneEndpointTx,
    ) -> std::io::Result<Self> {
        let socket_path = pane_endpoint_path(control_socket, pane_id);
        let live = control_socket
            .parent()
            .and_then(|dir| std::fs::read_dir(dir).ok())
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|entry| {
                        is_pane_endpoint_name(control_socket, &entry.file_name().to_string_lossy())
                    })
                    .count()
            })
            .unwrap_or(0);
        if live >= MAX_PANE_ENDPOINTS {
            return Err(std::io::Error::other(format!(
                "pane endpoint cap reached ({MAX_PANE_ENDPOINTS})"
            )));
        }
        prepare_socket_path(&socket_path)?;
        let listener = bind_local_listener(&socket_path)?;

        // Nonblocking accept on a poll, so Drop can end the thread through
        // `closed` instead of leaving it parked in accept() forever — one
        // thread leaks per spawned pane otherwise.
        use interprocess::local_socket::ListenerNonblockingMode;
        listener
            .set_nonblocking(ListenerNonblockingMode::Accept)
            .expect("the listener was just bound");
        let closed = Arc::new(AtomicBool::new(false));
        let thread_closed = Arc::clone(&closed);
        let thread_sink = sink;
        let accept_thread = std::thread::Builder::new()
            .name(format!("par-mux-pane-endpoint-{pane_id}"))
            .spawn(move || loop {
                match accept_connection(&listener) {
                    Ok((stream, abort)) => {
                        if thread_sink.inner.send((stream, abort, pane_id)).is_err() {
                            break;
                        }
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        if thread_closed.load(Ordering::Relaxed) {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                    // A signal may land mid-accept; that is not a fault.
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(err) => {
                        log::warn!("par-mux: pane endpoint accept failed: {err}");
                        break;
                    }
                }
            })?;
        Ok(Self {
            socket_path,
            accept_thread: Some(accept_thread),
            closed,
        })
    }

    /// The path to export as the pane's `PAR_MUX_SOCKET`.
    pub(crate) fn socket_path_string(&self) -> String {
        self.socket_path.to_string_lossy().into_owned()
    }
}

impl Drop for PaneEndpoint {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Relaxed);
        if let Some(thread) = self.accept_thread.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// Remove stale pane-endpoint socket remnants beside `control_socket` at
/// daemon startup (ENH-039). [`PaneEndpoint`]'s `Drop` unlinks a live
/// endpoint's socket file, but a crash or SIGKILL does not — without the
/// sweep the remnants accumulate in the runtime directory. Each candidate
/// goes through [`prepare_socket_path`], which reclaims exactly the stale
/// ones (a dead socket file or a stray regular file) and refuses a live
/// one; at startup nothing of this daemon's can be live yet, and no other
/// daemon can own these names (the control socket path is exclusive).
pub fn sweep_pane_endpoint_remnants(control_socket: &Path) {
    let Some(dir) = control_socket.parent() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_pane_endpoint_name(control_socket, &name) {
            let _ = prepare_socket_path(&entry.path());
        }
    }
}

/// Serve one pane-endpoint connection (ENH-039): hook reports for the
/// endpoint's pane only, answered in place; any control-command line is
/// refused with the hook-only error and the connection closed. Reuses the
/// control socket's bounded line reader (SEC-104). No registration, no
/// broadcast-set membership, no eviction — a hook connection is one line
/// in, one reply out.
pub(super) fn serve_pane_connection(
    stream: LocalStream,
    bound: PaneId,
    tree: &Arc<Mutex<MuxTree>>,
    clients: &Clients,
) {
    let mut writer = match stream.try_clone() {
        Ok(writer) => writer,
        Err(_) => return,
    };
    // A pane connection is never in the broadcast set, so nothing ever
    // evicts it; the reader's flag exists for the shared line reader only.
    let never_evicted = AtomicBool::new(false);
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = match read_control_line(&mut reader, &never_evicted) {
            ControlLine::Line(line) => line,
            ControlLine::Closed => break,
            // Over-budget or non-UTF-8 input is refused and the connection
            // closed — a pane endpoint carries one well-formed report per
            // exchange, never a stream of malformed frames.
            ControlLine::Oversize | ControlLine::Undecodable(_) => {
                let _ = write_line(
                    &mut writer,
                    "{\"error\":\"hook-only endpoint\"}",
                    &never_evicted,
                );
                break;
            }
        };
        while line.ends_with('\n') || line.ends_with('\r') {
            line.pop();
        }
        if line.trim().is_empty() {
            continue;
        }
        match parse_line(&line) {
            Ok(Line::Hook(report)) => {
                let (reply, broadcast) = crate::mux::hooks::handle_report_for(bound, &report, tree);
                if let Some(notification) = broadcast {
                    broadcast_notification(clients, &notification);
                }
                if write_line(&mut writer, &reply, &never_evicted).is_err() {
                    break;
                }
            }
            // Anything else — a control command or an unparseable line — is
            // what the endpoint exists to refuse. The error is one JSON
            // line, then the connection closes: a client that meant to talk
            // control-mode learns immediately, not after a timeout.
            _ => {
                let _ = write_line(
                    &mut writer,
                    "{\"error\":\"hook-only endpoint\"}",
                    &never_evicted,
                );
                break;
            }
        }
    }
}
