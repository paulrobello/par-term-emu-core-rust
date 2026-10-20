//! Buffer, capture, color, and daemon-info command parsers.

use super::*;

pub(super) fn parse_capture_pane(a: &Args<'_>) -> Result<MuxCommand, String> {
    let a = a.with_value_flags(&["-t", "-S", "-E"]);
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

/// `wait-output -t <pane> (-m TEXT | -r REGEX) [-T ms] [-S start] [-E end]`.
/// Read through [`split_leading_flags`] so a quoted pattern containing
/// spaces or flag-like words stays one value.
pub(super) fn parse_wait_output(a: &Args<'_>) -> Result<MuxCommand, String> {
    use LeadingFlag::Valued;
    let lead = split_leading_flags(
        a.line,
        a.name,
        &[
            Valued("-t"),
            Valued("-m"),
            Valued("-r"),
            Valued("-T"),
            Valued("-S"),
            Valued("-E"),
        ],
    )?;
    if !lead.tail.is_empty() {
        return Err(format!("{}: unexpected argument {:?}", a.name, lead.tail));
    }
    let raw = match lead.value("-t") {
        None => return Err(format!("{} requires -t", a.name)),
        Some("") => return Err(format!("{}: -t requires a non-empty name", a.name)),
        Some(raw) => raw,
    };
    let pane = Target::parse(raw).map_err(|_| format!("invalid pane target: {raw}"))?;
    let pattern = match (lead.value("-m"), lead.value("-r")) {
        (Some(_), Some(_)) => return Err(format!("{}: -m and -r are exclusive", a.name)),
        (None, None) => return Err(format!("{} requires -m TEXT or -r REGEX", a.name)),
        (Some(""), None) | (None, Some("")) => {
            return Err(format!("{}: the pattern must be non-empty", a.name))
        }
        (Some(text), None) => OutputPattern::Substring(text.to_string()),
        (None, Some(re)) => {
            regex::Regex::new(re).map_err(|err| format!("{}: invalid -r: {err}", a.name))?;
            OutputPattern::Regex(re.to_string())
        }
    };
    let line_offset = |flag: &str| -> Result<Option<i64>, String> {
        lead.value(flag)
            .map(|raw| {
                raw.parse::<i64>()
                    .map_err(|_| format!("{}: invalid {flag}: {raw}", a.name))
            })
            .transpose()
    };
    let timeout_ms = lead
        .value("-T")
        .map(|raw| {
            raw.parse::<u64>()
                .map_err(|_| format!("{}: invalid -T milliseconds: {raw}", a.name))
        })
        .transpose()?;
    Ok(MuxCommand::WaitOutput {
        pane,
        pattern,
        timeout_ms,
        start_line: line_offset("-S")?,
        end_line: line_offset("-E")?,
    })
}

pub(super) fn parse_set_buffer(a: &Args<'_>) -> Result<MuxCommand, String> {
    // `-H` carries the payload as hex bytes, one byte per token — the same
    // form `send-keys -H` uses. The control wire is line-delimited, so
    // content containing a newline cannot ride the positional form in ANY
    // quoting; hex is the byte-exact escape hatch (clipboard sync).
    // Only a LEADING `-H` selects hex: a positional payload may contain the
    // word (`set-buffer 'grep -H x'`).
    if let Some(("-H", tokens)) = a.args.split_first().map(|(f, t)| (*f, t)) {
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

/// `list-commands [-J]`: `-J` selects the JSON schema reply. tmux's own
/// `-F format` is not taken: the reply rows have no format variables.
pub(super) fn parse_list_commands(a: &Args<'_>) -> Result<MuxCommand, String> {
    for word in a.args {
        if *word != "-J" {
            return Err(format!("{}: unknown flag {word}", a.name));
        }
    }
    Ok(MuxCommand::ListCommands {
        schema: a.has_flag("-J"),
    })
}

/// cap: Characters of a `show-notification` title (herdr's 80); longer is truncated.
pub const NOTIFICATION_TITLE_MAX: usize = 80;

/// cap: Characters of a `show-notification` body (herdr's 240); longer is truncated.
pub const NOTIFICATION_BODY_MAX: usize = 240;

/// `show-notification [-b body] [--] <title>`: the title is the rest of
/// the line (one quoted word is unquoted, like `agent-prompt`'s text), so
/// a title starting with `-` follows `--`. Control characters fold to
/// spaces and both fields are trimmed and truncated to their caps.
pub(super) fn parse_show_notification(a: &Args<'_>) -> Result<MuxCommand, String> {
    use LeadingFlag::Valued;
    let lead = split_leading_flags(a.line, a.name, &[Valued("-b")])?;
    let raw = match next_shell_word(lead.tail, 0) {
        Some((0, end, word)) if end == lead.tail.len() && lead.tail.starts_with(['\'', '"']) => {
            word
        }
        _ => lead.tail.to_string(),
    };
    let clean = |text: &str, max: usize| -> String {
        text.chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect::<String>()
            .trim()
            .chars()
            .take(max)
            .collect::<String>()
            .trim_end()
            .to_string()
    };
    let title = clean(&raw, NOTIFICATION_TITLE_MAX);
    if title.is_empty() {
        return Err(format!("{} requires a title", a.name));
    }
    let body = clean(lead.value("-b").unwrap_or_default(), NOTIFICATION_BODY_MAX);
    Ok(MuxCommand::ShowNotification { title, body })
}

/// `reload-config` takes no arguments — it always re-reads the canonical
/// config path; keeping it argument-free keeps the daemon's resolution
/// (env included) the single source of truth.
pub(super) fn parse_reload_config(a: &Args<'_>) -> Result<MuxCommand, String> {
    reject_positionals(a, &[])?;
    Ok(MuxCommand::ReloadConfig)
}
