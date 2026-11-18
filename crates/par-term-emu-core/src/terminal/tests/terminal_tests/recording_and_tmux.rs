//! Recording, asciicast export, and tmux control mode.

use super::*;

#[test]
fn test_join_wrapped_lines() {
    let mut term = Terminal::new(20, 24);
    term.process(b"This will wrap around");

    // The line should wrap
    let joined = term.join_wrapped_lines(0);
    assert!(joined.is_some());
    if let Some(j) = joined {
        assert!(j.lines_joined >= 1);
        assert!(!j.text.is_empty());
    }
}

#[test]
fn test_recording_session() {
    let mut term = Terminal::new(80, 24);
    term.start_recording(None);

    term.process(b"echo test");
    term.process(b"\r\n");

    let session = term.stop_recording().expect("recording returns a session");
    // process() records Output events while recording is active
    assert!(
        !session.events.is_empty(),
        "the processed bytes were recorded"
    );
    assert!(session
        .events
        .iter()
        .all(|e| e.event_type == RecordingEventType::Output));
    assert_eq!(session.initial_size, (80, 24));
}

#[test]
fn test_export_asciicast() {
    let mut term = Terminal::new(80, 24);
    term.start_recording(None);
    term.process(b"test\r\n");

    let session = term.stop_recording().unwrap();
    let asciicast = term.export_asciicast(&session);

    assert!(asciicast.contains("version"));
    assert!(asciicast.contains("width"));
    assert!(asciicast.contains("height"));
}

/// v3 output parses per the spec: header first with version 3 and a nested
/// `term` object, then NDJSON event lines with relative intervals.
#[test]
fn test_export_asciicast_v3_shape_and_intervals() {
    let mut term = Terminal::new(80, 24);
    term.start_recording(None);
    term.process(b"line one\r\n");
    term.process(b"line two\r\n");

    let session = term.stop_recording().unwrap();
    let cast = term.export_asciicast_v3(&session);
    let mut lines = cast.lines();

    let header: serde_json::Value =
        serde_json::from_str(lines.next().expect("header line")).expect("header json");
    assert_eq!(header["version"], 3);
    assert_eq!(header["term"]["cols"], 80);
    assert_eq!(header["term"]["rows"], 24);

    let mut count = 0;
    for line in lines {
        let v: serde_json::Value = serde_json::from_str(line).expect("event json");
        assert!(v.is_array(), "event lines are 3-element arrays");
        assert!(v[0].is_number(), "v3 intervals are numeric seconds");
        assert_eq!(v[1], "o");
        count += 1;
    }
    assert!(count >= 2, "both output chunks recorded, got {}", count);
}

/// Graphics present in the store are exported as `g` events with base64
/// RGBA data; scrollback-promoted graphics carry `scrollback: true`.
#[test]
fn test_export_asciicast_v3_graphics_events() {
    use crate::graphics::{GraphicProtocol, TerminalGraphic};

    let mut term = Terminal::new(80, 24);
    term.start_recording(None);
    term.process(b"hello\r\n");

    // Live placement (sixel-style, pixels in store) parked low on screen so
    // the scrollback promotion below leaves it visible
    let live = TerminalGraphic::new(7, GraphicProtocol::Sixel, (0, 20), 4, 4, vec![64u8; 64]);
    term.graphics.graphics_store.add_graphic(live);

    // Scrollback-promoted graphic: scroll it off the top like normal output
    let scrolled = TerminalGraphic::new(8, GraphicProtocol::Kitty, (0, 0), 4, 4, vec![64u8; 64]);
    term.graphics.graphics_store.add_graphic(scrolled);
    // The 4px image spans 2 rows at the default cell size; 3 lines take it
    // fully into scrollback.
    term.graphics
        .graphics_store
        .adjust_for_scroll_up_with_scrollback(3, 0, 23, 0);

    let session = term.stop_recording().unwrap();
    let cast = term.export_asciicast_v3(&session);

    let mut graphics_events = Vec::new();
    for line in cast.lines().skip(1) {
        let v: serde_json::Value = serde_json::from_str(line).expect("event json");
        if v[1] == "g" {
            graphics_events.push(v);
        }
    }
    assert_eq!(
        graphics_events.len(),
        2,
        "live + scrollback graphics exported"
    );

    let live_ev = &graphics_events[0];
    assert_eq!(live_ev[2]["protocol"], "sixel");
    assert_eq!(live_ev[2]["scrollback"], false);
    assert!(
        live_ev[2]["data"]
            .as_str()
            .map(|d| !d.is_empty())
            .unwrap_or(false),
        "base64 RGBA payload present"
    );

    let scrolled_ev = &graphics_events[1];
    assert_eq!(scrolled_ev[2]["protocol"], "kitty");
    assert_eq!(scrolled_ev[2]["scrollback"], true);
}

/// v3 resize data is the `"COLSxROWS"` string form.
#[test]
fn test_export_asciicast_v3_resize_string() {
    let mut term = Terminal::new(80, 24);
    term.start_recording(None);
    term.record_resize(120, 40);

    let session = term
        .get_recording_session()
        .expect("recording session should exist");
    let cast = term.export_asciicast_v3(session);
    let mut lines = cast.lines();
    let _header = lines.next();
    let resize: serde_json::Value =
        serde_json::from_str(lines.next().expect("resize event")).expect("json");
    assert_eq!(resize[1], "r");
    assert_eq!(resize[2], "120x40");
}

