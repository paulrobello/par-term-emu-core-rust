//! The par-mux daemon.
//!
//! Owns PTYs and serves the control-mode protocol over a local socket. Runs
//! until killed: clients come and go, panes do not (see par-mux.md D5).

fn main() -> std::io::Result<()> {
    // `par-mux <name>` binds that named default path; `par-mux --socket <p>`
    // binds an explicit path (what MuxClient::connect_or_spawn_at spawns).
    let mut args = std::env::args().skip(1);
    let path = match (args.next(), args.next()) {
        (Some(flag), Some(p)) if flag == "--socket" => std::path::PathBuf::from(p),
        (Some(name), _) => par_term_emu_core_rust::mux::default_socket_path(&name),
        _ => par_term_emu_core_rust::mux::default_socket_path("default"),
    };

    // D3.2/D3.3: a corrupt or unknown-version state file is quarantined
    // aside and the daemon starts fresh — unreadable state never blocks
    // startup. A readable state is REBUILT (D3.5): layout and content are
    // restored, and every pane's process is new — the original processes
    // died with the previous server, which is stated rather than papered
    // over.
    let state_path = par_term_emu_core_rust::mux::persist::state_file_path(&path);
    let restored = match par_term_emu_core_rust::mux::persist::load_or_quarantine(&state_path) {
        par_term_emu_core_rust::mux::persist::Loaded::Fresh => None,
        par_term_emu_core_rust::mux::persist::Loaded::State(state) => {
            match par_term_emu_core_rust::mux::tree::MuxTree::from_persist_state(
                &state,
                Box::new(par_term_emu_core_rust::mux::pane::ShellPaneFactory::default()),
            ) {
                Ok(tree) => Some(tree),
                Err(err) => {
                    eprintln!("par-mux: state restore failed ({err}); starting fresh");
                    None
                }
            }
        }
        par_term_emu_core_rust::mux::persist::Loaded::Quarantined { .. } => None,
    };

    // bind refuses a path a live server already owns, so a racing auto-spawn
    // loses cleanly instead of stealing the socket.
    let server = match restored {
        Some(tree) => par_term_emu_core_rust::mux::MuxServer::bind_with_tree(&path, tree)?,
        None => par_term_emu_core_rust::mux::MuxServer::bind(&path)?,
    };
    eprintln!("par-mux listening on {}", path.display());
    server.run_persisting(state_path);
    Ok(())
}
