# par-term-emu-core — workspace member design (ARC-007, option O2)

Decision (2026-10-07, card `01a1144d7bd576f5878df431e735a833`): split the
single `par-term-emu-core-rust` crate into a layered Cargo workspace with
**zero public-path breaks**. Every path an embedder or the Python package
reaches today (`par_term_emu_core_rust::terminal::Terminal`,
`par_term_emu_core_rust::debug_log!`, `par_term_emu_core_rust._native`, …)
resolves byte-identically after every phase.

## Target end state (all phases)

| Crate | Path | Owns |
|-------|------|------|
| `par-term-emu-core` | `crates/par-term-emu-core` | Terminal state machine, grid, VTE sequences, PTY session, graphics, screenshot renderer, tmux control-mode parser, and the text/ansi/keyboard/mouse/zone/unicode utilities |
| `par-mux` | `crates/par-mux` (Phase 2) | `mux` (daemon library) + `mux::attach` client |
| `par-term-emu-core-rust` | repo root (shell) | `streaming`, `python_bindings` + `_native` module, `ffi` (C ABI), `prelude`, both binaries; re-exports both members at their historical paths |

Dependency DAG (arrows point at the dependency):

```text
par-term-emu-derive (proc-macro, independent version)
        ^
        |
par-term-emu-core-rust (root shell) ---> par-mux ---> par-term-emu-core
        |                                                  ^
        +--------------------------------------------------+
```

`par-term-emu-core` depends on nothing in the workspace. `par-mux` depends on
core only. The root depends on both and on the derive crate.

### Deviation from the original O2 sketch: `tmux_control` lives in core

The O2 sketch put `tmux_control` with par-mux. It cannot go there: `Terminal`
owns a `TmuxControlParser` field (`terminal/mod.rs`, `TmuxState`), runs it in
`Terminal::process`, and returns `TmuxNotification` from public methods
(`tmux_notifications`, `drain_tmux_notifications`). With the DAG above, core
cannot reach into par-mux, so the parser moves with the Terminal. **par-mux is
`mux` + `mux::attach` only.** par-mux and the root consume
`par_term_emu_core::tmux_control` like any other core module.

## Phase 1 boundary — what moves into `par-term-emu-core`

Rule: a module moves iff every consumer is inside the core set or is a root
re-export surface (`lib.rs`, `prelude.rs`). Evidence: `crate::` reference
census of `src/` at f96a2e8.

| Module | Moves | Evidence |
|--------|-------|----------|
| `terminal/**` (incl. `trigger`, `replay_snapshot`, `snapshot_manager`, `search`, `observer`, `event_broker`, `event_fields`, `recording`) | yes | the core itself |
| `grid/**` (incl. `snapshot`) | yes | owned by `Terminal` |
| `pty_session/**` | yes | wraps `Terminal`; its only root-side hook (`PtyInputHandle`) is mux-gated |
| `graphics/**` | yes | `Terminal` owns the graphics store and kitty parser |
| `screenshot/**` (+ embedded fonts) | yes | renders `Grid`/`Terminal`; no root dependency |
| `cell`, `color`, `cursor`, `mouse`, `zone`, `conformance_level`, `shell_integration`, `sixel`, `grapheme` | yes | leaf types of `Terminal`/`Grid` |
| `text_utils`, `ansi_utils`, `color_utils`, `unicode_width_config`, `unicode_normalization_config` | yes | used by terminal/grid/screenshot; depend only on core types |
| `keyboard` | yes | depends on `terminal`; `macros` depends on it. Its `repr(C)` structs stay the FFI types (cbindgen reads them through `parse_deps`) |
| `macros` | yes | `Terminal` stores `Macro`/`MacroPlayback` |
| `badge` | yes | `Terminal` stores `SessionVariables`; OSC 1337 decodes badges |
| `coprocess`, `pty_error` | yes | consumed by `pty_session` |
| `debug` (+ `debug_*!` macros) | yes | used throughout core; macros use `$crate` |
| `html_export` | yes | holds an inherent `impl Color` block, which must live in `Color`'s crate |
| `tmux_control` | yes | see deviation above |
| `prelude` | no | root API-tier surface that re-exports core, mux, python and streaming items |
| `ffi` | no | C ABI; root feature `ffi`, cbindgen crate |
| `python_bindings`, `streaming`, `mux`, `bin/*` | no | root shell (mux moves to par-mux in Phase 2) |

`terminal/tests/ffi_tests.rs` is the one test file that leaves the subtree: it
exercises `crate::ffi::SharedState`, so it stays in the root
(`src/ffi_tests.rs`, wired from root `lib.rs`).

Root `src/lib.rs` keeps every historical path as a re-export:
`pub use par_term_emu_core::{terminal, grid, cell, …};` module by module, the
four `#[macro_export]` debug macros, and the existing convenience re-exports
(`NormalizationForm`, width helpers, recording and badge types, `observer`).

## Feature mapping

The member supports a python-free, PTY-free build: **the member with no
features is the headless (`sim`) profile.**

