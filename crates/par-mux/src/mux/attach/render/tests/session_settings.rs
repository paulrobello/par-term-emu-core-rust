//! WindowSession suites: the settings dialog.

use super::*;

/// The command menu's `settings` row opens the settings dialog: herdr's
/// tabbed panel. Tab switches sections, `j` moves the selection, Enter
/// writes the chosen key into the config file — keeping the user's
/// comments — and reloads (the daemon's `reload-config` rides the wire),
/// a click on a tab switches to it, and Escape closes.
#[test]
fn command_menu_settings_opens_the_dialog_and_applies() {
    use super::super::modal_settings::SettingsSection;
    let (rx, mut conn, mut session) = command_menu_session("cmdmenu-settings");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "# keep me\n[client]\nprefix = \"C-b\"\n").unwrap();
    session.settings_path = Some(path.clone());
    click_menu_row(&mut session, &mut conn, 0);
    let Modal::Settings(state) = &session.modal else {
        panic!("settings opened: {:?}", session.modal);
    };
    assert_eq!(state.section, SettingsSection::Sidebar);
    assert_eq!(
        session.renderer.overlay.as_ref().map(|o| o.0),
        Some(super::super::modal_settings::SETTINGS_TITLE)
    );
    // Tab three times: the notify section (sidebar, borders, general
    // first), its single row showing the test session's delivery.
    session.route_stdin_bytes(&mut conn, b"\t\t\t");
    let Modal::Settings(state) = &session.modal else {
        panic!("still open");
    };
    assert_eq!(state.section, SettingsSection::Notify);
    assert_eq!(state.selected, 0);
    let row = session.renderer.overlay.as_ref().expect("up").1[5]
        .text
        .trim_end()
        .to_string();
    assert!(row.starts_with(" \u{25B8} delivery"), "{row}");
    assert!(row.ends_with("off"), "{row}");
    // Enter: cycles to the next delivery (`inside`), written and reloaded.
    session.route_stdin_bytes(&mut conn, b"\r");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.starts_with("# keep me\n"), "{written}");
    assert!(
        written.contains("notify-delivery = \"inside\""),
        "{written}"
    );
    assert_eq!(wait_recorded(&rx, "reload-config"), "reload-config");
    assert_eq!(
        session.notify.delivery.name(),
        "inside",
        "the reload applied the written key"
    );
    // A click on the `borders` tab (inner columns 9..18 of row 0).
    let (x0, y0, _, _) = session.renderer.overlay_geometry().expect("dialog up");
    session.route_mouse(
        &mut conn,
        sgr(0, (x0 + 2 + 10) as u16, (y0 + 3) as u16, false),
    );
    let Modal::Settings(state) = &session.modal else {
        panic!("still open");
    };
    assert_eq!(state.section, SettingsSection::Borders);
    // A press on the box's ring stays inside; a press outside it closes
    // the dialog (herdr's click-away).
    let (x0, y0, _, _) = session.renderer.overlay_geometry().expect("dialog up");
    session.route_mouse(&mut conn, sgr(0, (x0 + 1) as u16, (y0 + 2) as u16, false));
    assert!(
        matches!(session.modal, Modal::Settings(_)),
        "the ring is inside"
    );
    session.route_mouse(&mut conn, sgr(0, 1, (y0 + 3) as u16, false));
    assert!(
        matches!(session.modal, Modal::None),
        "a click outside closes"
    );
    assert!(session.renderer.overlay.is_none());
    // Escape closes too.
    click_menu_row_after_reopen(&mut session, &mut conn);
    session.route_stdin_bytes(&mut conn, b"q");
    assert!(matches!(session.modal, Modal::None));
    assert!(session.renderer.overlay.is_none());
}

