//! Control-mode command parsing (client → server).

use crate::mux::ids::{PaneId, SessionId, WindowId};
use crate::mux::layout::{ResizeDirection, SplitDirection};

/// How a `resize-pane` moves a pane's borders (T4.C).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResizeAdjustment {
    /// `-L`/`-R`/`-U`/`-D` [cells]: move the bordering divider relatively;
    /// 5 cells when the flag carries no number (tmux's default adjustment).
    Relative {
        /// Which way the border moves.
        direction: ResizeDirection,
        /// Cells to move it by.
        cells: u32,
    },
    /// `-x COLS` and/or `-y ROWS`: set absolute extents — the
    /// renderer-driven form par-term sends (gateway resize), where the
    /// client knows the exact size a pane should be. At least one bound is
    /// present.
    Absolute {
        /// `-x`: the pane's width in columns.
        cols: Option<u16>,
        /// `-y`: the pane's height in rows.
        rows: Option<u16>,
    },
}

/// A command received from a control-mode client.
///
/// Phase 1 implements the four commands that prove the spine end to end.
/// Additional commands are new variants plus new dispatch arms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MuxCommand {
    /// Create a session, optionally named.
    NewSession {
        /// Session name; a default is chosen when absent.
        name: Option<String>,
    },
    /// List every live pane.
    ListPanes,
    /// List every pane a hook has claimed — the agent roster.
    ListAgents,
    /// Send keys to a pane.
    SendKeys {
        /// Target pane.
        pane: PaneId,
        /// Final bytes for the pane's PTY — key names interpreted, quotes
        /// resolved, no terminator added. `Enter` is an expressible key, not
        /// an implicit one (tmux semantics; par-mux.md Phase 4 T4.B).
        keys: Vec<u8>,
    },
    /// Kill a pane.
    KillPane {
        /// Target pane.
        pane: PaneId,
    },
    /// Replay a pane's current screen to the requesting client, or report
    /// the client's renderer size (`-C`) and resize the pane's window.
    RefreshClient {
        /// Target pane; the window it belongs to is the one a `-C` resize
        /// applies to.
        pane: PaneId,
        /// `-C WxH`: the client's renderer grid size. The window-size
        /// policy (par-mux.md Phase 4): the LATEST such report wins —
        /// par-mux has no other client-size input, so latest-attached-client
        /// and latest `-C` are the same rule here. `None` keeps the
        /// replay-only behavior.
        size: Option<(u16, u16)>,
    },
    /// Add a window to a session, optionally named.
    NewWindow {
        /// Target session; `None` means the most-recently-created one (the
        /// bare `new-window` tmux clients issue, which tmux resolves against
        /// the client's attached session — par-mux has no client-session
        /// attachment, so "newest" is the documented stand-in).
        session: Option<SessionId>,
        /// Window name; a default is chosen when absent.
        name: Option<String>,
    },
    /// Set a session's active window.
    SelectWindow {
        /// Target window; its session is derived from it server-side.
        window: WindowId,
    },
    /// Kill a window and every pane it holds.
    KillWindow {
        /// Target window.
        window: WindowId,
    },
    /// Rename a window.
    RenameWindow {
        /// Target window.
        window: WindowId,
        /// New name.
        name: String,
    },
    /// List every window across every session.
    ListWindows,
    /// List every session.
    ListSessions,
    /// Shut the daemon down cleanly: the accept loop stops, the final state
    /// save runs, and clients receive `%exit` — the same path SIGTERM takes.
    KillServer,
    /// Split a pane's area in two, creating and focusing a new pane.
    SplitWindow {
        /// Target pane to split.
        pane: PaneId,
        /// Split orientation after tmux's flag mapping: `-h` puts the new
        /// pane beside the target (side by side), `-v`/default below it.
        direction: SplitDirection,
        /// `-p`: percent of the split area given to the NEW pane, 50 when
        /// absent (tmux semantics — the target keeps the remainder).
        percent: u32,
    },
    /// Make a pane its window's active pane.
    SelectPane {
        /// Target pane.
        pane: PaneId,
        /// `-T`: the user title to set on the pane. `None` = flag absent
        /// (a pure focus change); `Some("")` = clear; any other value is
        /// the sticky user title, which the pane program's OSC 0/2 title
        /// never overwrites (the documented divergence from tmux).
        title: Option<String>,
    },
    /// Read a pane's effective title: the user `-T` title when one is set,
    /// else the pane terminal's current OSC 0/2 title.
    PaneTitle {
        /// Target pane.
        pane: PaneId,
    },
    /// Grow or shrink a pane by moving its bordering divider, or set its
    /// absolute extents.
    ResizePane {
        /// Target pane.
        pane: PaneId,
        /// Relative (`-L`/`-R`/`-U`/`-D` cells) or absolute (`-x`/`-y`).
        adjustment: ResizeAdjustment,
    },
    /// Exchange two panes' positions within their window.
    SwapPanes {
        /// Pane swapped into the source's position (`-t`).
        target: PaneId,
        /// Pane swapped into the target's position (`-s`).
        source: PaneId,
    },
    /// Print a pane's screen, optionally including scrollback.
    CapturePane {
        /// Target pane.
        pane: PaneId,
        /// `-S`: first line to capture, tmux offset convention — `0` is the
        /// first line of the visible screen, negative numbers are history
        /// lines counted back from there (`-1` is the line directly above
        /// the screen). `None` keeps tmux's default: the first visible line.
        start_line: Option<i64>,
        /// `-E`: last line to capture, inclusive, same offset convention.
        /// `None` keeps tmux's default: the bottom of the visible screen.
        end_line: Option<i64>,
        /// `-e`: include SGR escape sequences inline, one line per grid row
        /// (tmux's `-e` contract). `false` keeps the plain-text capture
        /// byte-identical to the pre-`-e` reply.
        escape: bool,
    },
    /// Store text in the paste buffer.
    SetBuffer {
        /// Buffer content.
        content: String,
    },
    /// Retrieve the paste buffer's content.
    ShowBuffer,
    /// Write the paste buffer's content to a pane, as `send-keys` would.
    PasteBuffer {
        /// Target pane.
        pane: PaneId,
    },
}

