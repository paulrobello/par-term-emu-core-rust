#!/bin/sh
# The scrape-tier restart fixture — a pane PROCESS invoked AS an agent (the
# test copies this file to a file named for the agent) whose screen matches
# that agent's bundled blocked rule, and which never reports a hook: no
# session identity exists, so the save carries no agent_session and a
# restart must re-derive the roster label from this invocation. A test
# harness, not a herdr port (the identity-reporting fixture is
# par-mux-fake-agent.sh).
i=0
while [ "$i" -lt 30 ]; do
  echo "waiting for permission"
  i=$((i + 1))
  sleep 2
done
sleep 300
