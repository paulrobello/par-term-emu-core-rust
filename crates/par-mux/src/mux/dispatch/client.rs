//! Client-facing query and sizing handlers: refresh-client and list-agents.

use super::*;
use crate::mux::layout::PaneChrome;

/// The client group's router; `route_command` sends only this group's
/// variants here.
pub(super) fn route_client_command(ctx: &Ctx<'_>, command: MuxCommand) -> Outcome {
    match command {
        MuxCommand::RefreshClient {
            pane,
            size,
            cell_pixels,
            chrome,
        } => cmd_refresh_client(ctx, pane, size, cell_pixels, chrome.unwrap_or_default()),
        MuxCommand::ListAgents => cmd_list_agents(ctx),
        other => unreachable!("route_command sent a non-client command: {other:?}"),
    }
}

pub(super) fn cmd_refresh_client(
    ctx: &Ctx<'_>,
    pane: Option<Target<PaneId>>,
    size: Option<(u16, u16)>,
    cell_pixels: Option<(u16, u16)>,
    chrome: PaneChrome,
) -> Outcome {
    // `-p` is applied first and independently of `-C` and of the target:
    // the cell size is daemon-wide state every later re-fit reads
    // (sync_pane_sizes), so a report lands the pixels even when the grid
    // resize below fails, and a pixels-only report re-fits at the current
    // grid through the same sync path. This runs before pane resolution so
    // the attach handshake's target-less report takes effect.
    if let Some((cell_w, cell_h)) = cell_pixels {
        ctx.tree.lock().set_client_cell_pixels(cell_w, cell_h);
    }
    match pane {
        None => {
            // The target-less size-report form (the attach handshake's
            // shape): never a replay — replay is what the -t form exists
            // for, and resyncing a pane the client never named would push
            // it screen bytes of the wrong pane. `-C` records the report as
            // the connection's sizing contribution against the window it
            // displays (the bare form's documented stand-in: the newest
            // session's active window) and re-fits that window to the
            // smallest-attached-client minimum.
            match size {
                Some((cols, rows)) => {
                    let outcome: Result<(WindowId, Vec<WindowId>), String> = {
                        let mut guard = ctx.tree.lock();
                        // Newest = highest id (ids are monotonic). The
                        // map has no insertion order, so this is the
                        // deterministic spelling of bare new-window's
                        // documented stand-in.
                        let session = guard.sessions().into_iter().max();
                        match session {
                            Some(session) => {
                                let window = guard
                                    .session(session)
                                    .and_then(|s| s.windows.get(s.active).copied());
                                match window {
                                    Some(window_id) => {
                                        let resized = match ctx.client_id {
                                            Some(client_id) => guard.set_client_view(
                                                client_id, window_id, cols, rows, chrome,
                                            ),
                                            // No connection identity (embedder/test
                                            // dispatch): keep the legacy direct resize.
                                            None => {
                                                match guard.resize_window_with_chrome(
                                                    window_id, cols, rows, chrome,
                                                ) {
                                                    Ok(()) => vec![window_id],
                                                    Err(err) => {
                                                        return Outcome::err(ctx, &err.to_string())
                                                    }
                                                }
                                            }
                                        };
                                        Ok((window_id, resized))
                                    }
                                    None => Err(format!("no such session: {session}")),
                                }
                            }
                            None => Err("no sessions exist to size".to_string()),
                        }
                    };
                    match outcome {
                        Ok((window_id, resized)) => {
                            let mut outcome = Outcome::ok(ctx, "").with_layout(window_id);
                            for extra in resized {
                                if extra != window_id {
                                    outcome = outcome.with_layout(extra);
                                }
                            }
                            outcome
                        }
                        Err(err) => Outcome::err(ctx, &err),
                    }
                }
                // Sizeless and targetless is meaningless (the parser
                // rejects it), but dispatch stays total: an empty ok.
                None => Outcome::ok(ctx, ""),
            }
        }
        Some(pane) => {
            let pane = {
                let guard = ctx.tree.lock();
                match guard.resolve_pane_target(pane) {
                    Ok(id) => id,
                    Err(err) => return Outcome::err(ctx, &err.to_string()),
                }
            };
            match size {
                // The window-size policy's input: a client's renderer reports
                // its grid against one of the window's panes; the report is
                // recorded as that connection's sizing contribution and the
                // window re-fits to the smallest-attached-client minimum.
                // Every pane terminal re-fits to the re-divided geometry (less the
                // declared `-I` chrome) —
                // followed by a %layout-change so clients re-render (the
                // seed path requires one even when the minimum did not move
                // the grid).
                Some((cols, rows)) => {
                    let outcome = {
                        let mut guard = ctx.tree.lock();
                        match guard.window_of_pane(pane) {
                            Some(window_id) => {
                                let resized = match ctx.client_id {
                                    Some(client_id) => guard
                                        .set_client_view(client_id, window_id, cols, rows, chrome),
                                    None => match guard
                                        .resize_window_with_chrome(window_id, cols, rows, chrome)
                                    {
                                        Ok(()) => vec![window_id],
                                        Err(err) => return Outcome::err(ctx, &err.to_string()),
                                    },
                                };
                                Ok((window_id, resized))
                            }
                            None => Err(MuxError::NoSuchPane(pane)),
                        }
                    };
                    match outcome {
                        Ok((window_id, resized)) => {
                            let mut outcome = Outcome::ok(ctx, "").with_layout(window_id);
                            for extra in resized {
                                if extra != window_id {
                                    outcome = outcome.with_layout(extra);
                                }
                            }
                            outcome
                        }
                        Err(err) => Outcome::err(ctx, &err.to_string()),
                    }
                }
                // Resync (D5.4): replay the pane's state as the screen-restore
                // encoder's byte stream so a reattached client's emulator
                // reproduces it exactly — the main screen's scrollback first (a
                // reattached pane can scroll back), alt-screen selection (a TUI
                // replays its TUI screen), then the styled content with
                // absolute row addressing (`\x1b[R;1H`; a `\n`-joined reply
                // staircases: LF preserves the column), attributes via SGR
                // (a plain reply loses every color), trailing background-styled
                // cells (plain text trims them), and finally the cursor
                // position/visibility/style and the input modes a full-screen
                // app's next %output deltas assume.
                None => {
                    let guard = ctx.tree.lock();
                    match guard.pane(pane) {
                        Some(target) => {
                            let screen = target.terminal().read().export_screen_restore_sequence();
                            Outcome::ok(ctx, &screen)
                        }
                        None => Outcome::err(ctx, &format!("no such pane: {pane}")),
                    }
                }
            }
        }
    }
}

