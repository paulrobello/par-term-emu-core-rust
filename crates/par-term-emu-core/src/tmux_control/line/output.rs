//! Pane output, damage, and flow-control notifications.

use crate::tmux_control::TmuxNotification;

pub(super) fn parse_output(args: &str) -> Option<TmuxNotification> {
    let parts: Vec<&str> = args.splitn(2, ' ').collect();
    if parts.is_empty() {
        return None;
    }

    let pane_id = parts[0].to_string();
    // The optional `gen=<u64>` second token (card 01a205ba): the chunk's
    // damage generation, stamped by a same-vintage daemon for the attach
    // client's replay fence. Recognized only when the token after the
    // pane id is exactly `gen=` + digits; output data that itself begins
    // with such text misparses (accepted — the same-binary pairing is
    // the deployment model, docs/MUX.md).
    let (generation, data) = match parts.get(1) {
        None => (None, Vec::new()),
        Some(rest) => {
            let (token, data) = rest.split_once(' ').unwrap_or((rest, ""));
            match token.strip_prefix("gen=").map(str::parse::<u64>) {
                Some(Ok(generation)) => (Some(generation), super::output::unescape_output(data)),
                _ => (None, super::output::unescape_output(rest)),
            }
        }
    };

    Some(TmuxNotification::Output {
        pane_id,
        generation,
        data,
    })
}

/// `%pane-damage <pane_id> <generation> [<start>-<end>…]` — the damage
/// report for the `%output` line immediately before it (0-based,
/// inclusive, viewport-relative rows; ascending). A line with no range
/// tokens parses as an empty set: the generation advanced, no viewport
/// row changed. `None` on any malformed token — an unparseable damage
/// line degrades to no notification, and its `%output` still applied
/// (a consumer without damage info full-scans).
pub(super) fn parse_output_damage(args: &str) -> Option<TmuxNotification> {
    let mut parts = args.split_whitespace();
    let pane_id = parts.next()?.to_string();
    let generation = parts.next()?.parse().ok()?;
    let mut ranges = Vec::new();
    for token in parts {
        let (start, end) = token.split_once('-')?;
        let start = start.parse().ok()?;
        let end = end.parse().ok()?;
        if start > end {
            return None;
        }
        ranges.push((start, end));
    }
    Some(TmuxNotification::OutputDamage {
        pane_id,
        generation,
        ranges,
    })
}

/// Unescape tmux output data (octal escape sequences)
pub(in crate::tmux_control) fn unescape_output(s: &str) -> Vec<u8> {
    let mut result = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            // Check for octal escape sequence (\nnn)
            if i + 3 < bytes.len()
                && bytes[i + 1].is_ascii_digit()
                && bytes[i + 2].is_ascii_digit()
                && bytes[i + 3].is_ascii_digit()
            {
                // Parse three octal digits
                let octal_str = String::from_utf8_lossy(&bytes[i + 1..i + 4]);
                if let Ok(byte_val) = u8::from_str_radix(&octal_str, 8) {
                    result.push(byte_val);
                    i += 4;
                    continue;
                }
            }
            // Not a valid octal sequence, treat as literal backslash
            result.push(b'\\');
            i += 1;
        } else {
            result.push(bytes[i]);
            i += 1;
        }
    }

    result
}

pub(super) fn parse_extended_output(args: &str) -> Option<TmuxNotification> {
    let parts: Vec<&str> = args.splitn(3, ' ').collect();
    if parts.len() < 3 {
        return None;
    }

    let pane_id = parts[0].to_string();
    let delay_ms = parts[1].parse().ok()?;
    // tmux(1): `pane-id age ... : value` — arguments up to the first
    // lone `:` are for future use and ignored; the value follows the
    // `: ` separator. A line with no lone colon keeps the legacy
    // leading-`:`-stripped reading.
    let rest = parts[2];
    let data_part = if let Some(value) = rest.strip_prefix(": ") {
        value
    } else if let Some(pos) = rest.find(" : ") {
        &rest[pos + 3..]
    } else {
        rest.strip_prefix(':').unwrap_or(rest)
    };
    let data = super::output::unescape_output(data_part);

    Some(TmuxNotification::ExtendedOutput {
        pane_id,
        delay_ms,
        data,
    })
}

pub(super) fn parse_pause(args: &str) -> Option<TmuxNotification> {
    Some(TmuxNotification::Pause {
        pane_id: args.trim().to_string(),
    })
}

pub(super) fn parse_continue(args: &str) -> Option<TmuxNotification> {
    Some(TmuxNotification::Continue {
        pane_id: args.trim().to_string(),
    })
}
