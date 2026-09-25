/**
 * Geometry for viewing a shared pane whose size this viewer does not own.
 *
 * A mux pane has one size (latest resize wins), so a viewer is often larger
 * or smaller than the grid it shows:
 * - larger: the surplus is filled with dim dots, tmux style (`surplusStrips`);
 * - smaller (a phone): the full grid is shown zoomed to fit the width, with
 *   pinch to zoom and drag to pan (`fitWidthFontSize`, `clampPan`, `pinchPan`).
 *
 * Zoom is committed as a font size, not a CSS scale, so xterm's own mouse,
 * selection and link hit-testing keep matching the rendered cells. A CSS
 * scale is used only as a live preview while a pinch is in progress.
 */

export interface Point {
  x: number;
  y: number;
}

export interface Size {
  width: number;
  height: number;
}

export interface Rect {
  left: number;
  top: number;
  width: number;
  height: number;
}

export const MIN_ZOOM_FONT_SIZE = 2;
export const MAX_ZOOM_FONT_SIZE = 32;

export const clamp = (value: number, min: number, max: number): number =>
  Math.min(max, Math.max(min, value));

/**
 * Font size at which `cols` columns exactly fill `availableWidth`.
 *
 * `pxPerColPerFontPx` is the measured cell width divided by the font size
 * (monospace cell width scales linearly with font size).
 */
export function fitWidthFontSize(
  availableWidth: number,
  cols: number,
  pxPerColPerFontPx: number,
  min = MIN_ZOOM_FONT_SIZE,
  max = MAX_ZOOM_FONT_SIZE,
): number | null {
  if (availableWidth <= 0 || cols <= 0 || !(pxPerColPerFontPx > 0)) return null;
  return clamp(availableWidth / (cols * pxPerColPerFontPx), min, max);
}

/**
 * Clamp a content offset so the content never leaves a gap it could cover.
 * Content no larger than the view on an axis is pinned to the origin on it.
 */
export function clampPan(pan: Point, content: Size, view: Size): Point {
  const axis = (offset: number, contentLen: number, viewLen: number): number =>
    contentLen <= viewLen ? 0 : clamp(offset, viewLen - contentLen, 0);
  return {
    x: axis(pan.x, content.width, view.width),
    y: axis(pan.y, content.height, view.height),
  };
}

/**
 * Offset that keeps the content point under the pinch's starting midpoint
 * under the current midpoint, at `scale` relative to the pinch start.
 */
export function pinchPan(startPan: Point, startMid: Point, mid: Point, scale: number): Point {
  return {
    x: mid.x - (startMid.x - startPan.x) * scale,
    y: mid.y - (startMid.y - startPan.y) * scale,
  };
}

/**
 * The parts of `view` not covered by `grid` (both in view coordinates): a
 * strip right of the grid and one below it. Null when there is no surplus.
 */
export function surplusStrips(grid: Rect, view: Size): { right: Rect | null; bottom: Rect | null } {
  const gridRight = Math.max(0, grid.left + grid.width);
  const gridBottom = Math.max(0, grid.top + grid.height);
  const right =
    gridRight < view.width
      ? { left: gridRight, top: 0, width: view.width - gridRight, height: view.height }
      : null;
  const bottom =
    gridBottom < view.height
      ? {
          left: 0,
          top: gridBottom,
          width: Math.min(gridRight, view.width),
          height: view.height - gridBottom,
        }
      : null;
  return { right, bottom: bottom && bottom.width > 0 ? bottom : null };
}
