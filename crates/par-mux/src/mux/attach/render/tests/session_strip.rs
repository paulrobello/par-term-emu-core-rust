//! The session strip's render tests: the row's drawing and hit-testing,
//! the auto-hide at one workspace session, the agent-state dots, and the
//! restart toast.

use super::*;

/// One workspace, two sessions, the first shown, one agent working in
/// the shown session.
fn two_session_body() -> Vec<String> {
    vec![
        "snapshot 1".to_string(),
        "workspace +1 * work".to_string(),
        "session $1 * +1 main".to_string(),
        "session $2 - +1 review".to_string(),
        "window @1 * dev".to_string(),
        "pane %1 hello".to_string(),
        "agent %1 claude working hook session=$1 win=@1".to_string(),
    ]
}

fn one_session_body() -> Vec<String> {
    vec![
        "snapshot 1".to_string(),
        "workspace +1 * work".to_string(),
        "session $1 * +1 main".to_string(),
        "window @1 * dev".to_string(),
        "pane %1 hello".to_string(),
    ]
}

/// Apply a snapshot body to the session's status state and repaint both
/// strips the way the status continuation does.
fn apply_and_draw(session: &mut WindowSession, body: Vec<String>) {
    let reply = crate::mux::client::Reply { ok: true, body };
    session
        .status
        .apply_snapshot_reply(&reply, 1)
        .expect("snapshot applies");
    session.draw_strips();
}

fn strip_row(session: &WindowSession) -> String {
    (0..session.chrome.session_strip.cols())
        .map(|x| {
            session.chrome.session_strip.buffer()[(x, 0)]
                .symbol()
                .to_string()
        })
        .collect()
}

fn tab_row(session: &WindowSession) -> String {
    (0..session.chrome.tab_strip.cols())
        .map(|x| {
            session.chrome.tab_strip.buffer()[(x, 0)]
                .symbol()
                .to_string()
        })
        .collect()
}

fn press(col: u16, row: u16) -> SgrMouse {
    SgrMouse {
        cb: 0,
        col: col + 1,
        row: row + 1,
        release: false,
    }
}

/// The strip paints one entry per workspace session above the tab strip;
/// the entries hit-test to their index and the tab strip sits one host
/// row down.
#[test]
fn session_strip_paints_the_workspace_sessions_above_the_tabs() {
    let mut session = WindowSession::new(80, 25);
    apply_and_draw(&mut session, two_session_body());

    assert!(session.chrome.session_strip_on, "two sessions show it");
    assert_eq!(session.geometry.strip_rows, 2, "the row count moved");
    let row = strip_row(&session);
    assert!(row.contains("main"), "{row:?}");
    assert!(row.contains("review"), "{row:?}");
    assert!(!row.contains('+'), "no new-session button here: {row:?}");
    let cells = session.chrome.session_strip.layout().cells.clone();
    assert_eq!(cells.len(), 2);
    // The shown session carries the active block, the sibling does not.
    let shown = &session.chrome.session_strip.buffer()[(cells[0].col + 2, 0)];
    let other = &session.chrome.session_strip.buffer()[(cells[1].col + 2, 0)];
    assert_eq!(shown.symbol(), "m", "the first entry's text at its pad");
    assert_ne!(shown.bg, other.bg, "the shown session is highlighted");
    // Hit-testing: each entry's block maps to its index, gaps hit nothing.
    assert_eq!(session.chrome.session_strip.hit_test(cells[0].col), Some(0));
    assert_eq!(session.chrome.session_strip.hit_test(cells[1].col), Some(1));
    assert_eq!(
        session
            .chrome
            .session_strip
            .hit_test(cells[1].col + cells[1].width + 1),
        None,
        "the gap hits nothing"
    );

    // The tab strip row still paints the window below the strip.
    assert!(tab_row(&session).contains("dev"), "{}", tab_row(&session));
}

