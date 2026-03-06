- **Validate published version**: Before any changes, check the currently published version on PyPI (`pip index versions par-term-emu-core-rust 2>/dev/null | head -1` or `curl -s https://pypi.org/pypi/par-term-emu-core-rust/json | jq -r .info.version`) and compare against the version in Cargo.toml/pyproject.toml/__init__.py. If the local version matches the published version, the version MUST be bumped before deploying — otherwise the deploy will publish stale code under the existing version number.
- Ensure the python bindings and streaming server are up to date
- Bump version (all 3 files: Cargo.toml, pyproject.toml, python/par_term_emu_core_rust/__init__.py)
- Update CHANGELOG.md, docs/ and README.md
- Run `make pre-commit-run`
- Commit and push
- Run `make deploy` to trigger cicd workflow

$ARGUMENTS
