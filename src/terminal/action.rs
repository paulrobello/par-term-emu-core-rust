//! Terminal parse actions, separated from state mutation (ARC-021).
//!
//! [`TerminalAction`] mirrors the `vte::Perform` callbacks as an owned,
//! structured value. A recorded stream of actions can be queued, replayed, or
//! — most usefully — constructed directly in a test and applied to a
//! [`Terminal`] without going through byte parsing ("test the handler in
//! isolation", the audit's goal).
//!
//! `vte::Params`'s constructors are `pub(crate)`, so a stored CSI/DCS action
//! can't rebuild a `&Params` to hand to the handlers. Instead, `apply_action`
//! reconstructs the action's **canonical byte form** and feeds it through a
//! fresh `vte::Parser` (which re-dispatches to the existing handlers). For
//! standard sequences this round-trips exactly; the structured action remains
//! the public interface, so callers/tests never deal with bytes.
//!
//! The hot path (`Terminal::process` → `Perform`) is unchanged — these are
//! opt-in entry points for replay/testing/queueing.

use vte::{Params, Perform};

use crate::terminal::Terminal;

/// A parsed terminal action, separated from state mutation (ARC-021).
///
/// Mirrors the `vte::Perform` callbacks. `params`/`intermediates` are stored
/// owned so an action can be constructed directly (e.g. in a test) and applied
/// without a parser. `params` is a vector of parameters, each a vector of
/// sub-parameters (the `:`-separated form), matching how `vte::Params` iterates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalAction {
    /// A printable character (`Perform::print`).
    Print(char),
    /// A C0 control byte (`Perform::execute`).
    Execute(u8),
    /// A CSI sequence (`Perform::csi_dispatch`).
    CsiDispatch {
        params: Vec<Vec<u16>>,
        intermediates: Vec<u8>,
        ignore: bool,
        action: char,
    },
    /// An OSC sequence (`Perform::osc_dispatch`).
    OscDispatch {
        params: Vec<Vec<u8>>,
        bell_terminated: bool,
    },
    /// Start of a DCS sequence (`Perform::hook`).
    DcsHook {
        params: Vec<Vec<u16>>,
        intermediates: Vec<u8>,
        ignore: bool,
        action: char,
    },
    /// One DCS payload byte (`Perform::put`).
    DcsPut(u8),
    /// End of a DCS sequence (`Perform::unhook`).
    DcsUnhook,
    /// An ESC sequence (`Perform::esc_dispatch`).
    EscDispatch {
        intermediates: Vec<u8>,
        ignore: bool,
        byte: u8,
    },
}

fn push_char(buf: &mut Vec<u8>, c: char) {
    buf.extend_from_slice(c.encode_utf8(&mut [0u8; 4]).as_bytes());
}

/// Encode parameters in their canonical wire form: parameters separated by `;`,
/// sub-parameters within a parameter separated by `:`. An empty parameter
/// contributes nothing (so a default parameter round-trips as an empty field).
fn encode_params(buf: &mut Vec<u8>, params: &[Vec<u16>]) {
    for (i, param) in params.iter().enumerate() {
        if i > 0 {
            buf.push(b';');
        }
        for (j, sub) in param.iter().enumerate() {
            if j > 0 {
                buf.push(b':');
            }
            buf.extend_from_slice(sub.to_string().as_bytes());
        }
    }
}

impl TerminalAction {
    /// Reconstruct the canonical byte form of this action (ARC-021).
    ///
    /// Feeding these bytes through a `vte::Parser` reproduces the original
    /// dispatch. `ignore` is not encoded — a re-parsed ignored sequence is
    /// ignored again (same no-op effect).
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            TerminalAction::Print(c) => {
                let mut buf = Vec::new();
                push_char(&mut buf, *c);
                buf
            }
            TerminalAction::Execute(b) => vec![*b],
            TerminalAction::CsiDispatch {
                params,
                intermediates,
                action,
                ..
            } => {
                let mut buf = vec![0x1b, b'['];
                encode_params(&mut buf, params);
                for &inter in intermediates {
                    buf.push(inter);
                }
                push_char(&mut buf, *action);
                buf
            }
            TerminalAction::OscDispatch {
                params,
                bell_terminated,
            } => {
                let mut buf = vec![0x1b, b']'];
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        buf.push(b';');
                    }
                    buf.extend_from_slice(p);
                }
                if *bell_terminated {
                    buf.push(0x07);
                } else {
                    buf.extend_from_slice(&[0x1b, b'\\']);
                }
                buf
            }
            TerminalAction::DcsHook {
                params,
                intermediates,
                action,
                ..
            } => {
                let mut buf = vec![0x1b, b'P'];
                encode_params(&mut buf, params);
                for &inter in intermediates {
                    buf.push(inter);
                }
                push_char(&mut buf, *action);
                buf
            }
            TerminalAction::DcsPut(b) => vec![*b],
            TerminalAction::DcsUnhook => vec![0x1b, b'\\'],
            TerminalAction::EscDispatch {
                intermediates,
                byte,
                ..
            } => {
                let mut buf = vec![0x1b];
                for &inter in intermediates {
                    buf.push(inter);
                }
                buf.push(*byte);
                buf
            }
        }
    }
}

