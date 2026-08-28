/**
 * Static onscreen-keyboard layout data (QA-008).
 *
 * Pure data + pure helpers for the onscreen keyboard, split out of
 * components/OnscreenKeyboard.tsx so the component keeps only behavior.
 */

export interface KeyDefinition {
  label: string;
  shortLabel?: string;
  code: string;
  width?: number; // width multiplier (1 = standard key width)
  isModifier?: boolean;
  modifierType?: 'ctrl' | 'alt' | 'shift';
}

export interface CtrlShortcut extends KeyDefinition {
  tooltip: string;
}

// ANSI escape sequences for special keys
export const ESCAPE_SEQUENCES = {
  escape: '\x1b',
  tab: '\t',
  enter: '\r',
  backspace: '\x7f',
  delete: '\x1b[3~',
  insert: '\x1b[2~',
  home: '\x1b[H',
  end: '\x1b[F',
  pageUp: '\x1b[5~',
  pageDown: '\x1b[6~',
  arrowUp: '\x1b[A',
  arrowDown: '\x1b[B',
  arrowRight: '\x1b[C',
  arrowLeft: '\x1b[D',
  f1: '\x1bOP',
  f2: '\x1bOQ',
  f3: '\x1bOR',
  f4: '\x1bOS',
  f5: '\x1b[15~',
  f6: '\x1b[17~',
  f7: '\x1b[18~',
  f8: '\x1b[19~',
  f9: '\x1b[20~',
  f10: '\x1b[21~',
  f11: '\x1b[23~',
  f12: '\x1b[24~',
};

// Control key combinations (Ctrl + letter = ASCII code 1-26)
export const getCtrlCode = (char: string): string => {
  const upper = char.toUpperCase();
  if (upper >= 'A' && upper <= 'Z') {
    return String.fromCharCode(upper.charCodeAt(0) - 64);
  }
  // Special cases
  switch (char) {
    case '[': return '\x1b';
    case '\\': return '\x1c';
    case ']': return '\x1d';
    case '^': return '\x1e';
    case '_': return '\x1f';
    default: return char;
  }
};

// Alt key combinations (ESC + character)
export const getAltCode = (char: string): string => {
  return '\x1b' + char;
};

// Keyboard layout definitions
export const FUNCTION_ROW: KeyDefinition[] = [
  { label: 'F1', code: ESCAPE_SEQUENCES.f1 },
  { label: 'F2', code: ESCAPE_SEQUENCES.f2 },
  { label: 'F3', code: ESCAPE_SEQUENCES.f3 },
  { label: 'F4', code: ESCAPE_SEQUENCES.f4 },
  { label: 'F5', code: ESCAPE_SEQUENCES.f5 },
  { label: 'F6', code: ESCAPE_SEQUENCES.f6 },
  { label: 'F7', code: ESCAPE_SEQUENCES.f7 },
  { label: 'F8', code: ESCAPE_SEQUENCES.f8 },
  { label: 'F9', code: ESCAPE_SEQUENCES.f9 },
  { label: 'F10', code: ESCAPE_SEQUENCES.f10 },
  { label: 'F11', code: ESCAPE_SEQUENCES.f11 },
  { label: 'F12', code: ESCAPE_SEQUENCES.f12 },
];

export const NAV_ROW: KeyDefinition[] = [
  { label: 'Esc', code: ESCAPE_SEQUENCES.escape },
  { label: 'Tab', code: ESCAPE_SEQUENCES.tab, width: 1.5 },
  { label: 'Ins', code: ESCAPE_SEQUENCES.insert },
  { label: 'Del', code: ESCAPE_SEQUENCES.delete },
  { label: 'Home', code: ESCAPE_SEQUENCES.home },
  { label: 'End', code: ESCAPE_SEQUENCES.end },
  { label: 'PgUp', shortLabel: 'PU', code: ESCAPE_SEQUENCES.pageUp },
  { label: 'PgDn', shortLabel: 'PD', code: ESCAPE_SEQUENCES.pageDown },
];

export const MODIFIER_ROW: KeyDefinition[] = [
  { label: 'Ctrl', code: '', width: 1.5, isModifier: true, modifierType: 'ctrl' },
  { label: 'Alt', code: '', width: 1.5, isModifier: true, modifierType: 'alt' },
  { label: 'Shift', code: '', width: 1.5, isModifier: true, modifierType: 'shift' },
  { label: 'Space', shortLabel: 'Spc', code: ' ', width: 1.5 },
  { label: 'Enter', shortLabel: '↵', code: '\r', width: 1.5 },
  { label: 'http://', code: 'http://', width: 1.5 },
  { label: 'https://', code: 'https://', width: 1.5 },
];

// Common Ctrl combinations
export const CTRL_SHORTCUTS: CtrlShortcut[] = [
  { label: 'B', code: 'b', tooltip: 'tmux prefix / move back' },
  { label: 'C', code: 'c', tooltip: 'Interrupt (SIGINT)' },
  { label: 'D', code: 'd', tooltip: 'EOF / Exit' },
  { label: 'Z', code: 'z', tooltip: 'Suspend (SIGTSTP)' },
  { label: 'L', code: 'l', tooltip: 'Clear screen' },
  { label: 'A', code: 'a', tooltip: 'Start of line' },
  { label: 'E', code: 'e', tooltip: 'End of line' },
  { label: 'K', code: 'k', tooltip: 'Kill line after cursor' },
  { label: 'U', code: 'u', tooltip: 'Kill line before cursor' },
  { label: 'W', code: 'w', tooltip: 'Delete word' },
  { label: 'R', code: 'r', tooltip: 'Reverse search history' },
  { label: 'Spc', code: '\x00', tooltip: 'Set mark / Autocomplete' },
];

// Symbol keys often hard to type on mobile
export const SYMBOL_ROW: KeyDefinition[] = [
  { label: '!', code: '!' },
  { label: '@', code: '@' },
  { label: '#', code: '#' },
  { label: '$', code: '$' },
  { label: '%', code: '%' },
  { label: '^', code: '^' },
  { label: '&', code: '&' },
  { label: '*', code: '*' },
  { label: '-', code: '-' },
  { label: '_', code: '_' },
  { label: '=', code: '=' },
  { label: '+', code: '+' },
  { label: '/', code: '/' },
  { label: '\\', code: '\\' },
  { label: '|', code: '|' },
  { label: '`', code: '`' },
  { label: '~', code: '~' },
  { label: '{', code: '{' },
  { label: '}', code: '}' },
  { label: '[', code: '[' },
  { label: ']', code: ']' },
  { label: '<', code: '<' },
  { label: '>', code: '>' },
  { label: '(', code: '(' },
  { label: ')', code: ')' },
  { label: ':', code: ':' },
  { label: ';', code: ';' },
  { label: "'", code: "'" },
  { label: '"', code: '"' },
  { label: ',', code: ',' },
  { label: '.', code: '.' },
  { label: '?', code: '?' },
];
