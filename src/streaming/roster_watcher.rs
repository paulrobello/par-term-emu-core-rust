//! Long-lived watcher that mirrors the par-mux agent roster to streaming clients.
//!
//! One control connection to the daemon feeds a roster cache and a message
//! callback. Every (re)connect fetches a fresh `list-agents` snapshot plus one
//! bounded `list-panes -F '#{pane_id} #{pane_title}'` query (every pane's
//! effective title in one round trip, filling `AgentEntry.pane_label`), replaces
//! the cache and emits it as an [`ServerMessage::AgentRoster`]; agent
//! notifications after that become [`ServerMessage::AgentStateChanged`] deltas.
//! Deltas carry no title or project, so an upsert keeps the values the cache
//! holds for its pane. `%agent-telemetry-changed` is answered with a full
//! refetch — the push is identity-only by wire contract, and the fresh rows'
//! `host_telemetry` tokens carry the probe-attributed project; a
//! `%pane-title-changed` push patches its pane's label directly (the user
//! title; an empty one means cleared, so the effective title rides a bounded
//! per-pane query). Loss of the connection is answered with a backoff redial —
//! cumulative and capped, reset only after a connection has held long enough to be
//! stable — and a fresh snapshot, so no agent from before a daemon restart
//! survives as a ghost. The fetches are timeout-bounded (bounded connect plus
//! the client's reply timeout), so a hung daemon delays the watcher but cannot
//! wedge its thread. There is no polling timer: the only waits are the fetch
//! bounds and the reconnect backoff.

use super::protocol::{AgentEntry, ServerMessage};
use super::roster::{
    parse_agents_output, parse_titles_output, translate_notification, RosterDelta,
};
use crate::mux::{emit, MuxClient};
use crate::tmux_control::TmuxNotification;
use parking_lot::Mutex;
use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const BACKOFF_INITIAL: Duration = Duration::from_millis(100);
const BACKOFF_MAX: Duration = Duration::from_secs(5);
/// How long one connect attempt may block before the fetch cycle gives up
/// and redials. A daemon hung with a full accept backlog would otherwise
/// block the watcher's connect forever — the client's reply timeout does
/// not cover the connect itself.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// A connection must hold at least this long for its end to reset the redial
/// backoff to [`BACKOFF_INITIAL`]. Anything shorter is a flap: the flap keeps
/// the cumulative (capped) delay instead of redialing at 100ms forever.
const BACKOFF_STABLE: Duration = Duration::from_secs(30);

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
    let mut entries = parse_agents_output(&reply.body.join("\n"));
    // One bounded query fills pane_label for every entry at once — the
    // daemon sanitizes each title (CR/LF → space) for the line-framed
    // reply, so rows parse cleanly. A rejected query fails the fetch:
    // labels stay honest (absent) rather than partially filled, and the
    // watcher redials through its backoff.
    let titles = client.send_checked("list-panes -F \"#{pane_id} #{pane_title}\"")?;
    if !titles.ok {
        return Err(io::Error::other(format!(
            "list-panes rejected: {}",
            titles.body.join(" | ")
        )));
    }
    let titles = parse_titles_output(&titles.body.join("\n"));
    for entry in &mut entries {
        entry.pane_label = titles.get(&entry.pane_id).cloned().unwrap_or_default();
    }
    Ok(entries)
}

/// The bounded per-pane title query, used when a rename push clears a
/// user title: the push itself never carries the display label that then
/// falls back to the OSC title.
fn pane_label(client: &mut MuxClient, pane_id: u32) -> io::Result<String> {
    let reply = client.send_checked(&format!("pane-title -t %{pane_id}"))?;
    if !reply.ok {
        return Err(io::Error::other(format!(
            "pane-title rejected: {}",
            reply.body.join(" | ")
        )));
    }
    Ok(reply.body.join("\n"))
}

/// The redial delay after one connect/fetch/serve cycle ends: a failed or
/// short-lived cycle grows the cumulative backoff toward [`BACKOFF_MAX`],
/// and only a connection that held [`BACKOFF_STABLE`] or longer resets it
/// to [`BACKOFF_INITIAL`]. Passing the grown value forward is what keeps a
/// connect-then-drop flap from redialing at the initial 100ms forever.
fn next_backoff(current: Duration, held: Option<Duration>) -> Duration {
    match held {
        Some(held) if held >= BACKOFF_STABLE => BACKOFF_INITIAL,
        _ => (current * 2).min(BACKOFF_MAX),
    }
}

