//! Control-mode command parsing (client → server).

use std::ops::Deref;

use crate::keyboard::{self, modifiers, TermKey, TermKeyEvent};
use crate::mux::ids::{AnyTarget, PaneId, SessionId, Target, WindowId, WorkspaceId};
use crate::mux::layout::{ResizeDirection, SplitDirection};
use crate::terminal::Terminal;

/// One piece of a `send-keys` payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendKeysPart {
    /// Bytes written verbatim: literal text, `-l`/`-H` payloads, `0xNN`
    /// tokens, and the control-byte key names (`C-x`, `Escape`, `BSpace`,
    /// `Space`, `Enter`, `Tab`).
    Bytes(Vec<u8>),
    /// A navigation or function key, encoded by [`keyboard::encode_key`]
    /// against the target pane's negotiated modes (DECCKM, kitty flags).
    Key(TermKeyEvent),
}

/// A parsed `send-keys` payload: the parts in order, adjacent `Bytes`
/// merged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SendKeysPayload(pub Vec<SendKeysPart>);

impl SendKeysPayload {
    /// The bytes for the pane's PTY: `Bytes` parts verbatim, `Key` parts
    /// encoded against the terminal `terminal` returns. `terminal` is
    /// called only when a `Key` part needs it, so a payload of plain bytes
    /// (par-term's keystroke stream) never takes the terminal lock.
    pub fn encode<T: Deref<Target = Terminal>>(&self, terminal: impl FnOnce() -> T) -> Vec<u8> {
        let term = self
            .0
            .iter()
            .any(|part| matches!(part, SendKeysPart::Key(_)))
            .then(terminal);
        let mut out = Vec::new();
        for part in &self.0 {
            match part {
                SendKeysPart::Bytes(bytes) => out.extend_from_slice(bytes),
                SendKeysPart::Key(ev) => {
                    if let Some(term) = term.as_deref() {
                        out.extend(keyboard::encode_key(ev, term));
                    }
                }
            }
        }
        out
    }

    fn push_bytes(&mut self, bytes: &[u8]) {
        if let Some(SendKeysPart::Bytes(last)) = self.0.last_mut() {
            last.extend_from_slice(bytes);
        } else {
            self.0.push(SendKeysPart::Bytes(bytes.to_vec()));
        }
    }

