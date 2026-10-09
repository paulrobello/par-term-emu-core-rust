#![no_main]

//! par-mux attach client stdin tokenizer (`InputParser`). The parser is
//! stateful across feeds (`pending` partial sequences, the `paste` flag)
//! and, since SEC-208, lives for the whole session, so the target drives ONE
//! parser through fuzzer-chosen burst splits.
//!
//! Input layout: the first `CTRL` bytes are burst lengths (each byte + 1),
//! the rest is the stdin payload.
//!
//! Invariants:
//! 1. `feed` never panics, and the parser never emits more bytes than it
//!    was given (pending bytes are re-scanned, never re-emitted).
//! 2. A bracketed-paste body comes out only as `Token::Paste`. It is never
//!    a `Key` or `Mouse`, and never a non-empty `Bytes` (the run the prefix
//!    and chord scanner sees). Empty `Bytes` are the marker-consumption
//!    drop shape and are allowed.
//! 3. The concatenated paste bytes equal the body for every split and for
//!    the whole-stream feed. The one designed divergence, a burst ending
//!    on the opener's bare ESC (the Escape key, xterm's no-timeout trade),
//!    is excluded by never cutting at stream offset 1.

use libfuzzer_sys::fuzz_target;
use par_term_emu_core_rust::mux::attach::fuzz_support::{InputParser, Token};

const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";
const CTRL: usize = 8;

/// Feed `stream` through one parser in bursts whose lengths `ctrl` encodes
/// (each byte + 1), then the remainder. A cut landing on `forbid` moves one
/// byte later.
fn feed_split(stream: &[u8], ctrl: &[u8], forbid: Option<usize>) -> Vec<Token> {
    let mut parser = InputParser::default();
    let mut tokens = Vec::new();
    let mut at = 0;
    for &c in ctrl {
        let mut cut = (at + usize::from(c) + 1).min(stream.len());
        if Some(cut) == forbid {
            cut = (cut + 1).min(stream.len());
        }
        tokens.extend(parser.feed(&stream[at..cut]));
        at = cut;
    }
    tokens.extend(parser.feed(&stream[at..]));
    tokens
}

fn emitted_len(tokens: &[Token]) -> usize {
    tokens
        .iter()
        .map(|t| match t {
            Token::Bytes(b) | Token::Paste(b) => b.len(),
            _ => 0,
        })
        .sum()
}

fn paste_bytes(tokens: &[Token]) -> Vec<u8> {
    let mut out = Vec::new();
    for t in tokens {
        match t {
            Token::Paste(b) => out.extend_from_slice(b),
            Token::Bytes(b) if b.is_empty() => {}
            other => panic!("paste body leaked out of Token::Paste as {other:?}"),
        }
    }
    out
}

/// Remove every `ESC[201~` until none remain (a removal can splice a new one).
fn strip_terminators(mut body: Vec<u8>) -> Vec<u8> {
    while let Some(p) = body.windows(PASTE_END.len()).position(|w| w == PASTE_END) {
        body.drain(p..p + PASTE_END.len());
    }
    body
}

fuzz_target!(|data: &[u8]| {
    let (ctrl, payload) = data.split_at(data.len().min(CTRL));

    // 1. Arbitrary stdin, arbitrary bursts, one parser.
    let tokens = feed_split(payload, ctrl, None);
    assert!(
        emitted_len(&tokens) <= payload.len(),
        "parser emitted more bytes than it was fed: {tokens:?}"
    );

    // 2 + 3. The payload as a bracketed-paste body.
    let body = strip_terminators(payload.to_vec());
    let mut stream = PASTE_START.to_vec();
    stream.extend_from_slice(&body);
    stream.extend_from_slice(PASTE_END);

    let whole = InputParser::default().feed(&stream);
    assert_eq!(
        paste_bytes(&whole),
        body,
        "whole-stream paste body mismatch"
    );

    let split = feed_split(&stream, ctrl, Some(1));
    assert_eq!(paste_bytes(&split), body, "split-feed paste body mismatch");
});
