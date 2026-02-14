//! Comprehensive tests for terminal streaming functionality
//!
//! Tests cover:
//! - Protocol message serialization/deserialization
//! - Server configuration
//! - HTTP Basic Authentication
//! - Binary protocol encoding/decoding
//! - Error handling

#[cfg(feature = "streaming")]
mod streaming_tests {
    use par_term_emu_core_rust::streaming::protocol::{
        ClientMessage, EventType, ServerMessage, ThemeInfo,
    };
    use par_term_emu_core_rust::streaming::{
        decode_client_message, decode_server_message, encode_client_message, encode_server_message,
        ApiAuthConfig, HttpBasicAuthConfig, PasswordConfig, StreamingConfig, StreamingError,
    };

    // =========================================================================
    // Protocol Message Constructor Tests
    // =========================================================================

    mod protocol_constructors {
        use super::*;

        #[test]
        fn test_server_message_output() {
            let msg = ServerMessage::output("test output".to_string());
            match msg {
                ServerMessage::Output { data, timestamp } => {
                    assert_eq!(data, "test output");
                    assert_eq!(timestamp, None);
                }
                _ => panic!("Expected Output variant"),
            }
        }

        #[test]
        fn test_server_message_output_with_timestamp() {
            let msg = ServerMessage::output_with_timestamp("test".to_string(), 1234567890);
            match msg {
                ServerMessage::Output { data, timestamp } => {
                    assert_eq!(data, "test");
                    assert_eq!(timestamp, Some(1234567890));
                }
                _ => panic!("Expected Output variant"),
            }
        }

        #[test]
        fn test_server_message_resize() {
            let msg = ServerMessage::resize(120, 40);
            match msg {
                ServerMessage::Resize { cols, rows } => {
                    assert_eq!(cols, 120);
                    assert_eq!(rows, 40);
                }
                _ => panic!("Expected Resize variant"),
            }
        }

        #[test]
        fn test_server_message_title() {
            let msg = ServerMessage::title("Terminal Title".to_string());
            match msg {
                ServerMessage::Title { title } => {
                    assert_eq!(title, "Terminal Title");
                }
                _ => panic!("Expected Title variant"),
            }
        }

        #[test]
        fn test_server_message_connected() {
            let msg = ServerMessage::connected(80, 24, "session-abc".to_string());
            match msg {
                ServerMessage::Connected {
                    cols,
                    rows,
                    initial_screen,
                    session_id,
                    theme,
                    ..
                } => {
                    assert_eq!(cols, 80);
                    assert_eq!(rows, 24);
                    assert_eq!(session_id, "session-abc");
                    assert!(initial_screen.is_none());
                    assert!(theme.is_none());
                }
                _ => panic!("Expected Connected variant"),
            }
        }

        #[test]
        fn test_server_message_connected_with_screen() {
            let msg = ServerMessage::connected_with_screen(
                80,
                24,
                "initial content".to_string(),
                "session-xyz".to_string(),
            );
            match msg {
                ServerMessage::Connected {
                    cols,
                    rows,
                    initial_screen,
                    session_id,
                    theme,
                    ..
                } => {
                    assert_eq!(cols, 80);
                    assert_eq!(rows, 24);
                    assert_eq!(session_id, "session-xyz");
                    assert_eq!(initial_screen, Some("initial content".to_string()));
                    assert!(theme.is_none());
                }
                _ => panic!("Expected Connected variant"),
            }
        }

        #[test]
        fn test_server_message_refresh() {
            let msg = ServerMessage::refresh(100, 50, "screen content here".to_string());
            match msg {
                ServerMessage::Refresh {
                    cols,
                    rows,
                    screen_content,
                } => {
                    assert_eq!(cols, 100);
                    assert_eq!(rows, 50);
                    assert_eq!(screen_content, "screen content here");
                }
                _ => panic!("Expected Refresh variant"),
            }
        }

        #[test]
        fn test_server_message_error() {
            let msg = ServerMessage::error("Something went wrong".to_string());
            match msg {
                ServerMessage::Error { message, code } => {
                    assert_eq!(message, "Something went wrong");
                    assert!(code.is_none());
                }
                _ => panic!("Expected Error variant"),
            }
        }

        #[test]
        fn test_server_message_error_with_code() {
            let msg =
                ServerMessage::error_with_code("Error occurred".to_string(), "E001".to_string());
            match msg {
                ServerMessage::Error { message, code } => {
                    assert_eq!(message, "Error occurred");
                    assert_eq!(code, Some("E001".to_string()));
                }
                _ => panic!("Expected Error variant"),
            }
        }

        #[test]
        fn test_server_message_cursor() {
            let msg = ServerMessage::cursor(10, 5, true);
            match msg {
                ServerMessage::CursorPosition { col, row, visible } => {
                    assert_eq!(col, 10);
                    assert_eq!(row, 5);
                    assert!(visible);
                }
                _ => panic!("Expected CursorPosition variant"),
            }
        }

        #[test]
        fn test_server_message_cursor_hidden() {
            let msg = ServerMessage::cursor(0, 0, false);
            match msg {
                ServerMessage::CursorPosition { col, row, visible } => {
                    assert_eq!(col, 0);
                    assert_eq!(row, 0);
                    assert!(!visible);
                }
                _ => panic!("Expected CursorPosition variant"),
            }
        }

        #[test]
        fn test_server_message_bell() {
            let msg = ServerMessage::bell();
            assert!(matches!(msg, ServerMessage::Bell));
        }

        #[test]
        fn test_server_message_shutdown() {
            let msg = ServerMessage::shutdown("Server maintenance".to_string());
            match msg {
                ServerMessage::Shutdown { reason } => {
                    assert_eq!(reason, "Server maintenance");
                }
                _ => panic!("Expected Shutdown variant"),
            }
        }

        #[test]
        fn test_client_message_input() {
            let msg = ClientMessage::input("hello\n".to_string());
            match msg {
                ClientMessage::Input { data } => {
                    assert_eq!(data, "hello\n");
                }
                _ => panic!("Expected Input variant"),
            }
        }

        #[test]
        fn test_client_message_resize() {
            let msg = ClientMessage::resize(132, 43);
            match msg {
                ClientMessage::Resize { cols, rows } => {
                    assert_eq!(cols, 132);
                    assert_eq!(rows, 43);
                }
                _ => panic!("Expected Resize variant"),
            }
        }

        #[test]
        fn test_client_message_ping() {
            let msg = ClientMessage::ping();
            assert!(matches!(msg, ClientMessage::Ping));
        }

        #[test]
        fn test_client_message_request_refresh() {
            let msg = ClientMessage::request_refresh();
            assert!(matches!(msg, ClientMessage::RequestRefresh));
        }

        #[test]
        fn test_client_message_subscribe() {
            let events = vec![EventType::Output, EventType::Bell, EventType::Cursor];
            let msg = ClientMessage::subscribe(events.clone());
            match msg {
                ClientMessage::Subscribe { events: e } => {
                    assert_eq!(e.len(), 3);
                    assert!(e.contains(&EventType::Output));
                    assert!(e.contains(&EventType::Bell));
                    assert!(e.contains(&EventType::Cursor));
                }
                _ => panic!("Expected Subscribe variant"),
            }
        }
    }

    // =========================================================================
    // Theme Info Tests
    // =========================================================================

    mod theme_info_tests {
        use super::*;

        fn create_test_theme() -> ThemeInfo {
            ThemeInfo {
                name: "test-theme".to_string(),
                background: (30, 30, 30),
                foreground: (220, 220, 220),
                normal: [
                    (0, 0, 0),
                    (205, 49, 49),
                    (13, 188, 121),
                    (229, 229, 16),
                    (36, 114, 200),
                    (188, 63, 188),
                    (17, 168, 205),
                    (229, 229, 229),
                ],
                bright: [
                    (102, 102, 102),
                    (241, 76, 76),
                    (35, 209, 139),
                    (245, 245, 67),
                    (59, 142, 234),
                    (214, 112, 214),
                    (41, 184, 219),
                    (255, 255, 255),
                ],
            }
        }

