#!/usr/bin/env python3
"""Gate docstring coverage of the generated ``_native.pyi`` stub (DOC-004).

The Rust side enforces ``missing_docs``; this is the Python-stub equivalent.
Every function in the stub must carry a docstring unless an allow-list rule
covers it. The rules are deliberately narrow so a new undocumented method or
property getter still fails:

- a property setter passes when its getter is documented (PyO3 attaches one
  ``__doc__`` to the property, and ``generate_stubs.py`` emits it on the getter);
- ``__init__`` passes when its class is documented (the class docstring is the
  constructor documentation);
- ``__enter__`` / ``__exit__`` pass by name (context-manager protocol).

Pure python3 stdlib, reading the committed stub, so it needs no build or venv.
Fix a failure in the binding source (``src/python_bindings/``) and regenerate
with ``make dev && make stubs``; never hand-edit the stub.

Usage:
    python3 scripts/check_stub_docstrings.py [STUB]
    python3 scripts/check_stub_docstrings.py --self-test
"""

from __future__ import annotations

import argparse
import ast
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
STUB_PATH = REPO_ROOT / "python" / "par_term_emu_core_rust" / "_native.pyi"

PROTOCOL_DUNDERS = {"__enter__", "__exit__"}


def _decorators(fn: ast.FunctionDef) -> list[str]:
    return [ast.unparse(d) for d in fn.decorator_list]


def check(source: str) -> tuple[int, int, list[str]]:
    """Return ``(total_defs, documented_defs, violations)`` for a stub source."""
    tree = ast.parse(source)
    total = documented = 0
    violations: list[str] = []

    def visit(
        fn: ast.FunctionDef, cls: ast.ClassDef | None, documented_getters: set[str]
    ) -> None:
        nonlocal total, documented
        total += 1
        if ast.get_docstring(fn):
            documented += 1
            return
        owner = f"{cls.name}." if cls else ""
        if cls is not None:
            if (
                any(d == f"{fn.name}.setter" for d in _decorators(fn))
                and fn.name in documented_getters
            ):
                return
            if fn.name == "__init__" and ast.get_docstring(cls):
                return
            if fn.name in PROTOCOL_DUNDERS:
                return
        violations.append(f"line {fn.lineno}: {owner}{fn.name}")

    for node in tree.body:
        if isinstance(node, ast.FunctionDef):
            visit(node, None, set())
        elif isinstance(node, ast.ClassDef):
            getters = {
                f.name
                for f in node.body
                if isinstance(f, ast.FunctionDef)
                and "property" in _decorators(f)
                and ast.get_docstring(f)
            }
            for f in node.body:
                if isinstance(f, ast.FunctionDef):
                    visit(f, node, getters)
    return total, documented, violations


def run_self_test() -> int:
    good = (
        'class A:\n    """Doc."""\n    def __init__(self) -> None: ...\n'
        "    def __enter__(self) -> A: ...\n"
        '    @property\n    def x(self) -> int:\n        """X."""\n'
        "    @x.setter\n    def x(self, value: int) -> None: ...\n"
        '    def m(self) -> None:\n        """M."""\n'
    )
    cases = {
        "undocumented method": good + "    def bad(self) -> None: ...\n",
        "undocumented getter": good + "    @property\n    def y(self) -> int: ...\n",
        "undocumented module function": good + "def f() -> None: ...\n",
        "__init__ of undocumented class": "class B:\n    def __init__(self) -> None: ...\n",
    }
    _, _, v = check(good)
    if v:
        print(f"self-test: FAIL — allow-listed baseline reported {v}")
        return 1
    print("self-test: allow-listed baseline passes")
    for label, src in cases.items():
        _, _, v = check(src)
        if not v:
            print(f"self-test: FAIL — {label} was not reported")
            return 1
        print(f"self-test: {label} reported")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("stub", nargs="?", type=Path, default=STUB_PATH)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return run_self_test()
    total, documented, violations = check(args.stub.read_text(encoding="utf-8"))
    print(
        f"stub docstrings: {documented}/{total} defs carry a docstring "
        f"({documented * 100 // total}%); {total - documented - len(violations)} allow-listed "
        f"(setters of documented getters, __init__ of documented classes, __enter__/__exit__)"
    )
    if violations:
        print(
            f"{len(violations)} undocumented stub defs (document them in src/python_bindings/, then make stubs):"
        )
        for v in violations:
            print(f"  {v}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
