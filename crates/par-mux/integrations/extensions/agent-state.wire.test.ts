// ENH-063: wire-shape harness for the shipped agent-state extensions.
// Loads the two shipped assets verbatim (their bytes ARE the template
// output; `make extensions-check` in checkall pins shipped == render) and
// asserts the requests they put on the wire against the daemon's
// hook-report grammar mirror in ./test/grammar.ts (authority:
// crates/par-mux/src/mux/hooks/report.rs + parse_header in hooks/mod.rs).
// Any token the daemon would reject must fail this harness loudly.

import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import * as grammar from "./test/grammar";
import {
  PANE_ID,
  PANE_SOCKET,
  SESSION_PATH,
  captured,
  clearPaneEnv,
  createPiDouble,
  endpointsSeen,
  flushTransport,
  loadExtension,
  resetTransport,
  sessionCtx,
  setPaneEnv,
  setSocketMode,
} from "./test/host";

// The node:net stand-in: every fresh extension instance binds the fake
// socket layer, so nothing a test loads can reach a real daemon socket.
vi.mock("node:net", async () => {
  const host = await import("./test/host");
  return { default: { createConnection: host.fakeCreateConnection } };
});

beforeEach(() => {
  resetTransport();
  clearPaneEnv();
});

afterEach(() => {
  vi.useRealTimers();
  clearPaneEnv();
});

async function startPi(overrides?: Record<string, string | undefined>) {
  setPaneEnv(overrides);
  vi.resetModules();
  const mod = await loadExtension("pi");
  const agent = createPiDouble();
  mod.default(agent.pi);
  return agent;
}

async function startOmp(overrides?: Record<string, string | undefined>) {
  setPaneEnv(overrides);
  vi.resetModules();
  const mod = await loadExtension("omp");
  const agent = createPiDouble();
  mod.default(agent.pi);
  return agent;
}

describe("shipped assets", () => {
  it("carry their install marker and no template residue", () => {
    for (const [file, marker] of [
      ["par-mux-agent-state.ts", "PAR_MUX_INTEGRATION_ID=pi"],
      ["par-mux-omp-agent-state.ts", "PAR_MUX_INTEGRATION_ID=omp"],
    ] as const) {
      const text = readFileSync(new URL("./" + file, import.meta.url), "utf8");
      expect(text, file + " install marker").toContain(marker);
      expect(text, file + " placeholder residue").not.toContain("__AGENT__");
    }
  });
});

