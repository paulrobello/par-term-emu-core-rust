#!/bin/sh
# Task 6.4's fake agent — a pane PROCESS that reports its own session
# identity over the control socket (with a resume invocation pointing back
# at itself) and announces on stdout which mode it started in. A test
# harness, not a herdr port (that proof lives in par-mux-agent-state.sh):
# the daemon under test must respawn this script through the persisted
# resume invocation, and the script's own report is the "the pane's hook
# reports" half of the assertion — it only claims resume when actually
# invoked with --resume.

mode="startup"
id="fx-42"
case "${1:-}" in
  --resume) mode="resume"; id="${2:-$id}" ;;
  ?*) id="$1" ;;
esac
self="$0"

if [ "${PAR_MUX_ENV:-}" = "1" ] && [ -n "${PAR_MUX_SOCKET:-}" ] \
   && [ -n "${PAR_MUX_PANE_ID:-}" ] && command -v python3 >/dev/null 2>&1; then
  python3 -c '
import json, os, socket, sys, time
mode, sid, self = sys.argv[1], sys.argv[2], sys.argv[3]
seq = time.time_ns()
request = json.dumps({
    "id": f"par-mux:fx:{seq}",
    "method": "pane.report_agent_session",
    "params": {
        "pane_id": os.environ["PAR_MUX_PANE_ID"],
        "source": f"par-mux:fx:{mode}",
        "agent": "fx",
        "seq": seq,
        "agent_session_id": sid,
        "session_start_source": mode,
        "session_resume_argv": ["/bin/sh", self, "--resume", sid],
    },
})
try:
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
        client.settimeout(0.5)
        client.connect(os.environ["PAR_MUX_SOCKET"])
        client.sendall((request + "\n").encode())
        client.recv(4096)
except Exception:
    pass
' "$mode" "$id" "$self" 2>/dev/null || true
fi

# Announce repeatedly for a minute — restore re-hangs the saved screen
# after spawn, so a single early print can race the snapshot — then stay
# alive, bounded.
i=0
while [ "$i" -lt 30 ]; do
  echo "FAKE-AGENT $mode $id"
  i=$((i + 1))
  sleep 2
done
sleep 300
