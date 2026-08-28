"""Characterization tests for the streaming Python dict API (ARC-003).

These tests pin the exact dict shape produced by
``decode_server_message`` / ``decode_client_message`` for every message
variant reachable through the public ``encode_*`` string API, plus the
``encode_*`` quirks that shape those dicts (default values, silently
ignored kwargs, silently dropped values).

The dict keys ARE the public Python API of the streaming codec: they are
consumed by tests, examples, and downstream clients, and must not change
when the codec implementation is refactored (e.g. the ARC-003
macro-generated conversion). Run against a streaming build:

    make dev-streaming && uv run pytest tests/test_streaming_dict_api.py -v

Seven ServerMessage variants (zone_*, environment_changed,
remote_host_transition, sub_shell_detected, semantic_snapshot) are not
constructible through ``encode_server_message`` (server-originated only)
and are therefore not covered here; their decode arms are generated from
the same enum fields as the covered variants.
"""

from __future__ import annotations

import re

import pytest

# Streaming is optional; skip cleanly when the module was built without it.
try:
    from par_term_emu_core_rust import (
        StreamingConfig,
        decode_client_message,
        decode_server_message,
        encode_client_message,
        encode_server_message,
    )

    # Classes exist even in non-streaming builds but raise at construction.
    StreamingConfig()
    HAS_STREAMING = True
except (ImportError, RuntimeError, TypeError):
    HAS_STREAMING = False
    pytestmark = pytest.mark.skip(reason="Streaming feature not built")

SERVER_VALID_TYPES = (
    "output, resize, title, bell, pong, connected, error, shutdown, cursor, "
    "refresh, action_notify, action_mark_line, mode_changed, graphics_added, "
    "hyperlink_added, badge_changed, selection_changed, clipboard_sync, "
    "shell_integration, cwd_changed, trigger_matched, user_var_changed, "
    "progress_bar_changed, system_stats, file_transfer_started, "
    "file_transfer_progress, file_transfer_completed, file_transfer_failed, "
    "upload_requested, screen_cleared"
)

CLIENT_VALID_TYPES = (
    "input, resize, ping, refresh, subscribe, snapshot_request, mouse, "
    "focus_change, paste, selection_request, clipboard_request"
)

THEME = {
    "name": "iterm2-dark",
    "background": (10, 11, 12),
    "foreground": (200, 201, 202),
    "normal": [(i, i, i) for i in range(8)],
    "bright": [(i + 1, i + 1, i + 1) for i in range(8)],
}

