/**
 * Tests for the framework-free TerminalConnection class (QA-005, targeting
 * the QA-008 extraction).
 *
 * The WebSocket global is replaced with a MockWebSocket whose event emission
 * is driven manually by the tests; fake timers drive the reconnect backoff
 * (500ms doubling to a 5s cap) and the 25s heartbeat with 10s stale-pong
 * timeout.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { create, fromBinary, toBinary } from '@bufbuild/protobuf';
import { TerminalConnection } from '@/lib/terminal-connection';
import { encodeClientMessage, createPingMessage, createResizeMessage } from '@/lib/protocol';
import {
  ClientMessageSchema,
  ServerMessageSchema,
  ConnectedSchema,
  ErrorSchema,
  ModeChangedSchema,
  OutputSchema,
  PongSchema,
  RefreshSchema,
  ResizeSchema,
  ShutdownSchema,
  TitleSchema,
  BellSchema,
} from '@/lib/proto/terminal_pb';
import type { ServerMessage } from '@/lib/proto/terminal_pb';

// Schedules mirrored from lib/terminal-connection.ts.
const HEARTBEAT_INTERVAL_MS = 25_000;
const RETRY_DELAY_BASE_MS = 500;
const RETRY_DELAY_MAX_MS = 5_000;

const encoder = new TextEncoder();

// =============================================================================
// Mock WebSocket
// =============================================================================

/** Every socket the connection ever constructed, oldest first. */
const sockets: MockWebSocket[] = [];

class MockWebSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;

  /** When set, the next construction throws this (invalid-URL path). */
  static nextConstructionError: Error | null = null;

  readonly url: string;
  readyState = MockWebSocket.CONNECTING;
  binaryType = 'blob';
  closeCalls = 0;
  sent: ArrayBuffer[] = [];

  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  onmessage: ((event: { data: ArrayBuffer }) => void) | null = null;

  constructor(url: string) {
    this.url = url;
    if (MockWebSocket.nextConstructionError) {
      const err = MockWebSocket.nextConstructionError;
      MockWebSocket.nextConstructionError = null;
      throw err;
    }
    sockets.push(this);
  }

  send(data: ArrayBuffer): void {
    this.sent.push(data);
  }

  close(): void {
    this.closeCalls += 1;
    this.readyState = MockWebSocket.CLOSED;
  }

  // --- test-side event controls (the real socket fires these itself) ---

  simulateOpen(): void {
    this.readyState = MockWebSocket.OPEN;
    this.onopen?.();
  }

  simulateClose(): void {
    this.readyState = MockWebSocket.CLOSED;
    this.onclose?.();
  }

  simulateError(): void {
    this.onerror?.();
  }

  simulateMessage(data: ArrayBuffer): void {
    this.onmessage?.({ data });
  }
}

// =============================================================================
// Helpers
// =============================================================================

// Return type is inferred (not TerminalConnectionCallbacks) so the vi.fn()
// Mock types survive for `.mock` access; the object remains structurally
// assignable to TerminalConnectionCallbacks at the constructor.
function createCallbacks() {
  return {
    onStatus: vi.fn(),
    onRetryingChange: vi.fn(),
    onConnectionClosed: vi.fn(),
    onConnectionError: vi.fn(),
    onInvalidUrl: vi.fn(),
    onOpen: vi.fn(),
    onOutput: vi.fn(),
    onConnected: vi.fn(),
    onServerResize: vi.fn(),
    onRefresh: vi.fn(),
    onTitle: vi.fn(),
    onServerError: vi.fn(),
    onShutdown: vi.fn(),
    onModeChanged: vi.fn(),
    onHyperlinkAdded: vi.fn(),
    onUserVarChanged: vi.fn(),
    onSelectionChanged: vi.fn(),
  };
}

/** Wire-format frame (uncompressed) for a server message oneof case. */
function serverFrame(message: ServerMessage['message']): ArrayBuffer {
  const payload = toBinary(
    ServerMessageSchema,
    create(ServerMessageSchema, { message }),
  );
  const frame = new Uint8Array(payload.length + 1);
  frame[0] = 0x00;
  frame.set(payload, 1);
  return frame.buffer;
}

const outputFrame = (text: string) =>
  serverFrame({ case: 'output', value: create(OutputSchema, { data: encoder.encode(text) }) });
