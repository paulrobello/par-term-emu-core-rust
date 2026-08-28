# VTE Throughput Benchmark Baseline — 2026-08

Committed reference numbers for `benches/terminal_throughput.rs` (ENH-007).
Every benchmark drives the real `Terminal::process` pipeline — APC pre-filter,
vte parser, sequence dispatch, grid writes, scrolling, and graphics ingestion —
on an 80×24 terminal with deterministically generated payloads. Criterion
reports throughput via `Throughput::Bytes`, so values below are MiB/s of raw
input consumed.

## Environment

| | |
|---|---|
| Machine | Apple M4 Max, 128 GB |
| rustc | 1.98.0 (88d9e12ae 2026-08-18) |
| Profile | cargo bench default (release, opt-level 3, LTO, codegen-units 1) |
| Features | `--no-default-features --features rust-only` |
| Commit | ENH-007 bench suite introduction |

## Baseline (2026-08-28, criterion 0.7.0, median of 100 samples)

| Benchmark | Payload | Median time | Throughput |
|---|---|---:|---:|
| `plain_ascii/1MiB_lorem_80x24` | 1 MiB lorem-ipsum lines | 290.66 ms | **3.44 MiB/s** |
| `sgr_heavy/256KiB_truecolor_80x24` | 256 KiB truecolor SGR + words | 21.44 ms | **11.66 MiB/s** |
| `unicode_wide/256KiB_cjk_emoji_zwj_80x24` | 256 KiB CJK/emoji/ZWJ | 46.94 ms | **5.33 MiB/s** |
| `scroll/1MiB_lines_80x24_scrollback10k` | 1 MiB lines, 10k scrollback | 294.04 ms | **3.40 MiB/s** |
| `cursor_addressing/512KiB_fullscreen_repaint_80x24` | 512 KiB CUP + line writes | 29.49 ms | **17.00 MiB/s** |
| `sixel_decode/8x_128x96_16color_dcs` | 8× 128×96 16-color sixel | 3.61 ms | **71.61 MiB/s** |
| `kitty_decode/4x_96x96_rgb_apc` | 4× 96×96 RGB (`f=24`) | 210.71 µs | **667.82 MiB/s** |

## Reading the numbers

- The newline-heavy groups (`plain_ascii`, `scroll`) are the slowest per byte:
  a line feed at the scroll boundary costs a full grid-window scroll, and these
  payloads emit one every ~80 bytes. `scroll` ≈ `plain_ascii` confirms
  scrollback insertion itself is not an additional cost once the region scrolls.
- `sgr_heavy` runs 3.4× faster per byte than `plain_ascii` despite parsing an
  escape per word — truecolor SGR dispatch is cheap next to the scroll churn
  the newline-dense payloads trigger.
- `write_char` (cyclomatic complexity ~90, audit QA-006's refactor target) is
  the shared hot path behind the text groups; these three text numbers are the
  regression guard for that refactor.
- `kitty_decode`'s high figure reflects its payload being mostly base64 of
  uncompressed RGB — the decode path is close to memcpy-bound; it guards
  against parser regressions, not absolute speed.

## Comparing after a change

```bash
make bench                                        # current numbers
cargo bench --no-default-features --features rust-only -- --save-baseline mine
# ...after changes...
cargo bench --no-default-features --features rust-only -- --baseline mine
```

Treat changes within ±3% as noise (criterion flags significance itself; on
this machine the text groups showed 3–6 outliers per 100 samples, all mild).
When a hot-path refactor lands, refresh this file in the same commit so the
committed baseline tracks HEAD.
