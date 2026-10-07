//! Golden wire test for the app <-> protobuf conversions (ARC-006).
//!
//! Captured from the hand-written conversions before they moved to the
//! `ProtoConvert` derive: every message variant's encoded bytes and decoded
//! value, plus the lossy decode paths a round trip never reaches (clamps,
//! truncations, presence checks, error strings). A conversion change that
//! alters any of them fails here.
//!
//! Regenerate (only for an intended wire change):
//! `PROTO_GOLDEN_UPDATE=1 cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming proto_golden`

use super::*;
use crate::streaming::protocol::{
    CpuStats, DiskStats, LoadAverage, MemoryStats, NetworkInterfaceStats,
};
use std::fmt::Write as _;

const GOLDEN_PATH: &str = "src/streaming/testdata/proto_golden.txt";
const GOLDEN: &str = include_str!("testdata/proto_golden.txt");

fn theme() -> AppThemeInfo {
    let mut normal = [(0u8, 0u8, 0u8); 8];
    let mut bright = [(0u8, 0u8, 0u8); 8];
    for i in 0..8u8 {
        normal[i as usize] = (i, i + 10, i + 20);
        bright[i as usize] = (i + 100, i + 110, i + 120);
    }
    AppThemeInfo {
        name: "golden-theme".into(),
        background: (1, 2, 3),
        foreground: (250, 251, 252),
        normal,
        bright,
    }
}

fn agent(pane_id: u32) -> AppAgentEntry {
    AppAgentEntry {
        pane_id,
        agent: "claude".into(),
        state: "working".into(),
        source: "hook".into(),
        reason: "tool".into(),
    }
}

