#![no_main]

//! OSC 1337 file-transfer state machine (ENH-064). Drives `Terminal::process`
//! bytes — never private APIs — through `MultipartFile=` starts, `FilePart=`
//! chunks (valid and malformed base64, 4-char multiples by construction when
//! valid), single-sequence `File=` inline images, resets, and raw garbage
//! payloads, in arbitrary interleavings.
//!
//! The oracle reads the SEC-001 caps from the manager at run time
//! (`fuzz_file_transfer_caps`) instead of hard-coding them, so the invariants
//! survive cap-value changes: active count, completed ring length,
//! per-transfer retained bytes, and total retained bytes stay within
//! caps × per-transfer cap, and the inline multipart buffer (which lives
//! outside the manager) stays bounded by the harness's own declared-size
//! clamp. A violation means some input grew retained memory past the caps —
//! the exact class SEC-001 closed.

use libfuzzer_sys::fuzz_target;
use par_term_emu_core_rust::terminal::Terminal;

/// Per-transfer cap applied at harness start so the size-cap enforcement path
/// is reachable with small inputs instead of 50 MiB appends. The enforcement
/// logic is value-independent; the oracle reads the live cap back.
const FUZZ_MAX_TRANSFER_SIZE: usize = 64 * 1024;

/// Upper bound for `size=` parameters emitted in fuzz starts, and the basis
/// for the multipart-buffer bound in the oracle.
const FUZZ_MAX_DECLARED_SIZE: usize = 4 * 1024 * 1024;

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.len() > 1) * u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.len() > 2) * u32::from(chunk.get(2).copied().unwrap_or(0));
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64[((n >> 18) & 63) as usize] as char);
        out.push(B64[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            B64[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Apply the oracle: the SEC-001 invariants, evaluated against the caps the
/// manager itself reports.
fn check_invariants(terminal: &Terminal) {
    let (max_transfer_size, max_active, max_completed, buffer_bytes) =
        terminal.fuzz_file_transfer_caps();
    let active = terminal.get_active_transfers();
    let completed = terminal.get_completed_transfers();

    assert!(
        active.len() <= max_active,
        "active transfer count {} exceeds cap {}",
        active.len(),
        max_active
    );
    assert!(
        completed.len() <= max_completed,
        "completed ring length {} exceeds cap {}",
        completed.len(),
        max_completed
    );
    let mut retained = 0usize;
    for t in active.iter().chain(completed.iter()) {
        assert!(
            t.data.len() <= max_transfer_size,
            "transfer {} retains {} bytes over the per-transfer cap {}",
            t.id,
            t.data.len(),
            max_transfer_size
        );
        retained += t.data.len();
    }
    assert!(
        retained <= max_active.saturating_add(max_completed) * max_transfer_size,
        "retained bytes {} exceed the (active + ring) x per-transfer bound",
        retained
    );
    assert!(
        buffer_bytes <= 2 * FUZZ_MAX_DECLARED_SIZE + 8192,
        "inline multipart buffer retains {} bytes, past the declared-size clamp slack",
        buffer_bytes
    );
}

fn fuzz(data: &[u8]) {
    let mut terminal = Terminal::with_scrollback(80, 24, 1000);
    terminal.set_max_transfer_size(FUZZ_MAX_TRANSFER_SIZE);

    let mut i = 0usize;
    while i < data.len() {
        let op = data[i];
        i += 1;
        if i >= data.len() {
            break;
        }
        let len = (data[i] as usize) & 0x3f;
        i += 1;
        let end = (i + len).min(data.len());
        let payload = &data[i..end];
        i = end;

        match op & 0x07 {
            // MultipartFile start with sub-cap or missing size: half the
            // derived sizes pass admission, absent size hits the
            // missing-size fail path on the first FilePart.
            0 | 1 => {
                let inline = payload.first().is_some_and(|b| b & 1 == 1);
                let size = payload
                    .get(1..5)
                    .map(|b| {
                        usize::from_le_bytes([b[0], b[1], b[2], b[3], 0, 0, 0, 0])
                            % (2 * FUZZ_MAX_TRANSFER_SIZE)
                    })
                    .filter(|s| *s > 0);
                emit_multipart_start(&mut terminal, inline, size, &b64_encode(payload));
            }
            // Declared size deliberately over the per-transfer cap: the
            // reject-at-start admission path.
            2 => {
                let inline = payload.first().is_some_and(|b| b & 1 == 1);
                let size = payload
                    .get(1..5)
                    .map(|b| {
                        FUZZ_MAX_TRANSFER_SIZE
                            + usize::from_le_bytes([b[0], b[1], b[2], b[3], 0, 0, 0, 0])
                                % (FUZZ_MAX_DECLARED_SIZE - FUZZ_MAX_TRANSFER_SIZE)
                    })
                    .filter(|s| *s > 0);
                emit_multipart_start(&mut terminal, inline, size, &b64_encode(payload));
            }
            // FilePart with well-formed base64 (4-char multiples by construction).
            3 => {
                let seq = format!("\x1b]1337;FilePart={}\x07", b64_encode(payload));
                terminal.process(seq.as_bytes());
            }
            // FilePart with raw bytes: invalid base64, bad padding, control
            // characters — the decode-failure path.
            4 => {
                let seq = format!(
                    "\x1b]1337;FilePart={}\x07",
                    String::from_utf8_lossy(payload)
                );
                terminal.process(seq.as_bytes());
            }
            // Single-sequence File= inline image: params + payload in one shot.
            5 => {
                let seq = format!(
                    "\x1b]1337;File=name={};size={};inline=1:{}\x07",
                    b64_encode(b"f.bin"),
                    payload.len(),
                    b64_encode(payload)
                );
                terminal.process(seq.as_bytes());
            }
            // RIS reset interleaved mid-transfer: the restart-orphan surface
            // SEC-001 was about. Host caps survive RIS by design (ARC-100).
            6 => {
                terminal.process(b"\x1bc");
                terminal.process(b"noise after reset ");
            }
            // Raw garbage as the OSC 1337 payload: unknown commands, split
            // colons, unicode, junk — the router's fallthrough.
            _ => {
                let seq = format!("\x1b]1337;{}\x07", String::from_utf8_lossy(payload));
                terminal.process(seq.as_bytes());
            }
        }
        check_invariants(&terminal);
    }
    check_invariants(&terminal);
}

fn emit_multipart_start(terminal: &mut Terminal, inline: bool, size: Option<usize>, name: &str) {
    let inline_param = u8::from(inline);
    let size_param = match size {
        Some(s) => format!(";size={s}"),
        None => String::new(),
    };
    let seq = format!("\x1b]1337;MultipartFile=inline={inline_param}{size_param};name={name}\x07");
    terminal.process(seq.as_bytes());
}

fuzz_target!(|data: &[u8]| {
    fuzz(data);
});
