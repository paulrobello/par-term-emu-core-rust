//! Pane command parsers, plus the client-facing list-agents and
//! refresh-client.

use super::*;

pub(super) fn parse_list_panes(a: &Args<'_>) -> Result<MuxCommand, String> {
    let a = a.with_value_flags(LIST_PANES_VALUE_FLAGS);
    // `-t <window|session|pane>` scopes the listing (R7): a window lists
    // its own panes, a session its active window's, a pane its owning
    // window's — every scope a client can name. The bare form stays
    // global.
    let target = a.any_target_opt("-t", "window")?;
    reject_positionals(&a, LIST_PANES_VALUE_FLAGS)?;
    Ok(MuxCommand::ListPanes {
        target,
        format: a.quoted_flag("-F")?,
    })
}

/// tmux's value-taking `list-panes` flags (QA-219's rule). `-F` rides
/// here too — reject_positionals must skip its value while the parser
/// above reads the format out of it.
pub(super) const LIST_PANES_VALUE_FLAGS: &[&str] = &["-t", "-F", "-f"];

pub(super) fn parse_list_agents(_a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::ListAgents)
}

/// `report-state`'s value-taking flags.
const REPORT_STATE_VALUE_FLAGS: &[&str] = &["-t", "-a", "-s", "-m"];

/// `report-state -t <pane> [-a agent] [-s state] [-m message]` — the pane's
/// own agent-state report, the control-verb spelling of the hook
/// `pane.report_agent` shape. `-s` accepts the hook states minus `unknown`
/// (unknown is the absence of a claim, never a stored state); omitting `-s`
/// reports idle, which the shared report path derives to `done` for a pane
/// no viewer shows — the same semantics a hook's idle takes.
pub(super) fn parse_report_state(a: &Args<'_>) -> Result<MuxCommand, String> {
    let a = a.with_value_flags(REPORT_STATE_VALUE_FLAGS);
    let pane = a.pane("-t")?;
    let agent = a.quoted_flag("-a")?;
    let message = a.quoted_flag("-m")?;
    let state = match a.flag("-s") {
        Some(raw) => {
            let state = raw.trim();
            if !matches!(state, "working" | "blocked" | "question" | "done" | "idle") {
                return Err(format!(
                    "{}: invalid state: {state} (working, blocked, question, done, or idle)",
                    a.name
                ));
            }
            Some(state.to_string())
        }
        None => None,
    };
    if message.is_some() && state.is_none() {
        return Err(format!("{}: -m requires -s", a.name));
    }
    reject_positionals(&a, REPORT_STATE_VALUE_FLAGS)?;
    Ok(MuxCommand::ReportState {
        pane,
        agent,
        state,
        message,
    })
}

/// tmux's value-taking `events` flag (`--since`).
pub(super) const EVENTS_VALUE_FLAGS: &[&str] = &["--since"];

/// `events [--since <seq>]` — the replayable lifecycle ring read.
pub(super) fn parse_events(a: &Args<'_>) -> Result<MuxCommand, String> {
    let a = a.with_value_flags(EVENTS_VALUE_FLAGS);
    let since =
        match a.flag("--since") {
            Some(raw) => Some(raw.parse::<u64>().map_err(|_| {
                format!("{}: --since expects a sequence number, got {raw:?}", a.name)
            })?),
            None => None,
        };
    reject_positionals(&a, EVENTS_VALUE_FLAGS)?;
    Ok(MuxCommand::Events { since })
}

