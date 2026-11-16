#!/bin/sh
# par-mux codex session hook — reports session identity to the par-mux
# daemon on codex SessionStart events, and with codex's claude-wire hook
# events also keeps a live session registered and stateful between those:
# UserPromptSubmit → `prompt` (working), PermissionRequest → `question`,
# Stop → `stop` (idle — the idle-at-prompt capture this integration
# previously lacked: the roster lists only panes WITH state, and a codex
# pane used to fire no state event after SessionStart, card 01a19ec3), and
# releases the pane's claim on SessionEnd (`release`). Same reporter shape
# as the kimi port (core tests/assets/par-mux-agent-state.sh): one JSON
# line over the control socket, one reply read, close. Inert outside a
# par-mux pane (env guards) and silent on every failure.
# PAR_MUX_INTEGRATION_ID=codex
# PAR_MUX_INTEGRATION_VERSION=4

set -eu

action="${1:-}"
hook_input_file="$(mktemp "${TMPDIR:-/tmp}/par-mux-codex-hook.XXXXXX")" || exit 0
trap 'rm -f "$hook_input_file"' EXIT HUP INT TERM
cat >"$hook_input_file" 2>/dev/null || true

case "$action" in
  session|prompt|question|stop|release) ;;
  *) exit 0 ;;
esac

[ "${PAR_MUX_ENV:-}" = "1" ] || exit 0
[ -n "${PAR_MUX_SOCKET:-}" ] || exit 0
[ -n "${PAR_MUX_PANE_ID:-}" ] || exit 0
command -v python3 >/dev/null 2>&1 || exit 0

PAR_MUX_ACTION="$action" PAR_MUX_HOOK_INPUT_FILE="$hook_input_file" python3 - <<'PY'
import json
import os
import random
import socket
import time

action = os.environ.get("PAR_MUX_ACTION", "")
source = "par-mux:codex"
pane_id = os.environ.get("PAR_MUX_PANE_ID")
socket_path = os.environ.get("PAR_MUX_SOCKET")
hook_input_file = os.environ.get("PAR_MUX_HOOK_INPUT_FILE")

if not pane_id or not socket_path:
    raise SystemExit(0)

hook_input = {}
if hook_input_file:
    try:
        with open(hook_input_file, encoding="utf-8") as handle:
            content = handle.read()
        if content.strip():
            hook_input = json.loads(content)
    except Exception:
        hook_input = {}

# The action is ours — the installer registered it against a codex event —
# while hook_event_name is codex's surface, the one that renamed itself out
# from under v2 (card 01a19ec3). The arms therefore dispatch on the action
# and never demand a particular event name (except the session arm, whose
# payload shape is SessionStart's by contract).
session_id = hook_input.get("session_id")
if not isinstance(session_id, str) or not session_id:
    raise SystemExit(0)

# codex exports the pane's thread id into hook processes. When a fork or
# subagent fires an event under a different id, that session is not this
# pane's session — do not overwrite the roster with it.
inherited = os.environ.get("CODEX_THREAD_ID")
if inherited and inherited != session_id:
    raise SystemExit(0)

# codex's payload names the THREAD, and a thread id is not always one
# `codex resume` resolves — measured 2026-10-16 (codex 0.163): a restored
# pane ran `codex resume <payload id>` into "no rollout found for thread
# id" while the transcript's rollout existed under a different id, which
# `codex exec resume` did resolve. The rollout filename carries the
# resumable id (rollout-<timestamp>-<uuid>.jsonl), so when the transcript
# matches that naming and holds a different id, the file wins. The
# corrected id rides EVERY report so the daemon's release-proof check
# compares against the id the session report established.
transcript = hook_input.get("transcript_path")
if isinstance(transcript, str) and transcript:
    name = os.path.basename(transcript)
    if name.startswith("rollout-") and name.endswith(".jsonl"):
        candidate = "-".join(name[: -len(".jsonl")].split("-")[-5:])
        if len(candidate) == 36 and candidate != session_id:
            session_id = candidate

seq = time.time_ns()

if action == "session":
    hook_event_name = hook_input.get("hook_event_name")
    if hook_event_name != "SessionStart":
        raise SystemExit(0)

    session_start_source = hook_input.get("source")
    if not isinstance(session_start_source, str) or not session_start_source:
        session_start_source = "startup"

    params = {
        "pane_id": pane_id,
        "source": source,
        "agent": "codex",
        "seq": seq,
        "session_start_source": session_start_source,
        "agent_session_id": session_id,
        "session_resume_argv": ["codex", "resume", session_id],
    }
    if isinstance(transcript, str) and transcript:
        params["agent_session_path"] = transcript

    # The roster lists only panes WITH state, and codex's Stop fires per
    # turn — a fresh pane idling at the Ready prompt since startup had no
    # state until its first turn completed (card 01a19f8f). Idle rides the
    # same hook run and connection, ordered after the identity report so
    # the state claim carries the session id the daemon just learned.
    idle = {
        "pane_id": pane_id,
        "source": source,
        "agent": "codex",
        "seq": seq + 1,
        "state": "idle",
        "agent_session_id": session_id,
    }
    if isinstance(transcript, str) and transcript:
        idle["agent_session_path"] = transcript

    requests = [
        {
            "id": f"{source}:session:{seq}",
            "method": "pane.report_agent_session",
            "params": params,
        },
        {
            "id": f"{source}:session-idle:{seq}",
            "method": "pane.report_agent",
            "params": idle,
        },
    ]
elif action == "release":
    # pane.release_agent clears the pane claim. The corrected session id
    # rides along as proof of ownership; the daemon no-ops a release whose
    # proof does not match the claimed session (a nested codex exit never
    # owns the parent claim).
    params = {
        "pane_id": pane_id,
        "source": source,
        "agent": "codex",
        "seq": seq,
        "agent_session_id": session_id,
    }
    requests = [
        {
            "id": f"{source}:release:{seq}",
            "method": "pane.release_agent",
            "params": params,
        }
    ]
else:
    # prompt → working (UserPromptSubmit), question → question
    # (PermissionRequest), stop → idle (Stop: the turn finished, codex is
    # back at the prompt). The session id and transcript path ride every
    # state report so a claim that lost its identity is re-established
    # without waiting for a SessionStart.
    state = {"prompt": "working", "question": "question", "stop": "idle"}.get(action)
    if state is None:
        raise SystemExit(0)

    params = {
        "pane_id": pane_id,
        "source": source,
        "agent": "codex",
        "seq": seq,
        "state": state,
        "agent_session_id": session_id,
    }
    if isinstance(transcript, str) and transcript:
        params["agent_session_path"] = transcript

    requests = [
        {
            "id": f"{source}:{action}:{seq}",
            "method": "pane.report_agent",
            "params": params,
        }
    ]

try:
    client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    client.settimeout(0.5)
    client.connect(socket_path)
    for request in requests:
        client.sendall((json.dumps(request) + "\n").encode())
        try:
            client.recv(4096)
        except Exception:
            pass
    client.close()
except Exception:
    pass
PY