    fn push(&mut self, part: SendKeysPart) {
        match part {
            SendKeysPart::Bytes(bytes) => self.push_bytes(&bytes),
            key => self.0.push(key),
        }
    }
}

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
    /// `-Z`: toggle the pane's zoom — the full window grid while zoomed,
    /// the exact prior layout back on unzoom (tmux's `resize-pane -Z`).
    /// Not combinable with the other adjustment forms.
    Zoom,
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
        /// `-e NAME=VALUE` (repeatable): the session's initial environment,
        /// applied to every pane it spawns (tmux 3.x `new-session -e`).
        env: Vec<(String, String)>,
        /// `-t`: the workspace the session joins (id or name). Absent
        /// targets the daemon's active workspace, creating the default
        /// `main` workspace when none exists.
        workspace: Option<Target<WorkspaceId>>,
    },
    /// List live panes: globally when `session` is `None`, or the panes of
    /// one window when a window target rides `-t` (see
    /// [`MuxCommand::ListWindows`]'s shape note on the one-enum-two-forms
    /// rule).
    ListPanes {
        /// `-t <window>`: list that window's panes. `None` lists every pane
        /// across every session (the global wire shape clients already
        /// parse).
        window: Option<Target<WindowId>>,
    },
    /// List every pane a hook has claimed — the agent roster.
    ListAgents,
    /// List every dispatchable command with its feature tokens — capability
    /// discovery, so a client built against one daemon learns what another
    /// daemon supports without probing command by command (ENH-037).
    ListCommands,
    /// Send keys to a pane.
    SendKeys {
        /// Target pane.
        pane: Target<PaneId>,
        /// The payload — key names interpreted, quotes resolved, no
        /// terminator added. `Enter` is an expressible key, not an implicit
        /// one (tmux semantics; par-mux.md Phase 4 T4.B). Navigation and
        /// function keys are encoded against the pane's modes at dispatch
        /// ([`SendKeysPayload::encode`]).
        keys: SendKeysPayload,
    },
    /// Kill a pane.
    KillPane {
        /// Target pane.
        pane: Target<PaneId>,
    },
    /// Replay a pane's current screen to the requesting client, or report
    /// the client's renderer size (`-C`) and resize the pane's window.
    RefreshClient {
        /// Target pane; the window it belongs to is the one a `-C` resize
        /// applies to. `None` is the target-less size-report form the
        /// attach handshake sends before it has resolved a pane: with it,
        /// `-p` still applies daemon-wide and `-C` resizes the newest
        /// session's active window (the same newest-stand-in bare
        /// `new-window` uses), and there is never a screen-restore replay —
        /// replay is what the `-t` form is for.
        pane: Option<Target<PaneId>>,
        /// `-C WxH`: the client's renderer grid size. The window-size
        /// policy (par-mux.md Phase 4): the LATEST such report wins —
        /// par-mux has no other client-size input, so latest-attached-client
        /// and latest `-C` are the same rule here. `None` keeps the
        /// replay-only behavior.
        size: Option<(u16, u16)>,
        /// `-p WxH`: the client's per-CELL pixel size (font metrics), the
        /// one renderer measurement every pane shares. Applied daemon-wide
        /// so `TIOCGWINSZ`, XTWINOPS pixel reports, and image cell-span
        /// math carry the size cells actually render at. Independently
        /// optional from `-C`: a resize without pixels keeps the last
        /// reported cells, a pixel report without `-C` re-fits at the
        /// current grid.
        cell_pixels: Option<(u16, u16)>,
        /// `-I border=B,gap=N,gutter=G`: the per-pane chrome the client
        /// paints inside every pane rect (feature token `chrome`). Rides
        /// with `-C`: the connection's sizing contribution records it,
        /// and the window's division sizes each pane's PTY to the rect
        /// minus the declared chrome ([`crate::mux::layout::PaneChrome`]).
        /// `None` (an older client) declares no chrome — PTYs keep the
        /// full rect, exactly the pre-declaration sizing.
        chrome: Option<crate::mux::layout::PaneChrome>,
    },
    /// Add a window to a session, optionally named.
    NewWindow {
        /// Target session (`$N` or name, appending), or window/pane (`@N`
        /// and `%N`, inserting right after that window) — see [`AnyTarget`].
        /// `None` means the most-recently-created one (the bare
        /// `new-window` tmux clients issue, which tmux resolves against
        /// the client's attached session — par-mux has no client-session
        /// attachment, so "newest" is the documented stand-in).
        target: Option<AnyTarget>,
        /// Window name; a default is chosen when absent.
        name: Option<String>,
        /// `-c`: the new pane's start directory. Absent keeps the
        /// factory-wide default; a directory that does not exist degrades
        /// to home at dispatch rather than failing the command.
        start_dir: Option<String>,
    },
    /// Set a session's active window.
    SelectWindow {
        /// Target window; its session is derived from it server-side.
        window: Target<WindowId>,
    },
    /// Kill a window and every pane it holds.
    KillWindow {
        /// Target window.
        window: Target<WindowId>,
    },
    /// Rename a window.
    RenameWindow {
        /// Target window.
        window: Target<WindowId>,
        /// New name.
        name: String,
    },
    /// List windows: across every session when `session` is `None`, or one
    /// session's windows in order when a session target rides `-t`. One
    /// enum variant for both forms, because tmux's wire grammar is one
    /// command name with an optional `-t` — the parser cannot tell the
    /// forms apart without carrying the distinction, and dispatch then
    /// picks the reply shape.
    ListWindows {
        /// `-t <session>`: list that session's windows. `None` lists every
        /// window across every session (the global wire shape clients
        /// already parse).
        session: Option<Target<SessionId>>,
    },
    /// List sessions — every workspace's when bare, one workspace's when
    /// `-t` names one. The reply line shape carries the workspace prefix;
    /// see `cmd_list_sessions` for the grammar.
    ListSessions {
        /// `-t`: restrict the listing to this workspace (id or name).
        workspace: Option<Target<WorkspaceId>>,
    },
    /// Shut the daemon down cleanly: the accept loop stops, the final state
    /// save runs, and clients receive `%exit` — the same path SIGTERM takes.
    KillServer,
    /// Split a pane's area in two, creating and focusing a new pane.
    SplitWindow {
        /// Target pane (`%N`), window (`@N` — its active pane), or session
        /// (`$N` — its active window's active pane) to split; a name is a
        /// pane-title target. See [`AnyTarget`].
        target: AnyTarget,
        /// Split orientation after tmux's flag mapping: `-h` puts the new
        /// pane beside the target (side by side), `-v`/default below it.
        direction: SplitDirection,
        /// `-p`: percent of the split area given to the NEW pane, 50 when
        /// absent (tmux semantics — the target keeps the remainder).
        percent: u32,
        /// `-b`: place the new pane BEFORE the target — left of it under
        /// `-h`, above it in the default direction (tmux semantics).
        before: bool,
        /// `-c`: the new pane's start directory, with the same
        /// degrade-to-home rule as `new-window -c`.
        start_dir: Option<String>,
    },
    /// Make a pane its window's active pane.
    SelectPane {
        /// Target pane.
        pane: Target<PaneId>,
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
        pane: Target<PaneId>,
    },
    /// Read a pane's window and current grid size, one line:
    /// `%N @W COLSxROWS`. A mirroring client seeds at the pane's own size
    /// instead of resizing the pane to its viewport.
    PaneInfo {
        /// Target pane.
        pane: Target<PaneId>,
    },
    /// Ask for the currently held panes' `%pane-exited` lines on demand
    /// (ENH-042). The lines go to the issuing client only; the reply body
    /// is empty. Read-only.
    PaneExitedReplay,
    /// Clear a pane's scrollback and wipe its visible screen (tmux's
    /// `clear-history`). Structural: the cleared scrollback is what a
    /// restart-restore replays, so a content classification would
    /// resurrect the history the command just erased.
    ClearHistory {
        /// Target pane.
        pane: Target<PaneId>,
    },
    /// Grow or shrink a pane by moving its bordering divider, or set its
    /// absolute extents.
    ResizePane {
        /// Target pane.
        pane: Target<PaneId>,
        /// Relative (`-L`/`-R`/`-U`/`-D` cells) or absolute (`-x`/`-y`).
        adjustment: ResizeAdjustment,
    },
    /// Exchange two panes' positions within their window.
    SwapPanes {
        /// Pane swapped into the source's position (`-t`).
        target: Target<PaneId>,
        /// Pane swapped into the target's position (`-s`).
        source: Target<PaneId>,
    },
    /// Move a pane out of its window into a new window of the same
    /// session (`break-pane`).
    BreakPane {
        /// The pane promoted to its own window (`-s`).
        source: Target<PaneId>,
        /// The new window's name (`-n`); the pane's process, terminal,
        /// and title all move with it — nothing is re-spawned.
        name: Option<String>,
    },
    /// Move a pane next to another pane (`join-pane`), possibly in a
    /// different window.
    JoinPane {
        /// The pane that moves (`-s`).
        source: Target<PaneId>,
        /// The pane it lands next to (`-t`).
        target: Target<PaneId>,
        /// `-h` beside the target, `-v`/default below it — the same
        /// arrangement rule `split-window` uses.
        direction: SplitDirection,
        /// The share of the target's extent the moved pane takes
        /// (`-p`), 1-99, default 50.
        percent: u32,
    },
    /// Move a window to a position in its session's window list
    /// (`move-window`).
    MoveWindow {
        /// The window that moves (`-s`).
        source: Target<WindowId>,
        /// The list position it lands at (`-t`); out-of-range positions
        /// clamp to the ends.
        index: usize,
    },
    /// Exchange two windows' positions in their session (`swap-window`).
    SwapWindows {
        /// First window (`-s`).
        source: Target<WindowId>,
        /// Second window (`-t`).
        target: Target<WindowId>,
    },
    /// Restart a pane's process in place (`respawn-pane`): same pane id,
    /// window, and layout; a fresh terminal.
    RespawnPane {
        /// The pane to restart (`-t`).
        pane: Target<PaneId>,
        /// `-k`: kill a still-running process first; without it a live
        /// pane is refused.
        kill: bool,
        /// `-c`: the restart's working directory, replacing the stored
        /// one.
        start_dir: Option<String>,
        /// The command to run, replacing the stored one — the trailing
        /// text after the flags.
        command: Option<String>,
    },
    /// Print a pane's screen, optionally including scrollback.
    CapturePane {
        /// Target pane.
        pane: Target<PaneId>,
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
    /// Report the client's theme colors (`set-client-colors -f/-b rrggbb`)
    /// so OSC 10/11 queries in panes answer with what the client actually
    /// renders instead of the core's built-in theme. At least one of the
    /// two flags is required; each is independent.
    SetClientColors {
        /// `-f rrggbb`: the theme foreground (OSC 10's answer).
        fg: Option<(u8, u8, u8)>,
        /// `-b rrggbb`: the theme background (OSC 11's answer).
        bg: Option<(u8, u8, u8)>,
    },
    /// Retrieve the paste buffer's content.
    ShowBuffer,
    /// Write the paste buffer's content to a pane, as `send-keys` would.
    PasteBuffer {
        /// Target pane.
        pane: Target<PaneId>,
    },
    /// Set or unset one variable in a session's environment
    /// (`set-environment -t $N NAME VALUE` / `-u NAME`). Panes spawned
    /// afterwards see it; panes already running do not (tmux semantics).
    SetEnvironment {
        /// Target session. Required: env landing on the wrong session is
        /// worse than an error, so there is no newest-session default.
        session: Target<SessionId>,
        /// Variable name.
        name: String,
        /// `Some` sets the value; `None` (`-u`) removes the variable.
        value: Option<String>,
    },
    /// Rename a session (`rename-session -t $N <name>`). The tree's name is
    /// what future pane spawns export as `PAR_MUX_SESSION`; panes already
    /// running keep the spawn-time name (fixed-at-spawn contract, MUX.md).
    RenameSession {
        /// Target session.
        session: Target<SessionId>,
        /// New name.
        name: String,
    },
    /// Kill a session and every window and pane in it
    /// (`kill-session -t $N`).
    KillSession {
        /// Target session.
        session: Target<SessionId>,
    },
    /// Create a workspace — the level above sessions — and make it the
    /// daemon's active one. tmux has no workspace concept; this is
    /// par-mux's own command.
    NewWorkspace {
        /// `-n`: the workspace name; a default is chosen when absent.
        name: Option<String>,
    },
    /// List every workspace, one `+N: name` line per workspace, the
    /// daemon's active one suffixed ` active`.
    ListWorkspaces,
    /// Make a workspace the daemon's active one; its previously active
    /// session resumes as the active session.
    SelectWorkspace {
        /// Target workspace.
        workspace: Target<WorkspaceId>,
    },
    /// Move the displayed session (`switch-client -t <target>`, tmux's
    /// client-session switch over par-mux's one shared display): the
    /// target's session becomes its workspace's active session and that
    /// workspace the daemon's active one; a window or pane target also
    /// selects its window. Bare `switch-client` is a read-only query
    /// replying the displayed session's `$N`.
    SwitchClient {
        /// `-t`: a session, window, or pane (the window's session is the
        /// one displayed); absent = query.
        target: Option<AnyTarget>,
    },
    /// Rename a workspace (`rename-workspace -t +N <name>`).
    RenameWorkspace {
        /// Target workspace.
        workspace: Target<WorkspaceId>,
        /// New name.
        name: String,
    },
    /// Kill a workspace and every session, window, and pane in it
    /// (`kill-workspace -t +N`).
    KillWorkspace {
        /// Target workspace.
        workspace: Target<WorkspaceId>,
    },
    /// Report the daemon's build stamp — the `version` wire form of
    /// [`crate::mux::build_stamp`]. Read-only, tree-free: it exists so a
    /// client can compare the daemon's core build against its own linked
    /// one and surface a stale daemon instead of silently missing fixes.
    Version,
    /// Re-read the config file and apply what can be applied live. One
    /// reply line per setting: `applied: <name>` or `restart-required:
    /// <name>` (see [`crate::mux::config`] for the per-setting semantics).
    /// Mutates daemon state (the applied copies) without touching the
    /// tree, so it never triggers a state save.
    ReloadConfig,
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
            | MuxCommand::BreakPane { .. }
            | MuxCommand::JoinPane { .. }
            | MuxCommand::MoveWindow { .. }
            | MuxCommand::SwapWindows { .. }
            | MuxCommand::RespawnPane { .. }
            | MuxCommand::SetBuffer { .. }
            | MuxCommand::SetEnvironment { .. }
            | MuxCommand::RenameSession { .. }
            | MuxCommand::ClearHistory { .. }
            | MuxCommand::KillSession { .. }
            | MuxCommand::NewWorkspace { .. }
            | MuxCommand::SelectWorkspace { .. }
            | MuxCommand::RenameWorkspace { .. }
            | MuxCommand::KillWorkspace { .. } => true,
            MuxCommand::RefreshClient { size, .. } => size.is_some(),
            // The query form reads only; a switch moves persisted pointers.
            MuxCommand::SwitchClient { target } => target.is_some(),
            MuxCommand::ListPanes { .. }
            | MuxCommand::ListAgents
            | MuxCommand::ListCommands
            | MuxCommand::ListWindows { .. }
            | MuxCommand::ListSessions { .. }
            | MuxCommand::ListWorkspaces
            | MuxCommand::KillServer
            | MuxCommand::SendKeys { .. }
            | MuxCommand::CapturePane { .. }
            | MuxCommand::PaneTitle { .. }
            | MuxCommand::PaneInfo { .. }
            | MuxCommand::PaneExitedReplay
            | MuxCommand::ShowBuffer
            | MuxCommand::PasteBuffer { .. }
            | MuxCommand::SetClientColors { .. }
            // Reload-config mutates the daemon's in-memory settings
            // copies, but nothing the persistence format carries — the
            // config file on disk is the durable record. Like
            // `set-client-colors` (which mutates pane terminals yet never
            // saves): no state save, ever.
            | MuxCommand::ReloadConfig
            | MuxCommand::Version => false,
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

/// cap: Columns a par-mux client may report for a window grid (`refresh-client -C`).
pub(crate) const MAX_CLIENT_COLS: u16 = 1000;
/// cap: Rows a par-mux client may report for a window grid (`refresh-client -C`).
pub(crate) const MAX_CLIENT_ROWS: u16 = 500;
/// cap: Pixels per cell axis a par-mux client may report (`refresh-client -p`).
pub(crate) const MAX_CELL_PIXELS: u16 = 512;

// Per-verb parsers (ARC-003): `COMMANDS` below names them; the shared
// `Args`/`LeadingFlags` grammar stays here.
mod buffers;
mod keys;
mod panes;
mod sessions;
mod windows;
use buffers::*;
use keys::*;
use panes::*;
use sessions::*;
use windows::*;

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

    /// Every value following `flag`, in order, read through
    /// [`shell_split`] so a quoted value survives as one word — the
    /// repeatable-flag form (`-e A=1 -e 'B=two words'`).
    fn quoted_values(&self, flag: &str) -> Vec<String> {
        let words = shell_split(self.line);
        words
            .iter()
            .enumerate()
            .filter(|(_, w)| *w == flag)
            .filter_map(|(i, _)| words.get(i + 1).cloned())
            .collect()
    }

    /// Presence check for valueless flags (`-h`, `-R`, …) — [`Self::flag`]
    /// cannot distinguish "absent" from "present with no following token".
    fn has_flag(&self, flag: &str) -> bool {
        self.args.contains(&flag)
    }

    /// A target flag's value: a typed `%N`/`@N`/`$N` id or a name (see
    /// [`Target`]), read through [`Self::quoted_flag`] so a name may
    /// contain spaces. The value is classified, not resolved — the
    /// daemon-side tree does the name matching.
    fn pane(&self, flag_name: &str) -> Result<Target<PaneId>, String> {
        let raw = self
            .quoted_flag(flag_name)?
            .ok_or_else(|| format!("{} requires {flag_name}", self.name))?;
        Target::parse(&raw).map_err(|_| format!("invalid pane target: {raw}"))
    }

    /// [`Self::pane`] for window targets.
    fn window(&self, flag_name: &str) -> Result<Target<WindowId>, String> {
        let raw = self
            .quoted_flag(flag_name)?
            .ok_or_else(|| format!("{} requires {flag_name}", self.name))?;
        Target::parse(&raw).map_err(|_| format!("invalid window target: {raw}"))
    }

    /// [`Self::pane`] for session targets, flag-optional as before.
    fn session(&self, flag_name: &str) -> Result<Option<Target<SessionId>>, String> {
        match self.quoted_flag(flag_name)? {
            Some(raw) => Ok(Some(
                Target::parse(&raw).map_err(|_| format!("invalid session target: {raw}"))?,
            )),
            None => Ok(None),
        }
    }

    /// [`Self::pane`] for a target of any object kind — the
    /// `split-window`/`new-window` shape (tmux accepts `$`/`@`/`%` targets
    /// on both). The sigil picks the id kind; a bare value stays a name
    /// with the command's own kind. `label` names that kind for the
    /// malformed-id error, matching the single-kind helpers' wording.
    fn any_target_opt(&self, flag_name: &str, label: &str) -> Result<Option<AnyTarget>, String> {
        match self.quoted_flag(flag_name)? {
            Some(raw) => Ok(Some(
                AnyTarget::parse(&raw).map_err(|_| format!("invalid {label} target: {raw}"))?,
            )),
            None => Ok(None),
        }
    }

    /// The flag-required form of [`Self::any_target_opt`].
    fn any_target(&self, flag_name: &str, label: &str) -> Result<AnyTarget, String> {
        self.any_target_opt(flag_name, label)?
            .ok_or_else(|| format!("{} requires {flag_name}", self.name))
    }

    /// [`Self::pane`] for workspace targets: a `+N`-sigiled value parses
    /// as the typed id, anything else is a name.
    fn workspace(&self, flag_name: &str) -> Result<Target<WorkspaceId>, String> {
        self.workspace_opt(flag_name)?
            .ok_or_else(|| format!("{} requires {flag_name}", self.name))
    }

    /// The flag-optional form of [`Self::workspace`].
    fn workspace_opt(&self, flag_name: &str) -> Result<Option<Target<WorkspaceId>>, String> {
        match self.quoted_flag(flag_name)? {
            Some(raw) => Ok(Some(
                Target::parse(&raw).map_err(|_| format!("invalid workspace target: {raw}"))?,
            )),
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

    /// A `WxH` size-pair flag (`-C 120x40`) — the renderer-report form —
    /// with each axis in `1..=max` (QA-182: an unbounded grid report can
    /// abort the daemon on allocation).
    fn size_pair(&self, flag_name: &str, max: (u16, u16)) -> Result<Option<(u16, u16)>, String> {
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
        if width > max.0 || height > max.1 {
            return Err(format!(
                "{}: {flag_name} size exceeds {}x{}: {raw}",
                self.name, max.0, max.1
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

/// One flag a command accepts ahead of its free-text tail (SEC-126).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LeadingFlag {
    /// Presence only (`-k`).
    Bare(&'static str),
    /// Exactly one value word (`-t v`); a second occurrence is an error.
    Valued(&'static str),
    /// A value word that may repeat (`-e NAME=V`).
    #[allow(dead_code)] // reserved for QA-219's new-session -e
    Repeated(&'static str),
}

impl LeadingFlag {
    fn name(self) -> &'static str {
        match self {
            Self::Bare(n) | Self::Valued(n) | Self::Repeated(n) => n,
        }
    }
}

/// The flags that lead a line, and the raw text after them.
#[derive(Debug, Default, PartialEq, Eq)]
struct LeadingFlags<'a> {
    /// Flags in line order; values unquoted by the [`shell_split`] grammar.
    flags: Vec<(&'static str, Option<String>)>,
    /// Raw slice after the last flag (or after `--`), leading whitespace
    /// dropped and trailing whitespace trimmed; empty when nothing follows.
    tail: &'a str,
}

impl LeadingFlags<'_> {
    /// The last value given for `flag`.
    fn value(&self, flag: &str) -> Option<&str> {
        self.flags
            .iter()
            .rev()
            .find(|(f, _)| *f == flag)
            .and_then(|(_, v)| v.as_deref())
    }

    /// Every value given for `flag`, in line order.
    #[allow(dead_code)] // reserved for QA-219's new-session -e
    fn values(&self, flag: &str) -> Vec<&str> {
        self.flags
            .iter()
            .filter(|(f, _)| *f == flag)
            .filter_map(|(_, v)| v.as_deref())
            .collect()
    }

    /// Whether `flag` appeared.
    fn has(&self, flag: &str) -> bool {
        self.flags.iter().any(|(f, _)| *f == flag)
    }
}

/// The next shell word of `s` at or after byte `from`: its raw byte range
/// and its unquoted text, under exactly [`shell_split`]'s rules (`'…'` and
/// `"…"` literal, `\` escapes the next char, unquoted Unicode whitespace
/// separates). `None` when only whitespace remains. Every index comes from
/// `char_indices`, so the range always lies on char boundaries.
fn next_shell_word(s: &str, from: usize) -> Option<(usize, usize, String)> {
    let mut chars = s[from..]
        .char_indices()
        .map(|(i, c)| (from + i, c))
        .peekable();
    while chars.next_if(|(_, c)| c.is_whitespace()).is_some() {}
    let (start, _) = *chars.peek()?;
    let mut end = s.len();
    let mut word = String::new();
    while let Some((i, c)) = chars.next() {
        match c {
            '\'' | '"' => {
                for (_, inner) in chars.by_ref() {
                    if inner == c {
                        break;
                    }
                    word.push(inner);
                }
            }
            '\\' => {
                if let Some((_, escaped)) = chars.next() {
                    word.push(escaped);
                }
            }
            c if c.is_whitespace() => {
                end = i;
                break;
            }
            c => word.push(c),
        }
    }
    Some((start, end, word))
}

/// Split `line` into the flags that lead it and the raw text after them
/// (SEC-126), with getopt semantics: flags are read only until the first
/// non-flag word or `--`, so a `-k`/`-c` inside a command belongs to the
/// command. The tail is a raw slice, not a rejoined token list, so its
/// quoting and whitespace reach the pane's shell unchanged.
///
/// An unknown `-x` word before the tail is an error (a command starting
/// with `-` must follow `--`), as is a missing value or a repeated
/// [`LeadingFlag::Valued`] flag. Combined flags (`-kt`) are not supported.
fn split_leading_flags<'a>(
    line: &'a str,
    name: &str,
    spec: &[LeadingFlag],
) -> Result<LeadingFlags<'a>, String> {
    // trim_start first: the name may sit behind leading whitespace (the
    // fuzz-found `parse_send_keys` panic).
    let trimmed = line.trim_start();
    let rest = trimmed
        .strip_prefix(name)
        .ok_or_else(|| format!("{name}: malformed command line"))?;
    let base = line.len() - rest.len();
    let mut lead = LeadingFlags::default();
    let mut pos = base;
    while let Some((start, end, word)) = next_shell_word(line, pos) {
        if word == "--" {
            lead.tail = line[end..].trim();
            return Ok(lead);
        }
        match spec.iter().copied().find(|f| f.name() == word) {
            Some(LeadingFlag::Bare(flag)) => {
                lead.flags.push((flag, None));
                pos = end;
            }
            Some(kind @ (LeadingFlag::Valued(flag) | LeadingFlag::Repeated(flag))) => {
                if matches!(kind, LeadingFlag::Valued(_)) && lead.has(flag) {
                    return Err(format!("{name}: duplicate {flag}"));
                }
                let (value_start, value_end, unquoted) = next_shell_word(line, end)
                    .ok_or_else(|| format!("{name}: {flag} requires a value"))?;
                // Same rule as `Args::quoted_flag`: only a value that opens
                // a quote is unquoted, so a bare `C:\Users\me` or `a\b`
                // stays byte-for-byte.
                let raw = &line[value_start..value_end];
                let value = if raw.starts_with(['\'', '"']) {
                    unquoted
                } else {
                    raw.to_string()
                };
                lead.flags.push((flag, Some(value)));
                pos = value_end;
            }
            None if word.len() > 1 && word.starts_with('-') => {
                return Err(format!("{name}: unknown flag {word}"));
            }
            None => {
                lead.tail = line[start..].trim_end();
                return Ok(lead);
            }
        }
    }
    Ok(lead)
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

/// The command table (ARC-094b): every command's wire name, parser, and —
/// third column — the feature tokens `list-commands` (ENH-037) reports for
/// it, so a command's parser and its advertised metadata live on one
/// declarative row instead of parallel lists. Feature grammar: tokens are
/// `[a-z-]+`; a client must ignore unknown tokens and lines, and a daemon
/// never removes a token without a CHANGELOG "Removed" entry. Adding a tmux
/// command is one `parse_<cmd>` function plus one row here.
const COMMANDS: &[(&str, CommandParser, &[&str])] = &[
    ("new-session", parse_new_session, &["workspace"]),
    ("new-workspace", parse_new_workspace, &[]),
    ("list-workspaces", parse_list_workspaces, &[]),
    ("select-workspace", parse_select_workspace, &[]),
    ("switch-client", parse_switch_client, &[]),
    ("rename-workspace", parse_rename_workspace, &[]),
    ("kill-workspace", parse_kill_workspace, &[]),
    ("list-panes", parse_list_panes, &["targeted"]),
    ("list-agents", parse_list_agents, &[]),
    ("list-commands", parse_list_commands, &[]),
    ("kill-pane", parse_kill_pane, &[]),
    (
        "refresh-client",
        parse_refresh_client,
        &["cell-pixels", "chrome"],
    ),
    ("send-keys", parse_send_keys, &[]),
    ("new-window", parse_new_window, &[]),
    ("select-window", parse_select_window, &[]),
    ("kill-window", parse_kill_window, &[]),
    ("rename-window", parse_rename_window, &[]),
    ("list-windows", parse_list_windows, &["targeted"]),
    ("list-sessions", parse_list_sessions, &["workspace"]),
    ("kill-server", parse_kill_server, &[]),
    ("rename-session", parse_rename_session, &[]),
    ("kill-session", parse_kill_session, &[]),
    ("split-window", parse_split_window, &["before", "start-dir"]),
    ("select-pane", parse_select_pane, &[]),
    ("pane-title", parse_pane_title, &[]),
    ("pane-info", parse_pane_info, &["cmd"]),
    ("pane-exited-replay", parse_pane_exited_replay, &[]),
    ("clear-history", parse_clear_history, &[]),
    ("resize-pane", parse_resize_pane, &["zoom", "absolute"]),
    ("swap-pane", parse_swap_pane, &[]),
    ("break-pane", parse_break_pane, &[]),
    ("join-pane", parse_join_pane, &[]),
    ("move-window", parse_move_window, &[]),
    ("swap-window", parse_swap_windows, &[]),
    ("respawn-pane", parse_respawn_pane, &["kill", "start-dir"]),
    ("capture-pane", parse_capture_pane, &["escape"]),
    ("set-buffer", parse_set_buffer, &[]),
    ("set-client-colors", parse_set_client_colors, &[]),
    ("set-environment", parse_set_environment, &[]),
    ("show-buffer", parse_show_buffer, &[]),
    ("paste-buffer", parse_paste_buffer, &[]),
    ("version", parse_version, &[]),
    ("reload-config", parse_reload_config, &[]),
];

/// The `list-commands` reply body (ENH-037): one `name [feature …]` line
/// per [`COMMANDS`] row, sorted by name, plus one daemon-level trailing
/// line announcing that registration replays held state. The `features`
/// line has no `%` prefix and `features` is not a command name, so neither
/// line can be mistaken for a command row by a reader classifying by grammar.
pub(crate) fn list_commands_body() -> String {
    let mut lines: Vec<String> = COMMANDS
        .iter()
        .map(|(name, _, features)| {
            if features.is_empty() {
                (*name).to_string()
            } else {
                format!("{name} {}", features.join(" "))
            }
        })
        .collect();
    lines.sort();
    lines.push("features replay-held-state".to_string());
    lines.join("\n")
}

/// Parse one command line from a client.
///
/// Deliberately minimal: whitespace-split with a flag scan. tmux's real
/// argument grammar (`--`, per-command option tables, command sequences) is
/// not a goal here, and pretending to implement it would hide that.
///
/// Quoting is honored in a fixed set of places, all of them values that may
/// legitimately contain a space, and all sharing the one bounded grammar in
/// [`shell_split`] (single or double quotes, backslash escapes outside
/// quotes, the `'\''` close-escape-reopen idiom; no interpolation):
/// - the `send-keys` payload (see [`parse_send_keys_payload`]), because key
///   names, `-l` and `-H` cannot survive a whitespace split;
/// - `new-session -s NAME` and `new-window -n NAME` (see
///   [`Args::quoted_flag`]) — tmux admits any non-empty session or window
///   name, spaces included;
/// - environment values: `new-session -e NAME=VALUE` (see
///   [`Args::quoted_values`]) and the `set-environment` words;
/// - the `set-buffer` payload (see [`parse_set_buffer`]), because a
///   clipboard copy may contain spaces, quotes and newlines — with `-H`
///   as the hex escape hatch for anything the line-delimited wire cannot
///   carry.
///
/// Every other flag stays whitespace-split where values cannot contain
/// whitespace. `-t`/`-s` targets are the exception that now joins the
/// quoted set (see [`Args::pane`]): a target is a typed `$N`/`@N`/`%N`
/// id OR a name — pane user title, window name, session name — resolved
/// daemon-side against the tree, and a name may contain spaces, so it is
/// read through [`Args::quoted_flag`]. `send-keys` alone keeps a
/// single-token target (see [`parse_send_keys`]), quoted alongside the
/// payload it would collide with.
///
/// Two commands take trailing free text. `respawn-pane` reads its flags
/// only up to the first command word or `--` and passes the raw rest of
/// the line to the pane's shell, quoting and whitespace intact (see
/// [`split_leading_flags`]). `rename-window` is the one remaining
/// [`Args::trailing_after`] user, joining the words after `-t`.
pub fn parse_command(line: &str) -> Result<MuxCommand, String> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    let Some((name, args)) = parts.split_first() else {
        return Err("empty command".to_string());
    };
    let a = Args { name, args, line };
    let (_, parse, _) = COMMANDS
        .iter()
        .find(|(known, _, _)| known == name)
        .ok_or_else(|| format!("unknown command: {name}"))?;
    parse(&a)
}

#[cfg(test)]
mod tests;
