//! Popup command windows: the open/close key routing, the wire
//! sequence, the fake-stream mirror and its paint, input capture, the
//! exit close, and the config reload.

// The fake-stream helpers serve the unix-only tests (the launcher path
// is unix-only), so they are compiled there alone.
#[cfg(unix)]
use super::super::popup::{PopupControl, PopupLink};
use super::*;
#[cfg(unix)]
use crate::mux::pane_stream::StreamEvent;
#[cfg(unix)]
use std::cell::RefCell;
#[cfg(unix)]
use std::sync::mpsc::Sender;

/// The size reports a fake stream recorded.
#[cfg(unix)]
type Sizes = std::rc::Rc<RefCell<Vec<(u16, u16)>>>;

#[cfg(unix)]
thread_local! {
    /// The fake stream's event sender and the size reports it got.
    static FAKE: RefCell<Option<(Sender<StreamEvent>, Sizes)>> = const { RefCell::new(None) };
}

#[cfg(unix)]
struct FakeControl(Sizes);

#[cfg(unix)]
impl PopupControl for FakeControl {
    fn resize(&mut self, cols: u16, rows: u16) -> std::io::Result<()> {
        self.0.borrow_mut().push((cols, rows));
        Ok(())
    }
}

/// The test opener: a channel the test feeds, size reports recorded.
#[cfg(unix)]
fn fake_opener(_socket: Option<&std::path::Path>, _pane: &str) -> std::io::Result<PopupLink> {
    let (tx, events) = std::sync::mpsc::channel();
    let sizes = std::rc::Rc::new(RefCell::new(Vec::new()));
    FAKE.with(|f| *f.borrow_mut() = Some((tx, sizes.clone())));
    Ok(PopupLink {
        events,
        control: Box::new(FakeControl(sizes)),
    })
}

#[cfg(unix)]
fn fake_send(event: StreamEvent) {
    FAKE.with(|f| {
        f.borrow()
            .as_ref()
            .expect("stream opened")
            .0
            .send(event)
            .expect("send");
    });
}

#[cfg(unix)]
fn fake_sizes() -> Vec<(u16, u16)> {
    FAKE.with(|f| {
        f.borrow()
            .as_ref()
            .expect("stream opened")
            .1
            .borrow()
            .clone()
    })
}

/// A two-pane session with one popup on `t` (60% x 50%), the fake
/// daemon answering the popup's `new-window` and `list-panes`.
#[cfg(unix)]
fn popup_session(
    tag: &str,
) -> (
    std::sync::mpsc::Receiver<(String, String)>,
    crate::mux::attach::conn::AttachConn,
    WindowSession,
) {
    let mut replies = std::collections::HashMap::new();
    replies.insert("new-window *".to_string(), "@7".to_string());
    replies.insert("list-panes -t @7".to_string(), "%9 0 *".to_string());
    let (rx, conn, mut session) = two_pane_session(
        tag,
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    );
    session.popups = crate::mux::popup::validate(&[crate::mux::popup::PopupEntry {
        key: "t".to_string(),
        command: "lazygit".to_string(),
        width: Some(crate::mux::popup::PopupSize::Percent(60).into()),
        height: Some(crate::mux::popup::PopupSize::Percent(50).into()),
        description: Some("git".to_string()),
    }])
    .expect("valid");
    session.popup_opener = fake_opener;
    (rx, conn, session)
}

