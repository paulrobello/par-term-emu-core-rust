/**
 * Terminal.tsx mouse mode-sync + wheel encoding tests (card 01a19af08).
 *
 * The real @xterm/xterm never builds its DOM under happy-dom (term.open()
 * adds no screen nodes and term.element resolves nothing), so the mouse
 * handlers' coordinate resolution cannot run against it. This file swaps in
 * a minimal xterm stub whose open() records the host div, appends the one
 * node the handlers resolve coordinates through (.xterm-screen), and lets
 * the REAL handlers run end to end: modeChanged gate, coordinate math, and
 * the createMouseMessage encoding all execute for real; only the terminal
 * widget itself is fake.
 */

import { describe, it, expect, vi, afterEach, beforeEach } from 'vitest';
import { render, cleanup, act } from '@testing-library/react';
import type { TerminalConnectionCallbacks } from '@/lib/terminal-connection';

// Capture every TerminalConnectionCallbacks object the component constructs,
// and every ClientMessage it sends, oldest first.
const constructedCallbacks: TerminalConnectionCallbacks[] = [];
type SentMessage = { message: { case?: string; value?: unknown } };
const sentMessages: SentMessage[] = [];

vi.mock('@/lib/terminal-connection', () => {
  class MockTerminalConnection {
    constructor(
      private readonly url: string,
      private readonly callbacks: TerminalConnectionCallbacks,
    ) {
      constructedCallbacks.push(callbacks);
    }
    getUrl(): string {
      return this.url;
    }
    isOpen(): boolean {
      return true;
    }
    connect(): void {}
    send(msg: SentMessage): void {
      sentMessages.push(msg);
    }
    cancelRetry(): void {}
    stopHeartbeat(): void {}
    dispose(): void {}
  }
  return { TerminalConnection: MockTerminalConnection };
});

// The xterm stub: just enough surface for Terminal.tsx's mount effect and
// its mouse handlers. open() records the host div it was given so the test
// can pin the coordinate geometry.
vi.mock('@xterm/xterm', () => {
  const state = { hosts: [] as HTMLElement[] };
  class MockTerminal {
    cols = 80;
    rows = 24;
    options: Record<string, unknown> = {};
    element: HTMLElement | null = null;
    parser = { registerCsiHandler: () => true };
    unicode = { activeVersion: '' };
    open(el: HTMLElement): void {
      this.element = el;
      state.hosts.push(el);
    }
    loadAddon(): void {}
    onData(): { dispose(): void } {
      return { dispose(): void {} };
    }
    onRender(): { dispose(): void } {
      return { dispose(): void {} };
    }
    onResize(): { dispose(): void } {
      return { dispose(): void {} };
    }
    write(): void {}
    reset(): void {}
    clear(): void {}
    focus(): void {}
    resize(c: number, r: number): void {
      this.cols = c;
      this.rows = r;
    }
    scrollLines(): void {}
    scrollToBottom(): void {}
    select(): void {}
    selectLines(): void {}
    clearSelection(): void {}
    refresh(): void {}
    dispose(): void {}
  }
  return { Terminal: MockTerminal, __mockState: state };
});

vi.mock('@xterm/addon-fit', () => ({ FitAddon: class { fit(): void {} } }));
vi.mock('@xterm/addon-webgl', () => ({ WebglAddon: class {} }));
vi.mock('@xterm/addon-web-links', () => ({ WebLinksAddon: class {} }));
vi.mock('@xterm/addon-unicode11', () => ({ Unicode11Addon: class {} }));
vi.mock('@xterm/addon-image', () => ({ ImageAddon: class {} }));

// Imported after the mocks so Terminal.tsx picks them up.
const { default: Terminal } = await import('@/components/Terminal');
const mockState = (
  (await import('@xterm/xterm')) as unknown as {
    __mockState: { hosts: HTMLElement[] };
  }
).__mockState;

beforeEach(() => {
  constructedCallbacks.length = 0;
  sentMessages.length = 0;
  mockState.hosts.length = 0;
  vi.useFakeTimers();
});

afterEach(() => {
  cleanup();
  // The unmount cleanup preserves the terminal and schedules a 100ms
  // delayed dispose that nulls the preservation. On the fake clock that
  // timer never fires otherwise, so the preserved term would leak into the
  // next test's mount, whose restore path never calls open() — advance
  // past it so every test starts with a fresh open().
  vi.advanceTimersByTime(150);
  vi.useRealTimers();
  vi.restoreAllMocks();
});