| Root feature | Member feature it forwards | Notes |
|--------------|---------------------------|-------|
| `screenshot` | `par-term-emu-core/screenshot` | gates `dep:swash` + embedded fonts in the member |
| `pty_session` | `par-term-emu-core/pty_session` | member gates `portable-pty`; root keeps `nix` (root mux/bin use it) |
| `serde` | `par-term-emu-core/serde` | `smallvec/serde`, `bitflags/serde` move to the member |
| `macro-yaml` | `par-term-emu-core/macro-yaml` | `dep:serde_yaml_ng` moves to the member |
| `mux` | `par-term-emu-core/mux` | member `mux` only exposes the two mux hooks (see promotions) |
| `ffi` | `par-term-emu-core/ffi` | member `ffi` gates `for_each_dirty_range*` and `event_fields` |
| `python`, `python-test` | `par-term-emu-core/python` | member `python` = optional `pyo3` dep hosting the `From<PtyError/GraphicsError/ScreenshotError> for PyErr` conversions (orphan rule: both types are foreign to the root) and `event_fields` |
| `sim` | (none) | stays a root marker feature; root keeps the `sim`+`python` `compile_error!` guard. Root `sim` resolves the member with no features |
| `rust-only`, `full`, `streaming`, `streaming-bin`, `mux-bin`, `attach`, `jemalloc`, `regenerate-proto` | (none directly) | root-only; they reach member features only through the forwards above |

Member feature `python` never builds bindings; it exists so the conversions
the root's `?` sites rely on can live next to the error types. The member
with no features compiles neither pyo3 nor swash nor portable-pty
(`scripts/check_features.sh` asserts it).

## Version train

- All members share the root's version (lockstep). `par-term-emu-core` is
  `0.58.1` today. This is behavior, not tidiness: the XTVERSION/DA reply in
  `terminal/sequences/csi/report.rs` embeds `env!("CARGO_PKG_VERSION")`,
  which after the move is the member's version.
- The root depends on the member with `path` + an exact `version = "=X.Y.Z"`
  pin, so a published root can only resolve the core it was built with.
- `make core-version-check` (wired into `checkall` and `release-check`) fails
  when `crates/par-term-emu-core/Cargo.toml`, the root version, or the root's
  dependency pin disagree. CLAUDE.md's version-sync list names the member
  manifest.
- `par-term-emu-derive` keeps its independent version (unchanged).
- **Publish order:** derive (if bumped) → `par-term-emu-core` → `par-mux`
  (Phase 2) → root. crates.io strips `path`, so each dependency must exist on
  the registry before its dependent publishes. `deployment.yml`
  (`publish-crates`) and `publish-crates.yml` must gain a "publish
  par-term-emu-core, wait for index propagation" step before the main
  publish **before the next release** (tracked as Phase-1 follow-up; the
  workflows are dispatch-only and not exercised by this phase).

## Invariants

1. **Public-path identity.** Every `par_term_emu_core_rust::…` path that
   resolved before the move still resolves (re-exports in root `lib.rs`).
   Integration tests, benches, examples and the fuzz crate compile unchanged.
2. **Stub identity.** `make dev && make stubs` regenerates
   `python/par_term_emu_core_rust/_native.pyi` identical to the pre-move
   stub (version line exempt). `make stub-drift` is the proof; all pyclasses
   stay in the root crate.
3. **Token-identical moves.** Moved files are `git mv`s. The only edits
   inside the moved subtree are cross-boundary ones, listed below; `crate::`
   paths inside the subtree are unchanged because they stay intra-crate.
4. **FFI header identity.** `include/terminal_core.h` is byte-identical;
   cbindgen parses the member via `[parse] parse_deps = true`,
   `include = ["par-term-emu-core"]`.
5. **Test-count preservation.** Each pre-move `make test-rust` invocation's
   test total equals the sum of its root + member counterparts after the move.

## Cross-boundary edits (Phase 1)

Visibility promotions (`pub(crate)` → `#[doc(hidden)] pub`, feature gates
kept) are listed in the section below, filled from the compiler's E0603 /
E0616 / E0624 errors when the root is built with its widest feature set — no
speculative promotions.

Other edits inside the moved subtree:

- `terminal/mod.rs`: `event_fields` gate `any(python, python-test, ffi)` →
  `any(python, ffi)` (the member has no `python-test`; root `python-test`
  forwards member `python`).
- `terminal/tests/mod.rs`: drops the `ffi_tests` module (moved to root).
- `terminal/event_broker.rs`: the observer-panic `log::error!` pins
  `target: "par_term_emu_core_rust::terminal::event_broker"`, the module path
  it logged under before the move, so `RUST_LOG` filters keep matching.
- Doctests naming `par_term_emu_core_rust::` inside the member now name
  `par_term_emu_core::` (they compile against the member).
- Member `lib.rs` hosts the three `From<…> for PyErr` impls (moved verbatim
  from root `lib.rs`, gated on member `python`).

### Promoted items

(Filled in by the move commit.)

## Follow-on phases (O2)

- **Phase 2 — `par-mux` member.** `git mv src/mux crates/par-mux/src/…`;
  par-mux depends on `par-term-emu-core` (`mux`, `pty_session`, `serde`
  features). The root re-exports it as `pub mod mux` (`pub use par_mux as
  mux;` or a module re-export) behind root feature `mux`. The `par-mux`
  binary and its `mux-bin`/`attach` features may stay in the root or move
  with the member; either way the binary name and CLI are unchanged.
  Prerequisite reading: the promotions list above (the mux hooks are already
  public on core), and `streaming::mux_factory` / `streaming::roster_watcher`,
  which stay in the root and consume par-mux.
- **Phase 3 — root shell cleanup.** Root keeps only `streaming`,
  `python_bindings`, `ffi`, `prelude`, binaries, and re-exports; publish
  workflow gains the par-mux step; docs (ARCHITECTURE.md crate diagram)
  updated.
