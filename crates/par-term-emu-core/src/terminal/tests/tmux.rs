// Tmux control mode tests
use crate::terminal::*;

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
    let mut term = Terminal::new(80, 24);
    term.set_tmux_control_mode(true);

    term.process(b"%begin 1234567890 1\n");
    term.process(b"%output %1 Hello World\n");
    term.process(b"%end 1234567890 1\n");

    let content = term.content();
    assert!(!content.contains("%begin"));
    assert!(!content.contains("%output"));
    assert!(!content.contains("%end"));

    let notifications = term.drain_tmux_notifications();
    assert!(notifications.len() >= 2);
}
