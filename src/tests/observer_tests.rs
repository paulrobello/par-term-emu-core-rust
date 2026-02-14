// Unit tests for the observer system
//
// These tests verify observer registration, removal, event dispatch, subscription
// filtering, and backward compatibility with poll_events().
// Included via include!() macro in terminal/mod.rs to maintain private field access.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use crate::observer::TerminalObserver;

/// Mock observer that records events for test assertions
struct MockObserver {
    zone_events: Mutex<Vec<String>>,
    command_events: Mutex<Vec<String>>,
    environment_events: Mutex<Vec<String>>,
    screen_events: Mutex<Vec<String>>,
    all_events: Mutex<Vec<String>>,
    subscriptions: Option<HashSet<TerminalEventKind>>,
}

impl MockObserver {
    fn new() -> Self {
        Self {
            zone_events: Mutex::new(Vec::new()),
            command_events: Mutex::new(Vec::new()),
            environment_events: Mutex::new(Vec::new()),
            screen_events: Mutex::new(Vec::new()),
            all_events: Mutex::new(Vec::new()),
            subscriptions: None,
        }
    }

    fn with_subscriptions(subs: HashSet<TerminalEventKind>) -> Self {
        Self {
            subscriptions: Some(subs),
            ..Self::new()
        }
    }

    fn event_type_str(event: &TerminalEvent) -> String {
        match event {
            TerminalEvent::BellRang(_) => "bell".to_string(),
            TerminalEvent::TitleChanged(t) => format!("title:{t}"),
            TerminalEvent::ZoneOpened { zone_id, .. } => format!("zone_opened:{zone_id}"),
            TerminalEvent::ZoneClosed { zone_id, .. } => format!("zone_closed:{zone_id}"),
            TerminalEvent::ZoneScrolledOut { zone_id, .. } => {
                format!("zone_scrolled_out:{zone_id}")
            }
            TerminalEvent::ShellIntegrationEvent { event_type, .. } => {
                format!("shell:{event_type}")
            }
            TerminalEvent::CwdChanged(c) => format!("cwd:{}", c.new_cwd),
            TerminalEvent::EnvironmentChanged { key, value, .. } => format!("env:{key}={value}"),
            TerminalEvent::SizeChanged(c, r) => format!("size:{c}x{r}"),
            _ => "other".to_string(),
        }
    }

    fn all_event_count(&self) -> usize {
        self.all_events.lock().unwrap().len()
    }

    fn zone_event_count(&self) -> usize {
        self.zone_events.lock().unwrap().len()
    }

    fn screen_event_count(&self) -> usize {
        self.screen_events.lock().unwrap().len()
    }

    fn command_event_count(&self) -> usize {
        self.command_events.lock().unwrap().len()
    }

    #[allow(dead_code)]
    fn environment_event_count(&self) -> usize {
        self.environment_events.lock().unwrap().len()
    }
}

impl TerminalObserver for MockObserver {
    fn on_zone_event(&self, event: &TerminalEvent) {
        self.zone_events
            .lock()
            .unwrap()
            .push(Self::event_type_str(event));
    }

    fn on_command_event(&self, event: &TerminalEvent) {
        self.command_events
            .lock()
            .unwrap()
            .push(Self::event_type_str(event));
    }

    fn on_environment_event(&self, event: &TerminalEvent) {
        self.environment_events
            .lock()
            .unwrap()
            .push(Self::event_type_str(event));
    }

    fn on_screen_event(&self, event: &TerminalEvent) {
        self.screen_events
            .lock()
            .unwrap()
            .push(Self::event_type_str(event));
    }

    fn on_event(&self, event: &TerminalEvent) {
        self.all_events
            .lock()
            .unwrap()
            .push(Self::event_type_str(event));
    }

    fn subscriptions(&self) -> Option<&HashSet<TerminalEventKind>> {
        self.subscriptions.as_ref()
    }
}

#[test]
fn test_observer_registration() {
    let mut term = Terminal::new(80, 24);
    let observer = Arc::new(MockObserver::new());
    let id = term.add_observer(observer);

    assert!(id > 0, "Observer ID should be positive");
    assert_eq!(term.observer_count(), 1, "Should have exactly 1 observer");
}

#[test]
fn test_observer_removal() {
    let mut term = Terminal::new(80, 24);
    let observer = Arc::new(MockObserver::new());
    let id = term.add_observer(observer);

    assert_eq!(term.observer_count(), 1);

    let removed = term.remove_observer(id);
    assert!(removed, "remove_observer should return true for valid ID");
    assert_eq!(
        term.observer_count(),
        0,
        "Observer count should be 0 after removal"
    );

    let removed_again = term.remove_observer(id);
    assert!(
        !removed_again,
        "remove_observer should return false for already-removed ID"
    );
}

#[test]
fn test_observer_unique_ids() {
    let mut term = Terminal::new(80, 24);

    let id1 = term.add_observer(Arc::new(MockObserver::new()));
    let id2 = term.add_observer(Arc::new(MockObserver::new()));
    let id3 = term.add_observer(Arc::new(MockObserver::new()));

    assert_ne!(id1, id2, "Observer IDs must be unique");
    assert_ne!(id2, id3, "Observer IDs must be unique");
    assert_ne!(id1, id3, "Observer IDs must be unique");
    assert_eq!(term.observer_count(), 3);
}