fn server_fixtures() -> Vec<AppServerMessage> {
    use AppServerMessage as S;
    vec![
        S::Output {
            data: "out\x1b[1mput".into(),
            timestamp: Some(11),
        },
        S::Output {
            data: String::new(),
            timestamp: None,
        },
        S::Resize {
            cols: 132,
            rows: 43,
        },
        S::Title {
            title: "title ✓".into(),
        },
        S::Connected {
            cols: 100,
            rows: 30,
            initial_screen: Some("screen".into()),
            session_id: "sess".into(),
            theme: Some(theme()),
            badge: Some("badge".into()),
            faint_text_alpha: Some(0.25),
            cwd: Some("/tmp".into()),
            modify_other_keys: Some(2),
            client_id: Some("cid".into()),
            readonly: Some(true),
        },
        S::Connected {
            cols: 1,
            rows: 2,
            initial_screen: None,
            session_id: String::new(),
            theme: None,
            badge: None,
            faint_text_alpha: None,
            cwd: None,
            modify_other_keys: None,
            client_id: None,
            readonly: None,
        },
        S::Refresh {
            cols: 7,
            rows: 8,
            screen_content: "refresh".into(),
        },
        S::CursorPosition {
            col: 3,
            row: 4,
            visible: true,
        },
        S::Bell,
        S::CwdChanged {
            old_cwd: Some("/old".into()),
            new_cwd: "/new".into(),
            hostname: Some("host".into()),
            username: Some("user".into()),
            timestamp: Some(12),
        },
        S::TriggerMatched {
            trigger_id: 13,
            row: 5,
            col: 6,
            end_col: 9,
            text: "match".into(),
            captures: vec!["a".into(), "b".into()],
            timestamp: 14,
        },
        S::ActionNotify {
            trigger_id: 15,
            title: "nt".into(),
            message: "nm".into(),
        },
        S::ActionMarkLine {
            trigger_id: 16,
            row: 17,
            label: Some("label".into()),
            color: Some((9, 8, 7)),
        },
        S::ActionMarkLine {
            trigger_id: 0,
            row: 0,
            label: None,
            color: None,
        },
        S::Error {
            message: "err".into(),
            code: Some("E1".into()),
        },
        S::Shutdown {
            reason: "bye".into(),
        },
        S::Pong,
        S::ModeChanged {
            mode: "mode".into(),
            enabled: true,
        },
        S::GraphicsAdded {
            row: 18,
            format: Some("sixel".into()),
        },
        S::HyperlinkAdded {
            url: "https://x".into(),
            row: 19,
            col: 20,
            id: Some("id".into()),
        },
        S::UserVarChanged {
            name: "n".into(),
            value: "v".into(),
            old_value: Some("o".into()),
        },
        S::ProgressBarChanged {
            action: "set".into(),
            id: "pb".into(),
            state: Some("normal".into()),
            percent: Some(42),
            label: Some("lbl".into()),
        },
        S::BadgeChanged {
            badge: Some("b".into()),
        },
        S::SelectionChanged {
            start_col: Some(1),
            start_row: Some(2),
            end_col: Some(3),
            end_row: Some(4),
            text: Some("sel".into()),
            mode: "block".into(),
            cleared: false,
        },
        S::SelectionChanged {
            start_col: None,
            start_row: None,
            end_col: None,
            end_row: None,
            text: None,
            mode: "chars".into(),
            cleared: true,
        },
        S::ClipboardSync {
            operation: "set".into(),
            content: "clip".into(),
            target: Some("c".into()),
        },
        S::ShellIntegrationEvent {
            event_type: "command_finished".into(),
            command: Some("ls".into()),
            exit_code: Some(-2),
            timestamp: Some(21),
            cursor_line: Some(22),
        },
        S::SystemStats {
            cpu: Some(CpuStats {
                overall_usage_percent: 12.5,
                physical_core_count: 8,
                per_core_usage_percent: vec![1.0, 2.5],
                brand: Some("cpu".into()),
                frequency_mhz: Some(3200),
            }),
            memory: Some(MemoryStats {
                total_bytes: 100,
                used_bytes: 50,
                available_bytes: 40,
                swap_total_bytes: 30,
                swap_used_bytes: 20,
            }),
            disks: vec![DiskStats {
                name: "disk".into(),
                mount_point: "/".into(),
                total_bytes: 1000,
                available_bytes: 500,
                kind: "SSD".into(),
                file_system: "apfs".into(),
                is_removable: true,
            }],
            networks: vec![NetworkInterfaceStats {
                name: "en0".into(),
                received_bytes: 1,
                transmitted_bytes: 2,
                total_received_bytes: 3,
                total_transmitted_bytes: 4,
                packets_received: 5,
                packets_transmitted: 6,
                errors_received: 7,
                errors_transmitted: 8,
            }],
            load_average: Some(LoadAverage {
                one_minute: 0.5,
                five_minutes: 1.5,
                fifteen_minutes: 2.5,
            }),
            hostname: Some("h".into()),
            os_name: Some("os".into()),
            os_version: Some("1".into()),
            kernel_version: Some("k".into()),
            uptime_secs: Some(23),
            timestamp: Some(24),
        },
        S::SystemStats {
            cpu: None,
            memory: None,
            disks: vec![],
            networks: vec![],
            load_average: None,
            hostname: None,
            os_name: None,
            os_version: None,
            kernel_version: None,
            uptime_secs: None,
            timestamp: None,
        },
        S::ZoneOpened {
            zone_id: 25,
            zone_type: "prompt".into(),
            abs_row_start: 26,
        },
        S::ZoneClosed {
            zone_id: 27,
            zone_type: "output".into(),
            abs_row_start: 28,
            abs_row_end: 29,
            exit_code: Some(1),
        },
        S::ZoneScrolledOut {
            zone_id: 30,
            zone_type: "command".into(),
        },
        S::EnvironmentChanged {
            key: "K".into(),
            value: "V".into(),
            old_value: Some("O".into()),
        },
        S::RemoteHostTransition {
            hostname: "rh".into(),
            username: Some("ru".into()),
            old_hostname: Some("oh".into()),
            old_username: Some("ou".into()),
        },
        S::SubShellDetected {
            depth: 31,
            shell_type: Some("zsh".into()),
        },
        S::SemanticSnapshot {
            snapshot_json: "{\"a\":1}".into(),
        },
        S::FileTransferStarted {
            id: 32,
            direction: "download".into(),
            filename: Some("f".into()),
            total_bytes: Some(33),
        },
        S::FileTransferProgress {
            id: 34,
            bytes_transferred: 35,
            total_bytes: Some(36),
        },
        S::FileTransferCompleted {
            id: 37,
            filename: Some("g".into()),
            size: 38,
        },
        S::FileTransferFailed {
            id: 39,
            reason: "nope".into(),
        },
        S::UploadRequested {
            format: "tgz".into(),
        },
        S::ScreenCleared {
            include_scrollback: true,
        },
        S::AgentRoster {
            agents: vec![agent(0), agent(7)],
        },
        S::AgentStateChanged {
            agent: agent(0),
            released: true,
        },
    ]
}

