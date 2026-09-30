# ENH-040: Dependency-audit gate — `cargo deny`, `bun audit`, `pip-audit` as `make audit-deps` plus a scheduled workflow

> Filed from the 2026-09-29 /opus-audit enhancement pass (cycle `audit-2026-09-29`). Board card: `[ENH-040]`.
> Sequencing: independent. Complements SEC-134 (track `paste`) and SEC-135 (pin `actions/*`, D3). It does not change CI triggers, which stay dispatch/schedule per D1.

**Priority**: medium · **Estimate**: S

## Goal

Every dependency audit so far was a manual step run by an audit agent. No `cargo audit`, `cargo deny`, `bun audit` or `pip-audit` step exists in `.github/` or the Makefile (security agent, Hardening E4). A new RUSTSEC advisory against a crate in the 447-crate graph therefore goes unnoticed until the next `/opus-audit`.

Add:
- a non-`checkall` `make audit-deps` target that runs all three ecosystems;
- a `deny.toml` that states the repo's advisory, source and ban policy, including the known `paste` advisory with its reason;
- a weekly-plus-dispatch workflow that runs the same target.

## Current state

- No `deny.toml` and no `.cargo/audit.toml` exist. `Cargo.lock` is committed since ARC-107 (D2 approved), so `cargo audit` scans the same lockfile CI and release builds use (`--locked`).
- **Local tools:** `cargo-deny` 0.20.2 and `cargo-audit` are installed, as are `bun` 1.4.2 and `uv` 0.12.1. `pip-audit` is **not** installed, so run it through `uvx`.
- **The `paste` advisory** (RUSTSEC-2024-0436, unmaintained, SEC-134).
  - Probe at HEAD f6535f2: `cargo audit` reports it. `cargo deny --all-features check advisories` with default config reports `advisories ok` and does not mention it.
  - The reason is that `paste` 1.0.15 is **lockfile-only**. In `Cargo.lock` it is required by `rav1e` (via `ravif`, the AVIF encoder) and by `pulp` (via `exr`), both optional dependencies of `image`. `Cargo.toml:139` enables `image` with an explicit feature list that includes neither `avif` nor `exr`, and `cargo tree -i paste --all-features --target all -e all` prints nothing.
  - So `paste` is never compiled into any artifact. cargo-deny prunes by the real feature graph; cargo-audit reads the whole lockfile.
  - SEC-134's "pulled in transitively" should be read as "present in the lockfile, not built". Record that correction when SEC-134 is worked.
- **Existing CI patterns to copy:**
  - Scheduled plus dispatch: `bench.yml:3-8` (weekly cron, off the hour), `fuzz.yml:7-10` (nightly).
  - Pinned `dtolnay/rust-toolchain@6bed0761… # stable` and `oven-sh/setup-bun@0c5077e5… # v2` (`ci.yml:141,260`); `actions/*` stay on tags (ENH-032, D3).
  - Discord notification steps exist in `deployment.yml`/`release.yml` if a failure alert is wanted.
- **Frontend and Python lockfiles:** `web-terminal-frontend/bun.lock`, `uv.lock`.

## Implementation

1. **`deny.toml`** (repo root, cargo-deny v2 schema).
   ```toml
   [graph]
   all-features = true            # audit every shipped feature combination's graph

   [advisories]
   version = 2
   yanked = "deny"
   # The paste ignore below documents a lockfile-only advisory that feature
   # pruning currently hides; "unused" is the expected state.
   unused-ignored-advisory = "allow"
   ignore = [
     # paste: unmaintained (no vulnerability). Lockfile-only: required by
     # rav1e/pulp, optional deps of `image`'s avif/exr features, which
     # Cargo.toml does not enable. Remove when image drops it or if avif/exr
     # is ever enabled (SEC-134).
     { id = "RUSTSEC-2024-0436", reason = "lockfile-only via image avif/exr; not compiled" },
   ]

   [bans]
   multiple-versions = "warn"     # informational; the graph has known dups
   wildcards = "deny"

   [sources]
   unknown-registry = "deny"
   unknown-git = "deny"
   allow-registry = ["https://github.com/rust-lang/crates.io-index"]
   ```
   - Add `unused-ignored-advisory = "allow"` under `[advisories]`, with a comment. The ignore entry documents a lockfile-only advisory that feature pruning currently hides, so "unused" is the expected state.
   - **Probe (2026-09-29, cargo-deny 0.20.2, scratch copy of HEAD f6535f2 with exactly this config):** `cargo deny check advisories sources bans` exited 0 with `advisories ok, bans ok, sources ok`. It printed 18 `warning[duplicate]` lines (base64, bitflags, syn, thiserror, …) from `multiple-versions = "warn"` and no errors. `unused-ignored-advisory` is accepted, and `wildcards = "deny"` does not trip on `par-term-emu-derive = { path = "derive", version = "0.45.0" }`, which carries a version.
   - Leave `licenses` out of the default check set. The brief scopes this to advisories, sources and bans, and a license policy is a separate decision.
