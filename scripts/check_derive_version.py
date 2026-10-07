#!/usr/bin/env python3
"""ARC-008 — derive-crate version skew gate.

`derive/Cargo.toml` versions independently of the main crate, but the main
crate's `par-term-emu-derive` dependency spec must name exactly the derive
crate's version: the publish workflows strip the `path` key, so crates.io
resolves the spec against the registry, and a spec that lags the sub-crate
silently builds against an older published derive.

Fails closed: a manifest that does not parse, or a spec without a `version`
key, is a broken gate rather than a pass.
"""

from __future__ import annotations

import argparse
import sys
import tempfile
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEP_NAME = "par-term-emu-derive"


def check(root: Path) -> list[str]:
    """Return the list of problems found under `root` (empty when in sync)."""
    main = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    derive = tomllib.loads((root / "derive" / "Cargo.toml").read_text(encoding="utf-8"))

    derive_name = derive.get("package", {}).get("name")
    derive_version = derive.get("package", {}).get("version")
    if derive_name != DEP_NAME or not derive_version:
        return [
            f"derive/Cargo.toml: expected package {DEP_NAME!r} with a version, got {derive_name!r} {derive_version!r}"
        ]

    spec = main.get("dependencies", {}).get(DEP_NAME)
    if not isinstance(spec, dict) or "version" not in spec:
        return [
            f"Cargo.toml: [dependencies] {DEP_NAME} must be a table with a version key, got {spec!r}"
        ]

    wanted = spec["version"].lstrip("=")
    if wanted != derive_version:
        return [
            (
                f"Cargo.toml pins {DEP_NAME} = {spec['version']!r} but derive/Cargo.toml is "
                f"{derive_version!r}; update the dependency spec to match"
            )
        ]
    return []


def self_test() -> None:
    """Prove the gate passes on a matching pair and fails on a skewed pair."""
    main_tpl = '[package]\nname = "x"\nversion = "9.9.9"\n[dependencies]\n{dep}\n'
    derive_tpl = '[package]\nname = "par-term-emu-derive"\nversion = "{v}"\n'
    cases = [
        ('par-term-emu-derive = { path = "derive", version = "1.2.3" }', "1.2.3", True),
        (
            'par-term-emu-derive = { path = "derive", version = "=1.2.3" }',
            "1.2.3",
            True,
        ),
        (
            'par-term-emu-derive = { path = "derive", version = "1.2.2" }',
            "1.2.3",
            False,
        ),
        ('par-term-emu-derive = { path = "derive" }', "1.2.3", False),
        ("", "1.2.3", False),
    ]
    for dep, version, ok in cases:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "derive").mkdir()
            (root / "Cargo.toml").write_text(main_tpl.format(dep=dep), encoding="utf-8")
            (root / "derive" / "Cargo.toml").write_text(
                derive_tpl.format(v=version), encoding="utf-8"
            )
            if (not check(root)) != ok:
                sys.exit(
                    f"self-test FAILED for dep={dep!r} derive={version!r}: expected ok={ok}"
                )
    print("check_derive_version self-test: OK")


def main() -> None:
    parser = argparse.ArgumentParser(
        description="ARC-008 derive-crate version skew gate"
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
    print(f"{DEP_NAME} dependency spec matches derive/Cargo.toml")


if __name__ == "__main__":
    main()
