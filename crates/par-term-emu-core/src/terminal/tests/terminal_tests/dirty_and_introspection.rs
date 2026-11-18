//! Dirty tracking, event caps, introspection, and bulk reads.

use super::*;

// ========== Tests for New TUI App Features ==========

#[test]
fn test_dirty_region_tracking() {
    let mut term = Terminal::new(80, 24);

    // Initially no dirty rows
    assert!(term.get_dirty_rows().is_empty());
    assert_eq!(term.get_dirty_region(), None);

    // Write to first row
    term.process(b"Hello");
    let dirty = term.get_dirty_rows();
    assert!(dirty.contains(&0));
    assert_eq!(term.get_dirty_region(), Some((0, 0, 0, 79)));

    // Write to another row
    term.process(b"\n\n");
    term.process(b"World");
    let dirty = term.get_dirty_rows();
    assert!(dirty.len() >= 2);

    // Mark clean and verify
    term.mark_clean();
    assert!(term.get_dirty_rows().is_empty());
    assert_eq!(term.get_dirty_region(), None);

    // Manual dirty marking
    term.mark_row_dirty(10);
    assert_eq!(term.get_dirty_rows(), vec![10]);
}

/// QA-150: every mutator honors the damage contract — mark_clean, apply
/// one sequence to a prefilled screen, assert the exact dirty row set.
#[test]
fn test_ich_dch_mark_cursor_row_dirty() {
    let mut term = Terminal::new(80, 24);
    term.process(b"prefill\r\nprefill\r\nprefill\r\nprefill\r\nprefill\r\nprefill");
    term.process(b"\x1b[6;10H"); // park the cursor on row 5 (0-indexed)
    term.mark_clean();

    term.process(b"\x1b[3@"); // ICH
    assert_eq!(term.get_dirty_rows(), vec![5], "ICH marks the cursor row");

    term.mark_clean();
    term.process(b"\x1b[2P"); // DCH
    assert_eq!(term.get_dirty_rows(), vec![5], "DCH marks the cursor row");
}

#[test]
fn test_rectangle_ops_mark_their_rows_dirty() {
    let mut term = Terminal::new(80, 24);
    for _ in 0..6 {
        term.process(b"prefill row content padding padding\r\n");
    }
    term.mark_clean();

    // DECFRA: fill 'A' rows 1-3, cols 1-5 (1-indexed).
    term.process(b"\x1b[65;1;1;3;5$x");
    assert_eq!(
        term.get_dirty_rows(),
        vec![0, 1, 2],
        "DECFRA marks top..=bottom"
    );

    term.mark_clean();
    // DECERA: erase rows 1-3, cols 1-5.
    term.process(b"\x1b[1;1;3;5$z");
    assert_eq!(
        term.get_dirty_rows(),
        vec![0, 1, 2],
        "DECERA marks top..=bottom"
    );

    term.mark_clean();
    // DECCRA: copy rows 1-3 cols 1-5 to destination row 10 — the
    // destination rows change, the unchanged source rows must NOT be
    // marked (marking the source is the classic false-dirty bug).
    term.process(b"\x1b[1;1;3;5;1;10;1$v");
    assert_eq!(
        term.get_dirty_rows(),
        vec![9, 10, 11],
        "DECCRA marks the destination rows only"
    );

    term.mark_clean();
    // DECCARA: change attributes rows 1-2, cols 1-5.
    term.process(b"\x1b[1;1;2;5;1$r");
    assert_eq!(
        term.get_dirty_rows(),
        vec![0, 1],
        "DECCARA marks top..=bottom"
    );

    term.mark_clean();
    // DECRARA: reverse attributes rows 1-2, cols 1-5.
    term.process(b"\x1b[1;1;2;5;7$t");
    assert_eq!(
        term.get_dirty_rows(),
        vec![0, 1],
        "DECRARA marks top..=bottom"
    );
}

#[test]
fn test_ris_marks_every_row_dirty() {
    let mut term = Terminal::new(80, 24);
    term.process(b"content");
    term.mark_clean();
    term.process(b"\x1bc");
    assert_eq!(
        term.get_dirty_rows(),
        (0..24).collect::<Vec<_>>(),
        "RIS marks every screen row"
    );
}