/** Render, then advance past the 500ms auto-connect timer so connect() runs. */
function renderAndConnect(props: React.ComponentProps<typeof Terminal>) {
  const result = render(<Terminal {...props} />);
  act(() => {
    vi.advanceTimersByTime(500);
  });
  return result;
}

/** A synthetic wheel event the handlers can read without DOM init support. */
function wheelEvent(deltaY: number, x: number, y: number): Event {
  const ev = new Event('wheel', { bubbles: true });
  Object.defineProperty(ev, 'deltaY', { value: deltaY });
  Object.defineProperty(ev, 'clientX', { value: x });
  Object.defineProperty(ev, 'clientY', { value: y });
  return ev;
}

/**
 * Pin the .xterm-screen rect so pixel-to-cell math is deterministic: 800x240
 * over the stub's 80x24 grid makes each cell 10x10, so clientX 105 / clientY
 * 15 is cell (col 10, row 1).
 */
function stubScreenRect(): HTMLElement {
  const host = mockState.hosts.at(-1);
  if (!host) throw new Error('xterm never opened on a host div');
  let screen = host.querySelector('.xterm-screen') as HTMLElement | null;
  if (!screen) {
    screen = document.createElement('div');
    screen.className = 'xterm-screen';
    host.appendChild(screen);
  }
  Object.defineProperty(screen, 'getBoundingClientRect', {
    configurable: true,
    value: () =>
      ({ left: 0, top: 0, width: 800, height: 240, right: 800, bottom: 240, x: 0, y: 0, toJSON: () => ({}) }),
  });
  return screen;
}

/** The most recent sent mouse message, if any. */
function lastMouse(): Record<string, unknown> | undefined {
  for (let i = sentMessages.length - 1; i >= 0; i--) {
    const m = sentMessages[i];
    if (m.message.case === 'mouse') {
      return m.message.value as Record<string, unknown>;
    }
  }
  return undefined;
}

/** Count of mouse messages sent so far. */
function mouseCount(): number {
  return sentMessages.filter((m) => m.message.case === 'mouse').length;
}

describe('Terminal mouse mode sync and wheel encoding', () => {
  it('a real tracking mode name enables mouse input; the phantom name does not', () => {
    const { container } = renderAndConnect({ wsUrl: 'ws://localhost:9999/term' });
    const screen = stubScreenRect();
    expect(container.querySelector('.terminal-scrollbar')).not.toBeNull();

    // The old phantom name enables nothing: the server never sends it, so
    // this must be checked while no real tracking mode is active yet.
    act(() => constructedCallbacks[0].onModeChanged?.('mouse_tracking', true));
    act(() => screen.dispatchEvent(wheelEvent(-100, 105, 15)));
    expect(mouseCount()).toBe(0);

    act(() => constructedCallbacks[0].onModeChanged?.('mouse_any_event', true));
    act(() => screen.dispatchEvent(wheelEvent(-100, 105, 15)));
    expect(lastMouse()).toMatchObject({ col: 10, row: 1, eventType: 'scroll', button: 64 });
  });

  it('wheel encodes raw SGR 64 up / 65 down while a tracking mode is on', () => {
    renderAndConnect({ wsUrl: 'ws://localhost:9999/term' });
    const screen = stubScreenRect();

    act(() => constructedCallbacks[0].onModeChanged?.('mouse_button_event', true));
    act(() => screen.dispatchEvent(wheelEvent(-100, 105, 15)));
    expect(lastMouse()).toMatchObject({ eventType: 'scroll', button: 64 });
    act(() => screen.dispatchEvent(wheelEvent(100, 105, 15)));
    expect(lastMouse()).toMatchObject({ eventType: 'scroll', button: 65 });
  });

  it('disabling the last tracking mode stops mouse input again', () => {
    renderAndConnect({ wsUrl: 'ws://localhost:9999/term' });
    const screen = stubScreenRect();

    act(() => constructedCallbacks[0].onModeChanged?.('mouse_normal', true));
    act(() => screen.dispatchEvent(wheelEvent(-100, 105, 15)));
    expect(lastMouse()).toMatchObject({ eventType: 'scroll', button: 64 });

    act(() => constructedCallbacks[0].onModeChanged?.('mouse_normal', false));
    const before = mouseCount();
    act(() => screen.dispatchEvent(wheelEvent(-100, 105, 15)));
    expect(mouseCount()).toBe(before);
  });
});
