/**
 * Framework-free terminal WebSocket connection (QA-008).
 *
 * Owns everything connection-shaped that used to live inside
 * components/Terminal.tsx: the WebSocket itself, reconnect/backoff state,
 * the heartbeat timer with stale-pong detection, and binary message
 * decode + dispatch. The component registers UI reactions as callbacks
 * and keeps only xterm wiring and React state.
 *
 * `dispose()` is idempotent, so it is safe under React StrictMode's
 * double-invoked effects.
 */

import type { ConnectionStatus } from '@/types/terminal';
import {
  decodeServerMessage,
  encodeClientMessage,
  createPingMessage,
} from '@/lib/protocol';
import type { ClientMessage, ServerMessage } from '@/lib/protocol';

/** Value carried by a given oneof case of `ServerMessage`. */
export type ServerCase<K extends ServerMessage['message']['case']> =
  Extract<ServerMessage['message'], { case: K }>['value'];

/** How a shutdown reason should be presented; also drives reconnect policy. */
export type ShutdownKind = 'session-ended' | 'idle-timeout' | 'server';

export interface TerminalConnectionCallbacks {
  /** Connection status transitions (connecting/connected/disconnected/error). */
  onStatus?(status: ConnectionStatus): void;
  /** Reconnect-backoff state changed. */
  onRetryingChange?(retrying: boolean): void;
  /** Socket closed (after banner-worthy disconnect). */
  onConnectionClosed?(): void;
  /** Socket errored. */
  onConnectionError?(): void;
  /** The configured URL was rejected by the WebSocket constructor. */
  onInvalidUrl?(url: string): void;
  /** Socket opened (after fitting the terminal, etc.). */
  onOpen?(): void;
  /** Decoded terminal output text. */
  onOutput?(data: string): void;
  onConnected?(connected: ServerCase<'connected'>): void;
  onServerResize?(resize: ServerCase<'resize'>): void;
  onRefresh?(refresh: ServerCase<'refresh'>): void;
  onTitle?(title: ServerCase<'title'>): void;
  onServerError?(message: string): void;
  /**
   * Server is shutting down. `kind` mirrors the reason classification:
   * 'session-ended' also stops auto-reconnect (the shell is gone).
   */
  onShutdown?(reason: string, kind: ShutdownKind): void;
  onModeChanged?(mode: string, enabled: boolean): void;
  onHyperlinkAdded?(link: ServerCase<'hyperlinkAdded'>): void;
  onUserVarChanged?(userVar: ServerCase<'userVarChanged'>): void;
  onSelectionChanged?(selection: ServerCase<'selectionChanged'>): void;
}

const HEARTBEAT_INTERVAL_MS = 25000; // Send ping every 25 seconds
const HEARTBEAT_TIMEOUT_MS = 10000; // Consider stale if no pong within 10 seconds
const RETRY_DELAY_BASE_MS = 500;
const RETRY_DELAY_MAX_MS = 5000;

const debugLog = (...args: unknown[]): void => {
  if (process.env.NODE_ENV !== 'production') {
    console.log(...args);
  }
};

// Shared TextDecoder instance - reuse instead of creating per message
const sharedDecoder = new TextDecoder();

export class TerminalConnection {
  private ws: WebSocket | null = null;
  private retryTimeout: ReturnType<typeof setTimeout> | null = null;
  private retryDelay = RETRY_DELAY_BASE_MS;
  private isRetrying = false;
  private retryCancelled = false;
  private heartbeatInterval: ReturnType<typeof setInterval> | null = null;
  private lastPong = 0;
  private disposed = false;

  constructor(
    private readonly url: string,
    private readonly callbacks: TerminalConnectionCallbacks,
  ) {}

  /** The URL this connection was created for. */
  getUrl(): string {
    return this.url;
  }

  /** Whether the underlying socket is currently open. */
  isOpen(): boolean {
    return this.ws?.readyState === WebSocket.OPEN;
  }

  /** Open the connection (closes an existing open socket first). */
  connect(): void {
    if (this.disposed) return;

    // If already connected, close existing socket first
    if (this.ws?.readyState === WebSocket.OPEN) {
      debugLog('Closing existing connection before reconnecting');
      this.ws.close();
      this.ws = null;
    }

    // Reset cancelled flag when manually connecting
    this.retryCancelled = false;
    this.callbacks.onStatus?.('connecting');

    let ws: WebSocket;
    try {
      ws = new WebSocket(this.url);
    } catch (err) {
      console.error('Invalid WebSocket URL:', err);
      this.callbacks.onStatus?.('error');
      this.callbacks.onInvalidUrl?.(this.url);
      return;
    }

    ws.binaryType = 'arraybuffer'; // Use binary protocol
    this.ws = ws;

    ws.onopen = () => {
      debugLog('WebSocket connected');
      this.callbacks.onStatus?.('connected');
      // Reset retry delay on successful connection
      this.retryDelay = RETRY_DELAY_BASE_MS;
      this.isRetrying = false;
      this.callbacks.onRetryingChange?.(false);

      // Start heartbeat for stale connection detection
      this.startHeartbeat();

      this.callbacks.onOpen?.();
      // Note: resize and refresh are sent after receiving 'connected' message
    };

    ws.onmessage = (event) => {
      try {
        const msg = decodeServerMessage(event.data);
        this.dispatch(msg);
      } catch (err) {
        console.error('Failed to decode message:', err);
      }
    };

    ws.onerror = (error) => {
      console.error('WebSocket error:', error);
      this.stopHeartbeat();
      this.callbacks.onStatus?.('error');
      this.callbacks.onConnectionError?.();
    };

    ws.onclose = () => {
      debugLog('WebSocket disconnected');
      this.stopHeartbeat();
      this.ws = null;
      this.callbacks.onStatus?.('disconnected');
      this.callbacks.onConnectionClosed?.();
      // Auto-reconnect unless cancelled
      if (!this.retryCancelled) {
        this.scheduleRetry();
      }
    };
  }

