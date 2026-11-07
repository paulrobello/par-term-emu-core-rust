//! The par-mux config file (`<config dir>/par-mux/config.toml`).
//!
//! One file carries client defaults (the attach prefix, the default attach
//! mode, the reload chord) and daemon defaults (socket, state dir, the
//! pane-endpoint switches). Resolution precedence everywhere is
//! **flags > env > file > built-in defaults**: a CLI flag wins, then the
//! relevant environment variable (`PAR_MUX_SOCKET` for the socket target —
//! the env var every pane spawns with), then the file, then the built-in.
//!
//! Reload semantics per setting (see docs/MUX.md "Configuration"):
//! - `client.prefix`, `client.reload`: LIVE — a running attach rebinds
//!   its chords immediately.
//! - everything else: startup-only. `daemon.state-dir` in particular is
//!   resolved once at daemon startup (`run_daemon` in the binary) and the
//!   persist worker writes to that fixed path, so a re-read can only
//!   report it restart-required.
//!
//! The reload entry points: the `reload-config` control command (daemon
//! side: re-read, report per-setting), the attach client's reload chord
//! (default `C-b C-r`, itself configurable as `[client] reload` — it
//! rebinds the client chords live and sends `reload-config` so the
//! daemon-side report rides the same keypress), and `--gen-config`.

mod chords;
mod client;
mod daemon;
mod file;
mod management;
mod parse;
mod reload;
mod resolve;

#[cfg(test)]
mod tests;

pub use chords::{
    default_agent_rows, display_option_warnings, reload_chord_key, Chords, SoundSettings,
    AGENT_ROW_TOKENS, NOTIFY_DELIVERIES, NOTIFY_STATES,
};
pub use client::ClientSection;
pub use daemon::{
    daemon_agent_append, daemon_agent_resume_prompt, daemon_remain_on_exit, AgentAppend,
    AgentAppendFile, DaemonSection, ExitAction, RestorePolicy, AGENT_APPEND_KINDS,
};
pub use file::{
    config_file_path, load_canonical, load_canonical_checked, load_file, render,
    socket_value_to_path, write_file,
};
pub use management::{management_key, Management};
pub use parse::{parse_hex_color, parse_prefix};
pub use reload::{reload_client_chords, reload_report};
pub use resolve::{resolve, EffectiveConfig, Overrides, DEFAULT_AGENT_RESUME_PROMPT};

use serde::{Deserialize, Serialize};

/// The config file's on-disk shape. Every field is optional: a missing
/// file, an empty file, and a file with only some keys all resolve against
/// the later tiers. Unknown keys are ignored (forward compatibility).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ConfigFile {
    /// Attach-client defaults.
    #[serde(default)]
    pub client: ClientSection,
    /// Daemon defaults.
    #[serde(default)]
    pub daemon: DaemonSection,
    /// Saved remote hosts (`[machines.<label>]`): SSH thin-client profiles.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub machines: std::collections::BTreeMap<String, MachineProfile>,
}

/// `[machines.<label>]`: one saved remote par-mux host. The profile stores
/// no credentials — ssh (the user's own config, agent, and askpass) owns
/// authentication, exactly as herdr's machines do.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct MachineProfile {
    /// The ssh target as openssh spells it (`user@host`, `host`, an alias
    /// the user's ssh config resolves; a `user@host:port` colon form is
    /// kept verbatim for ssh's `-p`-less spelling).
    pub target: String,
    /// The remote session this machine's attach lands on by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// Disabled machines are listed but never attached without naming
    /// them explicitly.
    #[serde(
        default = "machine_enabled_by_default",
        skip_serializing_if = "machine_enabled"
    )]
    pub enabled: bool,
}

fn machine_enabled_by_default() -> bool {
    true
}

fn machine_enabled(enabled: &bool) -> bool {
    *enabled
}
