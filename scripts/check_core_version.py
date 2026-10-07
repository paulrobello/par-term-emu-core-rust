#!/usr/bin/env python3
"""ARC-007 — workspace member lockstep version gate.

`crates/par-term-emu-core` and `crates/par-mux` version in lockstep with the
root crate (crates/par-term-emu-core/DESIGN.md "Version train"): the XTVERSION
reply embeds the core's CARGO_PKG_VERSION and `mux::build_stamp()` embeds
par-mux's, so a member that lags the root changes what the terminal and the
daemon report. Every in-workspace dependency edge must pin exactly that
version (`=X.Y.Z`), because publishing strips `path` and crates.io resolves
the spec against the registry. The edges: root -> par-term-emu-core,
root -> par-mux, par-mux -> par-term-emu-core.

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
CORE = "par-term-emu-core"
MUX = "par-mux"
MEMBERS = {
    CORE: Path("crates") / CORE / "Cargo.toml",
    MUX: Path("crates") / MUX / "Cargo.toml",
}
ROOT_MANIFEST = Path("Cargo.toml")
# (dependent manifest, dependency name): every edge that must pin exactly.
EDGES = [(ROOT_MANIFEST, CORE), (ROOT_MANIFEST, MUX), (MEMBERS[MUX], CORE)]


def _load(root: Path, rel: Path) -> dict:
    return tomllib.loads((root / rel).read_text(encoding="utf-8"))


def check(root: Path) -> list[str]:
    """Return the list of problems found under `root` (empty when in sync)."""
    main = _load(root, ROOT_MANIFEST)
    root_version = main.get("package", {}).get("version")

    problems: list[str] = []
    versions: dict[str, str] = {}
    for name, rel in MEMBERS.items():
        member = _load(root, rel)
        member_name = member.get("package", {}).get("name")
        member_version = member.get("package", {}).get("version")
        if member_name != name or not member_version:
            problems.append(
                f"{rel}: expected package {name!r} with a version, got {member_name!r} {member_version!r}"
            )
            continue
        versions[name] = member_version
        if member_version != root_version:
            problems.append(
                f"{rel} version {member_version!r} != root version {root_version!r} (lockstep; see DESIGN.md)"
            )

    for manifest, dep in EDGES:
        if dep not in versions:
            continue
        spec = _load(root, manifest).get("dependencies", {}).get(dep)
        if not isinstance(spec, dict) or "version" not in spec:
            problems.append(
                f"{manifest}: [dependencies] {dep} must be a table with a version key, got {spec!r}"
            )
        elif spec["version"] != f"={versions[dep]}":
            problems.append(
                f"{manifest} pins {dep} = {spec['version']!r}; expected '={versions[dep]}' (exact pin)"
            )
    return problems


def self_test() -> None:
    """Prove the gate passes on a lockstep workspace and fails on each drift."""
    main_tpl = (
        '[package]\nname = "x"\nversion = "{rv}"\n[dependencies]\n{core}\n{mux}\n'
    )
    core_tpl = '[package]\nname = "par-term-emu-core"\nversion = "{cv}"\n'
    mux_tpl = '[package]\nname = "par-mux"\nversion = "{xv}"\n[dependencies]\n{core}\n'
    core_dep = (
        'par-term-emu-core = { path = "crates/par-term-emu-core", version = "=1.2.3" }'
    )
    mux_dep = 'par-mux = { path = "crates/par-mux", version = "=1.2.3" }'
    good = {
        "rv": "1.2.3",
        "cv": "1.2.3",
        "xv": "1.2.3",
        "rcore": core_dep,
        "rmux": mux_dep,
        "xcore": core_dep,
    }
    cases = [
        ({}, True),
        ({"rv": "1.2.4"}, False),  # root bumped, members not
        ({"xv": "1.2.4"}, False),  # par-mux drifted alone
        ({"cv": "1.2.4"}, False),  # core drifted alone
        ({"rcore": core_dep.replace("=1.2.3", "1.2.3")}, False),  # not exact
        ({"rcore": core_dep.replace("=1.2.3", "=1.2.2")}, False),  # stale pin
        ({"rmux": mux_dep.replace("=1.2.3", "1.2.3")}, False),  # root->mux not exact
        ({"xcore": core_dep.replace("=1.2.3", "=1.2.2")}, False),  # mux->core stale
        ({"xcore": 'par-term-emu-core = { path = "../par-term-emu-core" }'}, False),
        ({"rmux": ""}, False),  # root lost its par-mux dependency
        ({"rcore": ""}, False),
    ]
    for override, ok in cases:
        v = {**good, **override}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for rel in MEMBERS.values():
                (root / rel).parent.mkdir(parents=True)
            (root / ROOT_MANIFEST).write_text(
                main_tpl.format(rv=v["rv"], core=v["rcore"], mux=v["rmux"]),
                encoding="utf-8",
            )
            (root / MEMBERS[CORE]).write_text(
                core_tpl.format(cv=v["cv"]), encoding="utf-8"
            )
            (root / MEMBERS[MUX]).write_text(
                mux_tpl.format(xv=v["xv"], core=v["xcore"]), encoding="utf-8"
            )
            if (not check(root)) != ok:
                sys.exit(f"self-test FAILED for {override!r}: expected ok={ok}")
    print("check_core_version self-test: OK")


def main() -> None:
    parser = argparse.ArgumentParser(
        description="ARC-007 workspace member lockstep version gate"
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
    print(f"{CORE} and {MUX} versions and their exact pins are in lockstep")


if __name__ == "__main__":
    main()
