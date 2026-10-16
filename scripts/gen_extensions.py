#!/usr/bin/env python3
"""Generate the pi and omp agent-state extension assets from one template.

The two shipped files under crates/par-mux/integrations/extensions/ were two
hand-maintained forks of one reporter: the transport half (the socket
retry ladder, the send queue, the seq/session-ref plumbing) was byte-shared,
while every fix landed in one fork and had to be back-ported by hand to the
other — a measured 23us seq-swap race was once fixed twice for exactly that
reason (ARC-002). This script is the single source: both assets are rendered
from the sections below, committed, and embedded into the binary with
include_str!. `make extensions-check` (part of checkall) re-renders and
diffs, so the next fix can only land here, never in one fork.

Dialects are knobs, not forks: the shared transport and session-ref
sections are one copy each; the agent name, the resume-argv shape, the
nested-session guard, and the whole default-export wiring are per-dialect.
The rendered files must stay single-file-installable — the installer drops
exactly one asset per agent into that agent's extensions directory
(integrations/extensions.rs), so no shared imported module.

Render with `make gen-extensions` after editing; `--check` fails when a
committed asset is not what the template renders; `--self-test` pins the
invariants the installer and the e2e count on (install markers, no
placeholder residue, deterministic output, dialect separation).
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
ASSET_DIR = REPO / "crates" / "par-mux" / "integrations" / "extensions"
PI_ASSET = ASSET_DIR / "par-mux-agent-state.ts"
OMP_ASSET = ASSET_DIR / "par-mux-omp-agent-state.ts"


def header(agent: str) -> str:
    return f"""\
// installed by par-mux
// managed by par-mux; reinstalling or updating the integration overwrites this file.
// add custom hooks/plugins beside this file instead of editing it.
// PAR_MUX_INTEGRATION_ID={agent}
// PAR_MUX_INTEGRATION_VERSION=1
// env-renamed port of herdr's {agent} agent-state extension (herdr 0.9.1); par-mux's
// hook endpoint accepts its reports verbatim.
// @ts-nocheck
"""


PRELUDE = r"""

import net from "node:net";
import path from "node:path";

const PAR_MUX_ENV = process.env.PAR_MUX_ENV;
const socketPath = process.env.PAR_MUX_SOCKET;
const socketEndpoint =
  process.platform === "win32" && socketPath ? `\\\\.\\pipe\\${socketPath}` : socketPath;
const paneId = process.env.PAR_MUX_PANE_ID;
const source = "par-mux:__AGENT__";
"""

OMP_PRELUDE = """
// OMP marks every shell it spawns with OMPCODE=1. A nested `omp` launched from
// a parent session's shell inherits it, so that process is not the pane's root
// agent and must not report its short-lived session over the parent's.
const nestedOmpSession = process.env.OMPCODE === "1";
"""

ENABLED = r"""

function enabled() {
  return PAR_MUX_ENV === "1" && !!socketPath && !!paneId;
}
"""

OMP_ENABLED = r"""

function enabled() {
  return PAR_MUX_ENV === "1" && !!socketPath && !!paneId && !nestedOmpSession;
}
"""

TRANSPORT = r"""

function sendRequestAttempt(request: unknown, timeoutMs: number): Promise<boolean> {
  if (!enabled()) {
    return Promise.resolve(true);
  }

  return new Promise((resolve) => {
    let done = false;
    let timeout: ReturnType<typeof setTimeout> | undefined;
    const finish = (delivered: boolean) => {
      if (done) return;
      done = true;
      if (timeout) {
        clearTimeout(timeout);
      }
      socket.destroy();
      resolve(delivered);
    };

    const socket = net.createConnection(socketEndpoint!);
    socket.on("error", () => finish(false));
    socket.on("connect", () => socket.write(`${JSON.stringify(request)}\n`));
    socket.on("data", () => finish(true));
    socket.on("end", () => finish(false));
    timeout = setTimeout(() => finish(false), timeoutMs);
    timeout.unref?.();
  });
}

// All sends serialize through one queue: reports carry a per-source
// monotonic seq the daemon orders by ARRIVAL, so two concurrent sends
// (agent_start's session report racing the state report queued by
// session_start) can arrive swapped — the higher seq lands first and the
// state report is then stale-dropped silently, losing the broadcast
// outright (measured on the e2e wire: agent_start's session report
// overtook the state report by 23us on a second connection). Now one
// template, so the queue and its reason live in both dialects.
let requestQueue = Promise.resolve();

