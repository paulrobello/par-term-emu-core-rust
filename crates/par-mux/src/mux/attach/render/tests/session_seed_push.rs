//! WindowSession suites: seed installs and pushes handled mid-seed.

use super::*;

/// The seed script for the mid-seed toast tests: `@0` holds pane `%9`
/// alone, the agent's pane `%1` is in the background, and the snapshot
/// already shows the agent `blocked` (its report landed during the
/// seed). `push_on` is the command whose reply the agent's push rides
/// ahead of.
fn mid_seed_script(push_on: &str) -> FakeScript {
    let mut script = snapshot_script();
    let snapshot = script
        .replies
        .get("client-snapshot -t @0")
        .expect("snapshot reply")
        .replace(
            "agent %1 claude working hook",
            "agent %1 claude blocked hook ws=+0",
        );
    script
        .replies
        .insert("client-snapshot -t @0".to_string(), snapshot);
    script
        .replies
        .insert("pane-info -t %9".to_string(), "%9 @0 90x24".to_string());
    script.notify.insert(
        "refresh-client *".to_string(),
        "%layout-change @0 0000,90x24,0,0,9 0000,90x24,0,0,9 *".to_string(),
    );
    let push = "%agent-state-changed %1 claude blocked source=hook ws=+0";
    let note = match script.notify.get(push_on) {
        Some(existing) => format!("{existing}\n{push}"),
        None => push.to_string(),
    };
    script.notify.insert(push_on.to_string(), note);
    script
}

/// Seed `script`'s `@0` view with the side panel up, and return.
/// A re-attach with QUIET agents (no state pushes coming): the seed's
/// panel install runs before the status snapshot lands — against an
/// empty roster — so the agents section must be installed once the
/// snapshot lands via the pump's continuation machinery (the async
/// status path, ENH-049), or the panel stays agents-empty until an
/// unrelated event marks the status stale.
#[test]
fn the_seed_installs_the_agents_section_after_the_roster_lands() {
    let mut script = snapshot_script();
    script
        .replies
        .insert("pane-info -t %1".to_string(), "%1 @0 90x24".to_string());
    script.notify.insert(
        "refresh-client -t %1 -C 90x24".to_string(),
        format!(
            "%layout-change @0 {} {} *",
            "0000,90x24,0,0{30x24,0,0,1,30x24,30,0,2,30x24,60,0,3}",
            "0000,90x24,0,0{30x24,0,0,1,30x24,30,0,2,30x24,60,0,3}"
        ),
    );
    let (_rx, mut conn) = fake_daemon("seedroster", script);
    let mut session = WindowSession::new(90, 26);
    session.sidebar.set(true);
    let mut sink = super::RecordingSink::default();
    let shown = session
        .seed(&mut conn, Some("%1"), &mut sink)
        .expect("the seed lands");
    assert!(shown, "the seed shows the view without waiting on the bar");
    assert!(
        session.status.agents().is_empty(),
        "the seed returned before the roster landed — the snapshot applies via the continuation"
    );
    settle(&mut session, &mut conn);
    let sections = session
        .renderer
        .sidebar_sections
        .clone()
        .expect("panel sections");
    let agents = sections
        .iter()
        .find(|section| section.title == "agents")
        .expect("agents section installed from the snapshot");
    assert_eq!(agents.rows.len(), 2, "the snapshot's two agents");
}

fn seed_then_drain(tag: &str, script: FakeScript) -> WindowSession {
    use super::super::notify::NotifyDelivery;
    let (_rx, mut conn) = fake_daemon(tag, script);
    let mut session = WindowSession::new(90, 26);
    session.notify.delivery = NotifyDelivery::Inside;
    session.notify.states = vec!["blocked".to_string()];
    let mut sink = super::RecordingSink::default();
    session
        .seed(&mut conn, Some("%9"), &mut sink)
        .expect("the seed lands");
    assert!(session.drain_pushes(&mut conn).is_continue());
    session
}

/// An agent's report that lands while the client is still seeding (the
/// test barrier `?1002h` is written before the seed runs, so a loaded
/// seed overlaps the report): its push queued during the size report's
/// drain must reach the pump, not be discarded with the replay-covered
/// output around it.
#[test]
fn a_push_drained_by_the_seed_still_toasts() {
    let session = seed_then_drain("seedtoast-drain", mid_seed_script("refresh-client *"));
    let toast = session.renderer.toast.clone().expect("the push toasts");
    assert_eq!(toast.title, "claude blocked");
    assert_eq!(session.notify.outbox, vec![0x07], "and rings the bell");
}

/// The same report whose push is still in flight when the seed's own
/// status refresh answers: the snapshot already shows `blocked`, so the
/// transition baseline must not adopt it for that pane, or the push
/// reads as a repeat.
#[test]
fn a_push_queued_across_the_seed_refresh_still_toasts() {
    let session = seed_then_drain(
        "seedtoast-refresh",
        mid_seed_script("client-snapshot -t @0"),
    );
    let toast = session.renderer.toast.clone().expect("the push toasts");
    assert_eq!(toast.title, "claude blocked");
    assert_eq!(session.notify.outbox, vec![0x07], "and rings the bell");
}

