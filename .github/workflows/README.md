# GitHub Actions Workflows

This directory contains the GitHub Actions workflows that build, test, and publish `par-term-emu-core-rust` to PyPI, crates.io, and GitHub Releases.

## Workflows at a glance

| Workflow | File | Trigger | Purpose |
|----------|------|---------|---------|
| **Build and Deploy** | `deployment.yml` | Manual (`workflow_dispatch`) | Full release: streaming binaries + Python wheels + sdist + web frontend → GitHub Release (Sigstore) → PyPI + crates.io |
| **Release and Publish** | `release.yml` | Manual (`workflow_dispatch`) | Thin wrapper that dispatches `deployment.yml` on `main` |
| **CI** | `ci.yml` | Manual (`workflow_dispatch`) | Version check + multi-OS test/lint/build gate (no publish) |
| **Publish 🐍 📦 to TestPyPI** | `publish-testpypi.yml` | Manual (`workflow_dispatch`) | Build + publish to TestPyPI + verify install |
| **Publish to crates.io** | `publish-crates.yml` | Manual (`workflow_dispatch`) | Standalone crates.io publish (idempotent; for republishing without a full release) |

All workflows are **manual-dispatch only** — none run on push or PR. The everyday quality gate is the local pre-commit setup (`make pre-commit-run`, also enforced on commit); these workflows are run on demand.

## The release process (`deployment.yml`)

`deployment.yml` is the canonical release workflow. It is what `make deploy` triggers (and what `release.yml` dispatches). It runs the whole pipeline end-to-end:

```
version-check ──┬─► build-streaming-binaries  (5 targets)
                ├─► linux    (x86_64 + aarch64 × py 3.12/3.13/3.14)
                ├─► macos    (x86_64 + universal2 × py 3.12/3.13/3.14)
                ├─► windows  (x86_64 × py 3.12/3.13/3.14)
                ├─► sdist
                └─► package-web-frontend
                              │
                              ▼
                       github-release  ──► publish        (PyPI)
                              │        └─► publish-crates (crates.io)
                              ▼
                  GitHub Release v$VERSION
```

**Jobs (8):**