/// While the settings dialog is up, wheels move the selection with the
/// arrows' scroll-into-view: walking past the body window's bottom
/// follows the window (`top`), walking back returns it, and no wheel
/// ever closes the dialog.
#[test]
fn settings_pointer_wheel_moves_selection_with_scroll_into_view() {
    use super::super::modal_settings::SettingsSection;
    let (_rx, mut conn, mut session) = command_menu_session("cmdmenu-wheel");
    session.enter_settings();
    // The borders tab, one Tab from the opening section.
    session.route_stdin_bytes(&mut conn, b"\t");
    let Modal::Settings(state) = &session.modal else {
        panic!("dialog open");
    };
    assert_eq!(state.section, SettingsSection::Borders);
    let count = state.section.row_count() + state.popup_rows();
    assert!(
        count > 8,
        "the borders section must overflow the 8-row body window"
    );
    // Wheel down to the last row: the selection walks and the body's
    // window follows it (the arrows' scroll-into-view).
    for _ in 0..(count - 1) {
        session.route_mouse(&mut conn, sgr(65, 40, 12, false));
    }
    let Modal::Settings(state) = &session.modal else {
        panic!("dialog open");
    };
    assert_eq!(state.selected, count - 1, "clamped at the last row");
    assert_eq!(state.top, count - 8, "the window followed the selection");
    // Wheel up all the way: back to the first row, window back at the
    // top.
    for _ in 0..(count - 1) {
        session.route_mouse(&mut conn, sgr(64, 40, 12, false));
    }
    let Modal::Settings(state) = &session.modal else {
        panic!("dialog open");
    };
    assert_eq!(state.selected, 0);
    assert_eq!(state.top, 0);
    assert!(
        matches!(session.modal, Modal::Settings(_)),
        "wheels never close the dialog"
    );
}

/// The row sections: a toggle writes its bool and the reload applies it
/// live, and a choice row cycles (the borders tab's `line style` and the
/// sidebar tab's `agent lines` preset).
#[test]
fn settings_rows_write_and_apply_live() {
    let (rx, mut conn, mut session) = command_menu_session("cmdmenu-rows");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[client]\n").unwrap();
    session.settings_path = Some(path.clone());
    session.enter_settings();
    // The borders tab (one Tab), `pane boxes` (row 1): Enter flips it on.
    session.route_stdin_bytes(&mut conn, b"\tj\r");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("pane-borders = true"), "{written}");
    assert_eq!(wait_recorded(&rx, "reload-config"), "reload-config");
    assert!(session.render_opts.pane_borders, "applied live");
    // Enter again flips it back off.
    session.route_stdin_bytes(&mut conn, b"\r");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("pane-borders = false"), "{written}");
    assert!(!session.render_opts.pane_borders);
    // The sidebar tab (one back-Tab), `agent lines` (row 3, under the
    // git poll row): cycles default → compact and writes the nested
    // token arrays.
    session.route_stdin_bytes(&mut conn, b"hjjj\r");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        written.contains("agent-rows = [[\"state_icon\", \"agent\"]]"),
        "{written}"
    );
    assert_eq!(
        session.agent_rows,
        vec![vec!["state_icon".to_string(), "agent".to_string()],],
        "the reload rebuilt the agent section"
    );
}

/// The borders tab's theme rows write and apply live: Enter on `theme`
/// cycles default → auto, the reload re-picks (no probe = the dark
/// preset), and `→` on `auto: dark host` steps to catppuccin, which the
/// auto pick then paints.
#[test]
fn settings_theme_rows_write_and_apply_live() {
    let (_rx, mut conn, mut session) = command_menu_session("cmdmenu-theme");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[client]\n").unwrap();
    session.settings_path = Some(path.clone());
    session.enter_settings();
    // The borders tab, `theme` (row 6).
    session.route_stdin_bytes(&mut conn, b"\tjjjjjj\r");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("theme = \"auto\""), "{written}");
    assert_eq!(session.theme.theme, "auto", "applied live");
    assert!(session.renderer.palette().is_none(), "dark default");
    // `auto: dark host` (row 7): → steps default → catppuccin.
    session.route_stdin_bytes(&mut conn, b"j\x1b[C");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("theme-dark = \"catppuccin\""), "{written}");
    assert_eq!(
        session.renderer.palette(),
        crate::mux::theme::palette("catppuccin")
    );
}

