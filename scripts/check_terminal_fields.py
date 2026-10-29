#!/usr/bin/env python3
"""ARC-001: a ratchet on the number of fields on the core `Terminal` struct.

`Terminal` is the integration point for every feature and the repo's widest
blast radius. The sub-struct program groups its state into cohesive
`pub(crate)` structs in `terminal/state.rs`; this gate stops new state from
landing directly on `Terminal` instead. It counts the named fields of
`pub struct Terminal { .. }` in `crates/par-term-emu-core/src/terminal/mod.rs`
and fails when the count exceeds BASELINE.

Policy: new feature state goes into an existing or new sub-struct in
`terminal/state.rs` (or behind the `EventBroker`/observer seam, the way
`trigger::TriggerState` does), never as a new top-level field. When an
extraction removes fields, lower BASELINE in the same commit (the gate prints
the new count) so the space cannot be refilled.

`--self-test` checks a synthetic struct for growth, shrink-without-ratchet,
and a missing struct, then requires the real file to pass.
"""

from __future__ import annotations

import argparse
import re
import tempfile
from pathlib import Path

from _gate_common import ROOT, expect_failure, fail

TERMINAL_REL = "crates/par-term-emu-core/src/terminal/mod.rs"

# The recorded ceiling. Lower it when fields are extracted.
BASELINE = 52

STRUCT_START_RE = re.compile(r"^pub struct Terminal \{\s*$", re.MULTILINE)
FIELD_RE = re.compile(r"^    (?:pub(?:\([a-z]+\))? )?([a-z_][a-z0-9_]*):", re.MULTILINE)


def count_fields(path: Path) -> int:
    """Named fields of `pub struct Terminal` in `path`."""
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as exc:
        fail(f"cannot read {path}: {exc}")
    start = STRUCT_START_RE.search(text)
    if start is None:
        fail(f"parsed nothing: {path} has no `pub struct Terminal {{`")
    end = text.find("\n}\n", start.end())
    if end == -1:
        fail(f"parsed nothing: unterminated `pub struct Terminal` in {path}")
    return len(FIELD_RE.findall(text[start.end() : end]))


def check(path: Path, baseline: int) -> None:
    n = count_fields(path)
    if n > baseline:
        fail(
            f"Terminal has {n} fields, above the frozen baseline of {baseline}. "
            "Put new state in a sub-struct in terminal/state.rs (or behind the "
            "EventBroker seam) instead of a new Terminal field (ARC-001)."
        )
    if n < baseline:
        fail(
            f"Terminal shrank to {n} fields (baseline {baseline}): lower BASELINE "
            f"in scripts/check_terminal_fields.py to {n} so the ratchet holds."
        )
    print(f"terminal fields ok: {n} (at baseline)")


def self_test() -> None:
    synthetic = (
        "pub struct Other {\n    pub(crate) x: u8,\n}\n"
        "pub struct Terminal {\n"
        "    /// doc\n"
        "    #[doc(hidden)]\n"
        "    pub grid: Grid,\n"
        "    pub(crate) cursor: Cursor,\n"
        "    // === section ===\n"
        "    pending_wrap: bool,\n"
        "}\n"
        "impl Terminal {\n    fn f(&self) {\n        let y: u8 = 0;\n    }\n}\n"
    )
    with tempfile.TemporaryDirectory(prefix="arc001-fields-selftest-") as tmp:
        path = Path(tmp) / "mod.rs"
        path.write_text(synthetic, encoding="utf-8")
        check(path, 3)  # Other's field and fn locals are not counted.

        path.write_text(
            synthetic.replace("    pending_wrap", "    extra: u8,\n    pending_wrap"),
            encoding="utf-8",
        )
        expect_failure(lambda: check(path, 3), "above the frozen baseline")

        path.write_text(
            synthetic.replace("    pending_wrap: bool,\n", ""), encoding="utf-8"
        )
        expect_failure(lambda: check(path, 3), "lower BASELINE")

        path.write_text("pub struct Other {}\n", encoding="utf-8")
        expect_failure(lambda: check(path, 3), "parsed nothing")

    check(ROOT / TERMINAL_REL, BASELINE)
    print("self-test ok: growth, unratcheted shrink, and a missing struct reported")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root", type=Path, default=ROOT, help="repository root to check"
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="check a synthetic struct for each failure mode, then the real file",
    )
    args = parser.parse_args()
    if args.self_test:
        self_test()
    else:
        check(args.root / TERMINAL_REL, BASELINE)


if __name__ == "__main__":
    main()
