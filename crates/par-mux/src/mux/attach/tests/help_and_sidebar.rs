//! Help rows, chrome sanitizing, and sidebar composition.

use super::*;

/// prefix ?: the bindings panel prints as plain text with the LIVE
/// chords — a remapped split chord shows its remapped key, and the
/// category headers are present.
#[test]
fn help_rows_carry_effective_bindings_and_categories() {
    use crate::mux::config::Management;
    let rows = super::help_rows(
        0x02,
        0x12,
        Management {
            split_right: crate::mux::config::ChordKey::plain(b's'),
            ..Management::default()
        },
        2,
    );
    let text: Vec<&str> = rows.iter().map(|r| r.text.as_str()).collect();
    let joined = text.join("\n");
    for category in [
        "global",
        "panes",
        "tabs / windows / sessions",
        "navigation",
        "mouse",
    ] {
        assert!(
            text.iter().any(|t| t.contains(category)),
            "the category header {category} is present: {joined}"
        );
    }
    // Effective bindings: the remapped split shows s, not %.
    assert!(
        text.iter()
            .any(|t| t.starts_with(" C-b s") && t.contains("split right")),
        "the remapped chord shows its remapped key: {joined}"
    );
    assert!(
        !text.iter().any(|t| t.contains("C-b %")),
        "the old default spelling is gone: {joined}"
    );
    // The resize step surfaces in the resize row.
    assert!(
        text.iter().any(|t| t.contains("edge moves by 2")),
        "the step reflects the live config: {joined}"
    );
}

/// The passthrough dump leads with a blank line (the first header used
/// to print on the cursor's current line — the prompt's) and bolds the
/// category headers; plain rows stay unstyled.
#[test]
fn passthrough_help_dump_leads_with_a_blank_line_and_bolds_headers() {
    let rows = super::help_rows(0x02, 0x12, Default::default(), 1);
    let dump = super::help_dump_text(&rows);
    assert!(
        dump.starts_with("\r\n"),
        "the dump must start on a fresh line: {dump:?}"
    );
    let accent_count = rows.iter().filter(|r| r.accent).count();
    assert_eq!(
        dump.matches("\x1b[1m").count(),
        accent_count,
        "every category header is bold, nothing else: {dump:?}"
    );
    assert!(
        dump.contains("\x1b[1m global \x1b[0m\r\n"),
        "the global header is bold: {dump:?}"
    );
    // The first entry row is plain: no SGR anywhere outside headers.
    assert!(
        dump.contains("\r\n C-b C-b  type a literal prefix\r\n"),
        "entry rows stay plain: {dump:?}"
    );
}

/// The passthrough help chord advances the pane past the dump — empty
/// Enters (one command line + fresh prompt each, ~2 rows) land the
/// shell below the text so later keystrokes do not paint over it.
#[test]
fn help_chord_advances_the_pane_past_the_dump() {
    let (_daemon, path) = FakeDaemon::bind("help-advance");
    let mut session = Session {
        conn: conn::AttachConn::connect(&path).expect("connect"),
        socket_path: path.clone(),
        pane: "%0".to_string(),
        emulator: render::PaneEmulator::new(80, 24),
        window: String::new(),
        session_id: None,
        session_name: String::new(),
        workspaces: Vec::new(),
        active_workspace: None,
        session_workspace: None,
        pane_title: String::new(),
        agents: 0,
        exited: None,
        drawn_size: None,
        settling: false,
        prefix: 0x02,
        prefix_pending: false,
        prefix_utf8: None,
        reload_key: 0x12,
        management: crate::mux::config::Management::default(),
        resize_step: 1,
        resize_mode: false,
        flash: None,
        paste: PasteTracker::default(),
    };
    let expected = super::help_rows(0x02, 0x12, Default::default(), 1).len() / 2 + 1;
    session.show_help();
    let line = wait_for_line(&_daemon.received, "send-keys");
    let sent = line.matches("0d").count();
    assert_eq!(
        sent, expected,
        "one Enter per ~2 dump rows advances the pane past the help: {line}"
    );
}

