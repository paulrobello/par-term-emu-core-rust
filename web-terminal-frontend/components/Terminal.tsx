'use client';

import { useEffect, useRef, useState, useCallback } from 'react';
import { Terminal as XTerm } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebglAddon } from '@xterm/addon-webgl';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { Unicode11Addon } from '@xterm/addon-unicode11';
import type { ConnectionStatus } from '@/types/terminal';
import {
  decodeServerMessage,
  encodeClientMessage,
  createInputMessage,
  createResizeMessage,
  createRefreshMessage,
  createPingMessage,
  themeToXtermOptions,
} from '@/lib/protocol';

interface TerminalProps {
  wsUrl: string;
  fontSize?: number;
  onStatusChange?: (status: ConnectionStatus) => void;
  onThemeChange?: (backgroundColor: string) => void;
  onRefit?: (refitFn: () => void) => void;
  onFocus?: (focusFn: () => void) => void;
  onRetryingChange?: (isRetrying: boolean) => void;
  onConnectControl?: (control: { connect: () => void; disconnect: () => void; cancelRetry: () => void }) => void;
  onSendInput?: (sendFn: (data: string) => void) => void;
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

// Shared TextDecoder instance - reuse instead of creating per message
const sharedDecoder = new TextDecoder();

// Maximum snapshot size (1MB) to prevent UI freeze from large payloads
const MAX_SNAPSHOT_SIZE = 1024 * 1024;

export default function Terminal({ wsUrl, fontSize, onStatusChange, onThemeChange, onRefit, onFocus, onRetryingChange, onConnectControl, onSendInput }: TerminalProps) {
  const terminalRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const xtermRef = useRef<XTerm | null>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const [status, setStatus] = useState<ConnectionStatus>('disconnected');

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

  // Auto-reconnect state
  const retryTimeoutRef = useRef<NodeJS.Timeout | null>(null);
  const retryDelayRef = useRef(500); // Start at 500ms
  const isRetryingRef = useRef(false);
  const retryCancelledRef = useRef(false);

  // Heartbeat/ping state for stale connection detection
  const heartbeatIntervalRef = useRef<NodeJS.Timeout | null>(null);
  const lastPongRef = useRef<number>(0);
  const HEARTBEAT_INTERVAL_MS = 25000; // Send ping every 25 seconds
  const HEARTBEAT_TIMEOUT_MS = 10000; // Consider stale if no pong within 10 seconds

  // Track previous wsUrl to detect changes
  const prevWsUrlRef = useRef<string>(wsUrl);

  // Track fontSize prop for use in handlers
  const fontSizeRef = useRef<number | undefined>(fontSize);

  const updateStatus = (newStatus: ConnectionStatus) => {
    setStatus(newStatus);
    onStatusChange?.(newStatus);
  };

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

  const cancelRetry = useCallback(() => {
    if (retryTimeoutRef.current) {
      clearTimeout(retryTimeoutRef.current);
      retryTimeoutRef.current = null;
    }
    isRetryingRef.current = false;
    retryCancelledRef.current = true;
    retryDelayRef.current = 500; // Reset delay
    onRetryingChange?.(false);
  }, [onRetryingChange]);

  const scheduleRetry = useCallback(() => {
    if (retryCancelledRef.current) return;

    isRetryingRef.current = true;
    onRetryingChange?.(true);

    const delay = retryDelayRef.current;
    console.log(`Scheduling reconnect in ${delay}ms`);

    retryTimeoutRef.current = setTimeout(() => {
      if (!retryCancelledRef.current) {
        // Increase delay for next retry (max 5 seconds)
        retryDelayRef.current = Math.min(retryDelayRef.current * 2, 5000);
        connect();
      }
    }, delay);
  }, [onRetryingChange]);

  // Stop heartbeat timer
  const stopHeartbeat = useCallback(() => {
    if (heartbeatIntervalRef.current) {
      clearInterval(heartbeatIntervalRef.current);
      heartbeatIntervalRef.current = null;
    }
  }, []);

  // Start heartbeat timer - sends pings and detects stale connections
  const startHeartbeat = useCallback(() => {
    stopHeartbeat(); // Clear any existing heartbeat
    lastPongRef.current = Date.now(); // Initialize last pong time

    heartbeatIntervalRef.current = setInterval(() => {
      const ws = wsRef.current;
      if (!ws || ws.readyState !== WebSocket.OPEN) {
        stopHeartbeat();
        return;
      }

      const now = Date.now();
      const timeSinceLastPong = now - lastPongRef.current;

      // Check if connection is stale (no pong received within timeout)
      if (timeSinceLastPong > HEARTBEAT_INTERVAL_MS + HEARTBEAT_TIMEOUT_MS) {
        console.warn(`Connection stale: no pong in ${timeSinceLastPong}ms, closing`);
        stopHeartbeat();
        ws.close();
        return;
      }

      // Send ping
      try {
        ws.send(encodeClientMessage(createPingMessage()));
        console.log('Heartbeat ping sent');
      } catch (err) {
        console.error('Failed to send heartbeat ping:', err);
        stopHeartbeat();
        ws.close();
      }
    }, HEARTBEAT_INTERVAL_MS);
  }, [stopHeartbeat]);

  // Apply theme to terminal (using protobuf ThemeInfo)
  const applyTheme = (theme: { name: string; background?: { r: number; g: number; b: number }; foreground?: { r: number; g: number; b: number }; normal: { r: number; g: number; b: number }[]; bright: { r: number; g: number; b: number }[] }) => {
    if (!xtermRef.current) return;

    console.log('Applying theme:', theme.name);

    const xtermTheme = themeToXtermOptions(theme as import('@/lib/protocol').ThemeInfo);

    // Update xterm.js theme
    xtermRef.current.options.theme = xtermTheme;

    const bgHex = xtermTheme.background || '#000000';

    // Update container background color
    if (containerRef.current) {
      containerRef.current.style.backgroundColor = bgHex;
    }

    // Notify parent component of background color change
    onThemeChange?.(bgHex);
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
      console.log('Restoring preserved terminal (StrictMode remount)');
      term = preservedTerminal;
      fitAddon = preservedFitAddon;

      // Clear preserved refs now that we've restored
      preservedTerminal = null;
      preservedFitAddon = null;
    } else {
      // Fresh initialization
      const mobile = isMobile();
      const initialFontSize = fontSize ?? getResponsiveFontSize();
      console.log(`Terminal init: width=${window.innerWidth}, mobile=${mobile}, fontSize=${initialFontSize}`);

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

      term.loadAddon(fitAddon);
      term.loadAddon(webLinksAddon);
      term.loadAddon(unicode11Addon);
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
            console.log('WebGL renderer enabled');
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
          console.log('Suppressed xterm.js DA/DSR responses (handled by backend terminal)');

          fitAddon.fit();
        });
      } else {
        term.open(terminalRef.current);

        // Suppress xterm.js DA (Device Attributes) responses (same as above)
        term.parser.registerCsiHandler({ final: 'c' }, () => true);
        term.parser.registerCsiHandler({ prefix: '>', final: 'c' }, () => true);
        term.parser.registerCsiHandler({ prefix: '=', final: 'c' }, () => true);
        term.parser.registerCsiHandler({ final: 'n' }, () => true);
        term.parser.registerCsiHandler({ prefix: '?', intermediates: '$', final: 'p' }, () => true);

        fitAddon.fit();
      }
    }

    // Store refs (both fresh and restored paths)
    fitAddonRef.current = fitAddon;
    xtermRef.current = term;

    // Expose refit function to parent
    if (onRefit) {
      onRefit(() => {
        setTimeout(() => {
          // Use explicit fontSize prop if set, otherwise use responsive sizing
          const newFontSize = fontSizeRef.current ?? getResponsiveFontSize();
          console.log(`Refit: current fontSize=${term.options.fontSize}, new=${newFontSize}`);
          console.log(`Refit: before fit - cols=${term.cols}, rows=${term.rows}`);

          if (term.options.fontSize !== newFontSize) {
            term.options.fontSize = newFontSize;
          }

          const container = terminalRef.current;
          if (container) {
            console.log(`Refit: container size - ${container.clientWidth}x${container.clientHeight}`);
          }

          fitAddon.fit();
          const newCols = term.cols;
          const newRows = term.rows;
          console.log(`Refit: after fit - cols=${newCols}, rows=${newRows}`);

          term.resize(newCols, newRows);
          term.refresh(0, newRows - 1);
          console.log(`Refit: after explicit resize - cols=${term.cols}, rows=${term.rows}`);

          if (wsRef.current?.readyState === WebSocket.OPEN) {
            console.log(`Refit: sending resize ${newCols}x${newRows}`);
            wsRef.current.send(encodeClientMessage(createResizeMessage(newCols, newRows)));
          }
        }, 50);
      });
    }

    // Expose focus function to parent
    if (onFocus) {
      onFocus(() => {
        term.focus();
      });
    }

    // Expose sendInput function to parent for onscreen keyboard
    if (onSendInput) {
      onSendInput((data: string) => {
        if (wsRef.current?.readyState === WebSocket.OPEN) {
          wsRef.current.send(encodeClientMessage(createInputMessage(data)));
        }
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
        fitAddon.fit();
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
        fitAddon.fit();
      }, 200);
    };

    window.addEventListener('orientationchange', handleOrientationChange);

    // Handle terminal input - use wsRef so it works across reconnects
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
      if (wsRef.current?.readyState === WebSocket.OPEN) {
        wsRef.current.send(encodeClientMessage(createInputMessage(data)));
      }
    });

    // Handle terminal resize - use wsRef so it works across reconnects
    const onResizeDisposable = term.onResize(({ cols, rows }) => {
      if (wsRef.current?.readyState === WebSocket.OPEN) {
        console.log(`Client resized to: ${cols}x${rows}`);
        wsRef.current.send(encodeClientMessage(createResizeMessage(cols, rows)));
      }
    });

    // Cleanup function
    // Note: In StrictMode, React unmounts then immediately remounts.
    // We preserve the terminal to restore it on remount, then dispose after a delay if not restored.
    return () => {
      clearTimeout(resizeTimeout);
      window.removeEventListener('resize', handleResize);
      window.removeEventListener('orientationchange', handleOrientationChange);
      onDataDisposable.dispose();
      onResizeDisposable.dispose();

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
          console.log('Real unmount - disposing terminal');
          term.dispose();
          wsRef.current?.close();
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

    // If already connected, close existing socket first
    if (wsRef.current?.readyState === WebSocket.OPEN) {
      console.log('Closing existing connection before reconnecting');
      wsRef.current.close();
      wsRef.current = null;
    }

    // Reset cancelled flag when manually connecting
    retryCancelledRef.current = false;
    updateStatus('connecting');

    // Validate URL and create WebSocket with error handling
    let ws: WebSocket;
    try {
      ws = new WebSocket(wsUrl);
    } catch (err) {
      console.error('Invalid WebSocket URL:', err);
      updateStatus('error');
      if (xtermRef.current) {
        xtermRef.current.write(`\r\n\x1b[1;31mInvalid WebSocket URL: ${wsUrl}\x1b[0m\r\n`);
      }
      return;
    }

    ws.binaryType = 'arraybuffer'; // Use binary protocol
    wsRef.current = ws;

    ws.onopen = () => {
      console.log('WebSocket connected');
      updateStatus('connected');
      // Reset retry delay on successful connection
      retryDelayRef.current = 500;
      isRetryingRef.current = false;
      onRetryingChange?.(false);

      // Start heartbeat for stale connection detection
      startHeartbeat();

      // Fit terminal to container
      if (fitAddonRef.current) {
        fitAddonRef.current.fit();
      }
      // Note: resize and refresh are sent after receiving 'connected' message
    };

    ws.onmessage = (event) => {
      try {
        const msg = decodeServerMessage(event.data);
        const term = xtermRef.current;
        if (!term) return;

        // Handle oneof message type
        switch (msg.message.case) {
          case 'output': {
            const output = msg.message.value;
            const data = sharedDecoder.decode(output.data);
            // Use RAF-batched write for better performance
            bufferWrite(data);
            break;
          }

          case 'connected': {
            const connected = msg.message.value;
            const initialScreenLength = connected.initialScreen?.length || 0;
            console.log(`Session ID: ${connected.sessionId}`);
            console.log(`Server initial size: ${connected.cols}x${connected.rows}, Client size: ${term.cols}x${term.rows}`);
            console.log(`Initial screen provided: ${!!connected.initialScreen}, length: ${initialScreenLength}`);

            // Guard against oversized initial screens
            if (initialScreenLength > MAX_SNAPSHOT_SIZE) {
              console.error(`Initial screen too large (${initialScreenLength} bytes), skipping`);
            }

            // Apply theme if provided
            if (connected.theme) {
              applyTheme(connected.theme);
            }

            // Reset and clear terminal on fresh connection
            term.reset();
            term.clear();

            // Clear any pending local echo from previous session
            pendingEchoRef.current = [];

            // Send our size to server, then request a fresh snapshot
            if (ws.readyState === WebSocket.OPEN) {
              const cols = term.cols;
              const rows = term.rows;
              console.log(`Sending resize after connect: ${cols}x${rows}`);
              ws.send(encodeClientMessage(createResizeMessage(cols, rows)));
              // Request fresh snapshot
              console.log('Requesting refresh after connect');
              ws.send(encodeClientMessage(createRefreshMessage()));
            }
            term.focus();
            break;
          }

          case 'resize': {
            const resize = msg.message.value;
            term.resize(resize.cols, resize.rows);
            console.log(`Terminal resized: ${resize.cols}x${resize.rows}`);
            if (ws.readyState === WebSocket.OPEN) {
              console.log('Requesting screen refresh after resize');
              ws.send(encodeClientMessage(createRefreshMessage()));
            }
            break;
          }

          case 'refresh': {
            const refresh = msg.message.value;
            const snapshotLength = refresh.screenContent?.length || 0;
            console.log(`Refresh response received: ${refresh.cols}x${refresh.rows}`);
            console.log('=== CLIENT REFRESH DEBUG ===');
            console.log(`Client terminal size: ${term.cols}x${term.rows}`);
            console.log(`Server snapshot size: ${refresh.cols}x${refresh.rows}`);
            console.log(`Snapshot length: ${snapshotLength} bytes`);
            console.log('============================');

            // Guard against oversized snapshots that could freeze the UI
            if (snapshotLength > MAX_SNAPSHOT_SIZE) {
              console.error(`Snapshot too large (${snapshotLength} bytes), rejecting to prevent UI freeze`);
              term.write('\r\n\x1b[1;33mWarning: Screen snapshot too large, display may be incomplete\x1b[0m\r\n');
              break;
            }

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
            break;
          }

          case 'title': {
            const title = msg.message.value;
            document.title = title.title + ' - Terminal Streaming';
            console.log(`Title changed: ${title.title}`);
            break;
          }

          case 'bell':
            console.log('Bell received');
            break;

          case 'error': {
            const error = msg.message.value;
            console.error('Server error:', error.message);
            term.write(`\r\n\x1b[1;31mError: ${error.message}\x1b[0m\r\n`);
            break;
          }

          case 'shutdown': {
            const shutdown = msg.message.value;
            console.log('Server shutdown:', shutdown.reason);
            term.write(`\r\n\x1b[1;33mServer shutdown: ${shutdown.reason}\x1b[0m\r\n`);
            break;
          }

          case 'modeChanged': {
            const mc = msg.message.value;
            console.log(`Mode changed: ${mc.mode} = ${mc.enabled}`);
            // Currently logged for debugging; downstream consumers
            // (like TermNexus) can act on these to sync xterm.js state
            break;
          }

          case 'pong':
            // Pong received - update last pong time for heartbeat tracking
            lastPongRef.current = Date.now();
            console.log('Heartbeat pong received');
            break;

          default:
            // Silently ignore other message types (cwdChanged, triggerMatched, etc.)
            break;
        }
      } catch (err) {
        console.error('Failed to decode message:', err);
      }
    };

    ws.onerror = (error) => {
      console.error('WebSocket error:', error);
      stopHeartbeat();
      updateStatus('error');
      if (xtermRef.current) {
        xtermRef.current.write('\r\n\x1b[1;31mConnection error\x1b[0m\r\n');
      }
    };

    ws.onclose = () => {
      console.log('WebSocket disconnected');
      stopHeartbeat();
      updateStatus('disconnected');
      wsRef.current = null;
      if (xtermRef.current) {
        xtermRef.current.write('\r\n\x1b[1;33mDisconnected from server\x1b[0m\r\n');
      }
      // Auto-reconnect unless cancelled
      if (!retryCancelledRef.current) {
        scheduleRetry();
      }
    };
  }, [wsUrl, onRetryingChange, scheduleRetry, bufferWrite, startHeartbeat, stopHeartbeat]);

  const disconnect = useCallback(() => {
    cancelRetry();
    stopHeartbeat();
    if (wsRef.current) {
      wsRef.current.close();
      wsRef.current = null;
    }
  }, [cancelRetry, stopHeartbeat]);

  // Expose control functions to parent
  useEffect(() => {
    onConnectControl?.({ connect, disconnect, cancelRetry });
  }, [connect, disconnect, cancelRetry, onConnectControl]);

  // Reconnect when wsUrl changes
  useEffect(() => {
    if (prevWsUrlRef.current !== wsUrl) {
      console.log(`WebSocket URL changed: ${prevWsUrlRef.current} -> ${wsUrl}`);
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
      console.log(`Font size changed to ${fontSize}px`);
      term.options.fontSize = fontSize;
      fitAddon.fit();
      // Send resize to server
      if (wsRef.current?.readyState === WebSocket.OPEN) {
        wsRef.current.send(encodeClientMessage(createResizeMessage(term.cols, term.rows)));
      }
    }
  }, [fontSize]);

  useEffect(() => {
    // Auto-connect on mount
    const timer = setTimeout(connect, 500);
    return () => {
      clearTimeout(timer);
      cancelRetry();
      stopHeartbeat();
    };
  }, [connect, cancelRetry, stopHeartbeat]);

  // Handle click/touch to focus terminal (needed for mobile keyboard)
  const handleTerminalClick = () => {
    if (xtermRef.current) {
      xtermRef.current.focus();
    }
  };

  return (
    <div ref={containerRef} className="terminal-shell" onClick={handleTerminalClick}>
      <div ref={terminalRef} className="flex-1 terminal-scrollbar" />
    </div>
  );
}
