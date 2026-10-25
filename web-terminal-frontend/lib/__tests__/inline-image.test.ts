/**
 * Inline-image render half: a fabricated ServerMessage::InlineImage frame
 * must produce the iTerm2 sequence the xterm ImageAddon decodes — and
 * refuse frames the renderer cannot honestly draw.
 */

import { describe, it, expect, vi, afterEach } from 'vitest';
import { create } from '@bufbuild/protobuf';
import { InlineImageSchema, type InlineImage } from '@/lib/proto/terminal_pb';
import { inlineImageSequence } from '@/lib/inline-image';

/** A 2x2 RGBA frame with a distinctive byte pattern. */
function makeFrame(overrides: Partial<InlineImage> = {}): InlineImage {
  return create(InlineImageSchema, {
    imageId: 42n,
    paneId: 7,
    format: 'rgba',
    width: 2,
    height: 2,
    data: new Uint8Array(16).map((_, i) => i + 1),
    ...overrides,
  });
}

/** jsdom has no canvas; record what the sequence builder put into one. */
interface CanvasRecord {
  width: number;
  height: number;
  pixels: Uint8ClampedArray | null;
}

let drawn: CanvasRecord | null = null;

function stubCanvas(): void {
  drawn = null;
  vi.spyOn(document, 'createElement').mockImplementation(((tag: string) => {
    if (tag !== 'canvas') {
      throw new Error(`unexpected createElement(${tag}) in test`);
    }
    const record: CanvasRecord = { width: 0, height: 0, pixels: null };
    drawn = record;
    return {
      get width() {
        return record.width;
      },
      set width(v: number) {
        record.width = v;
      },
      get height() {
        return record.height;
      },
      set height(v: number) {
        record.height = v;
      },
      getContext: () => ({
        createImageData: (w: number, h: number) => ({
          data: new Uint8ClampedArray(w * h * 4),
        }),
        putImageData: (img: { data: Uint8ClampedArray }) => {
          record.pixels = img.data;
        },
      }),
      toDataURL: () => 'data:image/png;base64,cG5nLXB5dGhvbg==',
    };
  }) as unknown as typeof document.createElement);
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe('inlineImageSequence', () => {
  it('turns an rgba frame into the iTerm2 sequence the ImageAddon decodes', () => {
    stubCanvas();
    const seq = inlineImageSequence(makeFrame());
    expect(seq).toBeTruthy();
    expect(seq?.startsWith('\x1b]1337;File=')).toBe(true);
    expect(seq?.endsWith(':cG5nLXB5dGhvbg==\x07')).toBe(true);
    expect(seq).toContain('inline=1');
    expect(seq).toContain('width=2px;height=2px');
    expect(drawn?.width).toBe(2);
    expect(drawn?.height).toBe(2);
    expect(Array.from(drawn?.pixels ?? [])).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
  });

  it('refuses a non-rgba frame', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    expect(inlineImageSequence(makeFrame({ format: 'png' }))).toBeNull();
    expect(warn).toHaveBeenCalled();
  });

  it('refuses a zero-extent frame', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    expect(inlineImageSequence(makeFrame({ width: 0, height: 2 }))).toBeNull();
    expect(warn).toHaveBeenCalled();
  });

  it('refuses a payload shorter than the declared pixels', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    expect(inlineImageSequence(makeFrame({ data: new Uint8Array(8) }))).toBeNull();
    expect(warn).toHaveBeenCalled();
  });
});
