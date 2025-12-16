'use client';

import { useState, useCallback, useRef, useEffect } from 'react';

interface OnscreenKeyboardProps {
  onInput: (data: string) => void;
  isVisible: boolean;
  onToggleVisibility: () => void;
  showControls?: boolean;
  onToggleControls?: () => void;
  fontSize?: number;
  onFontSizeChange?: (delta: number) => void;
  minFontSize?: number;
  maxFontSize?: number;
}

interface KeyDefinition {
  label: string;
  shortLabel?: string;
  code: string;
  width?: number; // width multiplier (1 = standard key width)
  isModifier?: boolean;
  modifierType?: 'ctrl' | 'alt' | 'shift';
}

interface Macro {
  id: string;
  name: string;
  script: string;
  sendEnter?: boolean; // Whether to send Enter after each line (default: true)
  isBuiltIn?: boolean; // Built-in macros cannot be edited/deleted
}

const MACROS_STORAGE_KEY = 'par-term-macros';
const MACRO_LINE_DELAY_MS = 200;


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
  { label: 'Space', shortLabel: 'Spc', code: ' ', width: 1.5 },
  { label: 'Enter', shortLabel: '↵', code: '\r', width: 1.5 },
  { label: 'http://', code: 'http://', width: 1.5 },
  { label: 'https://', code: 'https://', width: 1.5 },
];

// Common Ctrl combinations
interface CtrlShortcut extends KeyDefinition {
  tooltip: string;
}

