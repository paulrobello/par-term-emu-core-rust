'use client';

import { useState } from 'react';
import Terminal, { ConnectionStatus } from '@/components/Terminal';

export default function Home() {
  const [wsUrl, setWsUrl] = useState('ws://127.0.0.1:8080');
  const [status, setStatus] = useState<ConnectionStatus>('disconnected');

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
    <main className="flex min-h-screen flex-col p-6 gap-6">
      {/* Header */}
      <div className="glass rounded-2xl p-6 shadow-2xl">
        <div className="flex items-center justify-between">
          <div>
            <h1 className="text-3xl font-bold text-gradient mb-2">
              Terminal Streaming
            </h1>
            <p className="text-terminal-muted text-sm">
              Modern web-based terminal emulator with real-time streaming
            </p>
          </div>

          {/* Status Indicator */}
          <div className="flex items-center gap-3">
            <div className="status-indicator">
              {currentStatus.pulse && (
                <span className={`status-indicator-ping ${currentStatus.color}`} />
              )}
              <span className={`status-indicator-dot ${currentStatus.color}`} />
            </div>
            <span className="text-sm font-medium text-terminal-text">
              {currentStatus.text}
            </span>
          </div>
        </div>

        {/* Connection Settings */}
        <div className="mt-6 flex items-center gap-4">
          <div className="flex-1">
            <label htmlFor="wsUrl" className="block text-xs font-medium text-terminal-muted mb-2">
              WebSocket URL
            </label>
            <input
              id="wsUrl"
              type="text"
              value={wsUrl}
              onChange={(e) => setWsUrl(e.target.value)}
              className="w-full px-4 py-2 rounded-lg bg-terminal-bg border border-terminal-border text-terminal-text placeholder-terminal-muted focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent transition-all"
              placeholder="ws://127.0.0.1:8080"
            />
          </div>

          <div className="pt-6">
            <button
              onClick={() => window.location.reload()}
              className="px-6 py-2 rounded-lg bg-gradient-to-r from-blue-500 to-purple-500 hover:from-blue-600 hover:to-purple-600 text-white font-medium shadow-lg hover:shadow-xl transition-all duration-200 transform hover:scale-105 active:scale-95"
            >
              Reconnect
            </button>
          </div>
        </div>
      </div>

      {/* Terminal Container */}
      <div className="flex-1 glass rounded-2xl shadow-2xl overflow-hidden min-h-[600px]">
        <div className="h-full p-4">
          <Terminal wsUrl={wsUrl} onStatusChange={setStatus} />
        </div>
      </div>

      {/* Footer */}
      <div className="text-center text-terminal-muted text-sm">
        <p>
          Powered by{' '}
          <a
            href="https://github.com/xtermjs/xterm.js"
            target="_blank"
            rel="noopener noreferrer"
            className="text-blue-400 hover:text-blue-300 transition-colors"
          >
            xterm.js
          </a>
          {' '}and{' '}
          <a
            href="https://nextjs.org"
            target="_blank"
            rel="noopener noreferrer"
            className="text-blue-400 hover:text-blue-300 transition-colors"
          >
            Next.js
          </a>
        </p>
      </div>
    </main>
  );
}
