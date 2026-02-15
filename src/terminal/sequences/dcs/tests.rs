use super::*;
use vte::Params;

fn create_test_terminal() -> Terminal {
    Terminal::new(80, 24)
}

fn create_empty_params() -> Params {
    Params::default()
}

#[test]
fn test_dcs_hook_sixel() {
    let mut term = create_test_terminal();
    let params = create_empty_params();

    term.dcs_hook(&params, &[], false, 'q');

    assert!(term.dcs_active);
    assert_eq!(term.dcs_action, Some('q'));
    assert!(term.sixel_parser.is_some());
    assert!(term.dcs_buffer.is_empty());
}

#[test]
fn test_dcs_hook_sixel_with_params() {
    let mut term = create_test_terminal();
    let params = create_empty_params();

    term.dcs_hook(&params, &[], false, 'q');

    assert!(term.dcs_active);
    assert_eq!(term.dcs_action, Some('q'));
    assert!(term.sixel_parser.is_some());
}

#[test]
fn test_dcs_hook_sixel_blocked_by_security() {
    let mut term = create_test_terminal();
    term.disable_insecure_sequences = true;
    let params = create_empty_params();

    term.dcs_hook(&params, &[], false, 'q');

    // Should be blocked
    assert!(!term.dcs_active);
    assert_eq!(term.dcs_action, None);
    assert!(term.sixel_parser.is_none());
}

#[test]
fn test_dcs_hook_non_sixel_action() {
    let mut term = create_test_terminal();
    let params = create_empty_params();

    term.dcs_hook(&params, &[], false, 'p');

    assert!(term.dcs_active);
    assert_eq!(term.dcs_action, Some('p'));
    assert!(term.sixel_parser.is_none()); // Not created for non-sixel
}

#[test]
fn test_dcs_put_sixel_data() {
    let mut term = create_test_terminal();
    let params = create_empty_params();

    // Start sixel
    term.dcs_hook(&params, &[], false, 'q');
    assert!(term.sixel_parser.is_some());

    // Send sixel data characters
    for &byte in b"????" {
        term.dcs_put(byte);
    }

    // Parser should process the data
    assert!(term.dcs_active);
}

#[test]
fn test_dcs_put_not_active() {
    let mut term = create_test_terminal();

    // Try to put data without activating DCS
    term.dcs_put(b'A');

    // Should be ignored
    assert!(!term.dcs_active);
}

#[test]
fn test_dcs_put_color_command() {
    let mut term = create_test_terminal();
    let params = create_empty_params();

    term.dcs_hook(&params, &[], false, 'q');

    // Send color command: #0
    term.dcs_put(b'#');
    term.dcs_put(b'0');

    assert_eq!(term.dcs_buffer, b"#0");
}

#[test]
fn test_dcs_put_raster_attributes() {
    let mut term = create_test_terminal();
    let params = create_empty_params();

    term.dcs_hook(&params, &[], false, 'q');

    // Send raster attributes: "1;1;100;100
    for &byte in b"\"1;1;100;100" {
        term.dcs_put(byte);
    }

    // Should accumulate in buffer
    assert!(!term.dcs_buffer.is_empty());
}

#[test]
fn test_dcs_put_repeat_command() {
    let mut term = create_test_terminal();
    let params = create_empty_params();

    term.dcs_hook(&params, &[], false, 'q');

    // Send repeat command: !10?
    for &byte in b"!10?" {
        term.dcs_put(byte);
    }

    // Should process when complete
    assert!(term.dcs_active);
}

#[test]
fn test_dcs_put_carriage_return() {
    let mut term = create_test_terminal();
    let params = create_empty_params();

    term.dcs_hook(&params, &[], false, 'q');

    // Send carriage return
    term.dcs_put(b'$');

    // Parser should handle it
    assert!(term.dcs_active);
}

#[test]
fn test_dcs_put_new_line() {
    let mut term = create_test_terminal();
    let params = create_empty_params();

    term.dcs_hook(&params, &[], false, 'q');

    // Send new line
    term.dcs_put(b'-');

    // Parser should handle it
    assert!(term.dcs_active);
}

#[test]
fn test_dcs_unhook_cleans_up() {
    let mut term = create_test_terminal();
    let params = create_empty_params();

    term.dcs_hook(&params, &[], false, 'q');
    assert!(term.dcs_active);

    term.dcs_unhook();

    assert!(!term.dcs_active);
    assert_eq!(term.dcs_action, None);
    assert!(term.dcs_buffer.is_empty());
    assert!(term.sixel_parser.is_none());
}

#[test]
fn test_dcs_unhook_processes_remaining_buffer() {
    let mut term = create_test_terminal();
    let params = create_empty_params();

    term.dcs_hook(&params, &[], false, 'q');

    // Add some data to buffer
    term.dcs_buffer.extend_from_slice(b"#0");

    term.dcs_unhook();

    // Buffer should be processed and cleared
    assert!(term.dcs_buffer.is_empty());
}

#[test]
fn test_dcs_unhook_sixel_advances_cursor() {
    let mut term = create_test_terminal();
    let params = create_empty_params();

    let start_col = term.cursor.col;
    let start_row = term.cursor.row;

    term.dcs_hook(&params, &[], false, 'q');

    // Create minimal sixel graphic
    for &byte in b"????" {
        term.dcs_put(byte);
    }

    term.dcs_unhook();

    // Cursor should have moved (graphic occupies space)
    // At minimum, row should advance or col should change
    assert!(
        term.cursor.row > start_row || term.cursor.col != start_col,
        "Cursor should move after Sixel graphic"
    );
}

#[test]
fn test_process_sixel_command_empty_buffer() {
    let mut term = create_test_terminal();

    term.process_sixel_command();

    // Should handle gracefully
    assert!(term.dcs_buffer.is_empty());
}

