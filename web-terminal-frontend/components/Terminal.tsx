'use client';

import { useEffect, useRef, useState } from 'react';
import { Terminal as XTerm } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebglAddon } from '@xterm/addon-webgl';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { Unicode11Addon } from '@xterm/addon-unicode11';
import type { ConnectionStatus, ThemeInfo } from '@/types/terminal';

interface TerminalProps {
  wsUrl: string;
  onStatusChange?: (status: ConnectionStatus) => void;
  onThemeChange?: (backgroundColor: string) => void;
}

export default function Terminal({ wsUrl, onStatusChange, onThemeChange }: TerminalProps) {
  const terminalRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const xtermRef = useRef<XTerm | null>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const [status, setStatus] = useState<ConnectionStatus>('disconnected');

  const updateStatus = (newStatus: ConnectionStatus) => {
    setStatus(newStatus);
    onStatusChange?.(newStatus);
  };

  // Convert RGB array to hex color string
  const rgbToHex = (rgb: [number, number, number]): string => {
    return '#' + rgb.map(x => x.toString(16).padStart(2, '0')).join('');
  };

  // Apply theme to terminal
  const applyTheme = (theme: ThemeInfo) => {
    if (!xtermRef.current) return;

    console.log('Applying theme:', theme.name);

    const bgHex = rgbToHex(theme.background);
    const fgHex = rgbToHex(theme.foreground);

    // Update xterm.js theme
    xtermRef.current.options.theme = {
      background: bgHex,
      foreground: fgHex,
      black: rgbToHex(theme.normal[0]),
      red: rgbToHex(theme.normal[1]),
      green: rgbToHex(theme.normal[2]),
      yellow: rgbToHex(theme.normal[3]),
      blue: rgbToHex(theme.normal[4]),
      magenta: rgbToHex(theme.normal[5]),
      cyan: rgbToHex(theme.normal[6]),
      white: rgbToHex(theme.normal[7]),
      brightBlack: rgbToHex(theme.bright[0]),
      brightRed: rgbToHex(theme.bright[1]),
      brightGreen: rgbToHex(theme.bright[2]),
      brightYellow: rgbToHex(theme.bright[3]),
      brightBlue: rgbToHex(theme.bright[4]),
      brightMagenta: rgbToHex(theme.bright[5]),
      brightCyan: rgbToHex(theme.bright[6]),
      brightWhite: rgbToHex(theme.bright[7]),
    };

    // Update container background color
    if (containerRef.current) {
      containerRef.current.style.backgroundColor = bgHex;
    }

    // Notify parent component of background color change
    onThemeChange?.(bgHex);
  };

  useEffect(() => {
    if (!terminalRef.current) return;

    // Initialize xterm.js
    const term = new XTerm({
      cursorBlink: false,
      fontSize: 14,
      fontFamily: "'Symbols Nerd Font', 'JetBrains Mono', 'Fira Code', 'ui-monospace', 'SFMono-Regular', 'Menlo', 'Monaco', 'Consolas', 'monospace'",
      rescaleOverlappingGlyphs: true,
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
    const fitAddon = new FitAddon();
    const webLinksAddon = new WebLinksAddon();
    const unicode11Addon = new Unicode11Addon();

    term.loadAddon(fitAddon);
    term.loadAddon(webLinksAddon);
    term.loadAddon(unicode11Addon);
    term.unicode.activeVersion = '11';

    fitAddonRef.current = fitAddon;
    xtermRef.current = term;

    // Wait for fonts to load before opening terminal
    if (document.fonts) {
      document.fonts.ready.then(() => {
        term.open(terminalRef.current!);

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
      term.open(terminalRef.current!);
      fitAddon.fit();
    }

    // Handle terminal resize
    const handleResize = () => {
      fitAddon.fit();
    };

    window.addEventListener('resize', handleResize);

    // Cleanup
    return () => {
      window.removeEventListener('resize', handleResize);
      term.dispose();
      wsRef.current?.close();
    };
  }, []);

  const connect = () => {
    if (!xtermRef.current || wsRef.current?.readyState === WebSocket.OPEN) return;

    updateStatus('connecting');

    const ws = new WebSocket(wsUrl);
    wsRef.current = ws;

    ws.onopen = () => {
      console.log('WebSocket connected');
      updateStatus('connected');

      // Start from a clean screen before rendering snapshots
      xtermRef.current?.reset();
      xtermRef.current?.clear();

      // Send initial resize
      if (fitAddonRef.current && xtermRef.current) {
        fitAddonRef.current.fit();
        const cols = xtermRef.current.cols;
        const rows = xtermRef.current.rows;
        console.log(`Sending initial resize: ${cols}x${rows}`);
        ws.send(JSON.stringify({ type: 'resize', cols, rows }));
      }
    };

    ws.onmessage = (event) => {
      try {
        const msg = JSON.parse(event.data);
        const term = xtermRef.current;
        if (!term) return;

        switch (msg.type) {
          case 'output':
            term.write(msg.data);
            break;

          case 'connected':
            console.log(`Session ID: ${msg.session_id}`);
            console.log(`Server initial size: ${msg.cols}x${msg.rows}, Client size: ${term.cols}x${term.rows}`);

            // Apply theme if provided
            if (msg.theme) {
              applyTheme(msg.theme);
            }

            term.reset();
            term.clear();
            if (msg.initial_screen) {
              console.log('Rendering initial screen snapshot');
              term.write(msg.initial_screen);
              term.focus();
            }
            break;

          case 'resize':
            term.resize(msg.cols, msg.rows);
            console.log(`Terminal resized: ${msg.cols}x${msg.rows}`);
            if (ws.readyState === WebSocket.OPEN) {
              console.log('Requesting screen refresh after resize');
              ws.send(JSON.stringify({ type: 'refresh' }));
            }
            break;

          case 'refresh':
            console.log(`Refresh response received: ${msg.cols}x${msg.rows}`);
            console.log('=== CLIENT REFRESH DEBUG ===');
            console.log(`Client terminal size: ${term.cols}x${term.rows}`);
            console.log(`Server snapshot size: ${msg.cols}x${msg.rows}`);
            console.log(`Snapshot length: ${msg.screen_content.length} bytes`);
            console.log(`Newline count: ${(msg.screen_content.match(/\n/g) || []).length}`);
            console.log(`First 200 chars:`, msg.screen_content.substring(0, 200));
            console.log(`Snapshot starts with cursor home:`, msg.screen_content.startsWith('\x1b[H'));
            console.log('============================');

            term.reset();
            term.write(msg.screen_content);
            term.focus();
            break;

          case 'title':
            document.title = msg.title + ' - Terminal Streaming';
            console.log(`Title changed: ${msg.title}`);
            break;

          case 'bell':
            console.log('Bell received');
            break;

          case 'error':
            console.error('Server error:', msg.message);
            term.write(`\r\n\x1b[1;31mError: ${msg.message}\x1b[0m\r\n`);
            break;
        }
      } catch (err) {
        console.error('Failed to parse message:', err);
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
      if (xtermRef.current) {
        xtermRef.current.write('\r\n\x1b[1;33mDisconnected from server\x1b[0m\r\n');
      }
    };

    // Handle terminal input
    if (xtermRef.current) {
      xtermRef.current.onData((data) => {
        if (ws.readyState === WebSocket.OPEN) {
          ws.send(JSON.stringify({ type: 'input', data }));
        }
      });

      xtermRef.current.onResize(({ cols, rows }) => {
        if (ws.readyState === WebSocket.OPEN) {
          console.log(`Client resized to: ${cols}x${rows}`);
          ws.send(JSON.stringify({ type: 'resize', cols, rows }));
        }
      });
    }
  };

  const disconnect = () => {
    if (wsRef.current) {
      wsRef.current.close();
      wsRef.current = null;
    }
  };

  useEffect(() => {
    // Auto-connect on mount
    const timer = setTimeout(connect, 500);
    return () => clearTimeout(timer);
  }, [wsUrl]);

  return (
    <div ref={containerRef} className="terminal-shell">
      <div ref={terminalRef} className="flex-1 terminal-scrollbar" />
    </div>
  );
}
