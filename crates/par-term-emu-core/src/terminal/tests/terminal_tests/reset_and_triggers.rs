//! RIS/DECSTR resets, trigger scanning, and styled export.

use super::*;

// ARC-058: RIS resets the VT, not the embedder's configuration.

#[test]
fn ris_preserves_host_config() {
    use crate::graphics::kitty::FileMediaMode;
    use crate::observer::TerminalObserver;
    use crate::terminal::TriggerEngine;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let mut term = Terminal::new(80, 24);

    // Every embedder setter to a non-default value.
    term.set_allow_file_media(FileMediaMode::All);
    term.set_disable_insecure_sequences(true);
    term.set_accept_osc7(false);
    term.set_max_osc_data_length(4096);
    term.set_sixel_limits(11, 22, 33);
    term.set_max_sixel_graphics(7);
    term.set_cell_dimensions(9, 19);
    term.set_answerback_string(Some("par-test".to_string()));
    term.set_allow_clipboard_read(true);
    term.set_allow_clipboard_write(false);
    term.set_ansi_palette_color(1, Color::Rgb(1, 2, 3)).unwrap();
    term.set_faint_text_alpha(0.25);
    // HostConfig (ARC-100): these reverted before the carried struct.
    term.set_max_transfer_size(1234);
    term.set_bold_brightening(false);
    term.set_window_position(5, 6);
    term.set_window_iconified(true);
    term.set_max_mouse_history(3);
    term.set_max_inline_images(2);
    term.set_max_command_history(2);
    term.set_max_cwd_history(1);
    term.set_max_clipboard_sync_history(1);

    #[derive(Default)]
    struct CountingObserver {
        events: AtomicUsize,
    }
    impl TerminalObserver for CountingObserver {
        fn on_event(&self, _event: &TerminalEvent) {
            self.events.fetch_add(1, Ordering::SeqCst);
        }
    }
    let observer = std::sync::Arc::new(CountingObserver::default());
    term.add_observer(observer.clone());
    term.set_event_subscription(std::collections::HashSet::from([
        TerminalEventKind::BellRang,
        TerminalEventKind::TitleChanged,
    ]));
    TriggerEngine::add_trigger(&mut term, "t".into(), "MARKER".into(), vec![]).unwrap();

    term.mark_clean();
    term.process(b"\x1bc"); // RIS

    assert_eq!(term.allow_file_media(), FileMediaMode::All);
    assert!(term.disable_insecure_sequences());
    assert!(!term.accept_osc7());
    assert_eq!(term.max_osc_data_length(), 4096);
    assert_eq!(term.sixel_limits().max_width, 11);
    assert_eq!(term.sixel_limits().max_height, 22);
    assert_eq!(term.sixel_limits().max_repeat, 33);
    assert_eq!(term.max_sixel_graphics(), 7);
    assert_eq!(term.cell_dimensions(), (9, 19));
    assert_eq!(term.answerback_string(), Some("par-test"));
    assert!(term.allow_clipboard_read());
    assert!(!term.allow_clipboard_write());
    assert_eq!(term.get_ansi_color(1), Some(Color::Rgb(1, 2, 3)));
    assert_eq!(term.faint_text_alpha(), 0.25);

    assert_eq!(term.get_max_transfer_size(), 1234);
    assert!(!term.bold_brightening());
    assert_eq!(term.window_position(), (5, 6));
    assert!(term.window_iconified());
    assert_eq!(term.get_max_mouse_history(), 3);
    assert_eq!(term.host.max_inline_images, 2);
    assert_eq!(term.clipboard_sync.max_history, 1);
    assert_eq!(
        term.events.subscription(),
        Some(&std::collections::HashSet::from([
            TerminalEventKind::BellRang,
            TerminalEventKind::TitleChanged,
        ]))
    );

    assert_eq!(term.observer_count(), 1, "observers must survive RIS");
    assert_eq!(
        TriggerEngine::list_triggers(&term).len(),
        1,
        "trigger registry must survive RIS"
    );

    // The whole screen changed.
    let dirty = term.get_dirty_rows();
    assert_eq!(dirty.len(), 24, "every screen row must be dirty after RIS");
    assert_eq!(dirty.last(), Some(&23));

    // The surviving observer still receives events emitted after the reset.
    let batch = term.process_deferred(b"\x07");
    batch.deliver();
    assert_eq!(
        observer.events.load(Ordering::SeqCst),
        1,
        "observer must receive the post-reset bell"
    );

    // History caps are probed by what they retain; this emits events, so
    // it runs after the observer count above.
    for i in 0..6 {
        term.start_command_execution(format!("c{i}"));
        term.end_command_execution(Some(0));
    }
    assert_eq!(term.get_command_history().len(), 2);
    for i in 0..4 {
        term.record_cwd_change(crate::terminal::CwdChange {
            old_cwd: None,
            new_cwd: format!("/tmp/d{i}"),
            hostname: None,
            username: None,
            timestamp: 0,
        });
    }
    assert_eq!(term.get_cwd_changes().len(), 1);
}

