'use client';

import { useEffect, useRef, useCallback } from 'react';
import { Terminal as XTerm } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebglAddon } from '@xterm/addon-webgl';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { Unicode11Addon } from '@xterm/addon-unicode11';
import { ImageAddon } from '@xterm/addon-image';
import type { ConnectionStatus } from '@/types/terminal';
import {
  createInputMessage,
  createResizeMessage,
  createRefreshMessage,
  createMouseMessage,
  createFocusMessage,
  createPasteMessage,
  themeToXtermOptions,
} from '@/lib/protocol';
import { TerminalConnection } from '@/lib/terminal-connection';
import {
  clamp,
  clampPan,
  fitWidthFontSize,
  pinchPan,
  surplusStrips,
  MAX_ZOOM_FONT_SIZE,
  MIN_ZOOM_FONT_SIZE,
  type Point,
  type Rect,
} from '@/lib/viewport';

interface TerminalProps {
  wsUrl: string;
  fontSize?: number;
  onStatusChange?: (status: ConnectionStatus) => void;
  onThemeChange?: (backgroundColor: string) => void;
  // `resizePane: true` also sends the fitted size to the server. Phones only
  // resize the shared pane on that deliberate request (see `sendsResizeOnFit`).
  onRefit?: (refitFn: (opts?: { resizePane?: boolean }) => void) => void;
  onFocus?: (focusFn: () => void) => void;
  onRetryingChange?: (isRetrying: boolean) => void;
  onConnectControl?: (control: { connect: () => void; disconnect: () => void; cancelRetry: () => void }) => void;
  onSendInput?: (sendFn: (data: string) => void) => void;
  onHyperlinkAdded?: (url: string, row: number, col: number, id?: string) => void;
  onUserVarChanged?: (name: string, value: string, oldValue?: string) => void;
  onSelectionChanged?: (text: string | undefined, cleared: boolean) => void;
}

// Module-level storage to preserve terminal across StrictMode unmount/remount
// StrictMode: mount -> cleanup -> remount. We keep the terminal alive during the brief cleanup.
let preservedTerminal: XTerm | null = null;
let preservedFitAddon: FitAddon | null = null;

// Detect if running on mobile device
const isMobile = (): boolean => {
  if (typeof window === 'undefined') return false;
  return window.innerWidth < 640 || /Android|webOS|iPhone|iPad|iPod|BlackBerry|IEMobile|Opera Mini/i.test(navigator.userAgent);
};

// Whether fitting xterm to its container also resizes the server-side
// terminal. A shared pane has one size and the latest resize wins, so a phone
// that sent its size on every connect or rotation would steal the pane from
// the desktop just by looking. Phones send a size only on a deliberate action
// (the Fit button, or opening the keyboard: `onRefit` with `resizePane`).
const sendsResizeOnFit = (): boolean => !isMobile();

// Resize xterm to a size the server reported, without echoing it back.
const applyServerSize = (
  term: XTerm,
  cols: number,
  rows: number,
  applying: { current: boolean },
): void => {
  if (cols <= 0 || rows <= 0 || (term.cols === cols && term.rows === rows)) return;
  applying.current = true;
  try {
    term.resize(cols, rows);
  } finally {
    applying.current = false;
  }
};

// xterm's scrollbar overlays the right edge of the viewport (FitAddon reserves
// this width when scrollback is on), so it is not surplus to dot-fill.
const SCROLLBAR_RESERVE_PX = 14;

// Show a strip of dim dots (tmux's unused-area fill) aligned to the grid's
// cells, or hide the element when there is no strip.
const paintDots = (
  el: HTMLDivElement | null,
  strip: Rect | null,
  grid: Rect,
  cellW: number,
  cellH: number,
): void => {
  if (!el) return;
  if (!strip || cellW <= 0 || cellH <= 0) {
    el.style.display = 'none';
    return;
  }
  el.style.display = 'block';
  el.style.left = `${strip.left}px`;
  el.style.top = `${strip.top}px`;
  el.style.width = `${strip.width}px`;
  el.style.height = `${strip.height}px`;
  el.style.backgroundSize = `${cellW}px ${cellH}px`;
  el.style.backgroundPosition = `${grid.left - strip.left}px ${grid.top - strip.top}px`;
};

// Get responsive font size based on screen dimensions
const getResponsiveFontSize = (): number => {
  if (typeof window === 'undefined') return 14;
  const width = window.innerWidth;
  const height = window.innerHeight;
  // Use smaller dimension to detect mobile in any orientation
  const minDim = Math.min(width, height);

  // Mobile device detection by smaller dimension
  if (minDim < 500) {
    // Phone in any orientation - use height-based sizing for landscape
    if (height < width) {
      // Landscape - limited height, use smaller font
      return height < 400 ? 4 : 5;
    }
    // Portrait
    return 4;
  }
  if (minDim < 768) return 6;    // Small tablets
  if (width < 1024) return 10;   // Tablets
  return 14;                      // Desktop
};

// Debug-only logger - gated on NODE_ENV so verbose connection/lifecycle
// logs don't ship to production consoles. console.error/console.warn calls
// are left ungated since they surface real diagnostics.
const debugLog = (...args: unknown[]): void => {
  if (process.env.NODE_ENV !== 'production') {
    console.log(...args);
  }
};

// Shared TextDecoder instance - reuse instead of creating per message
const sharedDecoder = new TextDecoder();

// Maximum snapshot size (1MB) to prevent UI freeze from large payloads
const MAX_SNAPSHOT_SIZE = 1024 * 1024;