/// The notify tab's sound rows write and apply live: `sounds` flips on,
/// `muted agents` takes a comma list from the prompt, `done sound` a
/// path, and an empty prompt clears the list back to none.
#[test]
fn settings_sound_rows_write_and_apply_live() {
    let (_rx, mut conn, mut session) = command_menu_session("cmdmenu-sound");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[client]\n").unwrap();
    session.settings_path = Some(path.clone());
    session.enter_settings();
    // The notify tab (three Tabs), `sounds` (row 1): Enter flips it on.
    session.route_stdin_bytes(&mut conn, b"\t\t\tj\r");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("sound = true"), "{written}");
    assert!(session.notify.sound.enabled, "applied live");
    // `muted agents` (row 4): the prompt takes a comma list.
    session.route_stdin_bytes(&mut conn, b"jjj\r");
    assert!(
        matches!(session.modal, Modal::Prompt(_)),
        "{:?}",
        session.modal
    );
    session.route_stdin_bytes(&mut conn, b"droid, codex\r");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        written.contains("sound-mute = [\"droid\", \"codex\"]"),
        "{written}"
    );
    assert_eq!(session.notify.sound.mute, vec!["droid", "codex"]);
    assert!(matches!(session.modal, Modal::Settings(_)), "dialog back");
    // `done sound` (row 2): a path.
    session.route_stdin_bytes(&mut conn, b"kk\r");
    session.route_stdin_bytes(&mut conn, b"sounds/done.mp3\r");
    settle(&mut session, &mut conn);
    assert_eq!(session.notify.sound.done, "sounds/done.mp3");
    // `muted agents` again: clear the seeded list, commit empty.
    session.route_stdin_bytes(&mut conn, b"jj\r");
    session.route_stdin_bytes(&mut conn, b"\x03\r");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("sound-mute = []"), "{written}");
    assert!(session.notify.sound.mute.is_empty());
}

/// `%config-changed` (another client's reload): the pump re-applies the
/// file's `[client]` settings through the reload chord's rebind, flashes
/// once, and NEVER sends `reload-config` (that would re-broadcast to
/// every client). A second cue over the same file is a no-op — the
/// shape the writing client sees when its own reload's broadcast lands.
#[test]
fn config_changed_reapplies_the_file_without_resending_reload() {
    use super::super::notify::NotifyDelivery;
    use par_term_emu_core::tmux_control::TmuxNotification;
    let (rx, mut conn, mut session) = command_menu_session("cfg-changed");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[client]\nnotify-delivery = \"off\"\n").unwrap();
    session.settings_path = Some(path.clone());
    session.reload_config(&mut conn);
    assert_eq!(wait_recorded(&rx, "reload-config"), "reload-config");
    assert_eq!(session.notify.delivery, NotifyDelivery::Off);
    session.chrome = ChromeRows::new(80);

    // Another client writes `terminal` + `pane-borders` and reloads.
    std::fs::write(
        &path,
        "[client]\nnotify-delivery = \"terminal\"\npane-borders = true\nsidebar-width = 34\n",
    )
    .unwrap();
    session.handle_event(TmuxNotification::ConfigChanged);
    assert!(session.pending.config_changed, "the cue parks for the pump");
    session
        .apply_pending(&mut conn, &mut RecordingSink::default())
        .expect("apply pending");
    settle(&mut session, &mut conn);
    assert_eq!(session.notify.delivery, NotifyDelivery::Terminal);
    assert_eq!(session.render_opts.sidebar_width, 34, "width applies live");
    assert!(
        session.render_opts.pane_borders,
        "a chrome key applied live"
    );
    assert_eq!(session.chrome.flash_text(), Some("config re-applied"));
    assert!(
        !drained(&rx).iter().any(|l| l == "reload-config"),
        "the cue never re-sends reload-config"
    );

    // The same file again (the writer's own broadcast): nothing moves,
    // nothing flashes, nothing is sent.
    session.chrome = ChromeRows::new(80);
    session.handle_event(TmuxNotification::ConfigChanged);
    session
        .apply_pending(&mut conn, &mut RecordingSink::default())
        .expect("apply pending");
    settle(&mut session, &mut conn);
    assert_eq!(
        session.chrome.flash_text(),
        None,
        "an unchanged file is quiet"
    );
    let sent = drained(&rx);
    assert!(sent.is_empty(), "an unchanged file sends nothing: {sent:?}");
}