/// The v2 export is byte-shape compatible: absolute timestamps, `[cols, rows]`
/// resize arrays, and no graphics events (unchanged shipped behavior).
#[test]
fn test_export_asciicast_v2_unchanged_shape() {
    let mut term = Terminal::new(80, 24);
    term.start_recording(None);
    term.record_resize(120, 40);
    term.process(b"abc");

    let session = term.stop_recording().unwrap();
    let cast = term.export_asciicast(&session);
    let mut lines = cast.lines();

    let header: serde_json::Value =
        serde_json::from_str(lines.next().expect("header")).expect("json");
    assert_eq!(header["version"], 2);
    assert!(header["width"].is_number(), "v2 width stays flat");
    assert!(header.get("term").is_none(), "v2 has no nested term object");

    let resize: serde_json::Value =
        serde_json::from_str(lines.next().expect("resize")).expect("json");
    assert_eq!(resize[1], "r");
    assert_eq!(
        resize[2], 120,
        "v2 resize keeps the [cols, rows] array form"
    );
    assert_eq!(resize[3], 40);

    for line in lines {
        let v: serde_json::Value = serde_json::from_str(line).expect("json");
        assert_ne!(v[1], "g", "v2 export drops graphics entirely");
    }
}

#[test]
fn test_export_json() {
    let mut term = Terminal::new(80, 24);
    term.start_recording(None);
    term.process(b"test\r\n");

    let session = term.stop_recording().unwrap();
    let json = term.export_json(&session);

    assert!(json.contains("{"));
    assert!(json.contains("events") || json.contains("test"));
}

// =====================================================================
// Tmux Control Mode Tests
// =====================================================================

#[test]
fn test_tmux_control_mode_basic() {
    let mut term = Terminal::new(80, 24);
    assert!(!term.is_tmux_control_mode());

    term.set_tmux_control_mode(true);
    assert!(term.is_tmux_control_mode());

    term.set_tmux_control_mode(false);
    assert!(!term.is_tmux_control_mode());
}

#[test]
fn test_tmux_control_mode_suppresses_raw_protocol() {
    use crate::tmux_control::TmuxNotification;

    let mut term = Terminal::new(80, 24);
    term.set_tmux_control_mode(true);

    // Send tmux control protocol data
    term.process(b"%begin 1234567890 1\n");
    term.process(b"%output %1 Hello World\n");
    term.process(b"%end 1234567890 1\n");

    // The terminal should NOT display the raw protocol
    let content = term.content();
    assert!(
        !content.contains("%begin"),
        "Raw %begin should not be displayed"
    );
    assert!(
        !content.contains("%output"),
        "Raw %output should not be displayed"
    );
    assert!(
        !content.contains("%end"),
        "Raw %end should not be displayed"
    );

    // But notifications should be generated
    let notifications = term.drain_tmux_notifications();
    assert!(
        notifications.len() >= 2,
        "Should have at least Begin and Output notifications"
    );

    // Find and verify the Output notification
    let output_notif = notifications
        .iter()
        .find(|n| matches!(n, TmuxNotification::Output { .. }));
    assert!(output_notif.is_some(), "Should have Output notification");

    if let Some(TmuxNotification::Output { pane_id, data }) = output_notif {
        assert_eq!(pane_id, "%1");
        assert_eq!(data, b"Hello World");
    }
}

#[test]
fn test_tmux_auto_detect_enabled_with_control_mode() {
    let mut term = Terminal::new(80, 24);
    assert!(!term.is_tmux_auto_detect());

    // Enabling control mode should also enable auto-detect
    term.set_tmux_control_mode(true);
    assert!(term.is_tmux_auto_detect());
}

#[test]
fn test_tmux_auto_detect_handles_race_condition() {
    use crate::tmux_control::TmuxNotification;

    let mut term = Terminal::new(80, 24);

    // Enable auto-detect (simulating: user called set_tmux_control_mode(true)
    // but control_mode flag hasn't been checked yet due to race)
    term.set_tmux_auto_detect(true);
    // Note: control_mode is still false here

    // Data arrives with shell prompt followed by tmux protocol
    term.process(b"$ tmux -CC\n%begin 1234567890 1\n%output %1 Test\n");

    // The shell prompt should be displayed
    let content = term.content();
    assert!(
        content.contains("$ tmux -CC"),
        "Shell prompt should be displayed"
    );

    // But tmux protocol should NOT be displayed
    assert!(
        !content.contains("%begin"),
        "Raw %begin should not be displayed"
    );
    assert!(
        !content.contains("%output"),
        "Raw %output should not be displayed"
    );

    // Control mode should have auto-enabled
    assert!(
        term.is_tmux_control_mode(),
        "Control mode should be auto-enabled"
    );

    // Notifications should be generated
    let notifications = term.drain_tmux_notifications();
    assert!(!notifications.is_empty(), "Should have notifications");

    // Verify we got the Output notification
    let has_output = notifications
        .iter()
        .any(|n| matches!(n, TmuxNotification::Output { .. }));
    assert!(has_output, "Should have Output notification");
}

#[test]
fn test_tmux_terminal_output_still_displayed() {
    let mut term = Terminal::new(80, 24);
    term.set_tmux_control_mode(true);

    // In control mode, non-% lines should still be displayed
    // (this is for edge cases where regular output appears in control mode)
    term.process(b"regular terminal output\n");

    // Note: In strict control mode, this might be treated differently,
    // but currently non-% lines in control mode are passed through as TerminalOutput
    let content = term.content();
    assert!(
        content.contains("regular terminal output"),
        "Non-protocol lines should be displayed as terminal output"
    );
}