/// The sidebar compose after round 6: NO section header rows (the
/// title moved to the tab strip's lead segment), workspace rows
/// start at composed row 0, and the footer row pins ` new ` to the
/// panel's bottom-left and ` menu ` to its bottom-right — each chip
/// its own clickable span and id.
#[test]
fn chrome_safe_strips_bidi_and_zero_width() {
    let set = ('\u{202A}'..='\u{202E}')
        .chain('\u{2066}'..='\u{2069}')
        .chain(['\u{200E}', '\u{200F}', '\u{061C}', '\u{200B}', '\u{FEFF}']);
    for c in set {
        assert_eq!(
            super::chrome_safe(&format!("a{c}b")),
            "ab",
            "U+{:04X} is stripped",
            c as u32
        );
    }
}

#[test]
fn chrome_safe_keeps_zwj_and_zwnj() {
    for s in ["👨\u{200D}👩\u{200D}👧", "می\u{200C}خواهم"] {
        let out = super::chrome_safe(s);
        assert!(
            matches!(out, std::borrow::Cow::Borrowed(_)),
            "{s:?} borrowed"
        );
        assert_eq!(out, s);
    }
}

/// A side-panel row for the compose tests.
fn srow(id: super::SidebarRowId, label: &str, active: bool) -> super::SidebarRow {
    super::SidebarRow {
        id,
        label: label.to_string(),
        active,
        status: super::AgentStatus::None,
        continuation: Vec::new(),
        badge: false,
    }
}

/// The two areas the compose tests share: three workspaces, two agents.
/// The agents header carries the sort toggle at its right edge (herdr's
/// `agent_sort_toggle`): the label names the CURRENT mode and its span
/// clicks as `AgentsSort`, on the header's own row.
#[test]
fn agents_header_sort_toggle_is_clickable_and_names_the_mode() {
    let sections = two_area_sections();
    for mode in [
        super::AgentSortMode::Grouped,
        super::AgentSortMode::Priority,
    ] {
        let lines = super::compose_sidebar(&sections, 20, 20, super::SIDEBAR_SPLIT_DEFAULT, mode);
        let header_y = lines
            .iter()
            .find(|line| line.kind == super::SidebarLineKind::Header)
            .map(|line| line.y)
            .expect("agents header");
        let toggle = lines
            .iter()
            .find(|line| line.id == Some(super::SidebarRowId::AgentsSort))
            .expect("sort toggle");
        assert_eq!(toggle.y, header_y);
        assert_eq!(toggle.text, mode.label());
        assert_eq!(toggle.x_end, 19, "flush with the content's right edge");
    }
}

fn two_area_sections() -> Vec<super::SidebarSection> {
    vec![
        super::SidebarSection {
            title: "workspaces",
            rows: vec![
                srow(super::SidebarRowId::Workspace("+0".into()), "main", true),
                srow(super::SidebarRowId::Workspace("+1".into()), "dev", false),
                srow(super::SidebarRowId::Workspace("+2".into()), "ops", false),
            ],
        },
        super::SidebarSection {
            title: "agents",
            rows: vec![
                srow(super::SidebarRowId::Pane(1), "claude (working)", true),
                srow(super::SidebarRowId::Pane(3), "kimi (blocked)", false),
            ],
        },
    ]
}

/// The line composed at row `y` whose click target is `id`.
fn line_with(lines: &[super::SidebarLine], id: &super::SidebarRowId) -> super::SidebarLine {
    lines
        .iter()
        .find(|l| l.id.as_ref() == Some(id))
        .cloned()
        .unwrap_or_else(|| panic!("no line for {id:?}: {lines:?}"))
}

#[test]
fn compose_sidebar_drops_spoofing_chars() {
    let sections = vec![super::SidebarSection {
        title: "workspaces",
        rows: vec![srow(
            super::SidebarRowId::Workspace("+0".into()),
            "\u{202E}evil",
            false,
        )],
    }];
    let lines = super::compose_sidebar(
        &sections,
        20,
        10,
        super::SIDEBAR_SPLIT_DEFAULT,
        super::AgentSortMode::Grouped,
    );
    assert_eq!(lines[0].text, "   evil");
}

