//! WindowSession suites: status refresh round trips, roster, and sidebar agents.

use super::*;
use crate::mux::attach::status::RosterEntry;

/// ARC-125's round-trip bound: one status refresh over five sessions
/// and four visible panes, with the side panel up and one pane's title
/// marked stale by `%pane-title-changed`, costs at most six commands
/// (it was 5 + 4 + 6). One `list-windows -a` finds the owner and its
/// windows, the side panel reuses the refresh's `list-workspaces`, the
/// focused pane's title rides the bar's own query, and only the stale
/// pane is re-queried.
#[test]
fn status_refresh_round_trips_are_bounded() {
    let (rx, mut conn) = fake_daemon("rtcount", five_session_script());
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(FOUR_PANE).expect("parses"));
    session.window = "@0".to_string();
    session.sidebar.set(true);
    session.renderer.focus(1);
    // Steady state: every pane's title was learned on an earlier
    // refresh; then %3's user title is cleared (a set title patches from
    // the payload since ENH-043; a cleared one falls back to the OSC
    // title the payload lacks, so it re-queries).
    for pane in 1..=4 {
        session.renderer.set_user_title(pane, "t");
    }
    session.handle_event(TmuxNotification::PaneTitleChanged {
        pane_id: "%3".to_string(),
        title: String::new(),
    });
    drained(&rx);

    assert_eq!(session.refresh_status(&mut conn), EventOutcome::Continue);
    let sent = drained(&rx);
    assert_eq!(
        sent,
        vec![
            "list-sessions",
            "list-workspaces",
            "list-windows -a",
            "pane-title -t %1",
            "list-agents",
            "pane-title -t %3",
        ],
        "one refresh's wire"
    );
    assert!(sent.len() <= 6);
    assert_eq!(session.status.session_id.as_deref(), Some("$4"));
    assert!(!session.status_inflight.dirty);
}

/// A daemon that stops answering costs the pump at most the status
/// bound, not the 10 s reply timeout: the refresh stops at the first
/// late query, keeps the stale bar, and re-marks the status dirty. The
/// late reply is discarded, so the next command reads its own.
#[test]
fn status_refresh_times_out_and_keeps_replies_aligned() {
    let mut script = five_session_script();
    script.delays.insert(
        "list-sessions".to_string(),
        std::time::Duration::from_millis(900),
    );
    script
        .replies
        .insert("pane-info -t %1".to_string(), "%1 @0 20x24".to_string());
    let (rx, mut conn) = fake_daemon("rttimeout", script);
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(FOUR_PANE).expect("parses"));
    session.window = "@0".to_string();
    drained(&rx);

    let started = std::time::Instant::now();
    assert_eq!(session.refresh_status(&mut conn), EventOutcome::Continue);
    let took = started.elapsed();
    assert!(
        took < std::time::Duration::from_millis(800),
        "bounded by the status timeout: {took:?}"
    );
    assert!(
        !session.status_inflight.dirty,
        "a timeout leaves the re-query to the next poll or event — an immediate \
         re-issue queues behind the same slow reply, one 500 ms bound per pass"
    );
    assert_eq!(
        drained(&rx),
        vec!["list-sessions"],
        "stopped at the timeout"
    );

    // The late list-sessions reply must not answer this command.
    let reply = conn.send_checked("pane-info -t %1").expect("reply");
    assert_eq!(reply.body, vec!["%1 @0 20x24".to_string()]);
}

/// ENH-049 (superseding ARC-137's blocking bound): a user-initiated switch
/// against a slow daemon returns at once, the late reply lands on a later
/// tick, and the next sync command still reads its own answer.
#[test]
fn slow_switch_returns_at_once_and_keeps_replies_aligned() {
    let mut script = FakeScript::default();
    script.delays.insert(
        "list-sessions".to_string(),
        std::time::Duration::from_millis(500),
    );
    script
        .replies
        .insert("pane-info -t %1".to_string(), "%1 @0 40x24".to_string());
    let (rx, mut conn, mut session) = two_pane_session("int-switch", script);

    let started = std::time::Instant::now();
    session.route_plain(&[crate::mux::attach::C_B, b')'], &mut conn);
    let took = started.elapsed();
    assert!(
        took < std::time::Duration::from_millis(200),
        "the pump does not wait for the daemon: {took:?}"
    );
    assert_eq!(session.window, "@0", "the view stays put");
    settle(&mut session, &mut conn);
    assert_eq!(
        drained(&rx),
        vec!["list-sessions"],
        "the empty roster ends the chain"
    );

    let reply = conn.send_checked("pane-info -t %1").expect("reply");
    assert_eq!(reply.body, vec!["%1 @0 40x24".to_string()]);
}

