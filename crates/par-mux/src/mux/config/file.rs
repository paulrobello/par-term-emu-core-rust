//! The config file's location, loading, and `--gen-config` rendering.

use std::path::{Path, PathBuf};

use super::{
    spell_chord, AgentAppendFile, Chords, ClientSection, ConfigFile, DaemonSection, EffectiveConfig,
};

/// Environment override for the config file location (`PAR_MUX_CONFIG`),
/// else `<config dir>/par-mux/config.toml` via `dirs::config_dir()`.
/// `None` when the platform has no config dir and no override is set.
pub fn config_file_path() -> Option<PathBuf> {
    // `var_os` reads are allowed (QA-196 targets test WRITES); this is
    // the feature's one production env read.
    if let Some(path) = std::env::var_os("PAR_MUX_CONFIG").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(path));
    }
    dirs::config_dir().map(|dir| dir.join("par-mux").join("config.toml"))
}

/// Read and parse the config file at `path`. `Ok(None)` = no file
/// (defaults apply); `Err` = a file exists but does not parse — callers
/// surface that rather than silently ignoring the user's file.
pub fn load_file(path: &Path) -> Result<Option<ConfigFile>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("{}: {err}", path.display())),
    };
    toml::from_str(&text)
        .map(Some)
        .map_err(|err| format!("{}: TOML parse failed: {err}", path.display()))
}

/// Load the canonical config file (or `Ok(None)` when absent/mislocated),
/// logging a warning and falling back to defaults when it exists but does
/// not parse — unreadable config must not brick daemon startup, the same
/// rule the state file follows (D3.2).
pub fn load_canonical() -> ConfigFile {
    let Some(path) = config_file_path() else {
        return ConfigFile::default();
    };
    match load_file(&path) {
        Ok(file) => file.unwrap_or_default(),
        Err(err) => {
            log::warn!("par-mux: ignoring config: {err}");
            ConfigFile::default()
        }
    }
}

/// A reload's honest variant: absent stays `Ok(None)`-shaped (defaults /
/// current values apply), but a PRESENT file that does not parse is an
/// `Err` — a user editing their config mid-attach must see the mistake
/// on the status row, not have it silently ignored.
pub fn load_canonical_checked() -> Result<ConfigFile, String> {
    let Some(path) = config_file_path() else {
        return Ok(ConfigFile::default());
    };
    Ok(load_file(&path)?.unwrap_or_default())
}

/// Map a resolved `[daemon] socket` value to a socket path: an absolute
/// path stands as-is; anything else (including `default` and empty) names
/// a default socket path.
#[must_use]
pub fn socket_value_to_path(value: &str) -> PathBuf {
    if value.is_empty() {
        return crate::mux::default_socket_path("default");
    }
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        crate::mux::default_socket_path(value)
    }
}