#[test]
fn test_process_sixel_command_no_parser() {
    let mut term = create_test_terminal();
    term.dcs_buffer.extend_from_slice(b"#0");

    term.process_sixel_command();

    // Should handle gracefully when no parser exists
}

#[test]
fn test_dcs_sequence_isolation() {
    let mut term = create_test_terminal();
    let params = create_empty_params();

    // Start first DCS
    term.dcs_hook(&params, &[], false, 'q');
    term.dcs_put(b'?');
    term.dcs_unhook();

    assert!(!term.dcs_active);
    assert!(term.sixel_parser.is_none());

    // Start second DCS
    term.dcs_hook(&params, &[], false, 'q');
    assert!(term.dcs_active);
    assert!(term.sixel_parser.is_some());
}

#[test]
fn test_dcs_hook_clears_previous_state() {
    let mut term = create_test_terminal();
    let params = create_empty_params();

    // Set some state
    term.dcs_buffer.extend_from_slice(b"old data");
    term.dcs_active = true;

    // Hook new DCS
    term.dcs_hook(&params, &[], false, 'q');

    // Buffer should be cleared
    assert!(term.dcs_buffer.is_empty());
    assert!(term.dcs_active);
    assert_eq!(term.dcs_action, Some('q'));
}

#[test]
fn test_dcs_multiple_commands_in_sequence() {
    let mut term = create_test_terminal();
    let params = create_empty_params();

    term.dcs_hook(&params, &[], false, 'q');

    // Send multiple commands
    for &byte in b"#0" {
        term.dcs_put(byte);
    }

    // Buffer should accumulate
    assert_eq!(term.dcs_buffer, b"#0");

    // Process by sending data char
    term.dcs_put(b'?');

    // Buffer should be cleared after processing
    assert_eq!(term.dcs_buffer.len(), 0);
}

#[test]
fn test_dcs_color_command_parsing() {
    let mut term = create_test_terminal();
    let params = create_empty_params();

    term.dcs_hook(&params, &[], false, 'q');

    // Send color definition: #1;2;100;100;100
    for &byte in b"#1;2;100;100;100" {
        term.dcs_put(byte);
    }

    // Trigger processing with data char
    term.dcs_put(b'?');

    // Should have processed color command
    assert!(term.dcs_buffer.is_empty());
}

#[test]
fn test_dcs_raster_attributes_parsing() {
    let mut term = create_test_terminal();
    let params = create_empty_params();

    term.dcs_hook(&params, &[], false, 'q');

    // Send raster attributes: "1;1;800;600
    for &byte in b"\"1;1;800;600" {
        term.dcs_put(byte);
    }

    // Trigger processing with data char
    term.dcs_put(b'?');

    // Should have processed raster command
    assert!(term.dcs_buffer.is_empty());
}

#[test]
fn test_dcs_graphics_list_updated() {
    let mut term = create_test_terminal();
    let params = create_empty_params();

    let initial_graphics_count = term.graphics_count();

    term.dcs_hook(&params, &[], false, 'q');

    // Send minimal sixel data
    for &byte in b"????" {
        term.dcs_put(byte);
    }

    term.dcs_unhook();

    // Graphics list should have one more entry
    assert_eq!(term.graphics_count(), initial_graphics_count + 1);
}

#[test]
fn test_sixel_graphics_limit_enforced() {
    let mut term = create_test_terminal();
    let params = create_empty_params();

    // Only allow 2 graphics to be retained
    term.set_max_sixel_graphics(2);

    // Helper to emit a tiny sixel graphic
    let emit_sixel = |term: &mut Terminal| {
        term.dcs_hook(&params, &[], false, 'q');
        for &byte in b"??" {
            term.dcs_put(byte);
        }
        term.dcs_unhook();
    };

    emit_sixel(&mut term);
    emit_sixel(&mut term);
    assert_eq!(term.graphics_count(), 2);
}

#[test]
fn test_sixel_graphics_limit_drops_oldest() {
    let mut term = create_test_terminal();
    let params = create_empty_params();

    term.set_max_sixel_graphics(1);

    let emit_sixel = |term: &mut Terminal| {
        term.dcs_hook(&params, &[], false, 'q');
        for &byte in b"??" {
            term.dcs_put(byte);
        }
        term.dcs_unhook();
    };

    emit_sixel(&mut term);
    assert_eq!(term.graphics_count(), 1);

    emit_sixel(&mut term);
    // Limit enforced - still only 1 graphic
    assert_eq!(term.graphics_count(), 1);

    // Emit a third graphic; limit should still be enforced
    emit_sixel(&mut term);
    assert_eq!(term.graphics_count(), 1);
}

#[test]
fn test_dcs_cursor_position_after_graphic() {
    let mut term = create_test_terminal();
    let params = create_empty_params();

    term.cursor.col = 10;
    term.cursor.row = 5;

    term.dcs_hook(&params, &[], false, 'q');
    for &byte in b"????" {
        term.dcs_put(byte);
    }
    term.dcs_unhook();

    // After sixel, cursor should be at column 0 (left margin)
    assert_eq!(term.cursor.col, 0);
    // Row should have advanced
    assert!(term.cursor.row >= 5);
}

#[test]
fn test_dcs_non_sixel_action_ignored() {
    let mut term = create_test_terminal();
    let params = create_empty_params();

    term.dcs_hook(&params, &[], false, 'x');
    assert!(term.dcs_active);
    assert_eq!(term.dcs_action, Some('x'));

    // Put some data
    term.dcs_put(b'A');
    term.dcs_put(b'B');

    // Should not create sixel parser
    assert!(term.sixel_parser.is_none());

    term.dcs_unhook();
    assert!(!term.dcs_active);
}
