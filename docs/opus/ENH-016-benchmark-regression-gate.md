# ENH-016 — Benchmark Regression Gate (nightly criterion baselines)

> Filed from the 2026-09-26 /opus-audit enhancement pass. Board card: `[ENH-016]`
> (priority high, estimate M). Consumer: `/enhancement-all` / `/enhancement-next`.

## Goal

Catch throughput regressions automatically: the repo ships `benches/terminal_throughput.rs`
(criterion) but nothing tracks its numbers over time, so performance decay surfaces only by
accident. Add a nightly job that benchmarks `main`, compares against the stored baseline, and
flags (or fails on) statistically significant slowdowns.

## Current state

- `benches/terminal_throughput.rs` defines criterion benches (`bench_plain_ascii`,
  `bench_sgr_heavy`, `bench_unicode_wide`, `bench_scroll`, `bench_cursor_addressing`,
  `bench_sixel_decode`, `bench_kitty_decode`) but no CI or scheduled job runs them.
- CI has no performance awareness at all; `.github/workflows/` is dispatch-only today
  (see audit ARC-023 — that finding adds push triggers; this card adds the *scheduled* bench job
  and can build on it).
- Known methodology constraint (project memory, measured 2026): bench numbers swing ~2x with
  machine state. Any comparison must interleave A/B runs of two binaries rather than comparing
  numbers across separate runs.

## Implementation

1. **Baseline artifact**: add a scheduled GitHub Actions workflow
   `.github/workflows/bench.yml`: `on: schedule: [cron "17 9 * * 1"]` (weekly, off the :00 mark)
   plus `workflow_dispatch`. Runner: `ubuntu-latest` (self-hosted would be better for noise but
   adds ops burden — start with the GH runner and a generous threshold).
2. **Interleaved A/B compare job**: the job checks out HEAD and the last benchmarked commit
   (stored as a tag, e.g. `bench-baseline`), builds both release binaries of the bench target,
   and runs criterion with `--save-baseline` for one and `--baseline` for the other,
   **interleaving** the two runs (alternating A/B iterations) to cancel machine-state drift.
   Simplest robust shape: run each bench binary N=5 times alternating, collect the criterion
   `estimates.json`, and compare mean+CI of HEAD vs baseline per bench fn.
3. **Verdict script**: a small script (`scripts/bench_compare.py` or inline in the workflow)
   parses the criterion output dirs and flags any bench whose HEAD mean is >20% slower than the
   baseline mean with non-overlapping confidence intervals (tune the threshold after two runs of
   real data; 20% is a start against runner noise). Output: a job summary table (bench | baseline
   | HEAD | delta | verdict) and a non-blocking failure (start as warning-level; promote to
   failing once noise is characterized).
4. **Baseline rotation**: on a green compare, move the `bench-baseline` tag to HEAD
   (`git tag -f bench-baseline && git push -f origin bench-baseline` — needs a workflow
   `contents: write` permission; document the force-push is tag-only).
5. **Docs**: one section in `docs/BUILDING.md` (or a new `docs/BENCHMARKING.md`) covering how to
   run benches locally, the interleaving requirement, and how the nightly gate works.

## Files to touch

- `.github/workflows/bench.yml` (new)
- `scripts/bench_compare.py` (new, ~100 lines)
- `docs/BENCHMARKING.md` or a BUILDING.md section (new/edited)
- `benches/terminal_throughput.rs` — no changes expected; add `[[bench]] harness = false` only if
  criterion setup requires it (it already uses criterion)

## Verify (acceptance criteria on the card)

1. Baseline save + compare workflow exists and runs on its schedule; the comparison interleaves
   A/B runs (visible in the workflow logs).
2. A deliberate synthetic slowdown (e.g. a `std::hint::black_box` + sleep injected into a copy of
   a bench target on a scratch branch) is flagged by the comparison — demonstrated once in a
   manual `workflow_dispatch` run, then reverted.
3. `make checkall` green (the workflow and script don't touch library code, but the repo gate
   still runs).

## Rollback

Delete `bench.yml` and the tag; no library code depends on it.