const CTRL_SHORTCUTS: CtrlShortcut[] = [
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
const SYMBOL_ROW: KeyDefinition[] = [
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

export function OnscreenKeyboard({
  onInput,
  isVisible,
  onToggleVisibility,
  showControls,
  onToggleControls,
  fontSize,
  onFontSizeChange,
  minFontSize = 8,
  maxFontSize = 32,
}: OnscreenKeyboardProps) {
  const [ctrlActive, setCtrlActive] = useState(false);
  const [altActive, setAltActive] = useState(false);
  const [shiftActive, setShiftActive] = useState(false);
  const [showFunctionKeys, setShowFunctionKeys] = useState(false);
  const [showMacros, setShowMacros] = useState(false);
  const [macros, setMacros] = useState<Macro[]>([]);
  const [showMacroEditor, setShowMacroEditor] = useState(false);
  const [editingMacro, setEditingMacro] = useState<Macro | null>(null);
  const [macroName, setMacroName] = useState('');
  const [macroScript, setMacroScript] = useState('');
  const [macroSendEnter, setMacroSendEnter] = useState(true);
  const [playingMacro, setPlayingMacro] = useState<string | null>(null);
  const keyboardRef = useRef<HTMLDivElement>(null);
  const macroAbortRef = useRef<boolean>(false);

  // Load macros from localStorage on mount
  useEffect(() => {
    try {
      const stored = localStorage.getItem(MACROS_STORAGE_KEY);
      if (stored) {
        setMacros(JSON.parse(stored));
      }
    } catch (e) {
      console.error('Failed to load macros:', e);
    }
  }, []);

  // Save macros to localStorage when changed
  const saveMacros = useCallback((newMacros: Macro[]) => {
    setMacros(newMacros);
    try {
      localStorage.setItem(MACROS_STORAGE_KEY, JSON.stringify(newMacros));
    } catch (e) {
      console.error('Failed to save macros:', e);
    }
  }, []);

  // Process macro template commands in a line
  // Returns the processed text (with templates replaced) and any delay to apply
  const processMacroLine = useCallback((line: string): { text: string; delayMs: number } => {
    let text = line;
    let delayMs = 0;

    // Check for [[delay:X]] pattern (X in seconds)
    const delayMatch = text.match(/\[\[delay:([\d.]+)\]\]/i);
    if (delayMatch) {
      delayMs = parseFloat(delayMatch[1]) * 1000;
      text = text.replace(delayMatch[0], '');
    }

    // Replace [[enter]] with carriage return
    text = text.replace(/\[\[enter\]\]/gi, '\r');

    // Replace [[shift+enter]] with carriage return (same as enter in most terminals)
    text = text.replace(/\[\[shift\+enter\]\]/gi, '\r');

    // Replace [[shift+tab]] with reverse tab (backtab)
    text = text.replace(/\[\[shift\+tab\]\]/gi, '\x1b[Z');

    // Replace [[tab]] with tab character
    text = text.replace(/\[\[tab\]\]/gi, '\t');

    // Replace [[esc]] or [[escape]] with escape character
    text = text.replace(/\[\[esc(?:ape)?\]\]/gi, '\x1b');

    // Replace [[ctrl+shift+X]] or [[shift+ctrl+X]] with control character (same as ctrl+X in most terminals)
    text = text.replace(/\[\[(?:ctrl\+shift|shift\+ctrl)\+([a-z])\]\]/gi, (_, char) => {
      return String.fromCharCode(char.toUpperCase().charCodeAt(0) - 64);
    });

    // Replace [[ctrl+X]] with control character
    text = text.replace(/\[\[ctrl\+([a-z])\]\]/gi, (_, char) => {
      return String.fromCharCode(char.toUpperCase().charCodeAt(0) - 64);
    });

    // Replace [[shift+X]] with uppercase character
    text = text.replace(/\[\[shift\+([a-z])\]\]/gi, (_, char) => {
      return char.toUpperCase();
    });

    // Replace [[space]] with space
    text = text.replace(/\[\[space\]\]/gi, ' ');

    return { text, delayMs };
  }, []);

  // Play a macro with delay between lines
  const playMacro = useCallback(async (macro: Macro) => {
    if (playingMacro) return; // Already playing

    const shouldSendEnter = macro.sendEnter !== false; // Default to true

    // For quick insert (no enter), just send immediately
    if (!shouldSendEnter) {
      onInput(macro.script);
      return;
    }

    macroAbortRef.current = false;
    setPlayingMacro(macro.id);

    const lines = macro.script.split('\n').filter(line => line.length > 0);

    for (let i = 0; i < lines.length; i++) {
      if (macroAbortRef.current) break;

      const line = lines[i];

      // Process template commands
      const { text, delayMs } = processMacroLine(line);

      // Apply any delay from the template
      if (delayMs > 0) {
        await new Promise(resolve => setTimeout(resolve, delayMs));
        if (macroAbortRef.current) break;
      }

      // Skip empty lines after template processing
      if (!text) continue;

      // Send the line content
      onInput(text);

      // Wait before sending enter
      await new Promise(resolve => setTimeout(resolve, MACRO_LINE_DELAY_MS));

      if (macroAbortRef.current) break;

      // Send enter key
      onInput('\r');

      // Wait before next line (if not last line)
      if (i < lines.length - 1) {
        await new Promise(resolve => setTimeout(resolve, MACRO_LINE_DELAY_MS));
      }
    }

    setPlayingMacro(null);
  }, [playingMacro, onInput, processMacroLine]);

  // Stop macro playback
  const stopMacro = useCallback(() => {
    macroAbortRef.current = true;
    setPlayingMacro(null);
  }, []);

  // Save or update a macro
  const saveMacro = useCallback(() => {
    if (!macroName.trim() || !macroScript.trim()) return;

    if (editingMacro) {
      // Update existing macro
      const updated = macros.map(m =>
        m.id === editingMacro.id
          ? { ...m, name: macroName.trim(), script: macroScript, sendEnter: macroSendEnter }
          : m
      );
      saveMacros(updated);
    } else {
      // Create new macro
      const newMacro: Macro = {
        id: Date.now().toString(),
        name: macroName.trim(),
        script: macroScript,
        sendEnter: macroSendEnter,
      };
      saveMacros([...macros, newMacro]);
    }

    // Reset editor state
    setShowMacroEditor(false);
    setEditingMacro(null);
    setMacroName('');
    setMacroSendEnter(true);
    setMacroScript('');
  }, [macroName, macroScript, macroSendEnter, editingMacro, macros, saveMacros]);

  // Delete a macro
  const deleteMacro = useCallback((id: string) => {
    saveMacros(macros.filter(m => m.id !== id));
    if (editingMacro?.id === id) {
      setShowMacroEditor(false);
      setEditingMacro(null);
      setMacroName('');
      setMacroScript('');
    }
  }, [macros, editingMacro, saveMacros]);

  // Open macro editor for editing
  const editMacro = useCallback((macro: Macro) => {
    setEditingMacro(macro);
    setMacroName(macro.name);
    setMacroScript(macro.script);
    setMacroSendEnter(macro.sendEnter !== false);
    setShowMacroEditor(true);
  }, []);

  // Open macro editor for new macro
  const newMacro = useCallback(() => {
    setEditingMacro(null);
    setMacroName('');
    setMacroScript('');
    setMacroSendEnter(true);
    setShowMacroEditor(true);
  }, []);

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
        tabIndex={-1}
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
        tabIndex={-1}
        className="w-10 h-[30px] sm:w-12 sm:h-[34px] rounded-md text-sm font-medium
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
          tabIndex={-1}
          className="w-10 h-[30px] sm:w-12 sm:h-[34px] rounded-md text-sm font-medium
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
          tabIndex={-1}
          className="w-10 h-[30px] sm:w-12 sm:h-[34px] rounded-md text-sm font-medium
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
          tabIndex={-1}
          className="w-10 h-[30px] sm:w-12 sm:h-[34px] rounded-md text-sm font-medium
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
          tabIndex={-1}
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
          title={key.tooltip}
        >
          <span className="text-blue-400 text-[10px] sm:text-xs">^</span>{key.label}
        </button>
      ))}
      {/* Double Ctrl+C button */}
      <button
        key="ctrl-cc"
        tabIndex={-1}
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
        tabIndex={-1}
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
        tabIndex={-1}
        className="fixed bottom-2 right-[71px] z-50 p-2 rounded-full
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
            onClick={() => { setShowFunctionKeys(prev => !prev); setShowMacros(false); }}
            tabIndex={-1}
            className={`px-2.5 py-1 rounded text-xs font-medium transition-colors
              ${showFunctionKeys
                ? 'bg-blue-600/80 text-white'
                : 'bg-[#2a2a2a]/80 text-[#a0a0a0] hover:bg-[#3a3a3a]/80 hover:text-[#e0e0e0]'
              }`}
          >
            F1-F12
          </button>
          <button
            onClick={() => { setShowMacros(prev => !prev); setShowFunctionKeys(false); }}
            tabIndex={-1}
            className={`px-2.5 py-1 rounded text-xs font-medium transition-colors
              ${showMacros
                ? 'bg-amber-600/80 text-white'
                : 'bg-[#2a2a2a]/80 text-[#a0a0a0] hover:bg-[#3a3a3a]/80 hover:text-[#e0e0e0]'
              }`}
          >
            Macros
          </button>
        </div>

        {/* Center section: Modifier indicators and font size */}
        <div className="flex items-center gap-3">
          {/* Modifier indicators */}
          <div className="flex gap-1.5 text-[10px] sm:text-xs">
            {ctrlActive && <span className="px-1.5 py-0.5 rounded bg-blue-600/60 text-white">CTRL</span>}
            {altActive && <span className="px-1.5 py-0.5 rounded bg-purple-600/60 text-white">ALT</span>}
            {shiftActive && <span className="px-1.5 py-0.5 rounded bg-green-600/60 text-white">SHIFT</span>}
          </div>

          {/* Font size controls */}
          {fontSize !== undefined && onFontSizeChange && (
            <div className="flex items-center gap-0.5">
              <button
                onClick={() => onFontSizeChange(-1)}
                disabled={fontSize <= minFontSize}
                tabIndex={-1}
                className="p-1 rounded bg-[#2a2a2a]/80 hover:bg-[#3a3a3a]/80 text-[#a0a0a0] hover:text-[#e0e0e0]
                  disabled:opacity-30 disabled:cursor-not-allowed transition-colors"
                title="Decrease font size"
              >
                <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M20 12H4" />
                </svg>
              </button>
              <span className="text-[10px] font-mono text-[#a0a0a0] min-w-[2rem] text-center" title="Font size">
                {fontSize}px
              </span>
              <button
                onClick={() => onFontSizeChange(1)}
                disabled={fontSize >= maxFontSize}
                tabIndex={-1}
                className="p-1 rounded bg-[#2a2a2a]/80 hover:bg-[#3a3a3a]/80 text-[#a0a0a0] hover:text-[#e0e0e0]
                  disabled:opacity-30 disabled:cursor-not-allowed transition-colors"
                title="Increase font size"
              >
                <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4v16m8-8H4" />
                </svg>
              </button>
            </div>
          )}
        </div>

        <div className="flex items-center gap-1">
          {/* Header/Footer toggle button */}
          {onToggleControls && (
            <button
              onClick={onToggleControls}
              tabIndex={-1}
              className={`p-1.5 rounded-md transition-colors ${
                showControls
                  ? 'text-blue-400 hover:text-blue-300 hover:bg-blue-500/20'
                  : 'text-[#808080] hover:text-[#e0e0e0] hover:bg-[#2a2a2a]/80'
              }`}
              title={showControls ? 'Hide header/footer' : 'Show header/footer'}
            >
              <svg xmlns="http://www.w3.org/2000/svg" className="w-5 h-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                {/* Layout/panels icon */}
                <rect x="3" y="3" width="18" height="18" rx="2" ry="2"/>
                <line x1="3" y1="9" x2="21" y2="9"/>
                <line x1="3" y1="15" x2="21" y2="15"/>
              </svg>
            </button>
          )}

          {/* Close keyboard button */}
          <button
            onClick={onToggleVisibility}
            tabIndex={-1}
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
      </div>

      {/* Keyboard content */}
      <div className="p-2">
        {/* Macros section (conditional) - takes full keyboard space */}
        {showMacros ? (
          <div className="space-y-3">
            {/* Macro Editor */}
            {showMacroEditor ? (
              <div className="flex gap-3">
                {/* Editor panel */}
                <div className="flex-1 space-y-2 p-3 bg-[#1f1f1f]/80 rounded-lg">
                  <div className="flex items-center gap-2">
                    <input
                      type="text"
                      value={macroName}
                      onChange={(e) => setMacroName(e.target.value)}
                      placeholder="Macro name..."
                      className="flex-1 px-2 py-1.5 rounded text-sm bg-[#2a2a2a] text-[#e0e0e0]
                        border border-[#3a3a3a] focus:border-amber-500/50 focus:outline-none
                        placeholder-[#606060]"
                      maxLength={30}
                    />
                  </div>
                  <textarea
                    value={macroScript}
                    onChange={(e) => setMacroScript(e.target.value)}
                    placeholder="Enter commands (one per line)..."
                    className="w-full px-2 py-1.5 rounded text-sm bg-[#2a2a2a] text-[#e0e0e0]
                      border border-[#3a3a3a] focus:border-amber-500/50 focus:outline-none
                      placeholder-[#606060] resize-none font-mono"
                    rows={5}
                  />
                  <div className="flex items-center justify-between">
                    <label className="flex items-center gap-2 text-xs text-[#a0a0a0] cursor-pointer select-none">
                      <input
                        type="checkbox"
                        checked={macroSendEnter}
                        onChange={(e) => setMacroSendEnter(e.target.checked)}
                        className="w-4 h-4 rounded border-[#3a3a3a] bg-[#2a2a2a] text-amber-500
                          focus:ring-amber-500/50 focus:ring-offset-0"
                      />
                      Send Enter after each line
                    </label>
                    <div className="flex gap-2">
                      <button
                        onClick={() => {
                          setShowMacroEditor(false);
                          setEditingMacro(null);
                          setMacroName('');
                          setMacroScript('');
                          setMacroSendEnter(true);
                        }}
                        tabIndex={-1}
                        className="px-3 py-1.5 rounded text-xs font-medium
                          bg-[#2a2a2a]/80 text-[#a0a0a0] hover:bg-[#3a3a3a]/80 hover:text-[#e0e0e0]
                          transition-colors"
                      >
                        Cancel
                      </button>
                      <button
                        onClick={saveMacro}
                        tabIndex={-1}
                        disabled={!macroName.trim() || !macroScript.trim()}
                        className="px-3 py-1.5 rounded text-xs font-medium
                          bg-amber-600/80 text-white hover:bg-amber-500/80
                          disabled:opacity-50 disabled:cursor-not-allowed
                          transition-colors"
                      >
                        {editingMacro ? 'Update' : 'Save'}
                      </button>
                    </div>
                  </div>
                </div>

                {/* Help panel */}
                <div className="w-56 p-2 bg-[#1a1a1a]/80 rounded-lg border border-[#2a2a2a] text-[10px] text-[#808080]">
                  <div className="text-[11px] text-[#a0a0a0] font-medium mb-1.5">Template Commands</div>
                  <div className="space-y-0.5">
                    <div><code className="text-amber-400">[[delay:N]]</code> Wait N seconds</div>
                    <div><code className="text-amber-400">[[enter]]</code> Send Enter key</div>
                    <div><code className="text-amber-400">[[tab]]</code> Send Tab key</div>
                    <div><code className="text-amber-400">[[esc]]</code> Send Escape key</div>
                    <div><code className="text-amber-400">[[space]]</code> Send Space</div>
                    <div><code className="text-amber-400">[[ctrl+X]]</code> Send Ctrl+X</div>
                    <div><code className="text-amber-400">[[shift+X]]</code> Send Shift+X</div>
                    <div><code className="text-amber-400">[[ctrl+shift+X]]</code> Ctrl+Shift+X</div>
                    <div><code className="text-amber-400">[[shift+tab]]</code> Reverse Tab</div>
                    <div><code className="text-amber-400">[[shift+enter]]</code> Shift+Enter</div>
                  </div>
                </div>
              </div>
            ) : (
              <>
                {/* User macros */}
                <div>
                  <div className="flex flex-wrap gap-1.5 justify-center items-center">
                    {/* New macro button */}
                    <button
                      onClick={newMacro}
                      tabIndex={-1}
                      className="h-9 px-3 rounded-md text-xs sm:text-sm font-medium
                        select-none touch-manipulation transition-all duration-100
                        bg-amber-600/20 text-amber-400 border border-amber-500/50
                        hover:bg-amber-600/30 active:scale-95"
                      title="Create new macro"
                    >
                      + New
                    </button>

                    {/* Existing user macro buttons */}
                    {macros.map((macro) => (
                      <div key={macro.id} className="relative group">
                        <button
                          onClick={() => playingMacro === macro.id ? stopMacro() : playMacro(macro)}
                          tabIndex={-1}
                          className={`h-9 px-3 rounded-md text-xs sm:text-sm font-medium
                            select-none touch-manipulation transition-all duration-100
                            ${playingMacro === macro.id
                              ? 'bg-red-600/80 text-white border-red-400/50 animate-pulse'
                              : 'bg-[#252525]/90 text-amber-400 border-amber-500/30 hover:bg-[#353535]/90'
                            }
                            border active:scale-95`}
                          title={playingMacro === macro.id ? 'Stop macro' : `Run: ${macro.script.split('\n')[0]}${macro.sendEnter === false ? '' : '...'}${macro.sendEnter === false ? ' (no enter)' : ''}`}
                        >
                          {playingMacro === macro.id ? (
                            <span className="flex items-center gap-1">
                              <svg className="w-3 h-3" fill="currentColor" viewBox="0 0 24 24">
                                <rect x="6" y="6" width="12" height="12" />
                              </svg>
                              Stop
                            </span>
                          ) : (
                            <span className="flex items-center gap-1">
                              {macro.sendEnter !== false && (
                                <svg className="w-3 h-3" fill="currentColor" viewBox="0 0 24 24">
                                  <polygon points="5,3 19,12 5,21" />
                                </svg>
                              )}
                              {macro.name}
                            </span>
                          )}
                        </button>

                        {/* Edit/Delete dropdown on hover */}
                        {playingMacro !== macro.id && (
                          <div className="absolute right-0 bottom-full pb-1 hidden group-hover:flex z-10">
                            {/* Invisible bridge to prevent hover gap */}
                            <div className="flex bg-[#2a2a2a] rounded shadow-lg border border-[#3a3a3a] overflow-hidden">
                              <button
                                onClick={(e) => { e.stopPropagation(); editMacro(macro); }}
                                tabIndex={-1}
                                className="px-2 py-1 text-xs text-[#a0a0a0] hover:bg-[#3a3a3a] hover:text-[#e0e0e0]"
                                title="Edit macro"
                              >
                                Edit
                              </button>
                              <button
                                onClick={(e) => { e.stopPropagation(); deleteMacro(macro.id); }}
                                tabIndex={-1}
                                className="px-2 py-1 text-xs text-red-400 hover:bg-red-500/20"
                                title="Delete macro"
                              >
                                Del
                              </button>
                            </div>
                          </div>
                        )}
                      </div>
                    ))}

                    {macros.length === 0 && (
                      <span className="text-[10px] text-[#606060]">
                        No macros yet. Click &quot;+ New&quot; to create one.
                      </span>
                    )}
                  </div>
                </div>
              </>
            )}
          </div>
        ) : (
          <>
            {/* Main keyboard section with symbols grid */}
            <div className="flex gap-3 items-end justify-center">
              {/* Left section: Main keyboard controls */}
              <div className="space-y-2">
                {/* Function keys row (conditional) */}
                {showFunctionKeys && (
                  <div className="flex flex-wrap gap-1 justify-center pb-1 border-b border-[#2a2a2a]/30">
                    {FUNCTION_ROW.map(renderKey)}
                  </div>
                )}

                <div className="flex flex-col sm:flex-row gap-2 sm:gap-4 items-center justify-center">
                  {/* Navigation and modifiers */}
                  <div className="flex flex-col gap-0.5">
                    {/* Nav row */}
                    <div className="flex flex-wrap gap-1 justify-center">
                      {NAV_ROW.map(renderKey)}
                    </div>

                    {/* Modifier row */}
                    <div className="flex gap-1 justify-center">
                      {MODIFIER_ROW.map(renderKey)}
                    </div>
                  </div>

                  {/* Arrow keys */}
                  <div className="flex items-center">
                    {renderArrowKeys()}
                  </div>
                </div>

                {/* Ctrl shortcuts row */}
                <div className="pt-[3px] border-t border-[#2a2a2a]/30">
                  {renderCtrlShortcuts()}
                </div>
              </div>

              {/* Right section: Symbols grid */}
              <div className="flex flex-col">
                <div className="grid grid-cols-8 gap-0.5">
                  {SYMBOL_ROW.map((key, index) => (
                    <button
                      key={`sym-${key.label}-${index}`}
                      tabIndex={-1}
                      className="w-8 h-8 sm:w-9 sm:h-9 rounded text-xs sm:text-sm font-medium
                        select-none touch-manipulation transition-all duration-100 active:scale-95
                        bg-[#252525]/90 text-[#e0e0e0] border border-[#3a3a3a]/50
                        hover:bg-[#353535]/90 active:bg-[#454545]/90"
                      onMouseDown={(e) => {
                        e.preventDefault();
                        e.stopPropagation();
                        if ('vibrate' in navigator) navigator.vibrate(10);
                        handleKeyPress(key);
                      }}
                      onTouchStart={(e) => {
                        e.preventDefault();
                        e.stopPropagation();
                        if ('vibrate' in navigator) navigator.vibrate(10);
                        handleKeyPress(key);
                      }}
                      type="button"
                    >
                      {key.label}
                    </button>
                  ))}
                </div>
              </div>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