/// Render `eff` as config-file TOML: every setting spelled, with a short
/// header. Round-trips through [`load_file`].
#[must_use]
pub fn render(eff: &EffectiveConfig) -> String {
    // The display defaults come from the one canonical chord set, so the
    // generated file cannot drift from what the client applies (ARC-131).
    let defaults = Chords::with_defaults();
    let file = ConfigFile {
        client: ClientSection {
            prefix: Some(eff.prefix.clone()),
            mode: Some(eff.mode.clone()),
            reload: Some(eff.reload.clone()),
            git_poll_secs: Some(eff.git_poll_secs),
            agents_sort: Some(defaults.agents_sort.clone()),
            session_strip: Some(defaults.session_strip.clone()),
            agent_state_dots: Some(defaults.agent_state_dots.clone()),
            split_right: None,
            layout_even_horizontal: None,
            layout_even_vertical: None,
            layout_main_horizontal: None,
            layout_main_vertical: None,
            layout_tiled: None,
            split_down: None,
            kill_pane: None,
            break_pane: None,
            join_pane: None,
            palette: None,
            new_window: None,
            resize: None,
            resize_step: None,
            swap_prev: None,
            move_window_left: None,
            move_window_right: None,
            tab_strip_left: None,
            tab_strip_right: None,
            swap_next: None,
            workspace_next: None,
            workspace_prev: None,
            help: None,
            picker: Some(spell_chord(&defaults.management.picker)),
            zoom: None,
            rename_window: None,
            rename_pane: None,
            border_cycle: None,
            label_toggle: None,
            workspace_picker: None,
            sidebar: None,
            status_bar: None,
            respawn: None,
            notify_jump: None,
            sidebar_width: Some(defaults.sidebar_width),
            sidebar_on_launch: Some(defaults.sidebar_on_launch),
            agent_rows: Some(defaults.agent_rows.clone()),
            notify_delivery: Some(defaults.notify_delivery.clone()),
            notify_states: Some(defaults.notify_states.clone()),
            sound: Some(defaults.sound.enabled),
            sound_player: Some(defaults.sound.player.clone()),
            sound_done: Some(defaults.sound.done.clone()),
            sound_blocked: Some(defaults.sound.blocked.clone()),
            sound_mute: Some(defaults.sound.mute.clone()),
            border_lines: Some(defaults.border_lines.clone()),
            pane_borders: None,
            show_label_in_border: Some(defaults.show_label_in_border),
            pane_gaps: Some(defaults.pane_gaps),
            tab_width_min: Some(defaults.tab_width_min),
            tab_width_max: Some(defaults.tab_width_max),
            scrollbar_gutter: Some(defaults.scrollbar_gutter),
            copy_mode_auto_exit: Some(defaults.copy_mode_auto_exit),
            change_cursor_shape: Some(defaults.change_cursor_shape),
            kitty_graphics: Some(defaults.kitty_graphics),
            session_picker: Some(defaults.session_picker),
            prefer_default_workspace: Some(defaults.prefer_default_workspace),
            git_status: Some(defaults.git_status),
            border_active_color: Some(eff.border_active_color.clone()),
            border_color: Some(eff.border_color.clone()),
            theme: Some(defaults.theme.clone()),
            theme_dark: Some(defaults.theme_dark.clone()),
            theme_light: Some(defaults.theme_light.clone()),
            popup: None,
        },
        daemon: DaemonSection {
            auto_update: eff.auto_update,
            update_interval: Some(eff.update_interval.clone()),
            socket: Some(eff.socket.clone()),
            state_dir: Some(eff.state_dir.clone()),
            pane_endpoints: Some(eff.pane_endpoints),
            expose_control_socket: Some(eff.expose_control_socket),
            remain_on_exit: Some(eff.remain_on_exit),
            exit_action: Some(eff.exit_action.name().to_string()),
            exit_empty: Some(eff.exit_empty),
            split_start_dir: Some(eff.split_start_dir.clone()),
            agent_resume_prompt: Some(eff.agent_resume_prompt.clone()),
            agent_append: AgentAppendFile {
                claude: Some(eff.agent_append.claude.clone()),
                codex: Some(eff.agent_append.codex.clone()),
                grok: Some(eff.agent_append.grok.clone()),
                muse: Some(eff.agent_append.muse.clone()),
                droid: Some(eff.agent_append.droid.clone()),
                copilot: Some(eff.agent_append.copilot.clone()),
                cursor: Some(eff.agent_append.cursor.clone()),
                kimi: Some(eff.agent_append.kimi.clone()),
                qwen: Some(eff.agent_append.qwen.clone()),
                qodercli: Some(eff.agent_append.qodercli.clone()),
                devin: Some(eff.agent_append.devin.clone()),
                gemini: Some(eff.agent_append.gemini.clone()),
                amp: Some(eff.agent_append.amp.clone()),
                mastracode: Some(eff.agent_append.mastracode.clone()),
            },
        },
        machines: Default::default(),
    };
    let mut out = String::from(
        "# par-mux configuration.\n\
         # Precedence: CLI flags > environment > this file > built-in defaults.\n\
         # Regenerate with: par-mux --gen-config [--force]\n",
    );
    let body = toml::to_string_pretty(&file).unwrap_or_default();
    out.push_str(&body);
    out
}

/// Write `eff` as the config file at `path`, refusing to overwrite an
/// existing file unless `force`. Returns an error message on refusal or
/// I/O failure.
pub fn write_file(path: &Path, eff: &EffectiveConfig, force: bool) -> Result<(), String> {
    if path.exists() && !force {
        return Err(format!(
            "{} already exists; pass --force to overwrite",
            path.display()
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
    }
    std::fs::write(path, render(eff)).map_err(|err| format!("{}: {err}", path.display()))
}
