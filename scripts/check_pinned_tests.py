#!/usr/bin/env python3
"""ENH-057: every "pinned by `test`" citation in docs/*.md and CHANGELOG's
[Unreleased] section names a test fn or test target that exists in the tree.

A rename or deletion leaves a dangling citation that reads as proof but points
at nothing; this gate fails on one. `--self-test` builds a synthetic tree,
injects one drift per contract, and also runs the real tree requiring the
known baseline of citations.
"""

from __future__ import annotations

import argparse
import re
from pathlib import Path
from typing import NoReturn

ROOT = Path(__file__).resolve().parent.parent

# A pinned clause: the word "pinned" to the end of its sentence (". ", ";"
# or end of line). Qualifiers may sit between "pinned" and "by".
CLAUSE_RE = re.compile(
    r"\b[Pp]inned\b(?P<clause>[^\n]*?)(?=\.(?:\s|$)|;|$)", re.MULTILINE
)
# Candidate test names: lowercase snake_case only (skips `_Static_assert`,
# paths, flags).
CITED_RE = re.compile(r"`([a-z][a-z0-9_]*)`")
UNRELEASED_RE = re.compile(
    r"^## \[Unreleased\]\s*\n(?P<body>.*?)(?=^## \[)", re.MULTILINE | re.DOTALL
)
TEST_FN_RE = re.compile(
    r"#\[(?:[\w:]+::)?test\b[^\]]*\]\s*(?:#\[[^\]]*\]\s*|///[^\n]*\n\s*)*(?:pub\s+)?(?:async\s+)?fn\s+([a-z_][a-z0-9_]*)"
)
# A `mod <name>` in a file that carries `#[test]`s: the changelog cites test
# MODULES (`require_bcrypt_tests`), not only fns.
MOD_RE = re.compile(r"\bmod\s+([a-z_][a-z0-9_]*)\s*[{;]")
PY_TEST_RE = re.compile(r"^def (test_[a-z0-9_]+)\(", re.MULTILINE)
SKIP_DIRS = {"target", "node_modules"}


def fail(msg: str) -> NoReturn:
    print(msg)
    raise SystemExit(1)


def test_names(root: Path) -> set[str]:
    """Every #[test]-style fn name, integration-target stem, and pytest name."""
    names: set[str] = set()
    found_rs = False
    for path in root.rglob("*.rs"):
        rel = path.relative_to(root)
        if any(part in SKIP_DIRS for part in rel.parts):
            continue
        found_rs = True
        try:
            text = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue
        names.update(TEST_FN_RE.findall(text))
        if "#[test]" in text:
            names.update(MOD_RE.findall(text))
        # Integration-test target stems (crates/*/tests/*.rs and tests/*.rs).
        if "tests" in rel.parts and path.stem != "mod":
            names.add(path.stem)
    for path in root.glob("tests/*.py"):
        names.update(PY_TEST_RE.findall(path.read_text(encoding="utf-8")))
    for path in root.glob("crates/*/tests/*.py"):
        names.update(PY_TEST_RE.findall(path.read_text(encoding="utf-8")))
    if not found_rs or len(names) < 100:
        fail(f"parsed nothing: test-name scan found {len(names)} names")
    return names


def citations(root: Path, names: set[str]) -> tuple[list[tuple[str, int, str]], int]:
    """(file, line, name) for every dangling citation, and the total count."""
    dangling: list[tuple[str, int, str]] = []
    total = 0
    for doc in sorted(root.glob("docs/*.md")):
        total += check_text(
            doc.read_text(encoding="utf-8"), doc.name, names, dangling, 0
        )
    changelog = root / "CHANGELOG.md"
    if changelog.is_file():
        text = changelog.read_text(encoding="utf-8")
        m = UNRELEASED_RE.search(text)
        if m:
            offset = text[: m.start("body")].count("\n") + 1
            total += check_text(
                m.group("body"), "CHANGELOG.md", names, dangling, offset
            )
    return dangling, total


def check_text(
    text: str,
    where: str,
    names: set[str],
    dangling: list[tuple[str, int, str]],
    offset: int,
) -> int:
    total = 0
    first = CLAUSE_RE.search(text)
    base_line = text[: first.start()].count("\n") + 1 if first else 1
    for m in CLAUSE_RE.finditer(text):
        total += 1
        line = offset + base_line + text[m.start() : m.start()].count("\n")
        for name in CITED_RE.findall(m.group(0)):
            if name not in names:
                dangling.append((where, line, name))
    return total


def check(root: Path) -> None:
    names = test_names(root)
    dangling, total = citations(root, names)
    if total == 0:
        fail(
            "parsed nothing: no pinned clauses matched in docs/*.md or CHANGELOG [Unreleased]"
        )
    if dangling:
        for where, line, name in dangling:
            print(
                f"{where}:{line}: pinned clause cites `{name}`, which is not a test fn or test target in the tree"
            )
        raise SystemExit(1)
    print(f"pinned tests ok: {total} citations resolve")