/// herdr's panel shape at the default split: the workspace area takes
/// half the rows — its rows open the panel directly (the tab strip's
/// lead segment carries the title, so no in-panel header) and the
/// ` new ` / ` menu ` chips sit on its LAST row — then the rule, the
/// agents header and rows, and the collapse chevron on the panel's
/// last row.
#[test]
fn compose_sidebar_splits_half_and_ends_the_workspace_area_with_the_chips() {
    use super::{SidebarLineKind, SidebarRowId};
    let lines = super::compose_sidebar(
        &two_area_sections(),
        20,
        20,
        super::SIDEBAR_SPLIT_DEFAULT,
        super::AgentSortMode::Grouped,
    );
    let at = |y: u16| -> Vec<&super::SidebarLine> { lines.iter().filter(|l| l.y == y).collect() };
    assert_eq!(at(0)[0].text, " ▸ main");
    assert!(at(0)[0].active, "the active row is flagged");
    assert_eq!(at(1)[0].text, "   dev");
    assert_eq!(at(2)[0].text, "   ops");
    // The chips sit on the workspace area's last row (10 of 20 -> row 9).
    let new_chip = line_with(&lines, &SidebarRowId::New);
    assert_eq!((new_chip.y, new_chip.x, new_chip.x_end), (9, 0, 5));
    assert_eq!(new_chip.text, " new ");
    let menu_chip = line_with(&lines, &SidebarRowId::Menu);
    assert_eq!((menu_chip.y, menu_chip.x, menu_chip.x_end), (9, 13, 19));
    assert_eq!(menu_chip.text, " menu ");
    // The rule opens the agents area at the split row.
    assert_eq!(at(10)[0].kind, SidebarLineKind::Rule);
    assert_eq!(at(10)[0].text, "─".repeat(19));
    assert_eq!(at(10)[0].id, None, "the rule is never clickable");
    assert_eq!(at(11)[0].text, " agents");
    assert_eq!(at(11)[0].kind, SidebarLineKind::Header);
    assert_eq!(at(12)[0].text, " ▸ claude (working)");
    assert_eq!(at(12)[0].id, Some(SidebarRowId::Pane(1)));
    assert_eq!(at(13)[0].text, "   kimi (blocked)");
    // The chevron: the panel's last row, beside the divider column.
    let chevron = line_with(&lines, &SidebarRowId::Collapse);
    assert_eq!((chevron.y, chevron.x, chevron.x_end), (19, 18, 19));
    assert_eq!(chevron.text, "«");
    // Long labels clip to the content width (width - 1).
    let long = vec![super::SidebarSection {
        title: "workspaces",
        rows: vec![srow(
            SidebarRowId::Workspace("+0".into()),
            "a very long workspace name",
            false,
        )],
    }];
    let clipped = super::compose_sidebar(
        &long,
        20,
        20,
        super::SIDEBAR_SPLIT_DEFAULT,
        super::AgentSortMode::Grouped,
    );
    assert_eq!(clipped[0].text, "   a very long work");
    // With no agents section the agents area still opens with its rule
    // and header (the panel shape holds).
    assert!(clipped
        .iter()
        .any(|l| l.y == 10 && l.kind == SidebarLineKind::Rule));
    assert!(clipped.iter().any(|l| l.y == 11 && l.text == " agents"));
}

