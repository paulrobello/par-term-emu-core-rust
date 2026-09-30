//! Long-lived watcher that mirrors the par-mux agent roster to streaming clients.
//!
//! One control connection to the daemon feeds a roster cache and a message
//! callback. Every (re)connect fetches a fresh `list-agents` snapshot, replaces
//! the cache and emits it as an [`ServerMessage::AgentRoster`]; agent
//! notifications after that become [`ServerMessage::AgentStateChanged`] deltas.
//! Loss of the connection is answered with a backoff redial and a fresh
//! snapshot, so no agent from before a daemon restart survives as a ghost.
//! There is no polling timer: the only wait is the reconnect backoff.

use super::protocol::{AgentEntry, ServerMessage};
use super::roster::{parse_agents_output, translate_notification, RosterDelta};
use crate::mux::client::MuxClient;
use crate::mux::emit::emit;
use crate::tmux_control::TmuxNotification;
use parking_lot::Mutex;
use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const BACKOFF_INITIAL: Duration = Duration::from_millis(100);
const BACKOFF_MAX: Duration = Duration::from_secs(5);

/// Callback receiving every roster message the watcher produces.
pub type OnMessage = Arc<dyn Fn(ServerMessage) + Send + Sync>;

/// Handle to the running roster watcher thread and its roster cache.
///
/// Dropping the handle stops the thread the next time it wakes (on a
/// notification, a disconnect, or the end of a backoff wait).
pub struct RosterWatcher {
    roster: Arc<Mutex<BTreeMap<u32, AgentEntry>>>,
    stop: Arc<AtomicBool>,
}

impl RosterWatcher {
    /// Start the watcher against the daemon control socket at `socket`.
    pub fn spawn(socket: PathBuf, on_message: OnMessage) -> Self {
        let roster = Arc::new(Mutex::new(BTreeMap::new()));
        let stop = Arc::new(AtomicBool::new(false));
        {
            let roster = Arc::clone(&roster);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || run(&socket, &roster, &stop, &on_message));
        }
        Self { roster, stop }
    }

    /// The current roster as an `AgentRoster` message (pane-id ascending).
    ///
    /// Served from the cache the watcher keeps current, so a client connect
    /// costs no daemon round trip. The cache is updated before the matching
    /// delta is broadcast, so a client that subscribes to the broadcast first
    /// and calls this second either sees the delta in the snapshot, in the
    /// queue, or both. Deltas carry a full entry per pane, so replaying a
    /// delta the snapshot already reflects converges on the same state.
    pub fn snapshot(&self) -> ServerMessage {
        roster_message(&self.roster.lock())
    }
}

impl Drop for RosterWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn roster_message(map: &BTreeMap<u32, AgentEntry>) -> ServerMessage {
    ServerMessage::AgentRoster {
        agents: map.values().cloned().collect(),
    }
}

fn delta_message(delta: RosterDelta) -> ServerMessage {
    match delta {
        RosterDelta::Upsert(agent) => ServerMessage::AgentStateChanged {
            agent,
            released: false,
        },
        RosterDelta::Release { pane_id } => ServerMessage::AgentStateChanged {
            agent: AgentEntry {
                pane_id,
                ..AgentEntry::default()
            },
            released: true,
        },
    }
}

fn fetch_roster(client: &mut MuxClient) -> io::Result<Vec<AgentEntry>> {
    let reply = client.send_checked("list-agents")?;
    if !reply.ok {
        return Err(io::Error::other(format!(
            "list-agents rejected: {}",
            reply.body.join(" | ")
        )));
    }
    Ok(parse_agents_output(&reply.body.join("\n")))
}

