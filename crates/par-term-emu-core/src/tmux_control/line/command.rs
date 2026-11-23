//! Tmux `%begin` / `%end` / `%error` command-reply framing lines.

use crate::tmux_control::TmuxNotification;

pub(super) fn parse_begin(args: &str) -> Option<TmuxNotification> {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    Some(TmuxNotification::Begin {
        timestamp: parts[0].parse().ok()?,
        command_number: parts[1].parse().ok()?,
        flags: if parts.len() > 2 {
            parts[2..].join(" ")
        } else {
            String::new()
        },
    })
}

pub(super) fn parse_end(args: &str) -> Option<TmuxNotification> {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    Some(TmuxNotification::End {
        timestamp: parts[0].parse().ok()?,
        command_number: parts[1].parse().ok()?,
        flags: if parts.len() > 2 {
            parts[2..].join(" ")
        } else {
            String::new()
        },
    })
}

pub(super) fn parse_error(args: &str) -> Option<TmuxNotification> {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    Some(TmuxNotification::Error {
        timestamp: parts[0].parse().ok()?,
        command_number: parts[1].parse().ok()?,
        flags: if parts.len() > 2 {
            parts[2..].join(" ")
        } else {
            String::new()
        },
    })
}
