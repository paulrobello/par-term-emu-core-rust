//! Card 01a197c0: the seed replay's failure path. A refused or failed
//! `refresh-client -t` replay used to be skipped silently, leaving the
//! pane stale behind an otherwise-complete seed — the client-side half
//! of the ghost-corruption sightings. Now it retries once and reports.

use super::*;

/// A refused `refresh-client -t` replay retries once and reports
/// failure: two wire attempts for the pane, `false` returned, and no
/// bytes fed (the pane's emulator keeps its prior state).
#[test]
fn a_refused_replay_retries_once_and_reports_failure() {
    let (rx, mut conn) = fake_daemon(
        "replay-retry-refused",
        FakeScript {
            failing: ["refresh-client -t %1".to_string()].into_iter().collect(),
            ..FakeScript::default()
        },
    );
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    drained(&rx);

    let ok = session.replay_panes(&mut conn, &[1]);
    assert!(!ok, "a refused replay is not ok after its retry");
    let attempts = drained(&rx)
        .into_iter()
        .filter(|line| line == "refresh-client -t %1")
        .count();
    assert_eq!(attempts, 2, "the refusal is retried exactly once");
}

/// A successful replay feeds the pane's emulator exactly once and
/// reports ok. The reply body's lines are the screen-restore byte
/// stream (`refresh-client -t` contract), so the pane's grid carries
/// the seeded content.
#[test]
fn a_successful_replay_feeds_the_pane_once_and_reports_ok() {
    let (rx, mut conn) = fake_daemon(
        "replay-retry-ok",
        FakeScript {
            replies: [(
                "refresh-client -t %1".to_string(),
                "SEED-CONTENT\r\n".to_string(),
            )]
            .into_iter()
            .collect(),
            ..FakeScript::default()
        },
    );
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    drained(&rx);

    let ok = session.replay_panes(&mut conn, &[1]);
    assert!(ok, "a good replay is ok");
    let attempts = drained(&rx)
        .into_iter()
        .filter(|line| line == "refresh-client -t %1")
        .count();
    assert_eq!(attempts, 1, "no retry on success");

    let grid = session
        .renderer
        .pane_terminal(1)
        .expect("pane 1")
        .active_grid();
    let row: String = (0..12)
        .map(|col| grid.get(col, 0).expect("cell").c().to_string())
        .collect();
    assert_eq!(row.trim_end(), "SEED-CONTENT", "the replay seeded the pane");
}

/// A mixed list: the failed pane's retry does not stop the other
/// panes' replays, and the result is false overall.
#[test]
fn a_failed_pane_does_not_stop_the_other_panes_replays() {
    let (rx, mut conn) = fake_daemon(
        "replay-retry-mixed",
        FakeScript {
            failing: ["refresh-client -t %1".to_string()].into_iter().collect(),
            replies: [("refresh-client -t %2".to_string(), "OTHER\r\n".to_string())]
                .into_iter()
                .collect(),
            ..FakeScript::default()
        },
    );
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    drained(&rx);

    let ok = session.replay_panes(&mut conn, &[1, 2]);
    assert!(!ok, "pane 1's persistent failure fails the batch");
    let lines = drained(&rx);
    let pane1 = lines
        .iter()
        .filter(|line| *line == "refresh-client -t %1")
        .count();
    let pane2 = lines
        .iter()
        .filter(|line| *line == "refresh-client -t %2")
        .count();
    assert_eq!(pane1, 2, "pane 1 retried once");
    assert_eq!(pane2, 1, "pane 2 replayed once, unaffected");
}