fn client_fixtures() -> Vec<AppClientMessage> {
    use AppClientMessage as C;
    vec![
        C::Input {
            data: "in\x03".into(),
        },
        C::Resize { cols: 90, rows: 33 },
        C::Ping,
        C::RequestRefresh,
        C::Subscribe {
            events: vec![AppEventType::Output, AppEventType::ScreenCleared],
        },
        C::Mouse {
            col: 4,
            row: 5,
            button: 200,
            shift: true,
            ctrl: false,
            alt: true,
            event_type: MouseEventType::Scroll,
        },
        C::FocusChange { focused: true },
        C::Paste {
            content: "paste".into(),
        },
        C::SelectionRequest {
            start_col: 1,
            start_row: 2,
            end_col: 3,
            end_row: 4,
            mode: "line".into(),
        },
        C::ClipboardRequest {
            operation: "get".into(),
            content: Some("x".into()),
            target: None,
        },
        C::SnapshotRequest {
            scope: "full".into(),
            max_commands: Some(9),
        },
    ]
}

fn color(r: u32, g: u32, b: u32) -> pb::Color {
    pb::Color { r, g, b }
}

/// Hand-built wire messages that exercise the lossy decode paths.
fn lossy_server_inputs() -> Vec<(&'static str, pb::ServerMessage)> {
    use pb::server_message::Message as M;
    let wrap = |m: M| pb::ServerMessage { message: Some(m) };
    let wide_theme = pb::ThemeInfo {
        name: "w".into(),
        background: Some(color(256, 257, 511)),
        foreground: Some(color(1, 2, 3)),
        normal: (0..8).map(|i| color(300 + i, 0, 0)).collect(),
        bright: (0..8).map(|i| color(0, 0, 400 + i)).collect(),
    };
    let connected = |theme: Option<pb::ThemeInfo>| pb::Connected {
        cols: 70_000,
        rows: 65_536,
        initial_screen: Some(vec![0xff, b'a']),
        session_id: "s".into(),
        theme,
        ..Default::default()
    };
    vec![
        ("empty server oneof", pb::ServerMessage { message: None }),
        (
            "output invalid utf8",
            wrap(M::Output(pb::Output {
                data: vec![b'o', 0xc3, 0x28],
                timestamp: None,
            })),
        ),
        (
            "resize truncates u16",
            wrap(M::Resize(pb::Resize {
                cols: 65_537,
                rows: 70_000,
            })),
        ),
        (
            "connected wide theme truncates channels",
            wrap(M::Connected(connected(Some(wide_theme)))),
        ),
        (
            "connected theme missing background",
            wrap(M::Connected(connected(Some(pb::ThemeInfo {
                background: None,
                foreground: Some(color(1, 1, 1)),
                normal: vec![color(0, 0, 0); 8],
                bright: vec![color(0, 0, 0); 8],
                ..Default::default()
            })))),
        ),
        (
            "connected theme missing foreground",
            wrap(M::Connected(connected(Some(pb::ThemeInfo {
                background: Some(color(1, 1, 1)),
                foreground: None,
                normal: vec![color(0, 0, 0); 8],
                bright: vec![color(0, 0, 0); 8],
                ..Default::default()
            })))),
        ),
        (
            "connected theme 7 normal colors",
            wrap(M::Connected(connected(Some(pb::ThemeInfo {
                background: Some(color(1, 1, 1)),
                foreground: Some(color(1, 1, 1)),
                normal: vec![color(0, 0, 0); 7],
                bright: vec![color(0, 0, 0); 8],
                ..Default::default()
            })))),
        ),
        (
            "connected theme 9 bright colors",
            wrap(M::Connected(connected(Some(pb::ThemeInfo {
                background: Some(color(1, 1, 1)),
                foreground: Some(color(1, 1, 1)),
                normal: vec![color(0, 0, 0); 8],
                bright: vec![color(0, 0, 0); 9],
                ..Default::default()
            })))),
        ),
        (
            "mark line color truncates",
            wrap(M::ActionMarkLine(pb::ActionMarkLine {
                trigger_id: 1,
                row: 66_000,
                label: None,
                color: Some(color(256, 300, 1023)),
            })),
        ),
        (
            "progress percent clamps to 100",
            wrap(M::ProgressBarChanged(pb::ProgressBarChanged {
                action: "set".into(),
                id: "p".into(),
                state: None,
                percent: Some(250),
                label: None,
            })),
        ),
        (
            "progress percent above u8",
            wrap(M::ProgressBarChanged(pb::ProgressBarChanged {
                action: "set".into(),
                id: "p".into(),
                state: None,
                percent: Some(1000),
                label: None,
            })),
        ),
        (
            "selection truncates u16",
            wrap(M::SelectionChanged(pb::SelectionChanged {
                start_col: Some(65_536),
                start_row: Some(65_537),
                end_col: None,
                end_row: Some(1),
                text: None,
                mode: "m".into(),
                cleared: false,
            })),
        ),
        (
            "agent state missing agent",
            wrap(M::AgentStateChanged(pb::AgentStateChanged {
                agent: None,
                released: true,
            })),
        ),
        (
            "agent state pane zero default entry",
            wrap(M::AgentStateChanged(pb::AgentStateChanged {
                agent: Some(pb::AgentEntry::default()),
                released: false,
            })),
        ),
    ]
}

