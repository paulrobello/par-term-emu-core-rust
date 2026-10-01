//! Structured field view of a [`TerminalEvent`] (ARC-114).
//!
//! One renderer-neutral source of truth for an event's named fields, shared
//! by the Python event dicts (`python_bindings::observer::event_to_dict`) and
//! the C ABI's structured `on_event_v2` JSON payload (`ffi`), so both surfaces
//! carry the same keys and value types.

use crate::terminal::TerminalEvent;

/// A field value in an event dictionary, before it is rendered to Python
/// or to the FFI JSON payload.
///
/// `event_fields` is the single source of truth for event-to-dict conversion,
/// shared by `poll_events()`, `poll_subscribed_events()`, and observer
/// dispatch. Numeric, boolean, and optional Rust fields map to `Int`/`Bool`/
/// `None` so the Python-facing dicts carry native types.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum EventField {
    Str(String),
    Int(i64),
    Bool(bool),
    None,
}

/// Collect an event's dictionary fields with native Rust types.
///
/// Optional fields are always present, as `EventField::None` when unset — the
/// legacy renderer omits them instead.
pub(crate) fn event_fields(event: &TerminalEvent) -> Vec<(String, EventField)> {
    let mut fields: Vec<(String, EventField)> = Vec::new();
    macro_rules! put {
        ($key:expr, $val:expr) => {
            fields.push(($key.to_string(), $val))
        };
    }
    put!("type", EventField::Str(event.kind().as_str().to_string()));
    match event {
        TerminalEvent::BellRang(bell) => match bell {
            crate::terminal::BellEvent::VisualBell => {
                put!("bell_type", EventField::Str("visual".to_string()));
            }
            crate::terminal::BellEvent::WarningBell(vol) => {
                put!("bell_type", EventField::Str("warning".to_string()));
                put!("volume", EventField::Int(*vol as i64));
            }
            crate::terminal::BellEvent::MarginBell(vol) => {
                put!("bell_type", EventField::Str("margin".to_string()));
                put!("volume", EventField::Int(*vol as i64));
            }
        },
        TerminalEvent::TitleChanged(title) => {
            put!("title", EventField::Str(title.clone()));
        }
        TerminalEvent::SizeChanged(cols, rows) => {
            put!("cols", EventField::Int(*cols as i64));
            put!("rows", EventField::Int(*rows as i64));
        }
        TerminalEvent::ModeChanged(mode, enabled) => {
            put!("mode", EventField::Str(mode.clone()));
            put!("enabled", EventField::Bool(*enabled));
        }
        TerminalEvent::GraphicsAdded(row) => {
            put!("row", EventField::Int(*row as i64));
        }
        TerminalEvent::HyperlinkAdded { url, row, col, id } => {
            put!("url", EventField::Str(url.clone()));
            put!("row", EventField::Int(*row as i64));
            put!("col", EventField::Int(*col as i64));
            put!(
                "id",
                id.map_or(EventField::None, |v| EventField::Int(v as i64))
            );
        }
        TerminalEvent::DirtyRegion(first, last) => {
            put!("first_row", EventField::Int(*first as i64));
            put!("last_row", EventField::Int(*last as i64));
        }
        TerminalEvent::CwdChanged(change) => {
            put!(
                "old_cwd",
                change
                    .old_cwd
                    .clone()
                    .map_or(EventField::None, EventField::Str)
            );
            put!("new_cwd", EventField::Str(change.new_cwd.clone()));
            put!(
                "hostname",
                change
                    .hostname
                    .clone()
                    .map_or(EventField::None, EventField::Str)
            );
            put!(
                "username",
                change
                    .username
                    .clone()
                    .map_or(EventField::None, EventField::Str)
            );
            put!("timestamp", EventField::Int(change.timestamp as i64));
        }
        TerminalEvent::TriggerMatched(trigger_match) => {
            put!(
                "trigger_id",
                EventField::Int(trigger_match.trigger_id as i64)
            );
            put!("row", EventField::Int(trigger_match.row as i64));
            put!("col", EventField::Int(trigger_match.col as i64));
            put!("end_col", EventField::Int(trigger_match.end_col as i64));
            put!("text", EventField::Str(trigger_match.text.clone()));
            put!("timestamp", EventField::Int(trigger_match.timestamp as i64));
        }
        TerminalEvent::UserVarChanged {
            name,
            value,
            old_value,
        } => {
            put!("name", EventField::Str(name.clone()));
            put!("value", EventField::Str(value.clone()));
            put!(
                "old_value",
                old_value.clone().map_or(EventField::None, EventField::Str)
            );
        }
        TerminalEvent::ProgressBarChanged {
            action,
            id,
            state,
            percent,
            label,
        } => {
            let action_str = match action {
                crate::terminal::ProgressBarAction::Set => "set",
                crate::terminal::ProgressBarAction::Remove => "remove",
                crate::terminal::ProgressBarAction::RemoveAll => "remove_all",
            };
            put!("action", EventField::Str(action_str.to_string()));
            put!("id", EventField::Str(id.clone()));
            put!(
                "state",
                state
                    .as_ref()
                    .map(|s| EventField::Str(s.description().to_string()))
                    .unwrap_or(EventField::None)
            );
            put!(
                "percent",
                percent.map_or(EventField::None, |v| EventField::Int(v as i64))
            );
            put!(
                "label",
                label.clone().map_or(EventField::None, EventField::Str)
            );
        }
        TerminalEvent::BadgeChanged(badge) => {
            put!(
                "badge",
                badge.clone().map_or(EventField::None, EventField::Str)
            );
        }
        TerminalEvent::ShellIntegrationEvent {
            event_type,
            command,
            exit_code,
            timestamp,
            cursor_line,
        } => {
            put!("event_type", EventField::Str(event_type.clone()));
            put!(
                "command",
                command.clone().map_or(EventField::None, EventField::Str)
            );
            put!(
                "exit_code",
                exit_code.map_or(EventField::None, |v| EventField::Int(v as i64))
            );
            put!(
                "cursor_line",
                cursor_line.map_or(EventField::None, |v| EventField::Int(v as i64))
            );
            put!(
                "timestamp",
                timestamp.map_or(EventField::None, |v| EventField::Int(v as i64))
            );
        }
        TerminalEvent::ZoneOpened {
            zone_id,
            zone_type,
            abs_row_start,
        } => {
            put!("zone_id", EventField::Int(*zone_id as i64));
            put!("zone_type", EventField::Str(zone_type.to_string()));
            put!("abs_row_start", EventField::Int(*abs_row_start as i64));
        }
        TerminalEvent::ZoneClosed {
            zone_id,
            zone_type,
            abs_row_start,
            abs_row_end,
            exit_code,
        } => {
            put!("zone_id", EventField::Int(*zone_id as i64));
            put!("zone_type", EventField::Str(zone_type.to_string()));
            put!("abs_row_start", EventField::Int(*abs_row_start as i64));
            put!("abs_row_end", EventField::Int(*abs_row_end as i64));
            put!(
                "exit_code",
                exit_code.map_or(EventField::None, |v| EventField::Int(v as i64))
            );
        }
        TerminalEvent::ZoneScrolledOut { zone_id, zone_type } => {
            put!("zone_id", EventField::Int(*zone_id as i64));
            put!("zone_type", EventField::Str(zone_type.to_string()));
        }
        TerminalEvent::EnvironmentChanged {
            key,
            value,
            old_value,
        } => {
            put!("key", EventField::Str(key.clone()));
            put!("value", EventField::Str(value.clone()));
            put!(
                "old_value",
                old_value.clone().map_or(EventField::None, EventField::Str)
            );
        }
        TerminalEvent::RemoteHostTransition {
            hostname,
            username,
            old_hostname,
            old_username,
        } => {
            put!("hostname", EventField::Str(hostname.clone()));
            put!(
                "username",
                username.clone().map_or(EventField::None, EventField::Str)
            );
            put!(
                "old_hostname",
                old_hostname
                    .clone()
                    .map_or(EventField::None, EventField::Str)
            );
            put!(
                "old_username",
                old_username
                    .clone()
                    .map_or(EventField::None, EventField::Str)
            );
        }
        TerminalEvent::SubShellDetected { depth, shell_type } => {
            put!("depth", EventField::Int(*depth as i64));
            put!(
                "shell_type",
                shell_type.clone().map_or(EventField::None, EventField::Str)
            );
        }
        TerminalEvent::FileTransferStarted {
            id,
            direction,
            filename,
            total_bytes,
        } => {
            put!("id", EventField::Int(*id as i64));
            let dir_str = match direction {
                crate::terminal::TransferDirection::Download => "download",
                crate::terminal::TransferDirection::Upload => "upload",
            };
            put!("direction", EventField::Str(dir_str.to_string()));
            put!(
                "filename",
                filename.clone().map_or(EventField::None, EventField::Str)
            );
            put!(
                "total_bytes",
                total_bytes.map_or(EventField::None, |v| EventField::Int(v as i64))
            );
        }
        TerminalEvent::FileTransferProgress {
            id,
            bytes_transferred,
            total_bytes,
        } => {
            put!("id", EventField::Int(*id as i64));
            put!(
                "bytes_transferred",
                EventField::Int(*bytes_transferred as i64)
            );
            put!(
                "total_bytes",
                total_bytes.map_or(EventField::None, |v| EventField::Int(v as i64))
            );
        }
        TerminalEvent::FileTransferCompleted { id, filename, size } => {
            put!("id", EventField::Int(*id as i64));
            put!(
                "filename",
                filename.clone().map_or(EventField::None, EventField::Str)
            );
            put!("size", EventField::Int(*size as i64));
        }
        TerminalEvent::FileTransferFailed { id, reason } => {
            put!("id", EventField::Int(*id as i64));
            put!("reason", EventField::Str(reason.clone()));
        }
        TerminalEvent::UploadRequested { format } => {
            put!("format", EventField::Str(format.clone()));
        }
        TerminalEvent::ScreenCleared { include_scrollback } => {
            put!("include_scrollback", EventField::Bool(*include_scrollback));
        }
        TerminalEvent::InlineImageDropped { reason } => {
            put!("reason", EventField::Str(reason.clone()));
        }
    }
    fields
}
