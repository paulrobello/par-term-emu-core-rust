# ENH-055: Environment-variable docs gate (`env-docs-check`)

> Filed from the 2026-10-12 /opus-audit enhancement pass. Board card: `[ENH-055]`.
> Sequencing: land **after DOC-150** (which adds three of the missing rows). This card adds the remaining rows itself, so the gate is green on the tree it lands on.

**Priority**: medium · **Estimate**: M

## Goal

`docs/CONFIG_REFERENCE.md:829` claims "This is the one table of every environment variable the library, the `par-mux` binary, and the `par-term-streamer` binary read." Nothing enforces it, which is how DOC-150 shipped three unlisted variables. Add a checkall gate, `scripts/check_env_docs.py`, that:

1. Collects every environment-variable **read** in production code: `env::var("X")` / `env::var_os("X")` literals and const-resolved identifiers in the Rust crates, plus `process.env.X`, `process.env["X"]`, `parseDurationEnv("X", …)` in `crates/par-mux/integrations/extensions/*.ts`, `os.environ…("X")` / `${X…}` in the hook `.sh` files, `$env:X` in the hook `.ps1` files, and `os.environ.get("X")` in `python/par_term_emu_core_rust/*.py`.
2. Diffs that set against the documented set, both directions.
3. Excludes test-only reads by path and by `#[cfg(test)] mod` stripping, and fails closed on anything it cannot resolve.

A new script rather than a sixth check in `check_mux_docs.py`, because the scope is the whole workspace (core, streamer, Python), not par-mux. It imports `strip_test_modules` from `check_mux_docs.py` so the brace-matching logic is not duplicated.

## Current state