impl MuxCommand {
    /// Whether dispatching this command mutates the tree or buffers — the
    /// persistence rule stated on the command type: a structural command
    /// saves the whole state after it lands (par-mux.md D3.3); a content or
    /// read-only command does not, its staleness window bounded by the next
    /// structural save and the clean-shutdown save.
    ///
    /// `refresh-client` is the one dual-mode command: with `-C WxH` it
    /// resizes a window (structural); without it, it only replays a screen.
    pub fn mutates(&self) -> bool {
        match self {
            MuxCommand::NewSession { .. }
            | MuxCommand::KillPane { .. }
            | MuxCommand::NewWindow { .. }
            | MuxCommand::SelectWindow { .. }
            | MuxCommand::KillWindow { .. }
            | MuxCommand::RenameWindow { .. }
            | MuxCommand::SplitWindow { .. }
            | MuxCommand::SelectPane { .. }
            | MuxCommand::ResizePane { .. }
            | MuxCommand::SwapPanes { .. }
            | MuxCommand::SetBuffer { .. } => true,
            MuxCommand::RefreshClient { size, .. } => size.is_some(),
            MuxCommand::ListPanes
            | MuxCommand::ListAgents
            | MuxCommand::ListWindows
            | MuxCommand::ListSessions
            | MuxCommand::KillServer
            | MuxCommand::SendKeys { .. }
            | MuxCommand::CapturePane { .. }
            | MuxCommand::PaneTitle { .. }
            | MuxCommand::ShowBuffer
            | MuxCommand::PasteBuffer { .. } => false,
        }
    }
}

/// One line from a client, classified by grammar: a line whose first
/// non-whitespace byte is `{` is a JSON hook report (see
/// [`crate::mux::hooks`]); anything else is a control command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    /// A hook report, carried raw — the hook layer parses the JSON itself.
    Hook(String),
    /// A parsed control command.
    Control(MuxCommand),
}

/// Classify and parse one client line (see [`Line`]).
pub fn parse_line(line: &str) -> Result<Line, String> {
    if line.trim_start().starts_with('{') {
        Ok(Line::Hook(line.to_string()))
    } else {
        parse_command(line).map(Line::Control)
    }
}

/// One pre-split command line: the shared cursor every per-command parser
/// reads flags, targets, and payloads through.
///
/// Private by design — it is the grammar's internal shape, not part of the
/// [`MuxCommand`] contract callers consume.
struct Args<'a> {
    /// The command name (the first whitespace-separated token).
    name: &'a str,
    /// The tokens after the name.
    args: &'a [&'a str],
    /// The raw, untrimmed line — `send-keys` re-slices its payload from it,
    /// because its quoting cannot survive the whitespace split.
    line: &'a str,
}

impl Args<'_> {
    /// The value following `flag`, when present — `None` both for an absent
    /// flag and for one with no following token.
    fn flag(&self, flag: &str) -> Option<String> {
        self.args
            .iter()
            .position(|a| *a == flag)
            .and_then(|i| self.args.get(i + 1))
            .map(|v| (*v).to_string())
    }

    /// The value following `flag`, read through [`shell_split`] so a quoted
    /// value survives as one word.
    ///
    /// [`Self::flag`] reads the pre-split `args`, so it takes only the first
    /// whitespace-delimited token of a value — for a target that is a typed
    /// `$N`/`@N`/`%N` id that is exactly right, but a NAME may legitimately
    /// contain spaces. This re-splits the whole line with the same bounded
    /// quoting grammar `send-keys` payloads use, so `-s 'Par Mux Test'`
    /// yields `Par Mux Test` rather than `'Par` (par-term filed this against
    /// `new-session`: a spaced name silently created a session called `Par`).
    ///
    /// A value that does not open a quote keeps [`Self::flag`]'s verbatim
    /// result, so an unquoted name is byte-identical to what it parsed to
    /// before — a bare `a\b` stays `a\b` rather than becoming `ab`.
    ///
    /// An explicitly empty value (`-s ''`) is an error rather than a silent
    /// default: quoting is what makes it expressible at all, and tmux admits
    /// any session name except an empty one.
    fn quoted_flag(&self, flag: &str) -> Result<Option<String>, String> {
        match self.quoted_flag_allowing_empty(flag)? {
            // An empty name is reachable only through quoting, and `""` is
            // not a tmux name; a client that sent one has a bug worth
            // surfacing rather than defaulting away.
            Some(v) if v.is_empty() => {
                Err(format!("{}: {flag} requires a non-empty name", self.name))
            }
            other => Ok(other),
        }
    }

    /// [`Self::quoted_flag`] without the non-empty guard: an explicitly
    /// quoted empty value survives as `Some("")`, the shape a CLEAR
    /// operation needs (`select-pane -T ''`, where empty is meaningful
    /// rather than a client bug).
    fn quoted_flag_allowing_empty(&self, flag: &str) -> Result<Option<String>, String> {
        // The flat scan already answers every unquoted name, and answers it
        // byte-for-byte: only re-split when the value actually opens a
        // quote. A bare `a\b` is a name containing a backslash, not an
        // escape — re-splitting unconditionally would silently eat it.
        let flat = self.flag(flag);
        let opens_quote = flat
            .as_deref()
            .is_some_and(|v| v.starts_with('\'') || v.starts_with('"'));
        let value = if opens_quote {
            let words = shell_split(self.line);
            let at = words.iter().position(|w| w == flag);
            at.and_then(|i| words.get(i + 1)).cloned()
        } else {
            flat
        };
        Ok(value)
    }

    /// Presence check for valueless flags (`-h`, `-R`, …) — [`Self::flag`]
    /// cannot distinguish "absent" from "present with no following token".
    fn has_flag(&self, flag: &str) -> bool {
        self.args.contains(&flag)
    }

    fn pane(&self, flag_name: &str) -> Result<PaneId, String> {
        let raw = self
            .flag(flag_name)
            .ok_or_else(|| format!("{} requires {flag_name}", self.name))?;
        raw.parse::<PaneId>()
            .map_err(|_| format!("invalid pane target: {raw}"))
    }

    fn window(&self, flag_name: &str) -> Result<WindowId, String> {
        let raw = self
            .flag(flag_name)
            .ok_or_else(|| format!("{} requires {flag_name}", self.name))?;
        raw.parse::<WindowId>()
            .map_err(|_| format!("invalid window target: {raw}"))
    }

    fn session(&self, flag_name: &str) -> Result<Option<SessionId>, String> {
        match self.flag(flag_name) {
            Some(raw) => Some(
                raw.parse()
                    .map_err(|_| format!("invalid session target: {raw}")),
            )
            .transpose(),
            None => Ok(None),
        }
    }

    /// Everything after a `<flag> <value>` pair, joined back with spaces —
    /// the same "trailing free text is the payload" shape `send-keys`
    /// already uses.
    fn trailing_after(&self, flag: &str) -> String {
        self.args
            .iter()
            .skip_while(|a| **a != flag)
            .skip(2)
            .copied()
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// A positive single-dimension size flag (`-x 120`, `-y 40`) —
    /// [`parse_size_flag`] over [`Self::flag`].
    fn size(&self, flag_name: &str) -> Result<Option<u16>, String> {
        parse_size_flag(&self.flag(flag_name), flag_name, self.name)
    }

    /// A `WxH` size-pair flag (`-C 120x40`) — the renderer-report form.
    fn size_pair(&self, flag_name: &str) -> Result<Option<(u16, u16)>, String> {
        let Some(raw) = self.flag(flag_name) else {
            return Ok(None);
        };
        let (width, height) = raw
            .split_once('x')
            .ok_or_else(|| format!("{}: {flag_name} expects WxH, got: {raw}", self.name))?;
        let dims @ (width, height) = (
            width
                .parse::<u16>()
                .map_err(|_| format!("{}: invalid {flag_name} size: {raw}", self.name))?,
            height
                .parse::<u16>()
                .map_err(|_| format!("{}: invalid {flag_name} size: {raw}", self.name))?,
        );
        if width == 0 || height == 0 {
            return Err(format!(
                "{}: {flag_name} size must be positive: {raw}",
                self.name
            ));
        }
        Ok(Some(dims))
    }
}