/// ←/→ change a discrete row's value in place (the natural gesture on
/// the notify tab's `delivery` — the live report read the dialog as
/// unchangeable): right cycles forward, left back, both write and apply
/// live; on a prompt-edited row the arrows still switch sections.
#[test]
fn settings_choice_rows_cycle_with_the_arrows() {
    let (_rx, mut conn, mut session) = command_menu_session("setnotify-arrow");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[client]\n").unwrap();
    session.settings_path = Some(path.clone());
    session.enter_settings();
    // The notify tab (three Tabs), `delivery` at the default terminal:
    // → system, then ← off.
    session.route_stdin_bytes(&mut conn, b"\t\t\t");
    // A bare unit session's delivery is the struct default (off); →
    // cycles to inside, ← wraps back to system.
    session.route_key(&mut conn, &TermKeyEvent::functional(TermKey::Right, 0));
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        written.contains("notify-delivery = \"inside\""),
        "right wrote the next choice: {written}"
    );
    session.route_key(&mut conn, &TermKeyEvent::functional(TermKey::Left, 0));
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        written.contains("notify-delivery = \"off\""),
        "left wrote the previous choice: {written}"
    );
    assert_eq!(session.notify.delivery.name(), "off", "applied live");
    // On the general tab's `resize step` (a Number row) ← keeps
    // switching sections instead of opening the prompt.
    session.route_stdin_bytes(&mut conn, b"h");
    session.route_stdin_bytes(&mut conn, b"jjjj");
    session.route_key(&mut conn, &TermKeyEvent::functional(TermKey::Right, 0));
    settle(&mut session, &mut conn);
    let Modal::Settings(state) = &session.modal else {
        panic!("no prompt opened: {:?}", session.modal)
    };
    assert!(
        !std::fs::read_to_string(&path)
            .unwrap()
            .contains("resize-step"),
        "the arrow switched sections, it did not edit"
    );
    assert!(
        matches!(
            state.section,
            super::super::modal_settings::SettingsSection::Notify
        ),
        "the section moved: {:?}",
        state.section
    );
}

