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

/// Card 01a205ba, criterion 1 — the pinned desync repro. The daemon's
/// sink dispatch lags its grid apply (ARC-001 dispatches the output
/// callback after the terminal write guard drops), so a chunk whose
/// bytes a replay's export already captured can arrive AFTER the reply.
/// The reply's `gen=` fence must drop it: feeding it would paint the
/// stale bytes onto the replayed screen — the holed, mixed-era grid the
/// sighting showed. A chunk above the fence feeds normally, and the
/// pre-fence behavior (generation `None`) is proven to still mutate the
/// grid, so the fence is what holds the line.
#[test]
fn a_replay_covered_chunk_arriving_after_the_reply_is_dropped() {
    // The replay carries fence gen=10 and leaves the cursor after the
    // seed row (the body's lines join back to the exact restore bytes,
    // so no trailing newline is implied).
    let (rx, mut conn) = fake_daemon(
        "replay-fence-desync",
        FakeScript {
            gen_replies: [(
                "refresh-client -t %1".to_string(),
                ("SEED-ROW".to_string(), 10),
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

    assert!(session.replay_panes(&mut conn, &[1]), "the replay is ok");
    drained(&rx);

    // Positive control — the pre-fence behavior mutates the grid: a
    // generation-less chunk paints at the tracked cursor, exactly the
    // double-apply an unfenced client performs.
    session.feed_pane("%1", b"UNFENCED", None);
    let row = |session: &WindowSession| {
        let grid = session
            .renderer
            .pane_terminal(1)
            .expect("pane 1")
            .active_grid();
        (0..40)
            .map(|col| grid.get(col, 0).expect("cell").c().to_string())
            .collect::<String>()
    };
    assert_eq!(row(&session).trim_end(), "SEED-ROWUNFENCED");

    // The gap shape, fenced: a chunk stamped AT the fence was already on
    // the replayed screen — dropped, the grid keeps the replay's row.
    session.feed_pane("%1", b"STALE", Some(10));
    assert_eq!(
        row(&session).trim_end(),
        "SEED-ROWUNFENCED",
        "the stale chunk did not apply"
    );
    session.feed_pane("%1", b"OLDER", Some(4));
    assert_eq!(
        row(&session).trim_end(),
        "SEED-ROWUNFENCED",
        "a pre-fence generation did not apply either"
    );

    // A chunk above the fence feeds: fresh bytes reach the grid.
    session.feed_pane("%1", b"FRESH", Some(11));
    assert_eq!(row(&session).trim_end(), "SEED-ROWUNFENCEDFRESH");
}

/// Card 01a205ba, criterion 3 — the feed invariant: with a fence set,
/// every chunk that reaches the grid carries a generation strictly above
/// it. The mixed stream below (replay-covered, older, fresh,
/// un-stamped) leaves the grid holding exactly the fence-plus content.
#[test]
fn only_chunks_above_the_fence_reach_the_grid() {
    let (rx, mut conn) = fake_daemon(
        "replay-fence-invariant",
        FakeScript {
            gen_replies: [("refresh-client -t %2".to_string(), ("SEED".to_string(), 5))]
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

    assert!(session.replay_panes(&mut conn, &[2]));
    drained(&rx);

    // Chunks in arrival order: replay-covered (5), older (2), fresh (6
    // and 7), and an un-stamped one (a pre-fence daemon feeds as today).
    session.feed_pane("%2", b"[covered]", Some(5));
    session.feed_pane("%2", b"[old]", Some(2));
    session.feed_pane("%2", b"[fresh6]", Some(6));
    session.feed_pane("%2", b"[fresh7]", Some(7));
    session.feed_pane("%2", b"[unstamped]", None);

    let grid = session
        .renderer
        .pane_terminal(2)
        .expect("pane 2")
        .active_grid();
    let row: String = (0..40)
        .map(|col| grid.get(col, 0).expect("cell").c().to_string())
        .collect();
    assert_eq!(
        row.trim_end(),
        "SEED[fresh6][fresh7][unstamped]",
        "only fence-plus chunks reached the grid"
    );
}

/// A replay whose reply carries no `gen=` flag (a pre-fence daemon)
/// clears any prior fence: the pane feeds unfenced rather than trusting
/// coverage an older replay proved.
#[test]
fn an_unfenced_replay_clears_the_prior_fence() {
    let (rx, mut conn) = fake_daemon(
        "replay-fence-clear",
        FakeScript {
            replies: [("refresh-client -t %1".to_string(), "PLAIN".to_string())]
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

    // A fence from an earlier generation-aware replay.
    session.output_fences.insert(1, 100);
    assert!(session.replay_panes(&mut conn, &[1]));
    drained(&rx);

    // The fence is gone: a chunk below the old fence number feeds.
    session.feed_pane("%1", b"X", Some(1));
    let grid = session
        .renderer
        .pane_terminal(1)
        .expect("pane 1")
        .active_grid();
    let row: String = (0..8)
        .map(|col| grid.get(col, 0).expect("cell").c().to_string())
        .collect();
    assert_eq!(row.trim_end(), "PLAINX", "no fence: the chunk feeds");
}

/// A `%pane-respawned` resets the fence with the held cue: the respawned
/// pane's fresh terminal has a fresh generation chain, so a chunk the
/// old fence would have covered must feed.
#[test]
fn a_respawn_clears_the_fence() {
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    session.output_fences.insert(1, 100);

    session.on_pane_respawned("%1");

    session.feed_pane("%1", b"X", Some(1));
    let grid = session
        .renderer
        .pane_terminal(1)
        .expect("pane 1")
        .active_grid();
    let row: String = (0..4)
        .map(|col| grid.get(col, 0).expect("cell").c().to_string())
        .collect();
    assert_eq!(row.trim_end(), "X", "the respawned pane feeds unfenced");
}
