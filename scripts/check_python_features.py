#!/usr/bin/env python3
"""Audit ARC-001 (2026-11-06 re-audit): `python` / `python-test` parity.

The root Cargo.toml defines the shipped PyO3 build (`python`) and its
cargo-test twin (`python-test`) as two hand-maintained lists. They must
stay in lockstep: everything except the pyo3 activation entry
(`pyo3/extension-module` on the wheel side, `pyo3/auto-initialize` on
the test side) is the same surface. A sub-feature added to one list but
not the other makes `cargo test` exercise a different surface than the
shipped wheel, silently. This gate fails the moment the two drift,
asserting on sets so cosmetic reordering passes.

`--self-test` injects each drift class into a copy of the parsed
manifest and requires the check to fail naming the drift, then requires
the real manifest to pass.
"""

from __future__ import annotations

import argparse
import tomllib
from pathlib import Path

from _gate_common import ROOT, expect_failure, fail

CARGO_REL = "Cargo.toml"

# The only entries allowed to differ between the two lists: how pyo3
# links a real interpreter (extension-module in the wheel,
# auto-initialize under `cargo test`).
ACTIVATION = {"pyo3/extension-module", "pyo3/auto-initialize"}


def load_features(root: Path) -> dict[str, list[str]]:
    """The `[features]` entries of `root`'s manifest, failing loudly."""
    path = root / CARGO_REL
    try:
        with path.open("rb") as f:
            manifest = tomllib.load(f)
    except (OSError, tomllib.TOMLDecodeError) as exc:
        fail(f"cannot parse {path}: {exc}")
    features = manifest.get("features")
    if not isinstance(features, dict):
        fail(f"{path} has no [features] table")
    return features


def feature_set(features: dict[str, list[str]], name: str) -> set[str]:
    entry = features.get(name)
    if not isinstance(entry, list):
        fail(f"[features] {name} is missing or not a list")
    return set(entry)


def check(features: dict[str, list[str]]) -> None:
    """Assert the two lists differ by exactly the pyo3 activation pair."""
    python = feature_set(features, "python")
    python_test = feature_set(features, "python-test")
    problems: list[str] = []

    # One-sided entries outside the activation pair are real drift.
    for name, side in (("python", python), ("python-test", python_test)):
        other = python_test if name == "python" else python
        unexpected = side - other - ACTIVATION
        if unexpected:
            problems.append(
                f"[features] {name} carries {sorted(unexpected)} that the "
                "other list lacks — the two must stay in lockstep"
            )

    # And the full difference must be exactly the activation pair: each
    # of extension-module/auto-initialize in exactly one list.
    drift = python ^ python_test
    if drift != ACTIVATION:
        problems.append(
            f"the python/python-test difference is {sorted(drift)}, expected "
            f"exactly {sorted(ACTIVATION)} (python has "
            f"{sorted(python & ACTIVATION)}, python-test has "
            f"{sorted(python_test & ACTIVATION)})"
        )

    if problems:
        for p in problems:
            print(p)
        raise SystemExit(1)
    shared = len(python & python_test)
    print(
        f"python features ok: {shared} shared entries, differing only by "
        f"{sorted(ACTIVATION)}"
    )


def self_test() -> None:
    features = load_features(ROOT)

    def with_drift(name: str, drop: set[str], add: set[str]) -> dict:
        drifted = {k: list(v) for k, v in features.items()}
        drifted[name] = [e for e in features[name] if e not in drop] + sorted(add)
        return drifted

    expect_failure(
        lambda: check(with_drift("python-test", {"screenshot"}, set())),
        "screenshot",
    )
    expect_failure(
        lambda: check(with_drift("python-test", set(), {"extra-feature"})),
        "extra-feature",
    )
    # Equal activation entries (both dropped here) collapse the required
    # difference to nothing — the gate must catch that too.
    expect_failure(
        lambda: check(with_drift("python", {"pyo3/extension-module"}, set())),
        "pyo3/extension-module",
    )

    check(features)
    print("self-test ok: dropped, added, and activation drifts all reported")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root", type=Path, default=ROOT, help="repository root to check"
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="check injected drifts for each failure mode, then the real manifest",
    )
    args = parser.parse_args()
    if args.self_test:
        self_test()
    else:
        check(load_features(args.root))


if __name__ == "__main__":
    main()