#[test]
fn ris_resets_osc4_palette_drift_to_configured() {
    let mut term = Terminal::new(80, 24);
    term.set_ansi_palette_color(2, Color::Rgb(10, 20, 30))
        .unwrap();
    // OSC 4 drifts palette entry 2 away from the configured value.
    term.process(b"\x1b]4;2;rgb:ff/00/00\x07");
    assert_eq!(term.get_ansi_color(2), Some(Color::Rgb(0xff, 0, 0)));

    term.process(b"\x1bc");

    // RIS restores the configured palette, not the factory default.
    assert_eq!(term.get_ansi_color(2), Some(Color::Rgb(10, 20, 30)));
}

#[test]
fn ris_restores_configured_conformance_level() {
    use crate::conformance_level::ConformanceLevel;

    let mut term = Terminal::new(80, 24);
    term.set_conformance_level(ConformanceLevel::VT220);
    assert_eq!(term.conformance_level(), ConformanceLevel::VT220);

    // A program's DECSCL changes the live level...
    term.process(b"\x1b[61\"p");
    assert_eq!(term.conformance_level(), ConformanceLevel::VT100);

    // ...until RIS, which restores the host's configured baseline.
    term.process(b"\x1bc");
    assert_eq!(term.conformance_level(), ConformanceLevel::VT220);
}

#[test]
fn ris_restores_configured_bell_volumes() {
    let mut term = Terminal::new(80, 24);
    term.set_warning_bell_volume(1);
    term.set_margin_bell_volume(2);
    assert_eq!(
        (term.warning_bell_volume(), term.margin_bell_volume()),
        (1, 2)
    );

    // DECSWBV / DECSMBV from a program change the live volumes...
    term.process(b"\x1b[7 t\x1b[6 u");
    assert_eq!(
        (term.warning_bell_volume(), term.margin_bell_volume()),
        (7, 6)
    );

    // ...until RIS, which restores the host's configured baselines.
    term.process(b"\x1bc");
    assert_eq!(
        (term.warning_bell_volume(), term.margin_bell_volume()),
        (1, 2)
    );
}

#[test]
fn bell_volume_setters_clamp_to_eight() {
    let mut term = Terminal::new(80, 24);
    term.set_warning_bell_volume(200);
    term.set_margin_bell_volume(9);
    assert_eq!(
        (term.warning_bell_volume(), term.margin_bell_volume()),
        (8, 8)
    );
}

#[test]
fn decstr_keeps_screen_and_scrollback() {
    let mut term = Terminal::with_scrollback(80, 5, 10);
    for i in 0..8 {
        term.process(format!("line{i}\r\n").as_bytes());
    }
    let before = term.export_text();
    assert_eq!(term.grid().total_lines_scrolled(), 4);

    // Non-default state a program can set: SGR, DECAWM off, DECOM on,
    // margins, cursor hidden and moved, DECSC saved state.
    term.process(b"\x1b[31;1m\x1b[?7l\x1b[?6h\x1b[2;3r\x1b[?25l\x1b[3;2H\x1b7");

    term.process(b"\x1b[!p"); // DECSTR

    // Screen content and scrollback are untouched.
    assert_eq!(term.export_text(), before);
    assert_eq!(term.grid().total_lines_scrolled(), 4);

    // Modes, SGR, margins, and cursor reset.
    assert_eq!(term.attrs.fg, Color::Named(NamedColor::White));
    assert_eq!(term.attrs.bg, Color::Named(NamedColor::Black));
    assert!(!term.attrs.flags.bold());
    assert!(term.modes.auto_wrap);
    assert!(!term.modes.origin_mode);
    assert_eq!(term.margins.scroll_region_top, 0);
    assert_eq!(term.margins.scroll_region_bottom, 4);
    assert!(term.cursor.visible);
    assert_eq!((term.cursor.col, term.cursor.row), (0, 0));

    // The saved cursor is home with default attributes.
    let saved = term.saved_state.saved_cursor.unwrap();
    assert_eq!((saved.col, saved.row), (0, 0));
}