/// prefix + the popup's key opens it: a `new-window` beside the shown
/// window running the quoted launcher (which holds the command), then
/// the pane lookup and the stream; the seed builds the mirror and the
/// size report is the box's inner grid.
#[cfg(unix)]
#[test]
fn popup_key_opens_a_scratch_window_and_mirrors_its_stream() {
    let (rx, mut conn, mut session) = popup_session("popupopen");
    chord(&mut session, &mut conn, b't');
    assert!(session.popup_active(), "the popup owns the input");
    let new_window = wait_recorded(&rx, "new-window");
    // The command runs inline now (the start-command cut stops the flag
    // scan at the `--`), so the line carries it verbatim.
    assert_eq!(new_window, "new-window -t @0 -n popup -- lazygit");
    settle(&mut session, &mut conn);
    assert_eq!(wait_recorded(&rx, "list-panes"), "list-panes -t @7");
    settle(&mut session, &mut conn);
    assert_eq!(session.popup_window(), Some("@7"));

    // The box: 60% x 50% of the 80x23 pane area, centered.
    let outer = session
        .renderer
        .popup
        .as_ref()
        .map(|p| p.outer)
        .expect("box");
    assert_eq!((outer.width, outer.height), (48, 11));
    assert_eq!((outer.x, outer.y), (16, 6));
    fake_send(StreamEvent::Seed {
        pane: "%9".to_string(),
        size: (80, 24),
        exited: None,
        seed: b"HELLO-POPUP".to_vec(),
    });
    session.drain_popup(&mut conn);
    assert_eq!(fake_sizes(), vec![(46, 9)], "the inner grid is reported");
    fake_send(StreamEvent::Output(b" more".to_vec()));
    session.drain_popup(&mut conn);
    session.renderer.render_frame();
    let row: String = (outer.x + 1..outer.x + 20)
        .map(|x| {
            session
                .renderer
                .cell(x, outer.y + 1)
                .expect("cell")
                .symbol()
                .to_string()
        })
        .collect();
    assert!(
        row.starts_with("HELLO-POPUP more"),
        "the box paints the mirror: {row:?}"
    );
    let title: String = (outer.x..outer.x + 10)
        .map(|x| {
            session
                .renderer
                .cell(x, outer.y)
                .expect("cell")
                .symbol()
                .to_string()
        })
        .collect();
    assert!(
        title.contains("git"),
        "the border carries the title: {title:?}"
    );
}

/// While the popup is up, plain bytes, keys, and pastes go to the
/// popup's pane, never the focused tiled pane; other prefix chords are
/// consumed; prefix prefix sends the literal; prefix + the key closes
/// it and kills the scratch window.
#[cfg(unix)]
#[test]
fn popup_captures_input_and_its_key_closes_it() {
    let (rx, mut conn, mut session) = popup_session("popupkeys");
    chord(&mut session, &mut conn, b't');
    settle(&mut session, &mut conn);
    settle(&mut session, &mut conn);
    drained(&rx);
    session.route_stdin_bytes(&mut conn, b"ab");
    settle(&mut session, &mut conn);
    assert_eq!(
        wait_recorded(&rx, "send-keys"),
        "send-keys -t %9 -H 61 62",
        "typing reaches the popup pane"
    );
    // A management chord (split) is consumed, not run.
    session.route_stdin_bytes(&mut conn, &[crate::mux::attach::C_B, b'%']);
    settle(&mut session, &mut conn);
    assert!(
        !drained(&rx).iter().any(|l| l.starts_with("split-window")),
        "chords are consumed while the popup is up"
    );
    // prefix prefix: the literal reaches the popup.
    session.route_stdin_bytes(
        &mut conn,
        &[crate::mux::attach::C_B, crate::mux::attach::C_B],
    );
    settle(&mut session, &mut conn);
    assert_eq!(wait_recorded(&rx, "send-keys"), "send-keys -t %9 -H 02");
    // An arrow re-encodes against the POPUP's input modes: its seed
    // turned DECCKM on, so Up is SS3 A (the tiled pane would send CSI).
    fake_send(StreamEvent::Seed {
        pane: "%9".to_string(),
        size: (46, 9),
        exited: None,
        seed: b"\x1b[?1h".to_vec(),
    });
    session.drain_popup(&mut conn);
    session.route_stdin_bytes(&mut conn, b"\x1b[A");
    settle(&mut session, &mut conn);
    assert_eq!(
        wait_recorded(&rx, "send-keys"),
        "send-keys -t %9 -H 1b 4f 41"
    );
    // prefix t closes it.
    session.route_stdin_bytes(&mut conn, &[crate::mux::attach::C_B, b't']);
    settle(&mut session, &mut conn);
    assert!(!session.popup_active(), "the key closes the popup");
    assert!(session.renderer.popup.is_none(), "the box is cleared");
    assert_eq!(wait_recorded(&rx, "kill-window"), "kill-window -t @7");
    // Typing reaches the tiled pane again.
    session.renderer.focus(1);
    session.route_stdin_bytes(&mut conn, b"z");
    settle(&mut session, &mut conn);
    assert_eq!(wait_recorded(&rx, "send-keys"), "send-keys -t %1 -H 7a");
}