/// The prompt-edited rows: numbers, hex colors, and chords open the
/// prompt seeded with the value in force; a bad value flashes and writes
/// nothing; a good one writes, reloads, applies live, and returns to the
/// dialog (Escape cancels back to the dialog too).
#[test]
fn settings_prompt_rows_edit_numbers_hex_and_chords() {
    let (rx, mut conn, mut session) = command_menu_session("cmdmenu-prompt");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[client]\n").unwrap();
    session.settings_path = Some(path.clone());
    session.enter_settings();
    // General tab (two Tabs), `resize step` (row 4): Enter opens the
    // prompt seeded with 1.
    session.route_stdin_bytes(&mut conn, b"\t\tjjjj\r");
    let Modal::Prompt(prompt) = &session.modal else {
        panic!("prompt opened: {:?}", session.modal)
    };
    assert_eq!(prompt.text, "1");
    // A bad number flashes and keeps the prompt up (nothing written).
    session.route_stdin_bytes(&mut conn, b"\x03zzz\r");
    assert!(
        matches!(session.modal, Modal::Prompt(_)),
        "invalid value keeps the prompt: {:?}",
        session.modal
    );
    assert!(!std::fs::read_to_string(&path)
        .unwrap()
        .contains("resize-step"));
    // Clear, type 3, commit: written, reloaded, applied, dialog back.
    session.route_stdin_bytes(&mut conn, b"\x033\r");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("resize-step = 3"), "{written}");
    assert_eq!(wait_recorded(&rx, "reload-config"), "reload-config");
    assert_eq!(session.resize_step, 3, "applied live");
    assert!(
        matches!(session.modal, Modal::Settings(_)),
        "the dialog is back"
    );
    // The borders tab (one back-Tab from general), `focus color` (row
    // 4): the empty seed means the built-in; a hex commits and applies.
    session.route_stdin_bytes(&mut conn, b"hjjjj\r");
    let Modal::Prompt(prompt) = &session.modal else {
        panic!("hex prompt opened: {:?}", session.modal)
    };
    assert_eq!(prompt.text, "", "an unset hex seeds empty");
    session.route_stdin_bytes(&mut conn, b"#ff0000\r");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        written.contains("border-active-color = \"#ff0000\""),
        "{written}"
    );
    assert_eq!(
        session.render_opts.border_active,
        Some(ratatui::style::Color::Rgb(255, 0, 0)),
        "applied live"
    );
    // The keys tab (four more Tabs from borders), `prefix` (row 0): a
    // chord that does not parse is refused; a good one rebinds.
    session.route_stdin_bytes(&mut conn, b"\t\t\t\t\r");
    let Modal::Prompt(prompt) = &session.modal else {
        panic!("chord prompt opened: {:?}", session.modal)
    };
    assert_eq!(prompt.text, "C-b");
    session.route_stdin_bytes(&mut conn, b"\x03zz\r");
    assert!(matches!(session.modal, Modal::Prompt(_)), "refused");
    session.route_stdin_bytes(&mut conn, b"\x03C-a\r");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("prefix = \"C-a\""), "{written}");
    assert_eq!(session.input_route.prefix, 0x01, "rebound live");
    // Escape from a prompt returns to the dialog, not out of it.
    session.route_stdin_bytes(&mut conn, b"\r\x1b");
    assert!(matches!(session.modal, Modal::Settings(_)));
}

/// The keys section scrolls: every chord row is reachable and the
/// selection stays visible.
#[test]
fn settings_keys_section_scrolls_to_every_row() {
    use super::super::modal_settings::SettingsSection;
    let (_rx, mut conn, mut session) = command_menu_session("cmdmenu-keys");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[client]\n").unwrap();
    session.settings_path = Some(path.clone());
    session.enter_settings();
    // Five Tabs: the keys section (sidebar borders general notify keys).
    session.route_stdin_bytes(&mut conn, b"\t\t\t\t\t");
    let Modal::Settings(state) = &session.modal else {
        panic!("open")
    };
    assert_eq!(state.section, SettingsSection::Keys);
    assert_eq!(state.selected, 0);
    assert_eq!(state.top, 0);
    // Down to the last row: the window follows the selection (the keys
    // section holds every management chord plus prefix and reload — 35
    // rows with break-pane, join-pane, the move-tab and tab-strip
    // chords, notify-jump, and the five layout chords aboard).
    session.route_stdin_bytes(&mut conn, &[b'j'; 34][..]);
    let Modal::Settings(state) = &session.modal else {
        panic!("open")
    };
    let last = state.section.row_count() - 1;
    assert_eq!(state.selected, last);
    assert_eq!(state.top, last + 1 - 8);
    // The composed body shows the last row visible with its value.
    let overlay = session.renderer.overlay.as_ref().expect("up");
    let last_row = overlay.1[5 + (last - state.top)].text.clone();
    assert!(last_row.contains("respawn dead pane"), "{last_row}");
    assert!(last_row.trim_end().ends_with(" r"), "{last_row}");
    // Up to the first: the window snaps back.
    session.route_stdin_bytes(&mut conn, &[b'k'; 34][..]);
    let Modal::Settings(state) = &session.modal else {
        panic!("open")
    };
    assert_eq!((state.selected, state.top), (0, 0));
}

/// Reopen the settings dialog straight from the session (the menu is
/// already exercised above).
fn click_menu_row_after_reopen(
    session: &mut WindowSession,
    _conn: &mut crate::mux::attach::conn::AttachConn,
) {
    session.enter_settings();
    assert!(matches!(session.modal, Modal::Settings(_)));
}
