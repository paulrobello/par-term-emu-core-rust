/**
 * Round-trip and guard tests for the protobuf wire format in lib/protocol.ts
 * (QA-005).
 *
 * Covers every client-message factory (encode -> strip framing -> fromBinary),
 * the compressed/uncompressed framing decision, server-message decoding, and
 * the size guards that reject oversized payloads before inflation.
 */

import { describe, it, expect } from 'vitest';
import { create, fromBinary, toBinary } from '@bufbuild/protobuf';
import * as pako from 'pako';
import {
  encodeClientMessage,
  decodeServerMessage,
  createInputMessage,
  createResizeMessage,
  createPingMessage,
  createRefreshMessage,
  createMouseMessage,
  createFocusMessage,
  createPasteMessage,
  createSubscribeMessage,
} from '../protocol';
import {
  ClientMessageSchema,
  ServerMessageSchema,
  OutputSchema,
  TitleSchema,
  ErrorSchema,
  ShutdownSchema,
  PongSchema,
  ModeChangedSchema,
} from '../proto/terminal_pb';
import type { ClientMessage, ServerMessage } from '../proto/terminal_pb';

const FLAG_UNCOMPRESSED = 0x00;
const FLAG_COMPRESSED = 0x01;

const encoder = new TextEncoder();
const decoder = new TextDecoder();

/** Drop the 1-byte framing header from an encoded message. */
function stripFrameFlag(buf: ArrayBuffer): Uint8Array {
  return new Uint8Array(buf).slice(1);
}

/** Build a wire-format server frame the same way the server does. */
function frameServerMessage(msg: ServerMessage, compress = false): ArrayBuffer {
  const payload = toBinary(ServerMessageSchema, msg);
  const body = compress ? pako.deflate(payload) : payload;
  const frame = new Uint8Array(body.length + 1);
  frame[0] = compress ? FLAG_COMPRESSED : FLAG_UNCOMPRESSED;
  frame.set(body, 1);
  return frame.buffer;
}

/** Encode a client message, decode it back, and return the decoded copy. */
function roundTrip(msg: ClientMessage): ClientMessage {
  return fromBinary(ClientMessageSchema, stripFrameFlag(encodeClientMessage(msg)));
}

describe('encodeClientMessage round-trips', () => {
  it('round-trips an input message including non-ASCII bytes', () => {
    const original = createInputMessage('ls -la\r\necho "héllo 世界"\n');
    const decoded = roundTrip(original);

    expect(decoded.message.case).toBe('input');
    if (decoded.message.case === 'input') {
      expect(decoder.decode(decoded.message.value.data))
        .toBe('ls -la\r\necho "héllo 世界"\n');
    }
  });

  it('round-trips a resize message', () => {
    const decoded = roundTrip(createResizeMessage(132, 43));

    expect(decoded.message.case).toBe('resize');
    if (decoded.message.case === 'resize') {
      expect(decoded.message.value.cols).toBe(132);
      expect(decoded.message.value.rows).toBe(43);
    }
  });

  it('round-trips a ping message', () => {
    expect(roundTrip(createPingMessage()).message.case).toBe('ping');
  });

  it('round-trips a refresh request message', () => {
    expect(roundTrip(createRefreshMessage()).message.case).toBe('refresh');
  });

  it('round-trips a mouse message with all modifiers', () => {
    const decoded = roundTrip(createMouseMessage(12, 4, 0, 'down', true, false, true));

    expect(decoded.message.case).toBe('mouse');
    if (decoded.message.case === 'mouse') {
      // toMatchObject: decoded messages carry $typeName metadata.
      expect(decoded.message.value).toMatchObject({
        col: 12,
        row: 4,
        button: 0,
        eventType: 'down',
        shift: true,
        ctrl: false,
        alt: true,
      });
    }
  });

  it('round-trips focus and paste messages', () => {
    const focus = roundTrip(createFocusMessage(false));
    expect(focus.message.case).toBe('focus');
    if (focus.message.case === 'focus') {
      expect(focus.message.value.focused).toBe(false);
    }

    const paste = roundTrip(createPasteMessage('pasted text'));
    expect(paste.message.case).toBe('paste');
    if (paste.message.case === 'paste') {
      expect(paste.message.value.content).toBe('pasted text');
    }
  });

  it('round-trips a subscribe message with an event mask', () => {
    const decoded = roundTrip(createSubscribeMessage([1, 2, 4]));

    expect(decoded.message.case).toBe('subscribe');
    if (decoded.message.case === 'subscribe') {
      expect(decoded.message.value.events).toEqual([1, 2, 4]);
    }
  });
});

