//! ARC-001 regression tests: observer dispatch deferred out of the exclusive
//! write-lock scope via `Terminal::process_deferred` +
//! `ObserverDispatchBatch::deliver`.
//!
//! These exercise the batch-building/delivery contract directly (ordering,
//! process()-equivalence, empty-batch handling). The end-to-end proof that
//! `PtySession`'s reader thread actually releases its `RwLock<Terminal>`
//! write guard before delivering a batch lives in
//! `pty_session::tests::test_observer_dispatch_does_not_hold_write_lock`,
//! since that requires a real PTY + reader thread to observe.

use crate::observer::TerminalObserver;
use crate::terminal::*;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// Records a description of every event delivered to `on_event`, in
/// delivery order.
struct OrderRecordingObserver {
    seen: Mutex<Vec<String>>,
}

impl OrderRecordingObserver {
    fn new() -> Self {
        Self {
            seen: Mutex::new(Vec::new()),
        }
    }
}

fn describe(event: &TerminalEvent) -> String {
    match event {
        TerminalEvent::BellRang(_) => "bell".to_string(),
        TerminalEvent::TitleChanged(t) => format!("title:{t}"),
        other => format!("{:?}", other.kind()),
    }
}

impl TerminalObserver for OrderRecordingObserver {
    fn on_event(&self, event: &TerminalEvent) {
        self.seen.lock().unwrap().push(describe(event));
    }
}

#[test]
fn process_deferred_batch_preserves_event_order() {
    let mut term = Terminal::new(80, 24);
    let observer = Arc::new(OrderRecordingObserver::new());
    term.add_observer(observer.clone());

    // Three separate `process_deferred()` calls, each producing one
    // distinct event -- mirroring how the PTY reader thread calls
    // `process_deferred()` once per PTY read.
    let batch1 = term.process_deferred(b"\x07"); // bell
    let batch2 = term.process_deferred(b"\x1b]0;First\x07"); // title change
    let batch3 = term.process_deferred(b"\x1b]0;Second\x07"); // title change

    assert!(
        observer.seen.lock().unwrap().is_empty(),
        "nothing should be delivered before deliver() is called"
    );

    // Deliver in production order, exactly as pty_session.rs does
    // (deliver each batch right after the write guard that produced it
    // is dropped, before reading the next chunk).
    batch1.deliver();
    batch2.deliver();
    batch3.deliver();

    let seen = observer.seen.lock().unwrap();
    assert_eq!(
        *seen,
        vec![
            "bell".to_string(),
            "title:First".to_string(),
            "title:Second".to_string(),
        ],
        "observer must see events in the order they were produced"
    );
}

#[test]
fn process_deferred_matches_process_for_observer_delivery() {
    // Feed the identical byte sequence to two terminals: one via the
    // synchronous `process()` path, one via
    // `process_deferred()` + `deliver()`. Observer-visible results must
    // be identical -- deferring delivery must not change *what* an
    // observer sees, only *when* (and under what lock) it runs.
    let mut term_sync = Terminal::new(80, 24);
    let obs_sync = Arc::new(OrderRecordingObserver::new());
    term_sync.add_observer(obs_sync.clone());

    let mut term_deferred = Terminal::new(80, 24);
    let obs_deferred = Arc::new(OrderRecordingObserver::new());
    term_deferred.add_observer(obs_deferred.clone());

    let inputs: [&[u8]; 3] = [b"\x07", b"\x1b]0;Hello\x07", b"\x1b]133;A\x07"];
    for data in inputs {
        term_sync.process(data);
        let batch = term_deferred.process_deferred(data);
        batch.deliver();
    }

    assert_eq!(
        *obs_sync.seen.lock().unwrap(),
        *obs_deferred.seen.lock().unwrap(),
        "process_deferred()+deliver() must be observer-equivalent to process()"
    );
}

#[test]
fn empty_batch_when_no_observers_or_no_new_events() {
    let mut term = Terminal::new(80, 24);

    // No observers registered yet -- the bell event is queued into
    // `terminal_events` but `events_dispatched_up_to` is left untouched
    // (matching `dispatch_events()`'s original early return), so it
    // stays pending for whenever an observer *is* registered.
    let batch = term.process_deferred(b"\x07");
    assert!(
        batch.is_empty(),
        "batch should be empty when no observers are registered"
    );
    batch.deliver(); // must be a safe no-op

    let observer = Arc::new(OrderRecordingObserver::new());
    term.add_observer(observer.clone());

    // The previously-queued bell is still undelivered; the next call
    // flushes it even though no new bytes are processed.
    let batch = term.process_deferred(b"");
    assert!(
        !batch.is_empty(),
        "the bell queued before the observer was registered should now be included"
    );
    batch.deliver();
    assert_eq!(*observer.seen.lock().unwrap(), vec!["bell".to_string()]);

    // Now fully caught up: plain text queues no new TerminalEvent, so
    // the batch is empty.
    let batch = term.process_deferred(b"no events here");
    assert!(
        batch.is_empty(),
        "batch should be empty when no new events were queued"
    );
}