pub(super) fn parse_client_snapshot(a: &Args<'_>) -> Result<MuxCommand, String> {
    let window = a.window("-t")?;
    reject_positionals(a, &["-t"])?;
    Ok(MuxCommand::ClientSnapshot { window })
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
        // `-I`: no tmux refresh-client flag uses the letter.
        chrome: match a.flag("-I") {
            Some(raw) => Some(
                crate::mux::layout::PaneChrome::parse_wire(&raw)
                    .map_err(|err| format!("{}: {err}", a.name))?,
            ),
            None => None,
        },
        // `-G`: the seed's graphics section. tmux's refresh-client has no
        // `-G`; the streaming mirror is the only issuer.
        graphics: a.has_flag("-G"),
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
    // The first word that IS `-t` (raw bytes, as in the flag scanners: a
    // quoted `'-t'` is a payload word) anchors the split. Its value is
    // one shell word under the shared quoting grammar, so a quoted `%0`
    // parses exactly like `pane-info`'s target and a quoted spaced name
    // rides whole — while the payload stays the RAW remainder after that
    // word, its quoting intact for the payload's own grammar.
    let mut pos = a.line.len() - rest.len();
    while let Some((start, end, _)) = next_shell_word(a.line, pos) {
        if &a.line[start..end] != "-t" {
            pos = end;
            continue;
        }
        let (vstart, vend, unquoted) =
            next_shell_word(a.line, end).ok_or_else(|| format!("{} requires -t", a.name))?;
        // Only a value that opens a quote is unquoted (the `quoted_flag`
        // rule): a bare token keeps its flat bytes.
        let raw = &a.line[vstart..vend];
        let target_value = if raw.starts_with(['\'', '"']) {
            unquoted
        } else {
            raw.to_string()
        };
        let pane: Target<PaneId> = Target::parse(&target_value)
            .map_err(|_| format!("invalid pane target: {target_value}"))?;
        let keys = parse_send_keys_payload(&a.line[vend..])?;
        return Ok(MuxCommand::SendKeys { pane, keys });
    }
    Err(format!("{} requires -t", a.name))
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

/// `split-window`'s value-taking flags.
const SPLIT_WINDOW_VALUE_FLAGS: &[&str] = &["-t", "-c", "-p"];

pub(super) fn parse_split_window(a: &Args<'_>) -> Result<MuxCommand, String> {
    // The trailing start command (R4): tmux's positional form. The
    // geometry flags are valueless, `-t`/`-c`/`-p` take values. tmux's
    // other value flags are refused, so their value is never read as the
    // start command (`-l 30` would otherwise run `30`).
    let (command, a) = a.start_command(
        SPLIT_WINDOW_VALUE_FLAGS,
        &[("-l", ""), ("-e", ""), ("-F", "")],
    )?;
    let target = a.any_target("-t", "pane")?;
    let (direction, percent) = parse_split_geometry(&a)?;
    Ok(MuxCommand::SplitWindow {
        target,
        direction,
        percent,
        before: a.has_flag("-b"),
        start_dir: a.quoted_flag("-c")?,
        command,
    })
}

pub(super) fn parse_select_pane(a: &Args<'_>) -> Result<MuxCommand, String> {
    let a = a.with_value_flags(&["-t", "-T"]);
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
        mirror: a.has_flag("-M"),
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
    // The direction flags' cell count is optional, so they are not listed:
    // `-L -Z` must stay two flags.
    let a = a.with_value_flags(&["-t", "-x", "-y"]);
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

/// `join-pane`'s value-taking flags.
const JOIN_PANE_VALUE_FLAGS: &[&str] = &["-s", "-t", "-p"];

/// `join-pane -s <pane> -t <pane> [-h|-v] [-p 1-99]`. Every other word is
/// refused, as `split-window` refuses tmux's `-l`: a dropped `-l 30` once
/// joined at 50% without a word.
pub(super) fn parse_join_pane(a: &Args<'_>) -> Result<MuxCommand, String> {
    let a = a.with_value_flags(JOIN_PANE_VALUE_FLAGS);
    reject_leftovers(&a, &["-h", "-v"])?;
    let source = a.pane("-s")?;
    let target = a.pane("-t")?;
    let (direction, percent) = parse_split_geometry(&a)?;
    Ok(MuxCommand::JoinPane {
        source,
        target,
        direction,
        percent,
    })
}

/// `respawn-pane [-k] [-c dir] -t target [--] [command]`: flags lead, in
/// any order, and stop at the first command word; the command is the raw
/// rest of the line (SEC-126), a lone quoted word unquoted
/// ([`unquote_lone_quoted_word`], the same tail contract the create
/// commands' start commands follow). Never reads flags through the
/// [`Args`] helpers, which scan the whole line and so see into the
/// command.
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
    let command = (!lead.tail.is_empty()).then(|| unquote_lone_quoted_word(lead.tail));
    Ok(MuxCommand::RespawnPane {
        pane,
        kill: lead.has("-k"),
        start_dir,
        command,
    })
}