fn lossy_client_inputs() -> Vec<(&'static str, pb::ClientMessage)> {
    use pb::client_message::Message as M;
    let wrap = |m: M| pb::ClientMessage { message: Some(m) };
    let mouse = |button: u32, event_type: &str| pb::MouseInput {
        col: 70_000,
        row: 3,
        button,
        shift: false,
        ctrl: true,
        alt: false,
        event_type: event_type.into(),
    };
    vec![
        ("empty client oneof", pb::ClientMessage { message: None }),
        (
            "input invalid utf8",
            wrap(M::Input(pb::Input {
                data: vec![0xff, b'z'],
            })),
        ),
        (
            "mouse button clamps to 255",
            wrap(M::Mouse(mouse(1000, "release"))),
        ),
        (
            "mouse unknown event type is press",
            wrap(M::Mouse(mouse(1, "wiggle"))),
        ),
        (
            "subscribe drops unknown ints",
            wrap(M::Subscribe(pb::Subscribe {
                events: vec![0, 1, 9999, -3, pb::EventType::ScreenCleared as i32],
            })),
        ),
        (
            "selection request truncates u16",
            wrap(M::Selection(pb::SelectionRequest {
                start_col: 65_536,
                start_row: 1,
                end_col: 2,
                end_row: 65_539,
                mode: "w".into(),
            })),
        ),
    ]
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn render() -> String {
    let mut out = String::new();
    for msg in server_fixtures() {
        let wire: pb::ServerMessage = (&msg).into();
        let bytes = wire.encode_to_vec();
        let back = AppServerMessage::try_from(pb::ServerMessage::decode(&*bytes).unwrap());
        let _ = writeln!(out, "S {}\n  = {:?}", hex(&bytes), back);
    }
    for msg in client_fixtures() {
        let wire: pb::ClientMessage = (&msg).into();
        let bytes = wire.encode_to_vec();
        let back = AppClientMessage::try_from(pb::ClientMessage::decode(&*bytes).unwrap());
        let _ = writeln!(out, "C {}\n  = {:?}", hex(&bytes), back);
    }
    for (name, wire) in lossy_server_inputs() {
        let _ = writeln!(out, "LS {name}\n  = {:?}", AppServerMessage::try_from(wire));
    }
    for (name, wire) in lossy_client_inputs() {
        let _ = writeln!(out, "LC {name}\n  = {:?}", AppClientMessage::try_from(wire));
    }
    out
}

#[test]
fn proto_golden_wire_conversions_are_unchanged() {
    let rendered = render();
    if std::env::var_os("PROTO_GOLDEN_UPDATE").is_some() {
        std::fs::write(GOLDEN_PATH, &rendered).expect("write golden");
        return;
    }
    assert_eq!(
        rendered, GOLDEN,
        "app <-> protobuf conversion output changed; see the module doc to regenerate"
    );
}

/// Every variant appears in the fixtures: a new variant fails this match
/// until a fixture (and a golden line) covers it.
#[test]
fn proto_golden_fixtures_cover_every_variant() {
    use AppClientMessage as C;
    use AppServerMessage as S;
    let mut seen = std::collections::BTreeSet::new();
    for msg in server_fixtures() {
        seen.insert(match msg {
            S::Output { .. } => "Output",
            S::Resize { .. } => "Resize",
            S::Title { .. } => "Title",
            S::Connected { .. } => "Connected",
            S::Refresh { .. } => "Refresh",
            S::CursorPosition { .. } => "CursorPosition",
            S::Bell => "Bell",
            S::CwdChanged { .. } => "CwdChanged",
            S::TriggerMatched { .. } => "TriggerMatched",
            S::ActionNotify { .. } => "ActionNotify",
            S::ActionMarkLine { .. } => "ActionMarkLine",
            S::Error { .. } => "Error",
            S::Shutdown { .. } => "Shutdown",
            S::Pong => "Pong",
            S::ModeChanged { .. } => "ModeChanged",
            S::GraphicsAdded { .. } => "GraphicsAdded",
            S::HyperlinkAdded { .. } => "HyperlinkAdded",
            S::UserVarChanged { .. } => "UserVarChanged",
            S::ProgressBarChanged { .. } => "ProgressBarChanged",
            S::BadgeChanged { .. } => "BadgeChanged",
            S::SelectionChanged { .. } => "SelectionChanged",
            S::ClipboardSync { .. } => "ClipboardSync",
            S::ShellIntegrationEvent { .. } => "ShellIntegrationEvent",
            S::SystemStats { .. } => "SystemStats",
            S::ZoneOpened { .. } => "ZoneOpened",
            S::ZoneClosed { .. } => "ZoneClosed",
            S::ZoneScrolledOut { .. } => "ZoneScrolledOut",
            S::EnvironmentChanged { .. } => "EnvironmentChanged",
            S::RemoteHostTransition { .. } => "RemoteHostTransition",
            S::SubShellDetected { .. } => "SubShellDetected",
            S::SemanticSnapshot { .. } => "SemanticSnapshot",
            S::FileTransferStarted { .. } => "FileTransferStarted",
            S::FileTransferProgress { .. } => "FileTransferProgress",
            S::FileTransferCompleted { .. } => "FileTransferCompleted",
            S::FileTransferFailed { .. } => "FileTransferFailed",
            S::UploadRequested { .. } => "UploadRequested",
            S::ScreenCleared { .. } => "ScreenCleared",
            S::AgentRoster { .. } => "AgentRoster",
            S::AgentStateChanged { .. } => "AgentStateChanged",
        });
    }
    assert_eq!(seen.len(), 39, "server fixtures miss a variant");
    let mut seen = std::collections::BTreeSet::new();
    for msg in client_fixtures() {
        seen.insert(match msg {
            C::Input { .. } => "Input",
            C::Resize { .. } => "Resize",
            C::Ping => "Ping",
            C::RequestRefresh => "RequestRefresh",
            C::Subscribe { .. } => "Subscribe",
            C::Mouse { .. } => "Mouse",
            C::FocusChange { .. } => "FocusChange",
            C::Paste { .. } => "Paste",
            C::SelectionRequest { .. } => "SelectionRequest",
            C::ClipboardRequest { .. } => "ClipboardRequest",
            C::SnapshotRequest { .. } => "SnapshotRequest",
        });
    }
    assert_eq!(seen.len(), 11, "client fixtures miss a variant");
}
