//! The par-mux daemon.
//!
//! Owns PTYs and serves the control-mode protocol over a local socket. Runs
//! until killed: clients come and go, panes do not (see par-mux.md D5).

fn main() -> std::io::Result<()> {
    let name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "default".to_string());
    let path = par_term_emu_core_rust::mux::default_socket_path(&name);

    // bind refuses a path a live server already owns, so a racing auto-spawn
    // loses cleanly instead of stealing the socket.
    let server = par_term_emu_core_rust::mux::MuxServer::bind(&path)?;
    eprintln!("par-mux listening on {}", path.display());
    server.run();
    Ok(())
}