2. **`make audit-deps`** (not in `checkall`: network-dependent and slow).
   ```make
   # ENH-040: dependency advisories across all three ecosystems. Needs network
   # (advisory DB, npm and PyPI audit APIs); deliberately NOT in checkall.
   audit-deps:
   	@command -v cargo-deny >/dev/null 2>&1 || { echo "ERROR: cargo install cargo-deny --locked"; exit 1; }
   	cargo deny check advisories sources bans
   	cd web-terminal-frontend && bun audit
   	@reqs=$$(mktemp); \
   	uv export --frozen --no-emit-project --no-hashes --all-extras --format requirements-txt -o $$reqs && \
   	uvx pip-audit --strict -r $$reqs; \
   	status=$$?; rm -f $$reqs; exit $$status
   ```
   - The `mktemp` name avoids collisions between concurrent runs. Each step's own exit code decides the result, with no pipe into `tail` masking it.
   - Add a `make help` line.
3. **`.github/workflows/deps-audit.yml`.**
   - Triggers: `on: { schedule: [{cron: "23 7 * * 1"}], workflow_dispatch: {} }`, off the hour and not colliding with `bench.yml`'s Monday 09:17.
   - One `ubuntu-latest` job with `timeout-minutes: 20`. Steps:
     - `actions/checkout@v7`;
     - the pinned `dtolnay/rust-toolchain` SHA already used in `ci.yml`;
     - `cargo install cargo-deny --locked`, or `taiki-e/install-action@<sha> # vX` with `tool: cargo-deny`, pinned per ENH-032;
     - the pinned `oven-sh/setup-bun` SHA from `ci.yml:260`;
     - `astral-sh/setup-uv@<sha> # vX`, pinned;
     - `make audit-deps`.
   - Resolve every new third-party SHA with `gh api repos/<o>/<r>/git/ref/tags/<tag> --jq .object.sha`, dereferencing annotated tags. Dependabot (`.github/dependabot.yml`) keeps them current.
   - Optional: a Discord step on `failure()`, copied from `deployment.yml`'s pattern.
4. **Docs.**
   - A CONTRIBUTING "Verification" bullet: `make audit-deps` runs weekly in CI and on demand locally.
   - SECURITY.md: a one-line "Dependency advisories" note pointing at `deny.toml` for the accepted-advisory list and its reasons.
5. **Do not push or dispatch.** Verifying the workflow needs a user-approved `gh workflow run deps-audit.yml`.

## Files to touch

- `deny.toml` (new)
- `Makefile` (`audit-deps`, `help`)
- `.github/workflows/deps-audit.yml` (new)
- `CONTRIBUTING.md`, `docs/SECURITY.md` (one line each)

## Verify

- `cargo deny check advisories sources bans` exits 0 at the repo root and prints `advisories ok, bans ok, sources ok`, with no `error[` lines (duplicate-version warnings are allowed). `RUSTSEC-2024-0436` either does not appear or appears only as an ignored note, never as an error.
- With the `RUSTSEC-2024-0436` entry removed from a scratch copy of `deny.toml`, `cargo audit` still reports `paste`. The committed `deny.toml` carries that ID with a `reason` naming the lockfile-only avif/exr path.
- `make audit-deps` exits 0 locally, with each of `cargo deny`, `bun audit` and `uvx pip-audit` printing its clean summary, and `git status --porcelain` shows no new untracked file.
- `python3 -c "import yaml;d=yaml.safe_load(open('.github/workflows/deps-audit.yml'));assert 'schedule' in d[True] and 'workflow_dispatch' in d[True]"` passes (PyYAML parses the `on:` key as `True`). `grep -oE 'uses: [^ ]+' .github/workflows/deps-audit.yml | grep -v '^uses: actions/' | grep -vE '@[0-9a-f]{40}$'` prints nothing, so every non-`actions/*` action is SHA-pinned.
- `make -n checkall` does not mention `audit-deps` or `cargo deny`, and `make checkall` stays green.

## Rollback

Delete `deny.toml`, the workflow and the target. Nothing else depends on them.