describe("pi dialect (par-mux-agent-state.ts)", () => {
  it("registers nothing and sends nothing when the pane env is absent", async () => {
    const agent = await startPi({
      PAR_MUX_ENV: undefined,
      PAR_MUX_SOCKET: undefined,
      PAR_MUX_PANE_ID: undefined,
    });
    expect(agent.registeredNames()).toEqual([]);
    await agent.emit("session_start", {}, sessionCtx());
    await flushTransport();
    expect(captured()).toEqual([]);
  });

  it("session_start claims the session in the daemon's session-report shape", async () => {
    const agent = await startPi();
    await agent.emit("session_start", {}, sessionCtx());
    await flushTransport();

    const reports = captured();
    // The claim rides first, then the forced initial state report.
    expect(reports).toHaveLength(2);
    const request = reports[0]!.request;
    expect(request.method).toBe(grammar.SESSION_METHOD);
    expect(typeof request.id).toBe("string");
    expect(String(request.id)).toContain("par-mux:pi:");
    grammar.expectHeader(request.params, { paneId: PANE_ID, source: "par-mux:pi", agent: "pi" });
    expect(request.params.agent_session_path).toBe(SESSION_PATH);
    // herdr's identity rule: the path is preferred and the id is dropped.
    expect(request.params).not.toHaveProperty("agent_session_id");
    expect(request.params.session_resume_argv).toEqual(["pi", "--session", SESSION_PATH]);
    // pi passes session_start's reason verbatim: absent stays absent on the wire.
    expect(request.params).not.toHaveProperty("session_start_source");
    const initial = reports[1]!.request;
    expect(initial.method).toBe(grammar.STATE_METHOD);
    expect(initial.params.state).toBe("working");
    expect(initial.params.agent_session_path).toBe(SESSION_PATH);
    expect(
      (initial.params.seq as number) > (request.params.seq as number),
      "initial state seq advances past the claim",
    ).toBe(true);
  });

  it("agent_start re-reports the session (no start source) then reports working", async () => {
    const agent = await startPi();
    await agent.emit("session_start", {}, sessionCtx({ isIdle: () => true }));
    await flushTransport();
    const settled = captured().length;

    await agent.emit("agent_start", {}, sessionCtx());
    await flushTransport();

    const reports = captured();
    expect(reports.length).toBe(settled + 2);
    const [session, state] = reports.slice(settled);
    expect(session!.request.method).toBe(grammar.SESSION_METHOD);
    expect(session!.request.params).not.toHaveProperty("session_start_source");
    expect(session!.request.params.session_resume_argv).toEqual(["pi", "--session", SESSION_PATH]);
    expect(state!.request.method).toBe(grammar.STATE_METHOD);
    expect(state!.request.params.state).toBe("working");
    expect(state!.request.params.agent_session_path).toBe(SESSION_PATH);
    expect(
      (state!.request.params.seq as number) > (session!.request.params.seq as number),
      "state seq advances past the session report",
    ).toBe(true);
  });

  it("agent_settled reports idle inside the daemon's accepted state set", async () => {
    const agent = await startPi();
    await agent.emit("session_start", {}, sessionCtx());
    await flushTransport();
    const settled = captured().length;

    await agent.emit("agent_settled", {}, sessionCtx({ isIdle: () => true }));
    await flushTransport();
    const reports = captured();
    expect(reports.length).toBe(settled + 1);
    expect(reports[settled]!.request.method).toBe(grammar.STATE_METHOD);
    const params = reports[settled]!.request.params;
    expect(params.state).toBe("idle");
    expect(grammar.isAcceptedState(params.state)).toBe(true);
    expect(params).not.toHaveProperty("message");
    expect(params.agent_session_path).toBe(SESSION_PATH);
  });

  it("herdr:blocked reports blocked with the label, then idle again", async () => {
    const agent = await startPi();
    await agent.emit("session_start", {}, sessionCtx({ isIdle: () => true }));
    await flushTransport();
    const settled = captured().length;

    agent.bus("herdr:blocked", { active: true, label: "Fix the flaky test" });
    await flushTransport();
    let reports = captured();
    expect(reports.length).toBe(settled + 1);
    expect(reports[settled]!.request.method).toBe(grammar.STATE_METHOD);
    expect(reports[settled]!.request.params.state).toBe("blocked");
    expect(reports[settled]!.request.params.message).toBe("Fix the flaky test");

    agent.bus("herdr:blocked", { active: false });
    await flushTransport();
    reports = captured();
    expect(reports.length).toBe(settled + 2);
    expect(reports[settled + 1]!.request.params.state).toBe("idle");
    expect(reports[settled + 1]!.request.params).not.toHaveProperty("message");
  });

  it("session_shutdown releases only on quit, with the exact release shape", async () => {
    const agent = await startPi();
    await agent.emit("session_start", {}, sessionCtx());
    await flushTransport();
    const settled = captured().length;

    await agent.emit("session_shutdown", { reason: "reload" });
    await flushTransport();
    expect(captured().length).toBe(settled);

    await agent.emit("session_shutdown", { reason: "quit" });
    await flushTransport();
    const reports = captured();
    expect(reports.length).toBe(settled + 1);
    const { request } = reports[settled]!;
    expect(request.method).toBe(grammar.RELEASE_METHOD);
    grammar.expectHeader(request.params, { paneId: PANE_ID, source: "par-mux:pi", agent: "pi" });
    expect(Object.keys(request.params).sort()).toEqual(["agent", "pane_id", "seq", "source"]);
  });

  it("keeps the wire monotonically ordered under a queued event storm", async () => {
    const agent = await startPi();
    await agent.emit("session_start", {}, sessionCtx());
    await agent.emit("agent_start", {}, sessionCtx());
    agent.bus("herdr:blocked", { active: true, label: "blocked A" });
    agent.bus("herdr:blocked", { active: false });
    await agent.emit("agent_settled", {}, sessionCtx({ isIdle: () => true }));
    await flushTransport();

    const reports = captured();
    expect(reports.length).toBeGreaterThanOrEqual(4);
    const seqs = reports.map((report) => report.request.params.seq as number);
    for (let i = 1; i < seqs.length; i += 1) {
      expect(
        seqs[i]! > seqs[i - 1]!,
        "arrival " + String(i) + " carries seq " + String(seqs[i]) + " after " + String(seqs[i - 1]),
      ).toBe(true);
    }
  });

  it("rides one newline-terminated JSON line per report on the unix socket", async () => {
    const agent = await startPi();
    await agent.emit("session_start", {}, sessionCtx());
    await agent.emit("agent_start", {}, sessionCtx());
    agent.bus("herdr:blocked", { active: true, label: "blocked" });
    await flushTransport();
    const reports = captured();
    expect(reports).toHaveLength(4);
    for (const { raw } of reports) {
      expect(grammar.isSingleJsonLine(raw), JSON.stringify(raw)).toBe(true);
    }
    for (const endpoint of endpointsSeen()) {
      expect(endpoint, "unix socket endpoint").toBe(PANE_SOCKET);
    }
  });

  it("exhausts the three-rung send ladder when the socket refuses", async () => {
    setSocketMode("refuse");
    const errSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    let lines: string[] = [];
    try {
      const agent = await startPi({ PAR_MUX_SENDLOG: "1" });
      await agent.emit("session_start", {}, sessionCtx());
      await flushTransport();
      lines = errSpy.mock.calls.map((call) => String(call[0]));
    } finally {
      errSpy.mockRestore();
      setSocketMode("deliver");
    }
    expect(captured()).toEqual([]);
    const line = lines.find((text) => text.includes("SENDLOG"));
    expect(line, "one SENDLOG line for the abandoned session report").toBeTruthy();
    expect(line).toMatch(/delivered=false attempts=3/);
    expect(line).toMatch(/method=pane\.report_agent_session/);
  });
});

