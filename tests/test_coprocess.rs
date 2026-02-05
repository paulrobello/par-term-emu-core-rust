// Integration tests for coprocess system (Feature 18)
use par_term_emu_core_rust::coprocess::{CoprocessConfig, CoprocessManager};

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
    std::thread::sleep(std::time::Duration::from_millis(100));

    let output = mgr.read(id).unwrap();
    assert!(output.contains(&"hello".to_string()));
    assert!(output.contains(&"world".to_string()));

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
    std::thread::sleep(std::time::Duration::from_millis(100));

    let output = mgr.read(id).unwrap();
    assert!(output.contains(&"fed data".to_string()));

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

    std::thread::sleep(std::time::Duration::from_millis(200));

    assert_eq!(mgr.status(id), Some(false));
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

    // feed_output should NOT send to this coprocess
    mgr.feed_output(b"should not appear\n");
    std::thread::sleep(std::time::Duration::from_millis(100));

    let output = mgr.read(id).unwrap();
    assert!(output.is_empty());

    // But direct write should work
    mgr.write(id, b"direct write\n").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));

    let output = mgr.read(id).unwrap();
    assert!(output.contains(&"direct write".to_string()));

    mgr.stop(id).unwrap();
}