async function sendRequestNow(request: unknown): Promise<void> {
  const started = Date.now();
  let attempts = 1;
  let delivered = await sendRequestAttempt(request, 500);
  if (!delivered) {
    attempts = 2;
    delivered = await sendRequestAttempt(request, 1500);
  }
  if (!delivered) {
    attempts = 3;
    // Third, longer rung: under heavy machine load a reply can outlive both
    // budgets and a report abandoned that way drops its broadcast outright
    // (e2e saw 73/74 at load ~130). Duplicate replies are already inert —
    // the daemon drops a seq that does not advance.
    delivered = await sendRequestAttempt(request, 4000);
  }
  sendLogLine(request, delivered, attempts, Date.now() - started);
}

// Under PAR_MUX_SENDLOG=1, one stderr line per finished send: which report
// (by seq), how many rungs the ladder used, whether ANY attempt drew a
// reply, and the wall time. The live-daemon e2e sets this and folds the
// lines into its failure trace — the per-send outcome its notification
// trace cannot name (card 01a0ef2d).
function sendLogLine(request: unknown, delivered: boolean, attempts: number, ms: number): void {
  if (process.env.PAR_MUX_SENDLOG !== "1") {
    return;
  }
  const req = request as { method?: string; params?: { seq?: number } };
  console.error(
    `SENDLOG seq=${req.params?.seq} method=${req.method} delivered=${delivered} attempts=${attempts} ms=${ms}`,
  );
}

function sendRequest(request: unknown): Promise<void> {
  requestQueue = requestQueue.then(
    () => sendRequestNow(request),
    () => sendRequestNow(request),
  );
  return requestQueue;
}
"""

TYPES = r"""

type AgentState = "working" | "blocked" | "idle";

type QueuedState = {
  state: AgentState;
  message?: string;
  seq: number;
};
"""

SEQ_SESSION = r"""

let reportSeq = Date.now() * 1000;
let currentAgentSessionId: string | undefined;
let currentAgentSessionPath: string | undefined;

function nextReportSeq(): number {
  reportSeq += 1;
  return reportSeq;
}

function isAbsoluteSessionPath(file: unknown): file is string {
  return (
    typeof file === "string" &&
    (path.posix.isAbsolute(file) || path.win32.isAbsolute(file))
  );
}

function updateSessionRef(ctx: any): void {
  try {
    const file = ctx?.sessionManager?.getSessionFile?.();
    currentAgentSessionPath = isAbsoluteSessionPath(file) ? file : undefined;
  } catch {
    currentAgentSessionPath = undefined;
  }

  try {
    const id = ctx?.sessionManager?.getSessionId?.();
    currentAgentSessionId = typeof id === "string" && id.length > 0 ? id : undefined;
  } catch {
    currentAgentSessionId = undefined;
  }
}

function withSessionRef(params: Record<string, unknown>): Record<string, unknown> {
  if (currentAgentSessionPath) {
    return { ...params, agent_session_path: currentAgentSessionPath };
  }
  if (currentAgentSessionId) {
    return { ...params, agent_session_id: currentAgentSessionId };
  }
  return params;
}

function currentSessionRef(): Record<string, unknown> | undefined {
  if (currentAgentSessionPath) {
    return { agent_session_path: currentAgentSessionPath };
  }
  if (currentAgentSessionId) {
    return { agent_session_id: currentAgentSessionId };
  }
  return undefined;
}
"""

RESUME_ARGV_PI = r"""
// The agent's own resume invocation, reported so par-mux never needs a
// per-agent table entry for pi: `pi --session <ref>` takes a path or an id,
// and the same preference currentSessionRef applies picks the ref.
function resumeArgv(): string[] | undefined {
  const ref = currentAgentSessionPath ?? currentAgentSessionId;
  if (!ref) {
    return undefined;
  }
  return ["__AGENT__", "--session", ref];
}
"""

RESUME_ARGV_OMP = r"""
// The agent's own resume invocation, reported so par-mux never needs a
// per-agent table entry for omp: omp has no `--session` flag — its resume
// form is the =-joined `omp --resume=<ref>` (path or id, same preference
// currentSessionRef applies).
function resumeArgv(): string[] | undefined {
  const ref = currentAgentSessionPath ?? currentAgentSessionId;
  if (!ref) {
    return undefined;
  }
  return ["__AGENT__", `--resume=${ref}`];
}
"""

REPORTING = r"""

