# VTE Throughput Benchmark Baseline — 2026-09

Supersedes `BENCH-BASELINE-2026-08.md` (kept for history). Numbers below are
the post-ENH-010 state: ASCII fast lane in `write_char`/`print`, bit-per-row
dirty tracking, guarded scroll debug logs, and bounds-free row moves.

## Environment

| | |
|---|---|
| Machine | Apple M4 Max, 128 GB |
| rustc | 1.98.1 (48a229cea 2026-09-01) |
| Profile | cargo bench default (release, opt-level 3, LTO, codegen-units 1) |
| Features | `--no-default-features --features rust-only` |

## Measurement methodology — read this first

**Absolute numbers on this machine swing up to ~2× with machine state.** The
same unchanged binary measured 2.94–5.30 MiB/s on `plain_ascii` within one
evening, depending on thermal/load history (runs right after a long compile
read ~40–50% low). Two rules:

1. **Never compare a run today against a run from another session** (the
   2026-08 baseline's 3.44 `plain_ascii` and 667.82 `kitty_decode` are not
   reproducible; treat them as depressed/stale).
2. **Interleave A/B binaries** — run old, new, old, new in one session and
   compare within pairs. The numbers below were taken that way and were
   reproducible to ±1% across alternations.

## ENH-010 A/B (2026-09-23, interleaved, median)

| Benchmark | Pre-change | Post-change | Change |
|---|---:|---:|---:|
| `plain_ascii/1MiB_lorem_80x24` | 199–201 ms (≈5.2 MiB/s) | 198–200 ms (≈5.3 MiB/s) | +0.5–1% |
| `scroll/1MiB_lines_80x24_scrollback10k` | 352–356 ms (≈3.0 MiB/s) | 209–211 ms (≈5.0 MiB/s) | **1.68× faster** |
| `sgr_heavy/256KiB_truecolor_80x24` | 44.3–44.6 ms (≈5.6 MiB/s) | 17.9–18.0 ms (≈14.0 MiB/s) | **2.49× faster** |
| `unicode_wide/256KiB_cjk_emoji_zwj_80x24` | ≈53.6 ms (≈4.7 MiB/s) | ≈37.1 ms (≈6.7 MiB/s) | **1.44× faster** |
| `cursor_addressing/512KiB_fullscreen_repaint_80x24` | ≈28.0 ms (≈17.9 MiB/s) | ≈5.6 ms (≈89 MiB/s) | **5.0× faster** |
| `sixel_decode/8x_128x96_16color_dcs` | ≈4.03 ms | ≈4.00 ms | unchanged |
| `kitty_decode/4x_96x96_rgb_apc` | ≈1.95 ms | ≈1.95 ms | unchanged |

Why `plain_ascii` barely moved: a symbolicated `sample` profile shows
`scroll_region_up` at ~78% of `plain_ascii` runtime, dominated by
`SmallVec<[char;4]>::clone` inside the per-cell row move (~50% of total
runtime) — every line feed at the scroll bottom relocates all 23 rows cell by
cell, and `Cell` is not `Copy` while it carries `combining: SmallVec`. The
write-heavy groups (no bottom scroll) gain the full fast-lane + bitset effect;
the newline-dense groups stay scroll-bound until that structural cost is
addressed (candidate levers: move `combining` out of `Cell` into a side table,
row-ring rotation, or a scan-guarded bulk move that skips the element-wise
clone when no cell in the range has a spilled (heap) SmallVec).

## Comparing after a change

```bash
cargo bench --bench terminal_throughput --no-default-features --features rust-only -- --save-baseline mine
# ...after changes...
cargo bench --bench terminal_throughput --no-default-features --features rust-only -- --baseline mine
```

(The `--bench terminal_throughput` target selector is required — a bare
`cargo bench -- --save-baseline` fails in the lib-unittest bench pass.)

Treat changes within ±3% as noise. For decisions that matter, interleave the
two binaries as above rather than trusting `--baseline` across sessions.
