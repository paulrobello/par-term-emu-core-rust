#!/usr/bin/env python3
"""MSRV vs workflow toolchain pin parity gate.

The crate's ``rust-version`` floor must not exceed the toolchain pins CI
builds with: deployment.yml's maturin-action sites pin an exact rustc for
the wheel builds, and a floor bump that leaves those pins behind fails
only on the release runners ("rustc X is not supported by the following
packages"), after a deploy run has already been spent (0.84.1, run
44681467480). Local gates never see it — they build with the host
toolchain, and ci.yml installs stable.

Fails when any numeric ``rust-toolchain:``/``toolchain:`` pin in
.github/workflows/ is older than the root Cargo.toml ``rust-version``.
Channel names (stable, nightly, msrv…) are not version-comparable and are
skipped. Fails closed: a manifest without ``rust-version`` is a broken
gate rather than a pass.
"""

from __future__ import annotations

import argparse
import re
import sys
import tempfile
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORKFLOWS = ROOT / ".github" / "workflows"
PIN_RE = re.compile(r"^\s*(rust-)?toolchain:\s*(\S+)\s*$", re.MULTILINE)
NUMERIC_RE = re.compile(r"^\d+(?:\.\d+){0,2}$")


def parse_version(text: str) -> tuple[int, ...]:
    parts = tuple(int(p) for p in text.split("."))
    return parts + (0,) * (3 - len(parts))


def check(root: Path) -> list[str]:
    """Return the list of problems found under `root` (empty when in sync)."""
    manifest = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    floor = manifest.get("package", {}).get("rust-version")
    if not floor:
        return ["Cargo.toml: no [package] rust-version to verify workflow pins against"]

    problems: list[str] = []
    workflows = root / ".github" / "workflows"
    for path in sorted(workflows.glob("*.yml")):
        text = path.read_text(encoding="utf-8")
        for match in PIN_RE.finditer(text):
            key = "rust-toolchain" if match.group(1) else "toolchain"
            value = match.group(2)
            if not NUMERIC_RE.match(value):
                continue
            if parse_version(value) < parse_version(floor):
                line = text[: match.start()].count("\n") + 1
                problems.append(
                    f"{path.relative_to(root)}:{line}: {key}: {value} is older than the "
                    f"rust-version floor {floor}; bump the pin (or the floor is wrong)"
                )
    return problems


def self_test() -> None:
    """Prove the gate passes on fresh pins and fails on stale ones."""
    manifest = '[package]\nname = "x"\nversion = "0.0.0"\nrust-version = "1.100"\n'
    cases = [
        ("rust-toolchain: 1.100.0", True),
        ("rust-toolchain: 1.100", True),
        ("rust-toolchain: 1.101.0", True),
        ("rust-toolchain: stable", True),
        ("rust-toolchain: nightly-2026-01-01", True),
        ("rust-toolchain: 1.99.0", False),
        ("toolchain: 1.99", False),
        ("toolchain: stable", True),
        ("  toolchain: 1.98.0   ", False),
    ]
    for pin, ok in cases:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / ".github" / "workflows").mkdir(parents=True)
            (root / "Cargo.toml").write_text(manifest, encoding="utf-8")
            (root / ".github" / "workflows" / "deploy.yml").write_text(
                f"steps:\n  - uses: x\n    with:\n      {pin}\n", encoding="utf-8"
            )
            if (not check(root)) != ok:
                sys.exit(f"self-test FAILED for pin={pin!r}: expected ok={ok}")
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        (root / ".github" / "workflows").mkdir(parents=True)
        (root / "Cargo.toml").write_text('[package]\nname = "x"\n', encoding="utf-8")
        if not check(root):
            sys.exit("self-test FAILED: missing rust-version must fail closed")
    print("check_workflow_toolchains self-test: OK")


def main() -> None:
    parser = argparse.ArgumentParser(
        description="MSRV vs workflow toolchain pin parity gate"
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
    print("workflow toolchain pins all meet the Cargo.toml rust-version floor")


if __name__ == "__main__":
    main()