/// A push that lands before any roster commit (the seed's refresh timed
/// out) must not seed the transition baseline from that partial view:
/// the first committed roster does, so a pre-existing blocked agent's
/// unchanged rebroadcast (`pane.report_agent_session` re-sends the held
/// state) stays silent.
#[test]
fn a_push_before_the_first_roster_does_not_seed_a_partial_baseline() {
    use super::super::notify::NotifyDelivery;
    let mut session = snapshot_session();
    session.notify.delivery = NotifyDelivery::Inside;
    session.notify.states = vec!["blocked".to_string()];
    session
        .renderer
        .apply_layout(parse_layout("0000,90x24,0,0,9").expect("parses"));
    let push = |state: &str, pane: &str, agent: &str| TmuxNotification::AgentStateChanged {
        pane_id: pane.to_string(),
        agent: agent.to_string(),
        state: state.to_string(),
        source: "hook".to_string(),
        ws: "+0".to_string(),
        msg: String::new(),
    };
    session.handle_event(push("working", "%1", "claude"));
    // The first roster commit: kimi on %3 was already blocked.
    let (rx, mut conn) = fake_daemon("snapunseeded", snapshot_script());
    session.refresh_status_facts(&mut conn).expect("seed");
    drained(&rx);
    session.handle_event(push("blocked", "%3", "kimi"));
    assert!(
        session.renderer.toast.is_none(),
        "an unchanged rebroadcast is not a transition"
    );
    assert!(session.notify.outbox.is_empty(), "and rings nothing");
    // A real transition after it still toasts.
    session.handle_event(push("blocked", "%1", "claude"));
    assert!(session.renderer.toast.is_some());
}

/// The seed's race window: a push handled while the seed's snapshot is
/// in flight toasts (no roster is committed to judge it against), but
/// the push stream holds no record for the pane — `record_pushed_state`
/// seeds no baseline from the uncommitted view. The continuation's
/// baseline adoption must therefore leave the raced pane out (the sync
/// path drained its pre-refresh pushes and excluded them); otherwise
/// the snapshot's entry suppresses an identical rebroadcast on a record
/// the push stream never made.
#[test]
fn a_push_handled_mid_seed_stays_out_of_the_baseline_and_its_rebroadcast_toasts() {
    use super::super::notify::NotifyDelivery;
    let mut script = snapshot_script();
    // The daemon applies a report before it broadcasts it, so the
    // snapshot already carries the state the push announces.
    let snapshot = script
        .replies
        .get("client-snapshot -t @0")
        .expect("snapshot reply")
        .clone();
    script.replies.insert(
        "client-snapshot -t @0".to_string(),
        format!("{snapshot}\nagent %2 claude blocked hook ws=+0"),
    );
    // The push rides the snapshot reply (the wire order the race lives
    // in); the rebroadcast rides the pump's own refresh.
    script.notify.insert(
        "client-snapshot -t @0".to_string(),
        "%agent-state-changed %2 claude blocked source=hook ws=+0".to_string(),
    );
    let (rx, mut conn) = fake_daemon("seedrace", script);
    let mut session = snapshot_session();
    session.notify.delivery = NotifyDelivery::Inside;
    session.notify.states = vec!["blocked".to_string()];
    // The pushed pane is background (herdr's rule): a lone pane layout
    // takes %2 out of view.
    session
        .renderer
        .apply_layout(parse_layout("0000,90x24,0,0,9").expect("parses"));
    drained(&rx);

    // The seed's snapshot writes without waiting; the push is handled
    // while it is in flight. The fake writes the push ahead of the
    // reply block; the bounded drain waits out that write.
    session.seed_status_snapshot(&mut conn, Vec::new());
    assert!(session.status_inflight.flight.is_some());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while session.renderer.toast.is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "the mid-seed push never arrived"
        );
        assert!(session.drain_pushes(&mut conn).is_continue());
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let toast = session
        .renderer
        .toast
        .clone()
        .expect("the mid-seed push toasts");
    assert_eq!(toast.pane, Some(2));
    assert!(
        session.notify.transitions.is_none(),
        "handling a push seeds no baseline from the partial view"
    );

    // The reply's continuation adopts the baseline; the raced pane
    // stays out of it.
    settle(&mut session, &mut conn);
    assert!(session.status_inflight.flight.is_none());
    let baseline = session
        .notify
        .transitions
        .as_ref()
        .expect("the first roster seeded the baseline");
    assert!(
        !baseline.contains_key(&2),
        "the raced pane stays out of the baseline: {baseline:?}"
    );

    // The identical rebroadcast (the daemon re-announcing the state it
    // applied before it wrote the snapshot) toasts: there is no
    // push-stream record to suppress it on. It rides the pump's own
    // refresh (the way a late registration replay arrives).
    session.status_inflight.dirty = true;
    assert_eq!(session.refresh_status(&mut conn), EventOutcome::Continue);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while session.notify.outbox.len() < 2 {
        assert!(
            std::time::Instant::now() < deadline,
            "the rebroadcast never arrived"
        );
        assert!(session.drain_pushes(&mut conn).is_continue());
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let toast = session
        .renderer
        .toast
        .clone()
        .expect("the identical rebroadcast toasts");
    assert_eq!(toast.pane, Some(2));

    // Once recorded by that toast, an unchanged repeat is silent again:
    // no new bell (the rebroadcast's card is still the one up).
    let bells = session.notify.outbox.len();
    session.handle_event(TmuxNotification::AgentStateChanged {
        pane_id: "%2".to_string(),
        agent: "claude".to_string(),
        state: "blocked".to_string(),
        source: "hook".to_string(),
        ws: "+0".to_string(),
        msg: String::new(),
    });
    assert_eq!(
        session.notify.outbox.len(),
        bells,
        "a recorded repeat stays silent"
    );
}
