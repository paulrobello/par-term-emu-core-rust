- **Validate published version**: Before any changes, check the currently published version on PyPI (`pip index versions par-term-emu-core-rust 2>/dev/null | head -1` or `curl -s https://pypi.org/pypi/par-term-emu-core-rust/json | jq -r .info.version`) and compare against the version in Cargo.toml/pyproject.toml/__init__.py. If the local version matches the published version, the version MUST be bumped before deploying — otherwise the deploy will publish stale code under the existing version number.
- Ensure the python bindings and streaming server are up to date
- Bump version (all 5 files: root `Cargo.toml`, `pyproject.toml`, `python/par_term_emu_core_rust/__init__.py`, `crates/par-term-emu-core/Cargo.toml`, `crates/par-mux/Cargo.toml`) — the members version in lockstep with the root; `make core-version-check` fails if any pin drifts
- Update CHANGELOG.md, docs/ and README.md (note for downstream: `par_term_emu_core_rust::mux::…` imports move to the `par-mux` crate, and `cargo install par-term-emu-core-rust --bin par-mux` is replaced by `cargo install par-mux --features mux-bin,attach`)
- Run `make release-check` (needs `git fetch --tags` first); cite any reported uncited feat/fix commits in the top CHANGELOG section, then `python3 scripts/check_release_notes.py --write` to add the compare links
- Run `make pre-commit-run`
- Commit and push
- Run `make deploy` to trigger the release workflow — it now publishes in dependency order derive → `par-term-emu-core` → `par-mux` → root, each member step polling the crates.io index before the next publish. **Prerequisites**: the `CARGO_REGISTRY_TOKEN` secret needs the crates.io `publish-new` scope for the two new crate names, and this train has never run against crates.io — watch the first run end to end.

$ARGUMENTS
