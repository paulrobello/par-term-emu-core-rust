//! [`TmuxNotification`]: the tmux control-protocol notification taxonomy and
//! its `notification_type()` classification.

/// Tmux control protocol notification types
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TmuxNotification {
    /// Command output block started
    /// Arguments: timestamp, command_number, flags
    Begin {
        /// Unix timestamp of the block.
        timestamp: u64,
        /// Command sequence number.
        command_number: u32,
        /// Command flags.
        flags: String,
    },

    /// Command output block ended successfully
    /// Arguments: timestamp, command_number, flags
    End {
        /// Unix timestamp of the block.
        timestamp: u64,
        /// Command sequence number.
        command_number: u32,
        /// Command flags.
        flags: String,
    },

    /// Command output block ended with error
    /// Arguments: timestamp, command_number, flags
    Error {
        /// Unix timestamp of the block.
        timestamp: u64,
        /// Command sequence number.
        command_number: u32,
        /// Command flags.
        flags: String,
    },

    /// Pane output data
    /// Arguments: pane_id, output_data (octal-escaped for control chars)
    Output {
        /// Pane id (e.g. `%1`).
        pane_id: String,
        /// Raw output bytes (control characters decoded).
        data: Vec<u8>,
    },

    /// Pane mode changed
    /// Arguments: pane_id
    PaneModeChanged {
        /// Pane id (e.g. `%1`).
        pane_id: String,
    },

    /// Window's active pane changed
    /// Arguments: window_id, pane_id
    WindowPaneChanged {
        /// Window id (e.g. `@1`).
        window_id: String,
        /// Pane id (e.g. `%1`).
        pane_id: String,
    },

    /// Window closed in attached session
    /// Arguments: window_id
    WindowClose {
        /// Window id (e.g. `@1`).
        window_id: String,
    },

    /// Window closed in another session (unlinked)
    /// Arguments: window_id
    UnlinkedWindowClose {
        /// Window id (e.g. `@1`).
        window_id: String,
    },

    /// Window added to attached session
    /// Arguments: window_id, [window_layout, window_visible_layout, window_raw_flags]
    ///
    /// The layout triple mirrors [`TmuxNotification::LayoutChange`] and is
    /// optional: a bare-id line (real tmux's shape) parses with the three
    /// fields empty.
    WindowAdd {
        /// Window id (e.g. `@1`).
        window_id: String,
        /// Window layout string.
        window_layout: String,
        /// Visible window layout string.
        window_visible_layout: String,
        /// Raw window flags.
        window_raw_flags: String,
    },

    /// Window added to another session (unlinked)
    /// Arguments: window_id
    UnlinkedWindowAdd {
        /// Window id (e.g. `@1`).
        window_id: String,
    },

    /// Window renamed in attached session
    /// Arguments: window_id, new_name
    WindowRenamed {
        /// Window id (e.g. `@1`).
        window_id: String,
        /// New window name.
        name: String,
    },

    /// Window renamed in another session (unlinked)
    /// Arguments: window_id, new_name
    UnlinkedWindowRenamed {
        /// Window id (e.g. `@1`).
        window_id: String,
        /// New window name.
        name: String,
    },

    /// Attached session changed
    /// Arguments: session_id, session_name
    SessionChanged {
        /// Session id (e.g. `$1`).
        session_id: String,
        /// Session name.
        name: String,
    },

    /// Another client's session changed
    /// Arguments: client_name, session_id, session_name
    ClientSessionChanged {
        /// Client name.
        client: String,
        /// Session id (e.g. `$1`).
        session_id: String,
        /// Session name.
        name: String,
    },

    /// Session renamed
    /// Arguments: session_id, new_name
    SessionRenamed {
        /// Session id (e.g. `$1`).
        session_id: String,
        /// New session name.
        name: String,
    },

    /// Sessions changed (created or destroyed)
    SessionsChanged,

    /// Workspace roster or selection changed (par-mux's own notification,
    /// above tmux's set): a workspace was created, killed, renamed, or
    /// selected, or a session's death removed its emptied workspace.
    /// Argument-less by the same convention as `SessionsChanged` — clients
    /// re-query `list-workspaces` rather than parse a diff.
    WorkspacesChanged,

    /// The daemon re-read its config file on a successful `reload-config`
    /// (par-mux's own notification). Argument-less: attached clients
    /// re-read the file and re-apply their `[client]` settings, so a
    /// settings change made in one client reaches every other one.
    ConfigChanged,

    /// Session's current window changed
    /// Arguments: session_id, window_id
    SessionWindowChanged {
        /// Session id (e.g. `$1`).
        session_id: String,
        /// Window id (e.g. `@1`).
        window_id: String,
    },

    /// Client detached
    /// Arguments: client_name
    ClientDetached {
        /// Client name.
        client: String,
    },

    /// par-mux: a control client registered — joined the broadcast set
    /// on its first control command. Sent to the clients already
    /// registered, never to the joining client itself.
    /// Arguments: client_id
    ClientAttached {
        /// The daemon's connection id (decimal).
        client: String,
    },

    /// par-mux: a registered control client's connection ended — a clean
    /// disconnect, a socket EOF, or an eviction teardown alike.
    /// Arguments: client_id [session_id window_id]
    ClientLeft {
        /// The daemon's connection id (decimal).
        client: String,
        /// The session of the window the client was displaying; None when
        /// it never reported a view (`refresh-client -C`).
        session_id: Option<String>,
        /// The window the client was displaying; None when it never
        /// reported a view.
        window_id: Option<String>,
    },

    /// Client exited (only with -CC flag)
    Exit,

    /// Pane output paused (flow control)
    /// Arguments: pane_id
    Pause {
        /// Pane id (e.g. `%1`).
        pane_id: String,
    },

    /// Extended output notification (flow control)
    /// Arguments: pane_id, milliseconds_behind, output_data
    ExtendedOutput {
        /// Pane id (e.g. `%1`).
        pane_id: String,
        /// How far output is behind, in milliseconds.
        delay_ms: u64,
        /// Raw output bytes (control characters decoded).
        data: Vec<u8>,
    },

    /// Continue after pause (flow control)
    Continue {
        /// Pane id (e.g. `%1`) whose output resumes.
        pane_id: String,
    },

    /// Subscription value changed
    /// Arguments: subscription_name, value
    SubscriptionChanged {
        /// Subscription name.
        name: String,
        /// New subscription value.
        value: String,
    },

    /// Window layout changed
    /// Arguments: window_id, window_layout, window_visible_layout, window_raw_flags
    LayoutChange {
        /// Window id (e.g. `@1`).
        window_id: String,
        /// Window layout string.
        window_layout: String,
        /// Visible window layout string.
        window_visible_layout: String,
        /// Raw window flags.
        window_raw_flags: String,
    },

    /// Paste buffer changed
    /// Arguments: buffer_name
    PasteBufferChanged {
        /// Buffer name.
        name: String,
    },

    /// Paste buffer deleted
    /// Arguments: buffer_name
    PasteBufferDeleted {
        /// Buffer name.
        name: String,
    },

    /// A pane's agent reported a state change (par-mux hook-fed agent
    /// layer, Phase 5 seam S3; real tmux never emits this).
    /// Arguments: pane_id, agent_label, state, then trailing `key=value`
    /// tokens: `source` (`hook` or `scrape`) and `ws` (the owning
    /// workspace, ENH-061; unknown keys are ignored).
    AgentStateChanged {
        /// Pane id (e.g. `%1`).
        pane_id: String,
        /// Agent label.
        agent: String,
        /// Reported agent state.
        state: String,
        /// Who asserted the state: `hook` (the agent claimed it) or
        /// `scrape` (a pattern guessed it from pane content). Empty when
        /// the line carried no `source=` token.
        source: String,
        /// Owning workspace id (`+N`), the side panel's grouping key.
        /// Empty when the line carried no `ws=` token (a pre-ENH-061
        /// daemon).
        ws: String,
        /// The state's message (the blocked reason), the standard base64
        /// of the whitespace-collapsed text from the `msg=` token. Empty
        /// when the line carried no token (an older daemon, or a state
        /// reported without a message) — a consumer renders no message
        /// line, never a placeholder.
        msg: String,
    },

    /// A pane's agent released its claim (`pane.release_agent`; the agent
    /// exited and its hook announced it). The pane left the roster — there
    /// is no state to report, only the removal.
    /// Arguments: pane_id, agent_label
    AgentReleased {
        /// Pane id (e.g. `%1`).
        pane_id: String,
        /// Agent label.
        agent: String,
    },

    /// A pane's agent telemetry changed (`pane.report_agent_telemetry`;
    /// display-only roster data — model, effort, context and rate-limit
    /// percents). Carries identity only, no values: clients re-query
    /// `list-agents` for the fresh blob, the same shape `%sessions-changed`
    /// teaches.
    /// Arguments: pane_id, agent_label
    AgentTelemetryChanged {
        /// Pane id (e.g. `%1`).
        pane_id: String,
        /// Agent label.
        agent: String,
    },

    /// A pane's user title changed (`select-pane -T`; par-mux pane-title
    /// extension — real tmux never emits this).
    /// Arguments: pane_id, new_title (empty = cleared)
    PaneTitleChanged {
        /// Pane id (e.g. `%1`).
        pane_id: String,
        /// New pane title (empty = cleared).
        title: String,
    },

    /// A pane's process exited and the pane is held (remain-on-exit):
    /// the pane, its window, and its frozen screen stay in the tree for
    /// `respawn-pane`. The exit code is `None` only when the daemon had
    /// no exit status to read as it observed the death; a death by signal
    /// reports `1`.
    /// Arguments: pane_id, exit_code (empty = none)
    PaneExited {
        /// Pane id (e.g. `%1`).
        pane_id: String,
        /// Process exit code, if known.
        exit_code: Option<i32>,
    },

    /// A pane's process was restarted in place by `respawn-pane`: same
    /// pane id, window, and layout, fresh terminal. The cue clients use
    /// to clear their exited-state chrome.
    /// Arguments: pane_id
    PaneRespawned {
        /// Pane id (e.g. `%1`).
        pane_id: String,
    },

    /// Unknown or unrecognized notification
    /// Arguments: notification_line
    Unknown {
        /// The raw notification line.
        line: String,
    },

    /// Regular terminal output (non-control mode data)
    /// This is used when we receive data that's not a control protocol message
    TerminalOutput {
        /// Raw output bytes (control characters decoded).
        data: Vec<u8>,
    },
}

