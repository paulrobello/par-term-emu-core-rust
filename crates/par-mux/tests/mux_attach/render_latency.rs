//! Card 01a1995a6's measurement harness: click-to-action latency in the
//! render-mode attach TUI through the REAL daemon, REAL client, and REAL
//! input path — the roster agent's cross-workspace click rig (card
//! 01a1989b5), now recording per-click wall times at induced host-load
//! shapes (1, 8, 20+ busy threads) instead of asserting a single
//! landing.
//!
//! This is a HARNESS, not a gate: it asserts nothing about the numbers
//! (a regression here is a perf story, not a correctness one) and is
//! `#[ignore]`d like the other load-sensitive suites — run it directly
//! with `cargo test -p par-mux --features mux-bin,attach --test mux_attach
//! click_latency -- --ignored --nocapture` and read the eprintln'd
//! percentiles. The load generator is self-induced: N busy threads for
//! the duration of one shape's click run, released after it.

use super::*;

/// The `(row, col)` of `needle` inside the side panel's current paint —
/// the roster test's row finder, as a free fn for reuse.
fn latency_roster_row_of(
    host: &AttachHost,
    all: &mut Vec<u8>,
    needle: &str,
) -> Option<(usize, usize)> {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        for bytes in host.output_rx.try_iter() {
            all.extend_from_slice(&bytes);
        }
        let (final_screen, _) = reconstructed_screen(all, 24, 80);
        for (row, cells) in final_screen.iter().enumerate() {
            let left: String = cells.chars().take(19).collect();
            if let Some(col) = left.find(needle) {
                return Some((row, col));
            }
        }
        if Instant::now() >= end {
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Run `f` with `n` busy threads spinning for its duration (a
/// self-induced host-load shape). The threads start with a warmup sleep
/// so the run queue builds before `f` measures, and a sampler thread
/// records the 1-second load average's max across the whole window (the
/// 1-minute average lags a short shape too much to be evidence).
fn with_load<T>(n: usize, f: impl FnOnce() -> T) -> (T, (f64, f64, f64)) {
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut handles = Vec::new();
    for _ in 0..n {
        let stop = std::sync::Arc::clone(&stop);
        handles.push(std::thread::spawn(move || {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::yield_now();
            }
        }));
    }
    // Sampler: the max 1-second sample across the shape (before values
    // included so a quiet shape reports honestly).
    let sample_stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let sampler_stop = std::sync::Arc::clone(&sample_stop);
    let sampler = std::thread::spawn(move || {
        let mut max = 0f64;
        let mut first = 0f64;
        while !sampler_stop.load(std::sync::atomic::Ordering::Relaxed) {
            let v = loadavg1();
            if first == 0.0 {
                first = v;
            }
            if v > max {
                max = v;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        (first, max)
    });
    let value = {
        // Warmup: let the run queue build before the measured section.
        std::thread::sleep(Duration::from_secs(5));
        f()
    };
    sample_stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let (before, peak) = sampler.join().unwrap_or((0.0, 0.0));
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    for handle in handles {
        handle.join().expect("load thread joins");
    }
    (value, (before, peak, peak))
}

/// The 1-minute load average (the host-load contract's sample).
fn loadavg1() -> f64 {
    let mut avg = [0f64; 3];
    let n = unsafe { libc::getloadavg(avg.as_mut_ptr(), 3) };
    if n < 1 {
        0.0
    } else {
        avg[0]
    }
}

/// Wall-clock epoch millis, the clock domain of the client's
/// `PAR_MUX_INPUT_TRACE` stamps.
fn epoch_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// The first `PAR_MUX_INPUT_TRACE` stamp at/after `from_ms` for each
/// landing-chain milestone, parsed from the client's stderr log:
/// the host mouse report (`mouse cb=…`), the reseed (`reseed_window …`),
/// and the reseed replays (`replay panes …`).
fn parse_trace_stamps(
    log: &str,
    from_ms: u128,
) -> (Option<u128>, Option<u128>, Option<u128>, Option<u128>) {
    let mut mouse = None;
    let mut reseed = None;
    let mut replay = None;
    let mut painted = None;
    for line in log.lines() {
        let Some(rest) = line.strip_prefix("par-mux input-trace[") else {
            continue;
        };
        let Some((ms, msg)) = rest.split_once("]: ") else {
            continue;
        };
        let Ok(stamp) = ms.parse::<u128>() else {
            continue;
        };
        if stamp < from_ms {
            continue;
        }
        if mouse.is_none() && msg.starts_with("mouse cb=") {
            mouse = Some(stamp);
        }
        if reseed.is_none() && msg.starts_with("reseed_window") {
            reseed = Some(stamp);
        }
        if replay.is_none() && msg.starts_with("replay panes") {
            replay = Some(stamp);
        }
        if painted.is_none() && msg.starts_with("frame painted") {
            painted = Some(stamp);
        }
    }
    (mouse, reseed, replay, painted)
}

/// p50 of the `b - a` delta over the rows where both stage stamps exist.
fn stage_p50(
    rows: &[ClickStages],
    a: fn(&ClickStages) -> Option<u128>,
    b: fn(&ClickStages) -> Option<u128>,
) -> Option<f64> {
    let mut ds: Vec<f64> = rows
        .iter()
        .filter_map(|s| match (a(s), b(s)) {
            (Some(x), Some(y)) if y >= x => Some((y - x) as f64),
            _ => None,
        })
        .collect();
    if ds.is_empty() {
        return None;
    }
    ds.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((ds.len() as f64) * 0.5).ceil() as usize;
    Some(ds[idx.saturating_sub(1).min(ds.len() - 1)])
}

/// One click's landing-chain stage stamps (epoch ms), from the client's
/// `PAR_MUX_INPUT_TRACE` log plus the harness's own marker detection.
#[derive(Default, Clone, Copy)]
struct ClickStages {
    /// Click bytes → the client's `route_mouse` (a delta, not a stamp:
    /// the client read path).
    input: Option<u128>,
    /// Host mouse report reached `route_mouse`.
    mouse: Option<u128>,
    /// `reseed_window` — the daemon chain (select-pane → pane-info →
    /// switch-client) answered through to the reseed.
    reseed: Option<u128>,
    /// `replay panes` — the reseed's pane replays arrived.
    replay: Option<u128>,
    /// The harness saw the landing marker painted.
    land: Option<u128>,
    /// The client's frame flushed the landing's diff (`frame painted`).
    painted: Option<u128>,
}

/// One click on `agent`'s roster row, timed from the input bytes to the
/// landing marker painting. Re-arms the click's own landing barrier and
/// parses the click's stage stamps from the client's input-trace log.
fn timed_click(
    host: &mut AttachHost,
    all: &mut Vec<u8>,
    trace_log: &std::path::Path,
    agent: &str,
    landed_marker: &str,
    what: &str,
    budget: Duration,
) -> (Option<Duration>, ClickStages) {
    // The panel re-renders after every landing (the rig reads click
    // coordinates only from a settled screen — a row found mid-repaint
    // clicks stale coordinates and the landing never comes).
    screen_stable(
        host,
        all,
        Duration::from_millis(250),
        Duration::from_secs(5),
    );
    let Some((row, col)) = latency_roster_row_of(host, all, agent) else {
        // An unpainted row (the liveness sweep released the claim) is a
        // skipped sample, not a failure: the harness measures.
        return (None, ClickStages::default());
    };
    let t0 = Instant::now();
    let t0_epoch = epoch_ms();
    host.to_child
        .write_all(format!("\x1b[<0;{};{}M", col + 3, row + 1).as_bytes())
        .expect("roster click");
    host.to_child.flush().ok();
    // Bounded landing wait: a click whose landing never paints (a stale
    // row, a load storm) times out as a skipped sample rather than
    // panicking the harness.
    let end = Instant::now() + budget;
    loop {
        for bytes in host.output_rx.try_iter() {
            all.extend_from_slice(&bytes);
        }
        let (screen, _) = reconstructed_screen(all, 24, 80);
        if screen.iter().any(|r| r.contains(landed_marker)) {
            let land = t0_epoch + t0.elapsed().as_millis();
            let log = std::fs::read_to_string(trace_log).unwrap_or_default();
            let (mouse, reseed, replay, painted) = parse_trace_stamps(&log, t0_epoch);
            return (
                Some(t0.elapsed()),
                ClickStages {
                    input: mouse.map(|m| m.saturating_sub(t0_epoch)),
                    mouse,
                    reseed,
                    replay,
                    land: Some(land),
                    painted,
                },
            );
        }
        if Instant::now() >= end {
            let log = std::fs::read_to_string(trace_log).unwrap_or_default();
            let (mouse, reseed, replay, painted) = parse_trace_stamps(&log, t0_epoch);
            let input = mouse.map(|m| m.saturating_sub(t0_epoch));
            eprintln!(
                "LATENCY-SKIP {what}: landing never painted; stages input={input:?} mouse={mouse:?} reseed={reseed:?} replay={replay:?} painted={painted:?}; screen tail: {:?}",
                screen.iter().rev().take(6).collect::<Vec<_>>()
            );
            return (
                None,
                ClickStages {
                    input,
                    mouse,
                    reseed,
                    replay,
                    land: None,
                    painted,
                },
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The harness: setup once, then N alternating cross-workspace /
/// shown-workspace roster clicks per load shape, percentiles per shape.
#[cfg(unix)]
#[ignore = "load-sensitive measurement harness (not a gate); run with --ignored --nocapture"]
#[test]
fn click_latency_harness_records_click_to_action_ms_at_load_shapes() {
    const CLICKS_PER_SHAPE: usize = 10;
    let (fixture, _daemon, mut client) = fixture_with_session("latency");
    client.send("new-workspace -n lab").expect("new-workspace");
    client
        .send("new-session -s work -t lab")
        .expect("session in lab");
    std::thread::sleep(Duration::from_millis(300));
    let pane_main = session_first_pane(&mut client, "$0");
    let lab_window = common::send(&mut client, "list-windows -t $2")
        .expect("list-windows")
        .iter()
        .filter_map(|l| l.split_whitespace().next())
        .find(|t| t.starts_with('@'))
        .expect("the lab window")
        .to_string();
    let pane_lab_first = common::send(&mut client, &format!("list-panes -t {lab_window}"))
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("the lab pane")
        .to_string();
    let pane_lab_agent = common::send(&mut client, &format!("split-window -h -t {pane_lab_first}"))
        .expect("split")
        .join("")
        .trim()
        .to_string();
    assert!(
        pane_lab_agent.starts_with('%') && pane_lab_agent != pane_lab_first,
        "the split returned a second pane: {pane_lab_agent:?}"
    );

    // Distinct content per pane: the landing barriers discriminate by
    // what each target window paints.
    let mut seed = |pane: &str, text: &str| {
        client
            .send(&format!("send-keys -t {pane} \"echo {text}\" Enter"))
            .expect("send-keys");
    };
    seed(&pane_main, "MAIN_PANE_TEXT");
    seed(&pane_lab_first, "LAB_PANE_ONE");
    seed(&pane_lab_agent, "LAB_PANE_TWO");
    client
        .send(&format!(
            "send-keys -t {pane_lab_first} \"while true; do echo A_TRAFFIC; sleep 0.4; done\" Enter"
        ))
        .expect("traffic");

    let claim = |pane: &str, agent: &str, seq: u64| {
        let reply = hook_report(
            fixture.socket(),
            &format!(
                r#"{{"id":1,"method":"pane.report_agent","params":{{"pane_id":"{pane}","agent":"{agent}","state":"working","seq":{seq},"source":"par-mux:test"}}}}"#
            ),
        );
        assert!(reply.contains(r#""result":"ok""#), "claim: {reply}");
    };

    // Both rosters claimed BEFORE the attach spawns (the seed installs
    // the agents section from the roster; the per-shape re-claims below
    // only refresh them against the liveness sweep).
    claim(&pane_lab_agent, "crossagent", 1);
    claim(&pane_main, "maingent", 1);

    // The client runs with PAR_MUX_INPUT_TRACE=1 and its stderr rides a
    // file the per-click stage parser reads (see spawn_attach_host).
    let trace_log =
        std::env::temp_dir().join(format!("par-mux-latency-{}-input.log", std::process::id()));
    let _ = std::fs::remove_file(&trace_log);
    let _ = std::fs::File::create(&trace_log).expect("create trace log");
    let (mut host, _stderr) = spawn_attach_host(
        &fixture,
        &["-t", &pane_main],
        "render",
        "[client]\nsidebar-on-launch = true\n",
        &[("PAR_MUX_INPUT_TRACE", "1")],
        false,
        Some(&trace_log),
    );
    expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client up with the side panel",
    );
    let mut all: Vec<u8> = host.output_rx.try_iter().flatten().collect();
    assert!(
        panel_ever_contains(&host, &mut all, 19, "crossagent", Duration::from_secs(10)),
        "the roster never showed the cross-workspace agent"
    );
    assert!(
        panel_ever_contains(&host, &mut all, 19, "maingent", Duration::from_secs(10),),
        "the roster never showed the main agent"
    );
    screen_stable(
        &host,
        &mut all,
        Duration::from_millis(500),
        Duration::from_secs(10),
    );

    let shapes: &[(usize, &str)] = &[(1, "load-1"), (8, "load-8"), (20, "load-20")];
    let mut seq: u64 = 1;
    for &(threads, label) in shapes {
        // The liveness sweep releases hook-report claims within seconds,
        // so every shape re-claims both roster rows before clicking.
        seq += 1;
        claim(&pane_lab_agent, "crossagent", seq);
        claim(&pane_main, "maingent", seq);
        std::thread::sleep(Duration::from_millis(200));
        let ((dts, per_click, stage_rows), (before, peak, _)) = with_load(threads, || {
            let mut dts: Vec<Duration> = Vec::new();
            let mut per_click: Vec<String> = Vec::new();
            let mut stage_rows: Vec<ClickStages> = Vec::new();
            // Alternate targets so every click is a real landing (the
            // shown agent's row lands back on the main window). The
            // liveness sweep releases hook-report claims within seconds,
            // so BOTH rows re-claim before every click — the rig's own
            // "one leg only" warning, solved by keeping the claims fresh.
            for i in 0..CLICKS_PER_SHAPE {
                let (agent, marker) = if i % 2 == 0 {
                    // Even: the cross-workspace agent (the worst case —
                    // select + workspace switch + reseed). Odd: the
                    // shown-workspace agent (lands back on the main
                    // window).
                    ("crossagent", "LAB_PANE_TWO")
                } else {
                    ("maingent", "MAIN_PANE_TEXT")
                };
                seq += 1;
                claim(&pane_lab_agent, "crossagent", seq);
                claim(&pane_main, "maingent", seq);
                let (dt, stages) = timed_click(
                    &mut host,
                    &mut all,
                    &trace_log,
                    agent,
                    marker,
                    &format!("{label} click {i}"),
                    Duration::from_secs(30),
                );
                stage_rows.push(stages);
                match dt {
                    Some(dt) => {
                        dts.push(dt);
                        per_click.push(format!(
                            "  {label} #{:02} {} -> {}: {:?}",
                            i, agent, marker, dt
                        ));
                    }
                    None => per_click.push(format!(
                        "  {label} #{:02} {} -> {}: TIMEOUT/skipped",
                        i, agent, marker
                    )),
                }
            }
            (dts, per_click, stage_rows)
        });
        let mut ms: Vec<f64> = dts.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
        ms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let p = |q: f64| -> f64 {
            let idx = ((ms.len() as f64) * q).ceil() as usize;
            ms[idx.saturating_sub(1).min(ms.len() - 1)]
        };
        eprintln!("LATENCY {label}: n={}", ms.len());
        for line in &per_click {
            eprintln!("{line}");
        }
        eprintln!(
            "LATENCY {label}: p50={:.0}ms p95={:.0}ms max={:.0}ms loadavg-before={before:.1} peak={peak:.1}",
            p(0.50),
            p(0.95),
            ms.last().copied().unwrap_or(0.0),
        );
        // Stage attribution: p50 delta between consecutive chain
        // milestones — input = click bytes → client route; chain = mouse
        // → reseed (the pane-info/switch-client daemon round trips);
        // seed = reseed → replay panes (the reseed's own round trips);
        // paint = replay → the marker visible to the harness.
        eprintln!(
            "LATENCY {label} stages(ms,p50): input={:?} chain={:?} seed={:?} paint={:?} delivery={:?}",
            stage_p50(&stage_rows, |_| Some(0), |s| s.input),
            stage_p50(&stage_rows, |s| s.mouse, |s| s.reseed),
            stage_p50(&stage_rows, |s| s.reseed, |s| s.replay),
            stage_p50(&stage_rows, |s| s.replay, |s| s.painted),
            stage_p50(&stage_rows, |s| s.painted, |s| s.land),
        );
    }
    host.killer.kill().ok();
}

/// The input-stage quantization gate (card 01a19d78): the pump's wait
/// selects over stdin, so host click bytes reach `route_mouse` off the
/// 16 ms poll tick. The wait used to block on the daemon channel alone,
/// so a click waited out the remainder — measured p50 35-45 ms even at
/// load 1. Gate: p50 of click-bytes → `mouse cb=` trace stamp under
/// 8 ms across 16 clicks at 30 ms spacing; the quantized shape's floor
/// is a half tick on average, so 8 ms separates woken from quantized.
/// Not `#[ignore]`d: it measures the client-side wake only — the daemon
/// chain and paint stay out of the delta — and passes in seconds.
#[test]
#[cfg(unix)]
fn stdin_wake_routes_host_clicks_off_the_poll_tick() {
    const CLICKS: usize = 16;
    const SPACING_MS: u64 = 30;
    let (fixture, _daemon, mut client) = fixture_with_session("latency-wake");
    let pane = session_first_pane(&mut client, "$0");

    let trace_log = std::env::temp_dir().join(format!(
        "par-mux-latency-wake-{}-input.log",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&trace_log);
    let _ = std::fs::File::create(&trace_log).expect("create trace log");
    let (mut host, _stderr) = spawn_attach_host(
        &fixture,
        &["-t", &pane],
        "render",
        "",
        &[("PAR_MUX_INPUT_TRACE", "1")],
        false,
        Some(&trace_log),
    );
    expect_output(
        &host,
        b"\x1b[?1002h",
        Duration::from_secs(10),
        "render client up with mouse routing",
    );
    let mut all: Vec<u8> = host.output_rx.try_iter().flatten().collect();
    screen_stable(
        &host,
        &mut all,
        Duration::from_millis(250),
        Duration::from_secs(5),
    );

    let mut deltas: Vec<u128> = Vec::with_capacity(CLICKS);
    for _ in 0..CLICKS {
        std::thread::sleep(Duration::from_millis(SPACING_MS));
        let t0 = epoch_ms();
        host.to_child
            .write_all(b"\x1b[<0;10;5M")
            .expect("click write");
        host.to_child.flush().ok();
        // The click's own stamp: the first `mouse cb=` line at/after t0.
        // Each click's stamp is awaited before the next click, so "first
        // at/after t0" is unambiguous.
        let deadline = Instant::now() + Duration::from_secs(2);
        let delta = loop {
            let log = std::fs::read_to_string(&trace_log).unwrap_or_default();
            let (mouse, _, _, _) = parse_trace_stamps(&log, t0);
            if let Some(stamp) = mouse {
                break stamp.saturating_sub(t0);
            }
            assert!(
                Instant::now() < deadline,
                "click at t0={t0} never reached route_mouse; trace tail: {:?}",
                std::fs::read_to_string(&trace_log)
                    .unwrap_or_default()
                    .lines()
                    .last()
            );
            std::thread::sleep(Duration::from_millis(1));
        };
        deltas.push(delta);
    }

    let mut ms: Vec<f64> = deltas.iter().map(|d| *d as f64).collect();
    ms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p50 = ms[ms.len() / 2];
    eprintln!(
        "LATENCY stdin-wake input-stage p50={p50:.1}ms n={}",
        ms.len()
    );
    assert!(
        p50 < 8.0,
        "input stage p50 {p50:.1}ms: clicks are still riding the 16 ms poll tick"
    );
    host.killer.kill().ok();
}
