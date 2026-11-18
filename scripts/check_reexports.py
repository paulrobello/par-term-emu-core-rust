#!/usr/bin/env python3
"""ARC-006: the root crate re-exports every public `par-term-emu-core` module
at its historical `par_term_emu_core_rust::<module>` path.

`src/lib.rs` keeps those paths alive with a hand-maintained
`pub use par_term_emu_core::{...}` list. A module added to the member's
`lib.rs` does not appear at the historical path until someone adds it to that
list, and nothing noticed when they did not. This gate parses both files and
fails when a public member module has no root re-export.

What is compared: the `pub mod <name>;` declarations in
`crates/par-term-emu-core/src/lib.rs` (every `cfg` variant counts as present,
since feature gating is mirrored by hand) against the names the root brings
in through `pub use par_term_emu_core::...` (plain names, `{a, b}` groups, and
`#[cfg]`-gated single uses all count).

Deliberate omissions live in `NOT_REEXPORTED` with the reason, so leaving a
module off the historical surface is an explicit, reviewed decision.

`--self-test` injects each failure mode into a synthetic tree, then requires
the real tree to pass.
"""

from __future__ import annotations

import argparse
import re
import tempfile
from pathlib import Path

from _gate_common import ROOT, expect_failure, fail

MEMBER_LIB_REL = "crates/par-term-emu-core/src/lib.rs"
ROOT_LIB_REL = "src/lib.rs"

# Member modules the root intentionally does not re-export, with the reason.
NOT_REEXPORTED: dict[str, str] = {
    "otel": "internal OTLP plumbing; the root exposes its own `otel` feature, not the module",
    "self_update": "binary-only updater used by the streamer and par-mux binaries",
}

MEMBER_MOD_RE = re.compile(r"^\s*pub\s+mod\s+(\w+)\s*;", re.MULTILINE)
USE_RE = re.compile(
    r"pub\s+use\s+par_term_emu_core::(\{[^}]*\}|\w+)\s*;", re.MULTILINE | re.DOTALL
)
NAME_RE = re.compile(r"\w+")


def strip_comments(text: str) -> str:
    """Drop `//` comments so a commented-out re-export does not count."""
    return re.sub(r"//[^\n]*", "", text)


def member_modules(root: Path) -> set[str]:
    path = root / MEMBER_LIB_REL
    try:
        text = strip_comments(path.read_text(encoding="utf-8"))
    except OSError as exc:
        fail(f"cannot read {path}: {exc}")
    mods = set(MEMBER_MOD_RE.findall(text))
    if not mods:
        fail(f"parsed nothing: no `pub mod` declarations in {MEMBER_LIB_REL}")
    return mods


def root_reexports(root: Path) -> set[str]:
    path = root / ROOT_LIB_REL
    try:
        text = strip_comments(path.read_text(encoding="utf-8"))
    except OSError as exc:
        fail(f"cannot read {path}: {exc}")
    names: set[str] = set()
    for group in USE_RE.findall(text):
        names.update(NAME_RE.findall(group))
    if not names:
        fail(f"parsed nothing: no `pub use par_term_emu_core::...` in {ROOT_LIB_REL}")
    return names


def check(root: Path, allowed: dict[str, str] = NOT_REEXPORTED) -> None:
    members = member_modules(root)
    reexported = root_reexports(root)
    missing = sorted(members - reexported - set(allowed))
    stale_allow = sorted(m for m in allowed if m not in members or m in reexported)
    if missing or stale_allow:
        for mod in missing:
            print(
                f"member module `{mod}` has no root re-export: add it to the "
                f"`pub use par_term_emu_core::{{...}}` list in {ROOT_LIB_REL} "
                f"(or to NOT_REEXPORTED in scripts/check_reexports.py with a reason)"
            )
        for mod in stale_allow:
            print(
                f"NOT_REEXPORTED lists `{mod}`, which is re-exported or no longer a "
                f"member module: drop it from scripts/check_reexports.py"
            )
        raise SystemExit(1)
    print(
        f"re-exports ok: {len(members)} member modules, "
        f"{len(members) - len(allowed)} re-exported, {len(allowed)} deliberately omitted"
    )


def build_synthetic_tree(root: Path) -> None:
    (root / "crates/par-term-emu-core/src").mkdir(parents=True, exist_ok=True)
    (root / "src").mkdir(parents=True, exist_ok=True)
    (root / MEMBER_LIB_REL).write_text(
        "pub mod alpha;\n"
        '#[cfg(feature = "x")]\n'
        "pub mod beta;\n"
        "pub mod gamma;\n"
        "// pub mod commented;\n",
        encoding="utf-8",
    )
    (root / ROOT_LIB_REL).write_text(
        '#[cfg(feature = "x")]\n'
        "pub use par_term_emu_core::beta;\n"
        "pub use par_term_emu_core::{\n    alpha, gamma,\n};\n",
        encoding="utf-8",
    )


def self_test() -> None:
    with tempfile.TemporaryDirectory(prefix="arc006-selftest-") as tmp:
        root = Path(tmp)
        build_synthetic_tree(root)
        check(root, {})  # Groups, gated single uses, and comments all parse.

        # A member module added without its re-export fails and names it.
        lib = root / MEMBER_LIB_REL
        lib.write_text(
            lib.read_text(encoding="utf-8") + "pub mod delta;\n", encoding="utf-8"
        )
        expect_failure(lambda: check(root, {}), "`delta` has no root re-export")

        # A deliberate omission passes, and goes stale once it is re-exported.
        check(root, {"delta": "test"})
        (root / ROOT_LIB_REL).write_text(
            (root / ROOT_LIB_REL).read_text(encoding="utf-8")
            + "pub use par_term_emu_core::delta;\n",
            encoding="utf-8",
        )
        expect_failure(lambda: check(root, {"delta": "test"}), "NOT_REEXPORTED lists")

        # A commented-out re-export does not count.
        build_synthetic_tree(root)
        (root / ROOT_LIB_REL).write_text(
            "// pub use par_term_emu_core::{alpha, beta, gamma};\n"
            "pub use par_term_emu_core::{alpha, gamma};\n",
            encoding="utf-8",
        )
        expect_failure(lambda: check(root, {}), "`beta` has no root re-export")

        # Empty inputs fail closed.
        (root / ROOT_LIB_REL).write_text("fn main() {}\n", encoding="utf-8")
        expect_failure(lambda: check(root, {}), "parsed nothing")

    check(ROOT)
    print(
        "self-test ok: missing re-export, stale allowance, comments, and empty input reported"
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root", type=Path, default=ROOT, help="repository root to check"
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="inject each failure mode into a synthetic tree, then check the real tree",
    )
    args = parser.parse_args()
    if args.self_test:
        self_test()
    else:
        check(args.root)


if __name__ == "__main__":
    main()
