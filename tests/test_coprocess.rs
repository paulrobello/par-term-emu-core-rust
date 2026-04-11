// Integration tests for coprocess system (Feature 18)
use par_term_emu_core_rust::coprocess::{CoprocessConfig, CoprocessManager};

/// Poll `f` until it returns `true` or the timeout elapses. Uses a short
/// sleep between attempts so tests stay responsive even under heavy CPU
/// contention (avoids the "fixed N ms sleep races the OS scheduler" class
/// of flake).
fn poll_until<F: FnMut() -> bool>(timeout_ms: u64, mut f: F) -> bool {
    let start = std::time::Instant::now();
    let deadline = std::time::Duration::from_millis(timeout_ms);
    while start.elapsed() < deadline {
        if f() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    f()
}

#[test]
fn test_coprocess_spawn_cat() {
    let mut mgr = CoprocessManager::new();
    let config = CoprocessConfig {
        command: "cat".to_string(),
        ..Default::default()
    };
    let id = mgr.start(config).unwrap();
    assert_eq!(mgr.list(), vec![id]);
    assert_eq!(mgr.status(id), Some(true));
    mgr.stop(id).unwrap();
}

#[test]
fn test_coprocess_write_read() {
    let mut mgr = CoprocessManager::new();
    let config = CoprocessConfig {
        command: "cat".to_string(),
        ..Default::default()
    };
    let id = mgr.start(config).unwrap();

    mgr.write(id, b"hello\nworld\n").unwrap();

    // Poll until both lines round-trip through the `cat` subprocess, or give
    // up after 2s. 2s is 20x the previous fixed 100ms sleep — enough slack
    // for loaded CI boxes without slowing the happy path.
    let mut collected: Vec<String> = Vec::new();
    let ok = poll_until(2000, || {
        collected.extend(mgr.read(id).unwrap_or_default());
        collected.iter().any(|l| l == "hello") && collected.iter().any(|l| l == "world")
    });
    assert!(ok, "expected both 'hello' and 'world', got {collected:?}");

    mgr.stop(id).unwrap();
}

#[test]
fn test_coprocess_stop() {
    let mut mgr = CoprocessManager::new();
    let config = CoprocessConfig {
        command: "cat".to_string(),
        ..Default::default()
    };
    let id = mgr.start(config).unwrap();
    assert!(mgr.stop(id).is_ok());
    assert_eq!(mgr.list().len(), 0);
}

#[test]
fn test_coprocess_manager_list() {
    let mut mgr = CoprocessManager::new();
    let id1 = mgr
        .start(CoprocessConfig {
            command: "cat".to_string(),
            ..Default::default()
        })
        .unwrap();
    let id2 = mgr
        .start(CoprocessConfig {
            command: "cat".to_string(),
            ..Default::default()
        })
        .unwrap();

    let list = mgr.list();
    assert_eq!(list.len(), 2);
    assert!(list.contains(&id1));
    assert!(list.contains(&id2));

    mgr.stop_all();
    assert_eq!(mgr.list().len(), 0);
}

#[test]
fn test_coprocess_feed_output() {
    let mut mgr = CoprocessManager::new();
    let config = CoprocessConfig {
        command: "cat".to_string(),
        copy_terminal_output: true,
        ..Default::default()
    };
    let id = mgr.start(config).unwrap();

    mgr.feed_output(b"fed data\n");

    let mut collected: Vec<String> = Vec::new();
    let ok = poll_until(2000, || {
        collected.extend(mgr.read(id).unwrap_or_default());
        collected.iter().any(|l| l == "fed data")
    });
    assert!(ok, "expected 'fed data', got {collected:?}");

    mgr.stop(id).unwrap();
}

#[test]
fn test_coprocess_dead_process() {
    let mut mgr = CoprocessManager::new();
    let config = CoprocessConfig {
        command: "true".to_string(), // exits immediately
        ..Default::default()
    };
    let id = mgr.start(config).unwrap();

    // `true` exits immediately, but the reaper thread needs a moment to
    // observe the exit. Poll instead of banking on a fixed 200ms window —
    // on a loaded CI box that window can easily slip.
    let ok = poll_until(2000, || mgr.status(id) == Some(false));
    assert!(
        ok,
        "coprocess never transitioned to dead: {:?}",
        mgr.status(id)
    );
    mgr.stop(id).unwrap();
}

#[test]
fn test_coprocess_nonexistent() {
    let mut mgr = CoprocessManager::new();
    assert!(mgr.stop(999).is_err());
    assert!(mgr.write(999, b"data").is_err());
    assert!(mgr.read(999).is_err());
    assert_eq!(mgr.status(999), None);
}

#[test]
fn test_coprocess_no_copy_output() {
    let mut mgr = CoprocessManager::new();
    let config = CoprocessConfig {
        command: "cat".to_string(),
        copy_terminal_output: false,
        ..Default::default()
    };
    let id = mgr.start(config).unwrap();

    // feed_output should NOT send to this coprocess. A short sleep here is
    // unavoidable (we're asserting a negative — that nothing appears) but
    // 200ms is plenty of headroom; any routing bug surfaces immediately.
    mgr.feed_output(b"should not appear\n");
    std::thread::sleep(std::time::Duration::from_millis(200));

    let output = mgr.read(id).unwrap();
    assert!(
        output.is_empty(),
        "expected no output from feed_output, got {output:?}"
    );

    // But direct write should work — poll for it.
    mgr.write(id, b"direct write\n").unwrap();
    let mut collected: Vec<String> = Vec::new();
    let ok = poll_until(2000, || {
        collected.extend(mgr.read(id).unwrap_or_default());
        collected.iter().any(|l| l == "direct write")
    });
    assert!(ok, "expected 'direct write', got {collected:?}");

    mgr.stop(id).unwrap();
}