/// The split is a ratio, so the areas follow it and each keeps at least
/// three rows: rows past the workspace area's footer row clip, the rule
/// moves with the split, and an extreme split clamps.
#[test]
fn compose_sidebar_split_ratio_moves_the_rule_and_clamps() {
    use super::{SidebarLineKind, SidebarRowId};
    let sections = two_area_sections();
    let rule_y = |lines: &[super::SidebarLine]| {
        lines
            .iter()
            .find(|l| l.kind == SidebarLineKind::Rule)
            .map(|l| l.y)
    };
    // 0.3 of 20 rows: a 6-row workspace area, chips on row 5.
    let low = super::compose_sidebar(&sections, 20, 20, 0.3, super::AgentSortMode::Grouped);
    assert_eq!(rule_y(&low), Some(6));
    assert_eq!(line_with(&low, &SidebarRowId::New).y, 5);
    // A 4-row area holds all three rows and the chips (no header to
    // spend a row on).
    let tight = super::compose_sidebar(&sections, 20, 20, 0.2, super::AgentSortMode::Grouped);
    assert_eq!(rule_y(&tight), Some(4));
    assert!(tight.iter().any(|l| l.text == "   ops"));
    // The clamp floor: a 3-row area holds two rows and the chips — ops
    // clips past the footer row.
    let floor = super::compose_sidebar(&sections, 20, 20, 0.15, super::AgentSortMode::Grouped);
    assert_eq!(rule_y(&floor), Some(3));
    assert!(floor.iter().any(|l| l.text == "   dev"));
    assert!(
        !floor.iter().any(|l| l.text == "   ops"),
        "rows past the footer clip: {floor:?}"
    );
    // The extremes clamp to keep each area 3 rows.
    assert_eq!(
        rule_y(&super::compose_sidebar(
            &sections,
            20,
            20,
            0.01,
            super::AgentSortMode::Grouped
        )),
        Some(3)
    );
    assert_eq!(
        rule_y(&super::compose_sidebar(
            &sections,
            20,
            20,
            0.99,
            super::AgentSortMode::Grouped
        )),
        Some(17)
    );
    assert_eq!(super::sidebar_workspace_rows(20, 0.5), 10);
    assert_eq!(
        super::sidebar_workspace_rows(5, 0.9),
        3,
        "a short panel halves"
    );
    // Agent rows past the chevron row clip.
    let short = super::compose_sidebar(&sections, 20, 8, 0.5, super::AgentSortMode::Grouped);
    assert_eq!(rule_y(&short), Some(4));
    assert!(short
        .iter()
        .any(|l| l.y == 6 && l.id == Some(SidebarRowId::Pane(1))));
    assert!(!short.iter().any(|l| l.id == Some(SidebarRowId::Pane(3))));
    assert_eq!(line_with(&short, &SidebarRowId::Collapse).y, 7);
}

/// An agent entry renders its `agent-rows` token lines: tokens join with
/// ` · ` (one space after the state icon), a token with nothing to show
/// drops out, an empty line drops, and an all-empty entry still names
/// the agent.
#[test]
fn agent_entry_lines_render_the_token_rows() {
    let facts = super::AgentFacts {
        pane: 4,
        agent: "claude",
        state: "working",
        workspace: Some("main"),
        project: Some("par-term"),
        message: Some("waiting on permission"),
    };
    let rows = crate::mux::config::default_agent_rows();
    assert_eq!(
        super::agent_entry_lines(&rows, &facts),
        vec![
            "\u{25CF} main \u{b7} par-term".to_string(),
            "claude".to_string()
        ]
    );
    let bare = super::AgentFacts {
        workspace: None,
        project: None,
        message: None,
        ..facts
    };
    assert_eq!(
        super::agent_entry_lines(&rows, &bare),
        vec!["\u{25CF}".to_string(), "claude".to_string()]
    );
    let custom = vec![
        vec!["project".to_string()],
        vec!["pane".to_string(), "state".to_string(), "agent".to_string()],
    ];
    assert_eq!(
        super::agent_entry_lines(&custom, &bare),
        vec!["%4 \u{b7} working \u{b7} claude".to_string()],
        "the empty project line drops"
    );
    assert_eq!(
        super::agent_entry_lines(&[vec!["project".to_string()]], &bare),
        vec!["claude".to_string()],
        "an entry with nothing to show still names the agent"
    );
}