fn run(
    socket: &std::path::Path,
    roster: &Mutex<BTreeMap<u32, AgentEntry>>,
    stop: &AtomicBool,
    on_message: &OnMessage,
) {
    let mut backoff = BACKOFF_INITIAL;
    while !stop.load(Ordering::Relaxed) {
        let mut client = match MuxClient::connect_bounded(socket, Instant::now() + CONNECT_TIMEOUT)
        {
            Ok(client) => client,
            Err(_) => {
                std::thread::sleep(backoff);
                backoff = next_backoff(backoff, None);
                continue;
            }
        };
        // Notifications queue on the client from connect on, so deltas that
        // land while the snapshot is in flight are applied after it.
        let entries = match fetch_roster(&mut client) {
            Ok(entries) => entries,
            Err(_) => {
                std::thread::sleep(backoff);
                backoff = next_backoff(backoff, None);
                continue;
            }
        };
        let snapshot = {
            let mut map = roster.lock();
            *map = entries.into_iter().map(|e| (e.pane_id, e)).collect();
            roster_message(&map)
        };
        on_message(snapshot);

        let connected_at = Instant::now();
        while let Ok(notification) = client.notifications().recv() {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            match notification {
                // Telemetry pushes are identity-only by wire contract, so
                // freshness comes from re-running the roster fetch: the
                // host-attributed project the push itself never carries
                // rides the fresh rows' host_telemetry tokens.
                TmuxNotification::AgentTelemetryChanged { .. } => {
                    match fetch_roster(&mut client) {
                        Ok(entries) => {
                            let snapshot = {
                                let mut map = roster.lock();
                                *map = entries.into_iter().map(|e| (e.pane_id, e)).collect();
                                roster_message(&map)
                            };
                            on_message(snapshot);
                        }
                        // Connection degraded: fall to the redial path.
                        Err(_) => break,
                    }
                }
                // The rename push carries the USER title (empty =
                // cleared); the displayed label is the effective title,
                // which falls back to the OSC title on a clear — that
                // value rides a bounded per-pane query.
                TmuxNotification::PaneTitleChanged { pane_id, title } => {
                    let Ok(pane_id) = pane_id.trim_start_matches('%').parse::<u32>() else {
                        continue;
                    };
                    let label = if !title.is_empty() {
                        Ok(title)
                    } else {
                        pane_label(&mut client, pane_id)
                    };
                    let Ok(label) = label else { break };
                    let patched = {
                        let mut map = roster.lock();
                        match map.get_mut(&pane_id) {
                            Some(entry) => {
                                entry.pane_label = label;
                                Some(delta_message(RosterDelta::Upsert(entry.clone())))
                            }
                            None => None,
                        }
                    };
                    if let Some(message) = patched {
                        on_message(message);
                    }
                }
                _ => {
                    let Some(delta) = translate_notification(&emit(&notification)) else {
                        continue;
                    };
                    let delta = {
                        let mut map = roster.lock();
                        match delta {
                            RosterDelta::Upsert(mut entry) => {
                                // State pushes carry no title/project: a
                                // pane the watcher knows keeps its fetched
                                // values (the attach roster patch keeps the
                                // known one over a bare push).
                                if let Some(prev) = map.get(&entry.pane_id) {
                                    entry.pane_label = prev.pane_label.clone();
                                    entry.project = prev.project.clone();
                                }
                                map.insert(entry.pane_id, entry.clone());
                                RosterDelta::Upsert(entry)
                            }
                            RosterDelta::Release { pane_id } => {
                                map.remove(&pane_id);
                                RosterDelta::Release { pane_id }
                            }
                        }
                    };
                    on_message(delta_message(delta));
                }
            }
        }
        // Connection lost. A connection that held past BACKOFF_STABLE proves
        // the daemon healthy: redial immediately at the initial delay. A
        // short-lived one is a flap — wait out the cumulative backoff and
        // grow it, so a connect-then-drop storm cannot spin at 100ms.
        let held = connected_at.elapsed();
        if held < BACKOFF_STABLE {
            std::thread::sleep(backoff);
        }
        backoff = next_backoff(backoff, Some(held));
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
        /// The title query's reply body ("" serves no titles).
        list_panes_body: &'static str,
        /// Exact-command replies for bounded per-pane queries after the
        /// pushes (the cleared-rename `pane-title` query).
        pane_titles: &'static [(&'static str, &'static str)],
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
                // The roster fetch is always two commands now: the agent
                // list, then the bounded title query.
                let mut line = String::new();
                reader.read_line(&mut line).expect("read command");
                assert_eq!(line.trim(), "list-agents");
                write!(
                    stream,
                    "%begin 1 0 1\n{}%end 1 0 1\n",
                    conn.list_agents_body
                )
                .expect("reply");
                let mut line = String::new();
                reader.read_line(&mut line).expect("read title command");
                assert!(line.trim().starts_with("list-panes"), "got {line:?}");
                write!(stream, "%begin 1 0 1\n{}%end 1 0 1\n", conn.list_panes_body)
                    .expect("reply");
                for extra in conn.then_send {
                    writeln!(stream, "{extra}").expect("push");
                }
                if conn.hold {
                    loop {
                        let mut line = String::new();
                        match reader.read_line(&mut line) {
                            Ok(0) | Err(_) => break,
                            Ok(_) => {}
                        }
                        let command = line.trim();
                        let body = if command == "list-agents" {
                            conn.list_agents_body
                        } else if command.starts_with("list-panes") {
                            conn.list_panes_body
                        } else {
                            conn.pane_titles
                                .iter()
                                .find(|(query, _)| *query == command)
                                .map(|(_, body)| *body)
                                .unwrap_or("")
                        };
                        write!(
                            stream,
                            "%begin 1 0 1\n{}{}%end 1 0 1\n",
                            body,
                            // A non-empty body needs its line terminator for
                            // the body rows to frame as lines.
                            if body.is_empty() { "" } else { "\n" }
                        )
                        .expect("reply");
                    }
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
                list_agents_body: "15 claude-code blocked scrape reason=SGVsbG8= ws=+4 host_telemetry=eyJwcm9qZWN0Ijp7InNhbXBsZWRfYXRfdW5peF9tcyI6MSwidmFsdWUiOiJwYXJkZWNrIn19\n3 zed idle hook\n",
                list_panes_body: "%15 build: par-term\n%3 zed — the idle pane\n",
                pane_titles: &[],
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
        assert_eq!(agents[0].pane_label, "zed — the idle pane");
        assert_eq!(agents[1].reason, "Hello");
        assert_eq!(
            agents[1].workspace, "+4",
            "the roster row's ws= token rides to the wire snapshot"
        );
        assert_eq!(
            agents[1].project, "pardeck",
            "the row's host_telemetry token attributes the project"
        );
        assert_eq!(
            agents[0].workspace, "",
            "a row without ws= stays absent, never guessed"
        );
        assert_eq!(
            agents[0].project, "",
            "a row without host_telemetry stays absent, never guessed"
        );
        let ServerMessage::AgentRoster { agents: cached } = watcher.snapshot() else {
            panic!("snapshot is a roster");
        };
        assert_eq!(&cached, agents);
    }

    #[test]
    fn telemetry_changed_pushes_trigger_a_roster_refetch() {
        let dir = tempfile::tempdir().unwrap();
        let path = serve(
            dir.path(),
            vec![Conn {
                list_agents_body: "3 zed idle hook host_telemetry=eyJwcm9qZWN0Ijp7InNhbXBsZWRfYXRfdW5peF9tcyI6MSwidmFsdWUiOiJwYXJkZWNrIn19\n",
                list_panes_body: "%3 zed\n",
                pane_titles: &[],
                then_send: &["%agent-telemetry-changed %3 zed"],
                hold: true,
            }],
        );
        let (on_message, rx) = collect();
        // Alive until scope end, so pushes keep flowing during the asserts.
        let _watcher = RosterWatcher::spawn(path, on_message);
        assert_eq!(entries(next(&rx)), vec![(3, "idle".to_string())]);
        // The push is identity-only: the watcher answers it with a fresh
        // full roster carrying the newly attributed project and label.
        match next(&rx) {
            ServerMessage::AgentRoster { agents } => {
                assert_eq!(agents.len(), 1);
                assert_eq!(agents[0].pane_id, 3);
                assert_eq!(agents[0].project, "pardeck");
                assert_eq!(agents[0].pane_label, "zed");
            }
            other => panic!("expected a refetched roster, got {other:?}"),
        }
    }

    #[test]
    fn pane_title_changed_pushes_refresh_the_label() {
        let dir = tempfile::tempdir().unwrap();
        let path = serve(
            dir.path(),
            vec![Conn {
                list_agents_body: "3 zed idle hook\n",
                list_panes_body: "%3 old title\n",
                pane_titles: &[("pane-title -t %3", "osctitle")],
                then_send: &["%pane-title-changed %3 renamed", "%pane-title-changed %3"],
                hold: true,
            }],
        );
        let (on_message, rx) = collect();
        // Alive until scope end, so pushes keep flowing during the asserts.
        let _watcher = RosterWatcher::spawn(path, on_message);
        assert_eq!(entries(next(&rx)), vec![(3, "idle".to_string())]);
        match next(&rx) {
            ServerMessage::AgentStateChanged { agent, released } => {
                assert!(!released);
                assert_eq!(agent.pane_id, 3);
                assert_eq!(agent.pane_label, "renamed");
            }
            other => panic!("expected a rename delta, got {other:?}"),
        }
        // The cleared push carries no user title: the watcher serves the
        // effective label (the OSC title) from the bounded query.
        match next(&rx) {
            ServerMessage::AgentStateChanged { agent, released } => {
                assert!(!released);
                assert_eq!(agent.pane_label, "osctitle");
            }
            other => panic!("expected a clear delta, got {other:?}"),
        }
    }

    #[test]
    fn deltas_update_the_snapshot_and_broadcast() {
        let dir = tempfile::tempdir().unwrap();
        let path = serve(
            dir.path(),
            vec![Conn {
                list_agents_body: "3 zed idle hook\n",
                list_panes_body: "%3 zed\n",
                pane_titles: &[],
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
    fn flap_grows_the_backoff_and_stability_resets_it() {
        // A short-lived connection is a flap: the delay grows past the
        // initial 100ms instead of resetting to it, capping at BACKOFF_MAX.
        assert_eq!(
            next_backoff(BACKOFF_INITIAL, None),
            Duration::from_millis(200)
        );
        assert_eq!(
            next_backoff(Duration::from_secs(4), None),
            BACKOFF_MAX,
            "growth caps at BACKOFF_MAX"
        );
        assert_eq!(next_backoff(BACKOFF_MAX, None), BACKOFF_MAX);
        // A connection that held past BACKOFF_STABLE proves the daemon
        // healthy, and the next redial starts over at the initial delay.
        assert_eq!(
            next_backoff(BACKOFF_MAX, Some(BACKOFF_STABLE)),
            BACKOFF_INITIAL
        );
        // One millisecond under the stability mark still counts as a flap.
        assert_eq!(
            next_backoff(
                BACKOFF_INITIAL,
                Some(BACKOFF_STABLE - Duration::from_millis(1))
            ),
            Duration::from_millis(200)
        );
    }

    #[test]
    fn watcher_reconnect_resnapshots() {
        let dir = tempfile::tempdir().unwrap();
        let path = serve(
            dir.path(),
            vec![
                Conn {
                    list_agents_body: "1 claude idle hook\n9 ghost working scrape\n",
                    list_panes_body: "%1 main\n",
                    pane_titles: &[],
                    then_send: &["%agent-state-changed %1 claude working source=hook"],
                    hold: false,
                },
                Conn {
                    list_agents_body: "1 claude blocked hook\n2 zed idle scrape\n",
                    list_panes_body: "",
                    pane_titles: &[],
                    then_send: &[],
                    hold: true,
                },
            ],
        );
        let (on_message, rx) = collect();
        let watcher = RosterWatcher::spawn(path, on_message);
        assert_eq!(
            entries(next(&rx)),
            vec![(1, "idle".to_string()), (9, "working".to_string())]
        );
        assert!(matches!(
            next(&rx),
            ServerMessage::AgentStateChanged {
                released: false,
                ..
            }
        ));
        // The double dropped the connection: a fresh roster follows, carrying
        // the state the daemon now reports (including a pane the watcher
        // never saw a delta for) and dropping pane 9, which vanished while
        // the watcher was disconnected.
        assert_eq!(
            entries(next(&rx)),
            vec![(1, "blocked".to_string()), (2, "idle".to_string())]
        );
        assert_eq!(
            entries(watcher.snapshot()),
            vec![(1, "blocked".to_string()), (2, "idle".to_string())]
        );
    }
}
