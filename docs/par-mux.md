# par-mux.md — Design Document Pointer

Code comments and design discussions in this crate cite `par-mux.md` for the
D-numbered design decisions (`D1`–`D5`, `D3.3`, Phase 4 `T4.B`/`T4.C`/`T4.E`,
the Phase 5 rulings, …) behind the mux daemon. That document is the full
task-level design and plan for the multiplexer, and it is maintained in the
`par-agent-os` repository, not vendored here (a copy would drift):

- **Repository:** `par-agent-os`
- **File:** `par-mux.md` at the repository root
- **Local path (this machine):** `~/Repos/par-agent-os/par-mux.md`

The operational reference for the daemon as shipped — build, CLI, socket and
state paths, the command table, hook reports, scrape tier, persistence, and
shutdown — lives in [`docs/MUX.md`](MUX.md).

`docs/par-mux.md` exists so the `see par-mux.md` citations in `src/`,
`tests/`, and `Cargo.toml` resolve to something that explains where the
authoritative design record is.