/// A multi-line entry composes one panel line per entry line, the `▸`
/// marker on the first and the rest indented under the label; every line
/// keeps the entry's click target and active block, and an entry the
/// area cannot hold whole shows the lines that fit.
#[test]
fn compose_sidebar_lays_out_multi_line_agent_entries() {
    use super::SidebarRowId;
    let mut sections = two_area_sections();
    for row in &mut sections[1].rows {
        row.continuation = vec!["agent".to_string()];
    }
    let lines = super::compose_sidebar(
        &sections,
        20,
        20,
        super::SIDEBAR_SPLIT_DEFAULT,
        super::AgentSortMode::Grouped,
    );
    let at = |y: u16| lines.iter().find(|l| l.y == y).cloned().expect("line");
    assert_eq!(at(12).text, " ▸ claude (working)");
    assert_eq!(at(13).text, "   agent");
    assert_eq!(
        at(13).id,
        Some(SidebarRowId::Pane(1)),
        "the continuation clicks too"
    );
    assert!(at(13).active, "the active block covers the whole entry");
    assert_eq!(at(12).kind, super::SidebarLineKind::Row);
    assert_eq!(
        at(13).kind,
        super::SidebarLineKind::Continuation,
        "later lines paint as the entry's secondary lines"
    );
    assert_eq!(at(14).text, "   kimi (blocked)");
    assert_eq!(at(15).text, "   agent");
    assert!(!at(15).active);
    // The 8-row panel's agents area holds one line before the chevron:
    // the first entry's first line only.
    let short = super::compose_sidebar(&sections, 20, 8, 0.5, super::AgentSortMode::Grouped);
    assert_eq!(
        short
            .iter()
            .filter(|l| matches!(l.id, Some(SidebarRowId::Pane(_))))
            .count(),
        1
    );
}

/// A strip too narrow for a chip drops it; one too narrow for either
/// drops both, and the chevron needs two content columns.
#[test]
fn compose_sidebar_narrow_strips_drop_what_does_not_fit() {
    use super::SidebarRowId;
    let sections = two_area_sections();
    let narrow = super::compose_sidebar(&sections, 7, 10, 0.5, super::AgentSortMode::Grouped);
    assert!(narrow.iter().any(|l| l.id == Some(SidebarRowId::New)));
    assert!(!narrow.iter().any(|l| l.id == Some(SidebarRowId::Menu)));
    let tiny = super::compose_sidebar(&sections, 4, 10, 0.5, super::AgentSortMode::Grouped);
    assert!(
        !tiny
            .iter()
            .any(|l| matches!(l.id, Some(SidebarRowId::New | SidebarRowId::Menu))),
        "no chips fit a 3-col content: {tiny:?}"
    );
    assert!(tiny.iter().any(|l| l.id == Some(SidebarRowId::Collapse)));
}

/// herdr's collapsed rail at 2 columns: per workspace its index and its
/// status glyph, the rule at the same split, per agent its index and
/// glyph, and the expand chevron on the last row — every entry keeps its
/// click target.
#[test]
fn compose_sidebar_rail_shows_indices_statuses_and_the_expand_chevron() {
    use super::{AgentStatus, SidebarLineKind, SidebarRowId};
    let mut sections = two_area_sections();
    sections[0].rows[0].status = AgentStatus::Working;
    sections[0].rows[1].status = AgentStatus::Idle;
    sections[1].rows[0].status = AgentStatus::Working;
    sections[1].rows[1].status = AgentStatus::Blocked;
    let lines = super::compose_sidebar_rail(&sections, 12, super::SIDEBAR_SPLIT_DEFAULT);
    let at = |y: u16| lines.iter().find(|l| l.y == y).cloned();
    assert_eq!(at(0).unwrap().text, "1◐");
    assert!(
        at(0).unwrap().active,
        "the active workspace keeps its block"
    );
    assert_eq!(
        at(0).unwrap().id,
        Some(SidebarRowId::Workspace("+0".into()))
    );
    assert_eq!(at(1).unwrap().text, "2○");
    assert_eq!(at(2).unwrap().text, "3·", "no agents: the dim dot");
    assert_eq!(at(6).unwrap().kind, SidebarLineKind::Rule);
    assert_eq!(at(6).unwrap().text, "──");
    assert_eq!(at(7).unwrap().text, "1◐");
    assert_eq!(at(7).unwrap().id, Some(SidebarRowId::Pane(1)));
    assert_eq!(at(8).unwrap().text, "2●");
    let chevron = at(11).unwrap();
    assert_eq!(chevron.text, " »");
    assert_eq!(chevron.id, Some(SidebarRowId::Collapse));
    assert!(lines.iter().all(|l| l.x_end <= super::SIDEBAR_RAIL_WIDTH));
    // Past nine entries the index column reads `+`.
    let many = vec![super::SidebarSection {
        title: "workspaces",
        rows: (0..11)
            .map(|i| srow(SidebarRowId::Workspace(format!("+{i}")), "w", false))
            .collect(),
    }];
    let tall = super::compose_sidebar_rail(&many, 40, 0.5);
    assert_eq!(tall[8].text, "9·");
    assert_eq!(tall[9].text, "+·");
    // The workspace glyph is its agents' most urgent state: question
    // (a dialog asking the user to choose) outranks everything, and the
    // working glyph is the half-circle.
    assert!(AgentStatus::Question > AgentStatus::Blocked);
    assert!(AgentStatus::Blocked > AgentStatus::Done);
    assert!(AgentStatus::Done > AgentStatus::Working);
    assert!(AgentStatus::Working > AgentStatus::Idle);
    assert_eq!(AgentStatus::from_state("blocked"), AgentStatus::Blocked);
    assert_eq!(AgentStatus::from_state("question"), AgentStatus::Question);
    assert_eq!(AgentStatus::from_state("done"), AgentStatus::Done);
    assert_eq!(AgentStatus::Done.glyph(), "●", "unseen reads as busy");
    assert_eq!(
        AgentStatus::Working.glyph(),
        "◐",
        "working is the half-circle"
    );
    assert_eq!(AgentStatus::from_state("unknown"), AgentStatus::None);
}

