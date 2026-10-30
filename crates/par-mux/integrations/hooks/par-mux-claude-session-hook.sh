#!/bin/sh
# par-mux claude session hook — reports session identity to the par-mux
# daemon on claude SessionStart events (no action argument, or `session`),
# releases the pane's claim on SessionEnd (`release`), and keeps a live
# session registered between those (`prompt` on UserPromptSubmit,
# `notify` on Notification, `stop` on Stop). Same reporter shape as the
# kimi port (core tests/assets/par-mux-agent-state.sh): one JSON line over
# the control socket, one reply read, close. Inert outside a par-mux pane
# (env guards) and silent on every failure.
#
# A claude nested inside another claude's pane — the `claude -p` a
# sub-agent shells out to — inherits this pane's env, so its hooks would
# fire under the PARENT pane's id and hijack the parent's claim (its
# SessionStart re-keys the claim to the child's session; its SessionEnd
# releases the claim while the parent still runs). Every arm therefore
# first walks the process ancestry: two or more claude processes above
# this hook mean a nested claude, which sends nothing (the daemon's
# fresh-start and release-proof guards re-check identity on their side).
# Every report carries the claude session id, so the daemon can verify
# the claim's owner and so a state report re-establishes the resume
# identity after any claim-losing event without waiting for a SessionStart.
# PAR_MUX_INTEGRATION_ID=claude
# PAR_MUX_INTEGRATION_VERSION=4

action="${1:-}"

[ "${PAR_MUX_ENV:-}" = "1" ] || exit 0
[ -n "${PAR_MUX_SOCKET:-}" ] || exit 0
[ -n "${PAR_MUX_PANE_ID:-}" ] || exit 0
command -v python3 >/dev/null 2>&1 || exit 0

PAR_MUX_HOOK_ACTION="$action" python3 -c '
import json
import os
import socket
import sys
import time

try:
    payload = json.load(sys.stdin)
except Exception:
    payload = {}

action = os.environ.get("PAR_MUX_HOOK_ACTION", "")
seq = time.time_ns()


def is_claude_command(command):
    # The same toward-matching bias as the daemon argv matcher
    # (foreground.rs argv_matches): the label may ride argv[0], a path
    # component, or a package path an interpreter executes.
    for token in command.split():
        for component in token.split("/"):
            if component == "claude" or component.startswith(
                ("claude-", "claude_", "claude.")
            ):
                return True
    return False


def nested_claude():
    # Two or more claude processes above this hook mean the claude this
    # hook serves was spawned by another claude (its Bash tool), whose
    # pane claim must not be touched. A top-level claude hook sees
    # exactly one (its own parent). Best-effort: ps missing or unreadable
    # reads as not nested — the hook keeps its primary function and the
    # daemon identity guards re-check on their side.
    try:
        import subprocess

        out = subprocess.run(
            ["ps", "-eo", "pid=,ppid=,command="],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            timeout=2,
        )
        parents = {}
        command_of = {}
        for line in out.stdout.decode("utf-8", "replace").splitlines():
            parts = line.lstrip().split(None, 2)
            if len(parts) < 2:
                continue
            try:
                pid = int(parts[0])
                ppid = int(parts[1])
            except ValueError:
                continue
            parents[pid] = ppid
            command_of[pid] = parts[2] if len(parts) == 3 else ""
    except Exception:
        return False
    pid = os.getpid()
    seen = 0
    for _ in range(64):
        pid = parents.get(pid)
        if pid is None or pid <= 1:
            break
        if is_claude_command(command_of.get(pid, "")):
            seen += 1
            if seen >= 2:
                return True
    return False


if nested_claude():
    raise SystemExit(0)

if action == "release":
    # pane.release_agent clears the pane claim (core aed41c2). The guards
    # are on the daemon side; a later time_ns() always advances past the
    # session report seq from the same source. The session id rides along
    # as proof of ownership: the daemon no-ops a release whose proof does
    # not match the claimed session (a nested claude exit never owns the
    # parent claim).
    params = {
        "pane_id": os.environ["PAR_MUX_PANE_ID"],
        "source": "par-mux:claude",
        "agent": "claude",
        "seq": seq,
    }
    session_id = payload.get("session_id")
    if isinstance(session_id, str) and session_id:
        params["agent_session_id"] = session_id
    request = json.dumps(
        {
            "id": f"par-mux:claude:release:{seq}",
            "method": "pane.release_agent",
            "params": params,
        }
    )
elif action in ("prompt", "notify", "stop"):
    # State reports keep a live session on the roster between its start
    # and end events: a prompt submit is working, the permission prompt
    # is a question, and the 60s-idle notification / a finished turn are
    # idle. (prompt/notify/stop — the installer registers the same-named
    # action under UserPromptSubmit/Notification/Stop.)
    if action == "prompt":
        state = "working"
    elif action == "notify":
        message = payload.get("message")
        state = (
            "question"
            if isinstance(message, str) and "permission" in message.lower()
            else "idle"
        )
    else:
        state = "idle"
    # The session id and transcript path ride every state report: a claim
    # that lost its identity (a pre-proof nested release, a crash) is
    # re-established by the next parent prompt/notify/stop without
    # waiting for a SessionStart.
    params = {
        "pane_id": os.environ["PAR_MUX_PANE_ID"],
        "source": "par-mux:claude",
        "agent": "claude",
        "seq": seq,
        "state": state,
    }
    session_id = payload.get("session_id")
    if isinstance(session_id, str) and session_id:
        params["agent_session_id"] = session_id
    transcript = payload.get("transcript_path")
    if isinstance(transcript, str) and transcript:
        params["agent_session_path"] = transcript
    request = json.dumps(
        {
            "id": f"par-mux:claude:{action}:{seq}",
            "method": "pane.report_agent",
            "params": params,
        }
    )
else:
    session_id = payload.get("session_id")
    if not isinstance(session_id, str) or not session_id:
        raise SystemExit(0)

    source = payload.get("source")
    if not isinstance(source, str) or not source:
        source = "startup"

    params = {
        "pane_id": os.environ["PAR_MUX_PANE_ID"],
        "source": "par-mux:claude",
        "agent": "claude",
        "seq": seq,
        "session_start_source": source,
        "agent_session_id": session_id,
        "session_resume_argv": ["claude", "--resume", session_id],
    }
    transcript = payload.get("transcript_path")
    if isinstance(transcript, str) and transcript:
        params["agent_session_path"] = transcript

    request = json.dumps(
        {"id": f"par-mux:claude:{seq}", "method": "pane.report_agent_session", "params": params}
    )

try:
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
        client.settimeout(0.5)
        client.connect(os.environ["PAR_MUX_SOCKET"])
        client.sendall((request + "\n").encode())
        client.recv(4096)
except Exception:
    pass
' 2>/dev/null || true