// reportSession takes the source with NO default: pi's session_start passes
// event?.reason verbatim (an absent reason stays absent on the wire), while
// every omp call site passes an explicit value. A silent default here would
// rewrite what the wire carries.
function reportSession(sessionStartSource?: string): Promise<void> {
  const sessionRef = currentSessionRef();
  if (!sessionRef) {
    return Promise.resolve();
  }
  const argv = resumeArgv();

  return sendRequest({
    id: `${source}:session:${Date.now()}:${Math.random().toString(36).slice(2)}`,
    method: "pane.report_agent_session",
    params: {
      pane_id: paneId,
      source,
      agent: "__AGENT__",
      seq: nextReportSeq(),
      session_start_source: sessionStartSource,
      ...sessionRef,
      ...(argv ? { session_resume_argv: argv } : {}),
    },
  });
}

// The agent is gone: release the pane's claim so the roster drops the pane
// and a restart respawns the original command instead of a resume for a
// session with no live agent (pane.release_agent, core aed41c2). Guards are
// the daemon's: label must match the claim and seq must advance past the
// reports above, which nextReportSeq guarantees.
function releaseAgent(): Promise<void> {
  return sendRequest({
    id: `${source}:release:${Date.now()}:${Math.random().toString(36).slice(2)}`,
    method: "pane.release_agent",
    params: {
      pane_id: paneId,
      source,
      agent: "__AGENT__",
      seq: nextReportSeq(),
    },
  });
}

function sendState(state: AgentState, message?: string, seq = nextReportSeq()): Promise<void> {
  return sendRequest({
    id: `${source}:${Date.now()}:${Math.random().toString(36).slice(2)}`,
    method: "pane.report_agent",
    params: withSessionRef({
      pane_id: paneId,
      source,
      agent: "__AGENT__",
      state,
      message,
      seq,
    }),
  });
}

let sendInFlight = false;
let queuedState: QueuedState | undefined;

function queueState(state: AgentState, message?: string): void {
  queuedState = { state, message, seq: nextReportSeq() };
  if (!sendInFlight) {
    void drainStateQueue();
  }
}

async function drainStateQueue(): Promise<void> {
  if (sendInFlight) {
    return;
  }

  sendInFlight = true;
  try {
    while (queuedState) {
      const next = queuedState;
      queuedState = undefined;
      await sendState(next.state, next.message, next.seq);
    }
  } finally {
    sendInFlight = false;
    if (queuedState) {
      void drainStateQueue();
    }
  }
}
"""

OMP_KNOBS = r"""

function parseDurationEnv(name: string, fallback: number): number {
  const raw = process.env[name];
  if (!raw) {
    return fallback;
  }
  const parsed = Number.parseInt(raw, 10);
  if (!Number.isFinite(parsed) || parsed < 0) {
    return fallback;
  }
  return parsed;
}

const idleDebounceMs = parseDurationEnv("PAR_MUX_OMP_IDLE_DEBOUNCE_MS", 250);
const retryGraceMs = parseDurationEnv("PAR_MUX_OMP_RETRY_GRACE_MS", 2500);
const retryableErrorPattern =
  /overloaded|provider.?returned.?error|rate.?limit|too many requests|429|500|502|503|504|service.?unavailable|server.?error|internal.?error|network.?error|connection.?error|connection.?refused|connection.?lost|websocket.?closed|websocket.?error|other side closed|fetch failed|upstream.?connect|reset before headers|socket hang up|ended without|http2 request did not get a response|timed? out|timeout|terminated|retry delay/i;

function lastAssistantMessage(messages: unknown[]): any | undefined {
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const message = messages[i] as any;
    if (message?.role === "assistant") {
      return message;
    }
  }
  return undefined;
}

