// Behavioral tests for the par-mux agent-state mod, run by
// `claude plugin test <mod folder>`: the engine's own host loads the mod
// and each test raises the events a par-mux pane's claude emits. The
// `process.run` op event stands in for the host: recorded, answered clean.

import { expect, mock, test } from 'claude-code/testing'

const PANE_ENV = {
  PAR_MUX_ENV: '1',
  PAR_MUX_SOCKET: '/tmp/par-mux-test.sock',
  PAR_MUX_PANE_ID: '%3',
}

const TASK = { id: 'b1', type: 'shell', status: 'running', description: 'make' }
const CRON = { id: 'c1', schedule: '* * * * *', recurring: true, prompt: 'tick' }

function recordRuns(on: any): { commands: () => string[] } {
  const runs: string[] = []
  on('process.run', (_$: any, e: any) => {
    runs.push((e?.argv ?? []).join(' '))
    return { value: { exitCode: 0, stdout: '', stderr: '', isStdoutTruncated: false, isStderrTruncated: false } }
  })
  return { commands: () => runs }
}

// The chain bottoms: what the engine itself would answer beneath the
// plugin for each event the mod passes down.
function bottoms(on: any): void {
  on('prompt.submit', (_$: any, e: any) => ({ ...e, text: e.text }))
  on('tool.call', () => ({ result: {} }))
  on('classic.Stop', () => ({}))
  on('classic.Notification', () => ({}))
  on('classic.PostToolUse', () => ({}))
}

test('prompt submit reports working', async ($, on) => {
  mock.env(on, { ...PANE_ENV })
  const runs = recordRuns(on)
  bottoms(on)
  const clock = mock.clock(on)
  await $.prompt.submit({ text: 'go', wait: false, origin: { kind: 'composer' } })
  await clock.settle()
  expect(runs.commands()).toEqual(['par-mux --cmd report-state -t %3 -s working'])
})

test('AskUserQuestion reports question, then working once answered', async ($, on) => {
  mock.env(on, { ...PANE_ENV })
  const runs = recordRuns(on)
  bottoms(on)
  const clock = mock.clock(on)
  await $.tool.call({ tool: 'AskUserQuestion', questions: [] })
  await clock.settle()
  expect(runs.commands()).toEqual([
    'par-mux --cmd report-state -t %3 -s question',
    'par-mux --cmd report-state -t %3 -s working',
  ])
})

test('Stop with background tasks parks working with the count', async ($, on) => {
  mock.env(on, { ...PANE_ENV })
  const runs = recordRuns(on)
  bottoms(on)
  const clock = mock.clock(on)
  await $.classic.Stop({ stop_hook_active: false, background_tasks: [TASK], session_crons: [] })
  await clock.settle()
  expect(runs.commands()).toEqual([
    "par-mux --cmd report-state -t %3 -s working -m 'parked: 1 background'",
  ])
})

test('Stop with only a scheduled cron parks too', async ($, on) => {
  mock.env(on, { ...PANE_ENV })
  const runs = recordRuns(on)
  bottoms(on)
  const clock = mock.clock(on)
  await $.classic.Stop({ stop_hook_active: false, background_tasks: [], session_crons: [CRON] })
  await clock.settle()
  expect(runs.commands()).toEqual([
    "par-mux --cmd report-state -t %3 -s working -m 'parked: 1 cron'",
  ])
})

test('Stop with nothing in flight reports idle with no message', async ($, on) => {
  mock.env(on, { ...PANE_ENV })
  const runs = recordRuns(on)
  bottoms(on)
  const clock = mock.clock(on)
  await $.classic.Stop({ stop_hook_active: false, background_tasks: [], session_crons: [] })
  await clock.settle()
  expect(runs.commands()).toEqual(['par-mux --cmd report-state -t %3 -s idle'])
})

