use super::format::MAX_PERSISTED_SCROLLBACK_CELLS;
use super::save::cap_grid_scrollback;
use super::tests::{assert_same_shape, populated_tree, tree};
use super::*;
use crate::mux::pane::ShellPaneFactory;
use crate::mux::tree::MuxTree;
use par_term_emu_core::terminal::replay_snapshot::GridSnapshot;

#[test]
fn envelope_survives_serde_round_trip() {
    let original = populated_tree();
    let state = original.to_persist_state();

    let json = serde_json::to_string(&state).expect("envelope serializes");
    let revived: PersistState = serde_json::from_str(&json).expect("envelope deserializes");
    assert_eq!(revived.format_version, FORMAT_VERSION);
    assert_eq!(revived.next_ids, state.next_ids);

    // Restoring through the revived envelope must land in the same
    // state as restoring through the original, byte for byte in shape.
    let direct =
        MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default())).unwrap();
    let via_json = MuxTree::from_persist_state(&revived, Box::new(ShellPaneFactory::default()))
        .expect("revived envelope restores");
    assert_same_shape(&via_json, &direct);
    assert_eq!(
        via_json.get_buffer("default").map(str::to_string),
        Some("hello".to_string())
    );
}

/// The skip-when-absent rule that keeps non-agent panes byte-identical
/// to the pre-6.1 format: the serialized envelope of a metadata-less
/// tree never contains the new key.
#[test]
fn non_agent_envelope_carries_no_agent_session_key() {
    let json =
        serde_json::to_string(&populated_tree().to_persist_state()).expect("envelope serializes");
    assert!(
        !json.contains("agent_session"),
        "the new field must skip when absent: {json}"
    );

    // And when present, it round-trips through the JSON itself — the
    // wire form of the identity, not just the in-memory struct.
    let mut tree = tree();
    let session = tree.new_session("agents", 80, 24).unwrap();
    let pane_id = tree
        .session(session)
        .unwrap()
        .windows
        .iter()
        .filter_map(|window| tree.window(*window))
        .flat_map(|window| window.panes())
        .next()
        .unwrap();
    let pane = tree.pane_mut(pane_id).unwrap();
    pane.set_metadata("agent", "pi");
    pane.set_metadata("agent_session_path", "/tmp/pi.jsonl");
    pane.set_metadata("agent_resume_argv", r#"["pi","--session","/tmp/pi.jsonl"]"#);
    pane.set_metadata("agent_state", "working");
    pane.set_metadata("agent_state_source", "hook");

    let json = serde_json::to_string(&tree.to_persist_state()).expect("serializes");
    let revived: PersistState = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(
        revived.sessions[0].windows[0].panes[0].agent_session,
        Some(PersistAgentSession {
            agent: "pi".to_string(),
            session_id: None,
            session_path: Some("/tmp/pi.jsonl".to_string()),
            source: None,
            resume_argv: Some(r#"["pi","--session","/tmp/pi.jsonl"]"#.to_string()),
            state: Some("working".to_string()),
            state_source: Some("hook".to_string()),
            message: None,
        }),
        "identity and last-known state survive the JSON wire form"
    );
}

/// A grid snapshot whose scrollback ring holds `lines` logical lines of
/// `cols` cells: logical line i is filled with the marker byte for i,
/// laid out through the live logical→physical mapping so a rotated
/// `start` exercises the extraction, and every 3rd logical line is
/// flagged wrapped.
fn ring_snapshot(lines: usize, cols: usize, start: usize, max: usize) -> GridSnapshot {
    // Snapshots carry the flat payload OLDEST-FIRST (one line per
    // `cols`, `scrollback_start` field removed with the per-line
    // storage). `start` and `max` are kept in the signature for the
    // cap-depth math of the callers.
    let _ = start;
    let physical_lines = lines.min(max);
    let mut cells = vec![par_term_emu_core::cell::Cell::default(); physical_lines * cols];
    let mut wrapped = vec![false; physical_lines];
    for logical in 0..lines.min(physical_lines) {
        for c in 0..cols {
            cells[logical * cols + c] = marker_cell(logical);
        }
        wrapped[logical] = logical % 3 == 0;
    }
    GridSnapshot {
        cells: Vec::new(),
        scrollback_cells: cells,
        scrollback_lines: physical_lines,
        max_scrollback: max,
        cols,
        rows: 24,
        wrapped: Vec::new(),
        scrollback_wrapped: wrapped,
        zones: Vec::new(),
        total_lines_scrolled: lines,
    }
}

/// A default cell carrying one marker character, so extracted rows are
/// identifiable by their logical line.
fn marker_cell(line: usize) -> par_term_emu_core::cell::Cell {
    let mut cell = par_term_emu_core::cell::Cell::default();
    cell.c = char::from_u32((line % 10) as u32 + '0' as u32).unwrap_or('0');
    cell
}

/// The marker of the first logical line retained after capping
/// `lines` down to the const's line budget.
fn first_kept_marker(lines: usize, cols: usize) -> char {
    let keep = (MAX_PERSISTED_SCROLLBACK_CELLS / cols).min(lines);
    marker_cell(lines - keep).c
}

#[test]
fn cap_scrollback_drops_oldest_lines_from_a_linear_ring() {
    let cols = 80;
    let lines = MAX_PERSISTED_SCROLLBACK_CELLS / cols + 250;
    let keep = MAX_PERSISTED_SCROLLBACK_CELLS / cols;
    let grid = cap_grid_scrollback(ring_snapshot(lines, cols, 0, 10_000));

    assert_eq!(grid.scrollback_lines, keep);
    assert_eq!(grid.scrollback_cells.len(), keep * cols);
    assert_eq!(grid.scrollback_wrapped.len(), keep);
    assert_eq!(grid.max_scrollback, 10_000, "restored pane keeps its depth");
    assert_eq!(grid.total_lines_scrolled, lines, "absolute frame is kept");
    assert_eq!(
        grid.scrollback_cells[0].c,
        first_kept_marker(lines, cols),
        "the oldest retained line leads the ring"
    );
    // Wrapped flags follow their logical lines: the first retained
    // logical line is `lines - keep`, flagged wrapped iff divisible by 3.
    let first_retained = lines - keep;
    assert_eq!(grid.scrollback_wrapped[0], first_retained.is_multiple_of(3));
}

#[test]
fn cap_scrollback_follows_a_rotated_ring() {
    let cols = 80;
    let keep = MAX_PERSISTED_SCROLLBACK_CELLS / cols;
    let max = 2_000;
    let start = 737;
    let grid = cap_grid_scrollback(ring_snapshot(max, cols, start, max));

    assert_eq!(grid.scrollback_lines, keep);
    // Every retained slot must carry the marker of its logical line:
    // slot j holds logical line (max - keep + j).
    for (slot, expected_logical) in (max - keep..max).enumerate() {
        assert_eq!(
            grid.scrollback_cells[slot * cols].c,
            marker_cell(expected_logical).c,
            "slot {slot} must hold logical line {expected_logical}"
        );
    }
}

#[test]
fn cap_scrollback_evicts_and_clamps_zones_at_the_new_floor() {
    let cols = 80;
    let keep = MAX_PERSISTED_SCROLLBACK_CELLS / cols;
    let lines = keep + 750;
    let mut snapshot = ring_snapshot(lines, cols, 0, 10_000);
    let floor = lines - keep;
    let mut old =
        par_term_emu_core::zone::Zone::new(1, par_term_emu_core::zone::ZoneType::Output, 0, None);
    old.abs_row_end = floor.saturating_sub(1);
    let mut straddling = par_term_emu_core::zone::Zone::new(
        2,
        par_term_emu_core::zone::ZoneType::Output,
        floor - 10,
        None,
    );
    straddling.abs_row_end = floor + 10;
    let mut recent = par_term_emu_core::zone::Zone::new(
        3,
        par_term_emu_core::zone::ZoneType::Output,
        floor + 5,
        None,
    );
    recent.abs_row_end = floor + 20;
    snapshot.zones = vec![old, straddling, recent];

    let grid = cap_grid_scrollback(snapshot);
    let ids: Vec<usize> = grid.zones.iter().map(|z| z.id).collect();
    assert_eq!(ids, vec![2, 3], "the zone wholly below the floor is gone");
    assert_eq!(grid.zones[0].abs_row_start, floor, "straddler clamps");
    assert_eq!(grid.zones[1].abs_row_start, floor + 5, "recent zone intact");
}

#[test]
fn cap_scrollback_leaves_a_small_ring_untouched() {
    let cols = 80;
    let lines = 40;
    let snapshot = ring_snapshot(lines, cols, 0, 10_000);
    let cells = snapshot.scrollback_cells.clone();
    let grid = cap_grid_scrollback(snapshot);
    assert_eq!(grid.scrollback_lines, lines);
    assert_eq!(grid.scrollback_cells, cells, "under the cap, nothing moves");
}

/// The capture path applies the cap: a pane whose scrollback exceeds
/// the budget persists only the newest lines, while its in-memory
/// snapshot cache keeps the full history. The content pane runs
/// `sleep` so its process emits nothing beyond the fed lines.
#[cfg(unix)]
#[test]
fn persisted_state_caps_scrollback_to_the_newest_lines() {
    use crate::mux::SplitDirection;

    let mut tree = tree();
    let session = tree.new_session("main", 80, 24).unwrap();
    let window = tree.session(session).unwrap().windows[0];
    let first = tree.window(window).unwrap().panes()[0];
    let quiet = tree
        .split_pane(first, SplitDirection::Vertical, 0.5, Some("sleep 60"))
        .unwrap();
    let terminal = tree.pane(quiet).unwrap().terminal();
    // The split pane's width is whatever the layout gave it; the line
    // budget is derived from the actual pane, not assumed.
    let (cols, _rows) = terminal.read().size();
    let keep = MAX_PERSISTED_SCROLLBACK_CELLS / cols;
    {
        let mut guard = terminal.write();
        for i in 0..keep + 100 {
            guard.process(format!("L{i:06}\r\n").as_bytes());
        }
    }
    let state = tree.to_persist_state();
    // Panes serialize in layout order; the split pane is the second.
    let pane = &state.sessions[0].windows[0].panes[1];
    assert_eq!(
        pane.terminal.grid.scrollback_lines, keep,
        "persisted scrollback is capped to the budget"
    );
    let persisted_text: String = pane
        .terminal
        .grid
        .scrollback_cells
        .iter()
        .map(|c| c.c)
        .collect();
    // The last `rows` fed lines sit on the visible screen; keep+50 is
    // well inside the retained window, L000000 well below it.
    assert!(
        persisted_text.contains(&format!("L{:06}", keep + 50)),
        "the retained window is the newest content"
    );
    assert!(
        !persisted_text.contains("L000000"),
        "the oldest lines are dropped from the persisted form"
    );
    // The pane's in-memory cache is uncapped: the next capture still
    // sees the full history.
    let full = tree.pane(quiet).unwrap().persisted_snapshot();
    assert!(
        full.grid.scrollback_lines > keep,
        "the in-memory snapshot keeps the pane's full history ({})",
        full.grid.scrollback_lines
    );
}
