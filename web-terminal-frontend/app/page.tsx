'use client';

import { useState, useEffect } from 'react';
import dynamic from 'next/dynamic';
import type { ConnectionStatus } from '@/types/terminal';
import { OnscreenKeyboard } from '@/components/OnscreenKeyboard';

// Load Terminal component only on client side to avoid SSR issues with xterm.js
const Terminal = dynamic(() => import('@/components/Terminal'), {
  ssr: false,
  loading: () => (
    <div className="flex items-center justify-center h-full">
      <div className="text-terminal-muted">Loading terminal...</div>
    </div>
  ),
});

// Detect if we're on a mobile/touch device
const isMobileDevice = (): boolean => {
  if (typeof window === 'undefined') return false;
  return window.innerWidth < 640 || /Android|webOS|iPhone|iPad|iPod|BlackBerry|IEMobile|Opera Mini/i.test(navigator.userAgent);
};

// localStorage keys for persisting UI state
const STORAGE_KEY_SHOW_CONTROLS = 'par-term-show-controls';
const STORAGE_KEY_SHOW_KEYBOARD = 'par-term-show-keyboard';

// Helper to safely read from localStorage
const getStoredBoolean = (key: string, defaultValue: boolean): boolean => {
  if (typeof window === 'undefined') return defaultValue;
  const stored = localStorage.getItem(key);
  if (stored === null) return defaultValue;
  return stored === 'true';
};

