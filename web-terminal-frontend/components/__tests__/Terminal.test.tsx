/**
 * Terminal.tsx component tests (QA-111).
 *
 * The mount effect (deps `[]`) and the memoized `connect` callback (deps
 * `[wsUrl, onRetryingChange, bufferWrite]`) each ran once and closed over
 * whichever render's props were current at that moment. A parent passing a
 * *new* callback instance on a later render (a very common React pattern —
 * an inline arrow, or a `useCallback` whose own deps changed) had that
 * update silently dropped until the whole component unmounted and
 * remounted. `propsRef`, synced every render, fixes this by making every
 * one of those call sites read the latest callback through the ref instead
 * of the destructured prop.
 *
 * `onFocus`/`onRefit`/`onSendInput` are registered once, INSIDE the mount
 * effect, at mount time — propsRef.current at that instant equals the
 * first-render props either way, so those three call sites can't
 * distinguish pre-fix from post-fix behavior in a single-mount test. The
 * real distinguishing behavior lives in the callbacks handed to
 * `TerminalConnection`'s constructor inside `connect` (onStatus,
 * onThemeChange via applyTheme, onHyperlinkAdded, onUserVarChanged,
 * onSelectionChanged): those fire *after* mount, whenever the connection
 * dispatches, so a post-fix build must call the LATEST prop instance and a
 * pre-fix build would still be calling the one captured when `connect` was
 * first created. `TerminalConnection` is mocked so the test can invoke
 * those captured callbacks directly and assert which prop instance fires.
 *
 * happy-dom lacks `document.fonts`, so the component takes its synchronous
 * `term.open()` path (no font-ready wait) — see the `else` branch of the
 * mount effect.
 */

import { describe, it, expect, vi, afterEach, beforeEach } from 'vitest';
import { render, cleanup, act } from '@testing-library/react';
import type { TerminalConnectionCallbacks } from '@/lib/terminal-connection';

// Capture every TerminalConnectionCallbacks object the component
// constructs, so a test can invoke connection-driven callbacks (onStatus,
// onSelectionChanged, ...) directly without a real WebSocket.
const constructedCallbacks: TerminalConnectionCallbacks[] = [];

// Every ClientMessage the component sends, oldest first.
type SentMessage = { message: { case?: string; value?: unknown } };
const sentMessages: SentMessage[] = [];
const sentCases = (): (string | undefined)[] => sentMessages.map((m) => m.message.case);

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

// Imported after the mock so Terminal.tsx picks up the mocked class.
const { default: Terminal } = await import('@/components/Terminal');

