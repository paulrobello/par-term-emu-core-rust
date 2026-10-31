// installed by par-mux
// PAR_MUX_INTEGRATION_ID=agent-state
// PAR_MUX_INTEGRATION_VERSION=2
//
// par-mux claude agent-state mod: reports the pane's live agent state to the
// hosting daemon so the roster shows background-work truth. The classic
// session hook (par-mux-claude-session-hook.sh) owns session identity and
// release; this mod adds what that reporter cannot see:
//
// - prompt submit -> working
// - an AskUserQuestion tool call -> question, then working the moment the
//   answer returns (in finally, so a failed chain still clears the state)
// - a permission Notification -> question, cleared by the tool's own
//   PostToolUse (the user answered; the tool ran) — not left stale until
//   the turn ends
// - Stop -> working with a parked count while background_tasks or
//   session_crons will wake the session later, else idle. The classic Stop
//   chain runs FIRST (await next before the report), preserving the settings
//   hooks' authority ordering; this report lands after theirs.
// - Notification while parked -> plain working (a drain is starting or the
//   parked wait continues); the fresh count lands at the next Stop, so no
//   stale count is ever repeated.
//
// Terminal-path reports (Stop, Notification, PostToolUse, the question's
// clear) are awaited inside the hook so they cannot outlive it; the
// fire-and-forget reports are only the two a turn immediately follows
// (prompt submit, question up), and even those ride the same ordered queue.
//
// Delivery is the control CLI, one bounded argv per report (no shell):
//   par-mux --cmd "report-state -t <pane> -s <state> [-m '<message>']"
// with PAR_MUX_SOCKET deciding the daemon (the in-pane socket is the full
// control socket by design), the same report-state verb the .sh reporter's
// pane.report_agent reports ride on the wire. A non-zero exit (denied
// process, daemon gone) is recorded as lastError, never as success.
// Inert outside a par-mux pane and silent on every failure. The nested
// guard reads CLAUDECODE, which a claude spawned by another claude's Bash
// tool inherits and a top-level claude does not (measured 2026-10-30:
// `ps -Eww` on live top-level sessions shows PAR_MUX_* but no CLAUDECODE),
// so a nested claude never hijacks the parent pane's claim.

import type { EngineInterface, Register } from 'claude-code'

type AgentState = 'working' | 'question' | 'idle'

const RUN_TIMEOUT_MS = 3000

const LAST_PUSHED = { plugin: 'agent-state', key: 'lastPushed' } as const
const LAST_ERROR = { plugin: 'agent-state', key: 'lastError' } as const

// Serialized report queue: reports keep their emission order even when two
// fire around one `next`.
let queue: Promise<void> = Promise.resolve()

// The message the last parked Stop reported, cleared by any event that
// supersedes it (a prompt, an answer, an unparked Stop).
let parkedMessage: string | null = null

// A question is on screen (AskUserQuestion or a permission ask), cleared by
// its own resolution event so the roster never holds a stale question.
let questionUp = false

function shquote(text: string): string {
  return `'${text.replaceAll("'", `'\\''`)}'`
}

async function readIdentity($: EngineInterface): Promise<{ pane: string } | null> {
  const [flag, socket, pane] = await Promise.all([
    $.env.get('PAR_MUX_ENV'),
    $.env.get('PAR_MUX_SOCKET'),
    $.env.get('PAR_MUX_PANE_ID'),
  ])
  return flag === '1' && !!socket && !!pane ? { pane } : null
}

async function readNested($: EngineInterface): Promise<boolean> {
  return (await $.env.get('CLAUDECODE')) === '1'
}

function fireReport($: EngineInterface, state: AgentState, message?: string): Promise<void> {
  const run = () => sendReport($, state, message)
  queue = queue.then(run, run)
  return queue
}

async function sendReport($: EngineInterface, state: AgentState, message?: string): Promise<void> {
  try {
    const id = await readIdentity($)
    if (!id || (await readNested($))) {
      return
    }
    const cmd = ['report-state', '-t', id.pane, '-s', state]
    if (message) {
      cmd.push('-m', shquote(message))
    }
    const ran = await $.process.run(['par-mux', '--cmd', cmd.join(' ')], {
      timeoutMs: RUN_TIMEOUT_MS,
    })
    if (ran.exitCode !== 0) {
      await $.state
        .set(
          LAST_ERROR,
          `par-mux report-state exited ${ran.exitCode}: ${ran.stderr.slice(0, 200)}`,
        )
        .catch(() => {})
      return
    }
    await $.state.set(LAST_PUSHED, { state, message: message ?? '' })
    await $.state.set(LAST_ERROR, null)
  } catch (err) {
    await $.state
      .set(LAST_ERROR, err instanceof Error ? err.message : String(err))
      .catch(() => {})
  }
}

function noteParked(tasks: readonly unknown[], crons: readonly unknown[]): string | null {
  const parts: string[] = []
  if (tasks.length > 0) {
    parts.push(`${tasks.length} background`)
  }
  if (crons.length > 0) {
    parts.push(`${crons.length} cron${crons.length === 1 ? '' : 's'}`)
  }
  return parts.length > 0 ? `parked: ${parts.join(', ')}` : null
}

export const register: Register = on => {
  // Prompt submitted: the turn is starting; any park or question is over.
  on('prompt.submit', ($, e, next) => {
    parkedMessage = null
    questionUp = false
    void fireReport($, 'working')
    return next(e)
  }).catch(($, e, next) => next(e))

  // The question dialog: question while it is up, working the moment the
  // answer returns. A subagent's question (agentId present) is not the
  // pane's to show.
  on('tool.call', { tool: 'AskUserQuestion' }, async ($, e, next) => {
    if (e.agentId) {
      return next(e)
    }
    parkedMessage = null
    questionUp = true
    void fireReport($, 'question')
    try {
      return await next(e)
    } finally {
      questionUp = false
      await fireReport($, 'working')
    }
  }).catch(($, e, next) => next(e))

  // Turn ended. The classic chain (settings hooks' own Stop entries)
  // speaks first; this awaited report then settles the truth they cannot
  // see, and lands before the session can move on.
  on('classic.Stop', async ($, e, next) => {
    const result = await next(e)
    if (!e.agent_id) {
      parkedMessage = noteParked(e.background_tasks ?? [], e.session_crons ?? [])
      questionUp = false
      await fireReport($, parkedMessage ? 'working' : 'idle', parkedMessage ?? undefined)
    }
    return result
  }).catch(($, e, next) => next(e))

  // Notifications: permission asks are questions. While parked, report
  // plain working — a drain is starting or the parked wait continues — and
  // let the next Stop land the fresh count instead of repeating a stale one.
  on('classic.Notification', async ($, e, next) => {
    const result = await next(e)
    if (!e.agent_id) {
      if (/permission/i.test(e.message)) {
        parkedMessage = null
        questionUp = true
        await fireReport($, 'question')
      } else if (parkedMessage) {
        await fireReport($, 'working')
      }
    }
    return result
  }).catch(($, e, next) => next(e))

  // The permission question resolved: the tool ran, so the answer is in.
  // This is what clears a permission question immediately instead of at
  // turn end.
  on('classic.PostToolUse', async ($, e, next) => {
    const result = await next(e)
    if (!e.agent_id && questionUp) {
      questionUp = false
      await fireReport($, 'working')
    }
    return result
  }).catch(($, e, next) => next(e))
}