/// Split `rest` at its first whitespace-separated `flag` occurrence, into the
/// flag's value and the raw remainder following that value.
///
/// The remainder is a raw slice, not a joined token list: send-keys payloads
/// carry quoting that pre-splitting would destroy. The flag must precede the
/// payload — every sender par-mux targets puts `-t` first, and the bounded
/// grammar here documents that rather than papering over it.
fn split_after_flag<'a>(rest: &'a str, flag: &str) -> Option<(&'a str, &'a str)> {
    let bytes = rest.as_bytes();
    let flag = flag.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if start == i {
            break;
        }
        if &bytes[start..i] == flag {
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            let value_start = j;
            while j < bytes.len() && !bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if value_start == j {
                return None;
            }
            let mut k = j;
            while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                k += 1;
            }
            return Some((&rest[value_start..j], &rest[k..]));
        }
    }
    None
}

/// Split a payload into shell-style words: single- or double-quoted regions
/// contribute their literal content, a backslash outside quotes escapes the
/// next character, and unquoted whitespace separates words.
///
/// This is the bounded grammar par-term's senders actually emit (the `'\''`
/// idiom for embedded quotes); it is not a full shell parser and does not
/// interpolate anything.
fn shell_split(raw: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut have_token = false;
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' | '"' => {
                have_token = true;
                let quote = c;
                for inner in chars.by_ref() {
                    if inner == quote {
                        break;
                    }
                    current.push(inner);
                }
            }
            '\\' => {
                have_token = true;
                if let Some(escaped) = chars.next() {
                    current.push(escaped);
                }
            }
            c if c.is_whitespace() => {
                if have_token {
                    tokens.push(std::mem::take(&mut current));
                    have_token = false;
                }
            }
            c => {
                have_token = true;
                current.push(c);
            }
        }
    }
    if have_token {
        tokens.push(current);
    }
    tokens
}

/// Map one send-keys token to the bytes a terminal expects for that key.
///
/// The table covers exactly the names par-term's `escape_keys_for_tmux`
/// emits, plus `Enter` (which replaces the removed implicit newline) and the
/// arrow keys. Unknown tokens are NOT errors: they are written literally, so
/// passthrough text works without quoting every word (a deliberate, narrower
/// contract than tmux's, which rejects unknown key names).
fn key_to_bytes(name: &str) -> Option<Vec<u8>> {
    match name {
        "C-Space" => Some(vec![0x00]),
        "Enter" => Some(vec![0x0d]),
        "Escape" | "Esc" => Some(vec![0x1b]),
        "BSpace" => Some(vec![0x7f]),
        "Space" => Some(vec![b' ']),
        "Up" => Some(vec![0x1b, b'[', b'A']),
        "Down" => Some(vec![0x1b, b'[', b'B']),
        "Right" => Some(vec![0x1b, b'[', b'C']),
        "Left" => Some(vec![0x1b, b'[', b'D']),
        _ => {
            let letter = name.strip_prefix("C-")?;
            if letter.len() != 1 {
                return None;
            }
            let c = letter.chars().next()?.to_ascii_lowercase();
            if c.is_ascii_lowercase() {
                Some(vec![c as u8 - b'a' + 1])
            } else {
                None
            }
        }
    }
}

/// A bare `0xNN` token: one raw byte, the form `escape_keys_for_tmux` uses
/// for high bytes.
fn hex_byte_token(token: &str) -> Option<u8> {
    let digits = token.strip_prefix("0x")?;
    if digits.len() != 2 {
        return None;
    }
    u8::from_str_radix(digits, 16).ok()
}

