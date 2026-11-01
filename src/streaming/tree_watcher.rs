//! Long-lived watcher that mirrors the par-mux tree — every workspace,
//! session, window and pane — to streaming clients as full-replacement
//! snapshots.
//!
//! One control connection to the daemon feeds a tree cache and a message
//! callback, exactly the [`super::RosterWatcher`] shape: every (re)connect
//! fetches the whole tree with four existing read-only queries
//! (`list-workspaces`, `list-sessions -F`, `list-panes -F`, and
//! `list-windows -a -F` — the daemon renders `#{session_active}`,
//! `#{window_layout}`, `#{pane_dead}` and `#{pane_exit_code}` for
//! those), replaces the cache and broadcasts the fresh
//! [`ServerMessage::MuxTreeSnapshot`]. The daemon's structural
//! notifications (`%workspaces-changed`, `%sessions-changed`,
//! `%window-add`/`%window-close`/`%window-renamed` and their unlinked
//! twins, `%session-renamed`, `%session-window-changed`,
//! `%window-pane-changed`, `%layout-change`, `%pane-title-changed`,
//! `%pane-exited`, `%pane-respawned`) each
//! trigger one refetch: the updates ARE fresh full snapshots, never
//! deltas — a burst folds into one refetch through a channel drain, so a
//! resize storm costs one round of four small queries. Loss of the
//! connection is answered with the roster watcher's backoff redial and a
//! fresh broadcast, so no window or pane from before a daemon restart
//! survives as a ghost. A client that wants the tree on demand sends
//! `ClientMessage::MuxTreeRequest` and is served from the same cache —
//! no daemon round trip, and no daemon selection or sizing touched.
//!
//! The fetches are timeout-bounded (bounded connect plus the client's
//! reply timeout), so a hung daemon delays the watcher but cannot wedge
//! its thread. There is no polling timer: the only waits are the fetch
//! bounds and the reconnect backoff.

use super::protocol::{
    MuxTreePane, MuxTreeSession, MuxTreeWindow, MuxTreeWorkspace, ServerMessage,
};
use crate::mux::MuxClient;
use crate::tmux_control::TmuxNotification;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long one connect attempt may block before the fetch cycle gives up
/// and redials (the roster watcher's value).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const BACKOFF_INITIAL: Duration = Duration::from_millis(100);
const BACKOFF_MAX: Duration = Duration::from_secs(5);
/// A connection must hold at least this long for its end to reset the
/// redial backoff to [`BACKOFF_INITIAL`]; anything shorter is a flap that
/// keeps the cumulative (capped) delay.
const BACKOFF_STABLE: Duration = Duration::from_secs(30);

/// The read-only queries the tree fetch runs, in fetch order. No
/// `select-*`, no resize, no pane I/O: metadata browsing must not move
/// the daemon's selection or sizing.
///
/// The `-F` shapes put ids and flags in fixed leading fields so the free
/// text (names, titles) survives as the line remainder, the same rule the
/// roster's `#{pane_id} #{pane_title}` query uses.
const QUERY_WORKSPACES: &str = "list-workspaces";
const QUERY_SESSIONS: &str =
    "list-sessions -F '#{workspace_id} #{session_active} #{session_id} #{session_name}'";
const QUERY_WINDOWS: &str =
    "list-windows -a -F '#{session_id} #{window_active} #{window_id} #{window_layout} #{window_name}'";
const QUERY_PANES: &str =
    "list-panes -F '#{window_id} #{pane_active} #{pane_id} #{pane_dead} #{pane_exit_code} #{pane_title}'";

/// Callback receiving every tree message the watcher produces.
pub type OnTreeMessage = Arc<dyn Fn(ServerMessage) + Send + Sync>;

/// Handle to the running tree watcher thread and its tree cache. Dropping
/// the handle stops the thread the next time it wakes (on a notification,
/// a disconnect, or the end of a backoff wait).
pub struct TreeWatcher {
    tree: Arc<Mutex<Vec<MuxTreeWorkspace>>>,
    stop: Arc<AtomicBool>,
}

impl TreeWatcher {
    /// Start the watcher against the daemon control socket at `socket`.
    pub fn spawn(socket: PathBuf, on_message: OnTreeMessage) -> Self {
        let tree = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        {
            let tree = Arc::clone(&tree);
            let stop = Arc::clone(&stop);
            std::thread::Builder::new()
                .name("streamer-tree-watcher".to_string())
                .spawn(move || run(&socket, &tree, &stop, &on_message))
                .expect("spawn tree watcher thread");
        }
        Self { tree, stop }
    }

    /// The current tree as a `MuxTreeSnapshot` message, from the cache the
    /// watcher keeps current. Served without a daemon round trip: the
    /// cache is updated before the matching broadcast, so a request that
    /// races a structural change reads either the pre-change or the
    /// post-change state, and the change's own push corrects a stale read.
    pub fn snapshot(&self) -> ServerMessage {
        tree_message(&self.tree.lock())
    }
}