#[test]
fn decstr_preserves_host_config() {
    use crate::graphics::kitty::FileMediaMode;

    let mut term = Terminal::new(80, 24);
    term.set_disable_insecure_sequences(true);
    term.set_accept_osc7(false);
    term.set_max_osc_data_length(4096);
    term.set_allow_file_media(FileMediaMode::All);
    term.set_answerback_string(Some("par-test".to_string()));

    term.process(b"\x1b[!p");

    assert!(term.disable_insecure_sequences());
    assert!(!term.accept_osc7());
    assert_eq!(term.max_osc_data_length(), 4096);
    assert_eq!(term.allow_file_media(), FileMediaMode::All);
    assert_eq!(term.answerback_string(), Some("par-test"));
}

// --- ARC-064: triggers fire on written text, not on incidental damage ---

#[test]
fn alt_screen_switch_does_not_refire_triggers() {
    let mut term = Terminal::new(80, 24);
    TriggerEngine::add_trigger(&mut term, "err".into(), "ERROR".into(), vec![]).unwrap();
    term.process(b"ERROR one\r\nERROR two\r\n");
    TriggerEngine::process_trigger_scans(&mut term);
    assert_eq!(TriggerEngine::poll_trigger_matches(&mut term).len(), 2);

    term.process(b"\x1b[?1049h\x1b[?1049l");
    TriggerEngine::process_trigger_scans(&mut term);
    assert_eq!(
        TriggerEngine::poll_trigger_matches(&mut term).len(),
        0,
        "alt-screen switch marks damage but wrote no text; no trigger may re-fire"
    );
}

#[test]
fn scroll_does_not_refire_triggers() {
    let mut term = Terminal::new(80, 24);
    TriggerEngine::add_trigger(&mut term, "err".into(), "ERROR".into(), vec![]).unwrap();
    term.process(b"ERROR alpha\r\n");
    TriggerEngine::process_trigger_scans(&mut term);
    assert_eq!(TriggerEngine::poll_trigger_matches(&mut term).len(), 1);

    term.process(b"\x1b[1T"); // SD: scroll down; the ERROR line stays visible at row 1
    TriggerEngine::process_trigger_scans(&mut term);
    assert_eq!(
        TriggerEngine::poll_trigger_matches(&mut term).len(),
        0,
        "a line scrolled while visible must not re-match"
    );
}

#[test]
fn wrapped_line_is_scanned_where_the_scroll_left_it() {
    let mut term = Terminal::new(80, 24);
    TriggerEngine::add_trigger(&mut term, "err".into(), "ERROR".into(), vec![]).unwrap();
    term.process(b"\x1b[24;1HERROR bottom\r\n");
    TriggerEngine::process_trigger_scans(&mut term);
    let matches = TriggerEngine::poll_trigger_matches(&mut term);
    assert_eq!(matches.len(), 1, "the written line is scanned exactly once");
    assert_eq!(
        matches[0].row, 22,
        "the scan follows scrolled content, not the write index"
    );
}

// --- ARC-091: a written row that leaves the screen before the next scan is
// still scanned, exactly once, at the row it occupied as it left ---

fn error_trigger_terminal(rows: usize) -> Terminal {
    let mut term = Terminal::new(80, rows);
    TriggerEngine::add_trigger(&mut term, "err".into(), "ERROR".into(), vec![]).unwrap();
    term
}

#[test]
fn trigger_matches_row_scrolled_out_before_scan() {
    let mut term = error_trigger_terminal(5);
    let mut feed = b"ERROR lost\r\n".to_vec();
    for _ in 0..10 {
        feed.extend_from_slice(b"line\r\n");
    }
    term.process(&feed);
    TriggerEngine::process_trigger_scans(&mut term);

    let matches = TriggerEngine::poll_trigger_matches(&mut term);
    assert_eq!(matches.len(), 1, "the row scrolled into history is scanned");
    assert_eq!(matches[0].text, "ERROR");
    assert_eq!(matches[0].row, 0, "the row it occupied as it left");
}

