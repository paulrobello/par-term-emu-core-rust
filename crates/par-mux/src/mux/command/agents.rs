//! Agent lifecycle command parsers: `agent-start`, `agent-prompt`,
//! `agent-explain` (herdr `agent start` / `agent prompt` / `agent explain`).

use super::*;

/// `agent-start` waits this long for detection when no `-T` is given
/// (herdr's `DEFAULT_AGENT_START_TIMEOUT`).
pub const AGENT_START_DEFAULT_TIMEOUT_MS: u64 = 30_000;

/// The longest `agent-start -T` accepted (herdr's `MAX_AGENT_START_TIMEOUT`).
pub const AGENT_START_MAX_TIMEOUT_MS: u64 = 300_000;

/// The states `agent-prompt -u` may name.
const PROMPT_UNTIL_STATES: &[&str] = &["working", "blocked", "done", "idle"];

/// Whether `kind` is a usable agent label: it rides the space-split roster
/// and `%agent-state-changed` lines, so it is one lowercase word.
fn valid_agent_kind(kind: &str) -> bool {
    let mut chars = kind.chars();
    matches!(chars.next(), Some('a'..='z' | '0'..='9'))
        && kind.len() <= 64
        && chars
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'))
}

fn parse_timeout(name: &str, raw: Option<&str>) -> Result<Option<u64>, String> {
    raw.map(|raw| {
        raw.parse::<u64>()
            .map_err(|_| format!("{name}: invalid -T milliseconds: {raw}"))
    })
    .transpose()
}

pub(super) fn pane_flag(name: &str, raw: Option<&str>) -> Result<Target<PaneId>, String> {
    match raw {
        None => Err(format!("{name} requires -t")),
        Some("") => Err(format!("{name}: -t requires a non-empty name")),
        Some(raw) => Target::parse(raw).map_err(|_| format!("invalid pane target: {raw}")),
    }
}

/// `agent-start -t <pane> [-T ms] [-x program] <kind> [arg …]`.
pub(super) fn parse_agent_start(a: &Args<'_>) -> Result<MuxCommand, String> {
    use LeadingFlag::Valued;
    let lead = split_leading_flags(a.line, a.name, &[Valued("-t"), Valued("-T"), Valued("-x")])?;
    let pane = pane_flag(a.name, lead.value("-t"))?;
    let timeout_ms = parse_timeout(a.name, lead.value("-T"))?;
    if let Some(ms) = timeout_ms {
        if ms == 0 || ms > AGENT_START_MAX_TIMEOUT_MS {
            return Err(format!(
                "{}: -T must be between 1 and {AGENT_START_MAX_TIMEOUT_MS} milliseconds",
                a.name
            ));
        }
    }
    let program = match lead.value("-x") {
        Some("") => return Err(format!("{}: -x requires a non-empty program", a.name)),
        other => other.map(str::to_string),
    };
    let mut words = shell_split(lead.tail).into_iter();
    let Some(kind) = words.next() else {
        return Err(format!("{} requires an agent kind", a.name));
    };
    if !valid_agent_kind(&kind) {
        return Err(format!(
            "{}: invalid agent kind {kind:?} (lowercase letters, digits, '-', '_', '.')",
            a.name
        ));
    }
    let args: Vec<String> = words.collect();
    if args
        .iter()
        .chain(program.iter())
        .any(|word| word.chars().any(char::is_control))
    {
        return Err(format!(
            "{}: agent arguments must not contain control characters",
            a.name
        ));
    }
    Ok(MuxCommand::AgentStart {
        pane,
        kind,
        program,
        args,
        timeout_ms,
    })
}

/// `agent-prompt -t <pane> [-w] [-u state,…] [-T ms] [--] <text>`. The text
/// is the raw rest of the line, so a prompt needs no quoting; a text that
/// is exactly one quoted word is unquoted.
pub(super) fn parse_agent_prompt(a: &Args<'_>) -> Result<MuxCommand, String> {
    use LeadingFlag::{Bare, Valued};
    let lead = split_leading_flags(
        a.line,
        a.name,
        &[Valued("-t"), Bare("-w"), Valued("-u"), Valued("-T")],
    )?;
    let pane = pane_flag(a.name, lead.value("-t"))?;
    let wait = lead.has("-w");
    let timeout_ms = parse_timeout(a.name, lead.value("-T"))?;
    if timeout_ms.is_some() && !wait {
        return Err(format!("{}: -T requires -w", a.name));
    }
    let until = match lead.value("-u") {
        None => Vec::new(),
        Some(_) if !wait => return Err(format!("{}: -u requires -w", a.name)),
        Some(raw) => {
            let mut states = Vec::new();
            for state in raw.split(',').map(str::trim) {
                if !PROMPT_UNTIL_STATES.contains(&state) {
                    return Err(format!(
                        "{}: invalid -u state {state:?} (working, blocked, done, idle)",
                        a.name
                    ));
                }
                if !states.iter().any(|s| s == state) {
                    states.push(state.to_string());
                }
            }
            states
        }
    };
    let text = unquote_lone_quoted_word(lead.tail);
    if text.trim().is_empty() {
        return Err(format!("{} requires prompt text", a.name));
    }
    Ok(MuxCommand::AgentPrompt {
        pane,
        text,
        wait,
        until,
        timeout_ms,
    })
}

/// `agent-explain -t <pane>`.
pub(super) fn parse_agent_explain(a: &Args<'_>) -> Result<MuxCommand, String> {
    use LeadingFlag::Valued;
    let lead = split_leading_flags(a.line, a.name, &[Valued("-t")])?;
    if !lead.tail.is_empty() {
        return Err(format!("{}: unexpected argument {:?}", a.name, lead.tail));
    }
    Ok(MuxCommand::AgentExplain {
        pane: pane_flag(a.name, lead.value("-t"))?,
    })
}

/// `adopt-pane -t <pane> <kind>` — label a pane the user launched an agent
/// in by hand. The kind must be a valid agent label; the dispatch refuses
/// kinds the scrape tier has no patterns for.
pub(super) fn parse_adopt_pane(a: &Args<'_>) -> Result<MuxCommand, String> {
    use LeadingFlag::Valued;
    let lead = split_leading_flags(a.line, a.name, &[Valued("-t")])?;
    let mut words = shell_split(lead.tail).into_iter();
    let Some(kind) = words.next() else {
        return Err(format!("{} requires an agent kind", a.name));
    };
    if !valid_agent_kind(&kind) {
        return Err(format!(
            "{}: invalid agent kind {kind:?} (lowercase letters, digits, '-', '_', '.')",
            a.name
        ));
    }
    if let Some(extra) = words.next() {
        return Err(format!("{}: unexpected argument {extra:?}", a.name));
    }
    Ok(MuxCommand::AdoptPane {
        pane: pane_flag(a.name, lead.value("-t"))?,
        kind,
    })
}