impl Drop for TreeWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn tree_message(workspaces: &[MuxTreeWorkspace]) -> ServerMessage {
    ServerMessage::MuxTreeSnapshot {
        workspaces: workspaces.to_vec(),
    }
}

/// The `next_backoff` policy shared with the roster watcher: a connection
/// that held [`BACKOFF_STABLE`] resets to the initial delay, anything
/// shorter doubles the cumulative delay, capped at [`BACKOFF_MAX`].
fn next_backoff(current: Duration, held: Option<Duration>) -> Duration {
    match held {
        Some(held) if held >= BACKOFF_STABLE => BACKOFF_INITIAL,
        _ => (current * 2).min(BACKOFF_MAX),
    }
}

/// One bounded `send_checked`, with a `%error` reply converted to an
/// error so a rejected query fails the whole fetch and the watcher
/// redials instead of assembling a half tree.
fn query(client: &mut MuxClient, command: &str) -> io::Result<Vec<String>> {
    let reply = client.send_checked(command)?;
    if !reply.ok {
        return Err(io::Error::other(format!(
            "{command:?} rejected: {}",
            reply.body.join(" | ")
        )));
    }
    Ok(reply.body)
}

/// The whole tree in one fetch: four read-only queries, assembled by the
/// ids the rows carry. Each level keeps its query's row order, which is
/// the daemon's tree order (workspace-id order, then session, window, and
/// layout-leaf order).
fn fetch_tree(client: &mut MuxClient) -> io::Result<Vec<MuxTreeWorkspace>> {
    let mut workspaces: Vec<MuxTreeWorkspace> = query(client, QUERY_WORKSPACES)?
        .iter()
        .filter_map(|line| {
            let (id, rest) = line.split_once(": ")?;
            if !id.starts_with('+') {
                return None;
            }
            let (name, active) = match rest.strip_suffix(" active") {
                Some(name) => (name.to_string(), true),
                None => (rest.to_string(), false),
            };
            Some(MuxTreeWorkspace {
                id: id.to_string(),
                name,
                active,
                sessions: Vec::new(),
            })
        })
        .collect();
    let mut sessions: HashMap<String, MuxTreeSession> = HashMap::new();
    let mut session_order: Vec<(String, String)> = Vec::new(); // (workspace id, session id)
    for line in query(client, QUERY_SESSIONS)? {
        let mut fields = line.splitn(4, ' ');
        let (Some(workspace_id), Some(active), Some(id), Some(name)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        session_order.push((workspace_id.to_string(), id.to_string()));
        sessions.insert(
            id.to_string(),
            MuxTreeSession {
                id: id.to_string(),
                name: name.to_string(),
                active: active == "1",
                windows: Vec::new(),
            },
        );
    }
    let mut windows: HashMap<String, MuxTreeWindow> = HashMap::new();
    let mut window_order: Vec<(String, String)> = Vec::new(); // (session id, window id)
    for line in query(client, QUERY_WINDOWS)? {
        let mut fields = line.splitn(5, ' ');
        let (Some(session_id), Some(active), Some(id), Some(layout), Some(name)) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) else {
            continue;
        };
        window_order.push((session_id.to_string(), id.to_string()));
        windows.insert(
            id.to_string(),
            MuxTreeWindow {
                id: id.to_string(),
                name: name.to_string(),
                active: active == "1",
                layout: layout.to_string(),
                panes: Vec::new(),
            },
        );
    }
    let mut panes: HashMap<String, Vec<MuxTreePane>> = HashMap::new();
    for line in query(client, QUERY_PANES)? {
        let mut fields = line.splitn(6, ' ');
        let (Some(window_id), Some(active), Some(id), Some(dead), Some(code), Some(title)) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) else {
            continue;
        };
        // `pane_exit_code` is `-` when the pane is alive or the daemon
        // had no status to read; only dead-with-a-code carries digits.
        let (dead, code) = (dead == "1", code.parse::<i32>().ok());
        panes
            .entry(window_id.to_string())
            .or_default()
            .push(MuxTreePane {
                id: id.to_string(),
                title: title.to_string(),
                active: active == "1",
                cols: 0,
                rows: 0,
                x: 0,
                y: 0,
                process_state: if dead { "exited" } else { "running" }.to_string(),
                exit_code: dead.then_some(code).flatten(),
            });
    }
    // Geometry: each window's layout string gives its panes their
    // rectangles (the full layout rect, matching the string itself). A
    // pane the layout does not hold — a race between the pane query and a
    // layout change — keeps zero rects and stays listed: the pane query
    // is authoritative for membership.
    for (window_id, list) in panes.iter_mut() {
        let Some(window) = windows.get(window_id) else {
            continue;
        };
        let rects: HashMap<u32, (u32, u32, u32, u32)> = layout_panes(&window.layout)
            .into_iter()
            .map(|(id, cols, rows, x, y)| (id, (cols, rows, x, y)))
            .collect();
        for pane in list {
            if let Ok(num) = pane.id.trim_start_matches('%').parse::<u32>() {
                if let Some(&(cols, rows, x, y)) = rects.get(&num) {
                    pane.cols = cols;
                    pane.rows = rows;
                    pane.x = x;
                    pane.y = y;
                }
            }
        }
    }
    // Attach panes to their windows, windows to their sessions, sessions
    // to their workspaces — always in the owning query's row order.
    for (window_id, list) in panes {
        if let Some(window) = windows.get_mut(&window_id) {
            window.panes = list;
        }
    }
    for (session_id, window_id) in &window_order {
        if let Some(window) = windows.remove(window_id) {
            if let Some(session) = sessions.get_mut(session_id) {
                session.windows.push(window);
            }
        }
    }
    for (workspace_id, session_id) in &session_order {
        if let Some(session) = sessions.remove(session_id) {
            if let Some(ws) = workspaces.iter_mut().find(|ws| ws.id == *workspace_id) {
                ws.sessions.push(session);
            }
        }
    }
    Ok(workspaces)
}