/// Build one roster row: pane, agent, state, source, then zero or more
/// whitespace-free `key=value` tokens — `reason=<base64>` (blocked
/// reason), `telemetry=<base64>`, `host_telemetry=<base64>`. Every token
/// after `source` is key=value, so a consumer parses positions 1-4 then
/// splits each remaining token on its first `=` (ARC-060: a free-text
/// reason column was ambiguous).
pub(super) fn roster_row(
    pane: PaneId,
    agent: &str,
    state: &str,
    source: &str,
    reason: Option<&str>,
    telemetry_b64: Option<&str>,
    host_telemetry_b64: Option<&str>,
) -> crate::mux::ipc::AgentRow {
    let mut extras = Vec::new();
    if let Some(reason) = reason {
        let encoded = base64::engine::general_purpose::STANDARD.encode(reason.as_bytes());
        extras.push(("reason".to_string(), encoded));
    }
    if let Some(telemetry) = telemetry_b64 {
        extras.push(("telemetry".to_string(), telemetry.to_string()));
    }
    if let Some(host) = host_telemetry_b64 {
        extras.push(("host_telemetry".to_string(), host.to_string()));
    }
    crate::mux::ipc::AgentRow {
        pane: pane.to_string(),
        agent: agent.to_string(),
        state: state.to_string(),
        source: source.to_string(),
        extras,
    }
}

pub(super) fn cmd_list_agents(ctx: &Ctx<'_>) -> Outcome {
    // Wire contract: list-agents is the roster — one line per pane a
    // hook has CLAIMED or a pattern has MATCHED, `%N <agent> <state>
    // <source>` with source `hook` or `scrape` (T5.4 + the scrape
    // tier's provenance rule: a consumer must tell a claim from a
    // guess), then zero or more whitespace-free `key=value` tokens:
    // `reason=<base64>` (the blocked reason, standard base64 of the
    // whitespace-collapsed message) and `telemetry=<base64>` /
    // `host_telemetry=<base64>` (fresh samples only). Every token
    // after `source` is key=value, so a consumer parses positions 1-4
    // then splits each remaining token on its first `=` — a free-text
    // reason column after `source` was ambiguous (ARC-060: a reason
    // reading `hook` or containing `telemetry=` defeated positional
    // parsers). Panes without state are absent outright: `unknown`
    // means no hook ever reported and no rule ever matched, never
    // "idle" (the Phase 5 ruling). Fixed shape, no -F — the T4.E
    // decision.
    let guard = ctx.tree.lock();
    let mut roster: Vec<(PaneId, crate::mux::ipc::AgentRow)> = guard
        .sessions()
        .iter()
        .filter_map(|s| guard.session(*s))
        .flat_map(|s| s.windows.clone())
        .filter_map(|w| guard.window(w))
        .flat_map(|w| w.panes())
        .filter_map(|p| {
            let pane = guard.pane(p)?;
            let state = pane.metadata().get("agent_state")?;
            let agent = pane.metadata().get("agent")?;
            let source = pane
                .metadata()
                .get("agent_state_source")
                .map(String::as_str)
                .unwrap_or("hook");
            // The blocked reason rides as one whitespace-free
            // `reason=<base64>` token (same standard engine as the
            // telemetry tokens). Absent message, no token.
            let reason = pane.metadata().get("agent_message").map(String::as_str);
            // Fresh telemetry rides as one final whitespace-free token
            // (base64 of the canonical JSON — string values carry
            // spaces). Stale or absent telemetry adds nothing, so a
            // pane without it keeps the exact pre-telemetry row. The
            // host probe's sibling token follows the same rule, aged
            // per field — and neither ever triggers a probe: the roster
            // reads only what the cadence thread already wrote.
            let telemetry = crate::mux::hooks::fresh_telemetry_b64(pane.telemetry.as_ref());
            let host =
                crate::mux::host_probe::fresh_host_telemetry_b64(pane.host_telemetry.as_ref());
            Some((
                p,
                roster_row(p, agent, state, source, reason, telemetry, host.as_deref()),
            ))
        })
        .collect();
    roster.sort_by_key(|(pane, _)| *pane);
    let body = roster
        .iter()
        .map(|(_, row)| row.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    Outcome::ok(ctx, &body)
}