#[test]
fn test_snapshot_restore_marks_every_row_dirty() {
    let mut term = Terminal::new(80, 24);
    term.process(b"before");
    let snap = term.capture_snapshot();
    term.mark_clean();

    term.restore_from_snapshot(snap.clone());
    assert_eq!(
        term.get_dirty_rows(),
        (0..24).collect::<Vec<_>>(),
        "restore_from_snapshot marks every row"
    );

    term.mark_clean();
    term.restore_for_new_process(snap);
    assert_eq!(
        term.get_dirty_rows(),
        (0..24).collect::<Vec<_>>(),
        "restore_for_new_process marks every row"
    );
}

#[test]
fn test_bell_events() {
    let mut term = Terminal::new(80, 24);

    // Initially no bell events
    assert_eq!(term.drain_bell_events().len(), 0);

    // Trigger bell
    term.process(b"\x07");
    let events = term.drain_bell_events();
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        BellEvent::WarningBell(_) | BellEvent::VisualBell
    ));

    // Drain clears the buffer
    assert_eq!(term.drain_bell_events().len(), 0);

    // Multiple bells
    term.process(b"\x07\x07\x07");
    assert_eq!(term.drain_bell_events().len(), 3);
}

/// SEC-207: a BEL flood into a terminal nobody drains keeps at most
/// MAX_BELL_EVENTS pending, while bell_count still counts every bell.
#[test]
fn test_bell_events_are_capped_and_bell_count_keeps_the_total() {
    use crate::terminal::MAX_BELL_EVENTS;
    let mut term = Terminal::new(80, 24);
    term.process(&[0x07; 10_000]);
    assert_eq!(term.drain_bell_events().len(), MAX_BELL_EVENTS);
    assert_eq!(term.bell_count(), 10_000);
}

/// SEC-207: a device-query flood into a terminal nobody drains keeps at
/// most MAX_RESPONSE_BYTES pending — chunked or in one `process` call —
/// and the replies past the cap are the ones dropped, so the queue still
/// starts on a reply boundary.
#[test]
fn test_response_buffer_is_capped_without_a_drain() {
    use crate::terminal::MAX_RESPONSE_BYTES;
    let flood = b"\x1b[6n".repeat(20_000);
    let mut chunked = Terminal::new(80, 24);
    for chunk in flood.chunks(4096) {
        chunked.process(chunk);
    }
    let mut whole = Terminal::new(80, 24);
    whole.process(&flood);
    for term in [&mut chunked, &mut whole] {
        let responses = term.drain_responses();
        assert!(
            responses.len() <= MAX_RESPONSE_BYTES,
            "{} bytes pending",
            responses.len()
        );
        assert!(
            responses.starts_with(b"\x1b[1;1R"),
            "kept replies are whole"
        );
    }
}

/// SEC-207: the cap bounds what PARSING adds, never embedder pushes. An
/// iTerm2 upload reply larger than the cap stays intact through a later
/// parse pass that also queues a reply.
#[test]
fn test_response_cap_never_truncates_an_embedder_upload() {
    use crate::terminal::MAX_RESPONSE_BYTES;
    use base64::Engine;
    let mut term = Terminal::new(80, 24);
    let data = vec![b'a'; 100_000];
    term.send_upload_data(&data);
    term.process(b"x\x1b[6n");
    let encoded = base64::engine::general_purpose::STANDARD.encode(&data);
    let expected = format!("ok\n{encoded}\n\n").into_bytes();
    assert!(expected.len() > MAX_RESPONSE_BYTES);
    // The upload survives whole; the DSR reply that pass generated is the
    // one dropped, since the queue was already past the cap.
    assert_eq!(term.drain_responses(), expected);
    // Once drained, parsing answers normally again.
    term.process(b"\x1b[6n");
    assert_eq!(term.drain_responses(), b"\x1b[1;2R");
}

#[test]
fn test_mode_introspection() {
    let mut term = Terminal::new(80, 24);

    // Check default modes
    assert!(term.auto_wrap_mode());
    assert!(!term.origin_mode());
    assert!(!term.application_cursor());
    assert_eq!(term.scroll_region(), (0, 23));
    assert_eq!(term.left_right_margins(), (0, 79));

    // Enable origin mode
    term.process(b"\x1b[?6h");
    assert!(term.origin_mode());

    // Enable application cursor
    term.process(b"\x1b[?1h");
    assert!(term.application_cursor());

    // Disable auto wrap
    term.process(b"\x1b[?7l");
    assert!(!term.auto_wrap_mode());

    // Set scroll region
    term.process(b"\x1b[5;20r");
    assert_eq!(term.scroll_region(), (4, 19)); // 0-indexed

    // Enable left/right margins
    term.process(b"\x1b[?69h");
    term.process(b"\x1b[10;70s");
    let (left, right) = term.left_right_margins();
    assert_eq!(left, 9); // 0-indexed
    assert_eq!(right, 69); // 0-indexed
}

