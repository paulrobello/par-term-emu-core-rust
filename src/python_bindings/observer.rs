//! Python observer bindings for push-based event delivery
//!
//! Provides `PyCallbackObserver` (sync callback) and `PyQueueObserver` (asyncio.Queue)
//! that bridge the Rust `TerminalObserver` trait to Python callables.

use std::cell::Cell;
use std::collections::HashSet;

use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict};

use crate::observer::TerminalObserver;
use crate::terminal::event_fields::{event_fields, EventField};
use crate::terminal::{TerminalEvent, TerminalEventKind};

/// Convert a `TerminalEvent` to a Python dictionary with native value types
/// (`int` for numeric fields, `bool` for flags, `None` for unset optional
/// fields, `str` for text). Shared by `poll_events()`,
/// `poll_subscribed_events()`, and observer dispatch.
pub(crate) fn event_to_dict<'py>(py: Python<'py>, event: &TerminalEvent) -> Bound<'py, PyDict> {
    let dict = PyDict::new(py);
    for (key, field) in event_fields(event) {
        match field {
            EventField::Str(s) => dict.set_item(key, s),
            EventField::Int(i) => dict.set_item(key, i),
            EventField::Bool(b) => dict.set_item(key, b),
            EventField::None => dict.set_item(key, py.None()),
        }
        .expect("PyDict::set_item with a str key and a native value cannot fail");
    }
    dict
}

thread_local! {
    /// Reentrancy guard for Python observer callbacks (ARC-016).
    ///
    /// While a Python observer callback runs on this thread, further observer
    /// events are dropped instead of dispatched. This breaks the reentrant
    /// cycle — `process()` → dispatch → Python callback → terminal method →
    /// `process()` again — that would otherwise deadlock on the non-reentrant
    /// `parking_lot` Terminal mutex already held by the calling thread.
    static IN_PY_OBSERVER_CALLBACK: Cell<bool> = const { Cell::new(false) };
}

/// Clears the reentrancy flag on drop, including when the callback panics.
struct PyCallbackReentrancyGuard;
impl Drop for PyCallbackReentrancyGuard {
    fn drop(&mut self) {
        IN_PY_OBSERVER_CALLBACK.set(false);
    }
}

/// Run `f` only if this thread is not already inside a Python observer
/// callback; arm the reentrancy guard while it runs. Returns `None` (and skips
/// dispatch) on reentrant entry. See ARC-016.
fn with_py_callback_reentrancy_guard<R>(f: impl FnOnce() -> R) -> Option<R> {
    let already_inside = IN_PY_OBSERVER_CALLBACK.replace(true);
    if already_inside {
        // Nested entry: the outer callback is still running (flag stays true).
        return None;
    }
    let _guard = PyCallbackReentrancyGuard;
    Some(f())
}

/// Observer that calls a Python callable for each event
pub(crate) struct PyCallbackObserver {
    callback: Py<PyAny>,
    subscriptions: Option<HashSet<TerminalEventKind>>,
}

impl PyCallbackObserver {
    /// Wrap `callback`; `subscriptions` limits delivery to those event kinds (`None` = all).
    pub fn new(callback: Py<PyAny>, subscriptions: Option<HashSet<TerminalEventKind>>) -> Self {
        Self {
            callback,
            subscriptions,
        }
    }
}

impl TerminalObserver for PyCallbackObserver {
    fn on_event(&self, event: &TerminalEvent) {
        // Guard against reentrant dispatch deadlocking on the Terminal mutex
        // (ARC-016): drop the event if this thread is already inside a Python
        // observer callback.
        let _ = with_py_callback_reentrancy_guard(|| {
            Python::attach(|py| {
                let dict = event_to_dict(py, event);
                if let Err(e) = self.callback.call1(py, (dict,)) {
                    log::error!("Observer callback error: {e}");
                }
            });
        });
    }

    fn subscriptions(&self) -> Option<&HashSet<TerminalEventKind>> {
        self.subscriptions.as_ref()
    }
}

/// Observer that pushes events into a Python asyncio.Queue via put_nowait
pub(crate) struct PyQueueObserver {
    queue: Py<PyAny>,
    subscriptions: Option<HashSet<TerminalEventKind>>,
}

impl PyQueueObserver {
    /// Wrap an asyncio queue; `subscriptions` limits delivery to those event kinds (`None` = all).
    pub fn new(queue: Py<PyAny>, subscriptions: Option<HashSet<TerminalEventKind>>) -> Self {
        Self {
            queue,
            subscriptions,
        }
    }
}

impl TerminalObserver for PyQueueObserver {
    fn on_event(&self, event: &TerminalEvent) {
        // Same reentrancy guard as the sync callback (ARC-016).
        let _ = with_py_callback_reentrancy_guard(|| {
            Python::attach(|py| {
                let dict = event_to_dict(py, event);
                if let Err(e) = self.queue.call_method1(py, "put_nowait", (dict,)) {
                    log::error!("Observer queue.put_nowait error: {e}");
                }
            });
        });
    }

    fn subscriptions(&self) -> Option<&HashSet<TerminalEventKind>> {
        self.subscriptions.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // pyo3 implements Send + Sync for Py<T>, so both observers get the auto
    // traits without an `unsafe impl`; this fails to compile if a future
    // field breaks that.
    #[test]
    fn python_observers_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<PyCallbackObserver>();
        assert_send_sync::<PyQueueObserver>();
    }

    #[test]
    fn reentrancy_guard_blocks_nested_dispatch_and_clears_on_exit() {
        // Start from a known state on this test thread.
        IN_PY_OBSERVER_CALLBACK.with(|c| c.set(false));

        let mut outer_ran = false;
        let mut nested_result: Option<()> = None;

        let outer = with_py_callback_reentrancy_guard(|| {
            outer_ran = true;
            // Simulate a callback re-entering dispatch on the same thread:
            nested_result = with_py_callback_reentrancy_guard(|| {
                panic!("nested dispatch must not run");
            });
            IN_PY_OBSERVER_CALLBACK.with(|c| {
                assert!(c.get(), "flag must remain true while outer callback runs");
            });
        });

        assert!(outer.is_some(), "outer (first) dispatch should run");
        assert!(outer_ran);
        assert_eq!(nested_result, None, "nested dispatch must be suppressed");
        assert!(
            !IN_PY_OBSERVER_CALLBACK.with(Cell::get),
            "flag must be cleared once the callback returns"
        );
    }

    #[test]
    fn native_renderer_keeps_unset_optional_as_none() {
        // An unset optional field keeps its key with EventField::None, which
        // event_to_dict renders as Python None rather than omitting it.
        let fields = event_fields(&TerminalEvent::HyperlinkAdded {
            url: "https://example.com".to_string(),
            row: 1,
            col: 2,
            id: None,
        });
        let id_field = fields
            .iter()
            .find(|(key, _)| key == "id")
            .map(|(_, value)| value);
        assert_eq!(id_field, Some(&EventField::None));
    }
}
