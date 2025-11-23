'use client';

import { useState } from 'react';
import dynamic from 'next/dynamic';
import type { ConnectionStatus } from '@/types/terminal';

// Load Terminal component only on client side to avoid SSR issues with xterm.js
const Terminal = dynamic(() => import('@/components/Terminal'), {
  ssr: false,
  loading: () => (
    <div className="flex items-center justify-center h-full">
      <div className="text-terminal-muted">Loading terminal...</div>
    </div>
  ),
});

export default function Home() {
  // Auto-detect WebSocket URL based on current location
  const getDefaultWsUrl = () => {
    if (typeof window !== 'undefined') {
      const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
      const host = window.location.host;
      // If loaded from HTTP server, use /ws endpoint
      if (host && host !== '') {
        return `${protocol}//${host}/ws`;
      }
    }
    // Fallback for development
    return 'ws://127.0.0.1:8099';
  };

  const [wsUrl, setWsUrl] = useState(getDefaultWsUrl());
  const [status, setStatus] = useState<ConnectionStatus>('disconnected');
  const [terminalBgColor, setTerminalBgColor] = useState('#000000');

  const statusConfig = {
    disconnected: {
      color: 'bg-gray-500',
      text: 'Disconnected',
      pulse: false,
    },
    connecting: {
      color: 'bg-yellow-500',
      text: 'Connecting...',
      pulse: true,
    },
    connected: {
      color: 'bg-green-500',
      text: 'Connected',
      pulse: false,
    },
    error: {
      color: 'bg-red-500',
      text: 'Error',
      pulse: false,
    },
  };

  const currentStatus = statusConfig[status];

  return (
    <main className="flex h-screen flex-col p-3 gap-3 overflow-hidden">
      {/* Header */}
      <div className="glass rounded-2xl p-3 shadow-2xl flex-shrink-0">
        {/* Connection Settings */}
        <div className="flex items-center gap-2">
          <div className="flex-1">
            <label htmlFor="wsUrl" className="block text-xs font-medium text-terminal-muted mb-1">
              WebSocket URL
            </label>
            <input
              id="wsUrl"
              type="text"
              value={wsUrl}
              onChange={(e) => setWsUrl(e.target.value)}
              className="w-full px-2 py-1 rounded-lg bg-terminal-bg border border-terminal-border text-terminal-text placeholder-terminal-muted focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent transition-all"
              placeholder="ws://127.0.0.1:8080"
            />
          </div>

          <div className="pt-3">
            <button
              onClick={() => window.location.reload()}
              className="group relative px-4 py-1.5 rounded-lg bg-white/5 hover:bg-white/10 border border-white/10 hover:border-white/20 text-terminal-text font-medium shadow-sm hover:shadow-md transition-all duration-200 flex items-center gap-2 backdrop-blur-sm"
            >
              <svg
                className="w-4 h-4 transition-transform duration-200 group-hover:rotate-180"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
                xmlns="http://www.w3.org/2000/svg"
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"
                />
              </svg>
              Reconnect
            </button>
          </div>
        </div>
      </div>

      {/* Terminal Container */}
      <div
        className="flex-1 shadow-2xl overflow-hidden min-h-0 flex flex-col"
        style={{ backgroundColor: terminalBgColor }}
      >
        <Terminal
          wsUrl={wsUrl}
          onStatusChange={setStatus}
          onThemeChange={setTerminalBgColor}
        />
      </div>

      {/* Footer */}
      <div className="flex items-center justify-between text-terminal-muted text-sm flex-shrink-0">
        <p className="text-center flex-1">
          Powered by{' '}
          <a
            href="https://github.com/paulrobello/par-term-emu-core-rust"
            target="_blank"
            rel="noopener noreferrer"
            className="text-blue-400 hover:text-blue-300 transition-colors"
          >
            PAR Term Emu Rust Core
          </a>
          {' '}and{' '}
          <a
            href="https://github.com/xtermjs/xterm.js"
            target="_blank"
            rel="noopener noreferrer"
            className="text-blue-400 hover:text-blue-300 transition-colors"
          >
            xterm.js
          </a>
        </p>

        {/* Status Indicator */}
        <div className="flex items-center gap-2">
          <div className="status-indicator">
            {currentStatus.pulse && (
              <span className={`status-indicator-ping ${currentStatus.color}`} />
            )}
            <span className={`status-indicator-dot ${currentStatus.color}`} />
          </div>
          <span className="text-xs font-medium text-terminal-text">
            {currentStatus.text}
          </span>
        </div>
      </div>
    </main>
  );
}