def build_synthetic_tree(root: Path) -> None:
    """A minimal tree: one real test fn padded to 100 names, one target, docs."""
    crate = root / "crates/x/src"
    crate.mkdir(parents=True, exist_ok=True)
    fns = ["#[test]\nfn real_one() {}\n"]
    fns += [f"#[test]\nfn padding_{i:03d}() {{}}\n" for i in range(150)]
    (crate / "lib.rs").write_text("".join(fns), encoding="utf-8")
    targets = root / "crates/x/tests"
    targets.mkdir(parents=True, exist_ok=True)
    (targets / "suite_a.rs").write_text(
        "#[test]\nfn in_target() {}\n", encoding="utf-8"
    )
    docs = root / "docs"
    docs.mkdir(exist_ok=True)
    (docs / "guide.md").write_text(
        "Pinned by `real_one` and `suite_a`.\n\n"
        "Pinned at the unit level by `real_one`, which parses the wire bytes.\n\n"
        "Held together by `_Static_assert`s and (`crates/x/src/lib.rs`) paths.\n",
        encoding="utf-8",
    )
    (root / "CHANGELOG.md").write_text(
        "# Changelog\n\n## [Unreleased]\n\nPinned by `real_one`.\n\n## [0.1.0] - 2026-01-01\n\nPinned by `renamed_away_from_released`.\n",
        encoding="utf-8",
    )


def self_test() -> None:
    import tempfile

    with tempfile.TemporaryDirectory(prefix="enh057-selftest-") as tmp:
        root = Path(tmp)
        build_synthetic_tree(root)
        check(root)  # Baseline passes; `_Static_assert` and paths are ignored.

        # Drift: a docs citation renamed away fails, naming the file and line.
        (root / "docs/guide.md").write_text(
            "Pinned by `renamed_away`.\n", encoding="utf-8"
        )
        try:
            check(root)
        except SystemExit:
            pass
        else:
            fail("self-test: a dangling docs citation was not reported")

        # A CHANGELOG [Unreleased] citation fails too; the released section
        # and docs/opus/ stay out of scope.
        build_synthetic_tree(root)
        (root / "CHANGELOG.md").write_text(
            "# Changelog\n\n## [Unreleased]\n\nPinned by `renamed_away`.\n\n## [0.1.0] - 2026-01-01\n\nPinned by `also_gone`.\n",
            encoding="utf-8",
        )
        (root / "docs").mkdir(exist_ok=True)
        (root / "docs" / "opus").mkdir(exist_ok=True)
        (root / "docs" / "opus" / "x.md").write_text(
            "Pinned by `also_gone`.\n", encoding="utf-8"
        )
        buf = _capture(lambda: check(root), expect_failure=True)
        if "CHANGELOG.md" not in buf:
            fail("self-test: a dangling [Unreleased] citation was not reported")
        if "opus" in buf:
            fail("self-test: docs/opus/ must stay out of scope")

    # Fail closed: an empty .rs tree.
    with tempfile.TemporaryDirectory(prefix="enh057-selftest2-") as tmp:
        root = Path(tmp)
        (root / "docs").mkdir()
        (root / "docs" / "guide.md").write_text(
            "Pinned by `real_one`.\n", encoding="utf-8"
        )
        _capture(lambda: check(root), expect_failure=True, needle="parsed nothing")

    # Real tree: the grammar must keep matching the known baseline.
    names = test_names(ROOT)
    dangling, total = citations(ROOT, names)
    if total < 17:
        fail(
            f"self-test: the real tree yields only {total} citations; the grammar regressed"
        )
    if dangling:
        for where, line, name in dangling:
            print(
                f"{where}:{line}: pinned clause cites `{name}`, which is not a test fn or test target in the tree"
            )
        raise SystemExit(1)
    print(
        f"self-test ok: synthetic drifts reported, out-of-scope text ignored, "
        f"empty tree fails closed, real tree resolves {total} citations"
    )


def _capture(fn, expect_failure: bool, needle: str | None = None) -> str:
    import contextlib
    import io

    buf = io.StringIO()
    try:
        with contextlib.redirect_stdout(buf):
            fn()
    except SystemExit:
        if not expect_failure:
            fail(f"self-test: unexpected failure: {buf.getvalue()}")
        if needle is not None and needle not in buf.getvalue():
            fail(f"self-test: failure did not include {needle}: {buf.getvalue()}")
    else:
        if expect_failure:
            fail("self-test: expected a failure, got a pass")
    return buf.getvalue()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root", type=Path, default=ROOT, help="repository root to check"
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="build a synthetic tree, inject one drift per contract, and check the real tree",
    )
    args = parser.parse_args()
    if args.self_test:
        self_test()
    else:
        check(args.root)


if __name__ == "__main__":
    main()
