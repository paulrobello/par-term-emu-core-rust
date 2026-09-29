//! APC (Application Program Command) pre-filter for Kitty Terminal Graphics.
//!
//! `vte` 0.15 does not deliver APC payload bytes to the `Perform` trait. APC
//! content is silently dropped after state transitions through
//! `State::SosPmApcString`. To handle Kitty TGP (`ESC _ G ... ST`), we strip
//! Kitty-specific APC sequences out of the raw byte stream *before* feeding
//! the rest to `vte::Parser::advance`.
//!
//! Only APC sequences that begin with `ESC _ G` (Kitty TGP) are intercepted.
//! All other APC sequences (e.g. `ESC _ X ...`) are passed through unchanged
//! so that `vte` can swallow them as it normally would.
//!
//! ## State machine
//!
//! The filter is a streaming byte-level state machine. APC sequences may be
//! split arbitrarily across calls to `feed`, so state must be preserved
//! between invocations.
//!
//! Terminator: APC sequences end with either `ESC \\` (`0x1b 0x5c`, ST as a
//! 7-bit sequence) or the C1 byte `0x9c` (ST as 8-bit single byte).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ApcFilterState {
    /// Normal byte stream.
    #[default]
    Outside,
    /// Saw `ESC` outside an APC; next byte determines if this becomes APC.
    SawEsc,
    /// Saw `ESC _`; next byte tells us whether this is Kitty (`G`) or not.
    SawEscUnderscore,
    /// Inside a Kitty APC payload; accumulating bytes into `apc_buffer`.
    InKittyApc,
    /// Inside a Kitty APC and saw `ESC`; if next byte is `\\` we terminate.
    InKittyApcSawEsc,
    /// A Kitty APC ran past [`MAX_KITTY_APC_BYTES`]: payload bytes are
    /// discarded until the ST terminator, then the APC is dropped whole
    /// (SEC-116). The next APC starts from a clean buffer.
    KittyApcOverflow,
    /// Overflow state after an `ESC` — mirrors `InKittyApcSawEsc` for
    /// terminator detection while discarding.
    KittyApcOverflowSawEsc,
}

/// Append one payload byte unless the accumulator is at the cap
/// (SEC-116). Returns `false` when the buffer was full — the caller
/// moves to [`ApcFilterState::KittyApcOverflow`].
fn push_capped(apc_buffer: &mut Vec<u8>, byte: u8) -> bool {
    if apc_buffer.len() < MAX_KITTY_APC_BYTES {
        apc_buffer.push(byte);
        true
    } else {
        false
    }
}

/// cap: Bytes one Kitty APC payload may accumulate on the wire (SEC-116)
/// — base64 of the 64 MiB decoded payload cap (`MAX_KITTY_PAYLOAD_BYTES`)
/// plus headroom for key=value parameters. A payload at or past this size
/// is dropped at termination; the accumulator never grows past it.
pub(crate) const MAX_KITTY_APC_BYTES: usize = 96 * 1024 * 1024;

/// Outcome of a completed Kitty APC payload, returned by [`feed`].
pub(crate) struct CompletedKittyApc<'a> {
    /// Raw payload bytes (everything between `ESC _ G` and the ST terminator,
    /// excluding the leading `G`).
    pub payload: &'a [u8],
}

