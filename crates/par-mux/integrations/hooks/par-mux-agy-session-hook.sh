#!/bin/sh
# installed by par-mux
# managed by par-mux; reinstalling or updating the integration overwrites
# this file. add custom hooks beside this file instead of editing it.
# par-mux agy (Antigravity CLI) session hook. agy hands hooks camelCase JSON
# on stdin (conversationId, transcriptPath, fullyIdle, ...) and reads JSON
# from stdout, so the reporter prints `{}` (no injectSteps, no decision) on
# every path. Actions: session (SessionStart: identity plus the fresh-pane
# idle), working (PreInvocation), idle (Stop, skipped while the payload says
# fullyIdle is false). Same one-JSON-line-over-the-control-socket reporter
# shape as the codex arm. Inert outside a par-mux pane and silent on every
# failure.
# PAR_MUX_INTEGRATION_ID=agy
# PAR_MUX_INTEGRATION_VERSION=1

action="${1:-}"
hook_input_file="$(mktemp "${TMPDIR:-/tmp}/par-mux-agy-hook.XXXXXX")" || { echo '{}'; exit 0; }
trap 'rm -f "$hook_input_file"' EXIT HUP INT TERM
cat >"$hook_input_file" 2>/dev/null || true
echo '{}'

case "$action" in
  session|working|idle) ;;
  *) exit 0 ;;
esac

[ "${PAR_MUX_ENV:-}" = "1" ] || exit 0
[ -n "${PAR_MUX_SOCKET:-}" ] || exit 0
[ -n "${PAR_MUX_PANE_ID:-}" ] || exit 0
command -v python3 >/dev/null 2>&1 || exit 0

PAR_MUX_ACTION="$action" PAR_MUX_HOOK_INPUT_FILE="$hook_input_file" python3 - <<'PY' >/dev/null 2>&1
import json
import os
import socket
import time

action = os.environ.get("PAR_MUX_ACTION", "")
source = "par-mux:agy"
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
if not isinstance(hook_input, dict):
    hook_input = {}

session_id = hook_input.get("conversationId")
if not isinstance(session_id, str) or not session_id:
    raise SystemExit(0)
transcript = hook_input.get("transcriptPath")
seq = time.time_ns()

if action == "idle" and hook_input.get("fullyIdle") is False:
    # Background work still running: the turn ended but the agent is not idle.
    raise SystemExit(0)


def state_params(state, offset):
    params = {
        "pane_id": pane_id,
        "source": source,
        "agent": "agy",
        "seq": seq + offset,
        "state": state,
        "agent_session_id": session_id,
    }
    if isinstance(transcript, str) and transcript:
        params["agent_session_path"] = transcript
    return params


if action == "session":
    identity = {
        "pane_id": pane_id,
        "source": source,
        "agent": "agy",
        "seq": seq,
        "session_start_source": "startup",
        "agent_session_id": session_id,
        "session_resume_argv": ["agy", "--conversation", session_id],
    }
    if isinstance(transcript, str) and transcript:
        identity["agent_session_path"] = transcript
    requests = [
        {"id": f"{source}:session:{seq}", "method": "pane.report_agent_session", "params": identity},
        {"id": f"{source}:session-idle:{seq}", "method": "pane.report_agent", "params": state_params("idle", 1)},
    ]
else:
    requests = [
        {"id": f"{source}:{action}:{seq}", "method": "pane.report_agent", "params": state_params(action, 0)},
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
exit 0
