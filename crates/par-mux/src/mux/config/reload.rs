//! Reload reporting: the client-side chord re-derivation (pure over the
//! re-read file) and the daemon-side per-setting `reload-config` report.

use super::{
    default_agent_rows, management_chord, parse_prefix, reload_chord_key, Chords, ConfigFile,
    EffectiveConfig, ExitAction, Management, SoundSettings, AGENT_APPEND_KINDS,
};

/// The client-side half of a reload, PURE over the parsed file: re-derive
/// the prefix, reload key, and management chords from `file`. `current` is
/// the fallback for settings the (possibly partial) file does not name — a
/// reload only ever moves a chord when the file actually changed it.
/// Errors on a malformed chord (the caller surfaces it on the status row).
pub fn reload_client_chords(file: &ConfigFile, current: &Chords) -> Result<Chords, String> {
    let prefix = match file.client.prefix.as_deref() {
        Some(spec) => parse_prefix(spec).ok_or_else(|| {
            format!(
                "[client] prefix {spec:?} does not parse (expected the tmux spelling, e.g. C-b)"
            )
        })?,
        None => current.prefix,
    };
    let reload = match file.client.reload.as_deref() {
        Some(chord) => reload_chord_key(chord)?,
        None => current.reload,
    };
    // Each management chord: the file's spelling when present (an error
    // names the chord), else the current binding.
    macro_rules! chord {
        ($field:ident, $label:literal) => {
            match file.client.$field.as_deref() {
                Some(chord) => management_chord(chord, $label)?,
                None => current.management.$field,
            }
        };
    }
    let management = Management {
        split_right: chord!(split_right, "split-right"),
        split_down: chord!(split_down, "split-down"),
        kill_pane: chord!(kill_pane, "kill-pane"),
        break_pane: chord!(break_pane, "break-pane"),
        join_pane: chord!(join_pane, "join-pane"),
        palette: chord!(palette, "palette"),
        new_window: chord!(new_window, "new-window"),
        resize: chord!(resize, "resize"),
        swap_prev: chord!(swap_prev, "swap-prev"),
        swap_next: chord!(swap_next, "swap-next"),
        move_window_left: chord!(move_window_left, "move-window-left"),
        move_window_right: chord!(move_window_right, "move-window-right"),
        tab_strip_left: chord!(tab_strip_left, "tab-strip-left"),
        tab_strip_right: chord!(tab_strip_right, "tab-strip-right"),
        workspace_next: chord!(workspace_next, "workspace-next"),
        workspace_prev: chord!(workspace_prev, "workspace-prev"),
        help: chord!(help, "help"),
        picker: chord!(picker, "picker"),
        zoom: chord!(zoom, "zoom"),
        rename_window: chord!(rename_window, "rename-window"),
        rename_pane: chord!(rename_pane, "rename-pane"),
        border_cycle: chord!(border_cycle, "border-cycle"),
        label_toggle: chord!(label_toggle, "label-toggle"),
        workspace_picker: chord!(workspace_picker, "workspace-picker"),
        sidebar: chord!(sidebar, "sidebar"),
        status_bar: chord!(status_bar, "status-bar"),
        respawn: chord!(respawn, "respawn"),
        notify_jump: chord!(notify_jump, "notify-jump"),
        layout_even_horizontal: chord!(layout_even_horizontal, "layout-even-horizontal"),
        layout_even_vertical: chord!(layout_even_vertical, "layout-even-vertical"),
        layout_main_horizontal: chord!(layout_main_horizontal, "layout-main-horizontal"),
        layout_main_vertical: chord!(layout_main_vertical, "layout-main-vertical"),
        layout_tiled: chord!(layout_tiled, "layout-tiled"),
    };
    // A step the file names floors at 1 — a zero/negative step would make
    // every resize a no-op by construction.
    let resize_step = file
        .client
        .resize_step
        .map(|step| step.max(1))
        .unwrap_or(current.resize_step);
    let sidebar_width = file
        .client
        .sidebar_width
        .map(|width| width.clamp(6, 60))
        .unwrap_or(current.sidebar_width);
    let sidebar_on_launch = file
        .client
        .sidebar_on_launch
        .unwrap_or(current.sidebar_on_launch);
    // Display options keep their spelling: an unknown token or value is
    // the renderer's warn-and-default (the border-lines rule, and
    // [`display_option_warnings`]), never a reload error. An empty
    // agent-rows table falls back to the default entry.
    let agent_rows = match file.client.agent_rows.as_ref() {
        Some(rows) => {
            let rows: Vec<Vec<String>> = rows.iter().filter(|r| !r.is_empty()).cloned().collect();
            if rows.is_empty() {
                default_agent_rows()
            } else {
                rows
            }
        }
        None => current.agent_rows.clone(),
    };
    let notify_delivery = file
        .client
        .notify_delivery
        .clone()
        .unwrap_or_else(|| current.notify_delivery.clone());
    let notify_states = file
        .client
        .notify_states
        .clone()
        .unwrap_or_else(|| current.notify_states.clone());
    let client = &file.client;
    let sound = SoundSettings {
        enabled: client.sound.unwrap_or(current.sound.enabled),
        player: client
            .sound_player
            .clone()
            .unwrap_or_else(|| current.sound.player.clone()),
        done: client
            .sound_done
            .clone()
            .unwrap_or_else(|| current.sound.done.clone()),
        blocked: client
            .sound_blocked
            .clone()
            .unwrap_or_else(|| current.sound.blocked.clone()),
        mute: client
            .sound_mute
            .clone()
            .unwrap_or_else(|| current.sound.mute.clone()),
    };
    // Display options: default OFF — absent keys preserve the shared-
    // divider, label-free look exactly.
    let pane_borders = file.client.pane_borders.unwrap_or(current.pane_borders);
    let show_label_in_border = file
        .client
        .show_label_in_border
        .unwrap_or(current.show_label_in_border);
    // Herdr-parity display options: default OFF/0 — absent keys preserve
    // today's edge-to-edge, gutter-free, drag-inert look exactly.
    let pane_gaps = file.client.pane_gaps.unwrap_or(current.pane_gaps);
    let scrollbar_gutter = file
        .client
        .scrollbar_gutter
        .unwrap_or(current.scrollbar_gutter);
    let copy_mode_auto_exit = file
        .client
        .copy_mode_auto_exit
        .unwrap_or(current.copy_mode_auto_exit);
    let change_cursor_shape = file
        .client
        .change_cursor_shape
        .unwrap_or(current.change_cursor_shape);
    let kitty_graphics = file.client.kitty_graphics.unwrap_or(current.kitty_graphics);
    let session_picker = file.client.session_picker.unwrap_or(current.session_picker);
    let prefer_default_workspace = file
        .client
        .prefer_default_workspace
        .unwrap_or(current.prefer_default_workspace);
    let git_status = file.client.git_status.unwrap_or(current.git_status);
    // The poll interval clamps instead of erroring: a bad number is the
    // file philosophy's warn-and-default, and 0 would spin the poll.
    let git_poll_secs = file
        .client
        .git_poll_secs
        .map(|v| if v == 0 { 0 } else { v.clamp(3, 600) })
        .unwrap_or(current.git_poll_secs);
    // Tab-width bounds ride as plain values with 0 = none (the file
    // philosophy); max wins over min at the layout site.
    let tab_width_min = file.client.tab_width_min.unwrap_or(current.tab_width_min);
    let tab_width_max = file.client.tab_width_max.unwrap_or(current.tab_width_max);
    // The border line style rides as its spelling; an unknown value is
    // the renderer's warn-and-default (the file philosophy), not a
    // reload error.
    let border_lines = file
        .client
        .border_lines
        .clone()
        .unwrap_or_else(|| current.border_lines.clone());
    // The roster's ordering word: an unknown spelling keeps the current
    // (the file philosophy's warn-and-default; the header toggle and the
    // settings row are the only writers, so divergence is not a user
    // hazard).
    let agents_sort = match file.client.agents_sort.as_deref() {
        Some("priority") => "priority".to_string(),
        Some("grouped") => "grouped".to_string(),
        _ => current.agents_sort.clone(),
    };
    // The strip settings follow the same warn-and-default rule: an
    // unknown spelling keeps the current.
    let session_strip = match file.client.session_strip.as_deref() {
        Some("auto") | Some("always") | Some("off") => file.client.session_strip.clone().unwrap(),
        _ => current.session_strip.clone(),
    };
    let agent_state_dots = match file.client.agent_state_dots.as_deref() {
        Some("on") | Some("off") => file.client.agent_state_dots.clone().unwrap(),
        _ => current.agent_state_dots.clone(),
    };
    let theme = client
        .theme
        .clone()
        .unwrap_or_else(|| current.theme.clone());
    let theme_dark = client
        .theme_dark
        .clone()
        .unwrap_or_else(|| current.theme_dark.clone());
    let theme_light = client
        .theme_light
        .clone()
        .unwrap_or_else(|| current.theme_light.clone());
    let popups = match client.popup.as_deref() {
        Some(entries) => crate::mux::popup::validate(entries)?,
        None => current.popups.clone(),
    };
    Ok(Chords {
        prefix,
        reload,
        management,
        resize_step,
        pane_borders,
        show_label_in_border,
        pane_gaps,
        scrollbar_gutter,
        copy_mode_auto_exit,
        change_cursor_shape,
        kitty_graphics,
        session_picker,
        prefer_default_workspace,
        git_status,
        git_poll_secs,
        border_lines,
        agents_sort,
        session_strip,
        agent_state_dots,
        sidebar_width,
        sidebar_on_launch,
        agent_rows,
        notify_delivery,
        notify_states,
        sound,
        theme,
        theme_dark,
        theme_light,
        popups,
        tab_width_min,
        tab_width_max,
    })
}

