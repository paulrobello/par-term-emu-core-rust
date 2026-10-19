import { describe, expect, it } from 'vitest';
import { cursorForPointerShape } from '../pointer-shape';

describe('cursorForPointerShape', () => {
  it.each([
    ['ew-resize', 'ew-resize'],
    ['ns-resize', 'ns-resize'],
    ['text', 'text'],
    ['crosshair', 'crosshair'],
    ['col-resize', 'col-resize'],
    ['sb_h_double_arrow', 'ew-resize'],
    ['sb_v_double_arrow', 'ns-resize'],
    ['arrow', 'default'],
    ['ibeam', 'text'],
  ])('maps $shape to $expected', (shape, expected) => {
    expect(cursorForPointerShape(shape)).toBe(expected);
  });

  it('resets to the terminal default on empty/None', () => {
    for (const shape of [null, undefined, '']) {
      expect(cursorForPointerShape(shape)).toBe('text');
    }
  });

  it('unknown names return null so the current cursor is kept', () => {
    for (const shape of ['wobble', 'Ew-Resize', 'EW-RESIZE']) {
      expect(cursorForPointerShape(shape)).toBeNull();
    }
  });
});
