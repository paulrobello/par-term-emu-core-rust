//! Window and pane lifecycle notifications.

use crate::tmux_control::TmuxNotification;

pub(super) fn parse_pane_mode_changed(args: &str) -> Option<TmuxNotification> {
    Some(TmuxNotification::PaneModeChanged {
        pane_id: args.trim().to_string(),
    })
}

pub(super) fn parse_window_pane_changed(args: &str) -> Option<TmuxNotification> {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    Some(TmuxNotification::WindowPaneChanged {
        window_id: parts[0].to_string(),
        pane_id: parts[1].to_string(),
    })
}

pub(super) fn parse_window_close(args: &str) -> Option<TmuxNotification> {
    Some(TmuxNotification::WindowClose {
        window_id: args.trim().to_string(),
    })
}

pub(super) fn parse_unlinked_window_close(args: &str) -> Option<TmuxNotification> {
    Some(TmuxNotification::UnlinkedWindowClose {
        window_id: args.trim().to_string(),
    })
}

pub(super) fn parse_window_add(args: &str) -> Option<TmuxNotification> {
    // `window_id [layout visible-layout raw-flags]` — the triple is
    // optional, so a bare-id line (real tmux's shape, and this daemon's
    // pre-triple emission) parses with empty fields rather than failing.
    let mut parts = args.splitn(4, ' ');
    let window_id = parts.next().unwrap_or_default().trim().to_string();
    Some(TmuxNotification::WindowAdd {
        window_id,
        window_layout: parts.next().unwrap_or_default().to_string(),
        window_visible_layout: parts.next().unwrap_or_default().to_string(),
        window_raw_flags: parts.next().unwrap_or_default().to_string(),
    })
}

pub(super) fn parse_unlinked_window_add(args: &str) -> Option<TmuxNotification> {
    Some(TmuxNotification::UnlinkedWindowAdd {
        window_id: args.trim().to_string(),
    })
}

pub(super) fn parse_window_renamed(args: &str) -> Option<TmuxNotification> {
    let parts: Vec<&str> = args.splitn(2, ' ').collect();
    if parts.len() < 2 {
        return None;
    }
    Some(TmuxNotification::WindowRenamed {
        window_id: parts[0].to_string(),
        name: parts[1].to_string(),
    })
}

pub(super) fn parse_unlinked_window_renamed(args: &str) -> Option<TmuxNotification> {
    let parts: Vec<&str> = args.splitn(2, ' ').collect();
    if parts.len() < 2 {
        return None;
    }
    Some(TmuxNotification::UnlinkedWindowRenamed {
        window_id: parts[0].to_string(),
        name: parts[1].to_string(),
    })
}

pub(super) fn parse_layout_change(args: &str) -> Option<TmuxNotification> {
    let parts: Vec<&str> = args.splitn(4, ' ').collect();
    if parts.len() < 4 {
        return None;
    }
    Some(TmuxNotification::LayoutChange {
        window_id: parts[0].to_string(),
        window_layout: parts[1].to_string(),
        window_visible_layout: parts[2].to_string(),
        window_raw_flags: parts[3].to_string(),
    })
}

pub(super) fn parse_pane_title_changed(args: &str) -> Option<TmuxNotification> {
    // The title is the rest of the line — it may contain spaces — and a
    // MISSING second token is the explicit clear (the emitter omits the
    // separator for an empty title rather than trailing-space it).
    let (pane_id, title) = match args.split_once(' ') {
        Some((id, rest)) => (id, rest),
        None => (args, ""),
    };
    if pane_id.is_empty() {
        return None;
    }
    Some(TmuxNotification::PaneTitleChanged {
        pane_id: pane_id.to_string(),
        title: title.to_string(),
    })
}

pub(super) fn parse_pane_exited(args: &str) -> Option<TmuxNotification> {
    // pane_id [exit_code] — a missing code token means the daemon had
    // no exit status to read as it observed the death.
    let (pane_id, code) = match args.split_once(' ') {
        Some((id, rest)) => (id, rest.trim().parse::<i32>().ok()),
        None => (args, None),
    };
    if pane_id.is_empty() {
        return None;
    }
    Some(TmuxNotification::PaneExited {
        pane_id: pane_id.to_string(),
        exit_code: code,
    })
}

pub(super) fn parse_pane_id_only(args: &str) -> Option<TmuxNotification> {
    if args.is_empty() {
        return None;
    }
    Some(TmuxNotification::PaneRespawned {
        pane_id: args.trim().to_string(),
    })
}