describe("omp dialect (par-mux-omp-agent-state.ts)", () => {
  it("a nested OMPCODE session never enables or reports", async () => {
    const agent = await startOmp({ OMPCODE: "1" });
    expect(agent.registeredNames()).toEqual([]);
    await agent.emit("session_start", {}, sessionCtx());
    await flushTransport();
    expect(captured()).toEqual([]);
  });

  it("session_start claims only with hasUI, in the omp wire shape", async () => {
    const agent = await startOmp();
    await agent.emit("session_start", {}, sessionCtx({ hasUI: undefined }));
    await flushTransport();
    expect(captured()).toEqual([]);

    await agent.emit("session_start", {}, sessionCtx());
    await flushTransport();
    const [report] = captured();
    expect(report!.request.method).toBe(grammar.SESSION_METHOD);
    grammar.expectHeader(report!.request.params, { paneId: PANE_ID, source: "par-mux:omp", agent: "omp" });
    expect(report!.request.params.agent_session_path).toBe(SESSION_PATH);
    expect(report!.request.params).not.toHaveProperty("agent_session_id");
    expect(report!.request.params.session_start_source).toBe("startup");
    expect(report!.request.params.session_resume_argv).toEqual(["omp", "--resume=" + SESSION_PATH]);
  });

  it("session_switch reports the resume provenance token", async () => {
    const agent = await startOmp();
    await agent.emit("session_start", {}, sessionCtx());
    await flushTransport();
    const settled = captured().length;

    await agent.emit("session_switch", { reason: "resume" }, sessionCtx());
    await flushTransport();
    const reports = captured();
    expect(reports.length).toBe(settled + 2);
    const [session, state] = reports.slice(settled);
    expect(session!.request.method).toBe(grammar.SESSION_METHOD);
    expect(session!.request.params.session_start_source).toBe("resume");
    expect(session!.request.params.session_resume_argv).toEqual(["omp", "--resume=" + SESSION_PATH]);
    expect(state!.request.method).toBe(grammar.STATE_METHOD);
    expect(state!.request.params.state).toBe("idle");
  });

  it("a retryable agent_end holds the pane working, then blocks with the message", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"] });
    const agent = await startOmp();
    await agent.emit("session_start", {}, sessionCtx());
    await flushTransport();
    const settled = captured().length;

    await agent.emit("agent_end", {
      messages: [
        { role: "assistant", stopReason: "error", errorMessage: "provider returned error: 503 service unavailable" },
      ],
    });
    await flushTransport();
    // The hold keeps the pane working without a duplicate working report.
    expect(captured().length).toBe(settled);

    vi.advanceTimersByTime(2500);
    await flushTransport();
    const reports = captured();
    expect(reports.length).toBe(settled + 1);
    expect(reports[settled]!.request.method).toBe(grammar.STATE_METHOD);
    const params = reports[settled]!.request.params;
    expect(params.state).toBe("blocked");
    expect(params.message).toBe("provider returned error: 503 service unavailable");
    expect(grammar.isAcceptedState(params.state)).toBe(true);
  });

  it("a plain agent_end settles to idle after the debounce", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"] });
    const agent = await startOmp();
    await agent.emit("session_start", {}, sessionCtx());
    await flushTransport();
    const settled = captured().length;

    await agent.emit("agent_end", { messages: [{ role: "assistant", stopReason: "stop" }] });
    await flushTransport();
    // The idle debounce holds the wire until it fires.
    expect(captured().length).toBe(settled);

    vi.advanceTimersByTime(250);
    await flushTransport();
    const reports = captured();
    expect(reports.length).toBe(settled + 1);
    const params = reports[settled]!.request.params;
    expect(params.state).toBe("idle");
    expect(params).not.toHaveProperty("message");
  });

  it("session_shutdown releases only on quit", async () => {
    const agent = await startOmp();
    await agent.emit("session_start", {}, sessionCtx());
    await flushTransport();
    const settled = captured().length;

    await agent.emit("session_shutdown", { reason: "exit" });
    await flushTransport();
    expect(captured().length).toBe(settled);

    await agent.emit("session_shutdown", { reason: "quit" });
    await flushTransport();
    const reports = captured();
    expect(reports.length).toBe(settled + 1);
    const { request } = reports[settled]!;
    expect(request.method).toBe(grammar.RELEASE_METHOD);
    grammar.expectHeader(request.params, { paneId: PANE_ID, source: "par-mux:omp", agent: "omp" });
    expect(Object.keys(request.params).sort()).toEqual(["agent", "pane_id", "seq", "source"]);
  });
});