beforeEach(() => {
  constructedCallbacks.length = 0;
  sentMessages.length = 0;
  vi.useFakeTimers();
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

/** Render, then advance past the 500ms auto-connect timer so `connect()` runs. */
function renderAndConnect(props: React.ComponentProps<typeof Terminal>) {
  const result = render(<Terminal {...props} />);
  act(() => {
    vi.advanceTimersByTime(500);
  });
  return result;
}

describe('Terminal', () => {
  it('mounts and unmounts without throwing', () => {
    const { unmount } = renderAndConnect({ wsUrl: 'ws://localhost:9999/term' });
    expect(() => unmount()).not.toThrow();
  });

  it('registers onRefit and onSendInput exactly once on mount', () => {
    const onRefit = vi.fn();
    const onSendInput = vi.fn();

    renderAndConnect({ wsUrl: 'ws://localhost:9999/term', onRefit, onSendInput });

    expect(onRefit).toHaveBeenCalledTimes(1);
    expect(onRefit.mock.calls[0][0]).toBeInstanceOf(Function);
    expect(onSendInput).toHaveBeenCalledTimes(1);
    expect(onSendInput.mock.calls[0][0]).toBeInstanceOf(Function);
  });

  it('does not re-register onFocus/onRefit/onSendInput on prop-only re-renders', () => {
    const onFocus = vi.fn();
    const onRefit = vi.fn();
    const onSendInput = vi.fn();

    const { rerender } = renderAndConnect({
      wsUrl: 'ws://localhost:9999/term',
      onFocus,
      onRefit,
      onSendInput,
    });

    expect(onFocus).toHaveBeenCalledTimes(1);
    expect(onRefit).toHaveBeenCalledTimes(1);
    expect(onSendInput).toHaveBeenCalledTimes(1);

    // Re-render with fresh callback instances (parent didn't memoize) and
    // a different fontSize — the mount effect must still not re-run (it
    // has an empty dependency array by design; only the font-size effect
    // should react).
    rerender(
      <Terminal
        wsUrl="ws://localhost:9999/term"
        fontSize={16}
        onFocus={vi.fn()}
        onRefit={vi.fn()}
        onSendInput={vi.fn()}
      />,
    );

    expect(onFocus).toHaveBeenCalledTimes(1);
    expect(onRefit).toHaveBeenCalledTimes(1);
    expect(onSendInput).toHaveBeenCalledTimes(1);
  });

  it('applies a fontSize prop change without remounting the terminal', () => {
    const onRefit = vi.fn();
    const { rerender } = renderAndConnect({
      wsUrl: 'ws://localhost:9999/term',
      fontSize: 14,
      onRefit,
    });

    expect(onRefit).toHaveBeenCalledTimes(1);

    act(() => {
      rerender(<Terminal wsUrl="ws://localhost:9999/term" fontSize={20} onRefit={onRefit} />);
    });

    // A fontSize-only change is handled by the dedicated font-size effect,
    // not a remount — onRefit's mount-time registration is not repeated.
    expect(onRefit).toHaveBeenCalledTimes(1);
  });

  it('routes onSelectionChanged through propsRef, not the connect()-time closure', () => {
    const onSelectionChangedFirst = vi.fn();
    const onSelectionChangedSecond = vi.fn();

    const { rerender } = renderAndConnect({
      wsUrl: 'ws://localhost:9999/term',
      onSelectionChanged: onSelectionChangedFirst,
    });

    expect(constructedCallbacks).toHaveLength(1);
    const { onSelectionChanged } = constructedCallbacks[0];
    expect(onSelectionChanged).toBeInstanceOf(Function);

    // Parent re-renders with a brand-new onSelectionChanged instance.
    // `connect` is memoized on [wsUrl, onRetryingChange, bufferWrite] — none
    // of which changed — so it is NOT recreated and no second
    // TerminalConnection is constructed; the fix must route through the
    // ALREADY-CAPTURED onSelectionChanged closure via propsRef, not by
    // re-registering.
    rerender(
      <Terminal
        wsUrl="ws://localhost:9999/term"
        onSelectionChanged={onSelectionChangedSecond}
      />,
    );
    expect(constructedCallbacks).toHaveLength(1);

    // Fire the same captured callback the mocked connection would call on
    // a real selectionChanged message.
    onSelectionChanged?.({ cleared: true, mode: 'chars' } as never);

    expect(onSelectionChangedFirst).not.toHaveBeenCalled();
    expect(onSelectionChangedSecond).toHaveBeenCalledTimes(1);
    expect(onSelectionChangedSecond).toHaveBeenCalledWith(undefined, true);
  });

  it('routes onStatusChange through propsRef across a prop update', () => {
    const onStatusChangeFirst = vi.fn();
    const onStatusChangeSecond = vi.fn();

    const { rerender } = renderAndConnect({
      wsUrl: 'ws://localhost:9999/term',
      onStatusChange: onStatusChangeFirst,
    });

    const { onStatus } = constructedCallbacks[0];
    rerender(
      <Terminal wsUrl="ws://localhost:9999/term" onStatusChange={onStatusChangeSecond} />,
    );

    onStatus?.('connected');

    expect(onStatusChangeFirst).not.toHaveBeenCalled();
    expect(onStatusChangeSecond).toHaveBeenCalledWith('connected');
  });
});

// A phone viewing a mux pane must never resize it by just looking: the
// pane has one size (latest resize wins), so only a deliberate action (the
// Fit tap, or opening the keyboard) may send a Resize. A desktop viewer keeps
// fitting the pane to its window. Nobody echoes a server-sent resize back.
describe('Terminal resize policy', () => {
  const connectedMsg = { sessionId: 's', cols: 120, rows: 40 } as never;
  let savedWidth: number;

  beforeEach(() => {
    savedWidth = window.innerWidth;
  });
  afterEach(() => {
    Object.defineProperty(window, 'innerWidth', { configurable: true, value: savedWidth });
  });
  const setWidth = (w: number) =>
    Object.defineProperty(window, 'innerWidth', { configurable: true, value: w });

  it('a phone does not send Resize on connect, only a refresh', () => {
    setWidth(390);
    renderAndConnect({ wsUrl: 'ws://localhost:9999/term' });
    act(() => constructedCallbacks[0].onConnected?.(connectedMsg));
    expect(sentCases()).not.toContain('resize');
    expect(sentCases()).toContain('refresh');
  });

  it('a phone does not echo a server resize', () => {
    setWidth(390);
    renderAndConnect({ wsUrl: 'ws://localhost:9999/term' });
    act(() => constructedCallbacks[0].onServerResize?.({ cols: 100, rows: 30 } as never));
    expect(sentCases()).not.toContain('resize');
  });

  it('a phone sends Resize when the pane is deliberately fitted', () => {
    setWidth(390);
    let refit: ((opts?: { resizePane?: boolean }) => void) | undefined;
    renderAndConnect({ wsUrl: 'ws://localhost:9999/term', onRefit: (fn) => { refit = fn; } });
    act(() => {
      refit?.();
      vi.advanceTimersByTime(100);
    });
    expect(sentCases()).not.toContain('resize');
    act(() => {
      refit?.({ resizePane: true });
      vi.advanceTimersByTime(100);
    });
    expect(sentCases()).toContain('resize');
  });

  it('a desktop still sends its size on connect', () => {
    setWidth(1280);
    renderAndConnect({ wsUrl: 'ws://localhost:9999/term' });
    act(() => constructedCallbacks[0].onConnected?.(connectedMsg));
    expect(sentCases()).toContain('resize');
  });

  it('a desktop does not echo a server resize', () => {
    setWidth(1280);
    renderAndConnect({ wsUrl: 'ws://localhost:9999/term' });
    // A size no earlier test used: the StrictMode-preserved terminal can
    // carry over between tests, and resizing to its current size is a no-op.
    act(() => constructedCallbacks[0].onServerResize?.({ cols: 97, rows: 29 } as never));
    expect(sentCases()).not.toContain('resize');
  });
});
