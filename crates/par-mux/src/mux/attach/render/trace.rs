//! Opt-in stderr diagnostics: the cursor trace and the input trace.

use std::io::Write as _;

/// Opt-in diagnostics (`PAR_MUX_CURSOR_TRACE=1`) for the intermittent
/// invisible-cursor report (joe in a render attach): each host cursor
/// write and each [`PaneRenderer::focused_cursor`] `None` reason lands
/// on stderr, so the next occurrence names the hider. Cheap when unset
/// (one `OnceLock` read per call). In render mode stderr is normally the
/// same tty as the frame, so redirect it (`2>/tmp/cursor.log`) or the
/// trace lines paint into the screen being diagnosed.
pub(crate) fn cursor_trace() -> bool {
    use std::sync::OnceLock;
    static TRACE: OnceLock<bool> = OnceLock::new();
    *TRACE.get_or_init(|| {
        std::env::var("PAR_MUX_CURSOR_TRACE").is_ok_and(|v| v != "0" && !v.is_empty())
    })
}

/// Write one cursor-trace line to stderr when `PAR_MUX_CURSOR_TRACE` is set.
pub(crate) fn trace_cursor(line: std::fmt::Arguments<'_>) {
    if cursor_trace() {
        let _ = writeln!(std::io::stderr(), "par-mux cursor-trace: {line}");
    }
}

/// Trace one `focused_cursor` `None` reason (when tracing is on) and
/// yield `None`, so the lookup chain keeps its early-return shape.
pub(crate) fn trace_none<T>(reason: std::fmt::Arguments<'_>) -> Option<T> {
    trace_cursor(format_args!("focused_cursor None ({reason})"));
    None
}

/// Opt-in input diagnostics (`PAR_MUX_INPUT_TRACE=1`): every host mouse
/// report, the router's hit, and every byte forwarded to a pane land on
/// stderr with timestamps, so a live misbehavior repro names the events
/// that reached the pane (the htop menu defect hunt). Redirect stderr
/// (`2>/tmp/input.log`) or the lines paint into the screen.
pub(crate) fn input_trace() -> bool {
    use std::sync::OnceLock;
    static TRACE: OnceLock<bool> = OnceLock::new();
    *TRACE.get_or_init(|| {
        std::env::var("PAR_MUX_INPUT_TRACE").is_ok_and(|v| v != "0" && !v.is_empty())
    })
}

/// Write one input-trace line to stderr when `PAR_MUX_INPUT_TRACE` is set.
pub(crate) fn trace_input(line: std::fmt::Arguments<'_>) {
    if input_trace() {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let _ = writeln!(std::io::stderr(), "par-mux input-trace[{t}]: {line}");
    }
}