/// Parse a send-keys payload into the final bytes for the pane's PTY.
///
/// Three modes, mirroring tmux's contract:
/// - default: tokens are keys — quoted or bare words resolve through the key
///   table, `0xNN` tokens are raw bytes, anything else is literal text.
///   Tokens join with NOTHING between them; a space must be an explicit
///   `Space` key or live inside a quoted run (exactly how
///   `escape_keys_for_tmux` encodes spaces).
/// - `-l`: everything is literal text (quotes still resolved, no key
///   interpretation).
/// - `-H`: tokens are hex byte pairs, `0x` prefix optional.
///
/// No terminator is appended in any mode.
fn parse_send_keys_payload(raw: &str) -> Result<Vec<u8>, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("send-keys requires a payload".to_string());
    }
    let (literal, hex, body) = match raw {
        "-l" | "-H" => return Err("send-keys requires a payload".to_string()),
        _ if let Some(rest) = raw.strip_prefix("-l ") => (true, false, rest),
        _ if let Some(rest) = raw.strip_prefix("-H ") => (false, true, rest),
        _ => (false, false, raw),
    };
    let tokens = shell_split(body);
    if tokens.is_empty() {
        return Err("send-keys requires a payload".to_string());
    }
    let mut out = Vec::new();
    if hex {
        for token in &tokens {
            let digits = token.strip_prefix("0x").unwrap_or(token);
            let byte =
                u8::from_str_radix(digits, 16).map_err(|_| format!("invalid hex byte: {token}"))?;
            out.push(byte);
        }
    } else if literal {
        for token in tokens {
            out.extend_from_slice(token.as_bytes());
        }
    } else {
        for token in tokens {
            if let Some(bytes) = key_to_bytes(&token) {
                out.extend_from_slice(&bytes);
            } else if let Some(byte) = hex_byte_token(&token) {
                out.push(byte);
            } else {
                out.extend_from_slice(token.as_bytes());
            }
        }
    }
    Ok(out)
}

/// Parse a positive-size flag value (`-x 120`, `-y 40`, `-C 120x40`) into
/// one dimension or a WxH pair.
///
/// `None` flag means `None` value (the flag is absent); a present flag with
/// a missing or malformed value is an error, as is zero — tmux panes are
/// always at least one cell.
fn parse_size_flag(
    raw: &Option<String>,
    flag_name: &str,
    command: &str,
) -> Result<Option<u16>, String> {
    let Some(raw) = raw else { return Ok(None) };
    let parsed: u16 = raw
        .parse()
        .map_err(|_| format!("{command}: invalid {flag_name} size: {raw}"))?;
    if parsed == 0 {
        return Err(format!(
            "{command}: {flag_name} size must be positive: {raw}"
        ));
    }
    Ok(Some(parsed))
}

/// One per-command parser, dispatched by name from the [`COMMANDS`] table.
type CommandParser = fn(&Args<'_>) -> Result<MuxCommand, String>;

/// The command table: every command name with its parser. Adding a tmux
/// command is one `parse_<cmd>` function plus one row here.
const COMMANDS: &[(&str, CommandParser)] = &[
    ("new-session", parse_new_session),
    ("list-panes", parse_list_panes),
    ("list-agents", parse_list_agents),
    ("kill-pane", parse_kill_pane),
    ("refresh-client", parse_refresh_client),
    ("send-keys", parse_send_keys),
    ("new-window", parse_new_window),
    ("select-window", parse_select_window),
    ("kill-window", parse_kill_window),
    ("rename-window", parse_rename_window),
    ("list-windows", parse_list_windows),
    ("list-sessions", parse_list_sessions),
    ("kill-server", parse_kill_server),
    ("split-window", parse_split_window),
    ("select-pane", parse_select_pane),
    ("pane-title", parse_pane_title),
    ("resize-pane", parse_resize_pane),
    ("swap-pane", parse_swap_pane),
    ("capture-pane", parse_capture_pane),
    ("set-buffer", parse_set_buffer),
    ("show-buffer", parse_show_buffer),
    ("paste-buffer", parse_paste_buffer),
];

/// Parse one command line from a client.
///
/// Deliberately minimal: whitespace-split with a flag scan. tmux's real
/// argument grammar (`--`, per-command option tables, command sequences) is
/// not a goal here, and pretending to implement it would hide that.
///
/// Quoting is honored in exactly three places, all of them values that may
/// legitimately contain a space, and all sharing the one bounded grammar in
/// [`shell_split`] (single or double quotes, backslash escapes outside
/// quotes, the `'\''` close-escape-reopen idiom; no interpolation):
/// - the `send-keys` payload (see [`parse_send_keys_payload`]), because key
///   names, `-l` and `-H` cannot survive a whitespace split;
/// - `new-session -s NAME` and `new-window -n NAME` (see
///   [`Args::quoted_flag`]) — tmux admits any non-empty session or window
///   name, spaces included.
///
/// Every other flag stays whitespace-split, which is correct rather than
/// merely cheap: the `-t`/`-s` targets everywhere else parse as typed
/// `$N`/`@N`/`%N` identifiers, which cannot contain whitespace. The two
/// trailing-text commands need no quoting either, taking the rest of the
/// line verbatim — `rename-window` (via [`Args::trailing_after`]) and
/// `set-buffer`.
pub fn parse_command(line: &str) -> Result<MuxCommand, String> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    let Some((name, args)) = parts.split_first() else {
        return Err("empty command".to_string());
    };
    let a = Args { name, args, line };
    let (_, parse) = COMMANDS
        .iter()
        .find(|(known, _)| known == name)
        .ok_or_else(|| format!("unknown command: {name}"))?;
    parse(&a)
}

fn parse_new_session(a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::NewSession {
        name: a.quoted_flag("-s")?,
    })
}

fn parse_list_panes(_a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::ListPanes)
}

fn parse_list_agents(_a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::ListAgents)
}

fn parse_kill_pane(a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::KillPane {
        pane: a.pane("-t")?,
    })
}

fn parse_refresh_client(a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::RefreshClient {
        pane: a.pane("-t")?,
        size: a.size_pair("-C")?,
    })
}

fn parse_send_keys(a: &Args<'_>) -> Result<MuxCommand, String> {
    let rest = a
        .line
        .strip_prefix(a.name)
        .expect("the command name prefixes the line");
    let (target_value, payload_raw) =
        split_after_flag(rest, "-t").ok_or_else(|| format!("{} requires -t", a.name))?;
    let pane: PaneId = target_value
        .parse()
        .map_err(|_| format!("invalid pane target: {target_value}"))?;
    let keys = parse_send_keys_payload(payload_raw)?;
    Ok(MuxCommand::SendKeys { pane, keys })
}

