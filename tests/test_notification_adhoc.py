"""%notification (par-mux ``show-notification``) through the tmux notification API.

The daemon broadcasts ``%notification <b64 title> [body=<b64 body>]``. The
Python binding surfaces it with ``notification_type == "notification"``,
the decoded title in ``name`` and the decoded body in ``message`` (None
when the line carried no body).
"""

import base64

from par_term_emu_core_rust import Terminal
from par_term_emu_core_rust._native import TmuxNotification


def _drain(line: str) -> list[TmuxNotification]:
    term = Terminal(80, 24)
    term.set_tmux_control_mode(True)
    term.process_str(line)
    return term.drain_tmux_notifications()


def _b64(text: str) -> str:
    return base64.b64encode(text.encode()).decode()


def test_notification_decodes_title_and_body() -> None:
    notifications = _drain(
        f"%notification {_b64('build failed')} body={_b64('see the log')}\n"
    )
    assert len(notifications) == 1
    n = notifications[0]
    assert n.notification_type == "notification"
    assert n.name == "build failed"
    assert n.message == "see the log"
    assert n.pane_id is None


def test_notification_without_a_body() -> None:
    notifications = _drain(f"%notification {_b64('done')}\n")
    assert len(notifications) == 1
    assert notifications[0].name == "done"
    assert notifications[0].message is None
