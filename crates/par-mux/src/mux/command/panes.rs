//! Pane command parsers, plus the client-facing list-agents and
//! refresh-client.

use super::*;

pub(super) fn parse_list_panes(a: &Args<'_>) -> Result<MuxCommand, String> {
    // `-t <window>` scopes the listing to one window (tmux's
    // `list-panes -t <window>`); the bare form stays global. Any other
    // positional is a client bug the reject_positionals rule exists for.
    let window = match a.quoted_flag("-t")? {
        Some(raw) => {
            Some(Target::parse(&raw).map_err(|_| format!("invalid window target: {raw}"))?)
        }
        None => None,
    };
    reject_positionals(a, LIST_PANES_VALUE_FLAGS)?;
    Ok(MuxCommand::ListPanes { window })
}

/// tmux's value-taking `list-panes` flags (QA-219's rule): the target, and
/// the format flags a tmux-shaped sender may emit that this daemon ignores
/// rather than misreading as pane names.
pub(super) const LIST_PANES_VALUE_FLAGS: &[&str] = &["-t", "-F", "-f"];

pub(super) fn parse_list_agents(_a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::ListAgents)
}

pub(super) fn parse_kill_pane(a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::KillPane {
        pane: a.pane("-t")?,
    })
}

pub(super) fn parse_refresh_client(a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::RefreshClient {
        // `-t` is optional: the attach handshake reports its renderer size
        // (`-C`/`-p`) BEFORE it has resolved which pane it shows, so the
        // target-less form is a pure size report. Replay stays bound to the
        // `-t` form.
        pane: match a.quoted_flag("-t")? {
            Some(raw) => {
                Some(Target::parse(&raw).map_err(|_| format!("invalid pane target: {raw}"))?)
            }
            None => None,
        },
        size: a.size_pair("-C", (MAX_CLIENT_COLS, MAX_CLIENT_ROWS))?,
        cell_pixels: a.size_pair("-p", (MAX_CELL_PIXELS, MAX_CELL_PIXELS))?,
    })
}

pub(super) fn parse_send_keys(a: &Args<'_>) -> Result<MuxCommand, String> {
    // trim_start first: the name token `split_whitespace` found may sit
    // behind leading whitespace, so it prefixes the TRIMMED line, not the
    // raw one (fuzz-found: " send-keys …" panicked the old strip).
    let rest = a
        .line
        .trim_start()
        .strip_prefix(a.name)
        .expect("the command name prefixes the trimmed line");
    let (target_value, payload_raw) =
        split_after_flag(rest, "-t").ok_or_else(|| format!("{} requires -t", a.name))?;
    // The raw-line split takes one whitespace-delimited token, so a
    // send-keys target is a typed id or a SINGLE-WORD name — a spaced name
    // cannot survive next to the free-text payload, and quoting it would
    // eat into the payload's own quoting.
    let pane: Target<PaneId> =
        Target::parse(target_value).map_err(|_| format!("invalid pane target: {target_value}"))?;
    let keys = parse_send_keys_payload(payload_raw)?;
    Ok(MuxCommand::SendKeys { pane, keys })
}

/// The split arrangement `split-window` and `join-pane` share: `-h` puts
/// the pane beside the target (our Vertical orientation), `-v`/default
/// below it (Horizontal) — tmux's flags name the arrangement, not the
/// divider — and `-p` is the pane's share, 1-99 (default 50).
pub(super) fn parse_split_geometry(a: &Args<'_>) -> Result<(SplitDirection, u32), String> {
    let direction = if a.has_flag("-h") {
        SplitDirection::Vertical
    } else {
        SplitDirection::Horizontal
    };
    let percent = match a.flag("-p") {
        Some(raw) => {
            let percent: u32 = raw
                .parse()
                .map_err(|_| format!("invalid percentage: {raw}"))?;
            if !(1..=99).contains(&percent) {
                return Err(format!("percentage must be 1-99: {raw}"));
            }
            percent
        }
        None => 50,
    };
    Ok((direction, percent))
}

pub(super) fn parse_split_window(a: &Args<'_>) -> Result<MuxCommand, String> {
    let target = a.any_target("-t", "pane")?;
    let (direction, percent) = parse_split_geometry(a)?;
    Ok(MuxCommand::SplitWindow {
        target,
        direction,
        percent,
        before: a.has_flag("-b"),
        start_dir: a.quoted_flag("-c")?,
    })
}