export default function Terminal({ wsUrl, fontSize, onStatusChange, onThemeChange, onRefit, onFocus, onRetryingChange, onConnectControl, onSendInput, onHyperlinkAdded, onUserVarChanged, onSelectionChanged }: TerminalProps) {
  const terminalRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const xtermRef = useRef<XTerm | null>(null);
  const connectionRef = useRef<TerminalConnection | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);

  // RAF-batched write buffer for performance optimization
  // Instead of writing to terminal on every WebSocket message, we buffer
  // writes and flush once per animation frame (60fps = 16ms batches)
  const writeBufferRef = useRef<string[]>([]);
  const rafIdRef = useRef<number | null>(null);

  // Local echo (predictive input) for perceived latency reduction
  // Tracks characters we've echoed locally before server confirmation
  // so we can filter them from server output to avoid double display
  const pendingEchoRef = useRef<string[]>([]);
  const localEchoEnabledRef = useRef<boolean>(true);

  // Reconnect/backoff and heartbeat/stale-pong state live inside
  // TerminalConnection (QA-008).

  // Viewport over a pane this viewer does not size (see lib/viewport.ts).
  // On a phone, `zoomFontRef` is the committed zoom as a font size, or null
  // to fit the pane's width; `panRef` offsets the grid inside the view.
  const zoomFontRef = useRef<number | null>(null);
  const panRef = useRef<Point>({ x: 0, y: 0 });
  const connectedOnceRef = useRef<boolean>(false);
  const scheduleLayoutRef = useRef<() => void>(() => {});
  const rightDotsRef = useRef<HTMLDivElement>(null);
  const bottomDotsRef = useRef<HTMLDivElement>(null);

  // Set while applying a server-sent size, so `term.onResize` does not echo
  // it back (an echo can undo a newer resize from another client).
  const applyingServerSizeRef = useRef<boolean>(false);

  // Terminal mode tracking (from server modeChanged messages)
  const mouseTrackingRef = useRef<boolean>(false);
  const focusTrackingRef = useRef<boolean>(false);
  const bracketedPasteRef = useRef<boolean>(false);

  // Track hyperlinks: Map<row, {url, col, id}[]>
  const hyperlinksRef = useRef<Map<number, Array<{url: string; col: number; id?: string}>>>(new Map());
  // Track user vars: Map<name, value>
  const userVarsRef = useRef<Map<string, string>>(new Map());

  // Track previous wsUrl to detect changes
  const prevWsUrlRef = useRef<string>(wsUrl);

  // Track fontSize prop for use in handlers
  const fontSizeRef = useRef<number | undefined>(fontSize);

  // Latest callback props, updated every render (see the effect below).
  // The mount effect and `connect` below only run once (or on `wsUrl`
  // change) and must not close over the props from that one render, or a
  // later prop update (e.g. a new `onFocus`/`onRefit` from the parent)
  // would be silently ignored until the next full remount.
  const propsRef = useRef({
    onFocus,
    onRefit,
    onSendInput,
    onThemeChange,
    onStatusChange,
    onHyperlinkAdded,
    onSelectionChanged,
    onUserVarChanged,
  });
  useEffect(() => {
    propsRef.current = {
      onFocus,
      onRefit,
      onSendInput,
      onThemeChange,
      onStatusChange,
      onHyperlinkAdded,
      onSelectionChanged,
      onUserVarChanged,
    };
  });

  // Flush buffered writes to terminal - called once per animation frame
  const flushWrites = useCallback(() => {
    if (writeBufferRef.current.length > 0 && xtermRef.current) {
      // Join all buffered data and write once
      xtermRef.current.write(writeBufferRef.current.join(''));
      writeBufferRef.current = [];
    }
    rafIdRef.current = null;
  }, []);

  // Buffer a write and schedule RAF flush if not already scheduled
  // Also filters out locally echoed characters to avoid double display
  const bufferWrite = useCallback((data: string) => {
    let filteredData = data;

    // Filter out characters we already echoed locally
    // This reconciles local echo with server output
    while (pendingEchoRef.current.length > 0 && filteredData.length > 0) {
      const expected = pendingEchoRef.current[0];
      if (filteredData.startsWith(expected)) {
        // Server confirmed our local echo, remove from pending
        filteredData = filteredData.slice(expected.length);
        pendingEchoRef.current.shift();
      } else {
        // Mismatch - server sent something different (tab completion, etc.)
        // Clear pending echo and show full output
        pendingEchoRef.current = [];
        break;
      }
    }

    if (filteredData.length > 0) {
      writeBufferRef.current.push(filteredData);
      if (!rafIdRef.current) {
        rafIdRef.current = requestAnimationFrame(flushWrites);
      }
    }
  }, [flushWrites]);

  // Apply theme to terminal (using protobuf ThemeInfo)
  const applyTheme = (theme: { name: string; background?: { r: number; g: number; b: number }; foreground?: { r: number; g: number; b: number }; normal: { r: number; g: number; b: number }[]; bright: { r: number; g: number; b: number }[] }) => {
    if (!xtermRef.current) return;

    debugLog('Applying theme:', theme.name);

    const xtermTheme = themeToXtermOptions(theme as import('@/lib/protocol').ThemeInfo);

    // Update xterm.js theme
    xtermRef.current.options.theme = xtermTheme;

    const bgHex = xtermTheme.background || '#000000';

    // Update container background color
    if (containerRef.current) {
      containerRef.current.style.backgroundColor = bgHex;
    }

    // Notify parent component of background color change. Read from
    // propsRef (not the `onThemeChange` closure argument) since this
    // function is invoked from inside the memoized `connect` callback,
    // which must not need to be recreated every time the parent passes a
    // new callback instance.
    propsRef.current.onThemeChange?.(bgHex);
  };

  useEffect(() => {
    if (!terminalRef.current) return;

    // If already initialized and terminal exists, don't reinitialize
    if (xtermRef.current) return;

    let term: XTerm;
    let fitAddon: FitAddon;

    // Handle React StrictMode double-invocation:
    // StrictMode: mount -> cleanup -> remount. Restore preserved terminal on remount.
    if (preservedTerminal && preservedFitAddon) {
      debugLog('Restoring preserved terminal (StrictMode remount)');
      term = preservedTerminal;
      fitAddon = preservedFitAddon;

      // Clear preserved refs now that we've restored
      preservedTerminal = null;
      preservedFitAddon = null;
    } else {
      // Fresh initialization
      const mobile = isMobile();
      const initialFontSize = fontSizeRef.current ?? getResponsiveFontSize();
      debugLog(`Terminal init: width=${window.innerWidth}, mobile=${mobile}, fontSize=${initialFontSize}`);

      // Initialize xterm.js
      term = new XTerm({
        cursorBlink: !mobile,
        fontSize: initialFontSize,
        fontFamily: "'Symbols Nerd Font', 'JetBrains Mono', 'Fira Code', 'ui-monospace', 'SFMono-Regular', 'Menlo', 'Monaco', 'Consolas', 'monospace'",
        rescaleOverlappingGlyphs: true,
        scrollback: mobile ? 500 : 1000,
        scrollOnUserInput: true,
        theme: {
          background: '#0a0a0a',
          foreground: '#e0e0e0',
          cursor: '#60a5fa',
          cursorAccent: '#0a0a0a',
          selectionBackground: '#3a3a3a',
          black: '#1a1a1a',
          red: '#f87171',
          green: '#4ade80',
          yellow: '#fbbf24',
          blue: '#60a5fa',
          magenta: '#c084fc',
          cyan: '#22d3ee',
          white: '#e0e0e0',
          brightBlack: '#888888',
          brightRed: '#fca5a5',
          brightGreen: '#86efac',
          brightYellow: '#fde047',
          brightBlue: '#93c5fd',
          brightMagenta: '#d8b4fe',
          brightCyan: '#67e8f9',
          brightWhite: '#f5f5f5',
        },
        allowProposedApi: true,
      });

      // Initialize addons
      fitAddon = new FitAddon();
      // Custom link handler to prevent reverse-tabnabbing attacks
      // Without noopener,noreferrer, malicious links could hijack the original tab
      const webLinksAddon = new WebLinksAddon((_event, uri) => {
        window.open(uri, '_blank', 'noopener,noreferrer');
      });
      const unicode11Addon = new Unicode11Addon();
      // Renders Sixel (DCS q) and iTerm2 (OSC 1337) inline images from the
      // raw output stream. Kitty (APC) graphics are not supported by this
      // addon — see README ("Graphics support").
      const imageAddon = new ImageAddon();

      term.loadAddon(fitAddon);
      term.loadAddon(webLinksAddon);
      term.loadAddon(unicode11Addon);
      term.loadAddon(imageAddon);
      term.unicode.activeVersion = '11';

      // Wait for fonts to load before opening terminal
      if (document.fonts) {
        document.fonts.ready.then(() => {
          if (!terminalRef.current) return;
          term.open(terminalRef.current);

          // Try to load WebGL renderer for better performance
          try {
            const webglAddon = new WebglAddon();
            term.loadAddon(webglAddon);
            debugLog('WebGL renderer enabled');
          } catch (e) {
            console.warn('WebGL renderer failed, using default DOM renderer:', e);
          }

          // Suppress xterm.js DA (Device Attributes) responses
          // Our backend terminal emulator handles DA queries - xterm.js shouldn't respond
          // as that causes DA responses to be echoed back and displayed on screen
          // DA1 (Primary Device Attributes) - CSI c or CSI 0 c
          term.parser.registerCsiHandler({ final: 'c' }, () => true);
          // DA2 (Secondary Device Attributes) - CSI > c (note: '>' is a prefix, not intermediate)
          term.parser.registerCsiHandler({ prefix: '>', final: 'c' }, () => true);
          // DA3 (Tertiary Device Attributes) - CSI = c
          term.parser.registerCsiHandler({ prefix: '=', final: 'c' }, () => true);
          // DSR (Device Status Report) - CSI n (cursor position reports, etc.)
          term.parser.registerCsiHandler({ final: 'n' }, () => true);
          // DECRQM (Request Mode) - CSI ? Ps $ p
          term.parser.registerCsiHandler({ prefix: '?', intermediates: '$', final: 'p' }, () => true);
          debugLog('Suppressed xterm.js DA/DSR responses (handled by backend terminal)');

          // Fonts can finish loading after connect; a phone's grid must stay
          // at the pane's size.
          if (sendsResizeOnFit()) fitAddon.fit();
        });
      } else {
        term.open(terminalRef.current);

        // Suppress xterm.js DA (Device Attributes) responses (same as above)
        term.parser.registerCsiHandler({ final: 'c' }, () => true);
        term.parser.registerCsiHandler({ prefix: '>', final: 'c' }, () => true);
        term.parser.registerCsiHandler({ prefix: '=', final: 'c' }, () => true);
        term.parser.registerCsiHandler({ final: 'n' }, () => true);
        term.parser.registerCsiHandler({ prefix: '?', intermediates: '$', final: 'p' }, () => true);

        if (sendsResizeOnFit()) fitAddon.fit();
      }
    }

    // Store refs (both fresh and restored paths)
    fitAddonRef.current = fitAddon;
    xtermRef.current = term;

    // Lay the grid out in the view: a phone zooms (font size) and pans the
    // full pane grid; everyone dot-fills the view's surplus beyond the grid.
    //
    // The font only changes on a real trigger (`fitPending`: connect, a new
    // grid size, a view resize, a zoom change), never from a render, so a
    // font change cannot feed back into another one. The fit is computed from
    // the font's measured advance, then checked once the renderer has laid
    // the grid out: xterm snaps cell widths to device pixels, so the grid can
    // overshoot and is stepped down a bounded number of times.
    let layoutRaf: number | null = null;
    let fitPending = true;
    let fitChecks = 0;
    const measureCanvas = typeof document !== 'undefined' ? document.createElement('canvas') : null;
    const advancePerFontPx = (): number | null => {
      const ctx = measureCanvas?.getContext('2d');
      if (!ctx) return null;
      ctx.font = `100px ${term.options.fontFamily ?? 'monospace'}`;
      const w = ctx.measureText('W').width / 100;
      return w > 0 ? w : null;
    };
    const layoutViewport = (): void => {
      const view = containerRef.current;
      const host = terminalRef.current;
      const screen = term.element?.querySelector('.xterm-screen') as HTMLElement | null;
      if (!view || !host || !screen || term.cols <= 0 || term.rows <= 0) return;
      const viewSize = { width: view.clientWidth, height: view.clientHeight };

      if (isMobile()) {
        const font = term.options.fontSize ?? 14;
        if (fitPending) {
          fitPending = false;
          fitChecks = 0;
          const ratio = advancePerFontPx();
          const target = zoomFontRef.current ?? (ratio ? fitWidthFontSize(viewSize.width, term.cols, ratio) : null);
          if (target !== null && Math.abs(target - font) > 0.01) {
            term.options.fontSize = target;
            return; // the render that follows lays out and checks the fit
          }
        }
        const content = { width: screen.offsetWidth, height: screen.offsetHeight };
        if (zoomFontRef.current === null && content.width > viewSize.width && font > MIN_ZOOM_FONT_SIZE && fitChecks < 6) {
          fitChecks += 1;
          term.options.fontSize = Math.max(MIN_ZOOM_FONT_SIZE, font * Math.min(0.97, viewSize.width / content.width));
          return;
        }
        panRef.current = clampPan(panRef.current, content, viewSize);
        host.style.position = 'absolute';
        host.style.left = '0';
        host.style.top = '0';
        host.style.width = `${content.width}px`;
        host.style.height = `${content.height}px`;
        host.style.transformOrigin = '0 0';
        host.style.transform = `translate(${panRef.current.x}px, ${panRef.current.y}px)`;
      } else {
        host.style.position = '';
        host.style.width = '';
        host.style.height = '';
        host.style.transform = '';
        panRef.current = { x: 0, y: 0 };
      }

      const viewRect = view.getBoundingClientRect();
      const screenRect = screen.getBoundingClientRect();
      const grid = {
        left: screenRect.left - viewRect.left,
        top: screenRect.top - viewRect.top,
        width: screenRect.width,
        height: screenRect.height,
      };
      const cellW = grid.width / term.cols;
      const cellH = grid.height / term.rows;
      const { right, bottom } = surplusStrips(grid, viewSize);
      const rightStrip =
        right && right.width - SCROLLBAR_RESERVE_PX >= cellW
          ? { ...right, width: right.width - SCROLLBAR_RESERVE_PX }
          : null;
      paintDots(rightDotsRef.current, rightStrip, grid, cellW, cellH);
      paintDots(bottomDotsRef.current, bottom && bottom.height >= cellH ? bottom : null, grid, cellW, cellH);
    };
    // `refit` marks a real trigger: recompute the phone's fit-width font.
    const scheduleLayout = (refit = false): void => {
      if (refit) fitPending = true;
      if (layoutRaf !== null) cancelAnimationFrame(layoutRaf);
      layoutRaf = requestAnimationFrame(() => {
        layoutRaf = null;
        layoutViewport();
      });
    };
    scheduleLayoutRef.current = () => scheduleLayout(true);
    const layoutOnRender = term.onRender(() => {
      if (layoutRaf === null) scheduleLayout();
    });
    const resizeObserver = typeof ResizeObserver !== 'undefined'
      ? new ResizeObserver(() => scheduleLayout(true))
      : null;
    if (containerRef.current) resizeObserver?.observe(containerRef.current);

    // Phone gestures over the grid: two fingers pinch-zoom (previewed with a
    // CSS scale, committed as a font size on release), one finger pans when
    // the grid overflows the view and otherwise scrolls the scrollback. A tap
    // is left alone so it still focuses the terminal.
    type Gesture =
      | { kind: 'pinch'; startPan: Point; startMid: Point; startDist: number; startFont: number; scale: number; mid: Point }
      | { kind: 'touch'; start: Point; last: Point; startPan: Point; dragging: boolean };
    let gesture: Gesture | null = null;
    const touchPoint = (t: Touch, viewRect: DOMRect): Point => ({
      x: t.clientX - viewRect.left,
      y: t.clientY - viewRect.top,
    });
    const handleTouchStart = (e: TouchEvent) => {
      const view = containerRef.current;
      if (!isMobile() || !view) return;
      const viewRect = view.getBoundingClientRect();
      if (e.touches.length === 2) {
        const a = touchPoint(e.touches[0], viewRect);
        const b = touchPoint(e.touches[1], viewRect);
        const mid = { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
        gesture = {
          kind: 'pinch',
          startPan: { ...panRef.current },
          startMid: mid,
          startDist: Math.max(1, Math.hypot(a.x - b.x, a.y - b.y)),
          startFont: term.options.fontSize ?? 14,
          scale: 1,
          mid,
        };
        e.preventDefault();
        e.stopPropagation();
      } else if (e.touches.length === 1) {
        const p = touchPoint(e.touches[0], viewRect);
        gesture = { kind: 'touch', start: p, last: p, startPan: { ...panRef.current }, dragging: false };
      }
    };
    const handleTouchMove = (e: TouchEvent) => {
      const view = containerRef.current;
      const host = terminalRef.current;
      if (!gesture || !view || !host) return;
      const viewRect = view.getBoundingClientRect();
      if (gesture.kind === 'pinch' && e.touches.length >= 2) {
        const a = touchPoint(e.touches[0], viewRect);
        const b = touchPoint(e.touches[1], viewRect);
        gesture.mid = { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
        const font = clamp(
          gesture.startFont * (Math.hypot(a.x - b.x, a.y - b.y) / gesture.startDist),
          MIN_ZOOM_FONT_SIZE,
          MAX_ZOOM_FONT_SIZE,
        );
        gesture.scale = font / gesture.startFont;
        const pan = pinchPan(gesture.startPan, gesture.startMid, gesture.mid, gesture.scale);
        host.style.transform = `translate(${pan.x}px, ${pan.y}px) scale(${gesture.scale})`;
        e.preventDefault();
        e.stopPropagation();
      } else if (gesture.kind === 'touch' && e.touches.length === 1) {
        const p = touchPoint(e.touches[0], viewRect);
        if (!gesture.dragging && Math.hypot(p.x - gesture.start.x, p.y - gesture.start.y) < 8) return;
        gesture.dragging = true;
        const content = { width: host.offsetWidth, height: host.offsetHeight };
        const overflowsY = content.height > view.clientHeight;
        const desired = { x: gesture.startPan.x + p.x - gesture.start.x, y: gesture.startPan.y + p.y - gesture.start.y };
        panRef.current = clampPan(desired, content, { width: view.clientWidth, height: view.clientHeight });
        host.style.transform = `translate(${panRef.current.x}px, ${panRef.current.y}px)`;
        if (!overflowsY && term.rows > 0) {
          const lines = Math.trunc((gesture.last.y - p.y) / (content.height / term.rows));
          if (lines !== 0) {
            term.scrollLines(lines);
            gesture.last = p;
          }
        } else {
          gesture.last = p;
        }
        e.preventDefault();
        e.stopPropagation();
      }
    };
    const handleTouchEnd = (e: TouchEvent) => {
      if (!gesture) return;
      if (gesture.kind === 'pinch') {
        if (e.touches.length >= 2) return;
        const font = gesture.startFont * gesture.scale;
        zoomFontRef.current = font;
        panRef.current = pinchPan(gesture.startPan, gesture.startMid, gesture.mid, gesture.scale);
        gesture = null;
        scheduleLayout(true);
        e.preventDefault();
        e.stopPropagation();
        return;
      }
      if (e.touches.length === 0) gesture = null;
    };
    const touchOpts: AddEventListenerOptions = { capture: true, passive: false };
    const gestureTarget = containerRef.current;
    gestureTarget?.addEventListener('touchstart', handleTouchStart, touchOpts);
    gestureTarget?.addEventListener('touchmove', handleTouchMove, touchOpts);
    gestureTarget?.addEventListener('touchend', handleTouchEnd, touchOpts);
    gestureTarget?.addEventListener('touchcancel', handleTouchEnd, touchOpts);

    // Expose refit function to parent. Reads props from propsRef (not the
    // destructured `onRefit` argument) so this mount-only effect doesn't
    // need `onRefit` in its dependency array.
    if (propsRef.current.onRefit) {
      propsRef.current.onRefit((opts) => {
        setTimeout(() => {
          // Use explicit fontSize prop if set, otherwise use responsive sizing
          const newFontSize = fontSizeRef.current ?? getResponsiveFontSize();
          debugLog(`Refit: current fontSize=${term.options.fontSize}, new=${newFontSize}`);
          debugLog(`Refit: before fit - cols=${term.cols}, rows=${term.rows}`);

          if (term.options.fontSize !== newFontSize) {
            term.options.fontSize = newFontSize;
          }

          const container = terminalRef.current;
          if (container) {
            debugLog(`Refit: container size - ${container.clientWidth}x${container.clientHeight}`);
          }

          // A phone keeps the grid at the pane's size unless asked to resize it.
          if (!opts?.resizePane && !sendsResizeOnFit()) {
            term.refresh(0, term.rows - 1);
            scheduleLayout(true);
            return;
          }

          // Fit against the whole view, not the zoomed grid box.
          const host = terminalRef.current;
          if (host) {
            host.style.position = '';
            host.style.width = '';
            host.style.height = '';
            host.style.transform = '';
          }
          fitAddon.fit();
          const newCols = term.cols;
          const newRows = term.rows;
          debugLog(`Refit: after fit - cols=${newCols}, rows=${newRows}`);

          term.resize(newCols, newRows);
          term.refresh(0, newRows - 1);
          debugLog(`Refit: after explicit resize - cols=${term.cols}, rows=${term.rows}`);

          if (connectionRef.current?.isOpen()) {
            debugLog(`Refit: sending resize ${newCols}x${newRows}`);
            connectionRef.current.send(createResizeMessage(newCols, newRows));
          }
          if (!sendsResizeOnFit()) {
            // The pane now fits this screen at this font: keep it, unpanned.
            zoomFontRef.current = term.options.fontSize ?? newFontSize;
            panRef.current = { x: 0, y: 0 };
          }
          scheduleLayout(true);
        }, 50);
      });
    }

    // Expose focus function to parent
    if (propsRef.current.onFocus) {
      propsRef.current.onFocus(() => {
        term.focus();
      });
    }

    // Expose sendInput function to parent for onscreen keyboard
    if (propsRef.current.onSendInput) {
      propsRef.current.onSendInput((data: string) => {
        connectionRef.current?.send(createInputMessage(data));
      });
    }

    // Debounced resize handler for better performance
    // Only applies responsive font sizing when no explicit fontSize prop is set
    let resizeTimeout: NodeJS.Timeout;
    const handleResize = () => {
      clearTimeout(resizeTimeout);
      resizeTimeout = setTimeout(() => {
        // Only auto-adjust font size if no explicit fontSize prop
        if (fontSizeRef.current === undefined) {
          const newFontSize = getResponsiveFontSize();
          if (term.options.fontSize !== newFontSize) {
            term.options.fontSize = newFontSize;
          }
        }
        if (sendsResizeOnFit()) fitAddon.fit();
        scheduleLayout(true);
      }, 100);
    };

    window.addEventListener('resize', handleResize);

    // Handle orientation change specifically for mobile
    // Only applies responsive font sizing when no explicit fontSize prop is set
    const handleOrientationChange = () => {
      setTimeout(() => {
        // Only auto-adjust font size if no explicit fontSize prop
        if (fontSizeRef.current === undefined) {
          const newFontSize = getResponsiveFontSize();
          term.options.fontSize = newFontSize;
        }
        if (sendsResizeOnFit()) fitAddon.fit();
        scheduleLayout(true);
      }, 200);
    };

    window.addEventListener('orientationchange', handleOrientationChange);

    // Handle terminal input - goes through connectionRef so it works across reconnects
    // Implements local echo for printable characters to reduce perceived latency
    const onDataDisposable = term.onData((data) => {
      // Local echo for single printable ASCII characters
      // This makes typing feel instant even on slow connections
      if (localEchoEnabledRef.current && data.length === 1) {
        const code = data.charCodeAt(0);
        // Printable ASCII range: space (32) through tilde (126)
        if (code >= 32 && code <= 126) {
          // Echo locally immediately
          term.write(data);
          pendingEchoRef.current.push(data);
        }
      }

      // Always send to server
      connectionRef.current?.send(createInputMessage(data));
    });

    // Handle terminal resize - goes through connectionRef so it works across reconnects
    const onResizeDisposable = term.onResize(({ cols, rows }) => {
      scheduleLayout(true);
      if (applyingServerSizeRef.current || !sendsResizeOnFit()) return;
      if (connectionRef.current?.isOpen()) {
        debugLog(`Client resized to: ${cols}x${rows}`);
        connectionRef.current.send(createResizeMessage(cols, rows));
      }
    });

    // Reference to the terminal DOM element for event handlers
    const termElement = terminalRef.current;

    // Handle mouse events - send MouseInput when mouse tracking is active
    const getCellCoords = (e: MouseEvent): { col: number; row: number } | null => {
      const coreElement = term.element?.querySelector('.xterm-screen');
      if (!coreElement) return null;
      const rect = coreElement.getBoundingClientRect();
      const cellWidth = rect.width / term.cols;
      const cellHeight = rect.height / term.rows;
      const col = Math.floor((e.clientX - rect.left) / cellWidth);
      const row = Math.floor((e.clientY - rect.top) / cellHeight);
      if (col < 0 || col >= term.cols || row < 0 || row >= term.rows) return null;
      return { col, row };
    };

    const handleMouseDown = (e: MouseEvent) => {
      if (!mouseTrackingRef.current || !connectionRef.current?.isOpen()) return;
      const coords = getCellCoords(e);
      if (!coords) return;
      connectionRef.current.send(createMouseMessage(
        coords.col, coords.row, e.button, 'press', e.shiftKey, e.ctrlKey, e.altKey,
      ));
    };
    const handleMouseUp = (e: MouseEvent) => {
      if (!mouseTrackingRef.current || !connectionRef.current?.isOpen()) return;
      const coords = getCellCoords(e);
      if (!coords) return;
      connectionRef.current.send(createMouseMessage(
        coords.col, coords.row, 3, 'release', e.shiftKey, e.ctrlKey, e.altKey,
      ));
    };
    const handleMouseMove = (e: MouseEvent) => {
      if (!mouseTrackingRef.current || !connectionRef.current?.isOpen()) return;
      const coords = getCellCoords(e);
      if (!coords) return;
      connectionRef.current.send(createMouseMessage(
        coords.col, coords.row, 0, 'move', e.shiftKey, e.ctrlKey, e.altKey,
      ));
    };
    const handleWheel = (e: WheelEvent) => {
      if (!mouseTrackingRef.current || !connectionRef.current?.isOpen()) return;
      const coords = getCellCoords(e);
      if (!coords) return;
      const button = e.deltaY < 0 ? 4 : 5; // 4=scroll_up, 5=scroll_down
      connectionRef.current.send(createMouseMessage(
        coords.col, coords.row, button, 'scroll', e.shiftKey, e.ctrlKey, e.altKey,
      ));
    };
    termElement?.addEventListener('mousedown', handleMouseDown);
    termElement?.addEventListener('mouseup', handleMouseUp);
    termElement?.addEventListener('mousemove', handleMouseMove);
    termElement?.addEventListener('wheel', handleWheel);

    // Handle focus/blur events - send FocusChange when focus tracking is active
    const handleFocusIn = () => {
      if (focusTrackingRef.current && connectionRef.current?.isOpen()) {
        connectionRef.current.send(createFocusMessage(true));
      }
    };
    const handleFocusOut = () => {
      if (focusTrackingRef.current && connectionRef.current?.isOpen()) {
        connectionRef.current.send(createFocusMessage(false));
      }
    };
    window.addEventListener('focus', handleFocusIn);
    window.addEventListener('blur', handleFocusOut);

    // Handle paste events - send PasteInput when bracketed paste mode is active
    const handlePaste = (e: ClipboardEvent) => {
      if (bracketedPasteRef.current && connectionRef.current?.isOpen()) {
        const text = e.clipboardData?.getData('text');
        if (text) {
          e.preventDefault();
          connectionRef.current.send(createPasteMessage(text));
        }
      }
      // When not in bracketed paste mode, let xterm.js handle it normally
    };
    termElement?.addEventListener('paste', handlePaste);

    // Cleanup function
    // Note: In StrictMode, React unmounts then immediately remounts.
    // We preserve the terminal to restore it on remount, then dispose after a delay if not restored.
    return () => {
      clearTimeout(resizeTimeout);
      window.removeEventListener('resize', handleResize);
      window.removeEventListener('orientationchange', handleOrientationChange);
      window.removeEventListener('focus', handleFocusIn);
      window.removeEventListener('blur', handleFocusOut);
      termElement?.removeEventListener('paste', handlePaste);
      termElement?.removeEventListener('mousedown', handleMouseDown);
      termElement?.removeEventListener('mouseup', handleMouseUp);
      termElement?.removeEventListener('mousemove', handleMouseMove);
      termElement?.removeEventListener('wheel', handleWheel);
      onDataDisposable.dispose();
      onResizeDisposable.dispose();
      layoutOnRender.dispose();
      resizeObserver?.disconnect();
      if (layoutRaf !== null) cancelAnimationFrame(layoutRaf);
      gestureTarget?.removeEventListener('touchstart', handleTouchStart, touchOpts);
      gestureTarget?.removeEventListener('touchmove', handleTouchMove, touchOpts);
      gestureTarget?.removeEventListener('touchend', handleTouchEnd, touchOpts);
      gestureTarget?.removeEventListener('touchcancel', handleTouchEnd, touchOpts);

      // Cancel any pending RAF write flush
      if (rafIdRef.current) {
        cancelAnimationFrame(rafIdRef.current);
        rafIdRef.current = null;
      }
      // Flush any remaining buffered writes before cleanup
      if (writeBufferRef.current.length > 0 && term) {
        term.write(writeBufferRef.current.join(''));
        writeBufferRef.current = [];
      }

      // Preserve terminal for potential StrictMode remount
      preservedTerminal = term;
      preservedFitAddon = fitAddon;

      // Delay disposal to allow StrictMode remount to restore the terminal
      // If restored, preservedTerminal will be null and we skip disposal
      setTimeout(() => {
        if (preservedTerminal === term) {
          // Not restored - this is a real unmount, dispose everything
          debugLog('Real unmount - disposing terminal');
          term.dispose();
          connectionRef.current?.dispose();
          connectionRef.current = null;
          preservedTerminal = null;
          preservedFitAddon = null;
        }
      }, 100);

      xtermRef.current = null;
      fitAddonRef.current = null;
    };
  }, []);

  const connect = useCallback(() => {
    if (!xtermRef.current) return;

    // Replace the live connection when the URL changed; otherwise reuse it.
    if (connectionRef.current && connectionRef.current.getUrl() !== wsUrl) {
      connectionRef.current.dispose();
      connectionRef.current = null;
    }

    if (!connectionRef.current) {
      connectionRef.current = new TerminalConnection(wsUrl, {
        onStatus: (newStatus) => propsRef.current.onStatusChange?.(newStatus),
        onRetryingChange,
        onConnectionClosed: () => {
          xtermRef.current?.write('\r\n\x1b[1;33mDisconnected from server\x1b[0m\r\n');
        },
        onConnectionError: () => {
          xtermRef.current?.write('\r\n\x1b[1;31mConnection error\x1b[0m\r\n');
        },
        onInvalidUrl: (url) => {
          xtermRef.current?.write(`\r\n\x1b[1;31mInvalid WebSocket URL: ${url}\x1b[0m\r\n`);
        },
        onOpen: () => {
          // Fit terminal to container (a phone takes the pane's size instead)
          if (sendsResizeOnFit()) fitAddonRef.current?.fit();
          // Note: resize and refresh are sent after receiving 'connected' message
        },
        onOutput: (data) => {
          if (!xtermRef.current) return;
          // Use RAF-batched write for better performance
          bufferWrite(data);
        },
        onConnected: (connected) => {
          const term = xtermRef.current;
          if (!term) return;

          const initialScreenLength = connected.initialScreen?.length || 0;
          debugLog(`Session ID: ${connected.sessionId}`);
          debugLog(`Server initial size: ${connected.cols}x${connected.rows}, Client size: ${term.cols}x${term.rows}`);
          debugLog(`Initial screen provided: ${!!connected.initialScreen}, length: ${initialScreenLength}`);

          // Guard against oversized initial screens
          if (initialScreenLength > MAX_SNAPSHOT_SIZE) {
            console.error(`Initial screen too large (${initialScreenLength} bytes), skipping`);
          }

          // Apply theme if provided
          if (connected.theme) {
            applyTheme(connected.theme);
          }

          // A fresh connection starts fit-to-width and unpanned on a phone.
          connectedOnceRef.current = true;
          zoomFontRef.current = null;
          panRef.current = { x: 0, y: 0 };
          scheduleLayoutRef.current();

          // Reset and clear terminal on fresh connection
          term.reset();
          term.clear();

          // Clear any pending local echo from previous session
          pendingEchoRef.current = [];
          // Reset mode tracking for new session
          mouseTrackingRef.current = false;
          focusTrackingRef.current = false;
          bracketedPasteRef.current = false;
          // Clear tracked hyperlinks and user vars for new session
          hyperlinksRef.current.clear();
          userVarsRef.current.clear();

          // Send our size to server (desktop only), then request a fresh snapshot
          if (connectionRef.current?.isOpen()) {
            if (sendsResizeOnFit()) {
              const cols = term.cols;
              const rows = term.rows;
              debugLog(`Sending resize after connect: ${cols}x${rows}`);
              connectionRef.current.send(createResizeMessage(cols, rows));
            } else {
              applyServerSize(term, connected.cols, connected.rows, applyingServerSizeRef);
            }
            // Request fresh snapshot
            debugLog('Requesting refresh after connect');
            connectionRef.current.send(createRefreshMessage());
          }
          term.focus();
        },
        onServerResize: (resize) => {
          const term = xtermRef.current;
          if (!term) return;
          applyServerSize(term, resize.cols, resize.rows, applyingServerSizeRef);
          debugLog(`Terminal resized: ${resize.cols}x${resize.rows}`);
          if (connectionRef.current?.isOpen()) {
            debugLog('Requesting screen refresh after resize');
            connectionRef.current.send(createRefreshMessage());
          }
        },
        onRefresh: (refresh) => {
          const term = xtermRef.current;
          if (!term) return;

          const snapshotLength = refresh.screenContent?.length || 0;
          debugLog(`Refresh response received: ${refresh.cols}x${refresh.rows}`);
          debugLog('=== CLIENT REFRESH DEBUG ===');
          debugLog(`Client terminal size: ${term.cols}x${term.rows}`);
          debugLog(`Server snapshot size: ${refresh.cols}x${refresh.rows}`);
          debugLog(`Snapshot length: ${snapshotLength} bytes`);
          debugLog('============================');

          // Guard against oversized snapshots that could freeze the UI
          if (snapshotLength > MAX_SNAPSHOT_SIZE) {
            console.error(`Snapshot too large (${snapshotLength} bytes), rejecting to prevent UI freeze`);
            term.write('\r\n\x1b[1;33mWarning: Screen snapshot too large, display may be incomplete\x1b[0m\r\n');
            return;
          }

          // The snapshot is laid out for the server's grid; match it first.
          applyServerSize(term, refresh.cols, refresh.rows, applyingServerSizeRef);
          // Fully reset terminal state and clear all buffers
          term.reset();
          term.clear();
          // Write fresh content - the snapshot should include cursor positioning
          if (refresh.screenContent && snapshotLength > 0) {
            const content = sharedDecoder.decode(refresh.screenContent);
            term.write(content);
          }
          // Scroll to bottom to ensure cursor is visible
          term.scrollToBottom();
          term.focus();
        },
        onTitle: (title) => {
          if (!xtermRef.current) return;
          document.title = title.title + ' - Terminal Streaming';
          debugLog(`Title changed: ${title.title}`);
        },
        onServerError: (message) => {
          const term = xtermRef.current;
          if (!term) return;
          console.error('Server error:', message);
          term.write(`\r\n\x1b[1;31mError: ${message}\x1b[0m\r\n`);
        },
        onShutdown: (reason, kind) => {
          const term = xtermRef.current;
          if (!term) return;
          if (kind === 'session-ended') {
            term.write(`\r\n\x1b[1;31mSession ended: ${reason}\x1b[0m\r\n`);
          } else if (kind === 'idle-timeout') {
            term.write('\r\n\x1b[1;33mSession timed out due to inactivity\x1b[0m\r\n');
          } else {
            term.write(`\r\n\x1b[1;33mServer: ${reason}\x1b[0m\r\n`);
          }
        },
        onModeChanged: (mode, enabled) => {
          if (!xtermRef.current) return;
          debugLog(`Mode changed: ${mode} = ${enabled}`);
          // Track mode state for mouse/focus/paste handling
          if (mode === 'mouse_tracking') {
            mouseTrackingRef.current = enabled;
          } else if (mode === 'focus_tracking') {
            focusTrackingRef.current = enabled;
          } else if (mode === 'bracketed_paste') {
            bracketedPasteRef.current = enabled;
          }
        },
        onHyperlinkAdded: (link) => {
          if (!xtermRef.current) return;
          const entry = { url: link.url, col: link.col, id: link.id };
          const rowLinks = hyperlinksRef.current.get(link.row) || [];
          rowLinks.push(entry);
          hyperlinksRef.current.set(link.row, rowLinks);
          propsRef.current.onHyperlinkAdded?.(link.url, link.row, link.col, link.id);
        },
        onUserVarChanged: (uv) => {
          if (!xtermRef.current) return;
          if (uv.value === '') {
            userVarsRef.current.delete(uv.name);
          } else {
            userVarsRef.current.set(uv.name, uv.value);
          }
          propsRef.current.onUserVarChanged?.(uv.name, uv.value, uv.oldValue);
        },
        onSelectionChanged: (sel) => {
          const term = xtermRef.current;
          if (!term) return;

          if (sel.cleared) {
            term.clearSelection();
          } else if (
            sel.startCol !== undefined &&
            sel.startRow !== undefined &&
            sel.endCol !== undefined &&
            sel.endRow !== undefined
          ) {
            if (sel.mode === 'line') {
              term.selectLines(sel.startRow, sel.endRow);
            } else {
              // Compute character-span length for select()
              const cols = term.cols;
              let length: number;
              if (sel.startRow === sel.endRow) {
                length = sel.endCol - sel.startCol;
              } else {
                length = (cols - sel.startCol)
                  + (sel.endRow - sel.startRow - 1) * cols
                  + sel.endCol;
              }
              if (length > 0) {
                term.select(sel.startCol, sel.startRow, length);
              }
            }

            // Copy to clipboard if text is provided
            if (sel.text) {
              navigator.clipboard.writeText(sel.text).catch(() => {
                // Clipboard write may fail without user gesture — silent fallback
              });
            }
          }

          propsRef.current.onSelectionChanged?.(sel.text, sel.cleared);
        },
      });
    }

    connectionRef.current.connect();
  }, [wsUrl, onRetryingChange, bufferWrite]);

  const disconnect = useCallback(() => {
    connectionRef.current?.dispose();
    connectionRef.current = null;
  }, []);

  // Expose control functions to parent
  useEffect(() => {
    const cancelRetry = () => connectionRef.current?.cancelRetry();
    onConnectControl?.({ connect, disconnect, cancelRetry });
  }, [connect, disconnect, onConnectControl]);

  // Reconnect when wsUrl changes
  useEffect(() => {
    if (prevWsUrlRef.current !== wsUrl) {
      debugLog(`WebSocket URL changed: ${prevWsUrlRef.current} -> ${wsUrl}`);
      prevWsUrlRef.current = wsUrl;
      // Disconnect and reconnect with new URL
      disconnect();
      const timer = setTimeout(connect, 100);
      return () => clearTimeout(timer);
    }
  }, [wsUrl, connect, disconnect]);

  // Update font size when prop changes
  useEffect(() => {
    fontSizeRef.current = fontSize;
    const term = xtermRef.current;
    const fitAddon = fitAddonRef.current;
    if (term && fitAddon && fontSize !== undefined) {
      debugLog(`Font size changed to ${fontSize}px`);
      if (!sendsResizeOnFit()) {
        // On a phone the font is the zoom; before the first connect it only
        // sets the base size a deliberate Fit uses.
        if (connectedOnceRef.current) {
          zoomFontRef.current = fontSize;
          scheduleLayoutRef.current();
        }
        return;
      }
      term.options.fontSize = fontSize;
      if (sendsResizeOnFit()) {
        fitAddon.fit();
        connectionRef.current?.send(createResizeMessage(term.cols, term.rows));
      }
    }
  }, [fontSize]);

  useEffect(() => {
    // Auto-connect on mount
    const timer = setTimeout(connect, 500);
    return () => {
      clearTimeout(timer);
      // Stop reconnecting and quiet the heartbeat, but leave any open
      // socket alive: StrictMode remounts restore it (the connection is
      // disposed with the terminal on real unmount instead).
      connectionRef.current?.cancelRetry();
      connectionRef.current?.stopHeartbeat();
    };
  }, [connect]);

  // Handle click/touch to focus terminal (needed for mobile keyboard)
  const handleTerminalClick = () => {
    if (xtermRef.current) {
      xtermRef.current.focus();
    }
  };

  return (
    <div ref={containerRef} className="terminal-shell" onClick={handleTerminalClick}>
      <div ref={terminalRef} className="flex-1 terminal-scrollbar" />
      <div ref={rightDotsRef} className="terminal-dead-space" aria-hidden="true" />
      <div ref={bottomDotsRef} className="terminal-dead-space" aria-hidden="true" />
    </div>
  );
}