fn parse_new_window(a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::NewWindow {
        session: a.session("-t")?,
        name: a.quoted_flag("-n")?,
    })
}

fn parse_select_window(a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::SelectWindow {
        window: a.window("-t")?,
    })
}

fn parse_kill_window(a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::KillWindow {
        window: a.window("-t")?,
    })
}

fn parse_rename_window(a: &Args<'_>) -> Result<MuxCommand, String> {
    let window = a.window("-t")?;
    let name = a.trailing_after("-t");
    if name.is_empty() {
        return Err("rename-window requires a new name".to_string());
    }
    Ok(MuxCommand::RenameWindow { window, name })
}

fn parse_list_windows(_a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::ListWindows)
}

fn parse_list_sessions(_a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::ListSessions)
}

fn parse_kill_server(_a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::KillServer)
}

fn parse_split_window(a: &Args<'_>) -> Result<MuxCommand, String> {
    let pane = a.pane("-t")?;
    // tmux's flags name the arrangement, not the divider: `-h`
    // puts the new pane beside the target (our Vertical
    // orientation), `-v`/default below it (Horizontal).
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
    Ok(MuxCommand::SplitWindow {
        pane,
        direction,
        percent,
    })
}

fn parse_select_pane(a: &Args<'_>) -> Result<MuxCommand, String> {
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

fn parse_pane_title(a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::PaneTitle {
        pane: a.pane("-t")?,
    })
}

fn parse_resize_pane(a: &Args<'_>) -> Result<MuxCommand, String> {
    let pane = a.pane("-t")?;
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

fn parse_swap_pane(a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::SwapPanes {
        target: a.pane("-t")?,
        source: a.pane("-s")?,
    })
}

fn parse_capture_pane(a: &Args<'_>) -> Result<MuxCommand, String> {
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

fn parse_set_buffer(a: &Args<'_>) -> Result<MuxCommand, String> {
    let content = a.args.join(" ");
    if content.is_empty() {
        return Err("set-buffer requires content".to_string());
    }
    Ok(MuxCommand::SetBuffer { content })
}

fn parse_show_buffer(_a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::ShowBuffer)
}