function retryableErrorMessage(event: any): string | undefined {
  const messages = Array.isArray(event?.messages) ? event.messages : [];
  const assistant = lastAssistantMessage(messages);
  if (assistant?.stopReason !== "error") {
    return undefined;
  }

  const errorMessage = String(assistant.errorMessage ?? "");
  if (!retryableErrorPattern.test(errorMessage)) {
    return undefined;
  }
  return errorMessage || "retryable provider error";
}

function askBlockedMessage(args: any): string {
  const questions = Array.isArray(args?.questions) ? args.questions : [];
  const firstQuestion = questions.find((question: any) => typeof question?.question === "string");
  if (firstQuestion?.question) {
    return firstQuestion.question;
  }
  return "waiting for user input";
}
"""

WIRING_PI = r"""

export default function (pi) {
  if (!enabled()) {
    return;
  }

  let agentActive = false;
  let blockedCount = 0;
  let blockedMessage: string | undefined;
  let lastState: AgentState | undefined;
  let lastMessage: string | undefined;
  let rootSession = false;

  function desiredState() {
    if (blockedCount > 0) {
      return { state: "blocked" as const, message: blockedMessage };
    }
    if (agentActive) {
      return { state: "working" as const, message: undefined };
    }
    return { state: "idle" as const, message: undefined };
  }

  function publishState(force = false) {
    const next = desiredState();
    if (!force && next.state === lastState && next.message === lastMessage) {
      return;
    }
    lastState = next.state;
    lastMessage = next.message;
    queueState(next.state, next.message);
  }

  // The event name is herdr's emitter contract on pi's event bus: renaming it
  // would orphan the only producer. Under par-term alone it never fires.
  pi.events.on("herdr:blocked", (data) => {
    if (!rootSession) {
      return;
    }
    if (!data?.active) {
      blockedCount = Math.max(0, blockedCount - 1);
      if (blockedCount === 0) {
        blockedMessage = undefined;
      }
      publishState();
      return;
    }

    blockedCount += 1;
    blockedMessage = data.label;
    publishState();
  });

  pi.on("session_start", async (event, ctx) => {
    // TUI only: RPC/JSON/print modes are headless (no PTY herdr can display),
    // and RPC still reports hasUI=true, so mode is the reliable gate.
    if (ctx?.mode !== "tui") {
      return;
    }
    rootSession = true;
    updateSessionRef(ctx);
    await reportSession(event?.reason);
    // A reload can replace this extension mid-run without emitting another agent_start.
    agentActive = ctx?.isIdle?.() === false;
    publishState(true);
  });

  pi.on("agent_start", (_event, ctx) => {
    if (!rootSession) {
      return;
    }
    updateSessionRef(ctx);
    void reportSession();
    agentActive = true;
    publishState();
  });

  pi.on("agent_settled", (_event, ctx) => {
    if (!rootSession || ctx?.isIdle?.() !== true) {
      return;
    }

    agentActive = false;
    publishState();
  });

  // Only a real exit releases: reload/new/resume/fork replace the session
  // in the same pane and re-claim through session_start, and releasing
  // those would drop a live pane's claim mid-flight. The runtime awaits
  // shutdown handlers, so the release is delivered before the process
  // exits (bounded by the socket timeouts in sendRequest).
  pi.on("session_shutdown", async (event) => {
    if (!rootSession || event?.reason !== "quit") {
      return;
    }
    await releaseAgent();
  });
}
"""

WIRING_OMP = r"""

