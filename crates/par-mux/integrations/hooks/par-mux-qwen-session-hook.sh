#!/bin/sh
# installed by par-mux
# managed by par-mux; reinstalling or updating the integration overwrites
# this file. add custom hooks beside this file instead of editing it.
# Ported from herdr's qwen integration (Apache License 2.0): herdr's
# variant shelled out to the herdr CLI; this one speaks the control
# socket directly like the other par-mux reporters.
# PAR_MUX_INTEGRATION_ID=qwen
# PAR_MUX_INTEGRATION_VERSION=1

set -eu

action="${1:-}"
hook_input_file="$(mktemp "${TMPDIR:-/tmp}/par-mux-qwen-hook.XXXXXX")" || exit 0
trap 'rm -f "$hook_input_file"' EXIT HUP INT TERM
cat >"$hook_input_file" 2>/dev/null || true

case "$action" in
  session) ;;
  *) exit 0 ;;
esac

[ "${PAR_MUX_ENV:-}" = "1" ] || exit 0
[ -n "${PAR_MUX_SOCKET:-}" ] || exit 0
[ -n "${PAR_MUX_PANE_ID:-}" ] || exit 0
command -v python3 >/dev/null 2>&1 || exit 0

PAR_MUX_HOOK_INPUT_FILE="$hook_input_file" python3 - <<'PY'
import json
import os
import random
import socket
import time

source = "par-mux:qwen"
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
            parsed = json.loads(content)
            if isinstance(parsed, dict):
                hook_input = parsed
    except Exception:
        hook_input = {}

# The entry is registered under SessionStart only; a payload naming any
# other event is not ours to report. A missing field is allowed.
hook_event_name = hook_input.get("hook_event_name")
if hook_event_name not in (None, "SessionStart"):
    raise SystemExit(0)

session_id = hook_input.get("session_id")
if not isinstance(session_id, str) or not session_id:
    raise SystemExit(0)

params = {
    "pane_id": pane_id,
    "source": source,
    "agent": "qwen",
    "seq": time.time_ns(),
    "agent_session_id": session_id,
}
session_start_source = hook_input.get("source")
if isinstance(session_start_source, str) and session_start_source:
    params["session_start_source"] = session_start_source
request = {
    "id": f"{source}:{int(time.time() * 1000)}:{random.randrange(1_000_000):06d}",
    "method": "pane.report_agent_session",
    "params": params,
}

try:
    client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    client.settimeout(0.5)
    client.connect(socket_path)
    client.sendall((json.dumps(request) + "\n").encode())
    try:
        client.recv(4096)
    except Exception:
        pass
    client.close()
except Exception:
    pass
PY
