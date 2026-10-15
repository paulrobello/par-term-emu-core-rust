# ENH-060: `ipc_rows` fuzz target and golden seeds for `AgentRow::parse` extras

> Filed from the 2026-10-12 /opus-audit enhancement pass (Security posture hardening ideas, AUDIT.md `:239`). Board card: `[ENH-060]`.
> Sequencing: independent of the ARC-136/138/140 render fixes. ARC-140 retypes `AgentRow` consumers in `attach/status.rs` and `render/sidebar.rs` but not `ipc/rows.rs` itself, so this card can land before or after it. If ENH-061 (`ws=` on `%agent-state-changed`) lands first, nothing here changes, because that notification is parsed by `tmux_control.rs` (already fuzzed by `tmux_control`), not by `AgentRow::parse`.

**Priority**: low · **Estimate**: S

## Goal

`AgentRow::parse` is the attach client's only reader of the `list-agents` roster and of the snapshot's `agent` rows. Its trailing `key=value` extras (`reason=`, `telemetry=`, `host_telemetry=`, `ws=`) are the grammar the side panel groups on. No fuzz target reaches it today, and its unit test has no `ws=` case and no unknown-key case.

Add a dedicated cargo-fuzz target `ipc_rows` that drives all four typed row parsers in `crates/par-mux/src/mux/ipc/rows.rs` and asserts a parse/format round-trip invariant. Commit golden seeds for the extras grammar, and add a stable-toolchain unit test that replays the same seed lines through the same invariant, so `make checkall` covers them without nightly.

## Current state

- **The parser.** `crates/par-mux/src/mux/ipc/rows.rs:143-202`. `AgentRow { pane, agent, state, source, extras: Vec<(String, String)> }` (`:147-158`). `parse` (`:178-201`) splits on Unicode whitespace, requires a `%N` pane (`is_typed_id`, `:11-14`) and three more positional fields, then maps every remaining token through `split_once('=')`. One token without `=` makes the whole row `None` (`:187-193`). `Display` (`:160-172`) re-joins with single spaces.
  - The doc comment (`:143-145`) lists `reason=`, `telemetry=` and `host_telemetry=`, but not `ws=`. The emitter `roster_row` (`crates/par-mux/src/mux/dispatch/client.rs:200-226`) appends `ws=<+N>` when the pane has a workspace (`:218-220`).
- **Sibling parsers in the same file.** `WindowRow::parse` `:37-53`, `SessionRow::parse` `:82-100`, `WorkspaceRow::parse` `:126-140`. None is fuzzed.
- **Unit tests.** `rows.rs:204-313`, 5 tests. `agent_row_round_trips` (`:297-312`) covers `telemetry`/`host_telemetry`, a missing source and a stray token. It has no `ws=`, no unknown key, no empty key or value, no `=` inside a value, and no tab or Unicode-whitespace separators. Measured: `cargo test -p par-mux --features mux --lib ipc::rows` reports `5 passed`.
- **Callers.** `crates/par-mux/src/mux/attach/status.rs:250` (snapshot `agent <row>` lines, after `split_once(' ')` at `:208`) and `:353` (legacy `list-agents` body). Both take the first `ws` extra with `.find(|(k, _)| k == "ws")` (`:252-257`, `:356-361`), so a duplicate `ws=` keeps the first one.
- **Visibility.** `crates/par-mux/src/mux/ipc.rs:24-27`: `pub(crate) mod rows;` and `pub(crate) use rows::{AgentRow, …}`. The structs are `pub`, so a `pub use` from a `pub` module compiles. That is the pattern `attach/mod.rs:34-41` already uses (`#[cfg(fuzzing)] #[doc(hidden)] pub mod fuzz_support { pub use super::input::{InputParser, Token}; }`). `crates/par-mux/Cargo.toml:93-95` already declares `check-cfg = ['cfg(fuzzing)']`.
- **No existing target reaches `AgentRow::parse`.**
  - `fuzz/fuzz_targets/mux_parse_command.rs` calls only `command::{parse_command, parse_line}`, which is the daemon's request grammar.
  - `fuzz/fuzz_targets/attach_input.rs` calls only `attach::fuzz_support::{InputParser, Token}`, which is the stdin tokenizer.
  - `fuzz/fuzz_targets/tmux_control.rs` uses `TmuxControlParser`, which handles notifications.
  - `grep -rn "AgentRow\|ipc" fuzz/fuzz_targets` returns nothing.
