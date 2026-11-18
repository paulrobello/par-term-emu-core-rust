//! WindowSession suites: the layouts modal and layout chords.

use super::*;

/// Test-side reads of the layouts dialog's state.
fn layouts_state(session: &WindowSession) -> &super::super::modal_layouts::LayoutsState {
    match &session.modal {
        Modal::Layouts(state) => state,
        other => panic!("no layouts dialog up: {other:?}"),
    }
}

/// The overlay's row texts, joined for substring asserts.
fn layouts_overlay_text(session: &WindowSession) -> String {
    session
        .renderer
        .overlay
        .as_ref()
        .map(|o| {
            o.1.iter()
                .map(|r| r.text.clone())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// A scripted daemon answering `list-layouts` with `listed`, and a
/// session with the layouts dialog already opened over it.
fn layouts_conn(
    tag: &str,
    listed: &str,
) -> (
    std::sync::mpsc::Receiver<(String, String)>,
    crate::mux::attach::conn::AttachConn,
    WindowSession,
) {
    let mut replies = std::collections::HashMap::new();
    replies.insert("list-layouts".to_string(), listed.to_string());
    let (rx, conn) = scripted_conn(tag, replies);
    let mut session = WindowSession::new(80, 24);
    session.enter_layouts();
    (rx, conn, session)
}

fn escape_key(session: &mut WindowSession, conn: &mut crate::mux::attach::conn::AttachConn) {
    session.route_key(
        conn,
        &par_term_emu_core::keyboard::TermKeyEvent::functional(
            par_term_emu_core::keyboard::TermKey::Escape,
            0,
        ),
    );
}

/// The command menu carries a `Layouts…` row beside Settings; activating
/// it opens the layouts dialog on its Choose state.
#[test]
fn command_menu_offers_layouts_entry() {
    use super::super::modal_layouts::LayoutsState;
    let (rows, mapping) = compose_menu_panel(&MenuTarget::Commands, None, false);
    assert_eq!(rows[1].text, " Layouts… ");
    assert_eq!(mapping[1], Some(MenuAction::Layouts));
    let (_rx, mut conn, mut session) = command_menu_session("cmdmenu-layouts");
    click_menu_row(&mut session, &mut conn, 1);
    assert!(session.menu().is_none(), "the menu closed");
    assert_eq!(
        layouts_state(&session),
        &LayoutsState::Choose { selected: 0 }
    );
}

/// Save… -> type a name -> Enter sends `save-layout <name>` and closes.
#[test]
fn layouts_save_flow_prompts_then_sends() {
    use super::super::modal_layouts::LayoutsState;
    let (rx, mut conn, mut session) = layouts_conn("layouts-save", "");
    session.route_stdin_bytes(&mut conn, b"\r");
    assert!(matches!(
        layouts_state(&session),
        LayoutsState::SavePrompt { .. }
    ));
    session.route_stdin_bytes(&mut conn, b"dev");
    session.route_stdin_bytes(&mut conn, b"\r");
    settle(&mut session, &mut conn);
    assert_eq!(wait_recorded(&rx, "save-layout"), "save-layout dev");
    assert!(matches!(session.modal, Modal::None), "dialog closed");
}

/// A name that already exists asks before overwriting; Esc cancels back
/// to the prompt without sending, Enter confirms and sends.
#[test]
fn layouts_save_existing_name_confirms_overwrite() {
    use super::super::modal_layouts::LayoutsState;
    let (rx, mut conn, mut session) = layouts_conn("layouts-overwrite", "dev\nprod");
    session.route_stdin_bytes(&mut conn, b"\r");
    session.route_stdin_bytes(&mut conn, b"dev");
    session.route_stdin_bytes(&mut conn, b"\r");
    assert_eq!(
        layouts_state(&session),
        &LayoutsState::SaveConfirmOverwrite {
            name: "dev".to_string()
        }
    );
    assert!(layouts_overlay_text(&session).contains("Overwrite"));
    escape_key(&mut session, &mut conn);
    assert!(matches!(
        layouts_state(&session),
        LayoutsState::SavePrompt { .. }
    ));
    settle(&mut session, &mut conn);
    assert!(
        !drained(&rx).iter().any(|l| l.starts_with("save-layout")),
        "Esc sent nothing"
    );
    // Back on the prompt: Enter re-asks, Enter again confirms.
    session.route_stdin_bytes(&mut conn, b"\r");
    session.route_stdin_bytes(&mut conn, b"\r");
    settle(&mut session, &mut conn);
    assert_eq!(wait_recorded(&rx, "save-layout"), "save-layout dev");
    assert!(matches!(session.modal, Modal::None));
}

/// Load… lists the saved layouts, a pick asks to replace the live panes,
/// and the confirm sends `load-layout <name> -f`; Esc at the confirm
/// sends nothing.
#[test]
fn layouts_load_flow_picks_confirms_sends_force() {
    use super::super::modal_layouts::LayoutsState;
    let (rx, mut conn, mut session) = layouts_conn("layouts-load", "dev\nprod");
    session.route_stdin_bytes(&mut conn, b"j");
    session.route_stdin_bytes(&mut conn, b"\r");
    assert_eq!(
        layouts_state(&session),
        &LayoutsState::LoadPick {
            names: vec!["dev".to_string(), "prod".to_string()],
            selected: 0
        }
    );
    session.route_stdin_bytes(&mut conn, b"\r");
    assert_eq!(
        layouts_state(&session),
        &LayoutsState::LoadConfirm {
            name: "dev".to_string()
        }
    );
    assert!(layouts_overlay_text(&session).contains("Replace current panes"));
    escape_key(&mut session, &mut conn);
    assert!(matches!(
        layouts_state(&session),
        LayoutsState::LoadPick { .. }
    ));
    settle(&mut session, &mut conn);
    assert!(
        !drained(&rx).iter().any(|l| l.starts_with("load-layout")),
        "Esc at the confirm sent nothing"
    );
    // Back on the pick: Enter re-asks, Enter again confirms.
    session.route_stdin_bytes(&mut conn, b"\r");
    session.route_stdin_bytes(&mut conn, b"\r");
    settle(&mut session, &mut conn);
    assert_eq!(wait_recorded(&rx, "load-layout"), "load-layout dev -f");
    assert!(matches!(session.modal, Modal::None));
}

/// A load swaps the tree under every client: the sessions-changed push
/// closes the dialog.
#[test]
fn layouts_modal_closes_on_sessions_changed() {
    let (_rx, _conn, mut session) = layouts_conn("layouts-sc", "");
    assert!(matches!(session.modal, Modal::Layouts(_)));
    session.handle_event(TmuxNotification::SessionsChanged);
    assert!(matches!(session.modal, Modal::None), "dialog closed");
    assert!(session.renderer.overlay.is_none(), "overlay dropped");
}

/// A press on a load-list row selects it and moves to the confirm.
#[test]
fn layouts_pointer_click_on_list_row_asks_to_confirm() {
    use super::super::modal_layouts::LayoutsState;
    let (_rx, mut conn, mut session) = layouts_conn("layouts-click-row", "dev\nprod");
    session.route_stdin_bytes(&mut conn, b"j");
    session.route_stdin_bytes(&mut conn, b"\r");
    click_menu_row(&mut session, &mut conn, 1);
    assert_eq!(
        layouts_state(&session),
        &LayoutsState::LoadConfirm {
            name: "prod".to_string()
        }
    );
}

/// The footer is an overlay row, not a list row: with more layouts than
/// fit, pressing it must not pick one.
#[test]
fn layouts_pointer_click_on_footer_is_consumed() {
    use super::super::modal_layouts::LayoutsState;
    let names: Vec<String> = (0..12).map(|i| format!("l{i:02}")).collect();
    let (rx, mut conn, mut session) = layouts_conn("layouts-click-footer", &names.join("\n"));
    session.route_stdin_bytes(&mut conn, b"j");
    session.route_stdin_bytes(&mut conn, b"\r");
    let before = layouts_state(&session).clone();
    assert!(matches!(before, LayoutsState::LoadPick { selected: 0, .. }));
    click_menu_row(&mut session, &mut conn, 10);
    assert_eq!(
        layouts_state(&session),
        &before,
        "footer press changed state"
    );
    settle(&mut session, &mut conn);
    assert!(!drained(&rx).iter().any(|l| l.contains("load-layout")));
}

/// A press outside the box closes the dialog.
#[test]
fn layouts_pointer_click_outside_closes() {
    let (_rx, mut conn, mut session) = layouts_conn("layouts-click-out", "");
    session.route_mouse(&mut conn, sgr(0, 1, 1, false));
    assert!(matches!(session.modal, Modal::None));
    assert!(session.renderer.overlay.is_none());
}

/// No saved layouts: the placeholder row renders, Enter only closes.
#[test]
fn layouts_empty_list_shows_placeholder_and_only_closes() {
    use super::super::modal_layouts::LayoutsState;
    let (rx, mut conn, mut session) = layouts_conn("layouts-empty", "");
    session.route_stdin_bytes(&mut conn, b"j");
    session.route_stdin_bytes(&mut conn, b"\r");
    assert!(matches!(
        layouts_state(&session),
        LayoutsState::LoadPick { names, .. } if names.is_empty()
    ));
    assert!(layouts_overlay_text(&session).contains("(no saved layouts)"));
    click_menu_row(&mut session, &mut conn, 0);
    assert!(
        matches!(session.modal, Modal::Layouts(_)),
        "row click inert"
    );
    session.route_stdin_bytes(&mut conn, b"\r");
    assert!(matches!(session.modal, Modal::None));
    settle(&mut session, &mut conn);
    assert!(!drained(&rx).iter().any(|l| l.contains("load-layout")));
}

/// An invalid name flashes and stays on the prompt with no round trip.
#[test]
fn layouts_invalid_name_flashes_without_round_trip() {
    use super::super::modal_layouts::LayoutsState;
    let (rx, mut conn, mut session) = layouts_conn("layouts-badname", "");
    session.route_stdin_bytes(&mut conn, b"\r");
    session.route_stdin_bytes(&mut conn, b"a/b");
    session.route_stdin_bytes(&mut conn, b"\r");
    assert!(matches!(
        layouts_state(&session),
        LayoutsState::SavePrompt { .. }
    ));
    assert!(session.chrome.flash_text().is_some(), "flash shown");
    settle(&mut session, &mut conn);
    let sent = drained(&rx);
    assert!(
        !sent.iter().any(|l| l.contains("layout")),
        "no layout verb at all: {sent:?}"
    );
}

/// The functional-key spellings: arrows move, Backspace edits, Enter
/// and Esc act.
#[test]
fn layouts_key_spellings_navigate_and_edit() {
    use super::super::modal_layouts::LayoutsState;
    use par_term_emu_core::keyboard::{TermKey, TermKeyEvent};
    let (_rx, mut conn, mut session) = layouts_conn("layouts-keys", "dev");
    let key = TermKeyEvent::functional;
    session.route_key(&mut conn, &key(TermKey::Down, 0));
    assert_eq!(
        layouts_state(&session),
        &LayoutsState::Choose { selected: 1 }
    );
    session.route_key(&mut conn, &key(TermKey::Up, 0));
    assert_eq!(
        layouts_state(&session),
        &LayoutsState::Choose { selected: 0 }
    );
    session.route_key(&mut conn, &key(TermKey::Enter, 0));
    session.route_stdin_bytes(&mut conn, b"abc");
    session.route_key(&mut conn, &key(TermKey::Backspace, 0));
    assert_eq!(
        layouts_state(&session),
        &LayoutsState::SavePrompt {
            text: "ab".to_string()
        }
    );
    escape_key(&mut session, &mut conn);
    assert_eq!(
        layouts_state(&session),
        &LayoutsState::Choose { selected: 0 }
    );
    escape_key(&mut session, &mut conn);
    assert!(matches!(session.modal, Modal::None));
}

/// Shifted CSI-u text — `ESC[64;2u` decodes to Char '@' carrying the
/// shift bit, `ESC[65;2u` to 'A' — lands in the save-name prompt's
/// buffer instead of being dropped by the bare-modifier gate (card
/// 01a18127).
#[test]
fn layouts_save_prompt_accepts_shifted_csi_u_chars() {
    use super::super::modal_layouts::LayoutsState;
    use par_term_emu_core::keyboard::{TermKey, TermKeyEvent};
    let (_rx, mut conn, mut session) = layouts_conn("layouts-shift", "");
    session.route_key(&mut conn, &TermKeyEvent::functional(TermKey::Enter, 0));
    assert!(matches!(
        layouts_state(&session),
        LayoutsState::SavePrompt { .. }
    ));
    session.route_stdin_bytes(&mut conn, b"\x1b[64;2u\x1b[65;2u");
    assert_eq!(
        layouts_state(&session),
        &LayoutsState::SavePrompt {
            text: "@A".to_string()
        }
    );
}

/// Esc at the load confirm backs out to the pick list (cursor on the
/// layout) and sends nothing.
#[test]
fn layouts_escape_from_load_confirm_backs_out_without_sending() {
    use super::super::modal_layouts::LayoutsState;
    let (rx, mut conn, mut session) = layouts_conn("layouts-esc-confirm", "dev\nprod");
    session.route_stdin_bytes(&mut conn, b"j");
    session.route_stdin_bytes(&mut conn, b"\r");
    session.route_stdin_bytes(&mut conn, b"j");
    session.route_stdin_bytes(&mut conn, b"\r");
    escape_key(&mut session, &mut conn);
    assert_eq!(
        layouts_state(&session),
        &LayoutsState::LoadPick {
            names: vec!["dev".to_string(), "prod".to_string()],
            selected: 1
        }
    );
    settle(&mut session, &mut conn);
    assert!(!drained(&rx).iter().any(|l| l.starts_with("load-layout")));
}

/// A session on the three-pane snapshot layout, wire drained.
fn layout_chord_session(
    tag: &str,
) -> (
    std::sync::mpsc::Receiver<(String, String)>,
    crate::mux::attach::conn::AttachConn,
    WindowSession,
) {
    let (rx, conn) = fake_daemon(tag, FakeScript::default());
    let mut session = WindowSession::new(90, 26);
    session
        .renderer
        .apply_layout(parse_layout(THREE_PANE).expect("parses"));
    session.window = "@0".to_string();
    drained(&rx);
    (rx, conn, session)
}

/// prefix + M-1..M-5 each send `select-layout <preset> -t <shown window>`.
#[test]
fn alt_digit_chords_send_select_layout_per_preset() {
    let presets = [
        (b'1', "even-horizontal"),
        (b'2', "even-vertical"),
        (b'3', "main-horizontal"),
        (b'4', "main-vertical"),
        (b'5', "tiled"),
    ];
    for (digit, name) in presets {
        let (rx, mut conn, mut session) = layout_chord_session("altlayout");
        session.route_stdin_bytes(&mut conn, &[0x02, 0x1b, digit]);
        assert_eq!(
            received_within_300ms(&rx),
            vec![format!("select-layout {name} -t @0")],
            "M-{}",
            digit as char
        );
    }
}

#[test]
fn m1_chord_sends_select_layout_even_horizontal() {
    let (rx, mut conn, mut session) = layout_chord_session("m1layout");
    session.route_stdin_bytes(&mut conn, &[0x02, 0x1b, b'1']);
    assert_eq!(
        received_within_300ms(&rx),
        vec!["select-layout even-horizontal -t @0".to_string()]
    );
}

/// An unbound Alt chord after the prefix is consumed: nothing on the wire,
/// nothing forwarded to the pane.
#[test]
fn unbound_alt_chord_with_prefix_armed_is_consumed() {
    let (rx, mut conn, mut session) = layout_chord_session("altunbound");
    session.route_stdin_bytes(&mut conn, &[0x02, 0x1b, b'7']);
    assert!(received_within_300ms(&rx).is_empty());
    assert!(!session.input_route.prefix_pending, "the prefix is spent");
}

/// Without the prefix an Alt key still reaches the pane program.
#[test]
fn alt_key_without_prefix_still_forwards() {
    let (rx, mut conn, mut session) = layout_chord_session("altfwd");
    session.route_stdin_bytes(&mut conn, &[0x1b, b'1']);
    let lines = received_within_300ms(&rx);
    assert!(
        lines.iter().any(|l| l.starts_with("send-keys")),
        "Alt+1 forwards to the pane: {lines:?}"
    );
    assert!(!lines.iter().any(|l| l.starts_with("select-layout")));
}

/// M-5 on a zoomed window still sends the command; the daemon-side
/// unzoom and the %layout-change reseed clear the cue.
#[test]
fn m5_on_zoomed_window_sends_select_layout_tiled() {
    let (rx, mut conn, mut session) = layout_chord_session("m5zoom");
    session.zoomed = true;
    session.route_stdin_bytes(&mut conn, &[0x02, 0x1b, b'5']);
    assert_eq!(
        received_within_300ms(&rx),
        vec!["select-layout tiled -t @0".to_string()]
    );
}

#[test]
fn select_layout_refusal_flashes() {
    let script = FakeScript {
        failing: ["select-layout tiled -t @0".to_string()].into(),
        ..FakeScript::default()
    };
    let (rx, mut conn, mut session) = two_pane_session("layout-refused", script);
    session.route_stdin_bytes(&mut conn, &[0x02, 0x1b, b'5']);
    settle(&mut session, &mut conn);
    assert_eq!(drained(&rx), vec!["select-layout tiled -t @0"]);
    assert_eq!(session.chrome.flash_text(), Some("layout tiled: refused"));
}

#[test]
fn plain_byte_lookup_does_not_resolve_an_alt_bound_layout_chord() {
    let session = WindowSession::new(90, 26);
    assert_eq!(
        session.management_key(crate::mux::config::ChordKey::alt(b'1')),
        Some(ManagementKey::LayoutEvenHorizontal)
    );
    assert_eq!(
        session.management_key(crate::mux::config::ChordKey::plain(b'1')),
        None
    );
    assert!(crate::mux::config::management_key("M-1", "popup[0].key").is_err());
    assert_eq!(
        crate::mux::config::management_key("1", "popup[0].key"),
        Ok(b'1')
    );
}

#[test]
fn alt_non_ascii_chord_with_prefix_flashes_the_ascii_hint() {
    let (rx, mut conn, mut session) = layout_chord_session("altnonascii");
    for ch in ['中', ' '] {
        session.route_plain(&[0x02], &mut conn);
        session.route_key(
            &mut conn,
            &TermKeyEvent::char_(ch, par_term_emu_core::keyboard::modifiers::ALT),
        );
        assert_eq!(
            session.chrome.flash_text(),
            Some(non_ascii_chord_hint(ch).as_str())
        );
        assert!(!session.input_route.prefix_pending);
    }
    assert!(received_within_300ms(&rx).is_empty());
}

#[test]
fn palette_layout_row_sends_select_layout() {
    let (rx, mut conn, mut session) = two_pane_session("palette-layout", FakeScript::default());
    chord(&mut session, &mut conn, b':');
    drained(&rx);
    session.route_plain(b"layout tiled", &mut conn);
    session.route_plain(b"\r", &mut conn);
    settle(&mut session, &mut conn);
    assert_eq!(drained(&rx), vec!["select-layout tiled -t @0"]);
}