#[test]
fn test_palette_access() {
    let mut term = Terminal::new(80, 24);

    // Get default ANSI colors
    let palette = term.get_ansi_palette();
    assert_eq!(palette.len(), 16);

    // Get individual color
    assert!(term.get_ansi_color(0).is_some());
    assert!(term.get_ansi_color(15).is_some());
    assert!(term.get_ansi_color(16).is_none());

    // Modify a color
    term.process(b"\x1b]4;1;rgb:ff/00/00\x1b\\");
    let red = term.get_ansi_color(1).unwrap();
    assert_eq!(red, Color::Rgb(255, 0, 0));

    // Get cursor/link colors
    let _cursor_color = term.get_cursor_color();
    let _link_color = term.get_link_color();
    let _sel_bg = term.get_selection_bg_color();
    let _sel_fg = term.get_selection_fg_color();
}

#[test]
fn test_tab_stop_access() {
    let mut term = Terminal::new(80, 24);

    // Default tab stops at every 8 columns
    let stops = term.get_tab_stops();
    assert!(stops.contains(&0));
    assert!(stops.contains(&8));
    assert!(stops.contains(&16));

    // Clear a tab stop
    term.clear_tab_stop(8);
    let stops = term.get_tab_stops();
    assert!(!stops.contains(&8));
    assert!(stops.contains(&16));

    // Set a custom tab stop
    term.set_tab_stop(5);
    let stops = term.get_tab_stops();
    assert!(stops.contains(&5));

    // Clear all tab stops
    term.clear_all_tab_stops();
    assert!(term.get_tab_stops().is_empty());
}

#[test]
fn test_hyperlink_enumeration() {
    let mut term = Terminal::new(80, 24);

    // Initially no hyperlinks
    assert_eq!(term.get_all_hyperlinks().len(), 0);

    // Add a hyperlink
    term.process(b"\x1b]8;;https://example.com\x1b\\Link\x1b]8;;\x1b\\");
    let links = term.get_all_hyperlinks();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].url, "https://example.com");
    assert!(!links[0].positions.is_empty());

    // Add another hyperlink
    term.process(b" \x1b]8;;https://test.com\x1b\\Test\x1b]8;;\x1b\\");
    let links = term.get_all_hyperlinks();
    assert_eq!(links.len(), 2);
}

#[test]
fn test_bulk_read_operations() {
    let mut term = Terminal::new(10, 5);
    term.process(b"12345\n67890\nABCDE");

    // Get visible region
    let visible = term.get_rectangle(0, 0, 4, 9);
    assert_eq!(visible.len(), 5); // 5 rows
    assert_eq!(visible[0].len(), 10); // 10 cols

    // Get row range
    let rows = term.get_row_range(0, 2);
    assert_eq!(rows.len(), 2);

    // Get rectangle
    let rect = term.get_rectangle(0, 0, 2, 4);
    assert_eq!(rect.len(), 3); // 3 rows
    assert_eq!(rect[0].len(), 5); // 5 cols
}

#[test]
fn test_rectangle_operations() {
    let mut term = Terminal::new(20, 10);

    // Fill rectangle
    term.fill_rectangle(2, 2, 4, 6, 'X');
    let visible = term.get_rectangle(0, 0, 9, 19);
    assert_eq!(visible[2][2].c, 'X');
    assert_eq!(visible[3][4].c, 'X');
    assert_eq!(visible[4][6].c, 'X');

    // Erase rectangle
    term.erase_rectangle(2, 2, 4, 6);
    let visible = term.get_rectangle(0, 0, 9, 19);
    assert_eq!(visible[2][2].c, ' ');
    assert_eq!(visible[3][4].c, ' ');
}

