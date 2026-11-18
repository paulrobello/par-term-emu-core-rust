//! Terminal-event bridging and client event subscriptions: the
//! `TerminalEvent` -> `ServerMessage` mapping and the [`EventType`] a client
//! can subscribe to.

use super::ServerMessage;
use crate::terminal::file_transfer::TransferDirection;
use crate::terminal::TerminalEvent;
use serde::{Deserialize, Serialize};

/// Convert a `TerminalEvent` into the `ServerMessage` a streaming client
/// should receive, or `None` for events with no streaming representation
/// (e.g. `DirtyRegion`, a local rendering hint).
///
/// Single source of truth for the TerminalEvent -> ServerMessage mapping
/// (ARC-020), shared by the broadcast and per-session dispatch paths in the
/// streaming server (previously duplicated as two ~25-arm `match` blocks).
pub fn terminal_event_to_server_message(event: TerminalEvent) -> Option<ServerMessage> {
    Some(match event {
        TerminalEvent::BellRang(_) => ServerMessage::bell(),
        TerminalEvent::TitleChanged(title) => ServerMessage::title(title),
        TerminalEvent::SizeChanged(cols, rows) => ServerMessage::resize(cols as u16, rows as u16),
        TerminalEvent::CwdChanged(cwd) => ServerMessage::cwd_changed_full(
            cwd.old_cwd,
            cwd.new_cwd,
            cwd.hostname.clone(),
            cwd.username,
            cwd.timestamp,
            par_term_emu_core::shell_integration::osc7_host_is_local(cwd.hostname.as_deref()),
        ),
        TerminalEvent::TriggerMatched(tm) => ServerMessage::trigger_matched(
            tm.trigger_id,
            tm.row as u16,
            tm.col as u16,
            tm.end_col as u16,
            tm.text,
            tm.captures,
            tm.timestamp,
        ),
        TerminalEvent::ModeChanged(mode, enabled) => ServerMessage::mode_changed(mode, enabled),
        TerminalEvent::GraphicsAdded { row, .. } => ServerMessage::graphics_added(row as u16),
        TerminalEvent::HyperlinkAdded { url, row, col, id } => {
            if let Some(id) = id {
                ServerMessage::hyperlink_added_with_id(url, row as u16, col as u16, id.to_string())
            } else {
                ServerMessage::hyperlink_added(url, row as u16, col as u16)
            }
        }
        TerminalEvent::UserVarChanged {
            name,
            value,
            old_value,
        } => ServerMessage::user_var_changed_full(name, value, old_value),
        TerminalEvent::ProgressBarChanged {
            action,
            id,
            state,
            percent,
            label,
        } => ServerMessage::progress_bar_changed(action, id, state, percent, label),
        TerminalEvent::BadgeChanged(badge) => ServerMessage::badge_changed(badge),
        TerminalEvent::ShellIntegrationEvent {
            event_type,
            command,
            exit_code,
            timestamp,
            cursor_line,
        } => ServerMessage::shell_integration_event(
            event_type,
            command,
            exit_code,
            timestamp,
            cursor_line.map(|l| l as u64),
        ),
        TerminalEvent::PointerShapeChanged(shape) => ServerMessage::pointer_shape_changed(shape),
        // Local rendering hint: no streaming representation.
        TerminalEvent::DirtyRegion(_, _) => return None,
        TerminalEvent::ZoneOpened {
            zone_id,
            zone_type,
            abs_row_start,
        } => {
            ServerMessage::zone_opened(zone_id as u64, zone_type.to_string(), abs_row_start as u64)
        }
        TerminalEvent::ZoneClosed {
            zone_id,
            zone_type,
            abs_row_start,
            abs_row_end,
            exit_code,
        } => ServerMessage::zone_closed(
            zone_id as u64,
            zone_type.to_string(),
            abs_row_start as u64,
            abs_row_end as u64,
            exit_code,
        ),
        TerminalEvent::ZoneScrolledOut { zone_id, zone_type } => {
            ServerMessage::zone_scrolled_out(zone_id as u64, zone_type.to_string())
        }
        TerminalEvent::EnvironmentChanged {
            key,
            value,
            old_value,
        } => ServerMessage::environment_changed(key, value, old_value),
        TerminalEvent::RemoteHostTransition {
            hostname,
            username,
            old_hostname,
            old_username,
        } => ServerMessage::remote_host_transition(hostname, username, old_hostname, old_username),
        TerminalEvent::SubShellDetected { depth, shell_type } => {
            ServerMessage::sub_shell_detected(depth as u64, shell_type)
        }
        TerminalEvent::FileTransferStarted {
            id,
            direction,
            filename,
            total_bytes,
        } => {
            let dir_str = match direction {
                TransferDirection::Download => "download",
                TransferDirection::Upload => "upload",
            };
            ServerMessage::file_transfer_started(
                id,
                dir_str.to_string(),
                filename,
                total_bytes.map(|b| b as u64),
            )
        }
        TerminalEvent::FileTransferProgress {
            id,
            bytes_transferred,
            total_bytes,
        } => ServerMessage::file_transfer_progress(
            id,
            bytes_transferred as u64,
            total_bytes.map(|b| b as u64),
        ),
        TerminalEvent::FileTransferCompleted { id, filename, size } => {
            ServerMessage::file_transfer_completed(id, filename, size as u64)
        }
        TerminalEvent::FileTransferFailed { id, reason } => {
            ServerMessage::file_transfer_failed(id, reason)
        }
        TerminalEvent::UploadRequested { format } => ServerMessage::upload_requested(format),
        TerminalEvent::ScreenCleared { include_scrollback } => {
            ServerMessage::screen_cleared(include_scrollback)
        }
        TerminalEvent::InlineImageDropped { .. } => {
            // Diagnostic-only event (OSC 1337 drop signal), not part of the
            // streaming wire protocol; Python consumers read it via
            // poll_events()/observers instead.
            return None;
        }
        TerminalEvent::OutputBudgetExceeded { .. } => {
            // Diagnostic-only budget event (ENH-062), not part of the
            // streaming wire protocol; Python consumers read it via
            // poll_events()/observers instead.
            return None;
        }
    })
}

