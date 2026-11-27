// Shell integration tests
use crate::terminal::*;

#[test]
fn test_shell_integration() {
    let mut term = Terminal::new(80, 24);
    // audit SEC-005: OSC 7 tracking is opt-in by default; tests exercise the opt-in path
    term.set_accept_osc7(true);

    // Prompt start
    term.process(b"\x1b]133;A\x07");
    assert!(term.shell_integration().in_prompt());

    // Command start
    term.process(b"\x1b]133;B\x07");
    assert!(term.shell_integration().in_command_input());

    // Command executed
    term.process(b"\x1b]133;C\x07");
    assert!(term.shell_integration().in_command_output());

    // Set CWD (OSC 7 with file:// URL format)
    term.process(b"\x1b]7;file://hostname/home/user\x07");
    assert_eq!(term.shell_integration().cwd(), Some("/home/user"));
}