/// The command's exit (or the stream ending) closes the popup and
/// kills the scratch window; prefix d with a popup up closes it and
/// detaches.
#[cfg(unix)]
#[test]
fn popup_closes_when_its_command_exits_and_on_detach() {
    let (rx, mut conn, mut session) = popup_session("popupexit");
    chord(&mut session, &mut conn, b't');
    settle(&mut session, &mut conn);
    settle(&mut session, &mut conn);
    fake_send(StreamEvent::Seed {
        pane: "%9".to_string(),
        size: (46, 9),
        exited: None,
        seed: Vec::new(),
    });
    fake_send(StreamEvent::Exited(Some(0)));
    session.drain_popup(&mut conn);
    assert!(!session.popup_active(), "the exit closes the popup");
    assert_eq!(wait_recorded(&rx, "kill-window"), "kill-window -t @7");

    chord(&mut session, &mut conn, b't');
    settle(&mut session, &mut conn);
    settle(&mut session, &mut conn);
    assert!(session.popup_active());
    let detached = session.route_stdin_bytes(&mut conn, &[crate::mux::attach::C_B, b'd']);
    assert!(detached, "prefix d still detaches");
    assert!(!session.popup_active(), "and closes the popup first");
}

/// A key no popup claims stays unbound; a pane area under the 6x4
/// minimum refuses with a flash; a refused `new-window` drops the
/// popup with the reason.
#[cfg(unix)]
#[test]
fn popup_refusals_flash_and_leave_the_panes_in_charge() {
    let (rx, mut conn, mut session) = popup_session("popuprefuse");
    chord(&mut session, &mut conn, b'q');
    assert!(!session.popup_active(), "an unclaimed key opens nothing");
    session.refit_renderer(5, 3);
    chord(&mut session, &mut conn, b't');
    assert!(!session.popup_active());
    assert_eq!(
        session.chrome.flash_text(),
        Some("popup: the pane area is too small")
    );
    assert!(
        !drained(&rx).iter().any(|l| l.starts_with("new-window")),
        "nothing reaches the daemon"
    );

    let mut failing = std::collections::HashSet::new();
    failing.insert("new-window *".to_string());
    let mut replies = std::collections::HashMap::new();
    replies.insert("new-window *".to_string(), "no such session".to_string());
    let (_rx, mut conn, mut session) = two_pane_session(
        "popupfail",
        FakeScript {
            replies,
            failing,
            ..FakeScript::default()
        },
    );
    session.popups = crate::mux::popup::validate(&[crate::mux::popup::PopupEntry {
        key: "t".to_string(),
        command: "true".to_string(),
        ..Default::default()
    }])
    .expect("valid");
    chord(&mut session, &mut conn, b't');
    settle(&mut session, &mut conn);
    assert!(!session.popup_active(), "a refused window drops the popup");
    assert_eq!(
        session.chrome.flash_text(),
        Some("popup failed: no such session")
    );
}

