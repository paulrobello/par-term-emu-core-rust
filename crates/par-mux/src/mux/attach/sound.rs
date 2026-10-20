//! Agent state-change sounds (herdr's `[ui.sound]`): a `blocked` sound
//! when an agent needs input and a `done` sound when one finishes out of
//! view, each a built-in OS sound or a custom file, with per-agent mute.
//! Playback hands the file to a player command (argv only, no shell,
//! detached and reaped) — no audio dependency. When no player or sound
//! file is available the terminal bell rings instead.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::mux::config::SoundSettings;

/// Which sound an event plays (herdr's `Sound::Request` / `Sound::Done`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sound {
    /// An agent needs input (arrived in `blocked`).
    Blocked,
    /// An agent finished out of view (`done`, or `idle` straight from
    /// `working` / `blocked`).
    Done,
}

/// The sound a pushed agent transition plays, herdr's
/// `notification_sound_for_state_change`: `blocked` sounds even in view
/// (input is needed wherever the agent is), a finish sounds only for an
/// agent outside the shown window. A repeat of the same state and the
/// daemon's seen flip (`done` → `idle`) never sound.
pub(crate) fn sound_for_transition(
    previous: Option<&str>,
    state: &str,
    in_view: bool,
) -> Option<Sound> {
    if previous == Some(state) {
        return None;
    }
    match state {
        "blocked" => Some(Sound::Blocked),
        "done" if !in_view => Some(Sound::Done),
        "idle" if !in_view && matches!(previous, Some("working" | "blocked")) => Some(Sound::Done),
        _ => None,
    }
}

/// Whether `agent` may sound: sounds are on and the label is not in
/// `sound-mute` (matched case-insensitively).
pub(crate) fn allows(settings: &SoundSettings, agent: &str) -> bool {
    settings.enabled
        && !settings
            .mute
            .iter()
            .any(|muted| muted.trim().eq_ignore_ascii_case(agent.trim()))
}

/// How one sound is delivered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Playback {
    /// Spawn this argv (the player, its arguments, then the file).
    Spawn(Vec<OsString>),
    /// Ring the terminal bell (no player or no sound file).
    Bell,
}

/// The OS default player command (empty `sound-player`).
fn default_player() -> &'static str {
    if cfg!(target_os = "macos") {
        "afplay"
    } else if cfg!(windows) {
        ""
    } else {
        "paplay"
    }
}

/// The built-in sound file for `sound` on this OS: the stock macOS
/// system sounds, the freedesktop sound theme elsewhere on Unix, none on
/// Windows.
pub(crate) fn builtin_file(sound: Sound) -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        Some(match sound {
            Sound::Done => "/System/Library/Sounds/Glass.aiff",
            Sound::Blocked => "/System/Library/Sounds/Ping.aiff",
        })
    } else if cfg!(windows) {
        None
    } else {
        Some(match sound {
            Sound::Done => "/usr/share/sounds/freedesktop/stereo/complete.oga",
            Sound::Blocked => "/usr/share/sounds/freedesktop/stereo/bell.oga",
        })
    }
}

/// `spec` resolved against the config file's directory when relative
/// (herdr's `resolve_config_relative_path`); a leading `~/` is the home
/// directory.
fn resolve_path(spec: &str, config_dir: Option<&Path>) -> PathBuf {
    if let Some(rest) = spec.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    let path = PathBuf::from(spec);
    match config_dir {
        Some(dir) if path.is_relative() => dir.join(path),
        _ => path,
    }
}

/// How `sound` plays under `settings`: the custom file when one is set
/// and exists, else the built-in when it exists, handed to the
/// configured player (or the OS default); the bell when no file or no
/// player is left. `exists` is the filesystem probe (a parameter so the
/// plan is testable without real files).
pub(crate) fn plan(
    sound: Sound,
    settings: &SoundSettings,
    config_dir: Option<&Path>,
    exists: &dyn Fn(&Path) -> bool,
) -> Playback {
    let custom = match sound {
        Sound::Done => settings.done.trim(),
        Sound::Blocked => settings.blocked.trim(),
    };
    let file = (!custom.is_empty())
        .then(|| resolve_path(custom, config_dir))
        .filter(|path| exists(path))
        .or_else(|| {
            builtin_file(sound)
                .map(PathBuf::from)
                .filter(|path| exists(path))
        });
    let player = match settings.player.trim() {
        "" => default_player(),
        spec => spec,
    };
    let mut argv: Vec<OsString> = player.split_whitespace().map(OsString::from).collect();
    match file {
        Some(file) if !argv.is_empty() => {
            argv.push(file.into_os_string());
            Playback::Spawn(argv)
        }
        _ => Playback::Bell,
    }
}

