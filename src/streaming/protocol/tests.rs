use super::*;

#[test]
fn test_server_message_output_serialization() {
    let msg = ServerMessage::output("Hello, World!".to_string());
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"output"#));
    assert!(json.contains(r#""data":"Hello, World!"#));

    // Deserialize back
    let deserialized: ServerMessage = serde_json::from_str(&json).unwrap();
    match deserialized {
        ServerMessage::Output {
            data, timestamp, ..
        } => {
            assert_eq!(data, "Hello, World!");
            assert_eq!(timestamp, None);
        }
        _ => panic!("Wrong message type"),
    }
}

#[test]
fn test_server_message_resize_serialization() {
    let msg = ServerMessage::resize(80, 24);
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"resize"#));
    assert!(json.contains(r#""cols":80"#));
    assert!(json.contains(r#""rows":24"#));
}

#[test]
fn test_client_message_input_serialization() {
    let msg = ClientMessage::input("ls\n".to_string());
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"input"#));
    assert!(json.contains(r#""data":"ls\n"#));

    // Deserialize back
    let deserialized: ClientMessage = serde_json::from_str(&json).unwrap();
    match deserialized {
        ClientMessage::Input { data } => {
            assert_eq!(data, "ls\n");
        }
        _ => panic!("Wrong message type"),
    }
}

#[test]
fn mouse_event_type_serializes_as_its_wire_name() {
    for event_type in [
        MouseEventType::Press,
        MouseEventType::Release,
        MouseEventType::Move,
        MouseEventType::Scroll,
    ] {
        let msg = ClientMessage::mouse(1, 2, 0, false, false, false, event_type);
        let json = serde_json::to_string(&msg).unwrap();
        assert!(
            json.contains(&format!(r#""event_type":"{}""#, event_type.as_str())),
            "{json}"
        );
        let ClientMessage::Mouse {
            event_type: got, ..
        } = serde_json::from_str::<ClientMessage>(&json).unwrap()
        else {
            panic!("Wrong message type");
        };
        assert_eq!(got, event_type);
        assert_eq!(MouseEventType::parse(event_type.as_str()), Some(event_type));
    }
    assert_eq!(MouseEventType::parse("bogus"), None);
}

#[test]
fn test_client_message_resize_serialization() {
    let msg = ClientMessage::resize(100, 30);
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"resize"#));
    assert!(json.contains(r#""cols":100"#));
    assert!(json.contains(r#""rows":30"#));
}

#[test]
fn test_server_message_error_serialization() {
    let msg = ServerMessage::error("Something went wrong".to_string());
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"error"#));
    assert!(json.contains(r#""message":"Something went wrong"#));
}

#[test]
fn test_server_message_connected_serialization() {
    let msg = ServerMessage::connected_builder(80, 24, "session-123".to_string()).build();
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"connected"#));
    assert!(json.contains(r#""session_id":"session-123"#));
    assert!(!json.contains(r#""initial_screen"#)); // Should be omitted when None
}

#[test]
fn test_event_type_serialization() {
    let events = vec![EventType::Output, EventType::Bell];
    let json = serde_json::to_string(&events).unwrap();
    assert!(json.contains(r#""output"#));
    assert!(json.contains(r#""bell"#));
}

#[test]
fn test_theme_info_serialization() {
    let theme = ThemeInfo {
        name: "test-theme".to_string(),
        background: (0, 0, 0),
        foreground: (255, 255, 255),
        normal: [
            (0, 0, 0),
            (255, 0, 0),
            (0, 255, 0),
            (255, 255, 0),
            (0, 0, 255),
            (255, 0, 255),
            (0, 255, 255),
            (255, 255, 255),
        ],
        bright: [
            (128, 128, 128),
            (255, 128, 128),
            (128, 255, 128),
            (255, 255, 128),
            (128, 128, 255),
            (255, 128, 255),
            (128, 255, 255),
            (255, 255, 255),
        ],
    };

    let json = serde_json::to_string(&theme).unwrap();
    assert!(json.contains(r#""name":"test-theme"#));
    assert!(json.contains(r#""background":[0,0,0]"#));
    assert!(json.contains(r#""foreground":[255,255,255]"#));

    // Deserialize back
    let deserialized: ThemeInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.name, "test-theme");
    assert_eq!(deserialized.background, (0, 0, 0));
    assert_eq!(deserialized.foreground, (255, 255, 255));
}

#[test]
fn test_connected_message_with_theme() {
    let theme = ThemeInfo {
        name: "test-theme".to_string(),
        background: (0, 0, 0),
        foreground: (255, 255, 255),
        normal: [
            (0, 0, 0),
            (255, 0, 0),
            (0, 255, 0),
            (255, 255, 0),
            (0, 0, 255),
            (255, 0, 255),
            (0, 255, 255),
            (255, 255, 255),
        ],
        bright: [
            (128, 128, 128),
            (255, 128, 128),
            (128, 255, 128),
            (255, 255, 128),
            (128, 128, 255),
            (255, 128, 255),
            (128, 255, 255),
            (255, 255, 255),
        ],
    };

    let msg = ServerMessage::connected_builder(80, 24, "session-123".to_string())
        .theme(Some(theme))
        .build();
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"connected"#));
    assert!(json.contains(r#""session_id":"session-123"#));
    assert!(json.contains(r#""theme":{"#));
    assert!(json.contains(r#""name":"test-theme"#));
}

#[test]
#[allow(deprecated)]
fn test_connected_full_serialization() {
    let msg = ServerMessage::connected_full(
        120,
        40,
        None,
        "session-full".to_string(),
        None,
        Some("mybadge".to_string()),
        Some(0.5),
        Some("/home/user".to_string()),
        Some(2),
        None,
        None,
    );
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""badge":"mybadge"#));
    assert!(json.contains(r#""faint_text_alpha":0.5"#));
    assert!(json.contains(r#""cwd":"/home/user"#));
    assert!(json.contains(r#""modify_other_keys":2"#));
}

#[test]
fn test_cwd_changed_serialization() {
    let msg = ServerMessage::cwd_changed_full(
        Some("/old/dir".to_string()),
        "/new/dir".to_string(),
        Some("myhost".to_string()),
        Some("user".to_string()),
        1234567890,
        false,
    );
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"cwdchanged"#));
    assert!(json.contains(r#""new_cwd":"/new/dir"#));
    assert!(json.contains(r#""old_cwd":"/old/dir"#));
    assert!(json.contains(r#""is_local":false"#));
}

#[test]
fn test_trigger_matched_serialization() {
    let msg = ServerMessage::trigger_matched(
        42,
        10,
        5,
        15,
        "matched text".to_string(),
        vec!["group1".to_string()],
        9999999,
    );
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"triggermatched"#));
    assert!(json.contains(r#""trigger_id":42"#));
    assert!(json.contains(r#""text":"matched text"#));
}

#[test]
fn test_event_type_cwd_trigger_serialization() {
    let events = vec![EventType::Cwd, EventType::Trigger];
    let json = serde_json::to_string(&events).unwrap();
    assert!(json.contains(r#""cwd"#));
    assert!(json.contains(r#""trigger"#));
}

#[test]
fn test_semantic_snapshot_serialization() {
    let msg = ServerMessage::semantic_snapshot("{\"cols\":80}".to_string());
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"semantic_snapshot""#));
    assert!(json.contains(r#""snapshot_json"#));

    let deserialized: ServerMessage = serde_json::from_str(&json).unwrap();
    match deserialized {
        ServerMessage::SemanticSnapshot { snapshot_json } => {
            assert_eq!(snapshot_json, "{\"cols\":80}");
        }
        _ => panic!("Wrong message type"),
    }
}

#[test]
fn test_snapshot_request_serialization() {
    let msg = ClientMessage::snapshot_request("recent".to_string(), Some(5));
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"snapshot_request""#));

    let deserialized: ClientMessage = serde_json::from_str(&json).unwrap();
    match deserialized {
        ClientMessage::SnapshotRequest {
            scope,
            max_commands,
        } => {
            assert_eq!(scope, "recent");
            assert_eq!(max_commands, Some(5));
        }
        _ => panic!("Wrong message type"),
    }
}

#[test]
fn test_event_type_snapshot_serialization() {
    let events = vec![EventType::Snapshot];
    let json = serde_json::to_string(&events).unwrap();
    assert!(json.contains(r#""snapshot""#));
}