/// The settings dialog's general tab lists every popup plus an add row
/// (the project rule: every TUI `[client]` key is editable there). The
/// add row's prompt takes `KEY [WxH] COMMAND` and writes a
/// `[[client.popup]]` table that applies live; editing a row rewrites
/// it; a bad spelling or a duplicate key flashes and writes nothing; an
/// empty commit deletes the popup.
#[test]
fn settings_popup_rows_add_edit_and_delete() {
    let (rx, mut conn, mut session) = command_menu_session("popupsettings");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[client]\nresize-step = 2\n").unwrap();
    session.settings_path = Some(path.clone());
    session.enter_settings();
    // The general tab (two Tabs): its 9 fixed rows (the tab-width rows
    // included), then the add row — reach it before asserting, the
    // panel scrolls.
    session.route_stdin_bytes(&mut conn, b"\t\t");
    session.route_stdin_bytes(&mut conn, b"jjjjjjjjj");
    let rows = overlay_text(&session);
    assert!(
        rows.iter()
            .any(|r| r.contains("+ add popup") && r.contains("KEY [WxH] COMMAND")),
        "the add row is listed: {rows:#?}"
    );
    session.route_stdin_bytes(&mut conn, b"\r");
    assert!(session.is_prompt(), "the add row opens the prompt");
    type_and_commit(&mut session, &mut conn, "t 80%x20 lazygit -p .");
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        written.contains("resize-step = 2"),
        "other keys survive: {written}"
    );
    assert!(written.contains("[[client.popup]]"), "{written}");
    assert!(written.contains("key = \"t\""), "{written}");
    assert!(written.contains("command = \"lazygit -p .\""), "{written}");
    assert!(written.contains("width = \"80%\""), "{written}");
    assert!(written.contains("height = 20"), "{written}");
    assert_eq!(wait_recorded(&rx, "reload-config"), "reload-config");
    assert_eq!(session.popups.len(), 1, "applied live");
    assert_eq!(session.popups[0].key, b't');
    let rows = overlay_text(&session);
    assert!(
        rows.iter()
            .any(|r| r.contains("popup") && r.contains("t 80%x20 lazygit -p .")),
        "the new popup is a row: {rows:#?}"
    );

    // A bad spelling flashes and keeps the prompt; a clash with the
    // existing popup's key is refused too.
    session.route_stdin_bytes(&mut conn, b"j\r");
    type_and_commit(&mut session, &mut conn, "t htop");
    assert!(session.is_prompt(), "a duplicate key keeps the prompt up");
    assert!(
        session
            .chrome
            .flash_text()
            .is_some_and(|f| f.contains("earlier popup")),
        "{:?}",
        session.chrome.flash_text()
    );
    session.leave_prompt();

    // Edit the popup row (index 6): its size and command change.
    session.route_stdin_bytes(&mut conn, b"k\r");
    assert_eq!(session.prompt_mut().text, "t 80%x20 lazygit -p .");
    type_and_commit(&mut session, &mut conn, "t htop");
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("command = \"htop\""), "{written}");
    assert!(!written.contains("width"), "{written}");
    assert_eq!(session.popups[0].entry.command, "htop");

    // An empty commit deletes it: the key is gone from the file.
    session.route_stdin_bytes(&mut conn, b"\r");
    type_and_commit(&mut session, &mut conn, "");
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("popup = []"), "{written}");
    assert!(session.popups.is_empty(), "deleted live");
}

/// The config's `[[client.popup]]` tables reach the session on reload,
/// and the tab strip leaves an open popup's scratch window out.
#[test]
fn popup_tables_apply_on_reload() {
    let (_rx, _conn, mut session) = two_pane_session("popupreload", FakeScript::default());
    let file: crate::mux::config::ConfigFile =
        toml::from_str("[client]\n[[client.popup]]\nkey = \"t\"\ncommand = \"htop\"\nwidth = 30\n")
            .expect("parses");
    let chords = crate::mux::config::reload_client_chords(
        &file,
        &crate::mux::config::Chords::with_defaults(),
    )
    .expect("chords");
    session.apply_chords(&chords, &file);
    assert_eq!(session.popups.len(), 1);
    assert_eq!(session.popups[0].key, b't');
    assert!(session.popup_for_key(b't').is_some());
}
