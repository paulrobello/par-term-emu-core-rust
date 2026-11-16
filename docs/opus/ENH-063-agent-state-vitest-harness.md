# ENH-063 — Vitest harness for the shipped agent-state TypeScript extensions

## Goal

A real test harness for `crates/par-mux/integrations/extensions/par-mux-agent-state.ts`,
`par-mux-omp-agent-state.ts`, and the shared core QA-002 extracts (`agent-state-core.ts`).
These files ship into users' editors, carry the agent-visibility contract (roster
registration, state reporting, release), and today have zero tests — the code-quality audit
found the two forks already drifting in `reportSession`/`sendState`/`releaseAgent` precisely
because nothing catches divergence.

## Current-state context

- Extensions live in `crates/par-mux/integrations/extensions/` and are installed by
  `par-mux --install-skill` / the integrations installer (`crates/par-mux/src/mux/integrations/`).
- No `package.json`/tsconfig under `integrations/` today; the web frontend
  (`web-terminal-frontend/`) has its own Node toolchain (next 16 / react 19), so a separate
  minimal harness is cleaner than sharing it.
- The shared core (post-QA-002) exposes a factory parameterized by agent kind; behavior to
  pin: retry/backoff on failed requests, state-queue draining, session-ref updates, release.

## Implementation steps

1. **Scaffold** `crates/par-mux/integrations/package.json` (private, no publish) with
   `vitest` as a devDependency; pin the version at install time (bun: `bun add -d vitest`).
   Add `integrations/vitest.config.ts` including only `extensions/**/*.test.ts`.
2. **Fake the editor API**: the extensions call an injected host API (survey the actual
   surface the extensions consume — global editor bridge, IPC send functions) and write a
   typed test double `integrations/extensions/test/host.ts` recording calls with
   programmable failures (fail-N-times responder for backoff tests).
3. **Core behavior tests** (`agent-state-core.test.ts`): request succeeds without retry;
   request fails twice then succeeds asserting backoff delays (fake timers) and final state;
   queue drains in order; release clears session refs; cap on queue length.
4. **Per-extension mapping tests**: each extension's field mapping produces the wire shapes
   the daemon's hook-report grammar expects (`crates/par-mux/src/mux/hooks/report.rs` is the
   grammar authority — mirror its accepted tokens).
5. **Wire the gate**: `Makefile` target `test-integrations:` running `cd crates/par-mux/integrations && bun run test`; add to `checkall` after the Python/Rust unit legs
   (cheap, deterministic) and to `checkall-ci`. Guard against absent node_modules with the
   same bootstrap pattern other JS legs use.
6. **CI**: `bun install` step for `integrations/` wherever `web-terminal-frontend` gets its
   install step (ci.yml + the worktree bootstrap docs in CLAUDE.md gain one line).

## Files to touch

`crates/par-mux/integrations/package.json` (new), `vitest.config.ts` (new),
`extensions/test/host.ts` (new), `extensions/agent-state-core.test.ts` (new),
`extensions/par-mux-agent-state.test.ts`, `extensions/par-mux-omp-agent-state.test.ts` (new),
`Makefile` (target + gate legs), `.github/workflows/ci.yml` (install + test leg),
`CLAUDE.md` (bootstrap note), `docs/MUX.md` (one line under development).

## Verification

- `cd crates/par-mux/integrations && bun run test` green
- `PATH="$PWD/.venv/bin:$PATH" make checkall` includes the new leg and passes
- Mutation check: flip an expected token in one test to prove the harness fails (then fix)

## Rollback

All new files plus two gate-leg additions; revert the commit. No shipped-code change — the
extensions themselves are untouched except whatever QA-002 already did.
