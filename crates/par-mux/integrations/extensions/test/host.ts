// Test host for the shipped agent-state extensions: a fake node:net layer
// that records every wire payload, plus the pane env and a pi-shaped event
// bus the default export registers against.
//
// The transport state lives on globalThis (keyed by Symbol.for) so a
// vi.mock factory and the test file see one capture buffer even when
// vi.resetModules() re-evaluates this module.

import { EventEmitter } from "node:events";

export type CapturedRequest = {
  /** The exact bytes socket.write received, newline included. */
  raw: string;
  /** The parsed wire request. */
  request: { id: unknown; method: string; params: Record<string, unknown> };
};

type SocketMode = "deliver" | "refuse";

class FakeSocket extends EventEmitter {
  writes: string[] = [];
  destroyed = false;
  write(data: string): void {
    this.writes.push(data);
  }
  destroy(): void {
    this.destroyed = true;
  }
}

type TransportState = {
  sockets: FakeSocket[];
  endpoints: unknown[];
  mode: SocketMode;
};

const TRANSPORT_KEY = Symbol.for("par-mux.enh063.transport");
type WithTransport = typeof globalThis & { [TRANSPORT_KEY]?: TransportState };
const g = globalThis as WithTransport;
if (!g[TRANSPORT_KEY]) {
  g[TRANSPORT_KEY] = { sockets: [], endpoints: [], mode: "deliver" };
}
const transport = g[TRANSPORT_KEY]!;

export function setSocketMode(next: SocketMode): void {
  transport.mode = next;
}

export function resetTransport(): void {
  transport.sockets.length = 0;
  transport.endpoints.length = 0;
  transport.mode = "deliver";
}

export function captured(): CapturedRequest[] {
  return transport.sockets
    .filter((socket) => socket.writes.length > 0)
    .map((socket) => {
      const raw = socket.writes[0]!;
      const request = JSON.parse(raw) as CapturedRequest["request"];
      return { raw, request };
    });
}

export function endpointsSeen(): unknown[] {
  return [...transport.endpoints];
}

/**
 * Wait until every opened socket finished (the extension calls destroy() in
 * both the deliver and refuse paths) and no new connection opens for one
 * more macrotask tick, so a serializing queue cannot straddle the flush.
 */
export async function flushTransport(): Promise<void> {
  for (let guard = 0; guard < 100; guard += 1) {
    await new Promise((resolve) => setImmediate(resolve));
    const allDone =
      transport.sockets.length > 0 && transport.sockets.every((s) => s.destroyed);
    if (allDone) {
      const before = transport.sockets.length;
      await new Promise((resolve) => setImmediate(resolve));
      if (
        transport.sockets.length === before &&
        transport.sockets.every((s) => s.destroyed)
      ) {
        return;
      }
    }
  }
}

/** The node:net stand-in handed to the extension through vi.mock. */
export function fakeCreateConnection(endpoint: unknown): FakeSocket {
  const socket = new FakeSocket();
  transport.sockets.push(socket);
  transport.endpoints.push(endpoint);
  queueMicrotask(() => {
    if (transport.mode === "refuse") {
      socket.emit("error", new Error("ECONNREFUSED (test host)"));
      return;
    }
    socket.emit("connect");
    socket.emit("data", Buffer.from("{}"));
  });
  return socket;
}

export type PiDouble = ReturnType<typeof createPiDouble>;

export function createPiDouble() {
  const handlers = new Map<string, Array<(event?: unknown, ctx?: unknown) => unknown>>();
  const busHandlers = new Map<string, Array<(data?: unknown) => unknown>>();
  const pi = {
    on(name: string, handler: (event?: unknown, ctx?: unknown) => unknown): void {
      const list = handlers.get(name) ?? [];
      list.push(handler);
      handlers.set(name, list);
    },
    events: {
      on(name: string, handler: (data?: unknown) => unknown): void {
        const list = busHandlers.get(name) ?? [];
        list.push(handler);
        busHandlers.set(name, list);
      },
    },
  };
  return {
    pi,
    /** Fire a pi.on handler chain (session_start, agent_start, ...). */
    async emit(name: string, event?: unknown, ctx?: unknown): Promise<void> {
      for (const handler of handlers.get(name) ?? []) {
        await handler(event, ctx);
      }
    },
    /** Fire the pi.events bus (herdr:blocked). */
    bus(name: string, data?: unknown): void {
      for (const handler of busHandlers.get(name) ?? []) {
        handler(data);
      }
    },
    registeredNames(): string[] {
      return [...handlers.keys(), ...busHandlers.keys()];
    },
  };
}

export type ExtensionModule = { default: (pi: unknown) => void };

/** Load a shipped extension module fresh (call after vi.resetModules()). */
export async function loadExtension(dialect: "pi" | "omp"): Promise<ExtensionModule> {
  if (dialect === "pi") {
    return await import("../par-mux-agent-state.ts");
  }
  return await import("../par-mux-omp-agent-state.ts");
}

const ENV_KEYS = [
  "PAR_MUX_ENV",
  "PAR_MUX_SOCKET",
  "PAR_MUX_PANE_ID",
  "PAR_MUX_SENDLOG",
  "OMPCODE",
] as const;

export const PANE_ID = "%3";
export const PANE_SOCKET = "/tmp/par-mux-test.sock";
export const SESSION_PATH = "/tmp/pi-sessions/main.jsonl";

/** Install the pane env the extension reads at import time. */
export function setPaneEnv(overrides: Record<string, string | undefined> = {}): void {
  process.env.PAR_MUX_ENV = "1";
  process.env.PAR_MUX_SOCKET = PANE_SOCKET;
  process.env.PAR_MUX_PANE_ID = PANE_ID;
  for (const [key, value] of Object.entries(overrides)) {
    if (value === undefined) {
      delete process.env[key];
    } else {
      process.env[key] = value;
    }
  }
}

export function clearPaneEnv(): void {
  for (const key of ENV_KEYS) {
    delete process.env[key];
  }
}

/** The session-manager double the extensions read session refs from. */
export function sessionCtx(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    mode: "tui",
    hasUI: true,
    isIdle: () => false,
    sessionManager: {
      getSessionFile: () => SESSION_PATH,
      getSessionId: () => "sess-123",
    },
    ...overrides,
  };
}
