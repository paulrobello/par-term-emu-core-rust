#!/bin/sh
# installed by par-mux
# managed by par-mux; reinstalling or updating the integration overwrites
# this file. add custom hooks beside this file instead of editing it.
# PAR_MUX_INTEGRATION_ID=muse
# PAR_MUX_INTEGRATION_VERSION=1

set -eu

action="${1:-}"
hook_input_file="$(mktemp "${TMPDIR:-/tmp}/par-mux-muse-hook.XXXXXX")" || exit 0
trap 'rm -f "$hook_input_file"' EXIT HUP INT TERM
cat >"$hook_input_file" 2>/dev/null || true

case "$action" in
  session) ;;
  *) exit 0 ;;
esac

command -v python3 >/dev/null 2>&1 || exit 0

# muse runs hooks with a cleared environment: PAR_MUX_SOCKET and
# PAR_MUX_PANE_ID never arrive (only MUSE_SESSION_ID/MUSE_THREAD_ID
# survive), so unlike the claude/codex/grok reporters this one discovers
# the daemon sockets itself and lets each daemon resolve the pane from the
# payload's cwd.
PAR_MUX_HOOK_INPUT_FILE="$hook_input_file" python3 - <<'PY'
import glob
import json
import os
import random
import socket
import tempfile
import time

source = "par-mux:muse"
hook_input_file = os.environ.get("PAR_MUX_HOOK_INPUT_FILE")

hook_input = {}
if hook_input_file:
    try:
        with open(hook_input_file, encoding="utf-8") as handle:
            content = handle.read()
        if content.strip():
            hook_input = json.loads(content)
    except Exception:
        hook_input = {}


def first_text(*keys):
    for key in keys:
        value = hook_input.get(key)
        if isinstance(value, str) and value:
            return value
    return None


# Only session starts are ours to report; a missing field is allowed for
# forward compatibility.
hook_event_name = first_text("hook_event_name", "hookEventName")
if hook_event_name not in (None, "session_start", "SessionStart", "sessionStart"):
    raise SystemExit(0)

# muse exports the top-level session id into every hook process; the
# payload field is the fallback.
session_id = os.environ.get("MUSE_SESSION_ID") or first_text("session_id", "sessionId")
if not (isinstance(session_id, str) and session_id):
    raise SystemExit(0)

# The cwd is the pane identity surrogate: muse never changes its own
# working directory, and the daemon resolves panes from it.
cwd = first_text("cwd")
if not cwd:
    raise SystemExit(0)

session_start_source = first_text("source")

params = {
    "source": source,
    "agent": "muse",
    "seq": time.time_ns(),
    "agent_session_id": session_id,
    "cwd": cwd,
}
if session_start_source:
    params["session_start_source"] = session_start_source
request = {
    "id": f"{source}:{int(time.time() * 1000)}:{random.randrange(1_000_000):06d}",
    "method": "pane.report_agent_session_cwd",
    "params": params,
}
payload = (json.dumps(request) + "\n").encode()


# The daemon sockets live at default_socket_path's tiers (mux ipc.rs).
# XDG_RUNTIME_DIR is stripped from muse's hook environment, so both tiers
# are reconstructed: /run/user/<uid>, then the per-UID temp directory.
def candidate_dirs():
    uid = os.getuid()
    dirs = []
    runtime = os.environ.get("XDG_RUNTIME_DIR")
    dirs.append(runtime if runtime else f"/run/user/{uid}")
    dirs.append(os.path.join(tempfile.gettempdir(), f"par-mux-{uid}"))
    return dirs


seen = set()
sockets = []
for base in candidate_dirs():
    try:
        found = sorted(glob.glob(os.path.join(base, "par-mux-*.sock")))
    except Exception:
        found = []
    for path in found:
        if path not in seen:
            seen.add(path)
            sockets.append(path)

for socket_path in sockets:
    try:
        client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        client.settimeout(0.5)
        client.connect(socket_path)
        client.sendall(payload)
        try:
            client.recv(4096)
        except Exception:
            pass
        client.close()
    except Exception:
        pass
PY