/// Every pane rectangle in a tmux layout string, in leaf order, as
/// `(pane number, cols, rows, x, y)`. A leaf is `WxH,X,Y,ID` and a
/// container header `WxH,X,Y` sits last in its bracket run, so chunking
/// each bracket-free run into groups of four fields leaves every header a
/// failed match — the same grammar the mux bridge's
/// `pane_size_in_layout` reads, extended with the origins.
fn layout_panes(layout: &str) -> Vec<(u32, u32, u32, u32, u32)> {
    let body = layout
        .split_once(',')
        .map_or(layout, |(_checksum, rest)| rest);
    body.split(['{', '}', '[', ']'])
        .flat_map(|run| {
            let fields: Vec<&str> = run.split(',').filter(|f| !f.is_empty()).collect();
            fields
                .chunks(4)
                .filter_map(|leaf| match leaf {
                    [size, x, y, id] => {
                        let (w, h) = size.split_once('x')?;
                        Some((
                            id.parse::<u32>().ok()?,
                            w.parse().ok()?,
                            h.parse().ok()?,
                            x.parse().ok()?,
                            y.parse().ok()?,
                        ))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Whether a notification can change any part of the tree snapshot:
/// structural membership, naming, active markers, split geometry, pane
/// titles, or the panes' process lifecycle state. Agent state is the
/// roster watcher's domain.
fn is_structural(notification: &TmuxNotification) -> bool {
    matches!(
        notification,
        TmuxNotification::WorkspacesChanged
            | TmuxNotification::SessionsChanged
            | TmuxNotification::SessionChanged { .. }
            | TmuxNotification::ClientSessionChanged { .. }
            | TmuxNotification::SessionRenamed { .. }
            | TmuxNotification::SessionWindowChanged { .. }
            | TmuxNotification::WindowAdd { .. }
            | TmuxNotification::UnlinkedWindowAdd { .. }
            | TmuxNotification::WindowClose { .. }
            | TmuxNotification::UnlinkedWindowClose { .. }
            | TmuxNotification::WindowRenamed { .. }
            | TmuxNotification::UnlinkedWindowRenamed { .. }
            | TmuxNotification::WindowPaneChanged { .. }
            | TmuxNotification::LayoutChange { .. }
            | TmuxNotification::PaneTitleChanged { .. }
            | TmuxNotification::PaneExited { .. }
            | TmuxNotification::PaneRespawned { .. }
    )
}

fn run(
    socket: &std::path::Path,
    tree: &Mutex<Vec<MuxTreeWorkspace>>,
    stop: &AtomicBool,
    on_message: &OnTreeMessage,
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
        // Notifications queue on the client from connect on, so a change
        // landing while the fetch runs is answered by the loop below.
        let workspaces = match fetch_tree(&mut client) {
            Ok(workspaces) => workspaces,
            Err(_) => {
                std::thread::sleep(backoff);
                backoff = next_backoff(backoff, None);
                continue;
            }
        };
        {
            let mut guard = tree.lock();
            *guard = workspaces;
            on_message(tree_message(&guard));
        }
        let connected_at = Instant::now();
        while let Ok(notification) = client.notifications().recv() {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            if !is_structural(&notification) {
                continue;
            }
            // Fold everything already queued behind the first structural
            // notification into the one refetch: the update is a whole
            // fresh snapshot, so N changes in a burst cost one fetch. A
            // change arriving during that refetch is answered by the next
            // blocking recv.
            while client.notifications().try_recv().is_ok() {}
            let Ok(workspaces) = fetch_tree(&mut client) else {
                break; // degraded connection: the redial path refetches.
            };
            {
                let mut guard = tree.lock();
                *guard = workspaces;
                on_message(tree_message(&guard));
            }
        }
        // Connection lost. A connection that held past BACKOFF_STABLE
        // proves the daemon healthy: redial immediately at the initial
        // delay. A short-lived one is a flap — wait out the cumulative
        // backoff and grow it, as the roster watcher does.
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
    use crate::streaming::mux_factory::test_wait;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::sync::mpsc::{channel, Receiver};

    /// What the double serves on one accepted connection: canned bodies
    /// for the four tree queries, then pushed lines.
    struct Conn {
        workspaces: &'static str,
        sessions: &'static str,
        windows: &'static str,
        panes: &'static str,
        then_send: &'static [&'static str],
        /// Keep the connection open until the test ends instead of
        /// dropping it.
        hold: bool,
    }

    /// Reply to whichever tree query arrives with its canned body.
    fn serve(dir: &std::path::Path, conns: Vec<Conn>) -> PathBuf {
        let path = dir.join("tree-double.sock");
        let listener = UnixListener::bind(&path).expect("bind double");
        std::thread::spawn(move || {
            for mut conn in conns {
                let Ok((stream, _)) = listener.accept() else {
                    return;
                };
                let mut stream = stream;
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                // A non-holding connection answers exactly one fetch (the
                // four tree queries) and then drops: the watcher redials
                // into the next accepted connection.
                let mut answered = 0;
                loop {
                    let mut line = String::new();
                    match reader.read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                    let command = line.trim();
                    let body = match command {
                        c if c.starts_with("list-workspaces") => conn.workspaces,
                        c if c.starts_with("list-sessions") => conn.sessions,
                        c if c.starts_with("list-windows") => conn.windows,
                        c if c.starts_with("list-panes") => conn.panes,
                        _ => "",
                    };
                    write!(
                        stream,
                        "%begin 1 0 1\n{body}{}%end 1 0 1\n",
                        if body.is_empty() { "" } else { "\n" }
                    )
                    .expect("reply");
                    for push in conn.then_send {
                        writeln!(stream, "{push}").expect("push");
                    }
                    // Only the first command of this connection gets the
                    // pushes; later ones (a refetch) get fresh bodies.
                    conn.then_send = &[];
                    answered += 1;
                    if !conn.hold && answered >= 4 {
                        break;
                    }
                }
            }
        });
        path
    }

    fn collect() -> (OnTreeMessage, Receiver<ServerMessage>) {
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

    fn workspaces(msg: ServerMessage) -> Vec<MuxTreeWorkspace> {
        match msg {
            ServerMessage::MuxTreeSnapshot { workspaces } => workspaces,
            other => panic!("expected a tree snapshot, got {other:?}"),
        }
    }

    /// The double's fixture: two workspaces, the first active, holding a
    /// session each; the active workspace's session has a split window
    /// and a second, inactive window. Exercises every field the fetch
    /// assembles, including geometry.
    const WS: &str = "+1: main active\n+2: side\n";
    const SESS: &str = "+1 1 $0 work\n+1 0 $1 build\n+2 1 $2 side-sess\n";
    const WINS: &str =
        "$0 1 @0 b25d,80x24,0,0{40x24,0,0,0,40x24,41,0,1} editor\n$0 0 @1 0000,80x24,0,0,2 logs\n$2 1 @2 0000,80x24,0,0,3 term\n";
    const PANES: &str = "@0 1 %0 0 - shell\n@0 0 %1 0 - vim\n@1 0 %2 1 7 logs\n@2 1 %3 0 - zsh\n";

    #[test]
    fn snapshot_assembles_the_full_tree_from_the_queries() {
        let dir = tempfile::tempdir().unwrap();
        let path = serve(
            dir.path(),
            vec![Conn {
                workspaces: WS,
                sessions: SESS,
                windows: WINS,
                panes: PANES,
                then_send: &[],
                hold: true,
            }],
        );
        let (on_message, rx) = collect();
        let watcher = TreeWatcher::spawn(path, on_message);
        let tree = workspaces(next(&rx));
        assert_eq!(tree.len(), 2, "{tree:?}");
        let main = &tree[0];
        assert_eq!(main.id, "+1");
        assert_eq!(main.name, "main");
        assert!(main.active);
        assert_eq!(main.sessions.len(), 2, "both sessions ride their workspace");
        let work = &main.sessions[0];
        assert_eq!((work.id.as_str(), work.name.as_str()), ("$0", "work"));
        assert!(work.active, "the workspace's active session is marked");
        assert_eq!(work.windows.len(), 2, "windows ride in window order");
        let split = &work.windows[0];
        assert_eq!((split.id.as_str(), split.name.as_str()), ("@0", "editor"));
        assert!(split.active);
        assert_eq!(split.panes.len(), 2, "the split window carries both panes");
        let (left, right) = (&split.panes[0], &split.panes[1]);
        assert_eq!(left.id, "%0");
        assert_eq!(left.title, "shell");
        assert!(left.active, "the window's active pane is marked");
        assert_eq!((left.cols, left.rows, left.x, left.y), (40, 24, 0, 0));
        assert!(!right.active);
        assert_eq!((right.cols, right.rows, right.x, right.y), (40, 24, 41, 0));
        // The inactive window of the inactive... still fully carried.
        assert!(!work.windows[1].active);
        let logs = &work.windows[1].panes[0];
        assert_eq!(logs.id, "%2");
        assert_eq!(logs.process_state, "exited", "the fixture's dead pane");
        assert_eq!(logs.exit_code, Some(7));
        // The other workspace and its non-active session.
        let side = &tree[1];
        assert!(!side.active);
        assert_eq!(side.sessions[0].id, "$2");
        assert_eq!(
            side.sessions[0].windows[0].panes[0].title, "zsh",
            "panes ride under inactive workspaces too"
        );
        // The cache serves requests without a daemon round trip.
        assert_eq!(workspaces(watcher.snapshot()), tree);
    }

    #[test]
    fn structural_notifications_refetch_and_broadcast() {
        let dir = tempfile::tempdir().unwrap();
        let path = serve(
            dir.path(),
            vec![Conn {
                workspaces: WS,
                sessions: SESS,
                windows: WINS,
                panes: PANES,
                then_send: &["%window-close @1"],
                hold: true,
            }],
        );
        let (on_message, rx) = collect();
        let _watcher = TreeWatcher::spawn(path, on_message);
        let _initial = next(&rx);
        // The push lands, the watcher refetches (same bodies: the double
        // is static) and broadcasts a fresh snapshot.
        let refreshed = workspaces(next(&rx));
        assert_eq!(refreshed.len(), 2);
    }

    #[test]
    fn non_structural_notifications_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = serve(
            dir.path(),
            vec![Conn {
                workspaces: WS,
                sessions: SESS,
                windows: WINS,
                panes: PANES,
                then_send: &["%paste-buffer-changed b0"],
                hold: true,
            }],
        );
        let (on_message, rx) = collect();
        let _watcher = TreeWatcher::spawn(path, on_message);
        let _initial = next(&rx);
        // A non-structural push triggers no refetch: only the initial
        // snapshot ever arrives.
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
    }

    #[test]
    fn reconnect_resyncs_with_a_fresh_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = serve(
            dir.path(),
            vec![
                Conn {
                    workspaces: WS,
                    sessions: SESS,
                    windows: WINS,
                    panes: PANES,
                    then_send: &[],
                    hold: false,
                },
                Conn {
                    workspaces: "+9: only\n",
                    sessions: "+9 1 $9 s\n",
                    windows: "",
                    panes: "",
                    then_send: &[],
                    hold: true,
                },
            ],
        );
        let (on_message, rx) = collect();
        let watcher = TreeWatcher::spawn(path, on_message);
        let _initial = next(&rx);
        // The double dropped the connection: the watcher redials and the
        // fresh broadcast carries the tree as the daemon now reports it —
        // including a workspace the watcher never saw a notification for.
        let tree = workspaces(next(&rx));
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].id, "+9");
        assert_eq!(workspaces(watcher.snapshot()).len(), 1);
    }

    #[test]
    fn layout_panes_covers_the_grammar() {
        let single = "b25d,80x24,0,0,0";
        assert_eq!(layout_panes(single), vec![(0, 80, 24, 0, 0)]);
        let split = "0000,81x24,0,0{40x24,0,0,1,40x24,41,0,2}";
        assert_eq!(
            layout_panes(split),
            vec![(1, 40, 24, 0, 0), (2, 40, 24, 41, 0)]
        );
        let nested = "0000,80x24,0,0[80x12,0,0,3,80x11,0,13{40x11,0,13,4,39x11,41,13,5}]";
        assert_eq!(
            layout_panes(nested),
            vec![(3, 80, 12, 0, 0), (4, 40, 11, 0, 13), (5, 39, 11, 41, 13),]
        );
        assert!(layout_panes("garbage").is_empty());
    }

    // --- Against a real in-process daemon -------------------------------

    use crate::mux::{MuxClient, MuxServer};

    /// Owns the serving thread: Drop raises the shutdown flag and joins,
    /// bounded, so a finished test cannot leak a detached daemon thread
    /// holding the socket (QA-185).
    struct ServingGuard {
        shutdown: Arc<AtomicBool>,
        serving: Option<std::thread::JoinHandle<()>>,
    }

    impl Drop for ServingGuard {
        fn drop(&mut self) {
            self.shutdown.store(true, Ordering::Relaxed);
            if let Some(serving) = self.serving.take() {
                let (done_tx, done_rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let _ = serving.join();
                    let _ = done_tx.send(());
                });
                if done_rx.recv_timeout(test_wait::CEILING).is_err() {
                    eprintln!("tree_watcher daemon still serving after the ceiling");
                }
            }
        }
    }

    /// A served daemon with one session; returns its socket path, the
    /// serving-thread guard, and a control client for driving changes.
    fn daemon() -> (PathBuf, ServingGuard, MuxClient) {
        let dir = tempfile::Builder::new()
            .prefix("par-mux-tree-")
            .tempdir()
            .expect("temp dir");
        let socket = dir.path().join("s");
        let server = MuxServer::bind(&socket).expect("bind");
        let shutdown = server.shutdown_handle();
        let serving = std::thread::spawn(move || server.run());
        let guard = ServingGuard {
            shutdown,
            serving: Some(serving),
        };
        let mut control = MuxClient::connect(&socket).expect("control client");
        control.send("new-session -s work").expect("new-session");
        // The socket dir must outlive the test: leak the TempDir handle.
        std::mem::forget(dir);
        (socket, guard, control)
    }

    /// Poll the message channel until `pred` holds on any snapshot that
    /// arrives; the ceiling is the hang detector, not a latency bound.
    fn wait_tree(
        rx: &Receiver<ServerMessage>,
        pred: impl Fn(&Vec<MuxTreeWorkspace>) -> bool,
        last: impl Fn() -> String,
    ) {
        let end = Instant::now() + test_wait::CEILING;
        loop {
            if let Ok(ServerMessage::MuxTreeSnapshot { workspaces }) =
                rx.recv_timeout(Duration::from_millis(50))
            {
                if pred(&workspaces) {
                    return;
                }
            }
            if Instant::now() >= end {
                panic!("tree never converged: {}", last());
            }
        }
    }

    #[test]
    fn live_daemon_snapshot_carries_the_tree_and_moves_nothing() {
        let (socket, _guard, mut control) = daemon();
        // Selection state the metadata path must not disturb: a second
        // pane, with the FIRST pane selected.
        control.send("split-window -t %0").expect("split");
        control.send("select-pane -t %0").expect("select-pane");

        let (on_message, rx) = collect();
        let watcher = TreeWatcher::spawn(socket.clone(), on_message);
        // Converge on the fetch that sees both panes, then check the
        // snapshot against the daemon's own state.
        wait_tree(
            &rx,
            |tree| {
                tree.iter().any(|ws| {
                    ws.sessions.iter().any(|s| {
                        s.windows
                            .iter()
                            .any(|w| w.panes.len() == 2 && w.panes.iter().any(|p| p.cols > 0))
                    })
                })
            },
            || format!("{:?}", watcher.snapshot()),
        );
        let ServerMessage::MuxTreeSnapshot { workspaces } = watcher.snapshot() else {
            panic!("snapshot is a tree");
        };
        let window = workspaces[0].sessions[0].windows[0].clone();
        assert_eq!(window.panes.len(), 2);
        let active = window.panes.iter().find(|p| p.active).expect("active pane");
        assert_eq!(active.id, "%0", "the daemon's own selection is reported");
        // Split geometry: the default split is top/bottom — the 80x24
        // window becomes an 80x12 leaf and an 80x11 leaf at y=13.
        assert_eq!(
            (
                window.panes[0].cols,
                window.panes[0].rows,
                window.panes[0].x,
                window.panes[0].y
            ),
            (80, 12, 0, 0)
        );
        assert_eq!(
            (
                window.panes[1].cols,
                window.panes[1].rows,
                window.panes[1].x,
                window.panes[1].y
            ),
            (80, 12, 0, 12)
        );
        // Criterion 3: metadata browsing neither resized the pane nor
        // moved the daemon's selection. `pane-info` reports the pane's
        // live size; the split pane stays 80x12, and the `*` marker still
        // names %0.
        let info = control.send_checked("pane-info -t %0").expect("pane-info");
        assert!(info.ok, "{}", info.body.join(" | "));
        assert!(
            info.body.join(" ").contains("80x12"),
            "pane size unchanged by tree fetches: {:?}",
            info.body
        );
        let marker = control
            .send_checked("list-panes -t @0")
            .expect("list-panes");
        // Row shape `%N <leaf> <marker>`: the `*` still names %0.
        assert!(
            marker
                .body
                .iter()
                .any(|row| row.trim_start().starts_with("%0 ") && row.trim_end().ends_with('*')),
            "pane %0 is still the active pane: {:?}",
            marker.body
        );
    }

    #[test]
    fn live_daemon_structural_changes_converge_through_pushes() {
        let (socket, _guard, mut control) = daemon();
        let (on_message, rx) = collect();
        let watcher = TreeWatcher::spawn(socket.clone(), on_message);
        // Converge on the initial snapshot first (it may be empty when a
        // fresh daemon races the watcher's first fetch; the notification
        // from new-session inside daemon() re-fills it).
        wait_tree(
            &rx,
            |tree| {
                tree.iter()
                    .any(|ws| ws.sessions.iter().any(|s| s.id == "$0"))
            },
            || format!("{:?}", watcher.snapshot()),
        );
        // Create: a new window appears, carrying its pane.
        control.send("new-window -t $0").expect("new-window");
        wait_tree(
            &rx,
            |tree| {
                tree.iter()
                    .any(|ws| ws.sessions.iter().any(|s| s.windows.len() == 2))
            },
            || format!("{:?}", watcher.snapshot()),
        );
        // Rename: the window's name follows.
        control
            .send("rename-window -t @0 agent-hub")
            .expect("rename");
        wait_tree(
            &rx,
            |tree| {
                tree.iter().any(|ws| {
                    ws.sessions
                        .iter()
                        .any(|s| s.windows.iter().any(|w| w.name == "agent-hub"))
                })
            },
            || format!("{:?}", watcher.snapshot()),
        );
        // Active selection: the second window becomes current.
        control.send("select-window -t @1").expect("select-window");
        wait_tree(
            &rx,
            |tree| {
                tree.iter().any(|ws| {
                    ws.sessions
                        .iter()
                        .any(|s| s.windows.iter().any(|w| w.id == "@1" && w.active))
                })
            },
            || format!("{:?}", watcher.snapshot()),
        );
        // Layout: a split changes the window's geometry and pane count.
        control.send("split-window -t @1").expect("split");
        wait_tree(
            &rx,
            |tree| {
                tree.iter().any(|ws| {
                    ws.sessions
                        .iter()
                        .any(|s| s.windows.iter().any(|w| w.id == "@1" && w.panes.len() == 2))
                })
            },
            || format!("{:?}", watcher.snapshot()),
        );
        // The redial resync is covered by
        // reconnect_resyncs_with_a_fresh_snapshot against the double;
        // this test pins convergence for create/rename/select/layout.
    }

    /// The pane lifecycle rides the tree surface too: `%pane-exited`,
    /// `%pane-respawned` and `kill-pane` each converge in a fresh
    /// snapshot — exited-with-status, running, then gone — so tree
    /// browsers (including `?metadata=1` clients) read the same truth
    /// the pane sessions' `pane_process_state` events carry.
    #[test]
    fn live_daemon_pane_lifecycle_converges_in_the_snapshots() {
        let (socket, _guard, mut control) = daemon();
        control
            .send("set-option remain-on-exit on")
            .expect("set-option");
        let (on_message, rx) = collect();
        let watcher = TreeWatcher::spawn(socket.clone(), on_message);
        wait_tree(
            &rx,
            |tree| {
                tree.iter()
                    .any(|ws| ws.sessions.iter().any(|s| s.id == "$0"))
            },
            || format!("{:?}", watcher.snapshot()),
        );

        // Exit with a status: the shell is replaced by a command that
        // dies with 7; the daemon holds the pane and announces it.
        control
            .send("respawn-pane -k -t %0 sh -c 'exit 7'")
            .expect("respawn to a dying command");
        wait_tree(
            &rx,
            |tree| {
                tree.iter().any(|ws| {
                    ws.sessions.iter().any(|s| {
                        s.windows.iter().any(|w| {
                            w.panes.iter().any(|p| {
                                p.id == "%0"
                                    && p.process_state == "exited"
                                    && p.exit_code == Some(7)
                            })
                        })
                    })
                })
            },
            || format!("{:?}", watcher.snapshot()),
        );

        // Respawn back to the default shell: the exited state clears.
        control
            .send("respawn-pane -k -t %0")
            .expect("respawn shell");
        wait_tree(
            &rx,
            |tree| {
                tree.iter().any(|ws| {
                    ws.sessions.iter().any(|s| {
                        s.windows.iter().any(|w| {
                            w.panes.iter().any(|p| {
                                p.id == "%0"
                                    && p.process_state == "running"
                                    && p.exit_code.is_none()
                            })
                        })
                    })
                })
            },
            || format!("{:?}", watcher.snapshot()),
        );

        // Deletion: kill-pane removes the pane (its emptied session with
        // it); the tree converges without it — no lifecycle field claims
        // a pane that no longer exists.
        control.send("kill-pane -t %0").expect("kill");
        wait_tree(
            &rx,
            |tree| {
                tree.iter().all(|ws| {
                    ws.sessions.iter().all(|s| {
                        s.windows
                            .iter()
                            .all(|w| !w.panes.iter().any(|p| p.id == "%0"))
                    })
                })
            },
            || format!("{:?}", watcher.snapshot()),
        );
    }

    /// A `?metadata=1` connection browses the tree without creating a
    /// pane session: no mirror appears in the registry, the daemon's pane
    /// keeps its size (no smallest-viewer size contribution), and the
    /// client still receives the connect push and the request reply.
    #[test]
    fn metadata_connect_skips_the_mirror_and_serves_the_tree() {
        use crate::streaming::protocol::ClientMessage;
        use crate::streaming::server::SessionFactory;
        use crate::streaming::{
            ConnectionParams, MuxPaneSelector, MuxSessionFactory, StreamingConfig, StreamingServer,
        };

        let (socket, _guard, mut control) = daemon();
        let pane = control
            .send("list-panes")
            .expect("list-panes")
            .into_iter()
            .find(|l| l.trim().starts_with('%'))
            .expect("a pane");
        let pane = pane.trim().to_string();
        // The size token (COLSxROWS) of the pane-info row; the row also
        // carries live counters (cpu_ms) that must not partake.
        let pane_size = |control: &mut MuxClient| {
            let reply = control
                .send_checked(&format!("pane-info -t {pane}"))
                .expect("pane-info");
            reply
                .body
                .join(" ")
                .split_whitespace()
                .find_map(|field| {
                    let (cols, rows) = field.split_once('x')?;
                    cols.parse::<u32>().ok()?;
                    rows.parse::<u32>().ok()?;
                    Some(field.to_string())
                })
                .expect("a COLSxROWS field")
        };
        let before = pane_size(&mut control);

        let factory = Arc::new(MuxSessionFactory::new(
            &socket,
            MuxPaneSelector::FromSessionId,
        ));
        let server = Arc::new(StreamingServer::with_factory(
            "127.0.0.1:0".to_string(),
            StreamingConfig::default(),
            Arc::clone(&factory) as Arc<dyn SessionFactory>,
        ));
        factory.set_streaming_server(Arc::clone(&server));
        let (on_message, _rx) = collect();
        server.set_tree_watcher(Arc::new(TreeWatcher::spawn(socket.clone(), on_message)));

        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let _entered = rt.enter();
        let params = ConnectionParams::from_uri_query(Some("metadata=1"));
        let (mut phone, connection) = rt.block_on(async {
            let (phone, connection) = tokio::io::duplex(65536);
            (
                tokio_tungstenite::WebSocketStream::from_raw_socket(
                    phone,
                    tokio_tungstenite::tungstenite::protocol::Role::Client,
                    None,
                )
                .await,
                tokio_tungstenite::WebSocketStream::from_raw_socket(
                    connection,
                    tokio_tungstenite::tungstenite::protocol::Role::Server,
                    None,
                )
                .await,
            )
        });
        let serving = Arc::clone(&server);
        let _ws = rt.spawn(async move {
            serving
                .handle_test_ws(connection, &params)
                .await
                .expect("metadata websocket session");
        });
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        // The connect push may read an empty cache (the watcher's first
        // fetch races the connect), so the test polls the REQUEST path
        // until the cache has filled: every request is answered from the
        // watcher's cache with no daemon round trip.
        let request_tree_reply = |phone: &mut tokio_tungstenite::WebSocketStream<
            tokio::io::DuplexStream,
        >| {
            rt.block_on(async {
                let bytes =
                    crate::streaming::proto::encode_client_message(&ClientMessage::MuxTreeRequest)
                        .expect("encode");
                phone
                    .send(Message::binary(bytes))
                    .await
                    .expect("client send");
                loop {
                    let frame = tokio::time::timeout(Duration::from_secs(30), phone.next())
                        .await
                        .expect("frame within the ceiling")
                        .expect("ws stream")
                        .expect("ws frame");
                    let Message::Binary(bytes) = frame else {
                        continue;
                    };
                    if let Ok(msg) = crate::streaming::proto::decode_server_message(&bytes) {
                        if matches!(msg, ServerMessage::MuxTreeSnapshot { .. }) {
                            return msg;
                        }
                    }
                }
            })
        };
        let deadline = Instant::now() + test_wait::CEILING;
        let filled = loop {
            let ServerMessage::MuxTreeSnapshot { workspaces } = request_tree_reply(&mut phone)
            else {
                panic!("request reply is a tree");
            };
            if !workspaces.is_empty() {
                break workspaces;
            }
            assert!(
                Instant::now() < deadline,
                "tree cache never filled: {workspaces:?}"
            );
        };
        assert!(
            filled
                .iter()
                .any(|ws| ws.sessions.iter().any(|s| !s.windows.is_empty())),
            "the reply carries the fetched tree: {filled:?}"
        );

        // No pane mirror was created; the metadata routing state exists.
        assert!(
            server.get_session("pane-0").is_none(),
            "no mirror session for a metadata connection"
        );
        assert!(
            server.get_session("metadata").is_some(),
            "the metadata routing state exists while connected"
        );
        // The pane kept its size: no size contribution was made.
        let after = pane_size(&mut control);
        assert_eq!(
            after, before,
            "pane size untouched: {before:?} vs {after:?}"
        );
        // Dropping the client removes the routing state with it.
        drop(phone);
        test_wait::until(
            "metadata state gone with its last client",
            test_wait::CEILING,
            || (server.get_session("metadata").is_none()).then_some(()),
            || "metadata state still registered".to_string(),
        );
    }

    #[test]
    fn tree_queries_are_read_only_list_commands() {
        // The fetch's whole daemon surface, pinned: every query is a
        // `list-*` read — no select-*, no resize, no pane I/O can ride
        // the metadata path.
        for query in [QUERY_WORKSPACES, QUERY_SESSIONS, QUERY_WINDOWS, QUERY_PANES] {
            assert!(
                query.starts_with("list-"),
                "metadata query is not read-only: {query:?}"
            );
        }
    }

    #[test]
    fn backoff_policy_matches_the_roster_watcher() {
        assert_eq!(
            next_backoff(BACKOFF_INITIAL, None),
            Duration::from_millis(200)
        );
        assert_eq!(next_backoff(BACKOFF_MAX, None), BACKOFF_MAX);
        assert_eq!(
            next_backoff(BACKOFF_MAX, Some(BACKOFF_STABLE)),
            BACKOFF_INITIAL
        );
    }
}