#[test]
fn test_enhanced_stats() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Test");
    term.process(b"\x1b]8;;https://example.com\x1b\\Link\x1b]8;;\x1b\\");

    let stats = term.get_stats();
    assert_eq!(stats.cols, 80);
    assert_eq!(stats.rows, 24);
    assert!(stats.hyperlink_count > 0);
    assert!(
        stats.estimated_memory_bytes
            >= stats.total_cells * std::mem::size_of::<crate::cell::Cell>()
    );
    assert!(stats.hyperlink_memory_bytes >= "https://example.com".len());
    assert_eq!(stats.color_stack_depth, 0);
    assert_eq!(stats.title_stack_depth, 0);
    assert_eq!(stats.keyboard_stack_depth, 0);
    assert!(stats.dirty_row_count > 0);
}

#[test]
fn test_terminal_events() {
    let mut term = Terminal::new(80, 24);

    // Initially no events
    assert_eq!(term.poll_events().len(), 0);

    // Trigger bell creates event
    term.process(b"\x07");
    let events = term.poll_events();
    assert!(events
        .iter()
        .any(|e| matches!(e, TerminalEvent::BellRang(_))));

    // Drain clears events
    assert_eq!(term.poll_events().len(), 0);
}

#[test]
fn test_dirty_tracking_on_scroll() {
    let mut term = Terminal::new(80, 24);
    term.mark_clean();

    // Fill screen and scroll
    for _ in 0..30 {
        term.process(b"Line\n");
    }

    // Should have dirty rows from scrolling
    let dirty = term.get_dirty_rows();
    assert!(!dirty.is_empty());
}

#[test]
fn test_asciicast_resize_schema() {
    let mut term = Terminal::new(80, 24);
    term.start_recording(None);
    term.record_resize(120, 40);

    let session = term
        .get_recording_session()
        .expect("recording session should exist");
    let asciicast = term.export_asciicast(session);
    let mut lines = asciicast.lines();
    lines.next().expect("header");
    let resize_line = lines.next().expect("resize event");
    let value: serde_json::Value = serde_json::from_str(resize_line).expect("valid json");
    assert_eq!(value[1], "r");
    assert_eq!(value[2], 120);
    assert_eq!(value[3], 40);
}

#[test]
fn test_resize_records_event_when_recording() {
    let mut term = Terminal::new(80, 24);
    term.start_recording(None);
    term.resize(90, 30);

    let session = term
        .get_recording_session()
        .expect("recording session should exist");
    assert!(session.events.iter().any(|event| {
        matches!(event.event_type, RecordingEventType::Resize) && event.metadata == Some((90, 30))
    }));
}

#[test]
fn test_notification_queue_is_capped() {
    let mut term = Terminal::new(80, 24);
    term.set_max_notifications(2);

    term.process(b"\x1b]9;first\x1b\\");
    term.process(b"\x1b]9;second\x1b\\");
    term.process(b"\x1b]9;third\x1b\\");

    let notifications = term.take_notifications();
    assert_eq!(notifications.len(), 2);
    assert_eq!(notifications[0].message, "second");
    assert_eq!(notifications[1].message, "third");
}

#[test]
fn test_clipboard_sync_event_limits() {
    let mut term = Terminal::new(80, 24);
    term.set_max_clipboard_sync_events(2);
    term.set_max_clipboard_event_bytes(24);

    term.record_clipboard_sync(
        ClipboardTarget::Clipboard,
        ClipboardOperation::Set,
        Some("short".to_string()),
        false,
    );
    term.record_clipboard_sync(
        ClipboardTarget::Clipboard,
        ClipboardOperation::Set,
        Some("abcdefghijklmnopqrstuvwxyz".to_string()),
        false,
    );
    term.record_clipboard_sync(
        ClipboardTarget::Clipboard,
        ClipboardOperation::Set,
        Some("final".to_string()),
        false,
    );

    let events = term.get_clipboard_sync_events();
    assert_eq!(events.len(), 2);
    let middle = &events[0].content;
    assert!(middle.contains("[truncated]"));
    assert_eq!(events[1].content, "final");
}

#[test]
fn test_silence_check_does_not_fire_immediately() {
    let mut term = Terminal::new(80, 24);
    term.notifications_state.notification_config.silence_enabled = true;
    term.notifications_state
        .notification_config
        .silence_threshold = 1;
    term.check_silence();
    assert!(!term.has_notifications());
}