/// ENH-049: `focus_pane` writes the select and the merged switch back to
/// back; a slow select delays neither the call nor the switch's write.
#[test]
fn focus_pane_returns_before_a_slow_select_answers() {
    let mut script = FakeScript::default();
    script.delays.insert(
        "select-pane -t %7".to_string(),
        std::time::Duration::from_millis(500),
    );
    let (rx, mut conn, mut session) = two_pane_session("int-focus", script);

    let started = std::time::Instant::now();
    session.focus_pane(&mut conn, 7);
    let took = started.elapsed();
    assert!(
        took < std::time::Duration::from_millis(200),
        "the pump does not wait for the daemon: {took:?}"
    );
    settle(&mut session, &mut conn);
    assert_eq!(
        drained(&rx),
        vec!["select-pane -t %7", "switch-client -t %7"],
        "pipelined in order"
    );
    assert_eq!(session.chrome.flash_text(), None);
}

/// An agent-row click lights its row on the click's own frame: focus_pane
/// moves the client-local focus (`renderer.focused()` — what the panel's
/// rows activate on) and rebuilds the panel from the cache, instead of
/// waiting for the throttled refresh a quiet pane may not tick for (the
/// second-click report).
#[test]
fn sidebar_agent_row_click_highlights_the_row_immediately() {
    let (_rx, mut conn, mut session) = two_pane_session("agent-hl", FakeScript::default());
    show_sidebar(&mut session);
    session.status.apply_snapshot_for_test(&[
        "snapshot 1".to_string(),
        "workspace +0 * app".to_string(),
        "session $0 * +0 main".to_string(),
        "window @0 * 1".to_string(),
        "agent %1 claude working hook".to_string(),
        "agent %2 kimi idle hook".to_string(),
    ]);
    session.rebuild_agent_section();
    let selected = |session: &WindowSession| -> Vec<u32> {
        session
            .renderer
            .sidebar_sections
            .clone()
            .expect("sections")
            .iter()
            .flat_map(|s| s.rows.iter())
            .filter_map(|row| match row.id {
                SidebarRowId::Pane(p) if row.active => Some(p),
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        selected(&session),
        vec![1],
        "the layout's first pane starts focused"
    );
    session.focus_pane(&mut conn, 2);
    settle(&mut session, &mut conn);
    assert_eq!(
        selected(&session),
        vec![2],
        "the clicked row lights on the click's own frame"
    );
}

/// An agent-roster entry.
fn roster_entry(pane: u32, agent: &str, state: &str, workspace: Option<&str>) -> RosterEntry {
    RosterEntry {
        pane,
        agent: agent.to_string(),
        state: state.to_string(),
        workspace: workspace.map(str::to_string),
        session: None,
        window: None,
        project: None,
        message: None,
    }
}

/// The workspace rows' `(id, label)` pairs, in panel order.
fn workspace_area(session: &WindowSession) -> Vec<(String, String)> {
    let sections = session.renderer.sidebar_sections.clone().expect("sections");
    sections
        .iter()
        .find(|s| s.title == "workspaces")
        .expect("workspace area")
        .rows
        .iter()
        .filter_map(|row| match &row.id {
            SidebarRowId::Workspace(id) => Some((id.clone(), row.label.clone())),
            _ => None,
        })
        .collect()
}

/// Linked worktree workspaces group under their repo workspace's row
/// (herdr's indented children): each follows its parent, tree-prefixed
/// and labelled by branch; the grouping comes from the snapshot's
/// `worktree` rows (`list-worktrees`' facts) and survives the agent
/// notification's no-round-trip rebuild. A worktree whose repo
/// workspace is closed (`parent` `-`) keeps its id-order place.
#[test]
fn sidebar_groups_worktree_workspaces_under_their_repo() {
    let lines = |body: &[&str]| body.iter().map(|l| l.to_string()).collect::<Vec<_>>();
    let mut session = snapshot_session();
    session.status.apply_snapshot_for_test(&lines(&[
        "snapshot 1",
        "workspace +0 * app",
        "workspace +1 - notes",
        "workspace +2 - app/feat/login",
        "workspace +3 - app/fix",
        "workspace +4 - lone/wip",
        "session $0 * +0 main",
        "window @0 * 1",
        "worktree +0 repo - main /r/app",
        "worktree +2 worktree +0 feat/login /r/app-worktrees/feat-login",
        "worktree +3 worktree +0 fix /r/app-worktrees/fix",
        "worktree +4 worktree - wip /r/lone-worktrees/wip",
    ]));
    session.rebuild_agent_section();
    let want = vec![
        ("+0".to_string(), "app".to_string()),
        ("+2".to_string(), "├─ feat/login".to_string()),
        ("+3".to_string(), "└─ fix".to_string()),
        ("+1".to_string(), "notes".to_string()),
        ("+4".to_string(), "lone/wip".to_string()),
    ];
    assert_eq!(workspace_area(&session), want);

    // The full refresh path builds through the same helper.
    let roster: Vec<(String, String, bool)> = session
        .status
        .workspaces()
        .iter()
        .map(|(id, name)| (id.clone(), name.clone(), id == "+0"))
        .collect();
    session.renderer.set_sidebar_sections(None);
    session.install_panel_sections(&roster);
    assert_eq!(workspace_area(&session), want);

    // A snapshot without worktree rows ungroups.
    session.status.apply_snapshot_for_test(&lines(&[
        "snapshot 1",
        "workspace +0 * app",
        "workspace +2 - app/feat/login",
        "session $0 * +0 main",
        "window @0 * 1",
    ]));
    session.rebuild_agent_section();
    assert_eq!(
        workspace_area(&session),
        vec![
            ("+0".to_string(), "app".to_string()),
            ("+2".to_string(), "app/feat/login".to_string()),
        ]
    );
}

/// A three-pane render session showing `@0` with the side panel up.
/// A pane program's OSC 52 copy — local or over SSH — bridges to the
/// host: the focused pane's clipboard state rides the frame's outbox as
/// the same wire form the yank writes, deduped per pane, and a pane
/// switch forwards the pane it lands on (par-term's clipboard_sync
/// contract, attach side).
#[test]
fn pane_osc52_clipboard_bridges_to_the_host_outbox() {
    let mut session = snapshot_session();
    session.feed_pane("%1", b"\x1b]52;c;aGVsbG8=\x07");
    session.bridge_pane_clipboard();
    let out = session.notify.outbox.clone();
    let needle = b"\x1b]52;c;aGVsbG8=\x1b\\";
    assert!(
        out.windows(needle.len()).any(|w| w == needle),
        "the pane's copy forwards: {:?}",
        String::from_utf8_lossy(&out)
    );

    // An unchanged clipboard state does not re-send.
    let len_before = session.notify.outbox.len();
    session.bridge_pane_clipboard();
    assert_eq!(session.notify.outbox.len(), len_before, "deduped");

    // New content re-forwards.
    session.feed_pane("%1", b"\x1b]52;c;aGVsbG8gd29ybGQ=\x07");
    session.bridge_pane_clipboard();
    let out = session.notify.outbox.clone();
    let needle = b"\x1b]52;c;aGVsbG8gd29ybGQ=\x1b\\";
    assert!(
        out.windows(needle.len()).any(|w| w == needle),
        "new content forwards: {:?}",
        String::from_utf8_lossy(&out)
    );

    // A pane switch forwards the pane it lands on.
    session.renderer.focus(2);
    session.feed_pane("%2", b"\x1b]52;c;cGFuZTI=\x07");
    session.bridge_pane_clipboard();
    let out = session.notify.outbox.clone();
    let needle = b"\x1b]52;c;cGFuZTI=\x1b\\";
    assert!(
        out.windows(needle.len()).any(|w| w == needle),
        "the landed pane forwards: {:?}",
        String::from_utf8_lossy(&out)
    );
}

/// The move-active-tab chords (tmux's `<` / `>`): `swap-window -s
/// <shown> -t <neighbor>` within the shown session — and an edge press
/// flashes instead of sending (no wrap).
#[test]
fn move_window_chords_swap_the_shown_window_with_its_neighbor() {
    let (rx, mut conn) = fake_daemon("movetab", snapshot_script());
    let mut session = snapshot_session();
    session
        .refresh_status_facts(&mut conn)
        .expect("snapshot refresh");
    drained(&rx);

    session.route_stdin_bytes(&mut conn, &[0x02, b'>']);
    assert_eq!(
        received_within_300ms(&rx),
        vec!["swap-window -s @0 -t @1".to_string()],
        "the shown window swaps with its right neighbor"
    );

    // The first tab has no left neighbor: a flash, no wire line.
    session.route_stdin_bytes(&mut conn, &[0x02, b'<']);
    assert!(
        received_within_300ms(&rx).is_empty(),
        "an edge press never wraps"
    );
    assert_eq!(session.chrome.flash_text(), Some("already the first tab"));
}

/// The agents section groups by workspace (herdr's panel): rows sort by
/// their workspace's position in the id-ordered cache, the focused
/// pane's row activates, and an unknown workspace keeps the bare
/// `{agent} ({state})` label and trails as the last group (ARC-138).
#[test]
fn sidebar_agents_section_groups_by_workspace() {
    let mut session = snapshot_session();
    let roster = vec![("+0".to_string(), "main".to_string(), true)];
    let agents = vec![
        roster_entry(3, "kimi", "blocked", Some("+1")),
        roster_entry(2, "claude", "working", Some("+0")),
        roster_entry(5, "pi", "idle", None),
    ];
    let names = vec![
        ("+0".to_string(), "main".to_string()),
        ("+1".to_string(), "zeta".to_string()),
    ];
    let sections = WindowSession::sidebar_sections(
        &roster,
        &agents,
        &names,
        Some(2),
        &crate::mux::config::default_agent_rows(),
        crate::mux::attach::panels::AgentSortMode::Grouped,
    );
    session.renderer.set_sidebar_sections(sections);
    let sections = session.renderer.sidebar_sections.clone().expect("sections");
    assert_eq!(sections.len(), 2, "{sections:?}");
    let rows = &sections[1].rows;
    assert_eq!(rows[0], arow(2, Some("main"), "claude", "working", true));
    assert_eq!(rows[1], arow(3, Some("zeta"), "kimi", "blocked", false));
    assert_eq!(rows[2], arow(5, None, "pi", "idle", false));
}

/// A `done` agent shows in the agents section with the done status, and
/// its workspace row carries it over a working sibling (herdr's
/// priority: blocked > done > working > idle).
#[test]
fn sidebar_shows_done_agents_and_ranks_them_over_working() {
    use crate::mux::attach::panels::AgentStatus;
    let roster = vec![("+0".to_string(), "main".to_string(), true)];
    let agents = vec![
        roster_entry(2, "claude", "working", Some("+0")),
        roster_entry(3, "pi", "done", Some("+0")),
    ];
    let names = vec![("+0".to_string(), "main".to_string())];
    let sections = WindowSession::sidebar_sections(
        &roster,
        &agents,
        &names,
        None,
        &crate::mux::config::default_agent_rows(),
        crate::mux::attach::panels::AgentSortMode::Grouped,
    )
    .expect("sections");
    // Grouped sub-sorts by status priority: the done pi leads.
    assert_eq!(
        sections[1].rows[0],
        arow(3, Some("main"), "pi", "done", false)
    );
    assert_eq!(sections[1].rows[0].status, AgentStatus::Done);
    assert_eq!(sections[0].rows[0].status, AgentStatus::Done);
    let custom = vec![vec!["agent".to_string(), "state".to_string()]];
    let sections = WindowSession::sidebar_sections(
        &roster,
        &agents,
        &names,
        None,
        &custom,
        crate::mux::attach::panels::AgentSortMode::Grouped,
    )
    .expect("sections");
    assert_eq!(sections[1].rows[0].label, "pi \u{b7} done");
}

/// The agent entry follows `agent-rows`: the default puts the project
/// beside the workspace (once a probe reported it), a custom table
/// renders its own tokens and lines, and the entry's workspace row
/// carries its agents' most urgent state.
#[test]
fn sidebar_agent_entries_follow_agent_rows() {
    use crate::mux::attach::panels::{AgentStatus, STATE_ICON};
    let roster = vec![("+0".to_string(), "main".to_string(), true)];
    let mut agents = vec![
        roster_entry(2, "claude", "working", Some("+0")),
        roster_entry(3, "kimi", "blocked", Some("+0")),
    ];
    agents[0].project = Some("par-term".to_string());
    let names = vec![("+0".to_string(), "main".to_string())];
    let default = WindowSession::sidebar_sections(
        &roster,
        &agents,
        &names,
        None,
        &crate::mux::config::default_agent_rows(),
        crate::mux::attach::panels::AgentSortMode::Grouped,
    )
    .expect("sections");
    // Grouped sub-sorts by status priority: the blocked kimi leads.
    assert_eq!(
        default[1].rows[1].label,
        format!("{STATE_ICON} main \u{b7} par-term")
    );
    assert_eq!(default[1].rows[1].continuation, vec!["claude".to_string()]);
    assert_eq!(
        default[0].rows[0].status,
        AgentStatus::Blocked,
        "the workspace carries its most urgent agent"
    );
    let custom = vec![
        vec!["agent".to_string(), "state".to_string()],
        vec!["pane".to_string(), "project".to_string()],
    ];
    let sections = WindowSession::sidebar_sections(
        &roster,
        &agents,
        &names,
        None,
        &custom,
        crate::mux::attach::panels::AgentSortMode::Grouped,
    )
    .expect("sections");
    assert_eq!(sections[1].rows[0].label, "kimi \u{b7} blocked");
    assert_eq!(sections[1].rows[1].label, "claude \u{b7} working");
    assert_eq!(
        sections[1].rows[1].continuation,
        vec!["%2 \u{b7} par-term".to_string()]
    );
    assert_eq!(
        sections[1].rows[0].continuation,
        vec!["%3".to_string()],
        "an unprobed project drops out of its line"
    );
}

/// Two workspaces sharing a display name stay two groups: the key is the
/// workspace id's position, not the label head (ARC-138).
#[test]
fn sidebar_agents_group_by_workspace_id_not_name() {
    let mut session = snapshot_session();
    let roster = vec![("+0".to_string(), "dev".to_string(), true)];
    let agents = vec![
        roster_entry(4, "a", "idle", Some("+1")),
        roster_entry(3, "b", "idle", Some("+0")),
        roster_entry(2, "c", "idle", Some("+1")),
    ];
    let names = vec![
        ("+0".to_string(), "dev".to_string()),
        ("+1".to_string(), "dev".to_string()),
    ];
    let sections = WindowSession::sidebar_sections(
        &roster,
        &agents,
        &names,
        None,
        &crate::mux::config::default_agent_rows(),
        crate::mux::attach::panels::AgentSortMode::Grouped,
    );
    session.renderer.set_sidebar_sections(sections);
    let sections = session.renderer.sidebar_sections.clone().expect("sections");
    let ids: Vec<SidebarRowId> = sections[1].rows.iter().map(|row| row.id.clone()).collect();
    // Group `+0` (pane 3), then group `+1` with panes ascending (2, 4).
    // A label-head key would merge both "dev" groups into pane order 2, 3, 4.
    assert_eq!(
        ids,
        [
            SidebarRowId::Pane(3),
            SidebarRowId::Pane(2),
            SidebarRowId::Pane(4)
        ]
    );
}

/// An agent event for a pane the roster lacks inserts a row with no
/// workspace (the payload carries no `ws=`): the status goes stale so the
/// throttled refresh places it, the row trails the grouped rows even
/// though its pane id is lower, and no query goes out (ARC-138).
#[test]
fn agent_insert_marks_status_dirty_and_rows_trail_ungrouped() {
    let mut script = snapshot_script();
    let snapshot = script
        .replies
        .get("client-snapshot -t @0")
        .expect("snapshot reply")
        .replace(
            "agent %1 claude working hook",
            "agent %1 claude working hook ws=+0",
        )
        .replace(
            "agent %3 kimi blocked hook",
            "agent %3 kimi blocked hook ws=+1",
        );
    script
        .replies
        .insert("client-snapshot -t @0".to_string(), snapshot);
    let (rx, mut conn) = fake_daemon("snapinsert", script);
    let mut session = snapshot_session();
    session.refresh_status_facts(&mut conn).expect("seed");
    session.status_inflight.dirty = false;
    drained(&rx);

    session.handle_event(TmuxNotification::AgentStateChanged {
        pane_id: "%2".to_string(),
        agent: "claude".to_string(),
        state: "blocked".to_string(),
        source: "hook".to_string(),
        ws: String::new(),
        msg: String::new(),
    });
    assert!(session.status_inflight.dirty);
    let sections = session.renderer.sidebar_sections.clone().expect("sections");
    assert_eq!(
        sections[1].rows,
        vec![
            arow(1, Some("main"), "claude", "working", true),
            arow(3, Some("side"), "kimi", "blocked", false),
            arow(2, None, "claude", "blocked", false),
        ]
    );
    assert!(received_within_300ms(&rx).is_empty(), "no round trip");
}