fn run(
    socket: &std::path::Path,
    roster: &Mutex<BTreeMap<u32, AgentEntry>>,
    stop: &AtomicBool,
    on_message: &OnMessage,
) {
    let mut backoff = BACKOFF_INITIAL;
    while !stop.load(Ordering::Relaxed) {
        let mut client = match MuxClient::connect(socket) {
            Ok(client) => client,
            Err(_) => {
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(BACKOFF_MAX);
                continue;
            }
        };
        // Notifications queue on the client from connect on, so deltas that
        // land while the snapshot is in flight are applied after it.
        let entries = match fetch_roster(&mut client) {
            Ok(entries) => entries,
            Err(_) => {
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(BACKOFF_MAX);
                continue;
            }
        };
        backoff = BACKOFF_INITIAL;
        let snapshot = {
            let mut map = roster.lock();
            *map = entries.into_iter().map(|e| (e.pane_id, e)).collect();
            roster_message(&map)
        };
        on_message(snapshot);

        while let Ok(notification) = client.notifications().recv() {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            if !matches!(
                notification,
                TmuxNotification::AgentStateChanged { .. }
                    | TmuxNotification::AgentReleased { .. }
                    | TmuxNotification::PaneExited { .. }
            ) {
                continue;
            }
            let Some(delta) = translate_notification(&emit(&notification)) else {
                continue;
            };
            {
                let mut map = roster.lock();
                match &delta {
                    RosterDelta::Upsert(entry) => {
                        map.insert(entry.pane_id, entry.clone());
                    }
                    RosterDelta::Release { pane_id } => {
                        map.remove(pane_id);
                    }
                }
            }
            on_message(delta_message(delta));
        }
        // Connection lost: fall through to the redial, which re-snapshots.
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::sync::mpsc::{channel, Receiver};

    /// What the double does on one accepted connection.
    struct Conn {
        list_agents_body: &'static str,
        then_send: &'static [&'static str],
        /// Keep the connection open until the test ends instead of dropping it.
        hold: bool,
    }

    fn serve(dir: &std::path::Path, conns: Vec<Conn>) -> PathBuf {
        let path = dir.join("double.sock");
        let listener = UnixListener::bind(&path).expect("bind double");
        std::thread::spawn(move || {
            for conn in conns {
                let (mut stream, _) = listener.accept().expect("accept");
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut line = String::new();
                reader.read_line(&mut line).expect("read command");
                assert_eq!(line.trim(), "list-agents");
                write!(
                    stream,
                    "%begin 1 0 1\n{}%end 1 0 1\n",
                    conn.list_agents_body
                )
                .expect("reply");
                for extra in conn.then_send {
                    writeln!(stream, "{extra}").expect("push");
                }
                if conn.hold {
                    std::thread::sleep(Duration::from_secs(30));
                }
            }
        });
        path
    }

    fn collect() -> (OnMessage, Receiver<ServerMessage>) {
        let (tx, rx) = channel();
        let tx = Mutex::new(tx);
        (
            Arc::new(move |m| {
                let _ = tx.lock().send(m);
            }),
            rx,
        )
    }

    fn next(rx: &Receiver<ServerMessage>) -> ServerMessage {
        rx.recv_timeout(Duration::from_secs(10)).expect("message")
    }

    fn entries(msg: ServerMessage) -> Vec<(u32, String)> {
        match msg {
            ServerMessage::AgentRoster { agents } => {
                agents.into_iter().map(|a| (a.pane_id, a.state)).collect()
            }
            other => panic!("expected roster, got {other:?}"),
        }
    }

    #[test]
    fn snapshot_builds_roster_from_list_agents_output() {
        let dir = tempfile::tempdir().unwrap();
        let path = serve(
            dir.path(),
            vec![Conn {
                list_agents_body:
                    "15 claude-code blocked scrape reason=SGVsbG8=\n3 zed idle hook\n",
                then_send: &[],
                hold: true,
            }],
        );
        let (on_message, rx) = collect();
        let watcher = RosterWatcher::spawn(path, on_message);
        let first = next(&rx);
        let ServerMessage::AgentRoster { agents } = &first else {
            panic!("expected roster, got {first:?}");
        };
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].pane_id, 3);
        assert_eq!(agents[1].reason, "Hello");
        let ServerMessage::AgentRoster { agents: cached } = watcher.snapshot() else {
            panic!("snapshot is a roster");
        };
        assert_eq!(&cached, agents);
    }

    #[test]
    fn deltas_update_the_snapshot_and_broadcast() {
        let dir = tempfile::tempdir().unwrap();
        let path = serve(
            dir.path(),
            vec![Conn {
                list_agents_body: "3 zed idle hook\n",
                then_send: &[
                    "%agent-state-changed %3 zed working source=hook",
                    "%agent-released %3 zed",
                ],
                hold: true,
            }],
        );
        let (on_message, rx) = collect();
        let watcher = RosterWatcher::spawn(path, on_message);
        assert_eq!(entries(next(&rx)), vec![(3, "idle".to_string())]);
        match next(&rx) {
            ServerMessage::AgentStateChanged { agent, released } => {
                assert!(!released);
                assert_eq!((agent.pane_id, agent.state.as_str()), (3, "working"));
            }
            other => panic!("expected delta, got {other:?}"),
        }
        match next(&rx) {
            ServerMessage::AgentStateChanged { agent, released } => {
                assert!(released);
                assert_eq!(agent.pane_id, 3);
            }
            other => panic!("expected release, got {other:?}"),
        }
        assert!(entries(watcher.snapshot()).is_empty());
    }

    #[test]
    fn watcher_reconnect_resnapshots() {
        let dir = tempfile::tempdir().unwrap();
        let path = serve(
            dir.path(),
            vec![
                Conn {
                    list_agents_body: "1 claude idle hook\n",
                    then_send: &["%agent-state-changed %1 claude working source=hook"],
                    hold: false,
                },
                Conn {
                    list_agents_body: "1 claude blocked hook\n2 zed idle scrape\n",
                    then_send: &[],
                    hold: true,
                },
            ],
        );
        let (on_message, rx) = collect();
        let watcher = RosterWatcher::spawn(path, on_message);
        assert_eq!(entries(next(&rx)), vec![(1, "idle".to_string())]);
        assert!(matches!(
            next(&rx),
            ServerMessage::AgentStateChanged {
                released: false,
                ..
            }
        ));
        // The double dropped the connection: a fresh roster follows, carrying
        // the state the daemon now reports (including a pane the watcher
        // never saw a delta for).
        assert_eq!(
            entries(next(&rx)),
            vec![(1, "blocked".to_string()), (2, "idle".to_string())]
        );
        assert_eq!(entries(watcher.snapshot()).len(), 2);
    }
}
