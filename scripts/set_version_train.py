#!/usr/bin/env python3
"""ARC-009: rewrite the whole version train to one version in a single step.

The lockstep train (CLAUDE.md "Version Sync", crates/par-term-emu-core/
DESIGN.md "Version train") is five version sites plus three exact pins:

- `Cargo.toml` [package] version
- `pyproject.toml` [project] version
- `python/par_term_emu_core_rust/__init__.py` `__version__`
- `crates/par-term-emu-core/Cargo.toml` [package] version
- `crates/par-mux/Cargo.toml` [package] version
- pins `=X.Y.Z`: root -> par-term-emu-core, root -> par-mux,
  par-mux -> par-term-emu-core

plus the three workspace entries in `Cargo.lock`, so `--locked` builds keep
working without a separate `cargo update`. `derive/Cargo.toml` versions
independently and is never touched (see `make derive-version-check`).

`make core-version-check` / `derive-version-check` verify the train; this is
the writer they lacked. Every site is matched against the CURRENT version and
must match exactly once, or nothing is written: a partial train is worse than
none. After writing, the core-version gate is re-run against the result.

Usage:
    python3 scripts/set_version_train.py 0.72.0           # rewrite
    python3 scripts/set_version_train.py 0.72.0 --dry-run # list the edits
    python3 scripts/set_version_train.py --self-test

It does not touch CHANGELOG.md: the release-notes gate owns that step.
"""

from __future__ import annotations

import argparse
import re
import shutil
import sys
import tempfile
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import NoReturn

ROOT = Path(__file__).resolve().parent.parent
SEMVER_RE = re.compile(r"^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$")
WORKSPACE_CRATES = ("par-term-emu-core-rust", "par-term-emu-core", "par-mux")


@dataclass(frozen=True)
class Site:
    """One version occurrence: `pattern` with `{v}` for the escaped version."""

    rel: str
    pattern: str
    what: str


SITES: tuple[Site, ...] = (
    Site("Cargo.toml", r'(?m)^(version = "){v}(")', "root [package] version"),
    Site(
        "Cargo.toml",
        r'(par-term-emu-core = \{{ path = "crates/par-term-emu-core", version = "=){v}(")',
        "root -> par-term-emu-core pin",
    ),
    Site(
        "Cargo.toml",
        r'(par-mux = \{{ path = "crates/par-mux", version = "=){v}(")',
        "root -> par-mux pin",
    ),
    Site("pyproject.toml", r'(?m)^(version = "){v}(")', "pyproject [project] version"),
    Site(
        "python/par_term_emu_core_rust/__init__.py",
        r'(?m)^(__version__ = "){v}(")',
        "python __version__",
    ),
    Site(
        "crates/par-term-emu-core/Cargo.toml",
        r'(?m)^(version = "){v}(")',
        "par-term-emu-core [package] version",
    ),
    Site(
        "crates/par-mux/Cargo.toml",
        r'(?m)^(version = "){v}(")',
        "par-mux [package] version",
    ),
    Site(
        "crates/par-mux/Cargo.toml",
        r'(par-term-emu-core = \{{ path = "\.\./par-term-emu-core", version = "=){v}(")',
        "par-mux -> par-term-emu-core pin",
    ),
) + tuple(
    Site(
        "Cargo.lock",
        r'(name = "' + re.escape(name) + r'"\nversion = "){v}(")',
        f"Cargo.lock {name}",
    )
    for name in WORKSPACE_CRATES
)


def fail(msg: str) -> NoReturn:
    print(f"ERROR: {msg}", file=sys.stderr)
    raise SystemExit(1)


def current_version(root: Path) -> str:
    try:
        manifest = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as exc:
        fail(f"cannot read {root / 'Cargo.toml'}: {exc}")
    version = manifest.get("package", {}).get("version")
    if not isinstance(version, str):
        fail("root Cargo.toml has no [package] version")
    return version


