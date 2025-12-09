'use client';

import { useState, useCallback, useRef, useEffect } from 'react';

interface OnscreenKeyboardProps {
  onInput: (data: string) => void;
  isVisible: boolean;
  onToggleVisibility: () => void;
}

interface KeyDefinition {
  label: string;
  shortLabel?: string;
  code: string;
  width?: number; // width multiplier (1 = standard key width)
  isModifier?: boolean;
  modifierType?: 'ctrl' | 'alt' | 'shift';
}

// ANSI escape sequences for special keys
const ESCAPE_SEQUENCES = {
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
const getCtrlCode = (char: string): string => {
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
const getAltCode = (char: string): string => {
  return '\x1b' + char;
};

// Keyboard layout definitions
const FUNCTION_ROW: KeyDefinition[] = [
  { label: 'Esc', code: ESCAPE_SEQUENCES.escape },
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

const NAV_ROW: KeyDefinition[] = [
  { label: 'Esc', code: ESCAPE_SEQUENCES.escape },
  { label: 'Tab', code: ESCAPE_SEQUENCES.tab, width: 1.5 },
  { label: 'Ins', code: ESCAPE_SEQUENCES.insert },
  { label: 'Del', code: ESCAPE_SEQUENCES.delete },
  { label: 'Home', code: ESCAPE_SEQUENCES.home },
  { label: 'End', code: ESCAPE_SEQUENCES.end },
  { label: 'PgUp', shortLabel: 'PU', code: ESCAPE_SEQUENCES.pageUp },
  { label: 'PgDn', shortLabel: 'PD', code: ESCAPE_SEQUENCES.pageDown },
];

const MODIFIER_ROW: KeyDefinition[] = [
  { label: 'Ctrl', code: '', width: 1.5, isModifier: true, modifierType: 'ctrl' },
  { label: 'Alt', code: '', width: 1.5, isModifier: true, modifierType: 'alt' },
  { label: 'Shift', code: '', width: 1.5, isModifier: true, modifierType: 'shift' },
];

// Common Ctrl combinations
const CTRL_SHORTCUTS: KeyDefinition[] = [
  { label: 'B', code: 'b' },    // Ctrl+B (tmux prefix / move back)
  { label: 'C', code: 'c' },    // Ctrl+C (SIGINT)
  { label: 'D', code: 'd' },    // Ctrl+D (EOF)
  { label: 'Z', code: 'z' },    // Ctrl+Z (SIGTSTP)
  { label: 'L', code: 'l' },    // Ctrl+L (clear screen)
  { label: 'A', code: 'a' },    // Ctrl+A (start of line)
  { label: 'E', code: 'e' },    // Ctrl+E (end of line)
  { label: 'K', code: 'k' },    // Ctrl+K (kill line after cursor)
  { label: 'U', code: 'u' },    // Ctrl+U (kill line before cursor)
  { label: 'W', code: 'w' },    // Ctrl+W (delete word)
  { label: 'R', code: 'r' },    // Ctrl+R (reverse search)
  { label: 'Spc', code: '\x00' },  // Ctrl+Space (NUL - set mark/autocomplete)
];

// Quick insert text snippets
const QUICK_INSERT: KeyDefinition[] = [
  { label: 'http://', code: 'http://', width: 1.5 },
  { label: 'https://', code: 'https://', width: 1.5 },
];

// Symbol keys often hard to type on mobile
const SYMBOL_ROW: KeyDefinition[] = [
  { label: '/', code: '/' },
  { label: '|', code: '|' },
  { label: '\\', code: '\\' },
  { label: '`', code: '`' },
  { label: '~', code: '~' },
  { label: '{', code: '{' },
  { label: '}', code: '}' },
  { label: '[', code: '[' },
  { label: ']', code: ']' },
  { label: '<', code: '<' },
  { label: '>', code: '>' },
];

export function OnscreenKeyboard({ onInput, isVisible, onToggleVisibility }: OnscreenKeyboardProps) {
  const [ctrlActive, setCtrlActive] = useState(false);
  const [altActive, setAltActive] = useState(false);
  const [shiftActive, setShiftActive] = useState(false);
  const [showFunctionKeys, setShowFunctionKeys] = useState(false);
  const [showSymbols, setShowSymbols] = useState(false);
  const keyboardRef = useRef<HTMLDivElement>(null);

  // Send a key twice with a delay between
  const sendDoubleKey = useCallback((code: string, delayMs: number = 20) => {
    onInput(code);
    setTimeout(() => {
      onInput(code);
    }, delayMs);
  }, [onInput]);

  // Reset modifiers after a key press (except when sticky mode could be added)
  const resetModifiers = useCallback(() => {
    setCtrlActive(false);
    setAltActive(false);
    setShiftActive(false);
  }, []);

  const handleKeyPress = useCallback((key: KeyDefinition) => {
    if (key.isModifier) {
      // Toggle modifier state
      switch (key.modifierType) {
        case 'ctrl':
          setCtrlActive(prev => !prev);
          break;
        case 'alt':
          setAltActive(prev => !prev);
          break;
        case 'shift':
          setShiftActive(prev => !prev);
          break;
      }
      return;
    }

    let codeToSend = key.code;

    // Apply modifiers
    if (ctrlActive && key.code.length === 1) {
      codeToSend = getCtrlCode(key.code);
    } else if (altActive) {
      codeToSend = getAltCode(key.code);
    } else if (shiftActive && key.code.length === 1) {
      codeToSend = key.code.toUpperCase();
    }

    onInput(codeToSend);
    resetModifiers();
  }, [ctrlActive, altActive, shiftActive, onInput, resetModifiers]);

  // Handle touch/click with haptic feedback if available
  const handleInteraction = useCallback((key: KeyDefinition, e: React.MouseEvent | React.TouchEvent) => {
    e.preventDefault();
    e.stopPropagation();

    // Haptic feedback on supported devices
    if ('vibrate' in navigator) {
      navigator.vibrate(10);
    }

    handleKeyPress(key);
  }, [handleKeyPress]);

  // Prevent keyboard from stealing focus from terminal
  useEffect(() => {
    const keyboard = keyboardRef.current;
    if (!keyboard) return;

    const preventFocus = (e: Event) => {
      e.preventDefault();
    };

    keyboard.addEventListener('mousedown', preventFocus);
    keyboard.addEventListener('touchstart', preventFocus, { passive: false });

    return () => {
      keyboard.removeEventListener('mousedown', preventFocus);
      keyboard.removeEventListener('touchstart', preventFocus);
    };
  }, []);

  const renderKey = (key: KeyDefinition, index: number) => {
    const isActive = (key.modifierType === 'ctrl' && ctrlActive) ||
                    (key.modifierType === 'alt' && altActive) ||
                    (key.modifierType === 'shift' && shiftActive);

    const widthClass = key.width === 1.5 ? 'min-w-[3rem] sm:min-w-[3.5rem]' : 'min-w-[2rem] sm:min-w-[2.5rem]';

    return (
      <button
        key={`${key.label}-${index}`}
        className={`
          ${widthClass} h-9 sm:h-10 px-1.5 sm:px-2
          rounded-md text-xs sm:text-sm font-medium
          select-none touch-manipulation
          transition-all duration-100 active:scale-95
          ${isActive
            ? 'bg-blue-600/80 text-white border-blue-400/50 shadow-[0_0_10px_rgba(59,130,246,0.5)]'
            : 'bg-[#252525]/90 text-[#e0e0e0] border-[#3a3a3a]/50 hover:bg-[#353535]/90 active:bg-[#454545]/90'
          }
          border backdrop-blur-sm
        `}
        onMouseDown={(e) => handleInteraction(key, e)}
        onTouchStart={(e) => handleInteraction(key, e)}
        type="button"
      >
        <span className="hidden sm:inline">{key.label}</span>
        <span className="sm:hidden">{key.shortLabel || key.label}</span>
      </button>
    );
  };

  const renderArrowKeys = () => (
    <div className="flex flex-col items-center gap-0.5">
      <button
        className="w-10 h-7 sm:w-12 sm:h-8 rounded-md text-sm font-medium
          select-none touch-manipulation transition-all duration-100 active:scale-95
          bg-[#252525]/90 text-[#e0e0e0] border border-[#3a3a3a]/50
          hover:bg-[#353535]/90 active:bg-[#454545]/90 backdrop-blur-sm"
        onMouseDown={(e) => handleInteraction({ label: '▲', code: ESCAPE_SEQUENCES.arrowUp }, e)}
        onTouchStart={(e) => handleInteraction({ label: '▲', code: ESCAPE_SEQUENCES.arrowUp }, e)}
        type="button"
      >
        ▲
      </button>
      <div className="flex gap-0.5">
        <button
          className="w-10 h-7 sm:w-12 sm:h-8 rounded-md text-sm font-medium
            select-none touch-manipulation transition-all duration-100 active:scale-95
            bg-[#252525]/90 text-[#e0e0e0] border border-[#3a3a3a]/50
            hover:bg-[#353535]/90 active:bg-[#454545]/90 backdrop-blur-sm"
          onMouseDown={(e) => handleInteraction({ label: '◀', code: ESCAPE_SEQUENCES.arrowLeft }, e)}
          onTouchStart={(e) => handleInteraction({ label: '◀', code: ESCAPE_SEQUENCES.arrowLeft }, e)}
          type="button"
        >
          ◀
        </button>
        <button
          className="w-10 h-7 sm:w-12 sm:h-8 rounded-md text-sm font-medium
            select-none touch-manipulation transition-all duration-100 active:scale-95
            bg-[#252525]/90 text-[#e0e0e0] border border-[#3a3a3a]/50
            hover:bg-[#353535]/90 active:bg-[#454545]/90 backdrop-blur-sm"
          onMouseDown={(e) => handleInteraction({ label: '▼', code: ESCAPE_SEQUENCES.arrowDown }, e)}
          onTouchStart={(e) => handleInteraction({ label: '▼', code: ESCAPE_SEQUENCES.arrowDown }, e)}
          type="button"
        >
          ▼
        </button>
        <button
          className="w-10 h-7 sm:w-12 sm:h-8 rounded-md text-sm font-medium
            select-none touch-manipulation transition-all duration-100 active:scale-95
            bg-[#252525]/90 text-[#e0e0e0] border border-[#3a3a3a]/50
            hover:bg-[#353535]/90 active:bg-[#454545]/90 backdrop-blur-sm"
          onMouseDown={(e) => handleInteraction({ label: '▶', code: ESCAPE_SEQUENCES.arrowRight }, e)}
          onTouchStart={(e) => handleInteraction({ label: '▶', code: ESCAPE_SEQUENCES.arrowRight }, e)}
          type="button"
        >
          ▶
        </button>
      </div>
    </div>
  );

  const renderCtrlShortcuts = () => (
    <div className="flex flex-wrap gap-1 justify-center">
      {CTRL_SHORTCUTS.map((key, index) => (
        <button
          key={`ctrl-${key.label}-${index}`}
          className="min-w-[2.5rem] sm:min-w-[3rem] h-8 sm:h-9 px-2
            rounded-md text-xs sm:text-sm font-medium
            select-none touch-manipulation transition-all duration-100 active:scale-95
            bg-[#252525]/90 text-[#e0e0e0] border border-[#3a3a3a]/50
            hover:bg-[#353535]/90 active:bg-[#454545]/90 backdrop-blur-sm"
          onMouseDown={(e) => {
            e.preventDefault();
            e.stopPropagation();
            if ('vibrate' in navigator) navigator.vibrate(10);
            onInput(getCtrlCode(key.code));
          }}
          onTouchStart={(e) => {
            e.preventDefault();
            e.stopPropagation();
            if ('vibrate' in navigator) navigator.vibrate(10);
            onInput(getCtrlCode(key.code));
          }}
          type="button"
        >
          <span className="text-blue-400 text-[10px] sm:text-xs">^</span>{key.label}
        </button>
      ))}
      {/* Double Ctrl+C button */}
      <button
        key="ctrl-cc"
        className="min-w-[2.5rem] sm:min-w-[3rem] h-8 sm:h-9 px-2
          rounded-md text-xs sm:text-sm font-medium
          select-none touch-manipulation transition-all duration-100 active:scale-95
          bg-[#252525]/90 text-[#e0e0e0] border border-red-500/50
          hover:bg-[#353535]/90 active:bg-[#454545]/90 backdrop-blur-sm"
        onMouseDown={(e) => {
          e.preventDefault();
          e.stopPropagation();
          if ('vibrate' in navigator) navigator.vibrate(10);
          sendDoubleKey(getCtrlCode('c'), 20);
        }}
        onTouchStart={(e) => {
          e.preventDefault();
          e.stopPropagation();
          if ('vibrate' in navigator) navigator.vibrate(10);
          sendDoubleKey(getCtrlCode('c'), 20);
        }}
        type="button"
        title="Send Ctrl+C twice with 20ms delay"
      >
        <span className="text-red-400 text-[10px] sm:text-xs">^</span>c<span className="text-red-400 text-[10px] sm:text-xs">^</span>c
      </button>
      {/* Double Ctrl+D button */}
      <button
        key="ctrl-dd"
        className="min-w-[2.5rem] sm:min-w-[3rem] h-8 sm:h-9 px-2
          rounded-md text-xs sm:text-sm font-medium
          select-none touch-manipulation transition-all duration-100 active:scale-95
          bg-[#252525]/90 text-[#e0e0e0] border border-orange-500/50
          hover:bg-[#353535]/90 active:bg-[#454545]/90 backdrop-blur-sm"
        onMouseDown={(e) => {
          e.preventDefault();
          e.stopPropagation();
          if ('vibrate' in navigator) navigator.vibrate(10);
          sendDoubleKey(getCtrlCode('d'), 20);
        }}
        onTouchStart={(e) => {
          e.preventDefault();
          e.stopPropagation();
          if ('vibrate' in navigator) navigator.vibrate(10);
          sendDoubleKey(getCtrlCode('d'), 20);
        }}
        type="button"
        title="Send Ctrl+D twice with 20ms delay"
      >
        <span className="text-orange-400 text-[10px] sm:text-xs">^</span>d<span className="text-orange-400 text-[10px] sm:text-xs">^</span>d
      </button>
    </div>
  );

  if (!isVisible) {
    return (
      <button
        onClick={onToggleVisibility}
        className="fixed bottom-2 right-14 z-50 p-2 rounded-full
          bg-[#252525]/95 text-[#e0e0e0] border border-[#3a3a3a]/50
          backdrop-blur-md shadow-lg hover:bg-[#353535]/95
          transition-all duration-200 hover:scale-105
          touch-manipulation select-none"
        title="Show keyboard"
      >
        <svg xmlns="http://www.w3.org/2000/svg" className="w-6 h-6" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <rect x="2" y="4" width="20" height="16" rx="2" ry="2"/>
          <line x1="6" y1="8" x2="6" y2="8"/>
          <line x1="10" y1="8" x2="10" y2="8"/>
          <line x1="14" y1="8" x2="14" y2="8"/>
          <line x1="18" y1="8" x2="18" y2="8"/>
          <line x1="6" y1="12" x2="6" y2="12"/>
          <line x1="10" y1="12" x2="10" y2="12"/>
          <line x1="14" y1="12" x2="14" y2="12"/>
          <line x1="18" y1="12" x2="18" y2="12"/>
          <line x1="7" y1="16" x2="17" y2="16"/>
        </svg>
      </button>
    );
  }

  return (
    <div
      ref={keyboardRef}
      className="fixed bottom-0 left-0 right-0 z-50
        bg-[#1a1a1a]/95 backdrop-blur-xl
        border-t border-[#2a2a2a]/80
        shadow-[0_-4px_30px_rgba(0,0,0,0.5)]
        transition-transform duration-300 ease-out
        select-none"
    >
      {/* Header with mode toggles and close button */}
      <div className="flex items-center justify-between px-2 py-1.5 border-b border-[#2a2a2a]/50">
        <div className="flex gap-1">
          <button
            onClick={() => { setShowFunctionKeys(prev => !prev); setShowSymbols(false); }}
            className={`px-2.5 py-1 rounded text-xs font-medium transition-colors
              ${showFunctionKeys
                ? 'bg-blue-600/80 text-white'
                : 'bg-[#2a2a2a]/80 text-[#a0a0a0] hover:bg-[#3a3a3a]/80 hover:text-[#e0e0e0]'
              }`}
          >
            F1-F12
          </button>
          <button
            onClick={() => { setShowSymbols(prev => !prev); setShowFunctionKeys(false); }}
            className={`px-2.5 py-1 rounded text-xs font-medium transition-colors
              ${showSymbols
                ? 'bg-blue-600/80 text-white'
                : 'bg-[#2a2a2a]/80 text-[#a0a0a0] hover:bg-[#3a3a3a]/80 hover:text-[#e0e0e0]'
              }`}
          >
            Symbols
          </button>
        </div>

        {/* Modifier indicators */}
        <div className="flex gap-1.5 text-[10px] sm:text-xs">
          {ctrlActive && <span className="px-1.5 py-0.5 rounded bg-blue-600/60 text-white">CTRL</span>}
          {altActive && <span className="px-1.5 py-0.5 rounded bg-purple-600/60 text-white">ALT</span>}
          {shiftActive && <span className="px-1.5 py-0.5 rounded bg-green-600/60 text-white">SHIFT</span>}
        </div>

        <button
          onClick={onToggleVisibility}
          className="p-1.5 rounded-md text-[#808080] hover:text-[#e0e0e0]
            hover:bg-[#2a2a2a]/80 transition-colors"
          title="Hide keyboard"
        >
          <svg xmlns="http://www.w3.org/2000/svg" className="w-5 h-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <line x1="18" y1="6" x2="6" y2="18"/>
            <line x1="6" y1="6" x2="18" y2="18"/>
          </svg>
        </button>
      </div>

      {/* Keyboard content */}
      <div className="p-2 space-y-2">
        {/* Function keys row (conditional) */}
        {showFunctionKeys && (
          <div className="flex flex-wrap gap-1 justify-center pb-2 border-b border-[#2a2a2a]/30">
            {FUNCTION_ROW.map(renderKey)}
          </div>
        )}

        {/* Symbol keys row (conditional) */}
        {showSymbols && (
          <div className="flex flex-wrap gap-1 justify-center pb-2 border-b border-[#2a2a2a]/30">
            {SYMBOL_ROW.map(renderKey)}
          </div>
        )}

        {/* Main keyboard section */}
        <div className="flex flex-col sm:flex-row gap-2 sm:gap-4 items-center justify-center">
          {/* Left section: Navigation and modifiers */}
          <div className="flex flex-col gap-1.5">
            {/* Nav row */}
            <div className="flex flex-wrap gap-1 justify-center">
              {NAV_ROW.map(renderKey)}
            </div>

            {/* Modifier row */}
            <div className="flex gap-1 justify-center">
              {MODIFIER_ROW.map(renderKey)}
            </div>
          </div>

          {/* Right section: Arrow keys */}
          <div className="flex items-center">
            {renderArrowKeys()}
          </div>
        </div>

        {/* Ctrl shortcuts row */}
        <div className="pt-1.5 border-t border-[#2a2a2a]/30">
          <div className="text-[10px] text-[#606060] text-center mb-1">Quick Ctrl Shortcuts</div>
          {renderCtrlShortcuts()}
        </div>

        {/* Quick insert row */}
        <div className="pt-1.5 border-t border-[#2a2a2a]/30">
          <div className="text-[10px] text-[#606060] text-center mb-1">Quick Insert</div>
          <div className="flex flex-wrap gap-1 justify-center">
            {QUICK_INSERT.map((key, index) => (
              <button
                key={`insert-${key.label}-${index}`}
                className={`${key.width === 1.5 ? 'min-w-[4rem] sm:min-w-[5rem]' : 'min-w-[2.5rem] sm:min-w-[3rem]'} h-8 sm:h-9 px-2
                  rounded-md text-xs sm:text-sm font-medium
                  select-none touch-manipulation transition-all duration-100 active:scale-95
                  bg-[#252525]/90 text-[#e0e0e0] border border-emerald-500/50
                  hover:bg-[#353535]/90 active:bg-[#454545]/90 backdrop-blur-sm`}
                onMouseDown={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  if ('vibrate' in navigator) navigator.vibrate(10);
                  onInput(key.code);
                }}
                onTouchStart={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  if ('vibrate' in navigator) navigator.vibrate(10);
                  onInput(key.code);
                }}
                type="button"
              >
                <span className="text-emerald-400">{key.label}</span>
              </button>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