- **Fuzz crate.** `fuzz/Cargo.toml` depends on `par-term-emu-core-rust` with `features = ["rust-only", "mux", "attach"]` and has eight `[[bin]]` entries. `par_term_emu_core_rust::mux` re-exports `par_mux::mux` (`src/lib.rs:106-107`), and `ipc` is `pub mod ipc` (`crates/par-mux/src/mux/mod.rs:24`). `fuzz/.gitignore` tracks only `corpus/**/*.seed`.
- **Target lists that say "eight".**
  - `Makefile:79` (help line) and `Makefile:1126` (`fuzz-all`), plus the per-target help block around `:80-87` and the per-target rules `:1102-1124`.
  - `CLAUDE.md:25`.
  - `CONTRIBUTING.md:84` ("All eight have a `make fuzz-<target>` entry and a slot in the CI matrix").
  - `.github/workflows/fuzz.yml:24` (the matrix).
- **Toolchain.** Locally there is `nightly-aarch64-apple-darwin` and `cargo-fuzz 0.13.2`.

## Implementation

1. **Shared invariant in `rows.rs`.** Add this after `impl AgentRow` (`:202`):
   ```rust
   /// Fuzz/test invariant (ENH-060): every parser either rejects `line` or
   /// returns a row whose `Display` parses back to the same row.
   #[cfg(any(test, fuzzing))]
   pub fn assert_round_trip(line: &str) {
       if let Some(row) = WindowRow::parse(line) {
           assert_eq!(WindowRow::parse(&row.to_string()), Some(row), "window: {line:?}");
       }
       if let Some(row) = SessionRow::parse(line) {
           assert_eq!(SessionRow::parse(&row.to_string()), Some(row), "session: {line:?}");
       }
       if let Some(row) = WorkspaceRow::parse(line) {
           assert_eq!(WorkspaceRow::parse(&row.to_string()), Some(row), "workspace: {line:?}");
       }
       if let Some(row) = AgentRow::parse(line) {
           assert!(row.extras.iter().all(|(k, _)| !k.contains('=')), "agent key kept '=': {line:?}");
           assert_eq!(AgentRow::parse(&row.to_string()), Some(row), "agent: {line:?}");
       }
   }
   ```
   The round-trip holds by construction for every parser, which is what the invariant is meant to pin. For example, an `AgentRow` field never contains whitespace, and a key never contains `=` because the split is on the first `=`. If the fuzzer breaks it, that is a finding. Follow the CONTRIBUTING.md:94 crash-to-regression policy and file a defect card. Do not weaken the assertion.
   - Also add `ws=<+N>` and "unknown keys are kept, in wire order" to the `AgentRow` doc comment at `:143-145`, so it matches `roster_row`.
2. **Fuzz-only re-export in `crates/par-mux/src/mux/ipc.rs`.** Add this after `:27`, mirroring `attach/mod.rs:34-41`:
   ```rust
   // Fuzz-only re-export (ENH-060): `rows` is pub(crate), the fuzz crate is an
   // external path dependency, and cargo-fuzz is the only builder that sets the cfg.
   #[cfg(fuzzing)]
   #[doc(hidden)]
   pub mod fuzz_support {
       pub use super::rows::{assert_round_trip, AgentRow, SessionRow, WindowRow, WorkspaceRow};
   }
   ```
3. **Target `fuzz/fuzz_targets/ipc_rows.rs`.**
   ```rust
   #![no_main]

   //! par-mux reply-row parsers (`ipc::rows`): the attach client reads every
   //! `list-agents`/`list-windows`/`list-sessions`/`list-workspaces` and snapshot
   //! row from the daemon through these. Invariant: each parser rejects the line
   //! or returns a row whose `Display` parses back to itself. Parsed whole from
   //! lossy UTF-8, then per line, then with a snapshot-style `<kind> ` prefix
   //! stripped (status.rs splits it off before `AgentRow::parse`).

   use libfuzzer_sys::fuzz_target;
   use par_term_emu_core_rust::mux::ipc::fuzz_support::assert_round_trip;

   fuzz_target!(|data: &[u8]| {
       let text = String::from_utf8_lossy(data);
       assert_round_trip(&text);
       for line in text.split('\n') {
           assert_round_trip(line);
           if let Some((_, rest)) = line.split_once(' ') {
               assert_round_trip(rest);
           }
       }
   });
   ```
   - Add the matching `[[bin]] name = "ipc_rows"` block to `fuzz/Cargo.toml` after `attach_input`, with `test = false`, `doc = false` and `bench = false`.
   - Extend the comment at `fuzz/Cargo.toml` (the "targets mirror…" paragraph) with "and (ENH-060) the attach client's reply-row parsers".