- **Doc table**: `docs/CONFIG_REFERENCE.md:827` `## Environment Variables`, table rows `:833-847` (15 rows). First-column names: `DEBUG_LEVEL`, `PAR_TERM_REPLY_XTWINOPS`, `SHELL`, `COMSPEC`, `PAR_MUX_CONFIG`, `PAR_MUX_SOCKET`, `PAR_MUX_CONTROL_SOCKET`, `PAR_MUX_ENV`, `PAR_MUX_ALLOW_NESTED`, `XDG_RUNTIME_DIR`, `TMPDIR`, `HOME`, `PATH`, `COMPUTERNAME`, and the wildcard row `PAR_TERM_*` (40 variables) at `:847`, delegated to `docs/STREAMING.md`.
- **Pane env contract**: `docs/MUX.md:721` `## Agent Hook Reports`, table `:727-736` (`PAR_MUX_PANE_ID`, `PAR_MUX_SOCKET`, `PAR_MUX_CONTROL_SOCKET`, `PAR_MUX_ENV`, `PAR_MUX_SESSION_ID`, `PAR_MUX_SESSION`, `PAR_MUX_WINDOW_ID`, `PAR_MUX_BIN`). CONFIG_REFERENCE `:829` delegates "the variables par-mux sets inside panes" here, so these count as documented.
- **Streamer**: `src/bin/streaming_server/cli.rs` has 40 `#[arg(…, env = "PAR_TERM_…")]` attributes (e.g. `:99`, `:103`, `:228`). All 40 already appear backticked in `docs/STREAMING.md` (measured).
- **Rust literal reads** (production, measured on aaf35ef): `debug.rs:60` `DEBUG_LEVEL`; `pty_session/mod.rs:229` `PAR_TERM_REPLY_XTWINOPS`; `pty_session/lifecycle.rs:106,113` `COMSPEC`/`SHELL`; `mux/mod.rs:98-99` `PAR_MUX_ENV`/`PAR_MUX_ALLOW_NESTED`; `mux/config.rs:460` `PAR_MUX_CONFIG`; `bin/par_mux/main.rs:44,805-806` `PAR_MUX_SOCKET`/`PAR_MUX_CONTROL_SOCKET`; `attach/mod.rs:218` `PAR_MUX_SOCKET`; `attach/render/mod.rs:396` `PAR_MUX_CURSOR_TRACE`; `mux/pane.rs:1134` `COMPUTERNAME`; `mux/client.rs:81,403` `XDG_RUNTIME_DIR`/`PATH`; `mux/ipc.rs:865` `XDG_RUNTIME_DIR`; `mux/win_resume.rs:60` `PATH`.
- **Rust const-indirect reads**: `integrations/claude.rs:49,79` (`CLAUDE_CONFIG_DIR_ENV_VAR = "CLAUDE_CONFIG_DIR"`), `codex.rs:46,83` (`CODEX_HOME`), `grok.rs:43,70` (`GROK_HOME`), `extensions.rs:37-38` (`PI_CODING_AGENT_DIR`, `PI_CONFIG_DIR`) read through the helpers `env_var_nonempty(name)` (`extensions.rs:159-161`) and `agent_dir_from_env_or_home(env_var, …)` (`:163-164`), called at `:52`, `:59`, `:63`.
- **Non-`env::var` read**: `mux/pane.rs:796` checks the session env for `name == "PAR_MUX_CONTROL" && value == "1"` (ENH-039). No `env::var` call, so it needs an explicit entry.
- **Integration scripts**: `par-mux-omp-agent-state.ts:108-109` `parseDurationEnv("PAR_MUX_OMP_IDLE_DEBOUNCE_MS", 250)` / `("PAR_MUX_OMP_RETRY_GRACE_MS", 2500)`, `:22` `OMPCODE`, `:83` and `par-mux-agent-state.ts:87` `PAR_MUX_SENDLOG`. Hooks: `par-mux-claude-session-hook.sh:18,30` `PAR_MUX_HOOK_ACTION` (set by the script itself for its own python child), `par-mux-codex-session-hook.sh:27,37` and grok `PAR_MUX_HOOK_INPUT_FILE` (same), `CODEX_THREAD_ID`, `GROK_SESSION_ID`, `TMPDIR`; `.ps1` `$env:OS`.
- **Test-only reads to exclude**: `src/streaming/proto_golden_tests.rs:597` `PROTO_GOLDEN_UPDATE` (included via `#[path]` at `src/streaming/proto.rs:544-546`); `crates/par-mux/build.rs:149` `CARGO_MANIFEST_DIR`; `src/python_bindings/pty.rs:1201` `HOME` inside `mod tests` (`:1092`); `crates/par-term-emu-core/src/pty_session/tests.rs:681,701,1759` (identifier args); `crates/par-mux/tests/*` `REEXEC_MARKER`; `crates/par-mux/src/mux/attach/tests.rs:362`.
- **Undocumented today** (prototype run on aaf35ef): `PAR_MUX_CURSOR_TRACE`, `PAR_MUX_CONTROL`, `PAR_MUX_OMP_IDLE_DEBOUNCE_MS` (the DOC-150 three), plus `PAR_MUX_OMP_RETRY_GRACE_MS`, `PAR_MUX_SENDLOG`, `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `GROK_HOME`, `PI_CODING_AGENT_DIR`, `PI_CONFIG_DIR`. Ten in total.
- **Reuse**: `scripts/check_mux_docs.py:226` `strip_test_modules(code)` (QA-229 string/char-literal-aware brace matching), `:143` `md_section` pattern, `:101` `fail()` and the self-test temp-copy pattern (`:586-610`).
- **Makefile**: `mux-docs-check` at `:432-434` is the template; `checkall` at `:525`; `.PHONY` `:4`; help lines `:62`, `:68`.

## Implementation

1. **Create `scripts/check_env_docs.py`** (stdlib only, `from __future__ import annotations`, typed). Module docstring: "ENH-055: every env var production code reads is in docs/CONFIG_REFERENCE.md's table (or MUX.md's pane env contract, or STREAMING.md for the clap `env =` set); every row names a variable something reads." Import: `sys.path.insert(0, str(Path(__file__).resolve().parent))`, then `from check_mux_docs import strip_test_modules`.
2. **Scan roots and exclusions** (walk with `Path.rglob`, not `git ls-files`, so `--self-test` works on a temp copy):
   ```python
   RUST_ROOTS = ("src", "crates/par-term-emu-core/src", "crates/par-mux/src", "derive/src")
   TS_GLOB = "crates/par-mux/integrations/extensions/*.ts"
   SH_GLOB = "crates/par-mux/integrations/hooks/*.sh"
   PS1_GLOB = "crates/par-mux/integrations/hooks/*.ps1"
   PY_GLOB = "python/par_term_emu_core_rust/*.py"
   TEST_PATH_RE = re.compile(
       r"(^|/)(tests|benches|fuzz|examples)/|(^|/)tests\.rs$|_tests\.rs$|(^|/)build\.rs$"
   )
   ```
   For every `.rs` file not matching `TEST_PATH_RE`, run `strip_test_modules()` before extraction.
3. **Rust extraction**:
   ```python
   LIT_RE = re.compile(r'\benv::var(?:_os)?\(\s*"([A-Z][A-Z0-9_]*)"')
   IDENT_RE = re.compile(
       r"\b(?:env::var(?:_os)?|env_var_nonempty|agent_dir_from_env_or_home)\(\s*([A-Za-z_]\w*)\s*[,)]"
   )
   CONST_RE = re.compile(
       r'const\s+([A-Z_][A-Z0-9_]*)\s*:\s*&str\s*=\s*"([A-Z][A-Z0-9_]*)"'
   )
   CLAP_ENV_RE = re.compile(r'\benv\s*=\s*"([A-Z][A-Z0-9_]*)"')
   ```
   - Literal hits are reads. Identifier hits resolve through the same file's `CONST_RE` map.
   - An identifier that does not resolve is a problem (`unresolvable env read <ident> in <file>`) unless it is in the guarded `HELPER_PARAMS` set `{("crates/par-mux/src/mux/integrations/extensions.rs", "name"), ("crates/par-mux/src/mux/integrations/extensions.rs", "env_var")}` (the helpers' own parameters; their call sites are scanned via `IDENT_RE`). A `HELPER_PARAMS` entry whose identifier no longer appears as an unresolved arg in that file is a stale-allowlist problem.
   - `CLAP_ENV_RE` hits in `src/bin/streaming_server/cli.rs` are collected separately as the streamer set.
4. **Script extraction**: TS `process\.env\.([A-Z][A-Z0-9_]*)`, `process\.env\[\s*"([A-Z]…)"`, `parseDurationEnv\(\s*"([A-Z]…)"`; `.sh` `os\.environ(?:\.get)?[\(\[]\s*"([A-Z]…)"` and `\$\{([A-Z][A-Z0-9_]*)` (only in `.sh`, since TS template literals produce `${JSON…}` noise); `.ps1` `\$env:([A-Za-z_]\w*)`; `.py` the `os.environ` pattern.
5. **Explicit non-`env::var` reads**, guarded:
   ```python
   EXTRA_READS: dict[str, str] = {
       "PAR_MUX_CONTROL": "crates/par-mux/src/mux/pane.rs",  # session-env grant (ENH-039), compared by name
   }
   ```
   Each entry must still appear as the string literal `"<NAME>"` in its file (after test stripping), or it is a stale-entry problem.
6. **Allowlist of reads that are not this project's knobs**, each with a reason; an entry nothing reads any more is a stale-allowlist problem:
   ```python
   NOT_OURS: dict[str, str] = {
       "PAR_MUX_HOOK_ACTION": "set by par-mux-claude-session-hook.sh for its own python child",
       "PAR_MUX_HOOK_INPUT_FILE": "set by the codex/grok hook .sh for its own python child",
       "CODEX_THREAD_ID": "codex's own session identity (MUX.md identity-var drop)",
       "GROK_SESSION_ID": "grok's own session identity",
       "OMPCODE": "omp's nested-session marker (MUX.md:150)",
       "OS": "Windows built-in, PowerShell platform test",
   }
   STD_INDIRECT: dict[str, str] = {
       "HOME": "dirs::home_dir() (mux/persist.rs:844, skill_installer.rs:20)"
   }
   ```
   `STD_INDIRECT` covers documented rows read through a std/dirs API instead of `env::var`; it is checked only in the doc-to-code direction.
7. **Documented set**:
   - `CONFIG_REFERENCE.md` `## Environment Variables` section (regex like `md_section`), rows starting `| \``, first-column backticked names `` `([A-Z][A-Z0-9_]*\*?)` ``.
   - `MUX.md` `## Agent Hook Reports` section up to the first `### `, same first-column extraction.
   - The `PAR_TERM_*` wildcard row covers exactly the `CLAP_ENV_RE` set. Check (a) every clap env name appears backticked in `docs/STREAMING.md`, and (b) the wildcard row's `(N variables)` equals `len(clap_set)`.
   - Fail closed (`fail("parsed nothing from …")`) when the CONFIG table, the MUX table, the Rust reads, the TS reads or the clap set is empty.
8. **Diff**:
   - `reads - NOT_OURS - documented - clap_set` gives `env var X read at <file> is not in docs/CONFIG_REFERENCE.md's Environment Variables table`.
   - `config_rows - reads - STD_INDIRECT - {"PAR_TERM_*"}` gives `CONFIG_REFERENCE row X: nothing in production code reads it`.
   - Print `env docs ok: N reads, M documented rows, 40 streamer vars` on success; exit 1 on any problem.
9. **`--self-test`** (copy the scan roots, the four globs and the three docs into a temp dir, as `check_mux_docs.copy_inputs` does):
   - The baseline copy passes.
   - Each drift is reported naming its variable: delete the `DEBUG_LEVEL` row; insert `let _ = std::env::var("PAR_MUX_FAKE_KNOB");` into `crates/par-mux/src/mux/mod.rs`; insert `parseDurationEnv("PAR_MUX_FAKE_MS", 1);` into the omp `.ts`; add a fake `| \`PAR_MUX_GHOST\` | … |` row; insert `std::env::var(unknown_ident)` into `mux/mod.rs` (`unresolvable`).
   - **Exclusion holds**: the same `PAR_MUX_FAKE_KNOB` read appended inside `#[cfg(test)]\nmod enh055 { fn f() { let _ = std::env::var("PAR_MUX_FAKE_KNOB"); } }` is **not** reported, and neither is one in a new `crates/par-mux/tests/enh055.rs`.
   - **Fail closed**: an empty CONFIG table raises `SystemExit` containing `parsed nothing`.
10. **Docs rows.** Add rows to the `CONFIG_REFERENCE.md` table (skipping any DOC-150 already added), grounded in the cited code:
    - `PAR_MUX_CURSOR_TRACE`: attach render client, unset, non-empty and not `0` logs cursor placement (`render/mod.rs:392-398`; see TROUBLESHOOTING.md:234).
    - `PAR_MUX_CONTROL`: session env, not process env. `1` in a session's env grants its panes `PAR_MUX_CONTROL_SOCKET` (`pane.rs:789-806`).
    - `PAR_MUX_OMP_IDLE_DEBOUNCE_MS` (250) and `PAR_MUX_OMP_RETRY_GRACE_MS` (2500): omp extension (`par-mux-omp-agent-state.ts:108-109`).
    - `PAR_MUX_SENDLOG`: pi/omp extensions, `1` writes one `SENDLOG` stderr line per send (`par-mux-agent-state.ts:80-87`).
    - `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `GROK_HOME`, `PI_CODING_AGENT_DIR`, `PI_CONFIG_DIR`: `par-mux` integration installer, where to write the hook/extension instead of `~/.claude` etc. (`integrations/claude.rs:77-82`, `codex.rs:83`, `grok.rs:70`, `extensions.rs:50-66`).
    - Widen the `:829` sentence to "…and the shipped par-mux agent integrations read".
11. **Makefile**:
    ```make
    # ENH-055: every env var production code reads is in the CONFIG_REFERENCE
    # table (or MUX.md's pane env contract / STREAMING.md). Pure python3.
    env-docs-check:
    	python3 scripts/check_env_docs.py
    	python3 scripts/check_env_docs.py --self-test
    ```
    Add it to `.PHONY` (`:4`), add a help line after `:62`, and put it in `checkall` (`:525`) right after `mux-docs-check`.

## Files to touch

- `scripts/check_env_docs.py` (new)
- `Makefile` (`.PHONY`, help, target, `checkall`)
- `docs/CONFIG_REFERENCE.md` (`:829` sentence, new rows)
- `CHANGELOG.md` `[Unreleased]` (one `### Added` bullet naming the gate)

## Verify

Acceptance criteria:
- `PATH="$PWD/.venv/bin:$PATH" make env-docs-check` exits 0 on the finished tree and prints `env docs ok:` plus a self-test line reporting every injected drift and the fail-closed case.
- Seeded violation fails: with only the script (no doc rows) on aaf35ef, `python3 scripts/check_env_docs.py` exits 1 and reports exactly the ten undocumented variables listed in Current state; adding `std::env::var("PAR_MUX_FAKE_KNOB")` to `crates/par-mux/src/mux/mod.rs` on the finished tree exits 1 naming `PAR_MUX_FAKE_KNOB`.
- Test-only reads stay excluded: `PROTO_GOLDEN_UPDATE`, `CARGO_MANIFEST_DIR` and `PAR_TEST_REEXEC` never appear in the output, and the self-test's `#[cfg(test)]` and `tests/` injections are not reported.
- `PATH="$PWD/.venv/bin:$PATH" make checkall-ci` exits 0 with `env-docs-check` in the `make -n checkall` output.

Commands:
```bash
python3 scripts/check_env_docs.py > /tmp/enh055.log 2>&1; echo EXIT=$?          # 0 after the doc rows
python3 scripts/check_env_docs.py --self-test > /tmp/enh055-st.log 2>&1; echo EXIT=$?   # 0
make -n checkall > /tmp/enh055-n.log 2>&1; grep -c check_env_docs /tmp/enh055-n.log     # 2
PATH="$PWD/.venv/bin:$PATH" make checkall-ci > /tmp/enh055-ci.log 2>&1; echo EXIT=$?
```

## Rollback

Revert the script and the Makefile lines. The doc rows are correct on their own and stay. If a regex misfires on a future idiom, add a guarded `EXTRA_READS`/`NOT_OURS` entry with a reason rather than loosening the extraction.

## Dependencies / sequencing

- After DOC-150. If DOC-150 has not landed, this card adds its three rows too, which makes DOC-150 a no-op to close.
- Independent of ENH-056/057. All three touch the `checkall` line of `Makefile`, so if they run in parallel, sequence those single-line edits.