pub(super) fn parse_select_pane(a: &Args<'_>) -> Result<MuxCommand, String> {
    let pane = a.pane("-t")?;
    // `-T` carries the user title. Unlike a NAME flag, an explicitly empty
    // value is meaningful — `-T ''` is the clear operation — so the value
    // is read through the quoting grammar without `quoted_flag`'s
    // non-empty guard.
    let title = if a.has_flag("-T") {
        match a.quoted_flag_allowing_empty("-T")? {
            Some(value) => Some(value),
            // `-T` present with nothing after it cannot mean anything.
            None => return Err(format!("{}: -T requires a value", a.name)),
        }
    } else {
        None
    };
    Ok(MuxCommand::SelectPane { pane, title })
}

pub(super) fn parse_pane_title(a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::PaneTitle {
        pane: a.pane("-t")?,
    })
}

pub(super) fn parse_pane_info(a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::PaneInfo {
        pane: a.pane("-t")?,
    })
}

pub(super) fn parse_pane_exited_replay(_a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::PaneExitedReplay)
}

pub(super) fn parse_clear_history(a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::ClearHistory {
        pane: a.pane("-t")?,
    })
}

pub(super) fn parse_resize_pane(a: &Args<'_>) -> Result<MuxCommand, String> {
    let pane = a.pane("-t")?;
    // The zoom form: -Z toggles, standalone.
    if a.has_flag("-Z") {
        if a.has_flag("-x")
            || a.has_flag("-y")
            || ["-L", "-R", "-U", "-D"].iter().any(|f| a.has_flag(f))
        {
            return Err("resize-pane: -Z cannot combine with -L -R -U -D or -x/-y".to_string());
        }
        return Ok(MuxCommand::ResizePane {
            pane,
            adjustment: ResizeAdjustment::Zoom,
        });
    }
    // The absolute form: -x COLS and/or -y ROWS, at least one.
    let cols = a.size("-x")?;
    let rows = a.size("-y")?;
    // tmux takes one direction flag; the first of the four wins.
    let direction_flag = [
        ("-L", ResizeDirection::Left),
        ("-R", ResizeDirection::Right),
        ("-U", ResizeDirection::Up),
        ("-D", ResizeDirection::Down),
    ]
    .into_iter()
    .find(|(flag, _)| a.has_flag(flag));
    if (cols.is_some() || rows.is_some()) && direction_flag.is_some() {
        return Err("resize-pane: -x/-y cannot combine with -L -R -U -D".to_string());
    }
    let adjustment = if cols.is_some() || rows.is_some() {
        ResizeAdjustment::Absolute { cols, rows }
    } else {
        let Some((flag_name, direction)) = direction_flag else {
            return Err("resize-pane requires one of -L -R -U -D, or -x/-y".to_string());
        };
        // The cell count is the flag's value when present and
        // numeric; tmux's default adjustment is 5 cells.
        let cells = a
            .flag(flag_name)
            .and_then(|raw| raw.parse::<u32>().ok())
            .unwrap_or(5);
        ResizeAdjustment::Relative { direction, cells }
    };
    Ok(MuxCommand::ResizePane { pane, adjustment })
}

pub(super) fn parse_swap_pane(a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::SwapPanes {
        target: a.pane("-t")?,
        source: a.pane("-s")?,
    })
}

pub(super) fn parse_break_pane(a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::BreakPane {
        source: a.pane("-s")?,
        name: a.quoted_flag("-n")?,
    })
}

pub(super) fn parse_join_pane(a: &Args<'_>) -> Result<MuxCommand, String> {
    let source = a.pane("-s")?;
    let target = a.pane("-t")?;
    let (direction, percent) = parse_split_geometry(a)?;
    Ok(MuxCommand::JoinPane {
        source,
        target,
        direction,
        percent,
    })
}

/// `respawn-pane [-k] [-c dir] -t target [--] [command]`: flags lead, in
/// any order, and stop at the first command word; the command is the raw
/// rest of the line (SEC-126). Never reads flags through the [`Args`]
/// helpers, which scan the whole line and so see into the command.
pub(super) fn parse_respawn_pane(a: &Args<'_>) -> Result<MuxCommand, String> {
    use LeadingFlag::{Bare, Valued};
    let lead = split_leading_flags(a.line, a.name, &[Valued("-t"), Valued("-c"), Bare("-k")])?;
    let raw = match lead.value("-t") {
        None => return Err(format!("{} requires -t", a.name)),
        Some("") => return Err(format!("{}: -t requires a non-empty name", a.name)),
        Some(raw) => raw,
    };
    let pane = Target::parse(raw).map_err(|_| format!("invalid pane target: {raw}"))?;
    let start_dir = match lead.value("-c") {
        Some("") => return Err(format!("{}: -c requires a non-empty directory", a.name)),
        other => other.map(str::to_string),
    };
    let command = (!lead.tail.is_empty()).then(|| lead.tail.to_string());
    Ok(MuxCommand::RespawnPane {
        pane,
        kill: lead.has("-k"),
        start_dir,
        command,
    })
}