4. **Golden seeds `fuzz/corpus/ipc_rows/*.seed`.** Each file holds one line with no trailing newline, except `rows_mixed.seed`. Write them with `printf '%s'`. Write `agent_unicode_space.seed` with `printf '%%10 pi idle hook\xe3\x80\x80ws=+0'`.

   | file | content | parse |
   |---|---|---|
   | `agent_basic.seed` | `%1 claude working hook` | Some, no extras |
   | `agent_reason.seed` | `%2 claude blocked hook reason=aGk=` | Some, `=` inside value |
   | `agent_ws.seed` | `%3 pi idle scrape ws=+0` | Some |
   | `agent_all_extras.seed` | `%4 omp working hook reason=d2FpdA== telemetry=e30= host_telemetry=e30= ws=+12` | Some, 4 extras in order |
   | `agent_unknown_key.seed` | `%5 pi idle hook future=1 ws=+1` | Some, unknown key kept |
   | `agent_dup_ws.seed` | `%6 pi idle hook ws=+0 ws=+1` | Some, both kept |
   | `agent_empty_kv.seed` | `%7 pi idle hook == =x k=` | Some: `("","=")`, `("","x")`, `("k","")` |
   | `agent_tabs.seed` | `%8<TAB>pi  working<TAB>hook ws=+2` (real tabs) | Some, normalised on Display |
   | `agent_unicode_space.seed` | `%10 pi idle hook<U+3000>ws=+0` | Some (`split_whitespace` splits U+3000) |
   | `agent_stray_token.seed` | `%9 pi idle hook stray ws=+0` | None |
   | `agent_bad_pane.seed` | `%x pi idle hook ws=+0` | None |
   | `rows_mixed.seed` | `@1 * two\n+0: main: $3: work\n+1: beta active\nagent %2 pi idle hook ws=+1` | one of each row kind |

5. **Stable replay test in `rows.rs` `mod tests`.**
   - Add `const FUZZ_SEED_LINES: &[&str] = &[…]` holding the same twelve contents as Rust literals (`\t`, `\u{3000}`, `\n`).
   - Add the test:
     ```rust
     /// ENH-060: the `fuzz/corpus/ipc_rows` seeds, replayed on stable.
     #[test]
     fn ipc_rows_fuzz_seeds_round_trip() {
         for seed in FUZZ_SEED_LINES {
             assert_round_trip(seed);
             for line in seed.split('\n') {
                 assert_round_trip(line);
                 if let Some((_, rest)) = line.split_once(' ') { assert_round_trip(rest); }
             }
         }
     }
     ```
   - Add `agent_row_extras_grammar` with explicit expectations for the parse column of the table: `ws` lookup on `agent_ws`, extras order and length on `agent_all_extras`, the three pairs on `agent_empty_kv`, and `None` on the two reject seeds.
   - Do **not** `include_str!` the seed files. `par-mux` is published to crates.io, and a path outside the crate would break `cargo package`. The Verify step below diff-checks the files against the literals instead.
