/**
 * OSC 22 pointer-shape handling for the streaming web frontend.
 *
 * The core parses OSC 22 as the kitty/ghostty/iTerm2 pointer shape and the
 * streamer forwards the name (or none) as `pointer_shape_changed`. Apps write
 * CSS names (`ew-resize`, `ns-resize`, `text`) plus the X11 cursor-font
 * synonyms iTerm2's map parses (`sb_h_double_arrow`, `sb_v_double_arrow`).
 * Browsers apply CSS cursor keywords natively; the synonyms map to their CSS
 * equivalents; an absent/empty shape resets to the terminal default (`text`);
 * an unknown name leaves the current cursor untouched (returns null).
 */

const RESET_CURSOR = 'text';

/** X11 cursor-font names an OSC 22 writer may also emit → CSS equivalents. */
const X11_SYNONYMS: Record<string, string> = {
  arrow: 'default',
  ibeam: 'text',
  sb_h_double_arrow: 'ew-resize',
  sb_v_double_arrow: 'ns-resize',
};

/** Standard CSS cursor keywords applied verbatim. */
const CSS_CURSORS = new Set([
  'default',
  'none',
  'context-menu',
  'help',
  'pointer',
  'progress',
  'wait',
  'cell',
  'crosshair',
  'text',
  'vertical-text',
  'alias',
  'copy',
  'move',
  'no-drop',
  'not-allowed',
  'grab',
  'grabbing',
  'all-scroll',
  'col-resize',
  'row-resize',
  'n-resize',
  'e-resize',
  's-resize',
  'w-resize',
  'ne-resize',
  'nw-resize',
  'se-resize',
  'sw-resize',
  'ew-resize',
  'ns-resize',
  'nesw-resize',
  'nwse-resize',
  'zoom-in',
  'zoom-out',
]);

/**
 * Map an OSC 22 shape name to a CSS cursor value for `style.cursor`.
 *
 * Returns the CSS cursor to apply; `null` for an unknown name (the caller
 * keeps the current cursor); the terminal default `text` for a reset.
 */
export function cursorForPointerShape(shape: string | null | undefined): string | null {
  if (shape === null || shape === undefined || shape === '') return RESET_CURSOR;
  const mapped = X11_SYNONYMS[shape];
  if (mapped) return mapped;
  if (CSS_CURSORS.has(shape)) return shape;
  return null;
}
