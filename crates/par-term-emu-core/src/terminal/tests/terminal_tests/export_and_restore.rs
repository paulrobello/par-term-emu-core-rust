//! HTML/ANSI export and screen-restore sequences.

use super::*;

#[test]
fn test_export_html_basic() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Hello World");

    let html = term.export_html(false);
    assert!(html.contains("Hello World"));
}

#[test]
fn test_export_html_with_styles() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[31mRed Text\x1b[0m");

    let html = term.export_html(true);
    assert!(html.contains("Red Text"));
    assert!(html.contains("style") || html.contains("class"));
}

#[test]
fn test_export_scrollback_plain() {
    let mut term = Terminal::with_scrollback(80, 5, 100);
    for i in 0..10 {
        term.process(format!("Line{}\r\n", i).as_bytes());
    }

    let export = term.export_scrollback(ExportFormat::Plain, None);
    // Should export some content
    assert!(!export.is_empty());
    // May contain Line content depending on scrollback state
    let has_lines = export.contains("Line") || export.len() > 10;
    assert!(has_lines);
}

#[test]
fn test_export_scrollback_with_limit() {
    let mut term = Terminal::with_scrollback(80, 5, 100);
    for i in 0..20 {
        term.process(format!("Line{}\r\n", i).as_bytes());
    }

    // 20 lines + the trailing blank row on a 5-row screen: Line0..Line15
    // scrolled off, Line15 directly above the screen.
    let lines: Vec<String> = term
        .export_scrollback(ExportFormat::Plain, Some(5))
        .lines()
        .map(|l| l.trim_end().to_string())
        .collect();
    assert_eq!(lines, ["Line15", "Line14", "Line13", "Line12", "Line11"]);
}

/// `max_lines` is a tail limit: `Some(n)` keeps the NEWEST `n` scrollback
/// rows in the same newest-first order as the full export, so it is always
/// the first `n` rows of the `None` export — in every format.
#[test]
fn export_scrollback_limit_keeps_the_newest_rows_in_every_format() {
    let mut term = Terminal::with_scrollback(20, 3, 100);
    for i in 0..12 {
        term.process(format!("\x1b[3{}mrow-{i:02}\x1b[0m\r\n", i % 7 + 1).as_bytes());
    }
    let len = term.grid().scrollback_len();
    assert!(len >= 8, "fixture scrolls enough history: {len}");

    let rows = |format: ExportFormat, max: Option<usize>| -> Vec<String> {
        let text = term.export_scrollback(format, max);
        let body = match format {
            ExportFormat::Html => text
                .strip_prefix("<pre>\n")
                .and_then(|t| t.strip_suffix("</pre>"))
                .expect("html framing")
                .to_string(),
            _ => text,
        };
        body.lines().map(str::to_string).collect()
    };

    for format in [ExportFormat::Plain, ExportFormat::Html, ExportFormat::Ansi] {
        let full = rows(format, None);
        assert_eq!(full.len(), len, "{format:?}");
        for n in [1, 3, len - 1] {
            assert_eq!(rows(format, Some(n)), full[..n], "{format:?} Some({n})");
        }
        assert_eq!(rows(format, Some(len + 10)), full, "{format:?} over-long");
        assert!(rows(format, Some(0)).is_empty(), "{format:?} Some(0)");
    }

    let newest = format!("row-{:02}", len - 1);
    let tail = term.export_scrollback(ExportFormat::Plain, Some(2));
    assert!(tail.contains(&newest), "newest row kept: {tail:?}");
    assert!(!tail.contains("row-00"), "oldest row dropped: {tail:?}");
    assert_eq!(
        term.export_scrollback(ExportFormat::Html, Some(0)),
        "<pre>\n</pre>"
    );
}

#[test]
fn test_export_scrollback_html_format() {
    let mut term = Terminal::with_scrollback(80, 5, 100);
    term.process(b"\x1b[31mRed\x1b[0m\r\nText");
    for _ in 0..5 {
        term.process(b"\r\n");
    }

    let export = term.export_scrollback(ExportFormat::Html, None);
    assert!(export.contains("Red"));
}

/// A 100-char line on an 80-col grid soft-wraps into two rows. The ansi
/// export joins them into ONE logical line, head row first, between its
/// true older and newer neighbors; Plain and Html never join, so they keep
/// one line per grid row (newest-first).
#[test]
fn export_scrollback_joins_a_wrapped_line_with_the_row_it_continues() {
    let mut term = Terminal::with_scrollback(80, 2, 100);
    let head = "A".repeat(80);
    let tail = "B".repeat(20);
    term.process(format!("older\r\n{head}{tail}\r\nnewer\r\n\r\n").as_bytes());
    let grid = term.grid();
    assert_eq!(grid.scrollback_len(), 4, "older, head, tail, newer");
    assert!(grid.is_scrollback_wrapped(1), "head row soft-wraps");
    assert!(!grid.is_scrollback_wrapped(2), "tail row ends the line");

    assert_eq!(
        term.export_scrollback(ExportFormat::Ansi, None),
        format!("newer\x1b[0m\n{head}{tail}\x1b[0m\nolder\x1b[0m\n")
    );
    // A cut inside the logical line keeps only its newest rows, terminated.
    assert_eq!(
        term.export_scrollback(ExportFormat::Ansi, Some(2)),
        format!("newer\x1b[0m\n{tail}\x1b[0m\n")
    );
    assert_eq!(
        term.export_scrollback(ExportFormat::Ansi, Some(3)),
        format!("newer\x1b[0m\n{head}{tail}\x1b[0m\n")
    );

    let plain = term.export_scrollback(ExportFormat::Plain, None);
    let plain_rows: Vec<&str> = plain.lines().map(str::trim_end).collect();
    assert_eq!(plain_rows, ["newer", tail.as_str(), head.as_str(), "older"]);
    let html = term.export_scrollback(ExportFormat::Html, None);
    let html_rows: Vec<&str> = html
        .strip_prefix("<pre>\n")
        .and_then(|t| t.strip_suffix("</pre>"))
        .expect("html framing")
        .lines()
        .map(str::trim_end)
        .collect();
    assert_eq!(html_rows, plain_rows);
}

/// The newest scrollback row soft-wraps into screen row 0: its continuation
/// is not history, so the ansi export still terminates it rather than
/// gluing it onto the older line that follows.
#[test]
fn export_scrollback_ansi_terminates_a_row_whose_continuation_is_on_screen() {
    let mut term = Terminal::with_scrollback(80, 2, 100);
    let head = "A".repeat(80);
    term.process(format!("older\r\n{head}BB\r\nnext").as_bytes());
    let grid = term.grid();
    assert_eq!(grid.scrollback_len(), 2, "older, head");
    assert!(grid.is_scrollback_wrapped(1), "head wraps onto the screen");

    assert_eq!(
        term.export_scrollback(ExportFormat::Ansi, None),
        format!("{head}\x1b[0m\nolder\x1b[0m\n")
    );
}

#[test]
fn test_export_scrollback_ansi_format() {
    let mut term = Terminal::with_scrollback(80, 5, 100);
    term.process(b"\x1b[31mRed\x1b[0m");
    for _ in 0..5 {
        term.process(b"\r\n");
    }

    let export = term.export_scrollback(ExportFormat::Ansi, None);
    // Ansi must preserve SGR (the old stub fell through to Plain)
    assert!(
        export.contains("\x1b[0;31"),
        "SGR missing from Ansi export: {export:?}"
    );
    assert_ne!(
        export,
        term.export_scrollback(ExportFormat::Plain, None),
        "Ansi export must not be identical to Plain"
    );
}

#[test]
fn screen_restore_sequence_round_trips_state() {
    // A full-screen-TUI-shaped stream: alt screen, scroll region, 256-color
    // and truecolor SGR, an attribute mix, wide chars, a combining char,
    // styled trailing blanks, then the input modes and a hidden, styled
    // cursor placed while origin mode is on.
    let mut src = Terminal::new(80, 24);
    src.process(b"\x1b[?1049h\x1b[2;6r");
    src.process(b"\x1b[1;5H\x1b[1;31mbold-red\x1b[0m");
    src.process(b"\x1b[3;1H\x1b[38;5;196midx-fg\x1b[0m");
    src.process(b"\x1b[3;10H\x1b[48;2;10;20;30mtrue-bg\x1b[0m");
    src.process(b"\x1b[4;1H\x1b[1;3;4;7;9mattrs\x1b[0m");
    src.process(b"\x1b[5;1H\xe5\xae\x89\xe5\xba\xb7");
    src.process(b"\x1b[6;1He\xcc\x81");
    src.process(b"\x1b[7;1H\x1b[44m   \x1b[0m");
    src.process(b"\x1b[?25l\x1b[4 q");
    src.process(b"\x1b[?1h\x1b[?2004h\x1b[?1004h");
    src.process(b"\x1b=");
    src.process(b"\x1b[?1000h\x1b[?1006h");
    src.process(b"\x1b[?6h");
    src.process(b"\x1b[3;5H");

    let restore = src.export_screen_restore_sequence();
    let mut dst = Terminal::new(80, 24);
    dst.process(restore.as_bytes());

    assert_eq!(src.alt_screen_active, dst.alt_screen_active);
    assert!(dst.alt_screen_active, "alt screen must be restored");
    for row in 0..24 {
        assert_eq!(
            src.active_grid().row(row),
            dst.active_grid().row(row),
            "grid row {row} differs after replay"
        );
    }
    assert_eq!(src.cursor.row, dst.cursor.row);
    assert_eq!(src.cursor.col, dst.cursor.col);
    assert_eq!(src.cursor.visible, dst.cursor.visible);
    assert!(!dst.cursor.visible, "hidden cursor must be restored");
    assert_eq!(src.cursor.style, dst.cursor.style);
    assert_eq!(src.modes.application_cursor, dst.modes.application_cursor);
    assert_eq!(src.modes.application_keypad, dst.modes.application_keypad);
    assert!(
        dst.modes.application_keypad,
        "application keypad mode must be restored"
    );
    assert_eq!(src.modes.bracketed_paste, dst.modes.bracketed_paste);
    assert_eq!(src.modes.mouse_mode, dst.modes.mouse_mode);
    assert_eq!(src.modes.mouse_encoding, dst.modes.mouse_encoding);
    assert_eq!(src.modes.origin_mode, dst.modes.origin_mode);
    assert!(dst.modes.origin_mode, "origin mode must be restored");
    assert_eq!(src.margins.scroll_region_top, dst.margins.scroll_region_top);
    assert_eq!(
        src.margins.scroll_region_bottom,
        dst.margins.scroll_region_bottom
    );
}

#[test]
fn screen_restore_sequence_replays_main_screen_scrollback() {
    // A reattached client pane starts empty; the restore stream must carry
    // the main screen's history so the client can scroll back through it.
    for lines in [8usize, 40] {
        let mut src = Terminal::with_scrollback(20, 5, 100);
        for i in 0..lines {
            src.process(format!("\x1b[3{}mline {i}\x1b[0m\r\n", i % 7 + 1).as_bytes());
        }
        src.process(b"\x1b[?1049h\x1b[2;2Htui");
        assert!(src.grid.scrollback_len() > 0, "{lines}: source has history");

        let restore = src.export_screen_restore_sequence();
        let mut dst = Terminal::with_scrollback(20, 5, 100);
        dst.process(restore.as_bytes());

        assert_eq!(
            src.grid.scrollback_len(),
            dst.grid.scrollback_len(),
            "{lines}: scrollback length"
        );
        for index in 0..src.grid.scrollback_len() {
            assert_eq!(
                src.grid.scrollback_line(index),
                dst.grid.scrollback_line(index),
                "{lines}: scrollback line {index} differs after replay"
            );
        }
        for row in 0..5 {
            assert_eq!(
                src.grid.row(row),
                dst.grid.row(row),
                "{lines}: main-screen row {row} differs after replay"
            );
            assert_eq!(
                src.active_grid().row(row),
                dst.active_grid().row(row),
                "{lines}: active row {row} differs after replay"
            );
        }
    }
}

#[test]
fn screen_restore_sequence_omits_keypad_when_off() {
    // Keypad-off (the default, or an explicit ESC >) is the replay target's
    // own initial state, so the restore stream must not carry ESC =.
    let mut src = Terminal::new(80, 24);
    src.process(b"\x1b="); // set, then explicitly clear
    src.process(b"\x1b>");
    assert!(!src.modes.application_keypad);

    let restore = src.export_screen_restore_sequence();
    assert!(!restore.contains("\x1b="), "off keypad must not be emitted");
}
