//! Subscription, paste-buffer, and desktop-notification lines.

use crate::tmux_control::TmuxNotification;

pub(super) fn parse_subscription_changed(args: &str) -> Option<TmuxNotification> {
    let parts: Vec<&str> = args.splitn(2, ' ').collect();
    if parts.is_empty() {
        return None;
    }
    // tmux(1): `name session-id window-id window-index pane-id ... :
    // value` — the value follows the first lone `:`; the ids and any
    // future-use arguments before it are not part of it. A line with no
    // lone colon keeps the whole remainder as the value.
    let rest = parts.get(1).copied().unwrap_or_default();
    let value = match rest.strip_prefix(": ") {
        Some(value) => value,
        None => rest.find(" : ").map_or(rest, |pos| &rest[pos + 3..]),
    };
    Some(TmuxNotification::SubscriptionChanged {
        name: parts[0].to_string(),
        value: value.to_string(),
    })
}

pub(super) fn parse_paste_buffer_changed(args: &str) -> Option<TmuxNotification> {
    Some(TmuxNotification::PasteBufferChanged {
        name: args.trim().to_string(),
    })
}

pub(super) fn parse_paste_buffer_deleted(args: &str) -> Option<TmuxNotification> {
    Some(TmuxNotification::PasteBufferDeleted {
        name: args.trim().to_string(),
    })
}

/// `%notification <b64 title> [body=<b64 body>]`: title and body ride
/// the standard base64 of their text, one whitespace-free token each.
/// A missing, malformed, non-UTF-8, or empty title drops the line; a
/// bad body token reads as no body.
pub(super) fn parse_notification(args: &str) -> Option<TmuxNotification> {
    use base64::Engine as _;
    let decode = |token: &str| {
        base64::engine::general_purpose::STANDARD
            .decode(token)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
    };
    let mut parts = args.split_whitespace();
    let title = decode(parts.next()?).filter(|t| !t.is_empty())?;
    let body = parts
        .find_map(|token| token.strip_prefix("body="))
        .and_then(decode)
        .unwrap_or_default();
    Some(TmuxNotification::Notification { title, body })
}