#[test]
fn test_observer_receives_bell_event() {
    let mut term = Terminal::new(80, 24);
    let observer = Arc::new(MockObserver::new());
    let obs_ref = Arc::clone(&observer);
    term.add_observer(observer);

    // Process a bell character
    term.process(b"\x07");

    assert!(
        obs_ref.screen_event_count() > 0,
        "on_screen_event should be called for bell"
    );
    assert!(
        obs_ref.all_event_count() > 0,
        "on_event should be called for bell"
    );

    let all = obs_ref.all_events.lock().unwrap();
    assert!(
        all.iter().any(|e| e == "bell"),
        "Should contain bell event, got: {:?}",
        *all
    );
}

#[test]
fn test_observer_receives_title_change() {
    let mut term = Terminal::new(80, 24);
    let observer = Arc::new(MockObserver::new());
    let obs_ref = Arc::clone(&observer);
    term.add_observer(observer);

    // OSC 0 ; title BEL sets the terminal title
    term.process(b"\x1b]0;Hello World\x07");

    let all = obs_ref.all_events.lock().unwrap();
    assert!(
        all.iter().any(|e| e == "title:Hello World"),
        "Should contain title change event, got: {:?}",
        *all
    );
}

#[test]
fn test_observer_zone_routing() {
    let mut term = Terminal::new(80, 24);
    let observer = Arc::new(MockObserver::new());
    let obs_ref = Arc::clone(&observer);
    term.add_observer(observer);

    // OSC 133;A is prompt_start which triggers zone events
    term.process(b"\x1b]133;A\x07");

    assert!(
        obs_ref.zone_event_count() > 0,
        "on_zone_event should be called for zone-related events"
    );

    let zone_events = obs_ref.zone_events.lock().unwrap();
    assert!(
        zone_events.iter().any(|e| e.starts_with("zone_opened:")),
        "Should contain zone_opened event, got: {:?}",
        *zone_events
    );
}

#[test]
fn test_observer_command_routing() {
    let mut term = Terminal::new(80, 24);
    let observer = Arc::new(MockObserver::new());
    let obs_ref = Arc::clone(&observer);
    term.add_observer(observer);

    // Full shell integration cycle:
    // A = prompt_start, B = command_start, C = command_executed, D = command_finished
    term.process(b"\x1b]133;A\x07");
    term.process(b"\x1b]133;B\x07");
    term.process(b"\x1b]133;C\x07");
    term.process(b"\x1b]133;D\x07");

    assert!(
        obs_ref.command_event_count() > 0,
        "on_command_event should be called for shell integration events"
    );

    let cmd_events = obs_ref.command_events.lock().unwrap();
    assert!(
        cmd_events
            .iter()
            .any(|e| e.starts_with("shell:prompt_start")),
        "Should contain prompt_start, got: {:?}",
        *cmd_events
    );
    assert!(
        cmd_events
            .iter()
            .any(|e| e.starts_with("shell:command_executed")),
        "Should contain command_executed, got: {:?}",
        *cmd_events
    );
}

#[test]
fn test_observer_subscription_filter() {
    let mut term = Terminal::new(80, 24);

    // Observer only subscribed to TitleChanged events
    let subs = HashSet::from([TerminalEventKind::TitleChanged]);
    let observer = Arc::new(MockObserver::with_subscriptions(subs));
    let obs_ref = Arc::clone(&observer);
    term.add_observer(observer);

    // Process a bell -- should NOT reach the filtered observer
    term.process(b"\x07");
    assert_eq!(
        obs_ref.all_event_count(),
        0,
        "Bell should be filtered out by subscription"
    );

    // Process a title change -- should reach the observer
    term.process(b"\x1b]0;Filtered Title\x07");

    let all = obs_ref.all_events.lock().unwrap();
    assert!(
        all.iter().any(|e| e == "title:Filtered Title"),
        "Title change should pass through subscription filter, got: {:?}",
        *all
    );
}

#[test]
fn test_multiple_observers() {
    let mut term = Terminal::new(80, 24);

    let observer1 = Arc::new(MockObserver::new());
    let obs1_ref = Arc::clone(&observer1);
    term.add_observer(observer1);

    let observer2 = Arc::new(MockObserver::new());
    let obs2_ref = Arc::clone(&observer2);
    term.add_observer(observer2);

    // Process a bell
    term.process(b"\x07");

    // Both observers should receive the bell event
    let all1 = obs1_ref.all_events.lock().unwrap();
    let all2 = obs2_ref.all_events.lock().unwrap();

    assert!(
        all1.iter().any(|e| e == "bell"),
        "Observer 1 should receive bell, got: {:?}",
        *all1
    );
    assert!(
        all2.iter().any(|e| e == "bell"),
        "Observer 2 should receive bell, got: {:?}",
        *all2
    );
}

#[test]
fn test_poll_events_still_works_with_observers() {
    let mut term = Terminal::new(80, 24);

    let observer = Arc::new(MockObserver::new());
    let obs_ref = Arc::clone(&observer);
    term.add_observer(observer);

    // Process a bell
    term.process(b"\x07");

    // Observer should have received the event
    assert!(
        obs_ref.all_event_count() > 0,
        "Observer should receive events"
    );

    // poll_events should still return events for backward compatibility
    let events = term.poll_events();
    assert!(
        !events.is_empty(),
        "poll_events should still return events even with observers registered"
    );
    assert!(
        events.iter().any(|e| matches!(e, TerminalEvent::BellRang(_))),
        "poll_events should contain the bell event"
    );
}