6. **Wiring.**
   - `Makefile`: add `fuzz-ipc_rows: ## Fuzz the par-mux reply-row parsers (ENH-060)` with the same recipe shape as `:1123-1124`. Add a help line after `:87`. Append `fuzz-ipc_rows` to `fuzz-all` (`:1126`). Change "eight" to "nine" at `:79` and `:1126`, and add `fuzz-ipc_rows` to the `.PHONY` list if the other fuzz targets are in it.
   - `CLAUDE.md:25`: change "All eight" to "All nine" and append `, fuzz-ipc_rows` inside the parenthesis.
   - `CONTRIBUTING.md:84`: add `` `ipc_rows` `` (the attach client's reply-row parsers, asserting a parse/format round-trip, ENH-060) to the target list. Change "All eight" to "All nine".
   - `.github/workflows/fuzz.yml:24`: append `ipc_rows` to the matrix. Edit no other line of that file.
   - `CHANGELOG.md` `[Unreleased]` `### Added`: add one bullet, "An `ipc_rows` cargo-fuzz target drives the attach client's reply-row parsers (ENH-060; `fuzz/fuzz_targets/ipc_rows.rs`, `crates/par-mux/src/mux/ipc/rows.rs`)". Put it under the existing `### Added` heading. Do not open a second one (ENH-048's lint rule).

## Files to touch

- `crates/par-mux/src/mux/ipc/rows.rs` (invariant fn, doc comment, two tests, seed literals)
- `crates/par-mux/src/mux/ipc.rs` (`cfg(fuzzing)` `fuzz_support`)
- `fuzz/fuzz_targets/ipc_rows.rs` (new), `fuzz/Cargo.toml`, `fuzz/corpus/ipc_rows/*.seed` (12 new)
- `Makefile`, `CLAUDE.md`, `CONTRIBUTING.md`, `.github/workflows/fuzz.yml`, `CHANGELOG.md`

## Verify

Acceptance criteria:
- `cargo test -p par-mux --features mux --lib ipc::rows` reports `7 passed` (5 existing + `ipc_rows_fuzz_seeds_round_trip` + `agent_row_extras_grammar`), not 0, and the replay test fails when `AgentRow`'s `Display` is seeded to drop extras.
- `cargo +nightly fuzz run ipc_rows -- -runs=0` replays all 12 committed seeds with exit 0, and a 60 s `make fuzz-ipc_rows FUZZ_SECONDS=60` run exits 0 with no `fuzz/artifacts/ipc_rows/crash-*`.
- `make fuzz-all`, `CLAUDE.md`, `CONTRIBUTING.md` and `.github/workflows/fuzz.yml` all list nine targets including `ipc_rows`.
- `make checkall-ci` exits 0.

Commands (run from the repo root):
- `cargo test -p par-mux --features mux --lib ipc::rows > /tmp/enh060-t.log 2>&1; echo EXIT=$?; grep 'test result' /tmp/enh060-t.log` prints `EXIT=0` and `7 passed`. `grep -c 'ipc_rows_fuzz_seeds_round_trip ... ok' /tmp/enh060-t.log` prints `1`.
- **Seeded violation.** Temporarily delete the extras loop in `impl fmt::Display for AgentRow` (`rows.rs:167-169`), then rerun the test command. It must print `EXIT=101`, with `ipc_rows_fuzz_seeds_round_trip` failing on an `agent:` message. `cargo +nightly fuzz run ipc_rows -- -runs=0 > /tmp/enh060-bad.log 2>&1; echo EXIT=$?` must be non-zero, with `panicked` in the log. Restore with `git checkout -- crates/par-mux/src/mux/ipc/rows.rs` (only if the seeding was the sole edit in that file; otherwise undo by hand), and confirm both commands are green again.
- `cargo +nightly fuzz run ipc_rows -- -runs=0 > /tmp/enh060-r0.log 2>&1; echo EXIT=$?` prints `EXIT=0`, and `grep -E 'INITED|Done' /tmp/enh060-r0.log` shows the corpus loaded.
- `make fuzz-ipc_rows FUZZ_SECONDS=60 > /tmp/enh060-f.log 2>&1; echo EXIT=$?` prints `EXIT=0`. `ls fuzz/artifacts/ipc_rows/ 2>/dev/null | grep -c crash` prints `0`.
- **Seeds tracked and in sync.** `ls fuzz/corpus/ipc_rows/*.seed | wc -l` prints `12`. `git status --porcelain fuzz/corpus/ipc_rows` lists all 12 (not ignored). This must print `ok`:
  ```
  python3 -c "import pathlib,re;src=pathlib.Path('crates/par-mux/src/mux/ipc/rows.rs').read_text();lits=[bytes(m,'utf-8').decode('unicode_escape').encode('latin-1').decode('utf-8') for m in re.findall(r'^\s*\"((?:[^\"\\\\]|\\\\.)*)\",\s*$',src.split('FUZZ_SEED_LINES')[1].split('];')[0],re.M)];fs=[p.read_text() for p in pathlib.Path('fuzz/corpus/ipc_rows').glob('*.seed')];assert sorted(lits)==sorted(fs),(len(lits),len(fs));print('ok')"
  ```
  If the escape decoding fights the literal style, replace this with an equivalent check. Every seed file's text must equal exactly one literal, and the counts must match.
- `grep -c 'fuzz-ipc_rows' Makefile` prints at least `2`. `grep -c 'All nine' CLAUDE.md CONTRIBUTING.md` prints `1` for each. `grep -c 'ipc_rows' .github/workflows/fuzz.yml` prints `1`. `grep -n 'eight fuzz' Makefile` prints nothing.
- `PATH="$PWD/.venv/bin:$PATH" make checkall-ci > /tmp/enh060-ci.log 2>&1; echo EXIT=$?` prints `EXIT=0`.

## Rollback

All additions. The invariant fn and `fuzz_support` are `cfg(any(test, fuzzing))` / `cfg(fuzzing)`, so release builds and the public API are unchanged. `#[doc(hidden)]` keeps the module out of rustdoc, so `rustdoc-check` (formerly `mux-rustdoc-check`) is unaffected. Revert by deleting the target, the seeds and the `[[bin]]`, and by reverting the count edits. If the 60 s run finds a real round-trip break, keep the target and seeds, mark the failing seed's assertion as the regression test per CONTRIBUTING.md:94, and file the parser defect separately. Do not loosen `assert_round_trip`.

## Dependencies / sequencing

- None blocking. It touches `rows.rs`, which ARC-140 does not edit (ARC-140 edits `status.rs`/`sidebar.rs`). If ARC-140 has been reworked to change `AgentRow`'s field types (for example a typed `ws`), adapt the invariant to the new fields. The seeds stay valid, because they are wire bytes.
- Needs a nightly toolchain and `cargo-fuzz` for the two fuzz criteria (both present locally). Everything else runs on stable.
- Coordinate the CHANGELOG bullet with any concurrent `[Unreleased]` edits (single `### Added`).
