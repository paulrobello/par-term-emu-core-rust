#!/usr/bin/env python3
"""ARC-001: a ratchet on the flat Python API surface of `Terminal` and
`PtyTerminal`.

The binding layer mirrors the Rust `Terminal` 1:1 onto two god-classes, and
every new flat method fans out across five artifacts (Rust impl, themed
binding, stub, API_REFERENCE.md, Python tests). This gate freezes that
growth: it counts each class's methods in the generated
`python/par_term_emu_core_rust/_native.pyi` and fails when a count exceeds
its recorded baseline.

Policy: new API areas ship as separate pyclasses with their own cohesive
surface, the way `ScreenshotConfig`, `StreamingConfig`, `WidthConfig`,
`Macro`, and the `types/` data classes already do, and are reached from
`Terminal` through at most one accessor. The surface may only hold or
shrink. When a change removes methods, lower the baseline in the same
commit (the gate prints the new count) so the space cannot be refilled.
Raising a baseline is an explicit, reviewed decision, never a side effect.

What is counted: the unique non-dunder `def` names in the class body of
the stub (`__init__`, `__str__`, `__enter__`, ... are protocol plumbing,
not API area). The generator emits each property as a plain getter `def`,
and a name defined twice (getter plus setter overload) counts once. The
stub is the source because it is what `make stub-check` keeps in sync with
the built module, and this gate runs without a build or venv.

`--self-test` checks a synthetic stub for growth, shrink-without-ratchet,
and a missing class, then requires the real stub to pass.
"""

from __future__ import annotations

import argparse
import ast
import tempfile
from pathlib import Path

from _gate_common import ROOT, expect_failure, fail

STUB_REL = "python/par_term_emu_core_rust/_native.pyi"

# The recorded ceilings. Lower these when methods are removed.
BASELINE: dict[str, int] = {
    "Terminal": 389,
    "PtyTerminal": 206,
}


def count_methods(stub: Path, classes: list[str]) -> dict[str, int]:
    """Unique non-dunder method names per class in `stub`."""
    try:
        tree = ast.parse(stub.read_text(encoding="utf-8"))
    except (OSError, SyntaxError) as exc:
        fail(f"cannot parse {stub}: {exc}")
    counts: dict[str, int] = {}
    for node in tree.body:
        if isinstance(node, ast.ClassDef) and node.name in classes:
            names = {
                n.name
                for n in node.body
                if isinstance(n, ast.FunctionDef | ast.AsyncFunctionDef)
                and not (n.name.startswith("__") and n.name.endswith("__"))
            }
            counts[node.name] = len(names)
    missing = [c for c in classes if c not in counts]
    if missing:
        fail(f"parsed nothing: {stub} has no class {', '.join(missing)}")
    return counts


def check(stub: Path, baseline: dict[str, int]) -> None:
    counts = count_methods(stub, list(baseline))
    problems: list[str] = []
    for cls, ceiling in baseline.items():
        n = counts[cls]
        if n > ceiling:
            problems.append(
                f"{cls} has {n} methods, above the frozen baseline of {ceiling}. "
                "Ship new API areas as a separate pyclass (see ScreenshotConfig/"
                "StreamingConfig) instead of adding flat methods (ARC-001)."
            )
        elif n < ceiling:
            problems.append(
                f"{cls} shrank to {n} methods (baseline {ceiling}): lower "
                f"BASELINE[{cls!r}] in scripts/check_api_surface.py to {n} so the "
                "ratchet holds."
            )
    if problems:
        for p in problems:
            print(p)
        raise SystemExit(1)
    summary = ", ".join(f"{c} {counts[c]}" for c in baseline)
    print(f"api surface ok: {summary} (at baseline)")


def self_test() -> None:
    synthetic = (
        "class Terminal:\n"
        "    def __init__(self) -> None: ...\n"
        "    def a(self) -> None: ...\n"
        "    def b(self) -> None: ...\n"
        "    def b(self, v: int) -> None: ...\n"
        "class PtyTerminal:\n"
        "    def __enter__(self) -> None: ...\n"
        "    def a(self) -> None: ...\n"
    )
    base = {"Terminal": 2, "PtyTerminal": 1}
    with tempfile.TemporaryDirectory(prefix="arc001-selftest-") as tmp:
        stub = Path(tmp) / "_native.pyi"
        stub.write_text(synthetic, encoding="utf-8")
        check(stub, base)  # Dunders ignored; a repeated name counts once.

        stub.write_text(synthetic + "    def c(self) -> None: ...\n", encoding="utf-8")
        expect_failure(lambda: check(stub, base), "above the frozen baseline")

        stub.write_text(
            synthetic.replace("    def a(self) -> None: ...\n", "", 1), encoding="utf-8"
        )
        expect_failure(lambda: check(stub, base), "lower BASELINE")

        stub.write_text(
            "class Terminal:\n    def a(self) -> None: ...\n", encoding="utf-8"
        )
        expect_failure(lambda: check(stub, base), "parsed nothing")

    check(ROOT / STUB_REL, BASELINE)
    print("self-test ok: growth, unratcheted shrink, and a missing class reported")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root", type=Path, default=ROOT, help="repository root to check"
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="check a synthetic stub for each failure mode, then the real stub",
    )
    args = parser.parse_args()
    if args.self_test:
        self_test()
    else:
        check(args.root / STUB_REL, BASELINE)


if __name__ == "__main__":
    main()