export default function Home() {
  // Auto-detect WebSocket URL based on current location
  const getDefaultWsUrl = () => {
    // In development mode, use localhost:8099/ws
    if (process.env.NODE_ENV === 'development') {
      return 'ws://localhost:8099/ws';
    }
    if (typeof window !== 'undefined') {
      const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
      const host = window.location.host;
      // If loaded from HTTP server, use /ws endpoint
      if (host && host !== '') {
        return `${protocol}//${host}/ws`;
      }
    }
    // Fallback
    return 'ws://localhost:8099/ws';
  };

  const [wsUrl, setWsUrl] = useState(getDefaultWsUrl());
  const [status, setStatus] = useState<ConnectionStatus>('disconnected');
  const [terminalBgColor, setTerminalBgColor] = useState('#000000');
  const [showControls, setShowControls] = useState(true);
  const [refitTerminal, setRefitTerminal] = useState<(() => void) | null>(null);
  const [focusTerminal, setFocusTerminal] = useState<(() => void) | null>(null);
  const [isRetrying, setIsRetrying] = useState(false);
  const [connectControl, setConnectControl] = useState<{
    connect: () => void;
    disconnect: () => void;
    cancelRetry: () => void;
  } | null>(null);
  const [sendInput, setSendInput] = useState<((data: string) => void) | null>(null);
  const [showKeyboard, setShowKeyboard] = useState(false);

  // Load persisted UI state from localStorage on mount
  useEffect(() => {
    // Load showControls from localStorage (default: true)
    const storedShowControls = getStoredBoolean(STORAGE_KEY_SHOW_CONTROLS, true);
    setShowControls(storedShowControls);

    // Load showKeyboard from localStorage, defaulting to mobile detection if not set
    const storedKeyboard = localStorage.getItem(STORAGE_KEY_SHOW_KEYBOARD);
    if (storedKeyboard !== null) {
      setShowKeyboard(storedKeyboard === 'true');
    } else if (isMobileDevice()) {
      setShowKeyboard(true);
    }
  }, []);

  // Persist showControls to localStorage when it changes
  useEffect(() => {
    localStorage.setItem(STORAGE_KEY_SHOW_CONTROLS, String(showControls));
  }, [showControls]);

  // Persist showKeyboard to localStorage when it changes
  useEffect(() => {
    localStorage.setItem(STORAGE_KEY_SHOW_KEYBOARD, String(showKeyboard));
  }, [showKeyboard]);

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

  // Status indicator component used in multiple places
  const StatusIndicator = ({ showText = true, className = '' }: { showText?: boolean; className?: string }) => (
    <div className={`flex items-center gap-1.5 ${className}`}>
      <div className="status-indicator">
        {currentStatus.pulse && (
          <span className={`status-indicator-ping ${currentStatus.color}`} />
        )}
        <span className={`status-indicator-dot ${currentStatus.color}`} />
      </div>
      {showText && (
        <span className="text-xs font-medium text-terminal-text">
          {currentStatus.text}
        </span>
      )}
    </div>
  );

  return (
    <main className="flex h-[100dvh] flex-col overflow-hidden">
      {/* Header - Hideable */}
      {showControls && (
        <div className="glass p-2 sm:p-3 shadow-2xl flex-shrink-0 m-2 sm:m-3 mb-0 rounded-xl sm:rounded-2xl">
          <div className="flex items-center gap-2">
            {/* URL Input with inline status */}
            <div className="flex-1">
              <div className="flex items-center gap-2 mb-1">
                <label htmlFor="wsUrl" className="text-xs font-medium text-terminal-muted">
                  WebSocket URL
                </label>
                <StatusIndicator showText={true} />
              </div>
              <input
                id="wsUrl"
                type="text"
                value={wsUrl}
                onChange={(e) => setWsUrl(e.target.value)}
                className="w-full px-2 py-1 rounded-lg bg-terminal-bg border border-terminal-border text-terminal-text placeholder-terminal-muted focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent transition-all text-xs sm:text-sm"
                placeholder="ws://127.0.0.1:8080"
              />
            </div>

            {/* Reconnect/Stop button */}
            <div className="pt-4">
              {isRetrying ? (
                <button
                  onClick={() => connectControl?.cancelRetry()}
                  className="group relative p-1.5 sm:px-3 sm:py-1.5 rounded-lg bg-red-500/20 hover:bg-red-500/30 border border-red-500/50 hover:border-red-500/70 text-red-400 font-medium shadow-sm hover:shadow-md transition-all duration-200 flex items-center gap-2 backdrop-blur-sm"
                  aria-label="Stop retrying"
                >
                  <svg
                    className="w-4 h-4"
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24"
                  >
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M6 6h12v12H6z"
                    />
                  </svg>
                  <span className="hidden sm:inline">Stop</span>
                </button>
              ) : (
                <button
                  onClick={() => connectControl?.connect()}
                  className="group relative p-1.5 sm:px-3 sm:py-1.5 rounded-lg bg-white/5 hover:bg-white/10 border border-white/10 hover:border-white/20 text-terminal-text font-medium shadow-sm hover:shadow-md transition-all duration-200 flex items-center gap-2 backdrop-blur-sm"
                  aria-label="Reconnect"
                >
                  <svg
                    className="w-4 h-4 transition-transform duration-200 group-hover:rotate-180"
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24"
                  >
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"
                    />
                  </svg>
                  <span className="hidden sm:inline">Reconnect</span>
                </button>
              )}
            </div>
          </div>
        </div>
      )}

      {/* Terminal Container */}
      <div
        className={`flex-1 shadow-2xl overflow-hidden flex flex-col ${showControls ? 'm-2 sm:m-3 mt-2 rounded-lg' : ''}`}
        style={{
          backgroundColor: terminalBgColor,
          minHeight: '200px',
          // Add padding for onscreen keyboard when visible
          paddingBottom: showKeyboard ? '180px' : '0',
          transition: 'padding-bottom 0.3s ease-out',
        }}
      >
        <Terminal
          wsUrl={wsUrl}
          onStatusChange={setStatus}
          onThemeChange={setTerminalBgColor}
          onRefit={(fn) => setRefitTerminal(() => fn)}
          onFocus={(fn) => setFocusTerminal(() => fn)}
          onRetryingChange={setIsRetrying}
          onConnectControl={setConnectControl}
          onSendInput={(fn) => setSendInput(() => fn)}
        />
      </div>

      {/* Footer - Hideable */}
      {showControls && (
        <div className="flex items-center justify-center text-terminal-muted text-xs flex-shrink-0 px-2 py-1 sm:px-3 sm:py-2">
          <p className="truncate">
            <a
              href="https://github.com/paulrobello/par-term-emu-core-rust"
              target="_blank"
              rel="noopener noreferrer"
              className="text-blue-400 hover:text-blue-300 transition-colors"
            >
              PAR Term
            </a>
            {' + '}
            <a
              href="https://github.com/xtermjs/xterm.js"
              target="_blank"
              rel="noopener noreferrer"
              className="text-blue-400 hover:text-blue-300 transition-colors"
            >
              xterm.js
            </a>
          </p>
        </div>
      )}

      {/* Toggle controls button - Always visible */}
      <button
        onClick={() => {
          setShowControls(!showControls);
          // Trigger refit after state update and DOM changes
          // Use requestAnimationFrame to wait for layout, then timeout to ensure complete
          requestAnimationFrame(() => {
            setTimeout(() => {
              refitTerminal?.();
              focusTerminal?.();
            }, 150);
          });
        }}
        className={`fixed ${showControls ? 'bottom-8 sm:bottom-10' : 'bottom-2'} right-2 p-2 rounded-full bg-white/10 hover:bg-white/20 border border-white/20 text-terminal-text shadow-lg transition-all duration-200 z-50`}
        aria-label={showControls ? 'Hide controls' : 'Show controls'}
      >
        <svg
          className={`w-4 h-4 transition-transform duration-200 ${showControls ? '' : 'rotate-180'}`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d={showControls ? 'M19 9l-7 7-7-7' : 'M5 15l7-7 7 7'}
          />
        </svg>
      </button>

      {/* Onscreen Keyboard for mobile/touch devices */}
      <OnscreenKeyboard
        onInput={(data) => {
          sendInput?.(data);
          focusTerminal?.();
        }}
        isVisible={showKeyboard}
        onToggleVisibility={() => {
          setShowKeyboard(!showKeyboard);
          // Refit terminal after keyboard visibility changes
          requestAnimationFrame(() => {
            setTimeout(() => {
              refitTerminal?.();
              focusTerminal?.();
            }, 350);
          });
        }}
      />
    </main>
  );
}
