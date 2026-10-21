//! The `update` subcommand handler: resolve, plan, execute — the library
//! side (`mux::update`) owns everything testable.

use std::process::ExitCode;

use par_mux::mux::update::{self, Channel, RealIo, UpdateIo as _};

use super::UpdateArgs;

pub(super) fn run_update_command(args: UpdateArgs, socket: std::path::PathBuf) -> ExitCode {
    match run_update(args, socket) {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("par-mux: {err}");
            ExitCode::from(2)
        }
    }
}

fn run_update(args: UpdateArgs, socket: std::path::PathBuf) -> Result<String, String> {
    let channel = args.channel.unwrap_or(Channel::Stable);
    let current_version = par_mux::mux::build_stamp()
        .split('+')
        .next()
        .unwrap_or("0")
        .to_string();
    let io = RealIo;
    let releases_url = format!(
        "https://api.github.com/repos/{}/releases",
        update::RELEASE_REPO
    );
    let listing =
        std::env::temp_dir().join(format!("par-mux-releases-{}.json", std::process::id()));
    io.fetch(&releases_url, &listing)?;
    let text =
        std::fs::read_to_string(&listing).map_err(|err| format!("{}: {err}", listing.display()))?;
    let _ = std::fs::remove_file(&listing);
    let releases: serde_json::Value = serde_json::from_str(&text)
        .map_err(|err| format!("the release listing is not JSON: {err}"))?;
    let plan = update::plan(&releases, channel, &current_version)?;
    let self_exe = std::env::current_exe().map_err(|err| format!("current exe: {err}"))?;
    update::execute(&plan, &io, &self_exe, &socket, args.check)
}
