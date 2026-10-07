#!/usr/bin/env python3
"""ARC-007 — core member lockstep version gate.

`crates/par-term-emu-core` versions in lockstep with the root crate
(crates/par-term-emu-core/DESIGN.md "Version train"): the XTVERSION reply
embeds the member's CARGO_PKG_VERSION, so a member that lags the root changes
what the terminal reports. The root's dependency spec must pin exactly that
version (`=X.Y.Z`), because publishing strips `path` and crates.io resolves
the spec against the registry.

Fails closed: an unparseable manifest or a spec without a version is a
broken gate, not a pass.
"""

from __future__ import annotations

import argparse
import sys
import tempfile
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEP_NAME = "par-term-emu-core"
MEMBER = Path("crates") / "par-term-emu-core" / "Cargo.toml"


def check(root: Path) -> list[str]:
    """Return the list of problems found under `root` (empty when in sync)."""
    main = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    member = tomllib.loads((root / MEMBER).read_text(encoding="utf-8"))

    root_version = main.get("package", {}).get("version")
    member_name = member.get("package", {}).get("name")
    member_version = member.get("package", {}).get("version")
    if member_name != DEP_NAME or not member_version:
        return [
            f"{MEMBER}: expected package {DEP_NAME!r} with a version, got {member_name!r} {member_version!r}"
        ]

    problems: list[str] = []
    if member_version != root_version:
        problems.append(
            f"{MEMBER} version {member_version!r} != root version {root_version!r} (lockstep; see DESIGN.md)"
        )
    spec = main.get("dependencies", {}).get(DEP_NAME)
    if not isinstance(spec, dict) or "version" not in spec:
        problems.append(
            f"Cargo.toml: [dependencies] {DEP_NAME} must be a table with a version key, got {spec!r}"
        )
    elif spec["version"] != f"={member_version}":
        problems.append(
            f"Cargo.toml pins {DEP_NAME} = {spec['version']!r}; expected '={member_version}' (exact pin)"
        )
    return problems


def self_test() -> None:
    """Prove the gate passes on a lockstep pair and fails on each drift."""
    main_tpl = '[package]\nname = "x"\nversion = "{rv}"\n[dependencies]\n{dep}\n'
    member_tpl = '[package]\nname = "par-term-emu-core"\nversion = "{mv}"\n'
    good = (
        'par-term-emu-core = { path = "crates/par-term-emu-core", version = "=1.2.3" }'
    )
    cases = [
        (good, "1.2.3", "1.2.3", True),
        (good, "1.2.4", "1.2.3", False),  # root bumped, member not
        (good.replace("=1.2.3", "1.2.3"), "1.2.3", "1.2.3", False),  # not exact
        (good.replace("=1.2.3", "=1.2.2"), "1.2.3", "1.2.3", False),  # stale pin
        (
            'par-term-emu-core = { path = "crates/par-term-emu-core" }',
            "1.2.3",
            "1.2.3",
            False,
        ),
        ("", "1.2.3", "1.2.3", False),
    ]
    for dep, rv, mv, ok in cases:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / MEMBER).parent.mkdir(parents=True)
            (root / "Cargo.toml").write_text(
                main_tpl.format(rv=rv, dep=dep), encoding="utf-8"
            )
            (root / MEMBER).write_text(member_tpl.format(mv=mv), encoding="utf-8")
            if (not check(root)) != ok:
                sys.exit(
                    f"self-test FAILED for dep={dep!r} root={rv} member={mv}: expected ok={ok}"
                )
    print("check_core_version self-test: OK")


def main() -> None:
    parser = argparse.ArgumentParser(
        description="ARC-007 core member lockstep version gate"
    )
    parser.add_argument(
        "--self-test", action="store_true", help="run the gate's own regression cases"
    )
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return
    problems = check(ROOT)
    if problems:
        for problem in problems:
            print(f"ERROR: {problem}", file=sys.stderr)
        sys.exit(1)
    print(f"{DEP_NAME} version and root pin are in lockstep")


if __name__ == "__main__":
    main()
