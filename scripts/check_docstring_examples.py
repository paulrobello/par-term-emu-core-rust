#!/usr/bin/env python3
"""DOC-117 warn-level lint: extract and RUN `Example:` blocks from pyo3 binding docstrings.

Scans src/python_bindings/**/*.rs for `Example:` sections (fenced ```python blocks
or doctest-style `>>>` lines), executes every one against the built
par_term_emu_core_rust module, and reports `#[pymethods]` methods whose docstring
lacks an Example section.

Warn-level: always exits 0 so it never gates CI; pass --strict to exit 1 on any
failing example. Failures should be treated as real defects in doc examples —
every Example must run against the real module (see vault pattern
"Verified Docstring Examples as API Probes").
"""

from __future__ import annotations

import argparse
import doctest
import re
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
BINDINGS = ROOT / "src" / "python_bindings"

# Deferred DOC-117 wave: pty.rs/streaming.rs examples need live processes and
# sockets; they fail loudly here until that wave lands. Pass them explicitly as
# path arguments to run them.
DEFERRED = {"pty.rs", "streaming.rs"}

FN_RE = re.compile(r"^(\s*)(?:pub(?:\([^)]*\))?\s+)?fn (\w+)")
DOC_RE = re.compile(r"^(\s*)///(?: (.*))?$")


def scan_file(path: Path) -> tuple[list[dict[str, Any]], int, list[str]]:
    """Return (example blocks, count of documented pymethods with Example, missing names)."""
    lines = path.read_text().splitlines()
    examples: list[dict[str, Any]] = []
    covered = 0
    missing: list[str] = []
    last_pymethods = -1
    last_impl_close = -1
    for i, line in enumerate(lines):
        stripped = line.strip()
        if stripped == "#[pymethods]":
            last_pymethods = i
        elif line.startswith("}"):
            last_impl_close = i
        m = FN_RE.match(line)
        if m and last_pymethods > last_impl_close:
            name = m.group(2)
            if name.startswith("test_") or (
                name.startswith("__") and name != "__repr__"
            ):
                continue
            j = i - 1
            has_doc = False
            has_example = False
            while j >= 0:
                s = lines[j].lstrip()
                if s.startswith("///"):
                    has_doc = True
                    if "Example" in s:
                        has_example = True
                    j -= 1
                elif s.startswith(("//", "#[")) or s == "":
                    j -= 1
                else:
                    break
            if has_doc and has_example:
                covered += 1
            elif has_doc:
                missing.append(f"{path.relative_to(ROOT)}:{i + 1} {name}")
        dm = DOC_RE.match(line)
        if dm and (dm.group(2) or "").strip() == "Example:":
            block: list[str] = []
            kind: str | None = first_block_kind(lines, i)
            if kind is None:
                continue
            j = i + 1
            if kind == "fenced":
                j += 1  # skip the ```python line
            while j < len(lines):
                d = DOC_RE.match(lines[j])
                if not d:
                    break
                content = d.group(2) or ""
                cs = content.strip()
                if kind == "fenced":
                    if cs == "```":
                        j += 1
                        break
                    # strip the 4-space doc indent; keep deeper continuation indents
                    block.append(content.removeprefix("    "))
                    j += 1
                else:
                    if cs == "":
                        break
                    block.append(content)
                    j += 1
            if block:
                examples.append(
                    {
                        "fn": nearest_fn(lines, i),
                        "line": i + 1,
                        "kind": kind,
                        "code": "\n".join(block),
                    }
                )
    return examples, covered, missing


def first_block_kind(lines: list[str], example_idx: int) -> str | None:
    """Classify the block following an `Example:` doc line (fenced python or doctest)."""
    for k in range(example_idx + 1, min(example_idx + 4, len(lines))):
        d = DOC_RE.match(lines[k])
        if not d:
            return None
        cs = (d.group(2) or "").strip()
        if cs.startswith("```python"):
            return "fenced"
        if cs.startswith(">>>"):
            return "doctest"
        if cs == "":
            continue
        return None
    return None


def nearest_fn(lines: list[str], from_idx: int) -> str:
    """Nearest fn name at or after the Example: line (the documented method)."""
    for k in range(from_idx, min(from_idx + 120, len(lines))):
        m = FN_RE.match(lines[k])
        if m:
            return m.group(2)
    return "?"


def run_examples(
    files: list[Path],
) -> tuple[int, list[tuple[str, str]], list[str]]:
    """Execute every extracted Example block; return (total, failures, missing)."""
    try:
        import par_term_emu_core_rust as m
    except Exception as e:  # noqa: BLE001 - environment guard, lint must not crash
        print(f"WARN: cannot import par_term_emu_core_rust ({e}); skipping run phase")
        return 0, [], []

    failures: list[tuple[str, str]] = []
    total = 0
    missing: list[str] = []
    for f in files:
        try:
            examples, _covered, file_missing = scan_file(f)
        except Exception as e:  # noqa: BLE001 - lint must survive bad docstrings
            failures.append((str(f), f"scan error: {type(e).__name__}: {e}"))
            continue
        missing.extend(file_missing)
        for ex in examples:
            total += 1
            try:
                rel = f.relative_to(ROOT)
            except ValueError:
                rel = f
            label = f"{rel}::{ex['fn']}:{ex['line']}"
            # every module-exported name resolves (enums, types, free functions)
            globs: dict[str, Any] = {
                **vars(m),
                "term": m.Terminal(80, 24),
                "terminal": m.Terminal(80, 24),
            }
            try:
                if ex["kind"] == "fenced":
                    code = compile(ex["code"], f"<{label}>", "exec")
                    exec(code, globs)  # noqa: S102 - the whole point of this lint
                else:
                    parser = doctest.DocTestParser()
                    test = parser.get_doctest(
                        ex["code"], globs, label, str(f), ex["line"]
                    )
                    runner = doctest.DocTestRunner(optionflags=doctest.ELLIPSIS)
                    runner.run(test)
                    if runner.failures:
                        failures.append(
                            (label, f"{runner.failures} doctest failure(s)")
                        )
                        continue
            except Exception as e:  # noqa: BLE001 - lint must survive bad docstrings
                failures.append((label, f"{type(e).__name__}: {e}"))
    return total, failures, missing


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--strict",
        action="store_true",
        help="exit 1 on any failing example (default: warn-level, always exit 0)",
    )
    ap.add_argument(
        "paths",
        nargs="*",
        help="files or directories to scan (default: src/python_bindings)",
    )
    args = ap.parse_args()
    targets = [Path(p) for p in args.paths] if args.paths else [BINDINGS]
    files: list[Path] = []
    for t in targets:
        if t.is_dir():
            files.extend(sorted(t.rglob("*.rs")))
        else:
            files.append(t)
    if not args.paths:
        files = [f for f in files if f.name not in DEFERRED]
        print(
            "note: pty.rs/streaming.rs excluded (deferred DOC-117 wave; pass paths to include)"
        )

    total, failures, missing = run_examples(files)
    print(f"docstring-examples: ran {total} Example blocks across {len(files)} files")
    for label, err in failures:
        print(f"FAIL {label}: {err}")
    if missing:
        print(
            f"docstring-examples: {len(missing)} documented pymethods lack an Example:"
        )
        for name in missing:
            print(f"  MISSING {name}")
    if failures:
        print("WARN: failing docstring examples present (DOC-117 warn-level lint)")
    return 1 if (args.strict and failures) else 0


if __name__ == "__main__":
    sys.exit(main())
