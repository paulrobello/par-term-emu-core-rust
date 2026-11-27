#!/usr/bin/env python3
"""ARC-011 — release-packaging drift gate.

The root crate's `[package] exclude` list decides what ships to crates.io.
It was hand-maintained with no gate: agent config dirs (.claude/, .codex/,
...), AGENTS.md, AUDIT.md, and a stray empty file all reached the published
crate, and stale entries (`handoff.md`, `streaming.md`) survived unnoticed.
Two checks fail closed:

1. The exclude list must match the BASELINE below exactly (order included).
   Any edit is a reviewed decision, like the doc-hidden baseline.
2. `cargo package --list` (real packaging semantics) must contain no path
   under a forbidden prefix — agent/infra trees and the Python/frontend dev
   trees the exclude list exists to keep out. This catches a regression even
   if the baseline was updated to match a bad edit.

Fails closed: a manifest that does not parse, a missing `cargo`, or a
failing `cargo package --list` is a broken gate rather than a pass.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Exact expected [package] exclude list in root Cargo.toml (order included).
BASELINE = [
    "*.png",
    "*.jpg",
    "*.jpeg",
    "*.svg",
    "*.sixel",
    "*.html",
    "*.yaml",
    "images/",
    "examples/",
    "tests/",
    "web-terminal-frontend/",
    "web_term/",
    "fuzz/",
    "terminfo/",
    "shell_integration/",
    ".github/",
    ".gitignore",
    ".gitattributes",
    ".claude/",
    ".codex/",
    ".config/",
    ".gemini/",
    ".impeccable/",
    ".parsightignore",
    ".pre-commit-config.yaml",
    "AGENTS.md",
    "AUDIT.md",
    "CLAUDE.md",
    "Makefile",
    "pyproject.toml",
    "python/",
    "scripts/",
    "uv.lock",
    "pyrightconfig.json",
    "theme.css",
]

# Path prefixes that must never appear in the packaged set. Mirrors the
# directory/file entries above plus the repo-infra dot entries; glob entries
# (*.png and friends) are covered by the baseline lock, not here.
FORBIDDEN_PREFIXES = [
    ".claude/",
    ".codex/",
    ".config/",
    ".gemini/",
    ".impeccable/",
    ".parsightignore",
    ".pre-commit-config.yaml",
    ".github/",
    ".gitattributes",
    ".gitignore",
    "AGENTS.md",
    "AUDIT.md",
    "CLAUDE.md",
    "Makefile",
    "pyproject.toml",
    "uv.lock",
    "pyrightconfig.json",
    "theme.css",
    "images/",
    "examples/",
    "tests/",
    "web-terminal-frontend/",
    "web_term/",
    "fuzz/",
    "terminfo/",
    "shell_integration/",
    "python/",
    "scripts/",
]


def check_exclude_list(manifest: bytes) -> list[str]:
    """Baseline-lock check over the parsed exclude list (self-testable)."""
    data = tomllib.loads(manifest.decode("utf-8"))
    exclude = data.get("package", {}).get("exclude")
    if not isinstance(exclude, list) or not all(isinstance(e, str) for e in exclude):
        return ["Cargo.toml: [package] exclude must be a list of strings"]

    problems: list[str] = []
    if exclude != BASELINE:
        extra = [e for e in exclude if e not in BASELINE]
        missing = [e for e in BASELINE if e not in exclude]
        if extra:
            problems.append(
                f"Cargo.toml exclude has entries outside the ARC-011 baseline: "
                f"{extra}; every edit is a reviewed decision — update "
                f"BASELINE in scripts/check_package_exclude.py in the same commit"
            )
        if missing:
            problems.append(
                f"Cargo.toml exclude lost baseline entries: {missing}; restore "
                f"them or consciously update BASELINE in "
                f"scripts/check_package_exclude.py (and FORBIDDEN_PREFIXES "
                f"when removing a rule)"
            )
    return problems


def check_stale_entries(manifest: bytes, root: Path) -> list[str]:
    """Every non-glob exclude entry must name an existing path."""
    data = tomllib.loads(manifest.decode("utf-8"))
    exclude = data.get("package", {}).get("exclude") or []
    problems: list[str] = []
    for entry in exclude:
        if "*" in entry:
            continue
        target = root / entry.rstrip("/")
        if not target.exists():
            problems.append(
                f"Cargo.toml excludes {entry!r} but no such file or directory "
                f"exists; drop the stale entry"
            )
    return problems


def forbidden_hits(packaged: list[str]) -> list[str]:
    """Return packaged paths that start with a forbidden prefix."""
    return [
        path
        for path in packaged
        if any(path.startswith(prefix) for prefix in FORBIDDEN_PREFIXES)
    ]


def check(root: Path) -> list[str]:
    """Return the list of problems found under `root` (empty when clean)."""
    manifest = (root / "Cargo.toml").read_bytes()
    problems = check_exclude_list(manifest) + check_stale_entries(manifest, root)

    try:
        result = subprocess.run(
            ["cargo", "package", "--list", "--allow-dirty"],
            cwd=root,
            capture_output=True,
            text=True,
            timeout=180,
            check=False,
        )
    except FileNotFoundError:
        return problems + ["cargo not found; the packaging gate cannot run"]
    if result.returncode != 0:
        stderr = result.stderr.strip()[:400]
        return problems + [
            f"cargo package --list failed (exit {result.returncode}): {stderr}"
        ]
    packaged = [line for line in result.stdout.splitlines() if line.strip()]
    hits = forbidden_hits(packaged)
    if hits:
        problems.append(
            "the packaged crate would ship forbidden paths: "
            f"{hits[:10]}{' ...' if len(hits) > 10 else ''}; add them to the "
            f"[package] exclude list and the ARC-011 baseline"
        )
    return problems


def self_test() -> None:
    """Prove baseline drift, stale entries, and forbidden hits all fail."""
    manifest_tpl = (
        '[package]\nname = "x"\nversion = "9.9.9"\nexclude = [\n{entries}\n]\n'
    )
    full_entries = "\n".join(f"    {e!r}," for e in BASELINE)
    missing_one = "\n".join(f"    {e!r}," for e in BASELINE[1:])

    cases: list[tuple[str, str, bool]] = [
        # (manifest-exclude-entries, label, expected-ok)
        (full_entries, "baseline passes", True),
        (full_entries + '\n    "sneaky/",', "extra entry fails", False),
        (missing_one, "missing entry fails", False),
    ]
    for entries, label, ok in cases:
        manifest = manifest_tpl.format(entries=entries)
        problems = check_exclude_list(manifest.encode())
        if (not problems) != ok:
            sys.exit(f"self-test FAILED ({label}): got problems={problems}")

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        (root / "src").mkdir()
        manifest = manifest_tpl.format(entries='    "gone-dir/",')
        problems = check_stale_entries(manifest.encode(), root)
        if not any("no such file or directory" in p for p in problems):
            sys.exit("self-test FAILED (stale entry): missing path not reported")
        manifest = manifest_tpl.format(entries='    "src/",')
        if check_stale_entries(manifest.encode(), root):
            sys.exit("self-test FAILED (existing entry): false positive")

    hits = forbidden_hits(["src/lib.rs", ".claude/settings.json", "AUDIT.md"])
    if [h for h in hits if h in (".claude/settings.json", "AUDIT.md")] != [
        ".claude/settings.json",
        "AUDIT.md",
    ]:
        sys.exit("self-test FAILED (forbidden detection)")
    if forbidden_hits(["src/lib.rs", "docs/guide.md"]):
        sys.exit("self-test FAILED (false positive on legit paths)")

    print("check_package_exclude self-test: OK")


def main() -> None:
    parser = argparse.ArgumentParser(description="ARC-011 release-packaging drift gate")
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
    print(
        "package exclude list matches baseline; no forbidden paths in cargo package --list"
    )


if __name__ == "__main__":
    main()
