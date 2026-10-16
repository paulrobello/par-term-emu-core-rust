"""%config-changed through the tmux notification API.

par-mux broadcasts the argument-less cue on every successful
``reload-config`` so attached clients re-apply their ``[client]``
settings. The Python binding surfaces it with ``notification_type ==
"config-changed"`` and every optional target field None.
"""

from par_term_emu_core_rust import Terminal
from par_term_emu_core_rust._native import TmuxNotification


def _drain(line: str) -> list[TmuxNotification]:
    term = Terminal(80, 24)
    term.set_tmux_control_mode(True)
    term.process_str(line)
    return term.drain_tmux_notifications()


def test_config_changed_surfaces_with_no_target_fields() -> None:
    notifications = _drain("%config-changed\n")
    assert len(notifications) == 1
    n = notifications[0]
    assert n.notification_type == "config-changed"
    assert n.pane_id is None
    assert n.window_id is None
    assert n.session_id is None
    assert n.name is None
    assert n.raw_line is None


def test_config_changed_inside_a_control_stream() -> None:
    notifications = _drain("%output %1 hello\n%config-changed\n%sessions-changed\n")
    assert [n.notification_type for n in notifications] == [
        "output",
        "config-changed",
        "sessions-changed",
    ]