fn parse_paste_buffer(a: &Args<'_>) -> Result<MuxCommand, String> {
    Ok(MuxCommand::PasteBuffer {
        pane: a.pane("-t")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_new_session_with_a_name() {
        let cmd = parse_command("new-session -s work").expect("parses");
        assert_eq!(
            cmd,
            MuxCommand::NewSession {
                name: Some("work".into())
            }
        );
    }

    #[test]
    fn parses_new_session_without_a_name() {
        let cmd = parse_command("new-session").expect("parses");
        assert_eq!(cmd, MuxCommand::NewSession { name: None });
    }

    /// A quoted session name survives as one name. Before the fix the flag
    /// scan read the pre-split tokens, so `-s 'Par Mux Test'` created a
    /// session literally called `'Par` — the bug par-term filed.
    #[test]
    fn new_session_keeps_a_quoted_name_whole() {
        for line in [
            "new-session -s 'Par Mux Test'",
            "new-session -s \"Par Mux Test\"",
        ] {
            assert_eq!(
                parse_command(line).expect("parses"),
                MuxCommand::NewSession {
                    name: Some("Par Mux Test".into())
                },
                "line: {line}"
            );
        }
    }

    /// The `'\''` close-escape-reopen idiom yields a real single quote —
    /// the same escaping `send-keys` payloads already round-trip.
    #[test]
    fn new_session_resolves_the_embedded_quote_idiom() {
        let cmd = parse_command(r"new-session -s 'Paul'\''s box'").expect("parses");
        assert_eq!(
            cmd,
            MuxCommand::NewSession {
                name: Some("Paul's box".into())
            }
        );
    }

    /// Quoting changes nothing for a name that never needed it: an
    /// unquoted name, a trailing flag after it, and a bare `-s` with no
    /// value all behave exactly as they did under the flat split.
    #[test]
    fn new_session_flat_names_are_unchanged() {
        assert_eq!(
            parse_command("new-session -s work").expect("parses"),
            MuxCommand::NewSession {
                name: Some("work".into())
            }
        );
        assert_eq!(
            parse_command("new-session   -s   work  ").expect("parses"),
            MuxCommand::NewSession {
                name: Some("work".into())
            }
        );
        assert_eq!(
            parse_command("new-session -s").expect("parses"),
            MuxCommand::NewSession { name: None }
        );
        // An unquoted value keeps the flat scan's verbatim bytes: a bare
        // backslash is part of the name, not an escape.
        assert_eq!(
            parse_command(r"new-session -s a\b").expect("parses"),
            MuxCommand::NewSession {
                name: Some(r"a\b".into())
            }
        );
    }

    /// The flag scan finds the name flag itself, not a same-looking value
    /// of an earlier flag, and works with the name flag in any slot.
    #[test]
    fn quoted_names_may_look_like_flags() {
        assert_eq!(
            parse_command("new-session -s '-n'").expect("parses"),
            MuxCommand::NewSession {
                name: Some("-n".into())
            }
        );
        assert_eq!(
            parse_command("new-window -t $0 -n '-s'").expect("parses"),
            MuxCommand::NewWindow {
                session: Some(SessionId(0)),
                name: Some("-s".into())
            }
        );
        // The name flag need not come first.
        assert_eq!(
            parse_command("new-window -n 'two words' -t $0").expect("parses"),
            MuxCommand::NewWindow {
                session: Some(SessionId(0)),
                name: Some("two words".into())
            }
        );
    }

    /// An empty name is only expressible through quoting, so the quoted
    /// path is where it has to be rejected — `""` is not a tmux name, and
    /// silently falling back to the default would hide a client bug.
    #[test]
    fn new_session_rejects_an_explicitly_empty_name() {
        let err = parse_command("new-session -s ''").expect_err("empty name is an error");
        assert!(err.contains("-s"), "error names the flag: {err}");
    }

    /// `new-window -n` is the same flag shape as `new-session -s` and got
    /// the same fix; its target stays a typed `$N` id.
    #[test]
    fn new_window_keeps_a_quoted_name_whole() {
        assert_eq!(
            parse_command("new-window -t $0 -n 'build and test'").expect("parses"),
            MuxCommand::NewWindow {
                session: Some(SessionId(0)),
                name: Some("build and test".into())
            }
        );
        assert!(parse_command("new-window -t $0 -n ''").is_err());
    }

    #[test]
    fn parses_send_keys_with_a_target() {
        let cmd = parse_command("send-keys -t %3 hello").expect("parses");
        assert_eq!(
            cmd,
            MuxCommand::SendKeys {
                pane: PaneId(3),
                keys: b"hello".to_vec()
            }
        );
    }

    #[test]
    fn send_keys_interprets_key_names() {
        let cmd = parse_command("send-keys -t %3 C-c").expect("parses");
        assert_eq!(
            cmd,
            MuxCommand::SendKeys {
                pane: PaneId(3),
                keys: vec![0x03]
            }
        );
        let cmd = parse_command("send-keys -t %3 C-Space Escape BSpace Space Enter").unwrap();
        let MuxCommand::SendKeys { keys, .. } = cmd else {
            panic!("send-keys");
        };
        assert_eq!(keys, vec![0x00, 0x1b, 0x7f, b' ', 0x0d]);
    }

    #[test]
    fn send_keys_maps_arrows_to_csi_sequences() {
        let cmd = parse_command("send-keys -t %3 Up Down Left Right").unwrap();
        let MuxCommand::SendKeys { keys, .. } = cmd else {
            panic!("send-keys");
        };
        assert_eq!(keys, b"\x1b[A\x1b[B\x1b[D\x1b[C".to_vec());
    }

    #[test]
    fn send_keys_resolves_quoted_runs_and_the_quote_idiom() {
        // Quoted runs are literal, spaces inside them survive, and the
        // '\'' idiom yields a real single quote — the escape_keys_for_tmux
        // round trip.
        let cmd = parse_command("send-keys -t %3 'hello world'").unwrap();
        let MuxCommand::SendKeys { keys, .. } = cmd else {
            panic!("send-keys");
        };
        assert_eq!(keys, b"hello world".to_vec());

        let cmd = parse_command("send-keys -t %3 'it'\\''s'").unwrap();
        let MuxCommand::SendKeys { keys, .. } = cmd else {
            panic!("send-keys");
        };
        assert_eq!(keys, b"it's".to_vec());
    }

    #[test]
    fn send_keys_space_between_bare_words_is_explicit_not_implicit() {
        // tmux semantics: tokens join with nothing between them; a space is
        // the Space key. escape_keys_for_tmux encodes exactly this.
        let cmd = parse_command("send-keys -t %3 'hello' Space 'world'").unwrap();
        let MuxCommand::SendKeys { keys, .. } = cmd else {
            panic!("send-keys");
        };
        assert_eq!(keys, b"hello world".to_vec());

        let cmd = parse_command("send-keys -t %3 hello world").unwrap();
        let MuxCommand::SendKeys { keys, .. } = cmd else {
            panic!("send-keys");
        };
        assert_eq!(keys, b"helloworld".to_vec());
    }

    #[test]
    fn send_keys_literal_flag_disables_interpretation() {
        let cmd = parse_command("send-keys -t %3 -l C-c").unwrap();
        let MuxCommand::SendKeys { keys, .. } = cmd else {
            panic!("send-keys");
        };
        assert_eq!(keys, b"C-c".to_vec());
    }

    #[test]
    fn send_keys_hex_flag_takes_byte_pairs() {
        // The form format_send_hex_keys emits for CSI-u sequences.
        let cmd = parse_command("send-keys -t %3 -H 1b 5b 41").unwrap();
        let MuxCommand::SendKeys { keys, .. } = cmd else {
            panic!("send-keys");
        };
        assert_eq!(keys, vec![0x1b, 0x5b, 0x41]);

        assert!(parse_command("send-keys -t %3 -H zz").is_err());
    }

    #[test]
    fn send_keys_bare_hex_token_is_one_byte() {
        let cmd = parse_command("send-keys -t %3 0x1b 'prompt> '").unwrap();
        let MuxCommand::SendKeys { keys, .. } = cmd else {
            panic!("send-keys");
        };
        assert_eq!(keys, b"\x1bprompt> ".to_vec());
    }

    #[test]
    fn send_keys_round_trips_an_escape_keys_for_tmux_stream() {
        // Representative output of par-term's escape_keys_for_tmux for the
        // bytes b"hi \xe2\x82\xacC-c": printable run quoted, high bytes as
        // 0xNN tokens, the control key by name.
        let cmd = parse_command("send-keys -t %3 'hi ' 0xe2 0x82 0xac C-c").unwrap();
        let MuxCommand::SendKeys { keys, .. } = cmd else {
            panic!("send-keys");
        };
        assert_eq!(keys, b"hi \xe2\x82\xac\x03".to_vec());
    }

    #[test]
    fn send_keys_requires_a_payload() {
        assert!(parse_command("send-keys -t %3").is_err());
        assert!(parse_command("send-keys -t %3 -l").is_err());
    }

    #[test]
    fn parses_list_panes_and_kill_pane() {
        assert_eq!(parse_command("list-panes").unwrap(), MuxCommand::ListPanes);
        assert_eq!(
            parse_command("kill-pane -t %7").unwrap(),
            MuxCommand::KillPane { pane: PaneId(7) }
        );
    }

    #[test]
    fn rejects_an_unknown_command() {
        assert!(parse_command("frobnicate").is_err());
    }

    #[test]
    fn rejects_a_malformed_target() {
        assert!(parse_command("kill-pane -t notapane").is_err());
        assert!(
            parse_command("kill-pane").is_err(),
            "kill-pane needs a target"
        );
    }

    #[test]
    fn parses_new_window_with_and_without_a_name() {
        assert_eq!(
            parse_command("new-window -t $0 -n build").unwrap(),
            MuxCommand::NewWindow {
                session: Some(SessionId(0)),
                name: Some("build".into())
            }
        );
        assert_eq!(
            parse_command("new-window -t $0").unwrap(),
            MuxCommand::NewWindow {
                session: Some(SessionId(0)),
                name: None
            }
        );
        // Bare new-window — the form tmux clients issue — targets the
        // most-recently-created session, resolved server-side.
        assert_eq!(
            parse_command("new-window").unwrap(),
            MuxCommand::NewWindow {
                session: None,
                name: None
            }
        );
    }

    #[test]
    fn parses_select_and_kill_window() {
        assert_eq!(
            parse_command("select-window -t @2").unwrap(),
            MuxCommand::SelectWindow {
                window: WindowId(2)
            }
        );
        assert_eq!(
            parse_command("kill-window -t @2").unwrap(),
            MuxCommand::KillWindow {
                window: WindowId(2)
            }
        );
    }

    #[test]
    fn parses_rename_window_and_rejects_a_missing_name() {
        assert_eq!(
            parse_command("rename-window -t @1 scratch").unwrap(),
            MuxCommand::RenameWindow {
                window: WindowId(1),
                name: "scratch".into()
            }
        );
        assert!(
            parse_command("rename-window -t @1").is_err(),
            "rename-window needs a new name"
        );
    }

    #[test]
    fn parses_list_windows_and_list_sessions() {
        assert_eq!(
            parse_command("list-windows").unwrap(),
            MuxCommand::ListWindows
        );
        assert_eq!(
            parse_command("list-sessions").unwrap(),
            MuxCommand::ListSessions
        );
    }

    #[test]
    fn parses_list_agents() {
        assert_eq!(
            parse_command("list-agents").unwrap(),
            MuxCommand::ListAgents
        );
    }

    #[test]
    fn rejects_malformed_window_and_session_targets() {
        assert!(parse_command("new-window -t notasession").is_err());
        assert!(parse_command("select-window -t notawindow").is_err());
        // Bare new-window is valid (targets the newest session); the
        // malformed-TARGET cases above are what this test guards.
    }

    #[test]
    fn parses_capture_pane_with_and_without_history() {
        assert_eq!(
            parse_command("capture-pane -t %3 -p").unwrap(),
            MuxCommand::CapturePane {
                pane: PaneId(3),
                start_line: None,
                end_line: None,
                escape: false
            }
        );
        assert_eq!(
            parse_command("capture-pane -t %3 -p -S 50 -E -1").unwrap(),
            MuxCommand::CapturePane {
                pane: PaneId(3),
                start_line: Some(50),
                end_line: Some(-1),
                escape: false
            }
        );
        assert_eq!(
            parse_command("capture-pane -t %3 -p -S -20 -E -11").unwrap(),
            MuxCommand::CapturePane {
                pane: PaneId(3),
                start_line: Some(-20),
                end_line: Some(-11),
                escape: false
            }
        );
        assert_eq!(
            parse_command("capture-pane -t %3 -p -e -S -20 -E -11").unwrap(),
            MuxCommand::CapturePane {
                pane: PaneId(3),
                start_line: Some(-20),
                end_line: Some(-11),
                escape: true
            }
        );
    }

    #[test]
    fn parses_split_window_with_flags_and_defaults() {
        // Default: new pane below the target (-v), 50 percent.
        assert_eq!(
            parse_command("split-window -t %0").unwrap(),
            MuxCommand::SplitWindow {
                pane: PaneId(0),
                direction: SplitDirection::Horizontal,
                percent: 50
            }
        );
        assert_eq!(
            parse_command("split-window -t %0 -v").unwrap(),
            MuxCommand::SplitWindow {
                pane: PaneId(0),
                direction: SplitDirection::Horizontal,
                percent: 50
            }
        );
        // -h: side by side; -p: the NEW pane's share.
        assert_eq!(
            parse_command("split-window -t %0 -h -p 25").unwrap(),
            MuxCommand::SplitWindow {
                pane: PaneId(0),
                direction: SplitDirection::Vertical,
                percent: 25
            }
        );
    }

    #[test]
    fn split_window_rejects_out_of_range_percent() {
        assert!(parse_command("split-window -t %0 -p 0").is_err());
        assert!(parse_command("split-window -t %0 -p 100").is_err());
    }

    #[test]
    fn parses_select_and_swap_pane() {
        assert_eq!(
            parse_command("select-pane -t %2").unwrap(),
            MuxCommand::SelectPane {
                pane: PaneId(2),
                title: None,
            }
        );
        assert_eq!(
            parse_command("swap-pane -t %2 -s %5").unwrap(),
            MuxCommand::SwapPanes {
                target: PaneId(2),
                source: PaneId(5)
            }
        );
    }

    #[test]
    fn parses_select_pane_title_flag() {
        // Quoted title with spaces stays one value (the same grammar
        // new-session -s uses for names).
        assert_eq!(
            parse_command("select-pane -t %0 -T 'My build pane'").unwrap(),
            MuxCommand::SelectPane {
                pane: PaneId(0),
                title: Some("My build pane".to_string()),
            }
        );
        // An explicitly empty -T is the CLEAR operation, not an error.
        assert_eq!(
            parse_command("select-pane -t %0 -T ''").unwrap(),
            MuxCommand::SelectPane {
                pane: PaneId(0),
                title: Some(String::new()),
            }
        );
        // Unquoted single word, and no -T at all.
        assert_eq!(
            parse_command("select-pane -t %0 -T logs").unwrap(),
            MuxCommand::SelectPane {
                pane: PaneId(0),
                title: Some("logs".to_string()),
            }
        );
        // -T with nothing following it is malformed, not "clear".
        assert!(parse_command("select-pane -t %0 -T").is_err());
    }

    #[test]
    fn parses_pane_title_query() {
        assert_eq!(
            parse_command("pane-title -t %3").unwrap(),
            MuxCommand::PaneTitle { pane: PaneId(3) }
        );
        assert!(parse_command("pane-title").is_err());
    }

    #[test]
    fn parses_resize_pane_with_default_and_explicit_cells() {
        assert_eq!(
            parse_command("resize-pane -t %0 -R").unwrap(),
            MuxCommand::ResizePane {
                pane: PaneId(0),
                adjustment: ResizeAdjustment::Relative {
                    direction: ResizeDirection::Right,
                    cells: 5
                }
            }
        );
        assert_eq!(
            parse_command("resize-pane -t %0 -U 12").unwrap(),
            MuxCommand::ResizePane {
                pane: PaneId(0),
                adjustment: ResizeAdjustment::Relative {
                    direction: ResizeDirection::Up,
                    cells: 12
                }
            }
        );
        assert!(parse_command("resize-pane -t %0").is_err());
    }

    #[test]
    fn parses_resize_pane_absolute_extents() {
        // The renderer-driven form par-term's gateway sends on drag-resize.
        assert_eq!(
            parse_command("resize-pane -t %0 -x 120 -y 40").unwrap(),
            MuxCommand::ResizePane {
                pane: PaneId(0),
                adjustment: ResizeAdjustment::Absolute {
                    cols: Some(120),
                    rows: Some(40)
                }
            }
        );
        // Either axis alone is valid.
        assert_eq!(
            parse_command("resize-pane -t %0 -x 25").unwrap(),
            MuxCommand::ResizePane {
                pane: PaneId(0),
                adjustment: ResizeAdjustment::Absolute {
                    cols: Some(25),
                    rows: None
                }
            }
        );
        assert_eq!(
            parse_command("resize-pane -t %0 -y 30").unwrap(),
            MuxCommand::ResizePane {
                pane: PaneId(0),
                adjustment: ResizeAdjustment::Absolute {
                    cols: None,
                    rows: Some(30)
                }
            }
        );
    }

    #[test]
    fn resize_pane_absolute_rejects_zero_and_mixed_forms() {
        assert!(parse_command("resize-pane -t %0 -x 0").is_err());
        assert!(parse_command("resize-pane -t %0 -y 0").is_err());
        assert!(parse_command("resize-pane -t %0 -x abc").is_err());
        assert!(
            parse_command("resize-pane -t %0 -x 120 -R").is_err(),
            "absolute and relative forms cannot combine"
        );
    }

    #[test]
    fn parses_refresh_client_with_and_without_a_size_report() {
        assert_eq!(
            parse_command("refresh-client -t %0").unwrap(),
            MuxCommand::RefreshClient {
                pane: PaneId(0),
                size: None
            }
        );
        assert_eq!(
            parse_command("refresh-client -t %0 -C 120x40").unwrap(),
            MuxCommand::RefreshClient {
                pane: PaneId(0),
                size: Some((120, 40))
            }
        );
    }

    #[test]
    fn refresh_client_size_report_rejects_malformed_values() {
        assert!(parse_command("refresh-client -t %0 -C 120").is_err());
        assert!(parse_command("refresh-client -t %0 -C 0x40").is_err());
        assert!(parse_command("refresh-client -t %0 -C 120x0").is_err());
        assert!(parse_command("refresh-client -t %0 -C ax40").is_err());
    }

    #[test]
    fn parses_set_buffer_show_buffer_and_paste_buffer() {
        assert_eq!(
            parse_command("set-buffer hello world").unwrap(),
            MuxCommand::SetBuffer {
                content: "hello world".into()
            }
        );
        assert_eq!(
            parse_command("show-buffer").unwrap(),
            MuxCommand::ShowBuffer
        );
        assert_eq!(
            parse_command("paste-buffer -t %2").unwrap(),
            MuxCommand::PasteBuffer { pane: PaneId(2) }
        );
    }

    #[test]
    fn rejects_a_set_buffer_with_no_content() {
        assert!(parse_command("set-buffer").is_err());
    }

    #[test]
    fn mutates_marks_exactly_the_structural_commands() {
        // The persistence rule on the type — the pre-decomposition
        // `mutated = true` set, verbatim. Structural commands save the state
        // file on success; content and read-only commands never do.
        let structural = [
            MuxCommand::NewSession { name: None },
            MuxCommand::KillPane { pane: PaneId(0) },
            MuxCommand::RefreshClient {
                pane: PaneId(0),
                size: Some((80, 24)),
            },
            MuxCommand::NewWindow {
                session: None,
                name: None,
            },
            MuxCommand::SelectWindow {
                window: WindowId(0),
            },
            MuxCommand::KillWindow {
                window: WindowId(0),
            },
            MuxCommand::RenameWindow {
                window: WindowId(0),
                name: String::new(),
            },
            MuxCommand::SplitWindow {
                pane: PaneId(0),
                direction: SplitDirection::Horizontal,
                percent: 50,
            },
            MuxCommand::SelectPane {
                pane: PaneId(0),
                title: None,
            },
            MuxCommand::ResizePane {
                pane: PaneId(0),
                adjustment: ResizeAdjustment::Relative {
                    direction: ResizeDirection::Right,
                    cells: 5,
                },
            },
            MuxCommand::SwapPanes {
                target: PaneId(0),
                source: PaneId(1),
            },
            MuxCommand::SetBuffer {
                content: String::new(),
            },
        ];
        for command in &structural {
            assert!(
                command.mutates(),
                "{command:?} is structural and must mutate"
            );
        }
        let non_mutating = [
            MuxCommand::ListPanes,
            MuxCommand::ListAgents,
            MuxCommand::SendKeys {
                pane: PaneId(0),
                keys: Vec::new(),
            },
            MuxCommand::RefreshClient {
                pane: PaneId(0),
                size: None,
            },
            MuxCommand::ListWindows,
            MuxCommand::ListSessions,
            MuxCommand::CapturePane {
                pane: PaneId(0),
                start_line: None,
                end_line: None,
                escape: false,
            },
            MuxCommand::ShowBuffer,
            MuxCommand::PasteBuffer { pane: PaneId(0) },
        ];
        for command in &non_mutating {
            assert!(
                !command.mutates(),
                "{command:?} is content or read-only and must not mutate"
            );
        }
    }

    #[test]
    fn parse_line_routes_hook_reports_and_control_commands() {
        assert_eq!(
            parse_line(r#" {"id":1,"method":"pane.report_agent"}"#).unwrap(),
            Line::Hook(r#" {"id":1,"method":"pane.report_agent"}"#.to_string())
        );
        assert_eq!(
            parse_line("list-panes").unwrap(),
            Line::Control(MuxCommand::ListPanes)
        );
        let Err(err) = parse_line("frobnicate") else {
            panic!("an unknown command is a parse error");
        };
        assert_eq!(err, "unknown command: frobnicate");
    }
}
