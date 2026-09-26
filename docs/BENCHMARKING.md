# Benchmarking

How to run the criterion throughput benches, why comparisons must interleave
two binaries, and how the scheduled benchmark regression gate works (ENH-016).

## Running benches locally

```bash
make bench         # cargo bench --no-default-features --features rust-only
```

The benches live in `benches/terminal_throughput.rs` and drive the real
`Terminal::process` pipeline (VTE parser, sequence dispatch, grid writes,
scrolling, Sixel/Kitty graphics), reporting throughput as MB/s. They build
without the Python feature, so no `make dev` / maturin step is needed and they
never run as part of `make checkall`.

## The 2x rule: only interleaved A/B comparisons are valid

**Absolute criterion numbers on one machine swing up to ~2x with machine
state** — the same unchanged binary measured 2.94–5.30 MiB/s on `plain_ascii`
within a single evening (runs right after a long compile read ~40–50% low).
Comparing a run today against a run from another session compares machine
states, not code.

To compare code, interleave the two binaries in one session — old, new, old,
new — and compare within pairs. That shape is reproducible to ±1%; the full
measurements live in `docs/fable/BENCH-BASELINE-2026-09.md`.

For quick manual A/B work:

```bash
cargo bench --no-default-features --features rust-only -- --save-baseline my-change
# ... after further changes ...
cargo bench --no-default-features --features rust-only -- --baseline my-change
```

(`--save-baseline` needs the named `--bench terminal_throughput` target when
cargo would also run the lib's unit-test pass. A criterion binary invoked
directly needs `--bench`, or it runs one test iteration and exits without
measuring.)

## The scheduled bench gate

`.github/workflows/bench.yml` runs weekly (Monday 09:17 UTC, plus manual
`workflow_dispatch`) and is driven by `scripts/bench_gate.sh`:

1. Builds the bench binary for `HEAD` and for the `bench-baseline` git tag in
   one shared `target/` dir, copying each binary out.
2. Runs them in **alternating interleaved rounds** (`BENCH_ROUNDS`, default 3;
   round order flips each round to cancel ordering bias), each round writing
   to its own `CRITERION_HOME`.
3. `scripts/bench_compare.py` aggregates per-bench means across rounds (median
   of round means, widest confidence envelope) and flags any bench where HEAD
   is both >`BENCH_THRESHOLD` (default 1.2 = 20%) slower than baseline **and**
   the confidence intervals do not overlap — the overlap check keeps runner
   noise from crying wolf.
4. The verdict table lands in the job summary. **Verdicts are warning-level
   for now**: the job stays green while noise on GitHub-hosted runners is
   characterized. Promote to failing by setting `BENCH_FAIL_ON_REGRESSION=1`
   in the workflow once the threshold is tuned against real data.
5. On a clean compare from the default branch, the `bench-baseline` tag is
   moved to `HEAD` via a **tag-only force push** (branch history is never
   rewritten). The first run with no tag bootstraps it.

To demo or debug the gate, dispatch it manually:

```bash
gh workflow run bench.yml            # on main: full compare + tag rotation
gh workflow run bench.yml --ref <branch>   # branch run: compare, no rotation
```

A synthetic-slowdown check (verify the gate actually flags): add e.g. a
`std::thread::sleep` into one bench function on a scratch branch, dispatch
`--ref scratch-branch`, confirm the bench is flagged `REGRESSION` in the
summary, then delete the branch.