/// Event types that clients can subscribe to
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[cfg_attr(
    all(
        any(feature = "python", feature = "python-test"),
        feature = "streaming"
    ),
    derive(par_term_emu_derive::PyDictConvert)
)]
#[serde(rename_all = "lowercase")]
pub enum EventType {
    /// Terminal output
    Output,
    /// Cursor position changes
    Cursor,
    /// Bell events
    Bell,
    /// Title changes
    Title,
    /// Resize events
    Resize,
    /// CWD change events
    Cwd,
    /// Trigger match events
    Trigger,
    /// Trigger action result events (Notify, MarkLine)
    Action,
    /// Terminal mode change events
    Mode,
    /// Graphics/image events
    Graphics,
    /// Hyperlink events
    Hyperlink,
    /// User variable change events
    #[serde(rename = "user_var")]
    UserVar,
    /// Named progress bar events
    #[serde(rename = "progress_bar")]
    ProgressBar,
    /// Badge change events
    Badge,
    /// Selection events
    Selection,
    /// Clipboard sync events
    Clipboard,
    /// Shell integration events
    Shell,
    /// System resource statistics events
    #[serde(rename = "system_stats")]
    SystemStats,
    /// Zone events (opened, closed, scrolled out)
    Zone,
    /// Environment change events
    Environment,
    /// Remote host transition events
    #[serde(rename = "remote_host")]
    RemoteHost,
    /// Sub-shell detection events
    #[serde(rename = "sub_shell")]
    SubShell,
    /// Semantic snapshot events
    #[serde(rename = "snapshot")]
    Snapshot,
    /// File transfer events (started, progress, completed, failed)
    #[serde(rename = "file_transfer")]
    FileTransfer,
    /// Upload request events
    #[serde(rename = "upload_request")]
    UploadRequest,
    /// Screen cleared events (ED 2J, ED 3J)
    #[serde(rename = "screen_cleared")]
    ScreenCleared,
    /// Pointer shape change events (OSC 22)
    #[serde(rename = "pointer_shape")]
    PointerShape,
}
