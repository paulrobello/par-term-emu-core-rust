/**
 * Protocol Buffers wire format handling for terminal streaming
 *
 * This module provides binary serialization using Protocol Buffers with
 * optional zlib compression for large payloads.
 *
 * Wire Format:
 * Each message is prefixed with a 1-byte header:
 * - 0x00: Uncompressed protobuf payload
 * - 0x01: Zlib-compressed protobuf payload
 *
 * Compression is applied automatically for payloads exceeding 1KB.
 */

import { create, toBinary, fromBinary } from '@bufbuild/protobuf';
import {
  ServerMessage,
  ServerMessageSchema,
  ClientMessage,
  ClientMessageSchema,
  InputSchema,
  ClientResizeSchema,
  PingSchema,
  RequestRefreshSchema,
  type ThemeInfo,
} from './proto/terminal_pb';
import pako from 'pako';

/** Compression threshold in bytes (1KB) */
const COMPRESSION_THRESHOLD = 1024;

/** Wire format flags */
const FLAG_UNCOMPRESSED = 0x00;
const FLAG_COMPRESSED = 0x01;

/**
 * Encode a client message to binary format with optional compression
 */
export function encodeClientMessage(msg: ClientMessage): ArrayBuffer {
  const payload = toBinary(ClientMessageSchema, msg);

  if (payload.length > COMPRESSION_THRESHOLD) {
    const compressed = pako.deflate(payload);
    if (compressed.length < payload.length) {
      const result = new Uint8Array(compressed.length + 1);
      result[0] = FLAG_COMPRESSED;
      result.set(compressed, 1);
      return result.buffer;
    }
  }

  // Uncompressed
  const result = new Uint8Array(payload.length + 1);
  result[0] = FLAG_UNCOMPRESSED;
  result.set(payload, 1);
  return result.buffer;
}

/**
 * Decode a server message from binary format
 */
export function decodeServerMessage(data: ArrayBuffer): ServerMessage {
  const bytes = new Uint8Array(data);

  if (bytes.length === 0) {
    throw new Error('Empty message');
  }

  const flags = bytes[0];
  const payload = bytes.slice(1);

  const decoded =
    flags & FLAG_COMPRESSED ? pako.inflate(payload) : payload;

  return fromBinary(ServerMessageSchema, decoded);
}

// =============================================================================
// Helper factories for creating client messages
// =============================================================================

/**
 * Create an input message from keyboard data
 */
export function createInputMessage(data: string): ClientMessage {
  const encoder = new TextEncoder();
  return create(ClientMessageSchema, {
    message: {
      case: 'input',
      value: create(InputSchema, {
        data: encoder.encode(data),
      }),
    },
  });
}

/**
 * Create a resize message
 */
export function createResizeMessage(cols: number, rows: number): ClientMessage {
  return create(ClientMessageSchema, {
    message: {
      case: 'resize',
      value: create(ClientResizeSchema, {
        cols,
        rows,
      }),
    },
  });
}

/**
 * Create a ping message for keepalive
 */
export function createPingMessage(): ClientMessage {
  return create(ClientMessageSchema, {
    message: {
      case: 'ping',
      value: create(PingSchema, {}),
    },
  });
}

/**
 * Create a refresh request message
 */
export function createRefreshMessage(): ClientMessage {
  return create(ClientMessageSchema, {
    message: {
      case: 'refresh',
      value: create(RequestRefreshSchema, {}),
    },
  });
}

// =============================================================================
// Theme conversion helpers
// =============================================================================

/**
 * Convert a protobuf Color to an RGB hex string
 */
export function colorToHex(color: { r: number; g: number; b: number }): string {
  const toHex = (n: number) => n.toString(16).padStart(2, '0');
  return `#${toHex(color.r)}${toHex(color.g)}${toHex(color.b)}`;
}

/**
 * Convert a ThemeInfo to xterm.js theme options
 */
export function themeToXtermOptions(theme: ThemeInfo): Record<string, string> {
  const result: Record<string, string> = {
    background: theme.background ? colorToHex(theme.background) : '#000000',
    foreground: theme.foreground ? colorToHex(theme.foreground) : '#ffffff',
  };

  // Map normal colors (0-7)
  const normalNames = ['black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white'];
  theme.normal.forEach((color, i) => {
    if (i < normalNames.length) {
      result[normalNames[i]] = colorToHex(color);
    }
  });

  // Map bright colors (8-15)
  const brightNames = [
    'brightBlack',
    'brightRed',
    'brightGreen',
    'brightYellow',
    'brightBlue',
    'brightMagenta',
    'brightCyan',
    'brightWhite',
  ];
  theme.bright.forEach((color, i) => {
    if (i < brightNames.length) {
      result[brightNames[i]] = colorToHex(color);
    }
  });

  return result;
}

// Re-export types for convenience
export type { ServerMessage, ClientMessage, ThemeInfo };