/// Drives the APC pre-filter for one input chunk.
///
/// `data` is the raw incoming byte slice. Bytes that are NOT part of an
/// intercepted Kitty APC are appended to `passthrough`. Each time a Kitty
/// APC payload completes (ST terminator seen), `on_kitty` is invoked with
/// the payload. The state machine and accumulator are stored on the caller
/// to allow resumption across chunks.
///
/// `on_kitty` receives the payload as a borrowed slice; it must process the
/// data synchronously (the buffer is reused for subsequent payloads).
pub(crate) fn feed<F>(
    state: &mut ApcFilterState,
    apc_buffer: &mut Vec<u8>,
    data: &[u8],
    passthrough: &mut Vec<u8>,
    mut on_kitty: F,
) where
    F: FnMut(CompletedKittyApc<'_>, usize),
{
    for &byte in data {
        match *state {
            ApcFilterState::Outside => {
                if byte == 0x1b {
                    *state = ApcFilterState::SawEsc;
                } else {
                    passthrough.push(byte);
                }
            }
            ApcFilterState::SawEsc => match byte {
                b'_' => {
                    *state = ApcFilterState::SawEscUnderscore;
                }
                0x1b => {
                    // Two ESCs in a row — emit the first and stay in SawEsc
                    passthrough.push(0x1b);
                }
                other => {
                    // Not an APC: emit the ESC and the byte, return to Outside.
                    passthrough.push(0x1b);
                    passthrough.push(other);
                    *state = ApcFilterState::Outside;
                }
            },
            ApcFilterState::SawEscUnderscore => match byte {
                b'G' => {
                    // Kitty APC. Begin accumulating payload (skip the 'G').
                    apc_buffer.clear();
                    *state = ApcFilterState::InKittyApc;
                }
                0x1b => {
                    // ESC _ ESC — emit the first APC opener, treat new ESC.
                    passthrough.push(0x1b);
                    passthrough.push(b'_');
                    *state = ApcFilterState::SawEsc;
                }
                other => {
                    // Not Kitty: pass through `ESC _ X` unchanged so vte can
                    // consume and discard it.
                    passthrough.push(0x1b);
                    passthrough.push(b'_');
                    passthrough.push(other);
                    *state = ApcFilterState::Outside;
                }
            },
            ApcFilterState::InKittyApc => match byte {
                0x1b => {
                    *state = ApcFilterState::InKittyApcSawEsc;
                }
                0x9c => {
                    // 8-bit ST: terminate Kitty APC.
                    on_kitty(
                        CompletedKittyApc {
                            payload: apc_buffer.as_slice(),
                        },
                        passthrough.len(),
                    );
                    *state = ApcFilterState::Outside;
                }
                other => {
                    if !push_capped(apc_buffer, other) {
                        // Overflow stops appending — the buffer stays at
                        // the cap and the payload is dropped at ST.
                        *state = ApcFilterState::KittyApcOverflow;
                    }
                }
            },
            ApcFilterState::InKittyApcSawEsc => match byte {
                b'\\' => {
                    // 7-bit ST: terminate Kitty APC.
                    on_kitty(
                        CompletedKittyApc {
                            payload: apc_buffer.as_slice(),
                        },
                        passthrough.len(),
                    );
                    *state = ApcFilterState::Outside;
                }
                0x1b => {
                    // ESC ESC inside APC — keep the first ESC as data and
                    // remain in `InKittyApcSawEsc` for the new ESC.
                    if !push_capped(apc_buffer, 0x1b) {
                        *state = ApcFilterState::KittyApcOverflow;
                    }
                }
                other => {
                    // ESC followed by something other than `\` — treat as
                    // payload bytes and continue.
                    if push_capped(apc_buffer, 0x1b) {
                        if !push_capped(apc_buffer, other) {
                            *state = ApcFilterState::KittyApcOverflow;
                        }
                    } else {
                        *state = ApcFilterState::KittyApcOverflow;
                    }
                }
            },
            ApcFilterState::KittyApcOverflow => match byte {
                0x1b => {
                    *state = ApcFilterState::KittyApcOverflowSawEsc;
                }
                0x9c => {
                    // 8-bit ST: drop the oversized APC whole and start clean.
                    crate::debug_error!(
                        "APC",
                        "Kitty APC payload exceeded {} bytes and was dropped",
                        MAX_KITTY_APC_BYTES
                    );
                    apc_buffer.clear();
                    *state = ApcFilterState::Outside;
                }
                // Everything else is discarded until the terminator.
                _ => {}
            },
            ApcFilterState::KittyApcOverflowSawEsc => match byte {
                b'\\' | 0x9c => {
                    // ST: drop the oversized APC whole and start clean.
                    crate::debug_error!(
                        "APC",
                        "Kitty APC payload exceeded {} bytes and was dropped",
                        MAX_KITTY_APC_BYTES
                    );
                    apc_buffer.clear();
                    *state = ApcFilterState::Outside;
                }
                0x1b => {
                    // Consecutive ESCs — stay armed for the terminator.
                    *state = ApcFilterState::KittyApcOverflowSawEsc;
                }
                _ => {
                    *state = ApcFilterState::KittyApcOverflow;
                }
            },
        }
    }
}

/// One full `feed` pass over `data` from a fresh state, returning how many
/// Kitty APC payloads completed. `#[doc(hidden)]` and `cfg(fuzzing)`: this
/// exists for the `apc_filter` cargo-fuzz target (cargo-fuzz builds the
/// library with `--cfg fuzzing`), not the public API — the byte-level
/// accumulator is otherwise reachable only through `Terminal::process`.
#[cfg(fuzzing)]
#[doc(hidden)]
pub fn fuzz_apc_filter(data: &[u8]) -> usize {
    let mut state = ApcFilterState::Outside;
    let mut apc_buffer = Vec::new();
    let mut passthrough = Vec::new();
    let mut completed = 0usize;
    feed(
        &mut state,
        &mut apc_buffer,
        data,
        &mut passthrough,
        |_payload, _offset| completed += 1,
    );
    completed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(chunks: &[&[u8]]) -> (Vec<u8>, Vec<Vec<u8>>) {
        let mut state = ApcFilterState::Outside;
        let mut buf = Vec::new();
        let mut pass = Vec::new();
        let mut completed: Vec<Vec<u8>> = Vec::new();
        for chunk in chunks {
            feed(&mut state, &mut buf, chunk, &mut pass, |apc, _offset| {
                completed.push(apc.payload.to_vec());
            });
        }
        (pass, completed)
    }

    #[test]
    fn passthrough_plain_text() {
        let (pass, comp) = run(&[b"hello"]);
        assert_eq!(pass, b"hello");
        assert!(comp.is_empty());
    }

    #[test]
    fn intercepts_kitty_apc_with_st() {
        let (pass, comp) = run(&[b"\x1b_Ga=t,i=1;ABCD\x1b\\"]);
        assert_eq!(pass, b"");
        assert_eq!(comp.len(), 1);
        assert_eq!(&comp[0], b"a=t,i=1;ABCD");
    }

    #[test]
    fn intercepts_kitty_apc_with_c1_st() {
        let (pass, comp) = run(&[b"\x1b_Ga=t,i=2;ZZ\x9c"]);
        assert_eq!(pass, b"");
        assert_eq!(comp.len(), 1);
        assert_eq!(&comp[0], b"a=t,i=2;ZZ");
    }

    #[test]
    fn split_across_chunks() {
        let (pass, comp) = run(&[b"\x1b_Ga=t,i=99;AA", b"AA\x1b\\"]);
        assert_eq!(pass, b"");
        assert_eq!(comp.len(), 1);
        assert_eq!(&comp[0], b"a=t,i=99;AAAA");
    }

    #[test]
    fn split_at_esc_boundary() {
        // ESC at end of one chunk, `_G` at start of next.
        let (pass, comp) = run(&[b"hi\x1b", b"_Ga=t;X\x1b\\"]);
        assert_eq!(pass, b"hi");
        assert_eq!(comp.len(), 1);
        assert_eq!(&comp[0], b"a=t;X");
    }

    #[test]
    fn split_at_st_boundary() {
        // ESC of ST split across chunks.
        let (pass, comp) = run(&[b"\x1b_Ga=t;X\x1b", b"\\after"]);
        assert_eq!(pass, b"after");
        assert_eq!(comp.len(), 1);
        assert_eq!(&comp[0], b"a=t;X");
    }

    #[test]
    fn non_kitty_apc_passes_through() {
        let (pass, comp) = run(&[b"\x1b_Xstuff\x1b\\"]);
        assert_eq!(pass, b"\x1b_Xstuff\x1b\\");
        assert!(comp.is_empty());
    }

    #[test]
    fn surrounding_text_preserved() {
        let (pass, comp) = run(&[b"hi\x1b_Ga=t;X\x1b\\bye"]);
        assert_eq!(pass, b"hibye");
        assert_eq!(comp.len(), 1);
    }

    #[test]
    fn esc_followed_by_non_underscore() {
        let (pass, comp) = run(&[b"\x1b[31m"]);
        assert_eq!(pass, b"\x1b[31m");
        assert!(comp.is_empty());
    }

    #[test]
    fn multiple_apcs_in_one_chunk() {
        let (pass, comp) = run(&[b"\x1b_Ga=t,i=1;A\x1b\\\x1b_Ga=t,i=2;B\x1b\\"]);
        assert_eq!(pass, b"");
        assert_eq!(comp.len(), 2);
        assert_eq!(&comp[0], b"a=t,i=1;A");
        assert_eq!(&comp[1], b"a=t,i=2;B");
    }

    /// SEC-116: an APC past the cap never reaches `on_kitty`, the
    /// accumulator stays at the cap, and the filter resets at ST — the
    /// next APC parses normally.
    #[test]
    fn oversized_kitty_apc_is_dropped_and_state_resets() {
        let mut state = ApcFilterState::Outside;
        let mut buf = Vec::new();
        let mut pass = Vec::new();
        let mut completed = 0usize;
        let mut data = vec![0x1b, b'_', b'G'];
        data.extend(std::iter::repeat_n(b'A', MAX_KITTY_APC_BYTES + 10));
        data.extend_from_slice(b"\x1b\\");
        feed(&mut state, &mut buf, &data, &mut pass, |_p, _o| {
            completed += 1
        });
        assert_eq!(completed, 0, "the oversized payload is dropped whole");
        assert!(
            buf.len() <= MAX_KITTY_APC_BYTES,
            "accumulator never grows past the cap: {}",
            buf.len()
        );
        assert_eq!(state, ApcFilterState::Outside, "ST resets the filter");
        // The overflow did not wedge the filter: a follow-up APC parses.
        feed(
            &mut state,
            &mut buf,
            b"\x1b_Ga=t;OK\x1b\\",
            &mut pass,
            |p, _o| {
                assert_eq!(p.payload, b"a=t;OK");
                completed += 1;
            },
        );
        assert_eq!(completed, 1);
    }

    /// SEC-116: the 8-bit C1 terminator resets the overflow state too —
    /// a bare `0x9c` must not leave the filter discarding the stream
    /// forever.
    #[test]
    fn oversized_kitty_apc_c1_st_resets() {
        let mut state = ApcFilterState::Outside;
        let mut buf = Vec::new();
        let mut pass = Vec::new();
        let mut data = vec![0x1b, b'_', b'G'];
        data.extend(std::iter::repeat_n(b'B', MAX_KITTY_APC_BYTES + 10));
        data.push(0x9c);
        feed(&mut state, &mut buf, &data, &mut pass, |_p, _o| {});
        assert_eq!(state, ApcFilterState::Outside, "C1 ST resets the filter");
        let mut completed = 0usize;
        feed(
            &mut state,
            &mut buf,
            b"\x1b_Ga=t;OK\x9c",
            &mut pass,
            |_p, _o| completed += 1,
        );
        assert_eq!(completed, 1, "the filter still parses after overflow");
    }

    /// SEC-116: a bomb split across feeds (the streaming shape a PTY
    /// actually delivers) stays bounded the same way.
    #[test]
    fn oversized_kitty_apc_split_across_feeds_stays_bounded() {
        let mut state = ApcFilterState::Outside;
        let mut buf = Vec::new();
        let mut pass = Vec::new();
        let mut completed = 0usize;
        feed(&mut state, &mut buf, b"\x1b_Ga=t;", &mut pass, |_p, _o| {});
        let filler = vec![b'C'; 256 * 1024];
        let mut sent = 0usize;
        while sent < MAX_KITTY_APC_BYTES + 10 {
            feed(&mut state, &mut buf, &filler, &mut pass, |_p, _o| {
                completed += 1
            });
            sent += filler.len();
        }
        feed(&mut state, &mut buf, b"\x1b\\", &mut pass, |_p, _o| {
            completed += 1
        });
        assert_eq!(completed, 0, "the split bomb is dropped whole");
        assert!(buf.len() <= MAX_KITTY_APC_BYTES);
        assert_eq!(state, ApcFilterState::Outside);
    }
}
