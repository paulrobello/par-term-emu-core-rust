# Contributing to par-term-emu-core-rust

This guide covers the development workflow, sync rules, and review expectations for the par-term-emu-core-rust terminal emulator library. Read it before opening a pull request.

## Table of Contents

- [Development Setup](#development-setup)
- [Build Rules](#build-rules)
- [Verification](#verification)
- [Benchmarks](#benchmarks)
- [Fuzzing](#fuzzing)
- [Version Sync](#version-sync)
- [Rust to Python Binding Sync](#rust-to-python-binding-sync)
- [Streaming Protocol Changes](#streaming-protocol-changes)
- [Pull Request Workflow](#pull-request-workflow)
- [Related Documentation](#related-documentation)

## Development Setup

Set up a working build environment.

```bash
make setup-venv   # Create .venv and install Python dependencies
make dev          # Build the library (release mode via maturin)
```

`make dev` rebuilds the Python extension in place. Run it after every Rust change you want to test from Python.

## Build Rules

> **Warning:** Never use `cargo build` directly for this PyO3 module. It fails at the link stage because the `extension-module` feature produces a Python extension that cannot be linked as a normal binary. Always build with `make dev` (maturin).

The only time you invoke `cargo` directly is for Rust tests, which require the `--no-default-features --features pyo3/auto-initialize` workaround so the test harness can bootstrap a Python interpreter. See `docs/BUILDING.md` for the full rationale.

## Verification

Run the full quality gate before every commit.

```bash
make checkall     # All checks: clippy, fmt, ruff, pyright, Rust + Python tests, FFI header/surface drift
```

Targeted checks during development:

```bash
make test         # All tests (Rust + Python)
make test-rust    # Rust tests only
make test-python  # Python tests only (rebuilds first)
make lint         # Rust clippy + fmt (auto-fix)
make lint-python  # Python ruff format + check + pyright
```

- `make mux-docs-check` fails until a new mux command, notification or notification type is listed in MUX.md and API_REFERENCE, and when MUX.md, a par-mux `[Unreleased]` CHANGELOG bullet, or `config.rs` rustdoc names a `--flag` that `par-mux`'s clap surface does not define (another program's flag goes in `FOREIGN_FLAGS` with a reason).
- `make audit-deps` runs the dependency audits (cargo deny, `bun audit`, `pip-audit`) across the Rust graph, the web frontend and the Python environment. It needs network, is not part of `checkall`, and runs weekly in CI (`.github/workflows/deps-audit.yml`); accepted advisories and their reasons live in `deny.toml`.

Do not push until `make checkall` passes cleanly. When fixing a failing test, confirm you are fixing the actual bug and not papering over a real issue in the code.

### Feature matrix

Any edit to `[features]` or the dependency lists in `Cargo.toml` gets an additional gate (ENH-019): `make check-features` builds every feature on its own (`cargo hack --each-feature`), checks the combinations that matter (`rust-only,mux`, `rust-only,streaming`, `streaming-bin`, `python-test`), and asserts that heavy dependencies stay out of the slim profiles — no `pyo3`/`tokio`/`clap` in `rust-only`, no `portable-pty`/`pyo3`/`tokio` in `sim`, no `tokio-tungstenite` in `mux`, no `clap`/`tracing-subscriber` in library `streaming`. It needs `cargo install cargo-hack --locked` locally and takes minutes, so it is not part of `make checkall`; the CI `features` job runs it on every dispatch.

`Cargo.lock` is committed (ARC-107), and CI and release builds run with `--locked`, so they fail instead of re-resolving when the lockfile is stale. Commit the `Cargo.lock` change together with any `Cargo.toml` dependency or feature edit, and bump versions with `cargo update -p <crate>`. `fuzz/` is a separate workspace whose lockfile stays untracked.

## Benchmarks

Criterion benchmarks for the VTE processing hot path live in `benches/terminal_throughput.rs`. They drive the real `Terminal::process` pipeline (vte parser, sequence dispatch, grid writes, scrolling, Sixel/Kitty graphics) and report throughput as MB/s.

```bash
make bench         # cargo bench --no-default-features --features rust-only
```

Benchmarks build without the Python feature, so no `make dev` / maturin step is needed. They are on-demand only and never run as part of `make checkall`. The committed baseline is `docs/fable/BENCH-BASELINE-2026-09.md` — compare your changes against it when touching the hot path (its methodology section explains why you must interleave A/B binaries rather than compare across sessions):

```bash
cargo bench --no-default-features --features rust-only -- --save-baseline my-change
# ... after further changes ...
cargo bench --no-default-features --features rust-only -- --baseline my-change
```

A quick compile-and-run sanity check without timing (useful in CI-like environments) is `cargo bench --no-default-features --features rust-only -- --test`. A scheduled gate also tracks these benches against a stored baseline tag over time — see [docs/BENCHMARKING.md](docs/BENCHMARKING.md).

## Fuzzing

Coverage-guided fuzz targets for the code that consumes bytes from untrusted programs live in `fuzz/` (a cargo-fuzz crate, deliberately outside the workspace). One target per untrusted-byte entry point: `terminal_process` (the whole VTE pipeline plus the export walkers), `sixel` (the Sixel state machine, asserting `SixelLimits` holds), `kitty` (the Kitty graphics APC parser, mirroring the real caller's continuation/final-chunk semantics), `apc_filter` (the APC pre-filter byte state machine every terminal byte walks, driven directly via a `cfg(fuzzing)` entry point — ENH-020), `tmux_control` (notification parsing, including re-parsed split points for partial-line buffering), and the par-mux control-protocol grammars `mux_parse_command` / `mux_hook_report`, and the attach client's stdin tokenizer `attach_input` (ENH-046: one long-lived parser across fuzzer-chosen splits, asserting paste bodies never leak out of `Token::Paste`) (ENH-018). All eight have a `make fuzz-<target>` entry and a slot in the CI matrix.

```bash
make fuzz-all                    # each target for FUZZ_SECONDS (default 60)
make fuzz-kitty                  # one target
cargo +nightly fuzz run kitty -- -max_total_time=600 -rss_limit_mb=512   # directly
```

Requirements: a nightly toolchain (`rustup toolchain add nightly`) and `cargo install cargo-fuzz --locked`. Fuzzing is never part of `make checkall`; the nightly CI job (`.github/workflows/fuzz.yml`) runs each matrix target for 10 minutes on a schedule and on manual dispatch. Every run is memory-bounded with `-rss_limit_mb=512` (ENH-020): a decompression bomb or unbounded buffer is a *finding*, not a slower run — SEC-109's 581 MB peak sat under libFuzzer's 2 GiB default and was never flagged.

**Crash-to-regression policy:** a crash artifact (`fuzz/artifacts/<target>/crash-*`) is never just deleted. Reproduce it locally, minimize it (`cargo +nightly fuzz fmt <target> <artifact>`), copy the minimized input into the parser's normal test module, and add a `#[test]` asserting the panic is gone — the artifact's bytes become a permanent regression test. Only then is the artifact file itself deletable. A crash found by fuzzing is a defect card, not a blocker for whatever change is in flight.

Seeds live in `fuzz/corpus/<target>/*.seed` and are committed; everything else the fuzzer writes under `fuzz/corpus/` is ignored.

## Version Sync

When bumping the project version, update all three files to the same value in one commit:

1. `Cargo.toml` (`version = "X.Y.Z"`)
2. `pyproject.toml` (`version = "X.Y.Z"`)
3. `python/par_term_emu_core_rust/__init__.py` (`__version__ = "X.Y.Z"`)

Also update `CHANGELOG.md` and note breaking changes in both `CHANGELOG.md` and the README "What's New" section.

Before tagging, run `python3 scripts/check_release_notes.py --write` (compare links), then `make release-check`, which must pass. It also lints the checked CHANGELOG section's structure: one subsection per Keep-a-Changelog type, no detached bullets, no `` `<this>` `` placeholders, existing paths, and an `[Unreleased]` link based on the newest version.

## Rust to Python Binding Sync

When you add or modify a Rust method on `Terminal` or `PtySession`, keep the layers in sync:

1. Add the Python binding in `src/python_bindings/terminal/` (in the themed `*_api.rs` file matching the feature area, or `mod.rs`; shared getter/setter pairs can go through the `common.rs` macro layer) or `src/python_bindings/pty.rs`.
2. Add docstrings with `Args`, `Returns`, and `Example` sections (Google style). Document every `#[getter]` as well: `make stub-docs-check` (part of `checkall`) fails when a def in the generated `_native.pyi` has no docstring, allowing only setters of documented getters, `__init__` of documented classes, and `__enter__`/`__exit__`.
3. Update `docs/API_REFERENCE.md`.
4. Update `README.md` if the change is user-facing.
5. Add Python tests in `tests/` when the feature is reachable from Python.

Files that must stay in lockstep:

- Rust impl (`crates/par-term-emu-core/src/terminal/mod.rs`) ↔ Python binding (`src/python_bindings/terminal/`)
- Rust impl (`crates/par-term-emu-core/src/pty_session/`) ↔ Python binding (`src/python_bindings/pty.rs`)
- Python binding ↔ API reference (`docs/API_REFERENCE.md`)

## Streaming Protocol Changes

The streaming protocol has three layers. A protocol change touches all of them:

1. `proto/terminal.proto` generates `src/streaming/terminal.pb.rs`. Never edit the generated file directly.
2. `src/streaming/protocol.rs` defines app-level types (`ServerMessage`, `ClientMessage`, `EventType` enums).
3. `src/streaming/proto.rs` converts between app types and the protobuf wire format. The message conversions are generated by `#[derive(ProtoConvert)]` (derive crate) on the `protocol.rs` types. A new variant or field usually needs no `proto.rs` edit; add `#[proto(oneof_variant = ..)]`/`#[proto(message = ..)]` when the wire names differ, and a `ToWire`/`FromWire` impl or a `#[proto(with = ..)]` module in `proto.rs` when the type pair is new. `proto_golden_tests.rs` pins the wire output; regenerate it only for an intended wire change.

Also update:

- `src/python_bindings/streaming.rs` (dict conversion + event type matching)
- `tests/test_streaming.rs` (use `..` in destructuring for forward compatibility)
- `src/streaming/session.rs` (`SessionRegistry::build_connect_message()`)

When extending the `Connected` message, add one method on `ConnectedBuilder` plus the field in `Connected`/builder/`build()` — the single edit site — then update `build_connect_message()` in `session.rs`. Do not add partial constructors.

## Pull Request Workflow

- Branch from `main` and use a descriptive branch name.
- Use [Conventional Commits](https://www.conventionalcommits.org/) messages (for example `feat:`, `fix:`, `docs:`, `chore:`, `refactor:`).
- Keep changes surgical: touch only what the task requires and match the surrounding style.
- Run `make checkall` before pushing. Fix all lint, type, and test failures.
- Do not push or open a PR unless the maintainer requests it.
- Keep the Python and Rust sides in sync (see the sync rules above).
- Keep sister projects (`par-term-emu-tui-rust`, `par-term`) in mind when changing shared CLI options, features, or config.

## Related Documentation

- [CLAUDE.md](CLAUDE.md) - Full build commands, architecture notes, and project conventions
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) - Internal architecture with diagrams
- [docs/BUILDING.md](docs/BUILDING.md) - Detailed build and test instructions
- [docs/DOCUMENTATION_STYLE_GUIDE.md](docs/DOCUMENTATION_STYLE_GUIDE.md) - Documentation standards
- [docs/SECURITY.md](docs/SECURITY.md) - PTY security considerations
