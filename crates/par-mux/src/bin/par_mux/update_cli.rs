//! Manual and opt-in automatic updates through the shared release engine.

use super::UpdateArgs;
use par_term_emu_core::self_update::{self, Binary, Interval};
use std::path::Path;
use std::process::ExitCode;

pub(super) fn run_update_command(args: UpdateArgs, socket: std::path::PathBuf) -> ExitCode {
    let preview = matches!(args.channel, Some(par_mux::mux::update::Channel::Preview));
    report(apply(args.check, false, &socket, preview))
}

fn apply(check: bool, force: bool, socket: &Path, preview: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let outcome = self_update::update_channel(
        Binary::Mux,
        env!("CARGO_PKG_VERSION"),
        &exe,
        check,
        force,
        preview,
    )?;
    println!("{}", outcome.report);
    if outcome.installed {
        let args: Vec<_> = std::env::args_os().collect();
        let state_dir = args
            .windows(2)
            .find(|pair| pair[0] == "--state-dir")
            .map(|pair| std::path::PathBuf::from(&pair[1]))
            .or_else(|| {
                args.iter().find_map(|arg| {
                    arg.to_str()?
                        .strip_prefix("--state-dir=")
                        .map(std::path::PathBuf::from)
                })
            });
        self_update::restart_mux(&exe, socket, state_dir.as_deref())?;
    }
    Ok(())
}

pub(super) fn run_self_update(check: bool, force: bool, socket: &Path) -> ExitCode {
    report(apply(check, force, socket, false))
}

fn report(result: Result<(), String>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("par-mux: {err}");
            ExitCode::from(2)
        }
    }
}

pub(super) fn start_auto_update(socket: &Path) {
    let config = par_mux::mux::config::load_canonical();
    if !config.daemon.auto_update {
        return;
    }
    let interval: Interval = config
        .daemon
        .update_interval
        .as_deref()
        .unwrap_or("daily")
        .parse()
        .expect("validated update interval");
    let socket = socket.to_path_buf();
    if let Err(err) = std::thread::Builder::new()
        .name("mux-updater".into())
        .spawn(move || loop {
            match apply(false, false, &socket, false) {
                Ok(()) => {}
                Err(err) => log::warn!("par-mux auto-update: {err}"),
            }
            std::thread::sleep(std::time::Duration::from_secs(interval.seconds()));
        })
    {
        log::warn!("par-mux: cannot start updater: {err}");
    }
}