/// The daemon-side `reload-config` report, over the re-read file and the
/// applied copy: one `unchanged:`/`restart-required:`/`applied:` line per
/// `[daemon]` setting. The file speaks only for settings it states — a
/// setting absent from the file is `unchanged`, which is what keeps a
/// daemon started with one-shot `--socket`/`--state-dir` flags quiet (a
/// flag is an override the file cannot express, so it never reads back
/// as a change).
///
/// `daemon.remain-on-exit`, `daemon.exit-empty`, `daemon.split-start-dir`,
/// and every `daemon.agent-append.<kind>` are the LIVE settings: a stated
/// difference is applied to `applied` and reported `applied:`; every
/// other stated difference is `restart-required:` — the socket and the
/// persist path are fixed at bind, and the boot-tier bools (pane
/// endpoints, expose-control-socket) decide how the daemon was built.
pub fn reload_report(applied: &mut EffectiveConfig, file: Option<&ConfigFile>) -> String {
    let Some(file) = file else {
        // Absent file: nothing changed.
        let mut lines: Vec<String> = [
            "unchanged: daemon.socket",
            "unchanged: daemon.state-dir",
            "unchanged: daemon.pane-endpoints",
            "unchanged: daemon.expose-control-socket",
            "unchanged: daemon.remain-on-exit",
            "unchanged: daemon.exit-action",
            "unchanged: daemon.exit-empty",
            "unchanged: daemon.split-start-dir",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        for kind in AGENT_APPEND_KINDS {
            lines.push(format!("unchanged: daemon.agent-append.{kind}"));
        }
        return lines.join("\n");
    };
    let mut lines = Vec::new();
    for (name, stated, current) in [
        (
            "daemon.socket",
            file.daemon.socket.as_deref(),
            applied.socket.as_str(),
        ),
        (
            "daemon.state-dir",
            file.daemon.state_dir.as_deref(),
            applied.state_dir.as_str(),
        ),
    ] {
        lines.push(match stated {
            Some(v) if v != current => format!("restart-required: {name}"),
            _ => format!("unchanged: {name}"),
        });
    }
    // agent-resume-prompt feeds the boot restore only: a stated difference
    // is restart-required (it applies at the next daemon start's restore),
    // never applied in place.
    lines.push(match file.daemon.agent_resume_prompt.as_deref() {
        Some(v) if v != applied.agent_resume_prompt => {
            "restart-required: daemon.agent-resume-prompt".to_string()
        }
        _ => "unchanged: daemon.agent-resume-prompt".to_string(),
    });
    for (name, stated, current) in [
        (
            "daemon.pane-endpoints",
            file.daemon.pane_endpoints,
            applied.pane_endpoints,
        ),
        (
            "daemon.expose-control-socket",
            file.daemon.expose_control_socket,
            applied.expose_control_socket,
        ),
    ] {
        lines.push(match stated {
            Some(v) if v != current => format!("restart-required: {name}"),
            _ => format!("unchanged: {name}"),
        });
    }
    lines.push(match file.daemon.remain_on_exit {
        Some(v) if v != applied.remain_on_exit => {
            applied.remain_on_exit = v;
            "applied: daemon.remain-on-exit".to_string()
        }
        _ => "unchanged: daemon.remain-on-exit".to_string(),
    });
    // An explicit `exit-action` key wins; an unknown spelling resolves to
    // remove, the same as the initial parse. With the key absent the fold
    // derives from remain-on-exit — which the block above may have just
    // flipped, so the derived value is recomputed here rather than merged.
    let exit_action_line = match file.daemon.exit_action.as_deref() {
        Some(v) => {
            let parsed = ExitAction::parse(v).unwrap_or(ExitAction::Remove);
            if parsed != applied.exit_action {
                applied.exit_action = parsed;
                "applied: daemon.exit-action"
            } else {
                "unchanged: daemon.exit-action"
            }
        }
        None => {
            let folded = if applied.remain_on_exit {
                ExitAction::Hold
            } else {
                ExitAction::Remove
            };
            if folded != applied.exit_action {
                applied.exit_action = folded;
                "applied: daemon.exit-action"
            } else {
                "unchanged: daemon.exit-action"
            }
        }
    };
    lines.push(exit_action_line.to_string());
    lines.push(match file.daemon.exit_empty {
        Some(v) if v != applied.exit_empty => {
            applied.exit_empty = v;
            "applied: daemon.exit-empty".to_string()
        }
        _ => "unchanged: daemon.exit-empty".to_string(),
    });
    lines.push(match file.daemon.split_start_dir.as_deref() {
        Some(v) if v != applied.split_start_dir => {
            applied.split_start_dir = v.to_string();
            "applied: daemon.split-start-dir".to_string()
        }
        _ => "unchanged: daemon.split-start-dir".to_string(),
    });
    let stated_append = &file.daemon.agent_append;
    for (kind, stated, current) in [
        (
            "claude",
            stated_append.claude.as_deref(),
            &mut applied.agent_append.claude,
        ),
        (
            "codex",
            stated_append.codex.as_deref(),
            &mut applied.agent_append.codex,
        ),
        (
            "grok",
            stated_append.grok.as_deref(),
            &mut applied.agent_append.grok,
        ),
        (
            "muse",
            stated_append.muse.as_deref(),
            &mut applied.agent_append.muse,
        ),
        (
            "droid",
            stated_append.droid.as_deref(),
            &mut applied.agent_append.droid,
        ),
        (
            "copilot",
            stated_append.copilot.as_deref(),
            &mut applied.agent_append.copilot,
        ),
        (
            "cursor",
            stated_append.cursor.as_deref(),
            &mut applied.agent_append.cursor,
        ),
        (
            "kimi",
            stated_append.kimi.as_deref(),
            &mut applied.agent_append.kimi,
        ),
        (
            "qwen",
            stated_append.qwen.as_deref(),
            &mut applied.agent_append.qwen,
        ),
        (
            "qodercli",
            stated_append.qodercli.as_deref(),
            &mut applied.agent_append.qodercli,
        ),
        (
            "devin",
            stated_append.devin.as_deref(),
            &mut applied.agent_append.devin,
        ),
        (
            "gemini",
            stated_append.gemini.as_deref(),
            &mut applied.agent_append.gemini,
        ),
        (
            "amp",
            stated_append.amp.as_deref(),
            &mut applied.agent_append.amp,
        ),
        (
            "mastracode",
            stated_append.mastracode.as_deref(),
            &mut applied.agent_append.mastracode,
        ),
    ] {
        lines.push(match stated {
            Some(v) if v != current => {
                *current = v.to_string();
                format!("applied: daemon.agent-append.{kind}")
            }
            _ => format!("unchanged: daemon.agent-append.{kind}"),
        });
    }
    lines.join("\n")
}
