import { describe, it, expect } from 'vitest';
import { clampPan, fitWidthFontSize, pinchPan, surplusStrips } from '@/lib/viewport';

describe('fitWidthFontSize', () => {
  it('picks the font size at which the columns fill the width', () => {
    // 0.6 px of cell width per font px: 100 cols at 6.5px font = 390px.
    expect(fitWidthFontSize(390, 100, 0.6)).toBeCloseTo(6.5);
  });

  it('clamps to the zoom range and rejects unmeasurable input', () => {
    expect(fitWidthFontSize(390, 1000, 0.6)).toBe(2);
    expect(fitWidthFontSize(3000, 10, 0.6)).toBe(32);
    expect(fitWidthFontSize(390, 0, 0.6)).toBeNull();
    expect(fitWidthFontSize(390, 100, 0)).toBeNull();
    expect(fitWidthFontSize(390, 100, NaN)).toBeNull();
  });
});

describe('clampPan', () => {
  const view = { width: 400, height: 300 };

  it('keeps an overflowing grid covering the view', () => {
    const content = { width: 1000, height: 900 };
    expect(clampPan({ x: 50, y: 20 }, content, view)).toEqual({ x: 0, y: 0 });
    expect(clampPan({ x: -5000, y: -5000 }, content, view)).toEqual({ x: -600, y: -600 });
    expect(clampPan({ x: -100, y: -200 }, content, view)).toEqual({ x: -100, y: -200 });
  });

  it('pins an axis the grid does not overflow to the origin', () => {
    expect(clampPan({ x: -100, y: -100 }, { width: 1000, height: 200 }, view)).toEqual({ x: -100, y: 0 });
  });
});

describe('pinchPan', () => {
  it('keeps the point under the fingers fixed while zooming', () => {
    const startPan = { x: -100, y: -50 };
    const startMid = { x: 200, y: 150 };
    const pan = pinchPan(startPan, startMid, startMid, 2);
    // Content point under the midpoint: (startMid - startPan) = (300, 200).
    // At 2x it sits at pan + 2 * (300, 200), which must equal the midpoint.
    expect(pan.x + 2 * 300).toBe(200);
    expect(pan.y + 2 * 200).toBe(150);
  });

  it('follows the midpoint when the fingers move', () => {
    expect(pinchPan({ x: 0, y: 0 }, { x: 10, y: 10 }, { x: 30, y: 40 }, 1)).toEqual({ x: 20, y: 30 });
  });
});

describe('surplusStrips', () => {
  it('returns the areas right of and below a smaller grid', () => {
    const { right, bottom } = surplusStrips({ left: 0, top: 0, width: 600, height: 400 }, { width: 1000, height: 700 });
    expect(right).toEqual({ left: 600, top: 0, width: 400, height: 700 });
    expect(bottom).toEqual({ left: 0, top: 400, width: 600, height: 300 });
  });

  it('returns nothing when the grid covers the view', () => {
    expect(surplusStrips({ left: -50, top: -20, width: 2000, height: 900 }, { width: 1000, height: 700 })).toEqual({
      right: null,
      bottom: null,
    });
  });

  it('handles a panned grid shorter than the view', () => {
    const { right, bottom } = surplusStrips({ left: -300, top: 0, width: 800, height: 400 }, { width: 400, height: 700 });
    expect(right).toBeNull();
    expect(bottom).toEqual({ left: 0, top: 400, width: 400, height: 300 });
  });
});
