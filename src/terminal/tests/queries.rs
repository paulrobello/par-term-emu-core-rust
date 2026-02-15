// Device queries and reports
use crate::terminal::*;

#[test]
fn test_da_primary() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[62\"p");
    term.process(b"\x1b[c");

    let response = term.drain_responses();
    assert_eq!(response, b"\x1b[?62;1;4;6;9;15;22;52c");
}

#[test]
fn test_da_secondary() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[>c");

    let response = term.drain_responses();
    assert_eq!(response, b"\x1b[>82;10000;0c");
}

#[test]
fn test_dsr_cursor_position() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[10;20H\x1b[6n");

    let response = term.drain_responses();
    assert_eq!(response, b"\x1b[10;20R");
}

#[test]
fn test_decrqm_application_cursor() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[?1$p");
    assert_eq!(term.drain_responses(), b"\x1b[?1;2$y");
}

#[test]
fn test_enq_answerback_string() {
    let mut term = Terminal::new(80, 24);
    term.set_answerback_string(Some("par-term".to_string()));
    term.process(b"\x05");
    assert_eq!(term.drain_responses(), b"par-term");
}

#[test]
fn test_osc_10_query_default_fg() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b]10;?\x1b\\");
    let responses = term.drain_responses();
    assert!(String::from_utf8_lossy(&responses).starts_with("\x1b]10;"));
}
