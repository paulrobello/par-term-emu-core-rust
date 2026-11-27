#!/usr/bin/env python3
"""ARC-012: a ratchet on the number of variants of the core `TerminalEvent` enum.

`TerminalEvent` is the event surface every feature appends to: a new
terminal feature adds a variant to it plus a branch in `event_fields.rs`
and, when Python-visible, `_native.pyi`, all in one commit. The surface
freeze gates (`api-surface-check`, `terminal-fields-check`) do not cover the
enum, so nothing recorded how wide it had grown. This gate gives the enum
the same shrink-only ratchet the `Terminal` struct has: it counts the
variants of `pub enum TerminalEvent` in
`crates/par-term-emu-core/src/terminal/event.rs` and fails when the count
exceeds BASELINE.

Policy: a variant is the right shape when a genuinely new observable
terminal condition appears; growing past BASELINE is a reviewed decision —
raise BASELINE in the same commit with a comment on the feature, mirroring
how `terminal-fields-check` treats field extractions. When variants are
removed, lower BASELINE in the same commit so the space cannot be refilled.

`--self-test` checks a synthetic enum for growth, shrink-without-ratchet,
and a missing enum, then requires the real file to pass.
"""

from __future__ import annotations

import argparse
import re
import tempfile
from pathlib import Path

from _gate_common import ROOT, expect_failure, fail

EVENT_REL = "crates/par-term-emu-core/src/terminal/event.rs"

# The recorded ceiling. Raise only as a reviewed decision; lower when variants
# are removed.
BASELINE = 28

ENUM_START_RE = re.compile(r"^pub enum TerminalEvent \{\s*$", re.MULTILINE)
VARIANT_RE = re.compile(r"^    ([A-Z][A-Za-z0-9_]*)\b", re.MULTILINE)


def count_variants(path: Path) -> int:
    """Variants of `pub enum TerminalEvent` in `path`."""
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as exc:
        fail(f"cannot read {path}: {exc}")
    start = ENUM_START_RE.search(text)
    if start is None:
        fail(f"parsed nothing: {path} has no `pub enum TerminalEvent {{`")
    end = text.find("\n}\n", start.end())
    if end == -1:
        fail(f"parsed nothing: unterminated `pub enum TerminalEvent` in {path}")
    return len(VARIANT_RE.findall(text[start.end() : end]))


def check(path: Path, baseline: int) -> None:
    n = count_variants(path)
    if n > baseline:
        fail(
            f"TerminalEvent has {n} variants (baseline {baseline}): a new "
            f"variant is a reviewed ARC-012 decision — raise BASELINE in "
            f"scripts/check_terminal_event.py in the same commit"
        )
    if n < baseline:
        fail(
            f"TerminalEvent shrank to {n} variants (baseline {baseline}): lower BASELINE "
            f"in scripts/check_terminal_event.py to {n} so the space cannot be refilled"
        )


def self_test() -> None:
    """Prove the gate detects growth, shrink, and a missing enum."""
    enum_tpl = "pub enum TerminalEvent {{\n{variants}\n}}\n"
    variants = "\n".join(f"    V{i}(u32)," for i in range(3))

    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "event.rs"
        path.write_text(enum_tpl.format(variants=variants), encoding="utf-8")

        check(path, 3)  # exact baseline passes

        expect_failure(lambda: check(path, 2), "raise BASELINE")
        expect_failure(lambda: check(path, 4), "lower BASELINE")

        path.write_text("pub struct Other;\n", encoding="utf-8")
        expect_failure(lambda: check(path, 3), "no `pub enum TerminalEvent")

    check(ROOT / EVENT_REL, BASELINE)


def main() -> None:
    parser = argparse.ArgumentParser(
        description="ARC-012 TerminalEvent variant-count ratchet"
    )
    parser.add_argument(
        "--self-test", action="store_true", help="run the gate's own regression cases"
    )
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return
    check(ROOT / EVENT_REL, BASELINE)
    n = count_variants(ROOT / EVENT_REL)
    print(f"terminal-event ok: {n} variants match the ARC-012 baseline")


if __name__ == "__main__":
    main()