impl Terminal {
    /// Apply one structured action, mutating terminal state (ARC-021).
    ///
    /// The action's canonical byte form is fed through a fresh `vte::Parser`,
    /// which dispatches to the existing handlers — so a test can construct an
    /// action (e.g. `CsiDispatch { params: vec![vec![1]], action: 'm', .. }`)
    /// and apply it without knowing the wire bytes. Self-contained actions
    /// (Print/Execute/CSI/OSC/ESC) apply cleanly in isolation; a DCS sequence
    /// should be applied as a group via [`Terminal::apply_actions`] so its
    /// Hook→Put→Unhook spans a single parser.
    pub fn apply_action(&mut self, action: TerminalAction) {
        let bytes = action.to_bytes();
        if !bytes.is_empty() {
            let mut parser = vte::Parser::new();
            parser.advance(self, &bytes);
        }
    }

    /// Apply a stream of actions (ARC-021 replay/queue).
    ///
    /// All actions' bytes are concatenated and fed through a single
    /// `vte::Parser`, so multi-action sequences like DCS (Hook + Put(s) +
    /// Unhook) replay correctly. Equivalent to re-running the original parse.
    pub fn apply_actions<I>(&mut self, actions: I)
    where
        I: IntoIterator<Item = TerminalAction>,
    {
        let bytes: Vec<u8> = actions.into_iter().flat_map(|a| a.to_bytes()).collect();
        if !bytes.is_empty() {
            let mut parser = vte::Parser::new();
            parser.advance(self, &bytes);
        }
    }
}

/// Collect `vte::Params` into an owned `Vec<Vec<u16>>` (parameters × sub-params).
fn params_to_owned(params: &Params) -> Vec<Vec<u16>> {
    params.iter().map(|sub| sub.to_vec()).collect()
}

/// A `vte::Perform` impl that records every callback as a [`TerminalAction`].
/// Used by [`parse_to_actions`] to turn a byte stream into a structured action
/// stream (ARC-021 recording, for replay/inspection).
struct ActionRecorder {
    actions: Vec<TerminalAction>,
}

impl Perform for ActionRecorder {
    fn print(&mut self, c: char) {
        self.actions.push(TerminalAction::Print(c));
    }
    fn execute(&mut self, byte: u8) {
        self.actions.push(TerminalAction::Execute(byte));
    }
    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        self.actions.push(TerminalAction::CsiDispatch {
            params: params_to_owned(params),
            intermediates: intermediates.to_vec(),
            ignore,
            action,
        });
    }
    fn osc_dispatch(&mut self, params: &[&[u8]], bell_terminated: bool) {
        self.actions.push(TerminalAction::OscDispatch {
            params: params.iter().map(|p| p.to_vec()).collect(),
            bell_terminated,
        });
    }
    fn hook(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        self.actions.push(TerminalAction::DcsHook {
            params: params_to_owned(params),
            intermediates: intermediates.to_vec(),
            ignore,
            action,
        });
    }
    fn put(&mut self, byte: u8) {
        self.actions.push(TerminalAction::DcsPut(byte));
    }
    fn unhook(&mut self) {
        self.actions.push(TerminalAction::DcsUnhook);
    }
    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
        self.actions.push(TerminalAction::EscDispatch {
            intermediates: intermediates.to_vec(),
            ignore,
            byte,
        });
    }
}

/// Parse a raw byte stream into a structured [`TerminalAction`] stream
/// (ARC-021). The result can be inspected, stored, or replayed via
/// [`Terminal::apply_actions`].
pub fn parse_to_actions(bytes: &[u8]) -> Vec<TerminalAction> {
    let mut recorder = ActionRecorder {
        actions: Vec::new(),
    };
    let mut parser = vte::Parser::new();
    parser.advance(&mut recorder, bytes);
    recorder.actions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_csi_action_sets_graphic_rendition() {
        // Construct the SGR "bold" action directly — no byte parsing.
        let mut term = Terminal::new(20, 4);
        term.apply_action(TerminalAction::CsiDispatch {
            params: vec![vec![1]],
            intermediates: Vec::new(),
            ignore: false,
            action: 'm',
        });
        term.apply_action(TerminalAction::Print('A'));
        let cell = term.active_grid().get(0, 0).expect("cell at 0,0");
        assert!(
            cell.flags.bold(),
            "SGR bold (ESC[1m) should apply to the printed char"
        );
    }

    #[test]
    fn apply_osc_action_sets_title() {
        let mut term = Terminal::new(20, 4);
        term.apply_action(TerminalAction::OscDispatch {
            params: vec![b"0".to_vec(), b"hello".to_vec()],
            bell_terminated: false,
        });
        assert_eq!(term.title(), "hello");
    }

    #[test]
    fn parse_to_actions_replays_identically() {
        let bytes = b"\x1b[4mAB\x1b[0m";
        // Direct parse.
        let mut direct = Terminal::new(20, 4);
        direct.process(bytes);
        // Parse to actions, then replay the action stream.
        let mut replayed = Terminal::new(20, 4);
        replayed.apply_actions(parse_to_actions(bytes));

        for col in 0..2 {
            let a = direct.active_grid().get(col, 0).expect("direct cell");
            let b = replayed.active_grid().get(col, 0).expect("replayed cell");
            assert_eq!(a.c, b.c, "char mismatch at col {col}");
            assert_eq!(
                a.flags.underline(),
                b.flags.underline(),
                "underline mismatch at col {col}"
            );
        }
    }

    #[test]
    fn execute_action_moves_cursor() {
        let mut term = Terminal::new(20, 4);
        term.apply_action(TerminalAction::Execute(b'\r'));
        term.apply_action(TerminalAction::Execute(b'\n'));
        // After CR+LF from (0,0): cursor at col 0, row 1.
        assert_eq!((term.cursor().col, term.cursor().row), (0, 1));
    }
}
