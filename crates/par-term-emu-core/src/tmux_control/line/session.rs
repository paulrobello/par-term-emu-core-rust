//! Session and client notifications.

use crate::tmux_control::TmuxNotification;

pub(super) fn parse_session_changed(args: &str) -> Option<TmuxNotification> {
    let parts: Vec<&str> = args.splitn(2, ' ').collect();
    if parts.len() < 2 {
        return None;
    }
    Some(TmuxNotification::SessionChanged {
        session_id: parts[0].to_string(),
        name: parts[1].to_string(),
    })
}

pub(super) fn parse_client_session_changed(args: &str) -> Option<TmuxNotification> {
    let parts: Vec<&str> = args.splitn(3, ' ').collect();
    if parts.len() < 3 {
        return None;
    }
    Some(TmuxNotification::ClientSessionChanged {
        client: parts[0].to_string(),
        session_id: parts[1].to_string(),
        name: parts[2].to_string(),
    })
}

pub(super) fn parse_session_renamed(args: &str) -> Option<TmuxNotification> {
    let parts: Vec<&str> = args.splitn(2, ' ').collect();
    if parts.len() < 2 {
        return None;
    }
    Some(TmuxNotification::SessionRenamed {
        session_id: parts[0].to_string(),
        name: parts[1].to_string(),
    })
}

pub(super) fn parse_session_window_changed(args: &str) -> Option<TmuxNotification> {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    Some(TmuxNotification::SessionWindowChanged {
        session_id: parts[0].to_string(),
        window_id: parts[1].to_string(),
    })
}

pub(super) fn parse_client_detached(args: &str) -> Option<TmuxNotification> {
    Some(TmuxNotification::ClientDetached {
        client: args.trim().to_string(),
    })
}

pub(super) fn parse_client_attached(args: &str) -> Option<TmuxNotification> {
    let client = args.split_whitespace().next()?;
    Some(TmuxNotification::ClientAttached {
        client: client.to_string(),
    })
}

pub(super) fn parse_client_left(args: &str) -> Option<TmuxNotification> {
    let mut parts = args.split_whitespace();
    let client = parts.next()?.to_string();
    let session_id = parts.next().map(str::to_string);
    let window_id = parts.next().map(str::to_string);
    Some(TmuxNotification::ClientLeft {
        client,
        session_id,
        window_id,
    })
}