        #[test]
        fn test_theme_info_creation() {
            let theme = create_test_theme();
            assert_eq!(theme.name, "test-theme");
            assert_eq!(theme.background, (30, 30, 30));
            assert_eq!(theme.foreground, (220, 220, 220));
            assert_eq!(theme.normal.len(), 8);
            assert_eq!(theme.bright.len(), 8);
        }

        #[test]
        fn test_connected_message_with_theme() {
            let theme = create_test_theme();
            let msg =
                ServerMessage::connected_with_theme(80, 24, "session-theme".to_string(), theme);

            match msg {
                ServerMessage::Connected {
                    cols,
                    rows,
                    session_id,
                    theme,
                    initial_screen,
                    ..
                } => {
                    assert_eq!(cols, 80);
                    assert_eq!(rows, 24);
                    assert_eq!(session_id, "session-theme");
                    assert!(initial_screen.is_none());
                    assert!(theme.is_some());
                    let t = theme.unwrap();
                    assert_eq!(t.name, "test-theme");
                }
                _ => panic!("Expected Connected variant"),
            }
        }

        #[test]
        fn test_connected_message_with_screen_and_theme() {
            let theme = create_test_theme();
            let msg = ServerMessage::connected_with_screen_and_theme(
                80,
                24,
                "screen data".to_string(),
                "session-both".to_string(),
                theme,
            );

            match msg {
                ServerMessage::Connected {
                    cols,
                    rows,
                    session_id,
                    theme,
                    initial_screen,
                    ..
                } => {
                    assert_eq!(cols, 80);
                    assert_eq!(rows, 24);
                    assert_eq!(session_id, "session-both");
                    assert_eq!(initial_screen, Some("screen data".to_string()));
                    assert!(theme.is_some());
                }
                _ => panic!("Expected Connected variant"),
            }
        }