#[cfg(test)]
thread_local! {
    /// Every argv a test's playback would have spawned.
    pub(crate) static SPAWNED: std::cell::RefCell<Vec<Vec<OsString>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Run the player detached, a reaper thread waiting on it so no zombie
/// stays behind. Returns whether it started.
#[cfg(not(test))]
fn spawn_player(argv: &[OsString]) -> bool {
    let Some((program, args)) = argv.split_first() else {
        return false;
    };
    let child = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match child {
        Ok(mut child) => {
            let _ = std::thread::Builder::new()
                .name("par-mux-sound".to_string())
                .spawn(move || {
                    let _ = child.wait();
                });
            true
        }
        Err(_) => false,
    }
}

/// Tests never play audio: record the argv instead.
#[cfg(test)]
fn spawn_player(argv: &[OsString]) -> bool {
    SPAWNED.with(|spawned| spawned.borrow_mut().push(argv.to_vec()));
    true
}

/// Play `sound`; returns whether the terminal bell should ring instead
/// (no file, no player, or the player failed to start).
pub(crate) fn play(sound: Sound, settings: &SoundSettings, config_dir: Option<&Path>) -> bool {
    match plan(sound, settings, config_dir, &|path| path.is_file()) {
        Playback::Spawn(argv) => !spawn_player(&argv),
        Playback::Bell => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> SoundSettings {
        SoundSettings {
            enabled: true,
            ..SoundSettings::default()
        }
    }

    /// Herdr's event mapping: blocked sounds even in view, a finish only
    /// out of view, repeats and the seen flip never.
    #[test]
    fn transitions_map_to_herdr_sounds() {
        use Sound::*;
        assert_eq!(
            sound_for_transition(Some("working"), "blocked", true),
            Some(Blocked)
        );
        assert_eq!(sound_for_transition(None, "blocked", false), Some(Blocked));
        assert_eq!(
            sound_for_transition(Some("blocked"), "blocked", false),
            None
        );
        assert_eq!(
            sound_for_transition(Some("working"), "done", false),
            Some(Done)
        );
        assert_eq!(sound_for_transition(Some("working"), "done", true), None);
        assert_eq!(
            sound_for_transition(Some("blocked"), "idle", false),
            Some(Done)
        );
        assert_eq!(
            sound_for_transition(Some("working"), "idle", false),
            Some(Done)
        );
        assert_eq!(sound_for_transition(Some("working"), "idle", true), None);
        assert_eq!(sound_for_transition(Some("done"), "idle", false), None);
        assert_eq!(sound_for_transition(None, "idle", false), None);
        assert_eq!(sound_for_transition(Some("idle"), "working", false), None);
    }

    /// Off by default; a muted label never sounds, matched without case.
    #[test]
    fn per_agent_mute_filters_sounds() {
        assert!(!allows(&SoundSettings::default(), "claude"));
        let mut settings = on();
        assert!(allows(&settings, "claude"));
        settings.mute = vec!["Droid".to_string(), " codex ".to_string()];
        assert!(!allows(&settings, "droid"));
        assert!(!allows(&settings, "codex"));
        assert!(allows(&settings, "claude"));
    }

    /// A custom file wins when it exists (relative to the config dir),
    /// falls back to the built-in when missing, and the configured player
    /// splits into argv with the file last; no file or no player rings
    /// the bell.
    #[test]
    fn plan_prefers_custom_then_builtin_then_bell() {
        let dir = Path::new("/cfg");
        let mut settings = on();
        settings.player = "mpv --no-video".to_string();
        settings.done = "sounds/done.mp3".to_string();
        settings.blocked = "/abs/blocked.wav".to_string();
        let all = |_: &Path| true;
        assert_eq!(
            plan(Sound::Done, &settings, Some(dir), &all),
            Playback::Spawn(vec![
                "mpv".into(),
                "--no-video".into(),
                dir.join("sounds/done.mp3").into_os_string()
            ])
        );
        assert_eq!(
            plan(Sound::Blocked, &settings, Some(dir), &all),
            Playback::Spawn(vec![
                "mpv".into(),
                "--no-video".into(),
                "/abs/blocked.wav".into()
            ])
        );
        let builtin_only =
            |path: &Path| builtin_file(Sound::Done).is_some_and(|b| path == Path::new(b));
        let expected = match builtin_file(Sound::Done) {
            Some(builtin) => {
                Playback::Spawn(vec!["mpv".into(), "--no-video".into(), builtin.into()])
            }
            None => Playback::Bell,
        };
        assert_eq!(
            plan(Sound::Done, &settings, Some(dir), &builtin_only),
            expected,
            "a missing custom file falls back to the built-in"
        );
        let none = |_: &Path| false;
        assert_eq!(
            plan(Sound::Done, &settings, Some(dir), &none),
            Playback::Bell
        );
        // The OS default player: a built-in file plays through it, or the
        // bell rings where the OS has neither (Windows).
        let default = on();
        match (builtin_file(Sound::Blocked), default_player()) {
            (Some(builtin), player) if !player.is_empty() => assert_eq!(
                plan(Sound::Blocked, &default, None, &all),
                Playback::Spawn(vec![player.into(), builtin.into()])
            ),
            _ => assert_eq!(plan(Sound::Blocked, &default, None, &all), Playback::Bell),
        }
    }

    /// `play` spawns through the test intercept, never real audio.
    #[test]
    fn play_records_the_argv_in_tests() {
        SPAWNED.with(|s| s.borrow_mut().clear());
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("done.wav");
        std::fs::write(&file, b"RIFF").unwrap();
        let mut settings = on();
        settings.player = "true".to_string();
        settings.done = "done.wav".to_string();
        assert!(!play(Sound::Done, &settings, Some(dir.path())));
        let spawned = SPAWNED.with(|s| s.borrow().clone());
        assert_eq!(
            spawned,
            vec![vec![OsString::from("true"), file.into_os_string()]]
        );
    }
}