# (id, kwargs passed to encode_server_message, expected decoded dict).
# `type` is filled in from the id. These pin keys AND values, including the
# encode defaults and the deliberately-preserved quirks.
SERVER_CASES = [
    # output ignores a `timestamp` kwarg (constructor sets None).
    (
        "output",
        {"data": "hello", "timestamp": 999},
        {"data": "hello", "timestamp": None},
    ),
    ("output", {"data": "esc\x1b[31m"}, {"data": "esc\x1b[31m", "timestamp": None}),
    ("resize", {}, {"cols": 80, "rows": 24}),
    ("resize", {"cols": 132, "rows": 43}, {"cols": 132, "rows": 43}),
    ("title", {"title": "My Title"}, {"title": "My Title"}),
    ("title", {}, {"title": ""}),
    ("bell", {}, {}),
    ("pong", {}, {}),
    (
        "connected",
        {
            "cols": 100,
            "rows": 30,
            "initial_screen": "screen\x1b[m",
            "session_id": "sess-1",
            "theme": THEME,
            # QUIRK: encode never reads badge/faint_text_alpha/cwd/
            # modify_other_keys/client_id/readonly kwargs — the Connected
            # constructors hard-code them to None.
            "badge": "main",
            "faint_text_alpha": 0.7,
            "cwd": "/tmp",
            "modify_other_keys": 2,
            "client_id": "c-7",
            "readonly": True,
        },
        {
            "cols": 100,
            "rows": 30,
            "initial_screen": "screen\x1b[m",
            "session_id": "sess-1",
            # theme decode emits only name/background/foreground.
            "theme": {
                "name": "iterm2-dark",
                "background": (10, 11, 12),
                "foreground": (200, 201, 202),
            },
            "badge": None,
            "faint_text_alpha": None,
            "cwd": None,
            "modify_other_keys": None,
            "client_id": None,
            "readonly": None,
        },
    ),
    (
        "connected",
        {"cols": 80, "rows": 24, "session_id": "s2"},
        {
            "cols": 80,
            "rows": 24,
            "initial_screen": None,
            "session_id": "s2",
            "theme": None,
            "badge": None,
            "faint_text_alpha": None,
            "cwd": None,
            "modify_other_keys": None,
            "client_id": None,
            "readonly": None,
        },
    ),
    ("error", {"message": "boom", "code": "E42"}, {"message": "boom", "code": "E42"}),
    ("error", {}, {"message": "Unknown error", "code": None}),
    ("shutdown", {"reason": "bye"}, {"reason": "bye"}),
    ("shutdown", {}, {"reason": "Server shutdown"}),
    (
        "cursor",
        {"col": 3, "row": 4, "visible": False},
        {"col": 3, "row": 4, "visible": False},
    ),
    ("cursor", {}, {"col": 0, "row": 0, "visible": True}),
    (
        "refresh",
        {"cols": 9, "rows": 8, "screen_content": "abc"},
        {"cols": 9, "rows": 8, "screen_content": "abc"},
    ),
    ("refresh", {}, {"cols": 80, "rows": 24, "screen_content": ""}),
    (
        "action_notify",
        {"trigger_id": 5, "title": "T", "message": "M"},
        {"trigger_id": 5, "title": "T", "message": "M"},
    ),
    (
        "action_mark_line",
        {"trigger_id": 6, "row": 7, "label": "L", "color": (1, 2, 3)},
        {"trigger_id": 6, "row": 7, "label": "L", "color": (1, 2, 3)},
    ),
    (
        "mode_changed",
        {"mode": "mouse_tracking", "enabled": True},
        {"mode": "mouse_tracking", "enabled": True},
    ),
    ("graphics_added", {"row": 2, "format": "sixel"}, {"row": 2, "format": "sixel"}),
    ("graphics_added", {"row": 3}, {"row": 3, "format": None}),
    (
        "hyperlink_added",
        {"url": "https://x", "row": 1, "col": 2, "id": "i9"},
        {"url": "https://x", "row": 1, "col": 2, "id": "i9"},
    ),
    ("hyperlink_added", {"url": "u"}, {"url": "u", "row": 0, "col": 0, "id": None}),
    ("badge_changed", {"badge": "b"}, {"badge": "b"}),
    ("badge_changed", {}, {"badge": None}),
    (
        "selection_changed",
        {
            "start_col": 1,
            "start_row": 2,
            "end_col": 3,
            "end_row": 4,
            "text": "sel",
            "mode": "line",
            "cleared": False,
        },
        {
            "start_col": 1,
            "start_row": 2,
            "end_col": 3,
            "end_row": 4,
            "text": "sel",
            "mode": "line",
            "cleared": False,
        },
    ),
    (
        "selection_changed",
        {},
        {
            "start_col": None,
            "start_row": None,
            "end_col": None,
            "end_row": None,
            "text": None,
            "mode": "chars",
            "cleared": False,
        },
    ),
    (
        "clipboard_sync",
        {"operation": "set", "content": "c", "target": "primary"},
        {"operation": "set", "content": "c", "target": "primary"},
    ),
    (
        "shell_integration",
        {
            "event_type": "command_finished",
            "command": "ls",
            "exit_code": 0,
            "timestamp": 12345,
            "cursor_line": 67,
        },
        {
            "event_type": "command_finished",
            "command": "ls",
            "exit_code": 0,
            "timestamp": 12345,
            "cursor_line": 67,
        },
    ),
    # With a timestamp the full constructor runs and optional fields flow.
    (
        "cwd_changed",
        {
            "new_cwd": "/new",
            "old_cwd": "/old",
            "hostname": "h",
            "username": "u",
            "timestamp": 99,
        },
        {
            "old_cwd": "/old",
            "new_cwd": "/new",
            "hostname": "h",
            "username": "u",
            "timestamp": 99,
        },
    ),
    # QUIRK: without a timestamp the plain constructor drops old_cwd /
    # hostname / username even when they were passed.
    (
        "cwd_changed",
        {"new_cwd": "/n2", "old_cwd": "/o2", "hostname": "h2", "username": "u2"},
        {
            "old_cwd": None,
            "new_cwd": "/n2",
            "hostname": None,
            "username": None,
            "timestamp": None,
        },
    ),
    (
        "trigger_matched",
        {
            "trigger_id": 1,
            "row": 2,
            "col": 3,
            "end_col": 5,
            "text": "hit",
            "captures": ["a", "b"],
            "timestamp": 77,
        },
        {
            "trigger_id": 1,
            "row": 2,
            "col": 3,
            "end_col": 5,
            "text": "hit",
            "captures": ["a", "b"],
            "timestamp": 77,
        },
    ),
    (
        "user_var_changed",
        {"name": "n", "value": "v", "old_value": "o"},
        {"name": "n", "value": "v", "old_value": "o"},
    ),
    (
        "user_var_changed",
        {"name": "n", "value": "v"},
        {"name": "n", "value": "v", "old_value": None},
    ),
    (
        "progress_bar_changed",
        {"action": "set", "id": "p1", "state": "error", "percent": 50, "label": "L"},
        {"action": "set", "id": "p1", "state": "error", "percent": 50, "label": "L"},
    ),
    (
        "progress_bar_changed",
        {"id": "p2"},
        {"action": "set", "id": "p2", "state": None, "percent": None, "label": None},
    ),
    # system_stats encode ignores all kwargs and emits the empty variant;
    # decode then omits the cpu/memory/disks/networks/load_average keys.
    (
        "system_stats",
        {"hostname": "ignored"},
        {
            "hostname": None,
            "os_name": None,
            "os_version": None,
            "kernel_version": None,
            "uptime_secs": None,
            "timestamp": None,
        },
    ),
    (
        "file_transfer_started",
        {"id": 9, "direction": "upload", "filename": "f.bin", "total_bytes": 10},
        {"id": 9, "direction": "upload", "filename": "f.bin", "total_bytes": 10},
    ),
    (
        "file_transfer_progress",
        {"id": 9, "bytes_transferred": 4, "total_bytes": 10},
        {"id": 9, "bytes_transferred": 4, "total_bytes": 10},
    ),
    (
        "file_transfer_completed",
        {"id": 9, "filename": "f.bin", "size": 10},
        {"id": 9, "filename": "f.bin", "size": 10},
    ),
    ("file_transfer_failed", {"id": 9, "reason": "io"}, {"id": 9, "reason": "io"}),
    ("file_transfer_failed", {"id": 1}, {"id": 1, "reason": "unknown"}),
    ("upload_requested", {"format": "base64"}, {"format": "base64"}),
    ("upload_requested", {}, {"format": "base64"}),
    ("screen_cleared", {"include_scrollback": True}, {"include_scrollback": True}),
    ("screen_cleared", {}, {"include_scrollback": False}),
]

