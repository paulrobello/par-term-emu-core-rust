'use client';

/**
 * Terminal Debug Component
 *
 * This component adds debugging capabilities to capture and analyze
 * the exact bytes sent to xterm.js during rendering issues.
 *
 * Usage: Import and use instead of Terminal component when debugging.
 *
 * Features:
 * - Logs all incoming data with timestamps
 * - Captures first N bytes of each output message
 * - Shows hex dump of suspicious data
 * - Tracks cursor position before/after writes
 */

import React, { useRef, useEffect, useState, useCallback, useImperativeHandle, forwardRef } from 'react';
import { Terminal as XTerm } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebglAddon } from '@xterm/addon-webgl';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { Unicode11Addon } from '@xterm/addon-unicode11';
import '@xterm/xterm/css/xterm.css';

interface TerminalDebugProps {
  wsUrl: string;
  className?: string;
}

interface DebugLog {
  timestamp: number;
  type: string;
  data: string;
  hex?: string;
  cursorBefore?: { x: number; y: number };
  cursorAfter?: { x: number; y: number };
}

const TerminalDebug = forwardRef<{ getLogs: () => DebugLog[] }, TerminalDebugProps>(
  ({ wsUrl, className }, ref) => {
    const terminalRef = useRef<HTMLDivElement>(null);
    const xtermRef = useRef<XTerm | null>(null);
    const fitAddonRef = useRef<FitAddon | null>(null);
    const wsRef = useRef<WebSocket | null>(null);
    const debugLogs = useRef<DebugLog[]>([]);
    const [isReady, setIsReady] = useState(false);

    // Expose debug logs via ref
    useImperativeHandle(ref, () => ({
      getLogs: () => debugLogs.current,
    }));

    const toHex = (str: string): string => {
      return str.split('').map(c => c.charCodeAt(0).toString(16).padStart(2, '0')).join(' ');
    };

    const logDebug = useCallback((type: string, data: string, includeHex: boolean = false) => {
      const term = xtermRef.current;
      const cursorBefore = term ? { x: term.buffer.active.cursorX, y: term.buffer.active.cursorY } : undefined;

      const log: DebugLog = {
        timestamp: Date.now(),
        type,
        data: data.length > 500 ? data.substring(0, 500) + '...' : data,
        cursorBefore,
      };

      if (includeHex && data.length < 200) {
        log.hex = toHex(data);
      }

      debugLogs.current.push(log);

      // Keep only last 1000 logs
      if (debugLogs.current.length > 1000) {
        debugLogs.current = debugLogs.current.slice(-1000);
      }

      // Log to console for immediate visibility
      console.log(`[DEBUG ${type}]`, log);
    }, []);

    useEffect(() => {
      if (!terminalRef.current) return;

      const term = new XTerm({
        cursorBlink: false,
        fontSize: 14,
        fontFamily: '"JetBrains Mono", "Fira Code", "SF Mono", Monaco, "Cascadia Code", "Roboto Mono", Consolas, "Liberation Mono", "Courier New", monospace',
        rescaleOverlappingGlyphs: true,
        theme: {
          background: '#1e1e1e',
          foreground: '#d4d4d4',
          cursor: '#aeafad',
          black: '#000000',
          red: '#cd3131',
          green: '#0dbc79',
          yellow: '#e5e510',
          blue: '#2472c8',
          magenta: '#bc3fbc',
          cyan: '#11a8cd',
          white: '#e5e5e5',
          brightBlack: '#666666',
          brightRed: '#f14c4c',
          brightGreen: '#23d18b',
          brightYellow: '#f5f543',
          brightBlue: '#3b8eea',
          brightMagenta: '#d670d6',
          brightCyan: '#29b8db',
          brightWhite: '#ffffff',
        },
        allowProposedApi: true,
      });

      const fitAddon = new FitAddon();
      term.loadAddon(fitAddon);

      try {
        const webglAddon = new WebglAddon();
        term.loadAddon(webglAddon);
        logDebug('INIT', 'WebGL addon loaded');
      } catch {
        logDebug('INIT', 'WebGL addon failed, using canvas');
      }

      const webLinksAddon = new WebLinksAddon();
      term.loadAddon(webLinksAddon);

      const unicode11Addon = new Unicode11Addon();
      term.loadAddon(unicode11Addon);
      term.unicode.activeVersion = '11';

      term.open(terminalRef.current);
      fitAddon.fit();

      xtermRef.current = term;
      fitAddonRef.current = fitAddon;
      setIsReady(true);

      logDebug('INIT', `Terminal initialized: ${term.cols}x${term.rows}`);

      // Connect WebSocket
      const ws = new WebSocket(wsUrl);
      wsRef.current = ws;

      ws.onopen = () => {
        logDebug('WS', 'Connected');

        term.reset();
        term.clear();

        if (fitAddonRef.current && xtermRef.current) {
          fitAddonRef.current.fit();
          const cols = xtermRef.current.cols;
          const rows = xtermRef.current.rows;
          logDebug('RESIZE', `Sending initial resize: ${cols}x${rows}`);
          ws.send(JSON.stringify({ type: 'resize', cols, rows }));
        }
      };

      ws.onmessage = (event) => {
        try {
          const msg = JSON.parse(event.data);

          switch (msg.type) {
            case 'output': {
              const cursorBefore = { x: term.buffer.active.cursorX, y: term.buffer.active.cursorY };

              // Log the output with hex for first 100 chars
              const preview = msg.data.substring(0, 100);
              logDebug('OUTPUT', `len=${msg.data.length} preview="${preview}"`, true);

              // Check for suspicious patterns
              if (msg.data.includes('\x1b[H') || msg.data.includes('\x1b[')) {
                const escCount = (msg.data.match(/\x1b\[/g) || []).length;
                logDebug('ESCAPE', `Found ${escCount} CSI sequences`);
              }

              term.write(msg.data);

              const cursorAfter = { x: term.buffer.active.cursorX, y: term.buffer.active.cursorY };
              logDebug('CURSOR', `moved from (${cursorBefore.x},${cursorBefore.y}) to (${cursorAfter.x},${cursorAfter.y})`);
              break;
            }

            case 'connected':
              logDebug('CONNECT', `Server size: ${msg.cols}x${msg.rows}, Client: ${term.cols}x${term.rows}`);
              term.reset();
              term.clear();
              if (msg.initial_screen) {
                logDebug('INITIAL_SCREEN', `len=${msg.initial_screen.length}`, true);
                term.write(msg.initial_screen);
              }
              break;

            case 'resize':
              logDebug('RESIZE', `Server resize to ${msg.cols}x${msg.rows}`);
              term.resize(msg.cols, msg.rows);
              break;

            case 'refresh':
              logDebug('REFRESH', `Refresh ${msg.cols}x${msg.rows}, len=${msg.screen_content?.length}`);
              term.reset();
              if (msg.screen_content) {
                term.write(msg.screen_content);
              }
              break;

            default:
              logDebug('MSG', `Unknown type: ${msg.type}`);
          }
        } catch (err) {
          logDebug('ERROR', `Parse error: ${err}`);
        }
      };

      ws.onerror = (err) => {
        logDebug('WS_ERROR', String(err));
      };

      ws.onclose = () => {
        logDebug('WS', 'Disconnected');
      };

      // Handle terminal input
      term.onData((data) => {
        if (ws.readyState === WebSocket.OPEN) {
          logDebug('INPUT', `Sending: "${data}"`, true);
          ws.send(JSON.stringify({ type: 'input', data }));
        }
      });

      // Handle resize
      const handleResize = () => {
        if (fitAddonRef.current && xtermRef.current && wsRef.current?.readyState === WebSocket.OPEN) {
          fitAddonRef.current.fit();
          const cols = xtermRef.current.cols;
          const rows = xtermRef.current.rows;
          logDebug('RESIZE', `Client resize to ${cols}x${rows}`);
          wsRef.current.send(JSON.stringify({ type: 'resize', cols, rows }));
        }
      };

      window.addEventListener('resize', handleResize);

      return () => {
        window.removeEventListener('resize', handleResize);
        ws.close();
        term.dispose();
      };
    }, [wsUrl, logDebug]);

    return (
      <div className={className}>
        <div ref={terminalRef} style={{ width: '100%', height: '100%' }} />
        {isReady && (
          <div style={{
            position: 'fixed',
            bottom: 10,
            right: 10,
            background: 'rgba(0,0,0,0.8)',
            color: 'lime',
            padding: '5px 10px',
            fontSize: '12px',
            fontFamily: 'monospace',
            borderRadius: '4px',
          }}>
            Debug Mode | Logs: {debugLogs.current.length} |
            Open console (F12) for details
          </div>
        )}
      </div>
    );
  }
);

TerminalDebug.displayName = 'TerminalDebug';

export default TerminalDebug;