  /** Send a client message (no-op unless the socket is open). */
  send(msg: ClientMessage): void {
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(encodeClientMessage(msg));
    }
  }

  /** Stop reconnecting and reset the backoff (does not close the socket). */
  cancelRetry(): void {
    if (this.retryTimeout) {
      clearTimeout(this.retryTimeout);
      this.retryTimeout = null;
    }
    this.isRetrying = false;
    this.retryCancelled = true;
    this.retryDelay = RETRY_DELAY_BASE_MS;
    this.callbacks.onRetryingChange?.(false);
  }

  /** Stop the heartbeat timer (does not close the socket). */
  stopHeartbeat(): void {
    if (this.heartbeatInterval) {
      clearInterval(this.heartbeatInterval);
      this.heartbeatInterval = null;
    }
  }

  /**
   * Tear the connection down: cancels retries, stops the heartbeat, and
   * closes the socket. The close still fires the usual disconnected
   * status/banner callbacks (deliberate disconnects are observable too,
   * matching the previous inline behavior). Idempotent — safe to call
   * from cleanup paths that may run more than once (React StrictMode).
   */
  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.cancelRetry();
    this.stopHeartbeat();
    if (this.ws) {
      this.ws.close();
    }
  }

  private scheduleRetry(): void {
    if (this.retryCancelled) return;

    this.isRetrying = true;
    this.callbacks.onRetryingChange?.(true);

    const delay = this.retryDelay;
    debugLog(`Scheduling reconnect in ${delay}ms`);

    this.retryTimeout = setTimeout(() => {
      if (!this.retryCancelled) {
        // Increase delay for next retry (max 5 seconds)
        this.retryDelay = Math.min(this.retryDelay * 2, RETRY_DELAY_MAX_MS);
        this.connect();
      }
    }, delay);
  }

  private startHeartbeat(): void {
    this.stopHeartbeat(); // Clear any existing heartbeat
    this.lastPong = Date.now(); // Initialize last pong time

    this.heartbeatInterval = setInterval(() => {
      const ws = this.ws;
      if (!ws || ws.readyState !== WebSocket.OPEN) {
        this.stopHeartbeat();
        return;
      }

      const now = Date.now();
      const timeSinceLastPong = now - this.lastPong;

      // Check if connection is stale (no pong received within timeout)
      if (timeSinceLastPong > HEARTBEAT_INTERVAL_MS + HEARTBEAT_TIMEOUT_MS) {
        console.warn(`Connection stale: no pong in ${timeSinceLastPong}ms, closing`);
        this.stopHeartbeat();
        ws.close();
        return;
      }

      // Send ping
      try {
        this.send(createPingMessage());
        debugLog('Heartbeat ping sent');
      } catch (err) {
        console.error('Failed to send heartbeat ping:', err);
        this.stopHeartbeat();
        ws.close();
      }
    }, HEARTBEAT_INTERVAL_MS);
  }

  private dispatch(msg: ServerMessage): void {
    switch (msg.message.case) {
      case 'output': {
        const output = msg.message.value;
        this.callbacks.onOutput?.(sharedDecoder.decode(output.data));
        break;
      }

      case 'connected':
        this.callbacks.onConnected?.(msg.message.value);
        break;

      case 'resize':
        this.callbacks.onServerResize?.(msg.message.value);
        break;

      case 'refresh':
        this.callbacks.onRefresh?.(msg.message.value);
        break;

      case 'title':
        this.callbacks.onTitle?.(msg.message.value);
        break;

      case 'bell':
        debugLog('Bell received');
        break;

      case 'error':
        this.callbacks.onServerError?.(msg.message.value.message);
        break;

      case 'shutdown': {
        const shutdown = msg.message.value;
        const reason = shutdown.reason || '';
        debugLog('Server shutdown:', shutdown.reason);

        let kind: ShutdownKind;
        if (reason.toLowerCase().includes('shell exited') || reason.toLowerCase().includes('dead session')) {
          kind = 'session-ended';
          this.retryCancelled = true; // Don't auto-reconnect
        } else if (reason.toLowerCase().includes('idle timeout')) {
          kind = 'idle-timeout';
          // Allow reconnect — will create new session
        } else {
          kind = 'server';
          // Allow reconnect for server restarts
        }
        this.callbacks.onShutdown?.(reason, kind);
        break;
      }

      case 'modeChanged':
        this.callbacks.onModeChanged?.(msg.message.value.mode, msg.message.value.enabled);
        break;

      case 'pong':
        // Pong received - update last pong time for heartbeat tracking
        this.lastPong = Date.now();
        debugLog('Heartbeat pong received');
        break;

      case 'hyperlinkAdded':
        this.callbacks.onHyperlinkAdded?.(msg.message.value);
        break;

      case 'userVarChanged':
        this.callbacks.onUserVarChanged?.(msg.message.value);
        break;

      case 'selectionChanged':
        this.callbacks.onSelectionChanged?.(msg.message.value);
        break;

      default:
        // Silently ignore other message types (cwdChanged, triggerMatched, etc.)
        break;
    }
  }
}