fn title(s: &str) -> TerminalEvent {
    TerminalEvent::TitleChanged(s.to_string())
}

fn cwd(path: &str) -> TerminalEvent {
    TerminalEvent::CwdChanged(CwdChange {
        old_cwd: None,
        new_cwd: path.to_string(),
        hostname: None,
        username: None,
        timestamp: 0,
    })
}

fn two_observers(
    term: &mut Terminal,
) -> (Arc<OrderRecordingObserver>, Arc<OrderRecordingObserver>) {
    let a = Arc::new(OrderRecordingObserver::new());
    let b = Arc::new(OrderRecordingObserver::new());
    term.add_observer(a.clone());
    term.add_observer(b.clone());
    (a, b)
}

fn clear(a: &OrderRecordingObserver, b: &OrderRecordingObserver) {
    a.seen.lock().unwrap().clear();
    b.seen.lock().unwrap().clear();
}

/// Card 01a11486: a typed poll that removes an already-dispatched event
/// must not leave the dispatch index past the end of the queue, or the
/// next pushed event is never delivered to observers.
#[test]
fn typed_poll_of_dispatched_event_does_not_lose_next_event() {
    let mut term = Terminal::new(80, 24);
    let (a, b) = two_observers(&mut term);

    for i in 0..9 {
        term.events.push(title(&format!("t{i}")));
    }
    term.events.push(cwd("/tmp"));
    term.take_observer_dispatch_batch().deliver();
    assert_eq!(a.seen.lock().unwrap().len(), 10);
    clear(&a, &b);

    assert_eq!(term.poll_cwd_events().len(), 1);
    term.events.push(title("new"));
    term.take_observer_dispatch_batch().deliver();

    for obs in [&a, &b] {
        assert_eq!(*obs.seen.lock().unwrap(), vec!["title:new".to_string()]);
    }
}

/// Removing a dispatched event below the index shifts later events
/// left; without adjusting the index the first undispatched event is
/// skipped even when the queue length never returns to the old index.
#[test]
fn typed_poll_of_dispatched_event_does_not_skip_shifted_event() {
    let mut term = Terminal::new(80, 24);
    let (a, b) = two_observers(&mut term);

    term.events.push(cwd("/a"));
    term.events.push(title("old"));
    term.take_observer_dispatch_batch().deliver();
    clear(&a, &b);

    assert_eq!(term.poll_cwd_events().len(), 1);
    term.events.push(title("x"));
    term.events.push(title("y"));
    term.take_observer_dispatch_batch().deliver();

    for obs in [&a, &b] {
        assert_eq!(
            *obs.seen.lock().unwrap(),
            vec!["title:x".to_string(), "title:y".to_string()]
        );
    }
}

/// Extracting a not-yet-dispatched event must not rewind the index over
/// events observers already received.
#[test]
fn typed_poll_of_undispatched_event_does_not_redispatch() {
    let mut term = Terminal::new(80, 24);
    let (a, b) = two_observers(&mut term);

    term.events.push(title("old"));
    term.take_observer_dispatch_batch().deliver();
    clear(&a, &b);

    term.events.push(cwd("/a"));
    assert_eq!(term.poll_cwd_events().len(), 1);
    term.events.push(title("new"));
    term.take_observer_dispatch_batch().deliver();

    for obs in [&a, &b] {
        assert_eq!(*obs.seen.lock().unwrap(), vec!["title:new".to_string()]);
    }
}

/// `poll_subscribed_events` with a filter removes matched events the
/// same way the typed polls do.
#[test]
fn subscribed_poll_of_dispatched_event_does_not_lose_next_event() {
    let mut term = Terminal::new(80, 24);
    let (a, b) = two_observers(&mut term);

    term.events.push(title("old"));
    term.events.push(cwd("/a"));
    term.take_observer_dispatch_batch().deliver();
    clear(&a, &b);

    let mut filter = HashSet::new();
    filter.insert(TerminalEventKind::CwdChanged);
    term.set_event_subscription(filter);
    assert_eq!(term.poll_subscribed_events().len(), 1);
    term.events.push(title("new"));
    term.take_observer_dispatch_batch().deliver();

    for obs in [&a, &b] {
        assert_eq!(*obs.seen.lock().unwrap(), vec!["title:new".to_string()]);
    }
}

/// The generic drain resets the index with the queue; pinned as a guard.
#[test]
fn generic_poll_then_push_reaches_observers() {
    let mut term = Terminal::new(80, 24);
    let (a, b) = two_observers(&mut term);

    term.events.push(title("old"));
    term.take_observer_dispatch_batch().deliver();
    clear(&a, &b);

    assert_eq!(term.poll_events().len(), 1);
    term.events.push(title("new"));
    term.take_observer_dispatch_batch().deliver();

    for obs in [&a, &b] {
        assert_eq!(*obs.seen.lock().unwrap(), vec!["title:new".to_string()]);
    }
}