#[test]
fn trigger_matches_row_discarded_by_region_scroll() {
    let mut term = error_trigger_terminal(5);
    // DECSTBM rows 2-4, cursor to the region top, then SU past the region.
    term.process(b"\x1b[2;4r\x1b[2;1HERROR r\x1b[5S");
    TriggerEngine::process_trigger_scans(&mut term);

    let matches = TriggerEngine::poll_trigger_matches(&mut term);
    assert_eq!(
        matches.len(),
        1,
        "a row a region scroll discards is scanned"
    );
    assert_eq!(matches[0].row, 1);
}

#[test]
fn trigger_matches_row_pushed_off_bottom_by_ri() {
    let mut term = error_trigger_terminal(5);
    term.process(b"\x1b[5;1HERROR b\x1b[H\x1bM\x1bM\x1bM");
    TriggerEngine::process_trigger_scans(&mut term);

    let matches = TriggerEngine::poll_trigger_matches(&mut term);
    assert_eq!(
        matches.len(),
        1,
        "a row RI pushes off the bottom is scanned"
    );
    assert_eq!(matches[0].row, 4);
}

#[test]
fn trigger_matches_rows_removed_by_il_and_dl() {
    let mut term = error_trigger_terminal(5);
    // IL at row 1 pushes row 4 off the bottom; DL at row 0 deletes row 0.
    term.process(b"\x1b[1;1HERROR top\x1b[5;1HERROR bottom\x1b[2;1H\x1b[L\x1b[1;1H\x1b[M");
    TriggerEngine::process_trigger_scans(&mut term);

    let mut rows: Vec<usize> = TriggerEngine::poll_trigger_matches(&mut term)
        .iter()
        .map(|m| m.row)
        .collect();
    rows.sort_unstable();
    assert_eq!(rows, vec![0, 4]);
}

#[test]
fn wide_char_wrap_scroll_keeps_pending_rows() {
    let mut term = error_trigger_terminal(5);
    // Fill the bottom row to column 78 so the cursor sits at column 79; a
    // wide char there wraps and scrolls the region by one.
    let mut feed = b"\x1b[5;1HERROR w".to_vec();
    feed.extend(std::iter::repeat_n(b'.', 79 - "ERROR w".len()));
    feed.extend_from_slice("世".as_bytes());
    term.process(&feed);
    TriggerEngine::process_trigger_scans(&mut term);

    let matches = TriggerEngine::poll_trigger_matches(&mut term);
    assert_eq!(matches.len(), 1, "the wrapped row is still pending");
    assert_eq!(matches[0].row, 3, "the row the wrap scroll moved it to");
}

#[test]
fn styled_export_omits_default_colors() {
    // The exact byte shape bash+starship+shell-integration emits at the pty
    // for one prompt draw (captured raw via `script`): OSC 133 D/A marks,
    // fg-only starship styles, CRLF, the character segment. The styled
    // export must omit the DEFAULT colors (fg White, bg Black — never
    // explicitly set), so a replaying host paints them with its own
    // defaults; emitting palette `37`/`40` instead painted visible grey
    // bands around replayed prompts on hosts whose palette-0 is not black.
    let mut src = Terminal::new(80, 24);
    src.process(b"prev \x1b[0moutput\r\n");
    src.process(b"\x1b]133;D;0\x07\x1b]133;A\x07");
    src.process(b"\x1b[1;31mprobello\x1b[33m@\x1b[32mMacDaddy\x1b[0m:\x1b[1;36m~\x1b[0m\r\n");
    src.process(b"\x1b[1;32m\xe2\x9d\xaf\x1b[0m");
    let restore = src.export_screen_restore_sequence();
    assert!(
        restore.contains("\x1b[0;31;1mprobello"),
        "default bg must not ride along with an explicit fg: {restore:?}"
    );
    assert!(
        !restore.contains("40m") && !restore.contains(";40;"),
        "no palette-black background may appear anywhere in the replay: {restore:?}"
    );
    // Explicit colors survive: the Yellow `@` and Cyan `~` keep their codes.
    assert!(
        restore.contains("\x1b[0;33;1m@") && restore.contains("\x1b[0;36;1m~"),
        "explicit fg colors must still be emitted: {restore:?}"
    );
}
