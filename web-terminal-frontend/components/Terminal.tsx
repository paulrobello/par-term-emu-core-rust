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
  themeToXtermOptions,
} from '@/lib/protocol';

interface TerminalProps {
  wsUrl: string;
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

export default function Terminal({ wsUrl, onStatusChange, onThemeChange, onRefit, onFocus, onRetryingChange, onConnectControl, onSendInput }: TerminalProps) {
  const terminalRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const xtermRef = useRef<XTerm | null>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const [status, setStatus] = useState<ConnectionStatus>('disconnected');

  // Auto-reconnect state
  const retryTimeoutRef = useRef<NodeJS.Timeout | null>(null);
  const retryDelayRef = useRef(500); // Start at 500ms
  const isRetryingRef = useRef(false);
  const retryCancelledRef = useRef(false);

  const updateStatus = (newStatus: ConnectionStatus) => {
    setStatus(newStatus);
    onStatusChange?.(newStatus);
  };

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
      const initialFontSize = getResponsiveFontSize();
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
      const webLinksAddon = new WebLinksAddon();
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

          fitAddon.fit();
        });
      } else {
        term.open(terminalRef.current);
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
          const newFontSize = getResponsiveFontSize();
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
    let resizeTimeout: NodeJS.Timeout;
    const handleResize = () => {
      clearTimeout(resizeTimeout);
      resizeTimeout = setTimeout(() => {
        const newFontSize = getResponsiveFontSize();
        if (term.options.fontSize !== newFontSize) {
          term.options.fontSize = newFontSize;
        }
        fitAddon.fit();
      }, 100);
    };

    window.addEventListener('resize', handleResize);

    // Handle orientation change specifically for mobile
    const handleOrientationChange = () => {
      setTimeout(() => {
        const newFontSize = getResponsiveFontSize();
        term.options.fontSize = newFontSize;
        fitAddon.fit();
      }, 200);
    };

    window.addEventListener('orientationchange', handleOrientationChange);

    // Handle terminal input - use wsRef so it works across reconnects
    const onDataDisposable = term.onData((data) => {
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
    if (!xtermRef.current || wsRef.current?.readyState === WebSocket.OPEN) return;

    // Reset cancelled flag when manually connecting
    retryCancelledRef.current = false;
    updateStatus('connecting');

    const ws = new WebSocket(wsUrl);
    ws.binaryType = 'arraybuffer'; // Use binary protocol
    wsRef.current = ws;

    ws.onopen = () => {
      console.log('WebSocket connected');
      updateStatus('connected');
      // Reset retry delay on successful connection
      retryDelayRef.current = 500;
      isRetryingRef.current = false;
      onRetryingChange?.(false);

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

        const decoder = new TextDecoder();

        // Handle oneof message type
        switch (msg.message.case) {
          case 'output': {
            const output = msg.message.value;
            const data = decoder.decode(output.data);
            term.write(data);
            break;
          }

          case 'connected': {
            const connected = msg.message.value;
            console.log(`Session ID: ${connected.sessionId}`);
            console.log(`Server initial size: ${connected.cols}x${connected.rows}, Client size: ${term.cols}x${term.rows}`);
            console.log(`Initial screen provided: ${!!connected.initialScreen}, length: ${connected.initialScreen?.length || 0}`);

            // Apply theme if provided
            if (connected.theme) {
              applyTheme(connected.theme);
            }

            // Reset and clear terminal on fresh connection
            term.reset();
            term.clear();

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
            console.log(`Refresh response received: ${refresh.cols}x${refresh.rows}`);
            console.log('=== CLIENT REFRESH DEBUG ===');
            console.log(`Client terminal size: ${term.cols}x${term.rows}`);
            console.log(`Server snapshot size: ${refresh.cols}x${refresh.rows}`);
            console.log(`Snapshot length: ${refresh.screenContent?.length || 0} bytes`);
            console.log('============================');

            // Fully reset terminal state and clear all buffers
            term.reset();
            term.clear();
            // Write fresh content - the snapshot should include cursor positioning
            if (refresh.screenContent && refresh.screenContent.length > 0) {
              const content = decoder.decode(refresh.screenContent);
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

          case 'pong':
            // Pong received - keepalive acknowledged
            break;

          default:
            console.warn('Unknown message type:', msg.message.case);
        }
      } catch (err) {
        console.error('Failed to decode message:', err);
      }
    };

    ws.onerror = (error) => {
      console.error('WebSocket error:', error);
      updateStatus('error');
      if (xtermRef.current) {
        xtermRef.current.write('\r\n\x1b[1;31mConnection error\x1b[0m\r\n');
      }
    };

    ws.onclose = () => {
      console.log('WebSocket disconnected');
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
  }, [wsUrl, onRetryingChange, scheduleRetry]);

  const disconnect = useCallback(() => {
    cancelRetry();
    if (wsRef.current) {
      wsRef.current.close();
      wsRef.current = null;
    }
  }, [cancelRetry]);

  // Expose control functions to parent
  useEffect(() => {
    onConnectControl?.({ connect, disconnect, cancelRetry });
  }, [connect, disconnect, cancelRetry, onConnectControl]);

  useEffect(() => {
    // Auto-connect on mount
    const timer = setTimeout(connect, 500);
    return () => {
      clearTimeout(timer);
      cancelRetry();
    };
  }, [wsUrl, connect, cancelRetry]);

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
