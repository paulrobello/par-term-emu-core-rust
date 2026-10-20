//! Copy mode's `e` (edit scrollback in `$VISUAL`/`$EDITOR`, herdr's
//! `edit_scrollback`): editor resolution, the private files, the
//! refusal, the zoomed-pane launch, and the typed-key routing.

use super::super::scrollback_edit::resolve_editor;
#[cfg(unix)]
use super::super::scrollback_edit::write_scrollback_files;
use super::*;

fn no_editor() -> (Option<String>, Option<String>) {
    (None, Some("   ".to_string()))
}

#[cfg(unix)]
fn fake_editor() -> (Option<String>, Option<String>) {
    (Some("true".to_string()), None)
}

/// VISUAL wins over EDITOR; an empty or whitespace-only value counts as
/// unset; neither set refuses.
#[test]
fn resolve_editor_prefers_visual_and_skips_blank_values() {
    let s = |v: &str| Some(v.to_string());
    assert_eq!(resolve_editor(s("nvim"), s("vi")), s("nvim"));
    assert_eq!(resolve_editor(s(""), s("vi")), s("vi"));
    assert_eq!(resolve_editor(s("  "), None), None);
    assert_eq!(resolve_editor(None, s("code -w")), s("code -w"));
    assert_eq!(resolve_editor(None, None), None);
}

/// The text file holds exactly what it was given, both files are
/// owner-only, and the launcher runs the editor on the text then removes
/// both — the files of two edits never collide.
#[cfg(unix)]
#[test]
fn scrollback_files_are_private_and_the_launcher_cleans_up() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().expect("tempdir");
    let files = write_scrollback_files(dir.path(), "alpha\nbeta's\n", "nvim -R").expect("write");
    assert_eq!(
        std::fs::read_to_string(&files.text).expect("text"),
        "alpha\nbeta's\n"
    );
    for path in [&files.text, &files.launcher] {
        let mode = std::fs::metadata(path).expect("meta").permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "{} is owner-only", path.display());
    }
    let script = std::fs::read_to_string(&files.launcher).expect("launcher");
    assert!(
        script.starts_with(&format!("nvim -R '{}'", files.text.display())),
        "the editor runs on the quoted text path: {script}"
    );
    assert!(script.contains("rm -f"), "the launcher cleans up: {script}");
    let again = write_scrollback_files(dir.path(), "x", "vi").expect("second");
    assert_ne!(again.text, files.text, "a second edit gets fresh names");

    // Run the launcher with a no-op editor: the editor's status is the
    // script's, and both files are gone afterwards.
    let ran = write_scrollback_files(dir.path(), "x", "true").expect("third");
    let status = std::process::Command::new("/bin/sh")
        .arg(&ran.launcher)
        .status()
        .expect("sh");
    assert!(status.success());
    assert!(!ran.text.exists() && !ran.launcher.exists(), "both removed");
}

/// Without `$VISUAL`/`$EDITOR` the key is refused on the flash: copy
/// mode stays up and nothing is sent to the daemon.
#[test]
fn edit_scrollback_refuses_without_an_editor() {
    let (rx, mut conn, mut session) = two_pane_session("editnone", FakeScript::default());
    session.editor_lookup = no_editor;
    for i in 0..40 {
        session
            .renderer
            .feed_output(1, format!("l{i}\r\n").as_bytes());
    }
    session.renderer.focus(1);
    chord(&mut session, &mut conn, b'[');
    assert!(session.is_scroll());
    drained(&rx);
    session.route_plain(b"e", &mut conn);
    settle(&mut session, &mut conn);
    assert_eq!(
        session.chrome.flash_text(),
        Some("edit scrollback: set $VISUAL or $EDITOR")
    );
    assert!(session.is_scroll(), "the refusal keeps copy mode up");
    assert!(
        rx.recv_timeout(std::time::Duration::from_millis(150))
            .is_err(),
        "nothing reaches the daemon"
    );
}

/// Off unix the launcher needs a POSIX shell that is not there: even
/// with an editor set, `e` is refused on the flash, copy mode stays up,
/// and no pane is split.
#[cfg(not(unix))]
#[test]
fn edit_scrollback_refuses_without_a_posix_shell() {
    let (rx, mut conn, mut session) = two_pane_session("editnoposix", FakeScript::default());
    session.editor_lookup = || (Some("notepad".to_string()), None);
    for i in 0..40 {
        session
            .renderer
            .feed_output(1, format!("l{i}\r\n").as_bytes());
    }
    session.renderer.focus(1);
    chord(&mut session, &mut conn, b'[');
    assert!(session.is_scroll());
    drained(&rx);
    session.route_plain(b"e", &mut conn);
    settle(&mut session, &mut conn);
    assert_eq!(
        session.chrome.flash_text(),
        Some("edit scrollback: needs a POSIX shell (unix only)")
    );
    assert!(session.is_scroll(), "the refusal keeps copy mode up");
    assert!(
        rx.recv_timeout(std::time::Duration::from_millis(150))
            .is_err(),
        "nothing reaches the daemon"
    );
}