test('notification while parked reports plain working, no stale count', async ($, on) => {
  mock.env(on, { ...PANE_ENV })
  const runs = recordRuns(on)
  bottoms(on)
  const clock = mock.clock(on)
  await $.classic.Stop({ stop_hook_active: false, background_tasks: [TASK], session_crons: [] })
  await clock.settle()
  await $.classic.Notification({ message: 'Claude is waiting for your input', notification_type: 'idle' })
  await clock.settle()
  await $.classic.Stop({ stop_hook_active: false, background_tasks: [], session_crons: [] })
  await clock.settle()
  expect(runs.commands()).toEqual([
    "par-mux --cmd report-state -t %3 -s working -m 'parked: 1 background'",
    'par-mux --cmd report-state -t %3 -s working',
    'par-mux --cmd report-state -t %3 -s idle',
  ])
})

test('permission notification reports question', async ($, on) => {
  mock.env(on, { ...PANE_ENV })
  const runs = recordRuns(on)
  bottoms(on)
  const clock = mock.clock(on)
  await $.classic.Notification({ message: 'Claude needs permission to use Bash', notification_type: 'permission' })
  await clock.settle()
  expect(runs.commands()).toEqual(['par-mux --cmd report-state -t %3 -s question'])
})

test('the permission question clears at the tool PostToolUse', async ($, on) => {
  mock.env(on, { ...PANE_ENV })
  const runs = recordRuns(on)
  bottoms(on)
  const clock = mock.clock(on)
  await $.classic.Notification({ message: 'Claude needs permission to use Bash', notification_type: 'permission' })
  await clock.settle()
  await $.classic.PostToolUse({ tool_name: 'Bash', tool_input: {}, tool_response: {}, tool_use_id: 't1' })
  await clock.settle()
  expect(runs.commands()).toEqual([
    'par-mux --cmd report-state -t %3 -s question',
    'par-mux --cmd report-state -t %3 -s working',
  ])
})

test('a denied report-state records lastError and never marks success', async ($, on) => {
  mock.env(on, { ...PANE_ENV })
  const runs: string[] = []
  on('process.run', (_$: any, e: any) => {
    runs.push((e?.argv ?? []).join(' '))
    return { value: { exitCode: 1, stdout: '', stderr: 'operation denied', isStdoutTruncated: false, isStderrTruncated: false } }
  })
  const writes: string[] = []
  on('state.set', (_$: any, e: any) => {
    writes.push(`${e.key}=${JSON.stringify(e.value)}`)
    return { value: { isSet: true, version: 1 } }
  })
  bottoms(on)
  const clock = mock.clock(on)
  await $.classic.Stop({ stop_hook_active: false, background_tasks: [], session_crons: [] })
  await clock.settle()
  expect(runs).toEqual(['par-mux --cmd report-state -t %3 -s idle'])
  expect(writes.some((w) => w.startsWith('lastPushed'))).toBe(false)
  expect(writes.some((w) => w.startsWith('lastError') && w.includes('exited 1'))).toBe(true)
})

test('a top-level claude in a par-mux pane reports, nested sends nothing', async ($, on) => {
  mock.env(on, { ...PANE_ENV })
  const runs = recordRuns(on)
  bottoms(on)
  const clock = mock.clock(on)
  await $.classic.Stop({ stop_hook_active: false, background_tasks: [], session_crons: [] })
  await clock.settle()
  expect(runs.commands()).toEqual(['par-mux --cmd report-state -t %3 -s idle'])
})

test('a nested claude sends nothing', async ($, on) => {
  mock.env(on, { ...PANE_ENV, CLAUDECODE: '1' })
  const runs = recordRuns(on)
  bottoms(on)
  const clock = mock.clock(on)
  await $.classic.Stop({ stop_hook_active: false, background_tasks: [], session_crons: [] })
  await clock.settle()
  expect(runs.commands()).toEqual([])
})

test('outside a par-mux pane sends nothing', async ($, on) => {
  mock.env(on, {})
  const runs = recordRuns(on)
  bottoms(on)
  const clock = mock.clock(on)
  await $.classic.Stop({ stop_hook_active: false, background_tasks: [], session_crons: [] })
  await clock.settle()
  expect(runs.commands()).toEqual([])
})

test('a subagent Stop does not report', async ($, on) => {
  mock.env(on, { ...PANE_ENV })
  const runs = recordRuns(on)
  bottoms(on)
  const clock = mock.clock(on)
  await $.classic.Stop({ stop_hook_active: false, agent_id: 'sa1', background_tasks: [], session_crons: [] })
  await clock.settle()
  expect(runs.commands()).toEqual([])
})
