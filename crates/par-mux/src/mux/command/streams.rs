//! Raw pane stream parsers: `observe-pane` and `control-pane`.

use super::*;

/// `observe-pane -t <pane>`: the read-only stream.
pub(super) fn parse_observe_pane(a: &Args<'_>) -> Result<MuxCommand, String> {
    reject_positionals(a, &["-t"])?;
    Ok(MuxCommand::PaneStream {
        pane: a.pane("-t")?,
        control: false,
    })
}

/// `control-pane -t <pane>`: the writable stream.
pub(super) fn parse_control_pane(a: &Args<'_>) -> Result<MuxCommand, String> {
    reject_positionals(a, &["-t"])?;
    Ok(MuxCommand::PaneStream {
        pane: a.pane("-t")?,
        control: true,
    })
}
