//! The `plugin` subcommand: install (with the trust preview), list,
//! update, remove. Never contacts a daemon — the daemon loads the
//! plugins directory at startup and `reload-config` re-reads it.

use std::process::ExitCode;

use par_mux::mux::plugins::{
    parse_manifest, read_installations, write_installations, InstallationRecord, MANIFEST_NAME,
};

use super::{PluginArgs, PluginCommand};

pub(super) fn run_plugin_command(args: PluginArgs) -> ExitCode {
    match run_plugin(args.command) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("par-mux: {err}");
            ExitCode::from(2)
        }
    }
}

/// The plugins directory this CLI's `--state-dir` (else the platform
/// default) resolves — the same base the daemon reads.
fn plugins_dir() -> Result<std::path::PathBuf, String> {
    let mut args = std::env::args().peekable();
    let base = loop {
        match args.next() {
            Some(arg) if arg == "--state-dir" => match args.next() {
                Some(dir) => break std::path::PathBuf::from(dir),
                None => return Err("--state-dir needs its value: --state-dir <DIR>".into()),
            },
            Some(arg) => {
                if let Some(dir) = arg.strip_prefix("--state-dir=") {
                    break std::path::PathBuf::from(dir);
                }
            }
            None => break par_mux::mux::persist::platform_state_dir(),
        }
    };
    Ok(par_mux::mux::plugins::plugins_dir(&base.join("par-mux")))
}

fn run_plugin(command: PluginCommand) -> Result<String, String> {
    let dir = plugins_dir()?;
    match command {
        PluginCommand::Install { source, trust } => install(&dir, &source, trust),
        PluginCommand::List => list(&dir),
        PluginCommand::Remove { name } => remove(&dir, &name),
        PluginCommand::Update { name, trust } => update(&dir, &name, trust),
    }
}

/// Resolve a source to a plugin directory with a manifest: a local
/// directory as-is, a git URL through a depth-1 clone into a temp dir.
fn fetch_source(source: &str) -> Result<(tempfile::TempDir, std::path::PathBuf), String> {
    if source.starts_with("http://") || source.starts_with("https://") || source.ends_with(".git") {
        let temp = tempfile::tempdir().map_err(|err| format!("temp dir: {err}"))?;
        let status = std::process::Command::new("git")
            .arg("clone")
            .arg("--depth")
            .arg("1")
            .arg("--quiet")
            .arg(source)
            .arg(temp.path())
            .status()
            .map_err(|err| format!("git clone {source}: {err}"))?;
        if !status.success() {
            return Err(format!("git clone {source} failed: {status}"));
        }
        // The manifest sits at the clone root or one level down (a repo
        // named for its plugin directory).
        let root = temp.path().to_path_buf();
        if root.join(MANIFEST_NAME).is_file() {
            return Ok((temp, root));
        }
        let mut found = None;
        for entry in std::fs::read_dir(&root)
            .map_err(|err| err.to_string())?
            .flatten()
        {
            if entry.path().join(MANIFEST_NAME).is_file() {
                found = Some(entry.path());
                break;
            }
        }
        found
            .map(|path| (temp, path))
            .ok_or_else(|| format!("{source} carries no {MANIFEST_NAME}"))
    } else {
        let path = std::path::PathBuf::from(source);
        if !path.join(MANIFEST_NAME).is_file() {
            return Err(format!("{} carries no {MANIFEST_NAME}", path.display()));
        }
        Ok((tempfile::tempdir().map_err(|err| err.to_string())?, path))
    }
}

/// The trust preview: source, identity, and every command an event runs —
/// everything the user is about to trust, nothing else.
fn preview(source: &str, manifest: &par_mux::mux::plugins::PluginManifest) -> String {
    let mut out = format!(
        "plugin {} v{}\n  source: {source}\n",
        manifest.name, manifest.version
    );
    if !manifest.description.is_empty() {
        out.push_str(&format!("  {}\n", manifest.description));
    }
    for hook in &manifest.hooks {
        out.push_str(&format!(
            "  on {} ({} ms): {}\n",
            hook.event,
            hook.effective_timeout_ms(),
            hook.command
        ));
    }
    out
}

fn install(dir: &std::path::Path, source: &str, trust: bool) -> Result<String, String> {
    let (_keep, plugin_dir) = fetch_source(source)?;
    let text = std::fs::read_to_string(plugin_dir.join(MANIFEST_NAME))
        .map_err(|err| format!("{}: {err}", plugin_dir.join(MANIFEST_NAME).display()))?;
    let manifest = parse_manifest(&text)?;
    let mut out = preview(source, &manifest);
    if !trust {
        out.push_str(&format!(
            "preview only — re-run with --trust to install into {}\n",
            dir.display()
        ));
        return Ok(out);
    }
    let target = dir.join(&manifest.name);
    if target.exists() {
        return Err(format!(
            "plugin {} is already installed (update or remove it first)",
            manifest.name
        ));
    }
    copy_tree(&plugin_dir, &target)?;
    let mut records = read_installations(dir)?;
    records.retain(|record| record.name != manifest.name);
    records.push(InstallationRecord {
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        source: source.to_string(),
        installed_unix_ms: now_unix_ms(),
    });
    write_installations(dir, &records)?;
    out.push_str(&format!(
        "installed into {} (send reload-config to a running daemon, or restart it)\n",
        target.display()
    ));
    Ok(out)
}

fn list(dir: &std::path::Path) -> Result<String, String> {
    let records = read_installations(dir)?;
    if records.is_empty() {
        return Ok(format!("no plugins installed under {}\n", dir.display()));
    }
    let mut out = String::new();
    for record in records {
        let manifest = std::fs::read_to_string(dir.join(&record.name).join(MANIFEST_NAME))
            .ok()
            .and_then(|text| parse_manifest(&text).ok());
        match manifest {
            Some(manifest) => out.push_str(&format!(
                "{}  v{}  {} hooks  from {}\n",
                record.name,
                record.version,
                manifest.hooks.len(),
                record.source
            )),
            None => out.push_str(&format!(
                "{}  v{}  (manifest unreadable)  from {}\n",
                record.name, record.version, record.source
            )),
        }
    }
    Ok(out)
}

fn remove(dir: &std::path::Path, name: &str) -> Result<String, String> {
    let mut records = read_installations(dir)?;
    let before = records.len();
    records.retain(|record| record.name != name);
    if records.len() == before {
        return Err(format!("no installed plugin {name:?}"));
    }
    write_installations(dir, &records)?;
    let target = dir.join(name);
    if target.exists() {
        std::fs::remove_dir_all(&target).map_err(|err| format!("{}: {err}", target.display()))?;
    }
    Ok(format!(
        "plugin {name} removed (send reload-config to a running daemon, or restart it)\n"
    ))
}

fn update(dir: &std::path::Path, name: &str, trust: bool) -> Result<String, String> {
    let records = read_installations(dir)?;
    let record = records
        .iter()
        .find(|record| record.name == name)
        .ok_or_else(|| format!("no installed plugin {name:?}"))?;
    let source = record.source.clone();
    drop(records);
    remove(dir, name)?;
    install(dir, &source, trust)
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|err| format!("{}: {err}", to.display()))?;
    for entry in std::fs::read_dir(from)
        .map_err(|err| format!("{}: {err}", from.display()))?
        .flatten()
    {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)
                .map_err(|err| format!("{}: {err}", target.display()))?;
        }
    }
    Ok(())
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
