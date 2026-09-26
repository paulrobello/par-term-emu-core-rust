#!/usr/bin/env python3
"""Compare interleaved criterion A/B benchmark rounds (ENH-016).

Reads the criterion ``new/estimates.json`` files produced by each round of
``scripts/bench_gate.sh`` for the baseline ("base") and candidate ("head")
binaries, aggregates per-bench means across rounds (median of round means,
widest confidence envelope), and flags benches whose head estimate is both
slower than ``--threshold`` x the baseline mean and non-overlapping in
confidence interval. The interleave-then-aggregate shape exists because
absolute criterion numbers swing up to ~2x with machine state; only
within-round-pair comparisons are trustworthy (docs/fable/BENCH-BASELINE-2026-09.md).

Verdicts are advisory by default (exit 0 even when benches are flagged);
pass ``--fail-on-regression`` to exit non-zero instead. Output is a markdown
table on stdout, optionally mirrored into a GitHub step-summary file, with a
final ``REGRESSIONS=<n>`` machine line.
"""

from __future__ import annotations

import argparse
import json
import statistics
import sys
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Aggregate:
    """Per-bench aggregation across interleaved rounds."""

    point_ms: float
    ci_lower_ms: float
    ci_upper_ms: float
    rounds: int


@dataclass(frozen=True)
class Verdict:
    """Per-bench comparison outcome."""

    bench: str
    base: Aggregate
    head: Aggregate
    ratio: float
    flagged: bool
    note: str = ""


def _load_estimate(path: Path) -> tuple[float, float, float]:
    """Return (mean_ms, ci_lower_ms, ci_upper_ms) from one estimates.json."""
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        mean = data["mean"]
        point = float(mean["point_estimate"])
        ci = mean["confidence_interval"]
        lower = float(ci["lower_bound"])
        upper = float(ci["upper_bound"])
    except (OSError, ValueError, KeyError, TypeError) as exc:
        raise SystemExit(f"error: unreadable criterion estimate {path}: {exc}") from exc
    # Criterion stores durations in nanoseconds; report milliseconds.
    return point / 1e6, lower / 1e6, upper / 1e6


def load_aggregates(root: Path) -> dict[str, Aggregate]:
    """Aggregate every bench found under ``root``'s round directories.

    ``root`` is the per-binary criterion root used by bench_gate.sh; each
    child directory (``r1``, ``r2``, ...) is one round's CRITERION_HOME.
    """
    aggregates: dict[str, list[tuple[float, float, float]]] = {}
    round_dirs = (
        sorted(d for d in root.iterdir() if d.is_dir()) if root.is_dir() else []
    )
    if not round_dirs:
        raise SystemExit(f"error: no round directories under {root}")
    for round_dir in round_dirs:
        for estimate_path in sorted(round_dir.glob("**/new/estimates.json")):
            bench = estimate_path.relative_to(round_dir).parent.parent.as_posix()
            aggregates.setdefault(bench, []).append(_load_estimate(estimate_path))
    result: dict[str, Aggregate] = {}
    for bench, samples in aggregates.items():
        points = [s[0] for s in samples]
        result[bench] = Aggregate(
            point_ms=statistics.median(points),
            ci_lower_ms=min(s[1] for s in samples),
            ci_upper_ms=max(s[2] for s in samples),
            rounds=len(samples),
        )
    return result


def compare(
    base: dict[str, Aggregate], head: dict[str, Aggregate], threshold: float
) -> tuple[list[Verdict], list[str], list[str]]:
    """Return (verdicts for benches present in both, new-in-head, missing-from-head)."""
    verdicts: list[Verdict] = []
    for bench in sorted(set(base) & set(head)):
        b, h = base[bench], head[bench]
        ratio = h.point_ms / b.point_ms if b.point_ms > 0 else float("inf")
        non_overlapping = h.ci_lower_ms > b.ci_upper_ms
        flagged = ratio > threshold and non_overlapping
        if flagged:
            note = "REGRESSION"
        elif ratio > threshold:
            note = "slower, CI overlap (noise-suspect)"
        else:
            note = "ok"
        verdicts.append(
            Verdict(
                bench=bench, base=b, head=h, ratio=ratio, flagged=flagged, note=note
            )
        )
    new = sorted(set(head) - set(base))
    removed = sorted(set(base) - set(head))
    return verdicts, new, removed


def render(verdicts: list[Verdict], new: list[str], removed: list[str]) -> str:
    """Render the verdict table plus new/removed bench lists as markdown."""
    lines = [
        "| Benchmark | Base ms | Head ms | Ratio | Verdict |",
        "|---|---:|---:|---:|---|",
    ]
    for v in verdicts:
        lines.append(
            f"| `{v.bench}` | {v.base.point_ms:.3f} | {v.head.point_ms:.3f} "
            f"| {v.ratio:.2f}x | {v.note} |"
        )
    if new:
        lines.append(
            f"\nNew in head (no baseline to compare): {', '.join(f'`{b}`' for b in new)}"
        )
    if removed:
        lines.append(
            f"\nMissing from head (present in baseline): {', '.join(f'`{b}`' for b in removed)}"
        )
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--base",
        type=Path,
        required=True,
        help="criterion root for the baseline binary rounds",
    )
    parser.add_argument(
        "--head",
        type=Path,
        required=True,
        help="criterion root for the head binary rounds",
    )
    parser.add_argument(
        "--threshold",
        type=float,
        default=1.2,
        help="flag head/base mean ratio above this (default 1.2 = 20%% slower)",
    )
    parser.add_argument(
        "--fail-on-regression",
        action="store_true",
        help="exit 1 when any bench is flagged",
    )
    parser.add_argument(
        "--summary",
        type=Path,
        help="append the table to this file (e.g. $GITHUB_STEP_SUMMARY)",
    )
    parser.add_argument(
        "--github-output",
        type=Path,
        help="write 'regressions=N' to this file (e.g. $GITHUB_OUTPUT)",
    )
    args = parser.parse_args(argv)

    base = load_aggregates(args.base)
    head = load_aggregates(args.head)
    verdicts, new, removed = compare(base, head, args.threshold)
    regressions = sum(1 for v in verdicts if v.flagged)
    table = render(verdicts, new, removed)

    print(table)
    print(f"\nREGRESSIONS={regressions}")
    if args.summary:
        with args.summary.open("a", encoding="utf-8") as fh:
            fh.write(table + "\n")
    if args.github_output:
        with args.github_output.open("a", encoding="utf-8") as fh:
            fh.write(f"regressions={regressions}\n")
    if regressions and args.fail_on_regression:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