/// The git badge's continuation line carries the Badge kind (blue in
/// paint_sidebar) while a plain continuation stays dim.
#[test]
fn compose_sidebar_marks_the_git_badge_line() {
    use super::SidebarLineKind;
    let mut sections = two_area_sections();
    sections[0].rows[0].continuation = vec![" main \u{2191}2".to_string()];
    sections[0].rows[0].badge = true;
    sections[0].rows[1].continuation = vec!["a second line".to_string()];
    let lines = super::compose_sidebar(
        &sections,
        20,
        20,
        super::SIDEBAR_SPLIT_DEFAULT,
        super::AgentSortMode::Grouped,
    );
    let badge = lines
        .iter()
        .find(|l| l.text == "    main \u{2191}2")
        .expect("the badge line composes");
    assert_eq!(badge.kind, SidebarLineKind::Badge);
    let plain = lines
        .iter()
        .find(|l| l.text == "   a second line")
        .expect("the plain continuation composes");
    assert_eq!(plain.kind, SidebarLineKind::Continuation);
}

/// A workspace's status glyph rides next to the NAME on the first line
/// (the STATE_ICON placeholder the renderer paints in the status's
/// color); the marker column stays the active pointer, and the
/// continuation lines — the git badge among them — carry no icon.
#[test]
fn compose_sidebar_puts_the_workspace_status_next_to_the_name() {
    let mut sections = two_area_sections();
    sections[0].rows[0].status = super::AgentStatus::Working;
    sections[0].rows[0].badge = true;
    sections[0].rows[0].continuation = vec![" main \u{2191}2".to_string()];
    let lines = super::compose_sidebar(
        &sections,
        20,
        20,
        super::SIDEBAR_SPLIT_DEFAULT,
        super::AgentSortMode::Grouped,
    );
    assert_eq!(
        lines[0].text,
        format!(" ▸ main {}", super::STATE_ICON),
        "the icon sits next to the name, the marker stays the active pointer"
    );
    let badge = lines
        .iter()
        .find(|l| l.kind == SidebarLineKind::Badge)
        .expect("the badge line composes");
    assert!(
        !badge.text.contains(super::STATE_ICON),
        "continuation lines carry no status icon: {:#?}",
        badge.text
    );
}

/// The new-workspace prompt's default bumps past conflicts: one past
/// the highest workspace ordinal, and past any workspace NAME that
/// already claims the number (the same rule next_window_name runs).
#[test]
fn next_workspace_name_bumps_past_conflicts() {
    assert_eq!(super::next_workspace_name(&[]), "1");
    let roster = vec![
        ("+1".to_string(), "1".to_string()),
        ("+2".to_string(), "3".to_string()),
    ];
    assert_eq!(
        super::next_workspace_name(&roster),
        "4",
        "one past the max ordinal, bumped past the claimed name '3'"
    );
    let claimed = vec![("+1".to_string(), "2".to_string())];
    assert_eq!(
        super::next_workspace_name(&claimed),
        "3",
        "a claimed default bumps again"
    );
}