impl TmuxNotification {
    /// Get a human-readable description of this notification type
    pub fn notification_type(&self) -> &'static str {
        match self {
            Self::Begin { .. } => "begin",
            Self::End { .. } => "end",
            Self::Error { .. } => "error",
            Self::Output { .. } => "output",
            Self::PaneModeChanged { .. } => "pane-mode-changed",
            Self::WindowPaneChanged { .. } => "window-pane-changed",
            Self::WindowClose { .. } => "window-close",
            Self::UnlinkedWindowClose { .. } => "unlinked-window-close",
            Self::WindowAdd { .. } => "window-add",
            Self::UnlinkedWindowAdd { .. } => "unlinked-window-add",
            Self::WindowRenamed { .. } => "window-renamed",
            Self::UnlinkedWindowRenamed { .. } => "unlinked-window-renamed",
            Self::SessionChanged { .. } => "session-changed",
            Self::ClientSessionChanged { .. } => "client-session-changed",
            Self::SessionRenamed { .. } => "session-renamed",
            Self::SessionsChanged => "sessions-changed",
            Self::WorkspacesChanged => "workspaces-changed",
            Self::ConfigChanged => "config-changed",
            Self::SessionWindowChanged { .. } => "session-window-changed",
            Self::ClientDetached { .. } => "client-detached",
            Self::ClientAttached { .. } => "client-attached",
            Self::ClientLeft { .. } => "client-left",
            Self::Exit => "exit",
            Self::Pause { .. } => "pause",
            Self::ExtendedOutput { .. } => "extended-output",
            Self::Continue { .. } => "continue",
            Self::SubscriptionChanged { .. } => "subscription-changed",
            Self::LayoutChange { .. } => "layout-change",
            Self::PasteBufferChanged { .. } => "paste-buffer-changed",
            Self::PasteBufferDeleted { .. } => "paste-buffer-deleted",
            Self::AgentStateChanged { .. } => "agent-state-changed",
            Self::AgentReleased { .. } => "agent-released",
            Self::AgentTelemetryChanged { .. } => "agent-telemetry-changed",
            Self::PaneTitleChanged { .. } => "pane-title-changed",
            Self::PaneExited { .. } => "pane-exited",
            Self::PaneRespawned { .. } => "pane-respawned",
            Self::Unknown { .. } => "unknown",
            Self::TerminalOutput { .. } => "terminal-output",
        }
    }
}