const pongFrame = () => serverFrame({ case: 'pong', value: create(PongSchema, {}) });
const shutdownFrame = (reason: string) =>
  serverFrame({ case: 'shutdown', value: create(ShutdownSchema, { reason }) });

type Callbacks = ReturnType<typeof createCallbacks>;

function makeConnection(url = 'ws://test/socket'): { conn: TerminalConnection; cbs: Callbacks } {
  const cbs = createCallbacks();
  const conn = new TerminalConnection(url, cbs);
  return { conn, cbs };
}

/** connect() and open the first socket. */
function openConnection(): { conn: TerminalConnection; cbs: Callbacks; ws: MockWebSocket } {
  const { conn, cbs } = makeConnection();
  conn.connect();
  const ws = sockets[0];
  ws.simulateOpen();
  return { conn, cbs, ws };
}

beforeEach(() => {
  vi.useFakeTimers({
    toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'Date'],
  });
  vi.stubGlobal('WebSocket', MockWebSocket);
  sockets.length = 0;
  MockWebSocket.nextConstructionError = null;
  vi.spyOn(console, 'log').mockImplementation(() => {});
  vi.spyOn(console, 'error').mockImplementation(() => {});
  vi.spyOn(console, 'warn').mockImplementation(() => {});
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

// =============================================================================
// connect / send basics
// =============================================================================

describe('connect', () => {
  it('constructs the socket with binary framing and reports connecting -> connected', () => {
    const { conn, cbs } = makeConnection('ws://test/terminal');

    expect(conn.isOpen()).toBe(false);
    conn.connect();

    expect(sockets).toHaveLength(1);
    expect(sockets[0].url).toBe('ws://test/terminal');
    expect(sockets[0].binaryType).toBe('arraybuffer');
    expect(cbs.onStatus).toHaveBeenCalledWith('connecting');
    expect(cbs.onOpen).not.toHaveBeenCalled();

    sockets[0].simulateOpen();

    expect(conn.isOpen()).toBe(true);
    expect(cbs.onStatus).toHaveBeenCalledWith('connected');
    expect(cbs.onOpen).toHaveBeenCalledTimes(1);
    expect(cbs.onRetryingChange).toHaveBeenCalledWith(false);
  });

  it('reports an error and skips socket assignment when the URL is rejected', () => {
    MockWebSocket.nextConstructionError = new Error('Invalid URL');
    const { conn, cbs } = makeConnection('not-a-url');

    conn.connect();

    expect(sockets).toHaveLength(0);
    expect(cbs.onStatus).toHaveBeenCalledWith('error');
    expect(cbs.onInvalidUrl).toHaveBeenCalledWith('not-a-url');
    expect(conn.isOpen()).toBe(false);
  });

  it('maps a socket error to onConnectionError and error status', () => {
    const { conn, cbs } = makeConnection();
    conn.connect();

    sockets[0].simulateError();

    expect(cbs.onStatus).toHaveBeenCalledWith('error');
    expect(cbs.onConnectionError).toHaveBeenCalledTimes(1);
  });

  it('drops undecodable frames without disrupting the connection', () => {
    const { cbs } = openConnection();

    sockets[0].simulateMessage(new ArrayBuffer(0)); // decodeServerMessage throws
    sockets[0].simulateMessage(outputFrame('still alive'));

    expect(cbs.onOutput).toHaveBeenCalledTimes(1);
    expect(cbs.onOutput).toHaveBeenCalledWith('still alive');
  });
});

describe('send', () => {
  it('encodes messages onto the open socket only', () => {
    const { conn } = makeConnection();
    conn.connect();

    // Not open yet: silently dropped.
    conn.send(createResizeMessage(80, 24));
    expect(sockets[0].sent).toHaveLength(0);

    sockets[0].simulateOpen();
    conn.send(createResizeMessage(120, 30));

    expect(sockets[0].sent).toHaveLength(1);
    const decoded = fromBinary(
      ClientMessageSchema,
      new Uint8Array(sockets[0].sent[0]).slice(1),
    );
    expect(decoded.message.case).toBe('resize');
    if (decoded.message.case === 'resize') {
      expect(decoded.message.value.cols).toBe(120);
      expect(decoded.message.value.rows).toBe(30);
    }
  });
});

// =============================================================================
// Reconnect backoff
// =============================================================================

describe('reconnect backoff', () => {
  it('doubles from 500ms and caps at 5s between reconnect attempts', () => {
    const { conn, cbs } = makeConnection();
    conn.connect();

    sockets[0].simulateClose();
    expect(cbs.onRetryingChange).toHaveBeenCalledWith(true);
    expect(cbs.onStatus).toHaveBeenCalledWith('disconnected');
    expect(cbs.onConnectionClosed).toHaveBeenCalledTimes(1);

    const expectedDelays = [
      RETRY_DELAY_BASE_MS,
      1000,
      2000,
      4000,
      RETRY_DELAY_MAX_MS,
      RETRY_DELAY_MAX_MS,
    ];
    for (const delay of expectedDelays) {
      const before = sockets.length;
      vi.advanceTimersByTime(delay - 1);
      expect(sockets.length).toBe(before); // not a millisecond early
      vi.advanceTimersByTime(1);
      expect(sockets.length).toBe(before + 1);
      sockets[sockets.length - 1].simulateClose();
    }

    expect(sockets.length).toBe(1 + expectedDelays.length);
  });

  it('resets the backoff to 500ms after a successful connection', () => {
    const { conn } = makeConnection();
    conn.connect();

    sockets[0].simulateClose();
    vi.advanceTimersByTime(500); // first retry (backoff now 1000ms)
    expect(sockets).toHaveLength(2);
    sockets[1].simulateOpen(); // resets the delay

    sockets[1].simulateClose();
    vi.advanceTimersByTime(499);
    expect(sockets).toHaveLength(2); // base delay again, not the doubled one
    vi.advanceTimersByTime(1);
    expect(sockets).toHaveLength(3);
  });

  it('stops reconnecting after cancelRetry()', () => {
    const { conn, cbs } = makeConnection();
    conn.connect();

    sockets[0].simulateClose();
    conn.cancelRetry();
    expect(cbs.onRetryingChange).toHaveBeenCalledWith(false);

    vi.advanceTimersByTime(120_000);
    expect(sockets).toHaveLength(1);
  });
});

// =============================================================================
// Heartbeat
// =============================================================================

describe('heartbeat', () => {
  it('sends a ping every 25s while pongs keep arriving', () => {
    const { ws } = openConnection();

    expect(ws.sent).toHaveLength(0);
    vi.advanceTimersByTime(HEARTBEAT_INTERVAL_MS);
    expect(ws.sent).toHaveLength(1);
    expect(new Uint8Array(ws.sent[0])).toEqual(
      new Uint8Array(encodeClientMessage(createPingMessage())),
    );

    ws.simulateMessage(pongFrame());
    vi.advanceTimersByTime(HEARTBEAT_INTERVAL_MS);
    expect(ws.sent).toHaveLength(2);
    expect(ws.closeCalls).toBe(0);
  });

  it('closes the socket when no pong arrives within the stale window', () => {
    const { ws } = openConnection();

    vi.advanceTimersByTime(HEARTBEAT_INTERVAL_MS); // ping sent, no pong
    expect(ws.sent).toHaveLength(1);

    vi.advanceTimersByTime(HEARTBEAT_INTERVAL_MS); // 50s since open -> stale
    expect(ws.closeCalls).toBe(1);
    expect(ws.readyState).toBe(MockWebSocket.CLOSED);

    // Heartbeat is stopped with the socket: nothing further is sent.
    vi.advanceTimersByTime(120_000);
    expect(ws.sent).toHaveLength(1);
  });
});

// =============================================================================
// Dispatch routing
// =============================================================================

describe('dispatch routing', () => {
  it('routes each decoded message type to its callback', () => {
    const { cbs, ws } = openConnection();

    ws.simulateMessage(outputFrame('hello \x1b[31mworld\x1b[0m'));
    expect(cbs.onOutput).toHaveBeenCalledWith('hello \x1b[31mworld\x1b[0m');

    ws.simulateMessage(
      serverFrame({
        case: 'connected',
        value: create(ConnectedSchema, { cols: 80, rows: 24, sessionId: 'sess-1' }),
      }),
    );
    expect(cbs.onConnected).toHaveBeenCalledTimes(1);
    expect(cbs.onConnected.mock.calls[0][0]).toMatchObject({
      cols: 80,
      rows: 24,
      sessionId: 'sess-1',
    });

    ws.simulateMessage(
      serverFrame({ case: 'resize', value: create(ResizeSchema, { cols: 100, rows: 30 }) }),
    );
    expect(cbs.onServerResize.mock.calls[0][0]).toMatchObject({ cols: 100, rows: 30 });

    ws.simulateMessage(
      serverFrame({
        case: 'refresh',
        value: create(RefreshSchema, { cols: 100, rows: 30, screenContent: encoder.encode('scr') }),
      }),
    );
    expect(cbs.onRefresh).toHaveBeenCalledTimes(1);

    ws.simulateMessage(
      serverFrame({ case: 'title', value: create(TitleSchema, { title: 'my tab' }) }),
    );
    expect(cbs.onTitle.mock.calls[0][0]).toMatchObject({ title: 'my tab' });

    ws.simulateMessage(
      serverFrame({ case: 'error', value: create(ErrorSchema, { message: 'boom' }) }),
    );
    expect(cbs.onServerError).toHaveBeenCalledWith('boom');

    ws.simulateMessage(
      serverFrame({
        case: 'modeChanged',
        value: create(ModeChangedSchema, { mode: 'cursor_visible', enabled: false }),
      }),
    );
    expect(cbs.onModeChanged).toHaveBeenCalledWith('cursor_visible', false);
  });

  it('ignores bell frames without touching any callback', () => {
    const { cbs, ws } = openConnection();

    ws.simulateMessage(serverFrame({ case: 'bell', value: create(BellSchema, {}) }));

    expect(cbs.onOutput).not.toHaveBeenCalled();
    expect(cbs.onStatus).not.toHaveBeenCalledWith('disconnected');
    expect(cbs.onServerError).not.toHaveBeenCalled();
  });
});

// =============================================================================
// Shutdown classification and reconnect policy
// =============================================================================

describe('shutdown classification', () => {
  it.each(['Shell exited with code 0', 'session dead: DEAD SESSION'])(
    'classifies %s as session-ended and stops auto-reconnect',
    (reason) => {
      const { cbs, ws } = openConnection();

      ws.simulateMessage(shutdownFrame(reason));
      expect(cbs.onShutdown).toHaveBeenCalledWith(reason, 'session-ended');

      ws.simulateClose();
      vi.advanceTimersByTime(120_000);
      expect(sockets).toHaveLength(1); // no reconnect attempt
    },
  );

  it('classifies idle timeout as idle-timeout but still reconnects', () => {
    const { cbs } = openConnection();

    sockets[0].simulateMessage(shutdownFrame('Idle timeout reached'));
    expect(cbs.onShutdown).toHaveBeenCalledWith('Idle timeout reached', 'idle-timeout');

    sockets[0].simulateClose();
    vi.advanceTimersByTime(RETRY_DELAY_BASE_MS);
    expect(sockets).toHaveLength(2); // reconnect allowed
  });

  it('classifies other reasons as server shutdown and still reconnects', () => {
    const { cbs } = openConnection();

    sockets[0].simulateMessage(shutdownFrame('server restarting'));
    expect(cbs.onShutdown).toHaveBeenCalledWith('server restarting', 'server');

    sockets[0].simulateClose();
    vi.advanceTimersByTime(RETRY_DELAY_BASE_MS);
    expect(sockets).toHaveLength(2);
  });
});

// =============================================================================
// dispose
// =============================================================================

describe('dispose', () => {
  it('closes the socket exactly once and is idempotent', () => {
    const { conn } = openConnection();

    conn.dispose();
    expect(sockets[0].closeCalls).toBe(1);

    conn.dispose(); // second call must be a no-op
    expect(sockets[0].closeCalls).toBe(1);
  });

  it('stops the heartbeat (no pings after dispose)', () => {
    const { conn, ws } = openConnection();

    conn.dispose();
    vi.advanceTimersByTime(120_000);
    expect(ws.sent).toHaveLength(0);
  });

  it('does not reconnect after a deliberate disconnect, but stays observable', () => {
    const { conn, cbs } = openConnection();

    conn.dispose();
    sockets[0].simulateClose(); // the real close event arriving

    expect(cbs.onStatus).toHaveBeenCalledWith('disconnected');
    expect(cbs.onConnectionClosed).toHaveBeenCalledTimes(1);

    vi.advanceTimersByTime(120_000);
    expect(sockets).toHaveLength(1);
  });

  it('makes later connect() calls a no-op', () => {
    const { conn } = openConnection();

    conn.dispose();
    conn.connect();

    expect(sockets).toHaveLength(1);
  });
});
