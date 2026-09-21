//! The control-mode socket server.
//!
//! One Unix socket, one accept loop, one thread-per-client writer. Pane output
//! is pushed to connected clients from the PTY reader callback as bytes arrive
//! — there is no polling anywhere in this path, which is the whole point of
//! the module (see `par-mux.md`).

use crate::mux::command::{parse_command, MuxCommand};
use crate::mux::emit::{emit, emit_block};
use crate::mux::ipc::{bind_local_listener, prepare_socket_path, LocalListener, LocalStream};
use crate::mux::pane::ShellPaneFactory;
use crate::mux::tree::MuxTree;
use crate::tmux_control::TmuxNotification;
use interprocess::local_socket::traits::Listener as _;
use interprocess::TryClone as _;
use parking_lot::Mutex;
use std::io::{BufRead, BufReader, Write};

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;

/// Default pane size for sessions created without an explicit size.
const DEFAULT_COLS: u16 = 80;
/// Default pane size for sessions created without an explicit size.
const DEFAULT_ROWS: u16 = 24;

/// Monotonic client ids, so a disconnecting client's broadcast sender can be
/// removed eagerly rather than waiting for the next broadcast to fail.
static CLIENT_SEQ: AtomicU64 = AtomicU64::new(0);

/// A control-mode multiplexer server listening on a Unix socket.
pub struct MuxServer {
    listener: LocalListener,
    path: PathBuf,
    tree: Arc<Mutex<MuxTree>>,
    clients: Arc<Mutex<Vec<(u64, Sender<String>)>>>,
}

impl MuxServer {
    /// Bind to `path`, refusing a path a live server already owns and
    /// reclaiming one only a stale remnant holds.
    ///
    /// Access control is the transport's: mode `0600` on Unix, an owner-only
    /// security descriptor on Windows — see [`crate::mux::ipc`].
    pub fn bind(path: &Path) -> std::io::Result<Self> {
        prepare_socket_path(path)?;
        let listener = bind_local_listener(path)?;

        Ok(Self {
            listener,
            path: path.to_path_buf(),
            tree: Arc::new(Mutex::new(MuxTree::new(Box::new(
                ShellPaneFactory::default(),
            )))),
            clients: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// The path this server is listening on.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Accept connections until the listener is closed.
    pub fn run(self) {
        loop {
            let Ok(stream) = self.listener.accept() else {
                break;
            };
            let tree = Arc::clone(&self.tree);
            let clients = Arc::clone(&self.clients);
            std::thread::spawn(move || handle_client(stream, tree, clients));
        }
    }
}

/// Serve one connected client: a writer thread draining a channel, and this
/// thread reading commands. On disconnect, only this client's broadcast
/// sender is removed — the accept loop and the tree are untouched.
fn handle_client(
    stream: LocalStream,
    tree: Arc<Mutex<MuxTree>>,
    clients: Arc<Mutex<Vec<(u64, Sender<String>)>>>,
) {
    let client_id = CLIENT_SEQ.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = channel::<String>();
    clients.lock().push((client_id, tx.clone()));

    let mut writer = match stream.try_clone() {
        Ok(stream) => stream,
        Err(_) => {
            clients.lock().retain(|(id, _)| *id != client_id);
            return;
        }
    };
    std::thread::spawn(move || {
        while let Ok(line) = rx.recv() {
            if writer.write_all(line.as_bytes()).is_err() || writer.flush().is_err() {
                break;
            }
        }
    });

    let reader = BufReader::new(stream);
    let mut command_number = 0u32;
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        command_number += 1;
        let reply = dispatch(&line, command_number, &tree, &clients);
        if tx.send(reply).is_err() {
            break;
        }
    }
    clients.lock().retain(|(id, _)| *id != client_id);
}

/// Execute one command and render its reply block.
fn dispatch(
    line: &str,
    command_number: u32,
    tree: &Arc<Mutex<MuxTree>>,
    clients: &Arc<Mutex<Vec<(u64, Sender<String>)>>>,
) -> String {
    let command = match parse_command(line) {
        Ok(command) => command,
        Err(err) => return emit_block(command_number, &err, false),
    };

    match command {
        MuxCommand::NewSession { name } => {
            let name = name.unwrap_or_else(|| "0".to_string());
            let mut guard = tree.lock();
            match guard.new_session(&name, DEFAULT_COLS, DEFAULT_ROWS) {
                Ok(session_id) => {
                    // Wire every pane in the new session to push its output.
                    let window_ids = guard
                        .session(session_id)
                        .map(|s| s.windows.clone())
                        .unwrap_or_default();
                    let pane_ids: Vec<_> = window_ids
                        .iter()
                        .filter_map(|w| guard.window(*w))
                        .flat_map(|w| w.panes.clone())
                        .collect();
                    for pane_id in pane_ids {
                        let sinks = Arc::clone(clients);
                        if let Some(pane) = guard.pane_mut(pane_id) {
                            pane.on_output(move |bytes: &[u8]| {
                                let line = emit(&TmuxNotification::Output {
                                    pane_id: pane_id.to_string(),
                                    data: bytes.to_vec(),
                                });
                                sinks.lock().retain(|(_, tx)| tx.send(line.clone()).is_ok());
                            });
                        }
                    }
                    emit_block(command_number, &session_id.to_string(), true)
                }
                Err(err) => emit_block(command_number, &err.to_string(), false),
            }
        }
        MuxCommand::ListPanes => {
            let guard = tree.lock();
            let body = guard
                .sessions()
                .iter()
                .filter_map(|s| guard.session(*s))
                .flat_map(|s| s.windows.clone())
                .filter_map(|w| guard.window(w))
                .flat_map(|w| w.panes.clone())
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            emit_block(command_number, &body, true)
        }
        MuxCommand::SendKeys { pane, keys } => {
            let mut guard = tree.lock();
            match guard.pane_mut(pane) {
                Some(target) => {
                    // The Phase 1 grammar is a whitespace split, so a literal
                    // newline cannot survive parse_command; append the one the
                    // shell needs to execute the payload. Expressing Enter as
                    // a distinct key is Phase 2 argument-grammar work.
                    let mut payload = keys.into_bytes();
                    payload.push(b'\n');
                    match target.write(&payload) {
                        Ok(()) => emit_block(command_number, "", true),
                        Err(err) => emit_block(command_number, &err.to_string(), false),
                    }
                }
                None => emit_block(command_number, &format!("no such pane: {pane}"), false),
            }
        }
        MuxCommand::KillPane { pane } => {
            let mut guard = tree.lock();
            match guard.kill_pane(pane) {
                Ok(()) => emit_block(command_number, "", true),
                Err(err) => emit_block(command_number, &err.to_string(), false),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_replaces_a_stale_socket_file_and_sets_mode_0600() {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "par-mux-stale-{}-{}.sock",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, b"stale junk").expect("write stale file");

        let server = MuxServer::bind(&path).expect("bind replaces a stale socket file");
        assert_eq!(server.path(), path.as_path());

        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path)
            .expect("socket file exists")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "socket must be owner-only");

        drop(server);
        let _ = std::fs::remove_file(&path);
    }
}
