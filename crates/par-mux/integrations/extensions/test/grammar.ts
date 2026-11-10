// Wire grammar of the daemon's hook-report door, mirrored from the Rust
// authority so a change to either side fails this harness loudly:
//   crates/par-mux/src/mux/hooks/report.rs  (state/session/release handlers)
//   crates/par-mux/src/mux/hooks/mod.rs     (parse_header + value budgets)
// The subjects are the shipped assets
// crates/par-mux/integrations/extensions/{par-mux-agent-state,par-mux-omp-agent-state}.ts;
// scripts/gen_extensions.py renders those bytes and `make extensions-check`
// (a checkall leg) pins shipped == render, so testing the shipped files IS
// testing the template output.
import { expect } from "vitest";

export const STATE_METHOD = "pane.report_agent";
export const SESSION_METHOD = "pane.report_agent_session";
export const RELEASE_METHOD = "pane.release_agent";

/** report.rs: the only state tokens the door accepts; anything else error-replies. */
export const ACCEPTED_STATES = [
  "working",
  "blocked",
  "question",
  "done",
  "idle",
  "unknown",
] as const;
export type AcceptedState = (typeof ACCEPTED_STATES)[number];

export function isAcceptedState(state: unknown): state is AcceptedState {
  return typeof state === "string" && (ACCEPTED_STATES as readonly string[]).includes(state);
}

/** parse_header: the agent label must be non-empty and free of whitespace/control chars. */
export function isValidAgentLabel(agent: unknown): boolean {
  if (typeof agent !== "string" || agent.length === 0) {
    return false;
  }
  return !/[\s\x00-\x1f\x7f]/.test(agent);
}

/** parse_header: seq rides as a JSON integer the daemon reads as u64. */
export function isValidSeq(seq: unknown): boolean {
  return (
    typeof seq === "number" &&
    Number.isInteger(seq) &&
    seq > 0 &&
    seq <= Number.MAX_SAFE_INTEGER
  );
}

/** The transport contract: one JSON object per newline-terminated line. */
export function isSingleJsonLine(raw: string): boolean {
  if (!raw.endsWith("\n")) {
    return false;
  }
  if (raw.indexOf("\n") !== raw.length - 1) {
    return false;
  }
  try {
    JSON.parse(raw);
  } catch {
    return false;
  }
  return true;
}

/** parse_header requirements every report must satisfy. */
export function expectHeader(
  params: Record<string, unknown>,
  expected: { paneId: string; source: string; agent: string },
): void {
  expect(params.pane_id, "pane_id").toBe(expected.paneId);
  expect(params.source, "source").toBe(expected.source);
  expect(isValidAgentLabel(params.agent), `agent label grammar on ${JSON.stringify(params.agent)}`).toBe(true);
  expect(params.agent, "agent").toBe(expected.agent);
  expect(isValidSeq(params.seq), `seq grammar on ${JSON.stringify(params.seq)}`).toBe(true);
}