1. **Verify Version Consistency** — asserts `Cargo.toml`, `pyproject.toml`, and `__init__.py` agree on the version; exports it for downstream jobs.
2. **Build streaming binary** (matrix, 5 targets) — builds the standalone `par-term-streamer` server binary for Linux x86_64/aarch64, macOS x86_64/aarch64, and Windows x86_64 (`--no-default-features --features streaming-bin`). Linux ARM64 cross-compiles with `gcc-aarch64-linux-gnu`; Unix binaries are stripped.
3. **linux / macos / windows** (matrix × Python 3.12/3.13/3.14) — builds wheels via `PyO3/maturin-action`. Linux ARM64 uses QEMU. x86_64 runners also install the wheel and run `pytest` (PTY/ioctl tests are ignored; Windows uses `-k "not pty"`). macOS builds both x86_64 and universal2 wheels.
4. **Build source distribution** — `maturin sdist`.
5. **Package web terminal frontend** — archives the committed `web_term/` directory into `par-term-web-frontend-v$VERSION.tar.gz` and `.zip`.
6. **Create GitHub Release** — Sigstore-signs the wheels + sdist, creates release `v$VERSION` (`--generate-notes --latest`), and uploads the wheels, streaming binaries, and web-frontend archives.
7. **Publish to PyPI** — trusted publishing (OIDC), `skip-existing`, environment `pypi`.
8. **Publish to crates.io** — publishes the `par-term-emu-derive` sub-crate **first** (crates.io strips `path` deps, so the sub-crate must exist on the registry before the main crate can resolve it), then the main crate with `--no-verify` (the PyO3 cdylib can't link libpython standalone at publish time; it is verified via `make dev` + the test suite). Uses `CARGO_REGISTRY_TOKEN`.

**Workflow permissions:** `contents: write` (create release + upload assets), `id-token: write` (PyPI trusted publishing + Sigstore signing).

### Platform & Python coverage

| Platform | Architecture | Python | Built | Tested |
|----------|--------------|--------|-------|--------|
| Linux | x86_64 | 3.12, 3.13, 3.14 | ✅ | ✅ pytest (PTY/ioctl tests ignored) |
| Linux | aarch64 | 3.12, 3.13, 3.14 | ✅ (QEMU cross-compile) | ⚠️ build only |
| macOS | x86_64 | 3.12, 3.13, 3.14 | ✅ | ✅ pytest (PTY/ioctl tests ignored) |
| macOS | universal2 (Intel + Apple Silicon) | 3.12, 3.13, 3.14 | ✅ | ✅ (on x86_64 runner) |
| Windows | x86_64 | 3.12, 3.13, 3.14 | ✅ | ✅ pytest (`-k "not pty"`) |
| Streaming binary | linux x86_64/aarch64, macos x86_64/aarch64, windows x86_64 | — | ✅ | ⚠️ build only |

## Triggering & verifying a release

Trigger the release workflow (any of these are equivalent — `release.yml` just dispatches `deployment.yml`):

```bash
make deploy            # runs: gh workflow run deployment.yml
gh workflow run deployment.yml
gh workflow run release.yml
```

Watch the run to green, then confirm each registry actually shows the new version (a green workflow is not the same as a published artifact):

```bash
gh run watch <run-id> --exit-status
curl -s https://pypi.org/pypi/par-term-emu-core-rust/json | jq -r .info.version
curl -s -H "User-Agent: release-check/1.0" https://crates.io/api/v1/crates/par-term-emu-core-rust | jq -r .crate.max_stable_version
gh release view v$VERSION --repo paulrobello/par-term-emu-core-rust
```

> **crates.io API gotcha:** anonymous `curl` to `crates.io/api/v1/...` is rejected with a data-access-policy error and returns empty/null. Always pass a `User-Agent` header when querying it.

## Other workflows

### `ci.yml` — CI (manual gate)
Manual-only. Runs the same version-consistency check, then a **test** matrix (ubuntu/macos/windows × Python 3.12/3.13/3.14), a **lint** job (`cargo fmt --check`, `cargo clippy --all-targets --features python,streaming`, `ruff format --check`, `ruff check`, `pyright`), and a **build** job (maturin wheel on each OS). No publishing. Use it for an on-demand full multi-OS gate independent of local pre-commit.

### `publish-testpypi.yml` — TestPyPI
Manual-only. Builds wheels for 5 platforms (Linux x86_64/aarch64, macOS x86_64/universal2, Windows) on **Python 3.14 only**, plus an sdist, publishes to TestPyPI via trusted publishing (environment `testpypi`), then verifies installation by importing `Terminal`/`PtyTerminal` and checking `__version__`.

### `publish-crates.yml` — standalone crates.io publish
Manual-only, with a `skip_tests` input. Checks whether the version already exists on crates.io (`cargo search`); if not, runs tests, dry-runs, then publishes with `CARGO_REGISTRY_TOKEN`. Idempotent — use it to republish or repair a crates.io release without rebuilding the Python wheels or cutting a new GitHub release.

## Required secrets

| Secret | Used by | Purpose |
|--------|---------|---------|
| `DISCORD_WEBHOOK` | all publishing jobs | Success notifications (`continue-on-error: true`, so a bad webhook never fails a release) |
| `CARGO_REGISTRY_TOKEN` | `publish-crates` job in `deployment.yml`, and `publish-crates.yml` | crates.io publish |

PyPI and TestPyPI use **trusted publishing (OIDC)** — no API-token secret is required, but the trusted publisher must be registered on each registry (see below).

## Trusted-publishing setup

Register an OpenID Connect publisher on **PyPI** (https://pypi.org/manage/account/publishing/) and **TestPyPI** (https://test.pypi.org/manage/account/publishing/):

| Field | PyPI | TestPyPI |
|-------|------|----------|
| PyPI project name | `par-term-emu-core-rust` | `par-term-emu-core-rust` |
| Owner | `paulrobello` | `paulrobello` |
| Repository name | `par-term-emu-core-rust` | `par-term-emu-core-rust` |
| Workflow name | `deployment.yml` | `publish-testpypi.yml` |
| Environment name | `pypi` | `testpypi` |

crates.io uses a token (`CARGO_REGISTRY_TOKEN`) rather than OIDC.

## Workflow permissions

| Workflow | Permissions | Why |
|----------|-------------|-----|
| `deployment.yml` | `contents: write`, `id-token: write` | Create release + upload assets; PyPI trusted publish + Sigstore signing |
| `release.yml` | `actions: write` | Dispatch `deployment.yml` |
| `ci.yml` | `contents: read` | Checkout only |
| `publish-testpypi.yml` | `contents: read` (publish job adds `id-token: write`) | Checkout; TestPyPI trusted publish |
| `publish-crates.yml` | `contents: read` | Checkout only |

## Troubleshooting

- **Discord notification not sent** — verify the `DISCORD_WEBHOOK` secret. Notifications use `continue-on-error: true`, so they never fail the workflow.
- **PyPI publish fails** — confirm the trusted publisher is configured (workflow filename and environment name must match exactly), the version doesn't already exist on PyPI, and the `publish` job has `id-token: write`.
- **crates.io publish fails** — confirm `CARGO_REGISTRY_TOKEN` is set and valid. The `par-term-emu-derive` sub-crate must publish first; both publish steps use `continue-on-error`, so check the job logs to confirm each actually published.
- **Version check fails** — `Cargo.toml`, `pyproject.toml`, and `__init__.py` must agree. (The two `derive/Cargo.toml` entries are not checked by the job — keep them in sync manually.)
- **`macos-latest` deprecation annotation** — informational only (the label migrates to macOS 26); it does not fail the run.

## Resources

- [PyPI Trusted Publishing](https://docs.pypi.org/trusted-publishers/)
- [Sigstore](https://www.sigstore.dev/)
- [Maturin Action](https://github.com/PyO3/maturin-action)
- [crates.io data-access policy](https://crates.io/data-access) (User-Agent requirement)