def plan(root: Path, old: str, new: str) -> dict[str, str]:
    """New text per file, or fail naming every site that does not match once."""
    texts: dict[str, str] = {}
    problems: list[str] = []
    for site in SITES:
        path = root / site.rel
        if site.rel not in texts:
            try:
                texts[site.rel] = path.read_text(encoding="utf-8")
            except OSError as exc:
                problems.append(f"{site.rel}: {exc}")
                continue
        regex = re.compile(site.pattern.format(v=re.escape(old)))
        updated, n = regex.subn(
            lambda m: f"{m.group(1)}{new}{m.group(2)}", texts[site.rel]
        )
        if n != 1:
            problems.append(
                f"{site.rel}: {site.what} matched {n} times (want 1) at {old!r}"
            )
            continue
        texts[site.rel] = updated
    if problems:
        fail("version train not rewritten:\n  " + "\n  ".join(problems))
    return texts


def apply(root: Path, new: str, dry_run: bool = False) -> str:
    """Rewrite the train under `root` to `new`; return the previous version."""
    if not SEMVER_RE.match(new):
        fail(f"{new!r} is not a semver version (X.Y.Z[-pre])")
    old = current_version(root)
    if old == new:
        fail(f"the train is already at {new}")
    texts = plan(root, old, new)
    for site in SITES:
        print(f"{'would set' if dry_run else 'set'} {site.what}: {old} -> {new}")
    if dry_run:
        return old
    for rel, text in texts.items():
        (root / rel).write_text(text, encoding="utf-8")
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import check_core_version

    problems = check_core_version.check(root)
    if problems:
        fail(
            "rewrote the train but core-version-check fails:\n  "
            + "\n  ".join(problems)
        )
    return old


def self_test() -> None:
    """Rewrite a copy of the real train, verify it, and prove the refusals."""
    with tempfile.TemporaryDirectory(prefix="arc009-selftest-") as tmp:
        root = Path(tmp)
        for rel in {site.rel for site in SITES}:
            (root / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / rel, root / rel)
        old = current_version(root)
        before = {
            rel: (root / rel).read_text(encoding="utf-8")
            for rel in {s.rel for s in SITES}
        }

        apply(root, "99.0.0")
        if current_version(root) != "99.0.0":
            fail("self-test: root version not rewritten")
        for rel, text in before.items():
            after = (root / rel).read_text(encoding="utf-8")
            if after.count("99.0.0") < 1 or after.replace("99.0.0", old) != text:
                fail(f"self-test: {rel} changed outside its version sites")

        derive = (ROOT / "derive/Cargo.toml").read_text(encoding="utf-8")
        if "99.0.0" in derive:
            fail("self-test: derive/Cargo.toml must never be rewritten")

        lock = (root / "Cargo.lock").read_text(encoding="utf-8")
        (root / "Cargo.lock").write_text(
            lock.replace(
                'name = "par-mux"\nversion = "99.0.0"',
                'name = "par-mux"\nversion = "0.0.1"',
            ),
            encoding="utf-8",
        )
        snapshot = {rel: (root / rel).read_text(encoding="utf-8") for rel in before}
        try:
            apply(root, "99.0.1")
        except SystemExit:
            pass
        else:
            fail("self-test: a drifted site must refuse the rewrite")
        for rel, text in snapshot.items():
            if (root / rel).read_text(encoding="utf-8") != text:
                fail(f"self-test: a refused rewrite still wrote {rel}")

        for bad in ("1.2", "v1.2.3", "99.0.0"):
            try:
                apply(root, bad)
            except SystemExit:
                continue
            fail(f"self-test: {bad!r} should be refused")
    print("set_version_train self-test: OK (rewrite, lockstep re-check, refusals)")


def main() -> None:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("version", nargs="?", help="the new version, e.g. 0.72.0")
    parser.add_argument(
        "--dry-run", action="store_true", help="list the edits without writing"
    )
    parser.add_argument(
        "--self-test", action="store_true", help="run against a temp copy of the train"
    )
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return
    if not args.version:
        parser.error("a version is required (or --self-test)")
    old = apply(ROOT, args.version, args.dry_run)
    if not args.dry_run:
        print(
            f"version train {old} -> {args.version}; next: CHANGELOG.md, then make release-check"
        )


if __name__ == "__main__":
    main()