export default function (pi) {
  if (!enabled()) {
    return;
  }

  let agentActive = false;
  let retryHoldActive = false;
  let failureBlocked = false;
  let failureMessage: string | undefined;
  let blockedCount = 0;
  let blockedMessage: string | undefined;
  let lastState: AgentState | undefined;
  let lastMessage: string | undefined;
  let idleTimer: ReturnType<typeof setTimeout> | undefined;
  let retryTimer: ReturnType<typeof setTimeout> | undefined;
  let rootSession = false;

  function clearTimer(timer: ReturnType<typeof setTimeout> | undefined) {
    if (timer) {
      clearTimeout(timer);
    }
  }

  function clearPendingTimers() {
    clearTimer(idleTimer);
    clearTimer(retryTimer);
    idleTimer = undefined;
    retryTimer = undefined;
  }

  function clearFailureState() {
    retryHoldActive = false;
    failureBlocked = false;
    failureMessage = undefined;
  }

  function desiredState() {
    if (blockedCount > 0) {
      return { state: "blocked" as const, message: blockedMessage };
    }
    if (failureBlocked) {
      return { state: "blocked" as const, message: failureMessage };
    }
    if (agentActive || retryHoldActive) {
      return { state: "working" as const, message: undefined };
    }
    return { state: "idle" as const, message: undefined };
  }

  function publishState(force = false) {
    const next = desiredState();
    if (!force && next.state === lastState && next.message === lastMessage) {
      return;
    }
    lastState = next.state;
    lastMessage = next.message;
    queueState(next.state, next.message);
  }

  function scheduleIdle() {
    clearPendingTimers();
    clearFailureState();
    idleTimer = setTimeout(() => {
      idleTimer = undefined;
      publishState();
    }, idleDebounceMs);
    idleTimer.unref?.();
  }

  function holdForRetry(message: string) {
    clearPendingTimers();
    retryHoldActive = true;
    failureBlocked = false;
    failureMessage = message;
    publishState();

    retryTimer = setTimeout(() => {
      retryTimer = undefined;
      retryHoldActive = false;
      failureBlocked = true;
      publishState();
    }, retryGraceMs);
    retryTimer.unref?.();
  }

  function activateRootSession(ctx: any, sessionStartSource = "startup"): boolean {
    if (ctx?.hasUI !== true) {
      return false;
    }
    rootSession = true;
    updateSessionRef(ctx);
    void reportSession(sessionStartSource);
    return true;
  }

  function resetSessionState() {
    clearPendingTimers();
    clearFailureState();
    agentActive = false;
    blockedCount = 0;
    blockedMessage = undefined;
  }

  function activateBlocked(message: string | undefined) {
    clearPendingTimers();
    blockedCount += 1;
    blockedMessage = message;
    publishState();
  }

  function deactivateBlocked() {
    blockedCount = Math.max(0, blockedCount - 1);
    if (blockedCount === 0) {
      blockedMessage = undefined;
    }
    publishState();
  }

  // The event name is herdr's emitter contract on pi's event bus: renaming it
  // would orphan the only producer. Under par-term alone it never fires.
  pi.events.on("herdr:blocked", (data) => {
    if (!rootSession) {
      return;
    }
    if (!data?.active) {
      deactivateBlocked();
      return;
    }

    activateBlocked(data.label);
  });

  pi.on("session_start", (_event, ctx) => {
    if (!activateRootSession(ctx)) {
      return;
    }
    // A reload can replace this extension mid-run without emitting another agent_start.
    agentActive = ctx?.isIdle?.() === false;
    publishState(true);
  });

  pi.on("session_switch", (event, ctx) => {
    if (!activateRootSession(ctx, event?.reason || "resume")) {
      return;
    }
    resetSessionState();
    publishState(true);
  });

  pi.on("agent_start", (_event, ctx) => {
    if (!rootSession && !activateRootSession(ctx)) {
      return;
    }
    updateSessionRef(ctx);
    void reportSession("startup");
    clearPendingTimers();
    clearFailureState();
    agentActive = true;
    publishState();
  });

  pi.on("tool_approval_requested", (event, ctx) => {
    if (!rootSession && !activateRootSession(ctx)) {
      return;
    }
    const label = event?.reason || `${event?.toolName || "Tool"} approval`;
    activateBlocked(label);
  });

  pi.on("tool_approval_resolved", (_event, ctx) => {
    if (!rootSession && !activateRootSession(ctx)) {
      return;
    }
    deactivateBlocked();
  });

  pi.on("tool_execution_start", (event, ctx) => {
    if (event?.toolName !== "ask") {
      return;
    }
    if (!rootSession && !activateRootSession(ctx)) {
      return;
    }
    activateBlocked(askBlockedMessage(event.args));
  });

  pi.on("tool_execution_end", (event, ctx) => {
    if (event?.toolName !== "ask") {
      return;
    }
    if (!rootSession && !activateRootSession(ctx)) {
      return;
    }
    deactivateBlocked();
  });

  pi.on("agent_end", (event) => {
    if (!rootSession) {
      return;
    }
    if (!agentActive) {
      // OMP can emit duplicate/late end events while auto-retry is already
      // holding the pane in Working. Do not let an unqualified duplicate end
      // cancel the retry hold and publish a false Idle.
      return;
    }
    if (event?.willContinue === true) {
      // A continuation is already scheduled, so this end is not a settle.
      // Older builds omit the field and fall through as before.
      return;
    }

    agentActive = false;

    const retryableMessage = retryableErrorMessage(event);
    if (retryableMessage) {
      holdForRetry(retryableMessage);
      return;
    }

    scheduleIdle();
  });

  // Only a real exit releases: reload/new/resume/fork replace the session
  // in the same pane and re-claim through session_start, and releasing
  // those would drop a live pane's claim mid-flight. The runtime awaits
  // shutdown handlers, so the release is delivered before the process
  // exits (bounded by the socket timeouts in sendRequest).
  pi.on("session_shutdown", async (event) => {
    if (!rootSession) {
      return;
    }
    clearPendingTimers();
    if (event?.reason === "quit") {
      await releaseAgent();
    }
  });
}
"""

DIALECTS: dict[str, dict[str, str]] = {
    "pi": {
        "prelude_extra": "",
        "enabled": ENABLED,
        "resume_argv": RESUME_ARGV_PI,
        "knobs": "",
        "wiring": WIRING_PI,
    },
    "omp": {
        "prelude_extra": OMP_PRELUDE,
        "enabled": OMP_ENABLED,
        "resume_argv": RESUME_ARGV_OMP,
        "knobs": OMP_KNOBS,
        "wiring": WIRING_OMP,
    },
}


def render(agent: str) -> str:
    dialect = DIALECTS[agent]
    parts = [
        header(agent),
        PRELUDE,
        dialect["prelude_extra"],
        dialect["enabled"],
        TRANSPORT,
        TYPES,
        SEQ_SESSION,
        dialect["resume_argv"],
        REPORTING,
        dialect["knobs"],
        dialect["wiring"],
    ]
    return "".join(parts).replace("__AGENT__", agent)


def self_test(problems: list[str]) -> None:
    pi = render("pi")
    omp = render("omp")
    if "PAR_MUX_INTEGRATION_ID=pi" not in pi:
        problems.append("pi asset lost its install marker")
    if "PAR_MUX_INTEGRATION_ID=omp" not in omp:
        problems.append("omp asset lost its install marker")
    if "OMPCODE" in pi:
        problems.append("pi asset leaked the omp nested-session guard")
    if "OMPCODE" not in omp:
        problems.append("omp asset lost the nested-session guard")
    for name, text in (("pi", pi), ("omp", omp)):
        if "__AGENT__" in text:
            problems.append(f"{name} asset has placeholder residue")
        if text != render("pi" if name == "pi" else "omp"):
            problems.append(f"{name} render is not deterministic")
        if not text.endswith("\n"):
            problems.append(f"{name} asset does not end with a newline")
        if "sessionStartSource?: string" not in text:
            problems.append(
                f"{name} asset lost the explicit-source reportSession signature"
            )
    if pi == omp:
        problems.append("the dialects rendered identical files")


def main() -> int:
    parser = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[0])
    parser.add_argument(
        "--check", action="store_true", help="fail when a committed asset is stale"
    )
    parser.add_argument(
        "--self-test", action="store_true", help="pin renderer invariants"
    )
    args = parser.parse_args()

    if args.self_test:
        problems: list[str] = []
        self_test(problems)
        if problems:
            for problem in problems:
                print(f"self-test: {problem}")
            return 1
        print("gen_extensions self-test ok")
        return 0

    pi = render("pi")
    omp = render("omp")

    if args.check:
        stale = [
            str(p.relative_to(REPO))
            for p, text in ((PI_ASSET, pi), (OMP_ASSET, omp))
            if p.read_text() != text
        ]
        if stale:
            for name in stale:
                print(f"stale: {name} is not what scripts/gen_extensions.py renders")
            print("run: make gen-extensions")
            return 1
        print("extensions: committed assets match the template")
        return 0

    PI_ASSET.write_text(pi)
    OMP_ASSET.write_text(omp)
    print(f"wrote {PI_ASSET.relative_to(REPO)}")
    print(f"wrote {OMP_ASSET.relative_to(REPO)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
