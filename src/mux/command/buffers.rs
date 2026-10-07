//! Buffer, capture, color, and daemon-info command parsers.

use super::*;

pub(super) fn parse_capture_pane(a: &Args<'_>) -> Result<MuxCommand, String> {
    let pane = a.pane("-t")?;
    // tmux's `-S`/`-E` select a start/end line; the raw offsets are
    // kept as-is (negative counts back from the screen top into
    // history) and the server-side adapter resolves them against
    // the combined scrollback+screen buffer.
    let start_line = a.flag("-S").and_then(|raw| raw.parse::<i64>().ok());
    let end_line = a.flag("-E").and_then(|raw| raw.parse::<i64>().ok());
    // `-e` is valueless (tmux's "include escape sequences").
    let escape = a.has_flag("-e");
    Ok(MuxCommand::CapturePane {
        pane,
        start_line,
        end_line,
        escape,
    })
}

pub(super) fn parse_set_buffer(a: &Args<'_>) -> Result<MuxCommand, String> {
    // `-H` carries the payload as hex bytes, one byte per token — the same
    // form `send-keys -H` uses. The control wire is line-delimited, so
    // content containing a newline cannot ride the positional form in ANY
    // quoting; hex is the byte-exact escape hatch (clipboard sync).
    if let Some(at) = a.args.iter().position(|t| *t == "-H") {
        let tokens = &a.args[at + 1..];
        if tokens.is_empty() {
            return Err("set-buffer -H requires a payload".to_string());
        }
        let mut bytes = Vec::with_capacity(tokens.len());
        for token in tokens {
            let digits = token.strip_prefix("0x").unwrap_or(token);
            let byte =
                u8::from_str_radix(digits, 16).map_err(|_| format!("invalid hex byte: {token}"))?;
            bytes.push(byte);
        }
        let content = String::from_utf8(bytes)
            .map_err(|_| "set-buffer -H payload is not UTF-8".to_string())?;
        if content.is_empty() {
            return Err("set-buffer requires content".to_string());
        }
        return Ok(MuxCommand::SetBuffer { content });
    }
    // Positional form: the payload is the rest of the line read through
    // the quoting grammar (the same bounded `shell_split` send-keys
    // payloads use), so `'a b'` and `a b` both store `a b` and an embedded
    // quote rides the close-escape-reopen idiom. Previously the raw
    // whitespace-split tokens were joined, which kept the quotes and could
    // never carry a newline.
    let words = shell_split(a.line);
    let content = words[1..].join(" ");
    if content.is_empty() {
        return Err("set-buffer requires content".to_string());
    }
    Ok(MuxCommand::SetBuffer { content })
}

/// `rrggbb` (exactly six hex digits, case-insensitive) — the color form
/// `set-client-colors` accepts. No leading `#`: the control wire is
/// whitespace-split, and a `#` would read as a comment in some shells'
/// history expansions clients paste from.
pub(super) fn parse_hex_color(raw: &str, flag: &str) -> Result<(u8, u8, u8), String> {
    if raw.len() != 6 || !raw.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "set-client-colors: {flag} expects rrggbb, got: {raw}"
        ));
    }
    let byte = |i: usize| u8::from_str_radix(&raw[i..i + 2], 16).expect("validated hex pair");
    Ok((byte(0), byte(2), byte(4)))
}

pub(super) fn parse_set_client_colors(a: &Args<'_>) -> Result<MuxCommand, String> {
    let fg = match a.flag("-f") {
        Some(raw) => Some(parse_hex_color(&raw, "-f")?),
        None => None,
    };
    let bg = match a.flag("-b") {
        Some(raw) => Some(parse_hex_color(&raw, "-b")?),
        None => None,
    };
    if fg.is_none() && bg.is_none() {
        return Err("set-client-colors requires -f and/or -b rrggbb".to_string());
    }
    Ok(MuxCommand::SetClientColors { fg, bg })
}

pub(super) fn parse_show_buffer(_a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::ShowBuffer)
}

pub(super) fn parse_paste_buffer(a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::PasteBuffer {
        pane: a.pane("-t")?,
    })
}

pub(super) fn parse_version(_a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::Version)
}

pub(super) fn parse_list_commands(_a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::ListCommands)
}

/// `reload-config` takes no arguments — it always re-reads the canonical
/// config path; keeping it argument-free keeps the daemon's resolution
/// (env included) the single source of truth.
pub(super) fn parse_reload_config(a: &Args<'_>) -> Result<MuxCommand, String> {
    reject_positionals(a, &[])?;
    Ok(MuxCommand::ReloadConfig)
}
