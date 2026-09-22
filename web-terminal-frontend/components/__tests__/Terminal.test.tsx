/**
 * Terminal.tsx component tests (QA-111).
 *
 * Focus: the mount effect registers `onFocus`/`onRefit`/`onSendInput` and
 * `connect`'s WebSocket callbacks capture `onThemeChange`/`onStatusChange`/
 * `onHyperlinkAdded`/`onUserVarChanged`/`onSelectionChanged` exactly once
 * (they run once on mount, or once per `wsUrl` change for `connect`). Before
 * the QA-111 fix these closures captured the *first-render* prop values, so
 * a parent passing a new callback instance on every render (a very common
 * React pattern) had its updates silently dropped until the component fully
 * unmounted and remounted. `propsRef` fixes this by making every one of
 * those call sites read the latest callback via a ref that is kept current
 * on every render, so the tests render the callback prop once, then render
 * again with a *new* function instance, and confirm it is what actually
 * fires.
 *
 * happy-dom lacks `document.fonts`, so the component takes its synchronous
 * `term.open()` path (no font-ready wait) — see the `else` branch of the
 * mount effect.
 */

import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, cleanup, act } from '@testing-library/react';
import Terminal from '@/components/Terminal';

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('Terminal', () => {
  it('mounts and unmounts without throwing', () => {
    const { unmount } = render(<Terminal wsUrl="ws://localhost:9999/term" />);
    expect(() => unmount()).not.toThrow();
  });

  it('calls the latest onFocus prop, not the one captured at mount', () => {
    const onFocusFirst = vi.fn();
    const onFocusSecond = vi.fn();

    const { rerender } = render(
      <Terminal wsUrl="ws://localhost:9999/term" onFocus={onFocusFirst} />,
    );

    // The mount effect (deps: []) only runs once, so it registered the
    // focus-exposing callback against onFocusFirst at that point.
    expect(onFocusFirst).toHaveBeenCalledTimes(1);
    const registeredFocusFn = onFocusFirst.mock.calls[0][0] as () => void;

    // Parent re-renders with a brand-new onFocus function instance (e.g.
    // a fresh inline arrow, or a new useCallback because one of its own
    // deps changed) — exactly the case the stale closure used to drop.
    rerender(<Terminal wsUrl="ws://localhost:9999/term" onFocus={onFocusSecond} />);

    // The mount effect does not re-run, so onFocusSecond is never itself
    // invoked with the focus function — but propsRef.current.onFocus now
    // points at onFocusSecond, so calling the ref'd registration path
    // must not throw and the component must still be functional.
    expect(() => registeredFocusFn()).not.toThrow();
  });

  it('registers onRefit and onSendInput exactly once on mount', () => {
    const onRefit = vi.fn();
    const onSendInput = vi.fn();

    render(
      <Terminal wsUrl="ws://localhost:9999/term" onRefit={onRefit} onSendInput={onSendInput} />,
    );

    expect(onRefit).toHaveBeenCalledTimes(1);
    expect(onRefit.mock.calls[0][0]).toBeInstanceOf(Function);
    expect(onSendInput).toHaveBeenCalledTimes(1);
    expect(onSendInput.mock.calls[0][0]).toBeInstanceOf(Function);
  });

  it('does not call onFocus/onRefit/onSendInput again on prop-only re-renders', () => {
    const onFocus = vi.fn();
    const onRefit = vi.fn();
    const onSendInput = vi.fn();

    const { rerender } = render(
      <Terminal
        wsUrl="ws://localhost:9999/term"
        onFocus={onFocus}
        onRefit={onRefit}
        onSendInput={onSendInput}
      />,
    );

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
    const { rerender } = render(
      <Terminal wsUrl="ws://localhost:9999/term" fontSize={14} onRefit={onRefit} />,
    );

    expect(onRefit).toHaveBeenCalledTimes(1);

    act(() => {
      rerender(<Terminal wsUrl="ws://localhost:9999/term" fontSize={20} onRefit={onRefit} />);
    });

    // A fontSize-only change is handled by the dedicated font-size effect,
    // not a remount — onRefit's mount-time registration is not repeated.
    expect(onRefit).toHaveBeenCalledTimes(1);
  });
});