CLIENT_CASES = [
    ("input", {"data": "ls\r"}, {"data": "ls\r"}),
    ("input", {}, {"data": ""}),
    ("resize", {}, {"cols": 80, "rows": 24}),
    ("resize", {"cols": 20, "rows": 10}, {"cols": 20, "rows": 10}),
    ("ping", {}, {}),
    ("refresh", {}, {}),
    (
        "subscribe",
        {"events": ["output", "bell", "screen_cleared"]},
        {"events": ["output", "bell", "screen_cleared"]},
    ),
    # Unknown event names are silently dropped (filter_map).
    ("subscribe", {"events": ["output", "nope", "cwd"]}, {"events": ["output", "cwd"]}),
    ("subscribe", {}, {"events": []}),
    (
        "snapshot_request",
        {"scope": "recent", "max_commands": 5},
        {"scope": "recent", "max_commands": 5},
    ),
    ("snapshot_request", {}, {"scope": "visible", "max_commands": None}),
    (
        "mouse",
        {
            "col": 1,
            "row": 2,
            "button": 0,
            "shift": True,
            "ctrl": False,
            "alt": True,
            "event_type": "press",
        },
        {
            "col": 1,
            "row": 2,
            "button": 0,
            "shift": True,
            "ctrl": False,
            "alt": True,
            "event_type": "press",
        },
    ),
    (
        "mouse",
        {},
        {
            "col": 0,
            "row": 0,
            "button": 0,
            "shift": False,
            "ctrl": False,
            "alt": False,
            "event_type": "press",
        },
    ),
    ("focus_change", {"focused": False}, {"focused": False}),
    ("focus_change", {}, {"focused": True}),
    ("paste", {"content": "p"}, {"content": "p"}),
    (
        "selection_request",
        {"start_col": 1, "start_row": 2, "end_col": 3, "end_row": 4, "mode": "block"},
        {"start_col": 1, "start_row": 2, "end_col": 3, "end_row": 4, "mode": "block"},
    ),
    (
        "selection_request",
        {},
        {"start_col": 0, "start_row": 0, "end_col": 0, "end_row": 0, "mode": "chars"},
    ),
    (
        "clipboard_request",
        {"operation": "set", "content": "c", "target": "select"},
        {"operation": "set", "content": "c", "target": "select"},
    ),
    (
        "clipboard_request",
        {"operation": "get"},
        {"operation": "get", "content": None, "target": None},
    ),
]


