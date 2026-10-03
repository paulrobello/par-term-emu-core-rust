"""%workspaces-changed through the tmux notification API (QA).

The workspace roster cue is argument-less on the wire (the
``%sessions-changed`` convention: clients re-query ``list-workspaces``),
so the Python binding must surface it with ``notification_type ==
"workspaces-changed"`` and every optional target field None.
"""

from par_term_emu_core_rust import Terminal
from par_term_emu_core_rust._native import TmuxNotification


def _drain(line: str) -> list[TmuxNotification]:
    term = Terminal(80, 24)
    term.set_tmux_control_mode(True)
    term.process_str(line)
    return term.drain_tmux_notifications()


def test_workspaces_changed_surfaces_with_no_target_fields() -> None:
    notifications = _drain("%workspaces-changed\n")
    assert len(notifications) == 1
    n = notifications[0]
    assert n.notification_type == "workspaces-changed"
    # Argument-less roster cue: no target fields ride the notification.
    assert n.pane_id is None
    assert n.window_id is None
    assert n.session_id is None
    assert n.name is None
    assert n.raw_line is None


def test_workspaces_changed_alongside_sessions_changed() -> None:
    notifications = _drain("%workspaces-changed\n%sessions-changed\n")
    assert [n.notification_type for n in notifications] == [
        "workspaces-changed",
        "sessions-changed",
    ]


def test_workspaces_changed_inside_a_control_stream() -> None:
    notifications = _drain("%output %1 hello\n%workspaces-changed\n")
    assert [n.notification_type for n in notifications] == [
        "output",
        "workspaces-changed",
    ]