/// `auto` (the default) hides the strip at exactly one workspace session
/// and hands the row back; `always` pins it on; `off` pins it off.
#[test]
fn session_strip_auto_hides_at_one_session() {
    let mut session = WindowSession::new(80, 25);
    apply_and_draw(&mut session, two_session_body());
    assert!(session.chrome.session_strip_on);
    assert!(
        session.pending.grid_refit,
        "the flip parks a refit (the content height moved)"
    );
    session.pending.grid_refit = false; // the pump consumes it

    // Back to one session: the row is handed back and the refit re-parks.
    apply_and_draw(&mut session, one_session_body());
    assert!(!session.chrome.session_strip_on, "one session hides it");
    assert_eq!(session.geometry.strip_rows, 1, "the row is handed back");
    assert_eq!(session.geometry.session_strip_rows, 0);
    assert!(session.pending.grid_refit, "the flip back re-parks a refit");

    session.session_strip_mode = SessionStripMode::Always;
    session.draw_strips();
    assert!(session.chrome.session_strip_on, "always shows it at one");
    assert_eq!(session.geometry.strip_rows, 2);

    session.session_strip_mode = SessionStripMode::Off;
    apply_and_draw(&mut session, two_session_body());
    assert!(!session.chrome.session_strip_on, "off hides it at two");
}

/// With the strip visible a press on host row 0 hits the session strip
/// (raw host column) and a press on host row 1 hits the tab strip;
/// hidden, the top row is the tab strip's again.
#[test]
fn mouse_rows_route_between_the_strips() {
    let mut session = WindowSession::new(80, 25);
    apply_and_draw(&mut session, two_session_body());
    assert_eq!(
        session.mouse_hit(&press(5, 0)),
        Hit::SessionPress(5),
        "host row 0 is the session strip's"
    );
    assert_eq!(
        session.mouse_hit(&press(5, 1)),
        Hit::TabPress(5),
        "host row 1 is the tab strip's"
    );

    session.session_strip_mode = SessionStripMode::Off;
    session.draw_strips();
    assert_eq!(
        session.mouse_hit(&press(5, 0)),
        Hit::TabPress(5),
        "hidden, the top row is the tab strip's again"
    );
}

/// With `agent-state-dots` on, each strip entry and window tab carries
/// its scoped roster state's glyph, most-urgent wins per entry; off (the
/// default), the names stay bare.
#[test]
fn agent_dots_ride_the_strip_and_the_tabs() {
    let mut session = WindowSession::new(80, 25);
    let mut body = two_session_body();
    // A second, more urgent agent in the sibling session.
    body.push("agent %2 kimi blocked hook session=$2 win=@2".to_string());
    apply_and_draw(&mut session, body);
    let bare = strip_row(&session);
    assert!(
        !bare.contains('\u{25CF}') && !bare.contains('\u{25D0}'),
        "off by default: {bare:?}"
    );

    session.agent_state_dots = true;
    session.draw_strips();
    let row = strip_row(&session);
    // $1's agent is working, $2's is blocked — most urgent wins per
    // entry, scoped by the row's session= extra.
    let cells: Vec<char> = row.chars().collect();
    let laid = session.chrome.session_strip.layout().cells.clone();
    assert_eq!(cells[laid[0].col as usize + 2], '\u{25D0}', "{row:?}");
    assert_eq!(cells[laid[1].col as usize + 2], '\u{25CF}', "{row:?}");
    // The tab strip's window tab carries its own scoped dot.
    let tabs = tab_row(&session);
    let tab_cells: Vec<char> = tabs.chars().collect();
    let tab = &session.chrome.tab_strip.layout().cells[0];
    assert_eq!(tab_cells[tab.col as usize + 2], '\u{25D0}', "{tabs:?}");
}

/// The restart toast: the seed snapshot's restore age within the window
/// raises the one-line toast once; a stale age or an absent row (a plain
/// re-attach, a fresh daemon) raises nothing.
#[test]
fn restart_toast_fires_once_within_the_window() {
    let mut session = WindowSession::new(80, 25);
    session.status.restored_age = Some(5);
    session.maybe_restart_toast();
    assert_eq!(
        session.renderer.toast.as_ref().map(|t| t.title.as_str()),
        Some("server was restarted")
    );
    assert!(session.notify.toast_until.is_some());

    // Once: dismissed, a second call refires nothing.
    session.renderer.set_toast(None);
    session.notify.toast_until = None;
    session.maybe_restart_toast();
    assert!(session.renderer.toast.is_none(), "once per attach");

    // Stale news and no marker: nothing.
    let mut late = WindowSession::new(80, 25);
    late.status.restored_age = Some(10_000);
    late.maybe_restart_toast();
    assert!(late.renderer.toast.is_none(), "past the window");

    let mut fresh = WindowSession::new(80, 25);
    fresh.maybe_restart_toast();
    assert!(fresh.renderer.toast.is_none(), "no marker, no toast");
}