def roundtrip_server(message_type: str, kwargs: dict) -> dict:
    encoded = encode_server_message(message_type, **kwargs)
    return decode_server_message(bytes(encoded))


def roundtrip_client(message_type: str, kwargs: dict) -> dict:
    encoded = encode_client_message(message_type, **kwargs)
    return decode_client_message(bytes(encoded))


@pytest.mark.parametrize(
    ("message_type", "kwargs", "expected_fields"),
    SERVER_CASES,
    ids=[f"server-{t}-{i}" for i, (t, _, _) in enumerate(SERVER_CASES)],
)
def test_server_message_dict_shape(
    message_type: str, kwargs: dict, expected_fields: dict
) -> None:
    result = roundtrip_server(message_type, kwargs)
    assert result == {"type": message_type, **expected_fields}


@pytest.mark.parametrize(
    ("message_type", "kwargs", "expected_fields"),
    CLIENT_CASES,
    ids=[f"client-{t}-{i}" for i, (t, _, _) in enumerate(CLIENT_CASES)],
)
def test_client_message_dict_shape(
    message_type: str, kwargs: dict, expected_fields: dict
) -> None:
    result = roundtrip_client(message_type, kwargs)
    assert result == {"type": message_type, **expected_fields}


def test_connected_generates_uuid_session_id_by_default() -> None:
    result = roundtrip_server("connected", {"cols": 80, "rows": 24})
    session_id = result["session_id"]
    assert isinstance(session_id, str)
    assert re.fullmatch(
        r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", session_id
    )
    # Two encodes produce distinct session ids.
    other = roundtrip_server("connected", {"cols": 80, "rows": 24})
    assert other["session_id"] != session_id


def test_connected_theme_roundtrip_keeps_extra_keys_out() -> None:
    """Decode emits exactly name/background/foreground for the theme."""
    theme = dict(THEME)
    theme["unexpected"] = "dropped-by-encode"
    result = roundtrip_server(
        "connected", {"cols": 1, "rows": 1, "session_id": "s", "theme": theme}
    )
    assert set(result["theme"].keys()) == {"name", "background", "foreground"}


def test_connected_bad_theme_falls_back_to_none() -> None:
    result = roundtrip_server(
        "connected",
        {"cols": 1, "rows": 1, "session_id": "s", "theme": {"name": "partial"}},
    )
    assert result["theme"] is None


def test_encode_ignores_wrong_typed_values() -> None:
    # Wrong-typed kwargs silently fall back to defaults (extract().ok()).
    result = roundtrip_server("resize", {"cols": "not-a-number", "rows": 5})
    assert result == {"type": "resize", "cols": 80, "rows": 5}


def test_encode_server_unknown_type_lists_valid_types() -> None:
    with pytest.raises(RuntimeError, match="Unknown message type: nope"):
        encode_server_message("nope")


def test_encode_client_unknown_type_lists_valid_types() -> None:
    with pytest.raises(RuntimeError, match="Unknown message type: nope"):
        encode_client_message("nope")


def test_decode_garbage_raises() -> None:
    with pytest.raises(RuntimeError, match="Decoding error"):
        decode_server_message(b"\xff\xff\xff\xff")
    with pytest.raises(RuntimeError, match="Decoding error"):
        decode_client_message(b"\xff\xff\xff\xff")