        #[test]
        fn test_theme_serialization_json() {
            let theme = create_test_theme();
            let json = serde_json::to_string(&theme).unwrap();
            assert!(json.contains("test-theme"));
            assert!(json.contains("background"));
            assert!(json.contains("foreground"));
            assert!(json.contains("normal"));
            assert!(json.contains("bright"));

            let deserialized: ThemeInfo = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized.name, theme.name);
            assert_eq!(deserialized.background, theme.background);
            assert_eq!(deserialized.foreground, theme.foreground);
        }
    }

    // =========================================================================
    // HTTP Basic Auth Tests
    // =========================================================================

    mod http_basic_auth_tests {
        use super::*;

        #[test]
        fn test_clear_text_password_correct() {
            let auth =
                HttpBasicAuthConfig::with_password("admin".to_string(), "secret123".to_string());
            assert!(auth.verify("admin", "secret123"));
        }

        #[test]
        fn test_clear_text_password_wrong_password() {
            let auth =
                HttpBasicAuthConfig::with_password("admin".to_string(), "secret123".to_string());
            assert!(!auth.verify("admin", "wrongpassword"));
        }

        #[test]
        fn test_clear_text_password_wrong_username() {
            let auth =
                HttpBasicAuthConfig::with_password("admin".to_string(), "secret123".to_string());
            assert!(!auth.verify("user", "secret123"));
        }

        #[test]
        fn test_clear_text_password_both_wrong() {
            let auth =
                HttpBasicAuthConfig::with_password("admin".to_string(), "secret123".to_string());
            assert!(!auth.verify("user", "wrongpassword"));
        }

        #[test]
        fn test_password_config_variants() {
            let clear = PasswordConfig::ClearText("password".to_string());
            let hash = PasswordConfig::Hash("$2y$...".to_string());

            match clear {
                PasswordConfig::ClearText(p) => assert_eq!(p, "password"),
                _ => panic!("Expected ClearText variant"),
            }

            match hash {
                PasswordConfig::Hash(h) => assert!(h.starts_with("$2y$")),
                _ => panic!("Expected Hash variant"),
            }
        }

        #[test]
        fn test_auth_config_with_hash() {
            // Create auth config with a hash placeholder
            let auth =
                HttpBasicAuthConfig::with_hash("testuser".to_string(), "placeholder".to_string());
            assert_eq!(auth.username, "testuser");
            match auth.password {
                PasswordConfig::Hash(h) => assert_eq!(h, "placeholder"),
                _ => panic!("Expected Hash variant"),
            }
        }

        #[test]
        fn test_auth_config_empty_strings() {
            let auth = HttpBasicAuthConfig::with_password(String::new(), String::new());
            assert!(auth.verify("", ""));
            assert!(!auth.verify("admin", ""));
            assert!(!auth.verify("", "password"));
        }
    }

    // =========================================================================
    // Streaming Config Tests
    // =========================================================================

    mod streaming_config_tests {
        use super::*;

        #[test]
        fn test_default_config() {
            let config = StreamingConfig::default();
            assert_eq!(config.max_clients, 1000);
            assert!(config.send_initial_screen);
            assert_eq!(config.keepalive_interval, 30);
            assert!(!config.default_read_only);
            assert!(!config.enable_http);
            assert_eq!(config.web_root, "./web_term");
            assert_eq!(config.initial_cols, 0);
            assert_eq!(config.initial_rows, 0);
            assert!(config.tls.is_none());
            assert!(config.http_basic_auth.is_none());
            assert_eq!(config.max_sessions, 10);
            assert_eq!(config.session_idle_timeout, 900);
            assert!(config.presets.is_empty());
            assert!(config.api_key.is_none());
        }

        #[test]
        fn test_custom_config() {
            let config = StreamingConfig {
                max_clients: 50,
                send_initial_screen: false,
                keepalive_interval: 60,
                default_read_only: true,
                enable_http: true,
                web_root: "/var/www/term".to_string(),
                initial_cols: 120,
                initial_rows: 40,
                tls: None,
                http_basic_auth: Some(HttpBasicAuthConfig::with_password(
                    "admin".to_string(),
                    "pass".to_string(),
                )),
                max_sessions: 5,
                session_idle_timeout: 600,
                presets: std::collections::HashMap::new(),
                max_clients_per_session: 0,
                input_rate_limit_bytes_per_sec: 0,
                enable_system_stats: false,
                system_stats_interval_secs: 5,
                api_key: Some("test-key-123".to_string()),
            };

            assert_eq!(config.max_clients, 50);
            assert!(!config.send_initial_screen);
            assert_eq!(config.keepalive_interval, 60);
            assert!(config.default_read_only);
            assert!(config.enable_http);
            assert_eq!(config.web_root, "/var/www/term");
            assert_eq!(config.initial_cols, 120);
            assert_eq!(config.initial_rows, 40);
            assert!(config.http_basic_auth.is_some());
            assert_eq!(config.max_sessions, 5);
            assert_eq!(config.session_idle_timeout, 600);
            assert!(config.presets.is_empty());
            assert_eq!(config.api_key.as_deref(), Some("test-key-123"));
        }

        #[test]
        fn test_config_clone() {
            let config1 = StreamingConfig {
                max_clients: 100,
                ..Default::default()
            };
            let config2 = config1.clone();
            assert_eq!(config1.max_clients, config2.max_clients);
            assert_eq!(config1.send_initial_screen, config2.send_initial_screen);
        }

        #[test]
        fn test_config_with_api_key() {
            let config = StreamingConfig {
                api_key: Some("secret-key".to_string()),
                ..Default::default()
            };
            assert_eq!(config.api_key.as_deref(), Some("secret-key"));
        }

        #[test]
        fn test_config_default_has_no_api_key() {
            let config = StreamingConfig::default();
            assert!(config.api_key.is_none());
        }

        #[test]
        fn test_api_auth_config_not_configured() {
            let config = ApiAuthConfig {
                api_key: None,
                http_basic_auth: None,
            };
            assert!(!config.is_configured());
        }

        #[test]
        fn test_api_auth_config_api_key_only() {
            let config = ApiAuthConfig {
                api_key: Some("key".to_string()),
                http_basic_auth: None,
            };
            assert!(config.is_configured());
        }

        #[test]
        fn test_api_auth_config_basic_auth_only() {
            let config = ApiAuthConfig {
                api_key: None,
                http_basic_auth: Some(HttpBasicAuthConfig::with_password(
                    "user".to_string(),
                    "pass".to_string(),
                )),
            };
            assert!(config.is_configured());
        }

        #[test]
        fn test_api_auth_config_both() {
            let config = ApiAuthConfig {
                api_key: Some("key".to_string()),
                http_basic_auth: Some(HttpBasicAuthConfig::with_password(
                    "user".to_string(),
                    "pass".to_string(),
                )),
            };
            assert!(config.is_configured());
        }
    }

    // =========================================================================
    // Binary Protocol Encoding/Decoding Tests
    // =========================================================================

    mod binary_protocol_tests {
        use super::*;

        #[test]
        fn test_encode_decode_all_server_message_types() {
            // Output
            let msg = ServerMessage::output("test data".to_string());
            let encoded = encode_server_message(&msg).unwrap();
            let decoded = decode_server_message(&encoded).unwrap();
            match decoded {
                ServerMessage::Output { data, .. } => assert_eq!(data, "test data"),
                _ => panic!("Expected Output"),
            }

            // Resize
            let msg = ServerMessage::resize(80, 24);
            let encoded = encode_server_message(&msg).unwrap();
            let decoded = decode_server_message(&encoded).unwrap();
            match decoded {
                ServerMessage::Resize { cols, rows } => {
                    assert_eq!(cols, 80);
                    assert_eq!(rows, 24);
                }
                _ => panic!("Expected Resize"),
            }

            // Title
            let msg = ServerMessage::title("My Terminal".to_string());
            let encoded = encode_server_message(&msg).unwrap();
            let decoded = decode_server_message(&encoded).unwrap();
            match decoded {
                ServerMessage::Title { title } => assert_eq!(title, "My Terminal"),
                _ => panic!("Expected Title"),
            }

            // Bell
            let msg = ServerMessage::bell();
            let encoded = encode_server_message(&msg).unwrap();
            let decoded = decode_server_message(&encoded).unwrap();
            assert!(matches!(decoded, ServerMessage::Bell));

            // Error
            let msg = ServerMessage::error_with_code("Error msg".to_string(), "E500".to_string());
            let encoded = encode_server_message(&msg).unwrap();
            let decoded = decode_server_message(&encoded).unwrap();
            match decoded {
                ServerMessage::Error { message, code } => {
                    assert_eq!(message, "Error msg");
                    assert_eq!(code, Some("E500".to_string()));
                }
                _ => panic!("Expected Error"),
            }

            // Shutdown
            let msg = ServerMessage::shutdown("Goodbye".to_string());
            let encoded = encode_server_message(&msg).unwrap();
            let decoded = decode_server_message(&encoded).unwrap();
            match decoded {
                ServerMessage::Shutdown { reason } => assert_eq!(reason, "Goodbye"),
                _ => panic!("Expected Shutdown"),
            }

            // Cursor
            let msg = ServerMessage::cursor(5, 10, true);
            let encoded = encode_server_message(&msg).unwrap();
            let decoded = decode_server_message(&encoded).unwrap();
            match decoded {
                ServerMessage::CursorPosition { col, row, visible } => {
                    assert_eq!(col, 5);
                    assert_eq!(row, 10);
                    assert!(visible);
                }
                _ => panic!("Expected CursorPosition"),
            }

            // Refresh
            let msg = ServerMessage::refresh(80, 24, "screen".to_string());
            let encoded = encode_server_message(&msg).unwrap();
            let decoded = decode_server_message(&encoded).unwrap();
            match decoded {
                ServerMessage::Refresh {
                    cols,
                    rows,
                    screen_content,
                } => {
                    assert_eq!(cols, 80);
                    assert_eq!(rows, 24);
                    assert_eq!(screen_content, "screen");
                }
                _ => panic!("Expected Refresh"),
            }

            // Connected
            let msg = ServerMessage::connected(80, 24, "sess-123".to_string());
            let encoded = encode_server_message(&msg).unwrap();
            let decoded = decode_server_message(&encoded).unwrap();
            match decoded {
                ServerMessage::Connected { session_id, .. } => {
                    assert_eq!(session_id, "sess-123");
                }
                _ => panic!("Expected Connected"),
            }
        }

        #[test]
        fn test_encode_decode_all_client_message_types() {
            // Input
            let msg = ClientMessage::input("ls -la\n".to_string());
            let encoded = encode_client_message(&msg).unwrap();
            let decoded = decode_client_message(&encoded).unwrap();
            match decoded {
                ClientMessage::Input { data } => assert_eq!(data, "ls -la\n"),
                _ => panic!("Expected Input"),
            }

            // Resize
            let msg = ClientMessage::resize(120, 40);
            let encoded = encode_client_message(&msg).unwrap();
            let decoded = decode_client_message(&encoded).unwrap();
            match decoded {
                ClientMessage::Resize { cols, rows } => {
                    assert_eq!(cols, 120);
                    assert_eq!(rows, 40);
                }
                _ => panic!("Expected Resize"),
            }

            // Ping
            let msg = ClientMessage::ping();
            let encoded = encode_client_message(&msg).unwrap();
            let decoded = decode_client_message(&encoded).unwrap();
            assert!(matches!(decoded, ClientMessage::Ping));

            // RequestRefresh
            let msg = ClientMessage::request_refresh();
            let encoded = encode_client_message(&msg).unwrap();
            let decoded = decode_client_message(&encoded).unwrap();
            assert!(matches!(decoded, ClientMessage::RequestRefresh));

            // Subscribe
            let msg = ClientMessage::subscribe(vec![EventType::Output, EventType::Bell]);
            let encoded = encode_client_message(&msg).unwrap();
            let decoded = decode_client_message(&encoded).unwrap();
            match decoded {
                ClientMessage::Subscribe { events } => {
                    assert!(events.contains(&EventType::Output));
                    assert!(events.contains(&EventType::Bell));
                }
                _ => panic!("Expected Subscribe"),
            }
        }

        #[test]
        fn test_compression_applied_for_large_messages() {
            // Create a large message that should trigger compression (>256 bytes)
            let large_data = "X".repeat(1000);
            let msg = ServerMessage::output(large_data.clone());
            let encoded = encode_server_message(&msg).unwrap();

            // First byte is compression flag
            // 0x01 means compressed
            assert_eq!(encoded[0], 0x01, "Large messages should be compressed");

            // Verify roundtrip works
            let decoded = decode_server_message(&encoded).unwrap();
            match decoded {
                ServerMessage::Output { data, .. } => assert_eq!(data, large_data),
                _ => panic!("Expected Output"),
            }
        }

        #[test]
        fn test_no_compression_for_small_messages() {
            let msg = ServerMessage::output("small".to_string());
            let encoded = encode_server_message(&msg).unwrap();

            // First byte is compression flag
            // 0x00 means uncompressed
            assert_eq!(encoded[0], 0x00, "Small messages should not be compressed");
        }

        #[test]
        fn test_ansi_escape_sequences_preserved() {
            // Test that ANSI escape sequences are properly preserved through encoding
            let ansi_data = "\x1b[31mRed Text\x1b[0m\x1b[32mGreen\x1b[0m";
            let msg = ServerMessage::output(ansi_data.to_string());
            let encoded = encode_server_message(&msg).unwrap();
            let decoded = decode_server_message(&encoded).unwrap();

            match decoded {
                ServerMessage::Output { data, .. } => {
                    assert_eq!(data, ansi_data);
                    assert!(data.contains("\x1b[31m"));
                    assert!(data.contains("\x1b[0m"));
                }
                _ => panic!("Expected Output"),
            }
        }

        #[test]
        fn test_unicode_preserved() {
            let unicode_data = "Hello 世界 🌍 مرحبا Привет";
            let msg = ServerMessage::output(unicode_data.to_string());
            let encoded = encode_server_message(&msg).unwrap();
            let decoded = decode_server_message(&encoded).unwrap();

            match decoded {
                ServerMessage::Output { data, .. } => assert_eq!(data, unicode_data),
                _ => panic!("Expected Output"),
            }
        }

        #[test]
        fn test_empty_data() {
            let msg = ServerMessage::output(String::new());
            let encoded = encode_server_message(&msg).unwrap();
            let decoded = decode_server_message(&encoded).unwrap();

            match decoded {
                ServerMessage::Output { data, .. } => assert!(data.is_empty()),
                _ => panic!("Expected Output"),
            }
        }

        #[test]
        fn test_decode_empty_message_error() {
            let result = decode_client_message(&[]);
            assert!(result.is_err());
            match result {
                Err(StreamingError::InvalidMessage(msg)) => {
                    assert!(msg.contains("Empty"));
                }
                _ => panic!("Expected InvalidMessage error"),
            }
        }

        #[test]
        fn test_decode_invalid_data_error() {
            // Random invalid data (not valid protobuf)
            let invalid_data = [0x00, 0xFF, 0xFE, 0xFD, 0xFC];
            let result = decode_server_message(&invalid_data);
            // Should return an error for invalid protobuf
            assert!(result.is_err());
        }
    }

    // =========================================================================
    // Event Type Tests
    // =========================================================================

    mod event_type_tests {
        use super::*;

        #[test]
        fn test_event_type_equality() {
            assert_eq!(EventType::Output, EventType::Output);
            assert_eq!(EventType::Cursor, EventType::Cursor);
            assert_eq!(EventType::Bell, EventType::Bell);
            assert_eq!(EventType::Title, EventType::Title);
            assert_eq!(EventType::Resize, EventType::Resize);

            assert_ne!(EventType::Output, EventType::Cursor);
            assert_ne!(EventType::Bell, EventType::Title);
        }

        #[test]
        fn test_event_type_hash() {
            use std::collections::HashSet;
            let mut set = HashSet::new();
            set.insert(EventType::Output);
            set.insert(EventType::Bell);
            set.insert(EventType::Output); // Duplicate

            assert_eq!(set.len(), 2);
            assert!(set.contains(&EventType::Output));
            assert!(set.contains(&EventType::Bell));
            assert!(!set.contains(&EventType::Cursor));
        }

        #[test]
        fn test_event_type_serialization() {
            let events = vec![
                EventType::Output,
                EventType::Cursor,
                EventType::Bell,
                EventType::Title,
                EventType::Resize,
            ];

            let json = serde_json::to_string(&events).unwrap();
            assert!(json.contains("output"));
            assert!(json.contains("cursor"));
            assert!(json.contains("bell"));
            assert!(json.contains("title"));
            assert!(json.contains("resize"));

            let deserialized: Vec<EventType> = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized.len(), 5);
        }
    }

    // =========================================================================
    // Streaming Error Tests
    // =========================================================================

    mod streaming_error_tests {
        use super::*;

        #[test]
        fn test_error_display_messages() {
            let errors = vec![
                (
                    StreamingError::WebSocketError("conn failed".to_string()),
                    "WebSocket error: conn failed",
                ),
                (
                    StreamingError::InvalidMessage("bad format".to_string()),
                    "Invalid message: bad format",
                ),
                (StreamingError::ConnectionClosed, "Connection closed"),
                (
                    StreamingError::ClientDisconnected("client-1".to_string()),
                    "Client disconnected: client-1",
                ),
                (
                    StreamingError::ServerError("internal".to_string()),
                    "Server error: internal",
                ),
                (
                    StreamingError::TerminalError("locked".to_string()),
                    "Terminal error: locked",
                ),
                (
                    StreamingError::InvalidInput("invalid".to_string()),
                    "Invalid input: invalid",
                ),
                (StreamingError::RateLimitExceeded, "Rate limit exceeded"),
                (
                    StreamingError::MaxClientsReached,
                    "Maximum number of clients reached",
                ),
                (
                    StreamingError::AuthenticationFailed("invalid token".to_string()),
                    "Authentication failed: invalid token",
                ),
                (
                    StreamingError::PermissionDenied("read only".to_string()),
                    "Permission denied: read only",
                ),
            ];

            for (error, expected_msg) in errors {
                assert_eq!(error.to_string(), expected_msg);
            }
        }

        #[test]
        fn test_error_from_io_error() {
            let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
            let err: StreamingError = io_err.into();
            match err {
                StreamingError::IoError(e) => {
                    assert_eq!(e.kind(), std::io::ErrorKind::NotFound);
                }
                _ => panic!("Expected IoError variant"),
            }
        }

        #[test]
        fn test_error_from_serde_error() {
            let json_err = serde_json::from_str::<serde_json::Value>("{invalid}").unwrap_err();
            let err: StreamingError = json_err.into();
            assert!(matches!(err, StreamingError::SerializationError(_)));
        }

        #[test]
        fn test_error_debug_format() {
            let err = StreamingError::WebSocketError("test error".to_string());
            let debug = format!("{:?}", err);
            assert!(debug.contains("WebSocketError"));
            assert!(debug.contains("test error"));
        }
    }

    // =========================================================================
    // JSON Serialization Tests
    // =========================================================================

    mod json_serialization_tests {
        use super::*;

        #[test]
        fn test_server_message_json_type_field() {
            // Verify the type field is correctly set in JSON
            let test_cases = vec![
                (ServerMessage::output("data".to_string()), "output"),
                (ServerMessage::resize(80, 24), "resize"),
                (ServerMessage::title("title".to_string()), "title"),
                (ServerMessage::bell(), "bell"),
                (
                    ServerMessage::connected(80, 24, "sess".to_string()),
                    "connected",
                ),
                (ServerMessage::error("err".to_string()), "error"),
                (ServerMessage::shutdown("bye".to_string()), "shutdown"),
                (ServerMessage::cursor(0, 0, true), "cursor"),
                (
                    ServerMessage::refresh(80, 24, "content".to_string()),
                    "refresh",
                ),
            ];

            for (msg, expected_type) in test_cases {
                let json = serde_json::to_string(&msg).unwrap();
                assert!(
                    json.contains(&format!("\"type\":\"{}\"", expected_type)),
                    "Expected type field '{}' in JSON: {}",
                    expected_type,
                    json
                );
            }
        }

        #[test]
        fn test_client_message_json_type_field() {
            let test_cases = vec![
                (ClientMessage::input("data".to_string()), "input"),
                (ClientMessage::resize(80, 24), "resize"),
                (ClientMessage::ping(), "ping"),
                (ClientMessage::request_refresh(), "refresh"),
                (
                    ClientMessage::subscribe(vec![EventType::Output]),
                    "subscribe",
                ),
            ];

            for (msg, expected_type) in test_cases {
                let json = serde_json::to_string(&msg).unwrap();
                assert!(
                    json.contains(&format!("\"type\":\"{}\"", expected_type)),
                    "Expected type field '{}' in JSON: {}",
                    expected_type,
                    json
                );
            }
        }

        #[test]
        fn test_json_roundtrip_server_messages() {
            let messages = vec![
                ServerMessage::output_with_timestamp("test".to_string(), 12345),
                ServerMessage::resize(100, 50),
                ServerMessage::title("Terminal".to_string()),
                ServerMessage::bell(),
                ServerMessage::error_with_code("Error".to_string(), "E001".to_string()),
                ServerMessage::shutdown("Closing".to_string()),
                ServerMessage::cursor(10, 20, false),
            ];

            for msg in messages {
                let json = serde_json::to_string(&msg).unwrap();
                let parsed: ServerMessage = serde_json::from_str(&json).unwrap();
                // Re-serialize and compare JSON strings
                let json2 = serde_json::to_string(&parsed).unwrap();
                assert_eq!(json, json2);
            }
        }

        #[test]
        fn test_json_roundtrip_client_messages() {
            let messages = vec![
                ClientMessage::input("hello".to_string()),
                ClientMessage::resize(120, 40),
                ClientMessage::ping(),
                ClientMessage::request_refresh(),
                ClientMessage::subscribe(vec![
                    EventType::Output,
                    EventType::Bell,
                    EventType::Resize,
                ]),
            ];

            for msg in messages {
                let json = serde_json::to_string(&msg).unwrap();
                let parsed: ClientMessage = serde_json::from_str(&json).unwrap();
                let json2 = serde_json::to_string(&parsed).unwrap();
                assert_eq!(json, json2);
            }
        }

        #[test]
        fn test_optional_fields_omitted() {
            // Test that optional fields with None are omitted from JSON
            let msg = ServerMessage::connected(80, 24, "sess".to_string());
            let json = serde_json::to_string(&msg).unwrap();

            // initial_screen should not be present when None
            assert!(
                !json.contains("initial_screen"),
                "initial_screen should be omitted when None"
            );

            // theme should not be present when None
            assert!(!json.contains("theme"), "theme should be omitted when None");
        }

        #[test]
        fn test_progress_bar_changed_set() {
            let msg = ServerMessage::ProgressBarChanged {
                action: "set".to_string(),
                id: "dl-1".to_string(),
                state: Some("normal".to_string()),
                percent: Some(50),
                label: Some("Downloading".to_string()),
            };
            let json = serde_json::to_string(&msg).unwrap();
            assert!(json.contains("progress_bar_changed"));
            assert!(json.contains("dl-1"));
            assert!(json.contains("Downloading"));
        }

        #[test]
        fn test_progress_bar_changed_remove() {
            let msg = ServerMessage::ProgressBarChanged {
                action: "remove".to_string(),
                id: "dl-1".to_string(),
                state: None,
                percent: None,
                label: None,
            };
            let json = serde_json::to_string(&msg).unwrap();
            assert!(json.contains("progress_bar_changed"));
            assert!(json.contains("remove"));
            assert!(json.contains("dl-1"));
            // None fields should be omitted
            assert!(!json.contains("state"));
            assert!(!json.contains("percent"));
            assert!(!json.contains("label"));
        }

        #[test]
        fn test_progress_bar_changed_remove_all() {
            let msg = ServerMessage::ProgressBarChanged {
                action: "remove_all".to_string(),
                id: String::new(),
                state: None,
                percent: None,
                label: None,
            };
            let json = serde_json::to_string(&msg).unwrap();
            assert!(json.contains("remove_all"));
        }

        #[test]
        fn test_event_type_progress_bar() {
            let msg = ClientMessage::Subscribe {
                events: vec![EventType::ProgressBar],
            };
            let json = serde_json::to_string(&msg).unwrap();
            assert!(json.contains("progress_bar"));
        }
    }

    // =========================================================================
    // Mode Sync on Connect Tests
    // =========================================================================

    mod mode_sync {
        use par_term_emu_core_rust::streaming::protocol::ServerMessage;
        use par_term_emu_core_rust::streaming::SessionState;
        use par_term_emu_core_rust::terminal::Terminal;
        use parking_lot::Mutex;
        use std::sync::Arc;

        fn create_session_with_terminal(term: Terminal) -> SessionState {
            SessionState::new(
                "test-session".to_string(),
                Arc::new(Mutex::new(term)),
                None,
                false,
            )
        }

        /// Helper to extract (mode, enabled) pairs from mode sync messages
        fn extract_modes(messages: &[ServerMessage]) -> Vec<(String, bool)> {
            messages
                .iter()
                .filter_map(|msg| match msg {
                    ServerMessage::ModeChanged { mode, enabled } => Some((mode.clone(), *enabled)),
                    _ => None,
                })
                .collect()
        }

        #[test]
        fn test_no_mode_sync_for_default_terminal() {
            let term = Terminal::new(80, 24);
            let session = create_session_with_terminal(term);
            let messages = session.build_mode_sync_messages();
            // Default terminal should have no non-default modes
            assert!(
                messages.is_empty(),
                "Expected no mode sync messages for default terminal, got: {:?}",
                extract_modes(&messages)
            );
        }

        #[test]
        fn test_mode_sync_mouse_normal() {
            let mut term = Terminal::new(80, 24);
            term.process(b"\x1b[?1000h"); // Enable normal mouse tracking
            let session = create_session_with_terminal(term);
            let modes = extract_modes(&session.build_mode_sync_messages());
            assert!(
                modes.contains(&("mouse_normal".to_string(), true)),
                "Expected mouse_normal mode, got: {:?}",
                modes
            );
        }

        #[test]
        fn test_mode_sync_mouse_any_event() {
            let mut term = Terminal::new(80, 24);
            term.process(b"\x1b[?1003h"); // Enable any-event mouse tracking
            let session = create_session_with_terminal(term);
            let modes = extract_modes(&session.build_mode_sync_messages());
            assert!(
                modes.contains(&("mouse_any_event".to_string(), true)),
                "Expected mouse_any_event mode, got: {:?}",
                modes
            );
        }

        #[test]
        fn test_mode_sync_mouse_sgr_encoding() {
            let mut term = Terminal::new(80, 24);
            term.process(b"\x1b[?1006h"); // SGR mouse encoding
            let session = create_session_with_terminal(term);
            let modes = extract_modes(&session.build_mode_sync_messages());
            assert!(
                modes.contains(&("mouse_sgr".to_string(), true)),
                "Expected mouse_sgr mode, got: {:?}",
                modes
            );
        }

        #[test]
        fn test_mode_sync_bracketed_paste() {
            let mut term = Terminal::new(80, 24);
            term.process(b"\x1b[?2004h"); // Enable bracketed paste
            let session = create_session_with_terminal(term);
            let modes = extract_modes(&session.build_mode_sync_messages());
            assert!(
                modes.contains(&("bracketed_paste".to_string(), true)),
                "Expected bracketed_paste mode, got: {:?}",
                modes
            );
        }

        #[test]
        fn test_mode_sync_application_cursor() {
            let mut term = Terminal::new(80, 24);
            term.process(b"\x1b[?1h"); // Enable application cursor keys
            let session = create_session_with_terminal(term);
            let modes = extract_modes(&session.build_mode_sync_messages());
            assert!(
                modes.contains(&("application_cursor".to_string(), true)),
                "Expected application_cursor mode, got: {:?}",
                modes
            );
        }

        #[test]
        fn test_mode_sync_focus_tracking() {
            let mut term = Terminal::new(80, 24);
            term.process(b"\x1b[?1004h"); // Enable focus tracking
            let session = create_session_with_terminal(term);
            let modes = extract_modes(&session.build_mode_sync_messages());
            assert!(
                modes.contains(&("focus_tracking".to_string(), true)),
                "Expected focus_tracking mode, got: {:?}",
                modes
            );
        }

        #[test]
        fn test_mode_sync_cursor_hidden() {
            let mut term = Terminal::new(80, 24);
            term.process(b"\x1b[?25l"); // Hide cursor
            let session = create_session_with_terminal(term);
            let modes = extract_modes(&session.build_mode_sync_messages());
            assert!(
                modes.contains(&("cursor_visible".to_string(), false)),
                "Expected cursor_visible=false mode, got: {:?}",
                modes
            );
        }

        #[test]
        fn test_mode_sync_alternate_screen() {
            let mut term = Terminal::new(80, 24);
            term.process(b"\x1b[?1049h"); // Enter alternate screen
            let session = create_session_with_terminal(term);
            let modes = extract_modes(&session.build_mode_sync_messages());
            assert!(
                modes.contains(&("alternate_screen".to_string(), true)),
                "Expected alternate_screen mode, got: {:?}",
                modes
            );
        }

        #[test]
        fn test_mode_sync_insert_mode() {
            let mut term = Terminal::new(80, 24);
            term.process(b"\x1b[4h"); // Enable insert mode
            let session = create_session_with_terminal(term);
            let modes = extract_modes(&session.build_mode_sync_messages());
            assert!(
                modes.contains(&("insert_mode".to_string(), true)),
                "Expected insert_mode mode, got: {:?}",
                modes
            );
        }

        #[test]
        fn test_mode_sync_auto_wrap_disabled() {
            let mut term = Terminal::new(80, 24);
            term.process(b"\x1b[?7l"); // Disable auto-wrap (default is on)
            let session = create_session_with_terminal(term);
            let modes = extract_modes(&session.build_mode_sync_messages());
            assert!(
                modes.contains(&("auto_wrap".to_string(), false)),
                "Expected auto_wrap=false mode, got: {:?}",
                modes
            );
        }

        #[test]
        fn test_mode_sync_multiple_modes() {
            let mut term = Terminal::new(80, 24);
            // Enable several modes at once (like a TUI app would)
            term.process(b"\x1b[?1003h"); // Any-event mouse
            term.process(b"\x1b[?1006h"); // SGR encoding
            term.process(b"\x1b[?2004h"); // Bracketed paste
            term.process(b"\x1b[?1004h"); // Focus tracking
            term.process(b"\x1b[?1049h"); // Alt screen (saves/restores cursor state)
            term.process(b"\x1b[?25l"); // Hide cursor (after alt screen to avoid restore)

            let session = create_session_with_terminal(term);
            let modes = extract_modes(&session.build_mode_sync_messages());

            assert!(modes.contains(&("mouse_any_event".to_string(), true)));
            assert!(modes.contains(&("mouse_sgr".to_string(), true)));
            assert!(modes.contains(&("bracketed_paste".to_string(), true)));
            assert!(modes.contains(&("focus_tracking".to_string(), true)));
            assert!(modes.contains(&("cursor_visible".to_string(), false)));
            assert!(modes.contains(&("alternate_screen".to_string(), true)));
            assert_eq!(modes.len(), 6, "Expected exactly 6 mode sync messages");
        }

        #[test]
        fn test_mode_sync_after_mode_reset() {
            let mut term = Terminal::new(80, 24);
            // Enable and then disable a mode
            term.process(b"\x1b[?1003h"); // Enable any-event mouse
            term.process(b"\x1b[?1003l"); // Disable it
            let session = create_session_with_terminal(term);
            let modes = extract_modes(&session.build_mode_sync_messages());
            // No mouse mode should be synced since it was reset
            assert!(
                !modes.iter().any(|(m, _)| m.starts_with("mouse_")),
                "Expected no mouse mode after reset, got: {:?}",
                modes
            );
        }

        #[test]
        fn test_mode_sync_messages_are_valid_server_messages() {
            let mut term = Terminal::new(80, 24);
            term.process(b"\x1b[?1003h"); // Any-event mouse
            term.process(b"\x1b[?1006h"); // SGR encoding
            let session = create_session_with_terminal(term);
            let messages = session.build_mode_sync_messages();

            // Verify all messages can be encoded/decoded via protobuf
            for msg in &messages {
                let encoded =
                    par_term_emu_core_rust::streaming::encode_server_message(msg).unwrap();
                let decoded =
                    par_term_emu_core_rust::streaming::decode_server_message(&encoded).unwrap();
                match (&msg, &decoded) {
                    (
                        ServerMessage::ModeChanged {
                            mode: m1,
                            enabled: e1,
                        },
                        ServerMessage::ModeChanged {
                            mode: m2,
                            enabled: e2,
                        },
                    ) => {
                        assert_eq!(m1, m2);
                        assert_eq!(e1, e2);
                    }
                    _ => panic!("Expected ModeChanged after round-trip"),
                }
            }
        }
    }

    mod system_stats_tests {
        use super::*;
        use par_term_emu_core_rust::streaming::protocol::{
            CpuStats, DiskStats, LoadAverage, MemoryStats, NetworkInterfaceStats,
        };

        fn sample_cpu_stats() -> CpuStats {
            CpuStats {
                overall_usage_percent: 42.5,
                physical_core_count: 8,
                per_core_usage_percent: vec![30.0, 50.0, 40.0, 55.0],
                brand: Some("TestCPU".to_string()),
                frequency_mhz: Some(3600),
            }
        }

        fn sample_memory_stats() -> MemoryStats {
            MemoryStats {
                total_bytes: 16_000_000_000,
                used_bytes: 8_000_000_000,
                available_bytes: 8_000_000_000,
                swap_total_bytes: 4_000_000_000,
                swap_used_bytes: 1_000_000_000,
            }
        }

        fn sample_disk_stats() -> Vec<DiskStats> {
            vec![DiskStats {
                name: "sda1".to_string(),
                mount_point: "/".to_string(),
                total_bytes: 500_000_000_000,
                available_bytes: 200_000_000_000,
                kind: "SSD".to_string(),
                file_system: "ext4".to_string(),
                is_removable: false,
            }]
        }

        fn sample_network_stats() -> Vec<NetworkInterfaceStats> {
            vec![NetworkInterfaceStats {
                name: "eth0".to_string(),
                received_bytes: 1024,
                transmitted_bytes: 2048,
                total_received_bytes: 1_000_000,
                total_transmitted_bytes: 500_000,
                packets_received: 100,
                packets_transmitted: 50,
                errors_received: 0,
                errors_transmitted: 0,
            }]
        }

        fn sample_load_average() -> LoadAverage {
            LoadAverage {
                one_minute: 1.5,
                five_minutes: 2.0,
                fifteen_minutes: 1.8,
            }
        }

        #[test]
        fn test_system_stats_constructor() {
            let msg = ServerMessage::system_stats(
                Some(sample_cpu_stats()),
                Some(sample_memory_stats()),
                sample_disk_stats(),
                sample_network_stats(),
                Some(sample_load_average()),
                Some("testhost".to_string()),
                Some("Linux".to_string()),
                Some("6.1.0".to_string()),
                Some("6.1.0-generic".to_string()),
                Some(86400),
                Some(1700000000000),
            );
            match msg {
                ServerMessage::SystemStats {
                    cpu,
                    memory,
                    hostname,
                    uptime_secs,
                    ..
                } => {
                    assert!(cpu.is_some());
                    assert!(memory.is_some());
                    assert_eq!(hostname, Some("testhost".to_string()));
                    assert_eq!(uptime_secs, Some(86400));
                }
                _ => panic!("Expected SystemStats"),
            }
        }

        #[test]
        fn test_system_stats_json_roundtrip() {
            let msg = ServerMessage::system_stats(
                Some(sample_cpu_stats()),
                Some(sample_memory_stats()),
                sample_disk_stats(),
                sample_network_stats(),
                Some(sample_load_average()),
                Some("testhost".to_string()),
                Some("Linux".to_string()),
                Some("6.1.0".to_string()),
                Some("6.1.0-generic".to_string()),
                Some(86400),
                Some(1700000000000),
            );
            let json = serde_json::to_string(&msg).unwrap();
            assert!(json.contains(r#""type":"system_stats"#));
            assert!(json.contains(r#""overall_usage_percent":42.5"#));
            assert!(json.contains(r#""total_bytes":16000000000"#));
            assert!(json.contains(r#""hostname":"testhost"#));

            let deserialized: ServerMessage = serde_json::from_str(&json).unwrap();
            match deserialized {
                ServerMessage::SystemStats {
                    cpu,
                    memory,
                    disks,
                    networks,
                    load_average,
                    hostname,
                    os_name,
                    ..
                } => {
                    let cpu = cpu.unwrap();
                    assert!((cpu.overall_usage_percent - 42.5).abs() < f64::EPSILON);
                    assert_eq!(cpu.physical_core_count, 8);
                    assert_eq!(cpu.per_core_usage_percent.len(), 4);
                    assert_eq!(cpu.brand, Some("TestCPU".to_string()));
                    assert_eq!(cpu.frequency_mhz, Some(3600));

                    let memory = memory.unwrap();
                    assert_eq!(memory.total_bytes, 16_000_000_000);
                    assert_eq!(memory.used_bytes, 8_000_000_000);

                    assert_eq!(disks.len(), 1);
                    assert_eq!(disks[0].name, "sda1");
                    assert_eq!(disks[0].file_system, "ext4");
                    assert!(!disks[0].is_removable);

                    assert_eq!(networks.len(), 1);
                    assert_eq!(networks[0].name, "eth0");
                    assert_eq!(networks[0].received_bytes, 1024);

                    let la = load_average.unwrap();
                    assert!((la.one_minute - 1.5).abs() < f64::EPSILON);

                    assert_eq!(hostname, Some("testhost".to_string()));
                    assert_eq!(os_name, Some("Linux".to_string()));
                }
                _ => panic!("Expected SystemStats"),
            }
        }

        #[test]
        fn test_system_stats_proto_roundtrip() {
            let msg = ServerMessage::system_stats(
                Some(sample_cpu_stats()),
                Some(sample_memory_stats()),
                sample_disk_stats(),
                sample_network_stats(),
                Some(sample_load_average()),
                Some("testhost".to_string()),
                Some("Linux".to_string()),
                Some("6.1.0".to_string()),
                Some("6.1.0-generic".to_string()),
                Some(86400),
                Some(1700000000000),
            );
            let encoded = encode_server_message(&msg).unwrap();
            let decoded = decode_server_message(&encoded).unwrap();
            match decoded {
                ServerMessage::SystemStats {
                    cpu,
                    memory,
                    disks,
                    networks,
                    load_average,
                    hostname,
                    os_name,
                    os_version,
                    kernel_version,
                    uptime_secs,
                    timestamp,
                } => {
                    let cpu = cpu.unwrap();
                    assert!((cpu.overall_usage_percent - 42.5).abs() < f64::EPSILON);
                    assert_eq!(cpu.physical_core_count, 8);
                    assert_eq!(cpu.per_core_usage_percent.len(), 4);
                    assert_eq!(cpu.brand, Some("TestCPU".to_string()));
                    assert_eq!(cpu.frequency_mhz, Some(3600));

                    let memory = memory.unwrap();
                    assert_eq!(memory.total_bytes, 16_000_000_000);
                    assert_eq!(memory.available_bytes, 8_000_000_000);
                    assert_eq!(memory.swap_total_bytes, 4_000_000_000);
                    assert_eq!(memory.swap_used_bytes, 1_000_000_000);

                    assert_eq!(disks.len(), 1);
                    assert_eq!(disks[0].mount_point, "/");
                    assert_eq!(disks[0].total_bytes, 500_000_000_000);

                    assert_eq!(networks.len(), 1);
                    assert_eq!(networks[0].total_received_bytes, 1_000_000);
                    assert_eq!(networks[0].packets_received, 100);

                    let la = load_average.unwrap();
                    assert!((la.five_minutes - 2.0).abs() < f64::EPSILON);
                    assert!((la.fifteen_minutes - 1.8).abs() < f64::EPSILON);

                    assert_eq!(hostname, Some("testhost".to_string()));
                    assert_eq!(os_name, Some("Linux".to_string()));
                    assert_eq!(os_version, Some("6.1.0".to_string()));
                    assert_eq!(kernel_version, Some("6.1.0-generic".to_string()));
                    assert_eq!(uptime_secs, Some(86400));
                    assert_eq!(timestamp, Some(1700000000000));
                }
                _ => panic!("Expected SystemStats"),
            }
        }

        #[test]
        fn test_system_stats_event_type_conversion() {
            let i32_val: i32 = EventType::SystemStats.into();
            assert_eq!(i32_val, 18);
        }

        #[test]
        fn test_system_stats_config_defaults() {
            let config = StreamingConfig::default();
            assert!(!config.enable_system_stats);
            assert_eq!(config.system_stats_interval_secs, 5);
        }

        #[test]
        fn test_system_stats_empty_roundtrip() {
            let msg = ServerMessage::system_stats(
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            );
            let json = serde_json::to_string(&msg).unwrap();
            assert!(json.contains(r#""type":"system_stats"#));

            let deserialized: ServerMessage = serde_json::from_str(&json).unwrap();
            match deserialized {
                ServerMessage::SystemStats {
                    cpu,
                    memory,
                    disks,
                    networks,
                    load_average,
                    hostname,
                    ..
                } => {
                    assert!(cpu.is_none());
                    assert!(memory.is_none());
                    assert!(disks.is_empty());
                    assert!(networks.is_empty());
                    assert!(load_average.is_none());
                    assert!(hostname.is_none());
                }
                _ => panic!("Expected SystemStats"),
            }
        }

        #[test]
        fn test_system_stats_empty_proto_roundtrip() {
            let msg = ServerMessage::system_stats(
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            );
            let encoded = encode_server_message(&msg).unwrap();
            let decoded = decode_server_message(&encoded).unwrap();
            match decoded {
                ServerMessage::SystemStats {
                    cpu,
                    memory,
                    disks,
                    networks,
                    load_average,
                    hostname,
                    ..
                } => {
                    assert!(cpu.is_none());
                    assert!(memory.is_none());
                    assert!(disks.is_empty());
                    assert!(networks.is_empty());
                    assert!(load_average.is_none());
                    assert!(hostname.is_none());
                }
                _ => panic!("Expected SystemStats"),
            }
        }

        #[test]
        fn test_system_stats_event_type_json_roundtrip() {
            let event = EventType::SystemStats;
            let json = serde_json::to_string(&event).unwrap();
            assert_eq!(json, r#""system_stats""#);
            let deserialized: EventType = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, EventType::SystemStats);
        }
    }

    // =========================================================================
    // Contextual Awareness Event Serialization Tests
    // =========================================================================

    mod contextual_awareness_tests {
        use super::*;

        #[test]
        fn test_zone_opened_serialization() {
            let msg = ServerMessage::zone_opened(42, "prompt".to_string(), 100);
            let json = serde_json::to_string(&msg).unwrap();
            assert!(json.contains(r#""type":"zone_opened"#));
            let deserialized: ServerMessage = serde_json::from_str(&json).unwrap();
            match deserialized {
                ServerMessage::ZoneOpened {
                    zone_id,
                    zone_type,
                    abs_row_start,
                } => {
                    assert_eq!(zone_id, 42);
                    assert_eq!(zone_type, "prompt");
                    assert_eq!(abs_row_start, 100);
                }
                _ => panic!("Wrong message type"),
            }
        }

        #[test]
        fn test_zone_closed_serialization() {
            let msg = ServerMessage::zone_closed(1, "output".to_string(), 10, 50, Some(0));
            let json = serde_json::to_string(&msg).unwrap();
            assert!(json.contains(r#""type":"zone_closed"#));
            let deserialized: ServerMessage = serde_json::from_str(&json).unwrap();
            match deserialized {
                ServerMessage::ZoneClosed {
                    zone_id,
                    zone_type,
                    abs_row_start,
                    abs_row_end,
                    exit_code,
                } => {
                    assert_eq!(zone_id, 1);
                    assert_eq!(zone_type, "output");
                    assert_eq!(abs_row_start, 10);
                    assert_eq!(abs_row_end, 50);
                    assert_eq!(exit_code, Some(0));
                }
                _ => panic!("Wrong message type"),
            }
        }

        #[test]
        fn test_zone_scrolled_out_serialization() {
            let msg = ServerMessage::zone_scrolled_out(5, "command".to_string());
            let json = serde_json::to_string(&msg).unwrap();
            assert!(json.contains(r#""type":"zone_scrolled_out"#));
            let deserialized: ServerMessage = serde_json::from_str(&json).unwrap();
            match deserialized {
                ServerMessage::ZoneScrolledOut { zone_id, zone_type } => {
                    assert_eq!(zone_id, 5);
                    assert_eq!(zone_type, "command");
                }
                _ => panic!("Wrong message type"),
            }
        }

        #[test]
        fn test_environment_changed_serialization() {
            let msg = ServerMessage::environment_changed(
                "cwd".to_string(),
                "/new/path".to_string(),
                Some("/old/path".to_string()),
            );
            let json = serde_json::to_string(&msg).unwrap();
            assert!(json.contains(r#""type":"environment_changed"#));
            let deserialized: ServerMessage = serde_json::from_str(&json).unwrap();
            match deserialized {
                ServerMessage::EnvironmentChanged {
                    key,
                    value,
                    old_value,
                } => {
                    assert_eq!(key, "cwd");
                    assert_eq!(value, "/new/path");
                    assert_eq!(old_value, Some("/old/path".to_string()));
                }
                _ => panic!("Wrong message type"),
            }
        }

        #[test]
        fn test_remote_host_transition_serialization() {
            let msg = ServerMessage::remote_host_transition(
                "server.com".to_string(),
                Some("deploy".to_string()),
                Some("localhost".to_string()),
                Some("user".to_string()),
            );
            let json = serde_json::to_string(&msg).unwrap();
            assert!(json.contains(r#""type":"remote_host_transition"#));
            let deserialized: ServerMessage = serde_json::from_str(&json).unwrap();
            match deserialized {
                ServerMessage::RemoteHostTransition {
                    hostname,
                    username,
                    old_hostname,
                    old_username,
                } => {
                    assert_eq!(hostname, "server.com");
                    assert_eq!(username, Some("deploy".to_string()));
                    assert_eq!(old_hostname, Some("localhost".to_string()));
                    assert_eq!(old_username, Some("user".to_string()));
                }
                _ => panic!("Wrong message type"),
            }
        }

        #[test]
        fn test_sub_shell_detected_serialization() {
            let msg = ServerMessage::sub_shell_detected(2, Some("bash".to_string()));
            let json = serde_json::to_string(&msg).unwrap();
            assert!(json.contains(r#""type":"sub_shell_detected"#));
            let deserialized: ServerMessage = serde_json::from_str(&json).unwrap();
            match deserialized {
                ServerMessage::SubShellDetected { depth, shell_type } => {
                    assert_eq!(depth, 2);
                    assert_eq!(shell_type, Some("bash".to_string()));
                }
                _ => panic!("Wrong message type"),
            }
        }

        #[test]
        fn test_new_event_types_serialization() {
            let events = vec![
                EventType::Zone,
                EventType::Environment,
                EventType::RemoteHost,
                EventType::SubShell,
            ];
            let json = serde_json::to_string(&events).unwrap();
            assert!(json.contains(r#""zone"#));
            assert!(json.contains(r#""environment"#));
            assert!(json.contains(r#""remote_host"#));
            assert!(json.contains(r#""sub_shell"#));
        }
    }

    // =========================================================================
    // File Transfer Streaming Roundtrip Tests
    // =========================================================================

    mod file_transfer_roundtrip_tests {
        use super::*;

        #[test]
        fn test_file_transfer_started_roundtrip() {
            let msg = ServerMessage::file_transfer_started(
                42,
                "download".to_string(),
                Some("test.pdf".to_string()),
                Some(1024),
            );

            // JSON roundtrip
            let json = serde_json::to_string(&msg).unwrap();
            assert!(json.contains(r#""type":"file_transfer_started"#));
            let json_decoded: ServerMessage = serde_json::from_str(&json).unwrap();
            match json_decoded {
                ServerMessage::FileTransferStarted {
                    id,
                    direction,
                    filename,
                    total_bytes,
                } => {
                    assert_eq!(id, 42);
                    assert_eq!(direction, "download");
                    assert_eq!(filename, Some("test.pdf".to_string()));
                    assert_eq!(total_bytes, Some(1024));
                }
                _ => panic!("Expected FileTransferStarted"),
            }

            // Proto (binary) roundtrip
            let encoded = encode_server_message(&msg).unwrap();
            let decoded = decode_server_message(&encoded).unwrap();
            match decoded {
                ServerMessage::FileTransferStarted {
                    id,
                    direction,
                    filename,
                    total_bytes,
                } => {
                    assert_eq!(id, 42);
                    assert_eq!(direction, "download");
                    assert_eq!(filename, Some("test.pdf".to_string()));
                    assert_eq!(total_bytes, Some(1024));
                }
                _ => panic!("Expected FileTransferStarted after proto roundtrip"),
            }
        }

        #[test]
        fn test_file_transfer_progress_roundtrip() {
            let msg = ServerMessage::file_transfer_progress(42, 512, Some(1024));

            // JSON roundtrip
            let json = serde_json::to_string(&msg).unwrap();
            assert!(json.contains(r#""type":"file_transfer_progress"#));
            let json_decoded: ServerMessage = serde_json::from_str(&json).unwrap();
            match json_decoded {
                ServerMessage::FileTransferProgress {
                    id,
                    bytes_transferred,
                    total_bytes,
                } => {
                    assert_eq!(id, 42);
                    assert_eq!(bytes_transferred, 512);
                    assert_eq!(total_bytes, Some(1024));
                }
                _ => panic!("Expected FileTransferProgress"),
            }

            // Proto (binary) roundtrip
            let encoded = encode_server_message(&msg).unwrap();
            let decoded = decode_server_message(&encoded).unwrap();
            match decoded {
                ServerMessage::FileTransferProgress {
                    id,
                    bytes_transferred,
                    total_bytes,
                } => {
                    assert_eq!(id, 42);
                    assert_eq!(bytes_transferred, 512);
                    assert_eq!(total_bytes, Some(1024));
                }
                _ => panic!("Expected FileTransferProgress after proto roundtrip"),
            }
        }

        #[test]
        fn test_file_transfer_completed_roundtrip() {
            let msg =
                ServerMessage::file_transfer_completed(42, Some("test.pdf".to_string()), 1024);

            // JSON roundtrip
            let json = serde_json::to_string(&msg).unwrap();
            assert!(json.contains(r#""type":"file_transfer_completed"#));
            let json_decoded: ServerMessage = serde_json::from_str(&json).unwrap();
            match json_decoded {
                ServerMessage::FileTransferCompleted { id, filename, size } => {
                    assert_eq!(id, 42);
                    assert_eq!(filename, Some("test.pdf".to_string()));
                    assert_eq!(size, 1024);
                }
                _ => panic!("Expected FileTransferCompleted"),
            }

            // Proto (binary) roundtrip
            let encoded = encode_server_message(&msg).unwrap();
            let decoded = decode_server_message(&encoded).unwrap();
            match decoded {
                ServerMessage::FileTransferCompleted { id, filename, size } => {
                    assert_eq!(id, 42);
                    assert_eq!(filename, Some("test.pdf".to_string()));
                    assert_eq!(size, 1024);
                }
                _ => panic!("Expected FileTransferCompleted after proto roundtrip"),
            }
        }

        #[test]
        fn test_file_transfer_failed_roundtrip() {
            let msg = ServerMessage::file_transfer_failed(42, "size exceeded".to_string());

            // JSON roundtrip
            let json = serde_json::to_string(&msg).unwrap();
            assert!(json.contains(r#""type":"file_transfer_failed"#));
            let json_decoded: ServerMessage = serde_json::from_str(&json).unwrap();
            match json_decoded {
                ServerMessage::FileTransferFailed { id, reason } => {
                    assert_eq!(id, 42);
                    assert_eq!(reason, "size exceeded");
                }
                _ => panic!("Expected FileTransferFailed"),
            }

            // Proto (binary) roundtrip
            let encoded = encode_server_message(&msg).unwrap();
            let decoded = decode_server_message(&encoded).unwrap();
            match decoded {
                ServerMessage::FileTransferFailed { id, reason } => {
                    assert_eq!(id, 42);
                    assert_eq!(reason, "size exceeded");
                }
                _ => panic!("Expected FileTransferFailed after proto roundtrip"),
            }
        }

        #[test]
        fn test_upload_requested_roundtrip() {
            let msg = ServerMessage::upload_requested("tgz".to_string());

            // JSON roundtrip
            let json = serde_json::to_string(&msg).unwrap();
            assert!(json.contains(r#""type":"upload_requested"#));
            let json_decoded: ServerMessage = serde_json::from_str(&json).unwrap();
            match json_decoded {
                ServerMessage::UploadRequested { format } => {
                    assert_eq!(format, "tgz");
                }
                _ => panic!("Expected UploadRequested"),
            }

            // Proto (binary) roundtrip
            let encoded = encode_server_message(&msg).unwrap();
            let decoded = decode_server_message(&encoded).unwrap();
            match decoded {
                ServerMessage::UploadRequested { format } => {
                    assert_eq!(format, "tgz");
                }
                _ => panic!("Expected UploadRequested after proto roundtrip"),
            }
        }

        #[test]
        fn test_file_transfer_event_types_serialization() {
            let events = vec![EventType::FileTransfer, EventType::UploadRequest];
            let json = serde_json::to_string(&events).unwrap();
            assert!(json.contains(r#""file_transfer"#));
            assert!(json.contains(r#""upload_request"#));

            let deserialized: Vec<EventType> = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized.len(), 2);
            assert!(deserialized.contains(&EventType::FileTransfer));
            assert!(deserialized.contains(&EventType::UploadRequest));
        }
    }
}

// Tests that work without streaming feature
mod non_streaming_tests {
    #[test]
    fn test_module_compiles_without_streaming() {
        // This test just verifies the module compiles without the streaming feature
        // No assertions needed - if it compiles, it passes
    }
}