describe('encodeClientMessage framing', () => {
  it('leaves payloads under 1KB uncompressed', () => {
    const buf = encodeClientMessage(createInputMessage('tiny'));
    const bytes = new Uint8Array(buf);

    expect(bytes[0]).toBe(FLAG_UNCOMPRESSED);
  });

  it('compresses payloads over 1KB and survives inflate + decode', () => {
    const original = createInputMessage('x'.repeat(5000));
    const bytes = new Uint8Array(encodeClientMessage(original));

    expect(bytes[0]).toBe(FLAG_COMPRESSED);
    const payload = pako.inflate(bytes.slice(1));
    const decoded = fromBinary(ClientMessageSchema, payload);
    expect(decoded.message.case).toBe('input');
    if (decoded.message.case === 'input') {
      expect(decoded.message.value.data.length).toBe(5000);
    }
  });
});

describe('decodeServerMessage', () => {
  it('round-trips an output message with ANSI escape sequences', () => {
    const text = 'line\r\n\x1b[31mred\x1b[0m plain';
    const msg: ServerMessage = create(ServerMessageSchema, {
      message: {
        case: 'output',
        value: create(OutputSchema, { data: encoder.encode(text) }),
      },
    });

    const decoded = decodeServerMessage(frameServerMessage(msg));
    expect(decoded).toEqual(msg);
    if (decoded.message.case === 'output') {
      expect(decoder.decode(decoded.message.value.data)).toBe(text);
    }
  });

  it('round-trips title, error, shutdown, pong, and modeChanged messages', () => {
    const cases: ServerMessage[] = [
      create(ServerMessageSchema, {
        message: { case: 'title', value: create(TitleSchema, { title: 'my tab' }) },
      }),
      create(ServerMessageSchema, {
        message: { case: 'error', value: create(ErrorSchema, { message: 'boom' }) },
      }),
      create(ServerMessageSchema, {
        message: { case: 'shutdown', value: create(ShutdownSchema, { reason: 'server restarting' }) },
      }),
      create(ServerMessageSchema, {
        message: { case: 'pong', value: create(PongSchema, {}) },
      }),
      create(ServerMessageSchema, {
        message: {
          case: 'modeChanged',
          value: create(ModeChangedSchema, { mode: 'cursor_visible', enabled: true }),
        },
      }),
    ];

    for (const msg of cases) {
      expect(decodeServerMessage(frameServerMessage(msg))).toEqual(msg);
    }
  });

  it('decodes zlib-compressed server frames', () => {
    const msg: ServerMessage = create(ServerMessageSchema, {
      message: {
        case: 'output',
        value: create(OutputSchema, { data: encoder.encode('data '.repeat(600)) }),
      },
    });

    const decoded = decodeServerMessage(frameServerMessage(msg, true));
    expect(decoded).toEqual(msg);
  });

  it('rejects empty frames', () => {
    expect(() => decodeServerMessage(new ArrayBuffer(0))).toThrow(/Empty message/);
  });

  it('rejects uncompressed payloads over 2MB', () => {
    const oversized = new Uint8Array(2 * 1024 * 1024 + 2);
    oversized[0] = FLAG_UNCOMPRESSED;

    expect(() => decodeServerMessage(oversized.buffer)).toThrow(/Payload too large/);
  });

  it('rejects compressed payloads over 256KB before inflating', () => {
    // Incompressible bytes so deflate output stays above the compressed limit.
    const raw = new Uint8Array(300 * 1024);
    let seed = 0x2f6e2b1;
    for (let i = 0; i < raw.length; i++) {
      // xorshift32
      seed ^= seed << 13;
      seed ^= seed >>> 17;
      seed ^= seed << 5;
      raw[i] = seed & 0xff;
    }
    const compressed = pako.deflate(raw);
    expect(compressed.length).toBeGreaterThan(256 * 1024);

    const frame = new Uint8Array(compressed.length + 1);
    frame[0] = FLAG_COMPRESSED;
    frame.set(compressed, 1);

    expect(() => decodeServerMessage(frame.buffer)).toThrow(/Compressed payload too large/);
  });
});
