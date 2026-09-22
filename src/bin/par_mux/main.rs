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

    // bind refuses a path a live server already owns, so a racing auto-spawn
    // loses cleanly instead of stealing the socket.
    let server = par_term_emu_core_rust::mux::MuxServer::bind(&path)?;

    // D3.2/D3.3: a corrupt or unknown-version state file is quarantined
    // aside and the daemon starts fresh — unreadable state never blocks
    // startup. A readable state is logged for now; rebuilding from it ships
    // with Task 3.4.
    let state_path = par_term_emu_core_rust::mux::persist::state_file_path(&path);
    match par_term_emu_core_rust::mux::persist::load_or_quarantine(&state_path) {
        par_term_emu_core_rust::mux::persist::Loaded::Fresh => {}
        par_term_emu_core_rust::mux::persist::Loaded::State(_) => eprintln!(
            "par-mux: found existing state at {} (automatic rebuild ships with Task 3.4)",
            state_path.display()
        ),
        par_term_emu_core_rust::mux::persist::Loaded::Quarantined { .. } => {}
    }

    eprintln!("par-mux listening on {}", path.display());
    server.run_persisting(state_path);
    Ok(())
}