/// A typed `e` in copy mode writes the pane's grid export to a private
/// file, splits a pane off the focused one running `/bin/sh <launcher>`,
/// focuses and zooms it, and leaves copy mode. The launcher names the
/// text file, whose content is byte-identical to the export.
#[cfg(unix)]
#[test]
fn edit_scrollback_opens_a_zoomed_editor_pane_on_the_export() {
    let mut replies = std::collections::HashMap::new();
    replies.insert("split-window *".to_string(), "%9".to_string());
    let (rx, mut conn, mut session) = two_pane_session(
        "editopen",
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    );
    session.editor_lookup = fake_editor;
    // A soft-wrapped long line (90 chars over the 40-col pane) and
    // enough lines for real scrollback.
    session
        .renderer
        .feed_output(1, format!("{}\r\n", "w".repeat(90)).as_bytes());
    for i in 0..40 {
        session
            .renderer
            .feed_output(1, format!("line {i:02}\r\n").as_bytes());
    }
    let export = session
        .renderer
        .pane_terminal(1)
        .expect("pane")
        .active_grid()
        .export_text_buffer();
    assert!(
        export.contains(&"w".repeat(90)),
        "the export rejoins the wrapped line"
    );
    session.renderer.focus(1);
    chord(&mut session, &mut conn, b'[');
    assert!(session.is_scroll());
    drained(&rx);

    session.route_plain(b"e", &mut conn);
    assert!(!session.is_scroll(), "the key leaves copy mode");
    let split = wait_recorded(&rx, "split-window");
    let launcher = split
        .strip_prefix("split-window -t %1 -- /bin/sh '")
        .and_then(|rest| rest.strip_suffix('\''))
        .unwrap_or_else(|| panic!("the split runs the quoted launcher: {split}"));
    let script = std::fs::read_to_string(launcher).expect("launcher exists");
    let text_path = script
        .strip_prefix("true '")
        .and_then(|rest| rest.split('\'').next())
        .unwrap_or_else(|| panic!("the launcher runs the editor: {script}"));
    assert_eq!(
        std::fs::read_to_string(text_path).expect("text exists"),
        export,
        "the temp file is the grid's text export"
    );
    settle(&mut session, &mut conn);
    // The fake's reseed installs no layout with %9, so the renderer
    // cannot focus it here; the daemon-side select is the contract.
    let select = wait_recorded(&rx, "select-pane");
    assert_eq!(select, "select-pane -t %9", "the editor pane takes focus");
    let zoom = wait_recorded(&rx, "resize-pane");
    assert_eq!(zoom, "resize-pane -t %9 -Z", "the editor pane zooms");
    settle(&mut session, &mut conn);
    assert!(session.zoomed);
    // The launcher never ran here (no daemon pane): clean up its files.
    let _ = std::fs::remove_file(text_path);
    let _ = std::fs::remove_file(launcher);
}

/// A refused split removes the temp files and flashes the failure.
#[cfg(unix)]
#[test]
fn edit_scrollback_failed_split_removes_the_files() {
    let mut failing = std::collections::HashSet::new();
    failing.insert("split-window *".to_string());
    let (rx, mut conn, mut session) = two_pane_session(
        "editfail",
        FakeScript {
            failing,
            ..FakeScript::default()
        },
    );
    session.editor_lookup = fake_editor;
    for i in 0..40 {
        session
            .renderer
            .feed_output(1, format!("l{i}\r\n").as_bytes());
    }
    session.renderer.focus(1);
    chord(&mut session, &mut conn, b'[');
    drained(&rx);
    session.route_plain(b"e", &mut conn);
    let split = wait_recorded(&rx, "split-window");
    let launcher = split
        .strip_prefix("split-window -t %1 -- /bin/sh '")
        .and_then(|rest| rest.strip_suffix('\''))
        .expect("quoted launcher")
        .to_string();
    let script = std::fs::read_to_string(&launcher).expect("launcher written before the reply");
    let text_path = script
        .strip_prefix("true '")
        .and_then(|rest| rest.split('\'').next())
        .expect("text path")
        .to_string();
    settle(&mut session, &mut conn);
    assert!(
        !std::path::Path::new(&launcher).exists() && !std::path::Path::new(&text_path).exists(),
        "the refused split removed both files"
    );
    assert!(
        session
            .chrome
            .flash_text()
            .is_some_and(|f| f.starts_with("edit scrollback failed")),
        "the failure flashes: {:?}",
        session.chrome.flash_text()
    );
}

/// The copy-mode letters typed as plain bytes (how a real tty delivers
/// them) reach the copy-mode handler: `v` anchors, `y` copies, `/`
/// opens the search line and the rest of the run is its query.
#[test]
fn copy_mode_letters_route_from_typed_bytes() {
    let (rx, mut conn, mut session) = two_pane_session("copybytes", FakeScript::default());
    for i in 0..60 {
        session
            .renderer
            .feed_output(1, format!("line {i:03} marker\r\n").as_bytes());
    }
    session.renderer.focus(1);
    chord(&mut session, &mut conn, b'[');
    assert!(session.is_scroll());
    session.route_plain(b"v", &mut conn);
    assert!(session.renderer.selection.is_some(), "typed v anchors");
    session.route_plain(b"y", &mut conn);
    assert!(session.renderer.selection.is_none(), "typed y copies");
    assert!(
        session.notify.outbox.windows(7).any(|w| w == b"\x1b]52;c;"),
        "the copy rode the outbox"
    );
    session.route_plain(b"/line 042\r", &mut conn);
    assert!(session.scroll_query.is_none(), "Enter committed the query");
    assert!(!session.scroll_matches.is_empty(), "the query matched");
    assert!(session.is_scroll(), "still in copy mode");
    assert!(drained(&rx).is_empty(), "no typed byte reached the pane");
}

/// The keybinds panel lists the copy-mode editor key.
#[test]
fn help_rows_list_the_scrollback_editor_key() {
    let rows = crate::mux::attach::help_rows(0x02, 0x12, Default::default(), 1);
    assert!(
        rows.iter()
            .any(|r| r.text.contains("C-b [ then e") && r.text.contains("$EDITOR")),
        "the editor key is in the help"
    );
}
