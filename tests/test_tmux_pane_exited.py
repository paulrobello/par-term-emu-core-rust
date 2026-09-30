"""TmuxNotification.exit_code for par-mux's %pane-exited (QA-190)."""

from par_term_emu_core_rust import Terminal
from par_term_emu_core_rust._native import TmuxNotification


def _drain(line: str) -> list[TmuxNotification]:
    term = Terminal(80, 24)
    term.set_tmux_control_mode(True)
    term.process_str(line)
    return term.drain_tmux_notifications()


def test_pane_exited_carries_exit_code() -> None:
    notifications = _drain("%pane-exited %3 7\n")
    assert len(notifications) == 1
    n = notifications[0]
    assert n.notification_type == "pane-exited"
    assert n.pane_id == "%3"
    assert n.exit_code == 7
    # Deprecated duplicate, kept for one release.
    assert n.name == "7"


def test_pane_exited_without_code_is_none() -> None:
    notifications = _drain("%pane-exited %3\n")
    assert len(notifications) == 1
    n = notifications[0]
    assert n.notification_type == "pane-exited"
    assert n.pane_id == "%3"
    assert n.exit_code is None
    assert n.name is None


def test_other_notification_types_have_no_exit_code() -> None:
    notifications = _drain("%pane-respawned %3\n")
    assert len(notifications) == 1
    n = notifications[0]
    assert n.notification_type == "pane-respawned"
    assert n.pane_id == "%3"
    assert n.exit_code is None
