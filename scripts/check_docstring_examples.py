#!/usr/bin/env python3
"""DOC-117 gate: extract and RUN `Example:` blocks from pyo3 binding docstrings.

Scans src/python_bindings/**/*.rs for `Example:` sections (fenced ```python blocks
or doctest-style `>>>` lines), executes every one against the built
par_term_emu_core_rust module, and reports `#[pymethods]` methods whose docstring
lacks an Example section.

Exits 1 when any Example fails, when the module cannot be imported, or when no
Example ran. On the default whole-tree scan it also exits 1 when the number of
documented pymethods without an Example exceeds MISSING_EXAMPLE_FLOOR (audit
DOC-002 ratchet): add Examples, then lower the floor to the new count. The floor
only goes down. `#[setter]` methods are not counted: PyO3 exposes one `__doc__`
per property, the getter's, so a setter Example is never shown to Python users.
Every Example must run against the real module (see vault pattern
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

# Ratchet ceiling for documented pymethods lacking an Example on the default
# scan (audit DOC-002). Lower it whenever the count drops; never raise it.
MISSING_EXAMPLE_FLOOR = 277

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
        if line.strip() == "#[setter]":
            continue
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
            is_setter = False
            while j >= 0:
                s = lines[j].lstrip()
                if s.startswith("#[setter]"):
                    is_setter = True
                if s.startswith("///"):
                    has_doc = True
                    if "Example" in s:
                        has_example = True
                    j -= 1
                elif s.startswith(("//", "#[")) or s == "":
                    j -= 1
                else:
                    break
            if is_setter:
                pass
            elif has_doc and has_example:
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
) -> tuple[int, list[tuple[str, str]], list[str]] | None:
    """Execute every extracted Example block; return (total, failures, missing).

    Returns None when the module cannot be imported.
    """
    try:
        import par_term_emu_core_rust as m
    except Exception as e:  # noqa: BLE001 - environment guard, lint must not crash
        print(f"ERROR: cannot import par_term_emu_core_rust ({e})")
        return None

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
        help="accepted for compatibility; failing examples always exit 1",
    )
    ap.add_argument(
        "paths",
        nargs="*",
        help="files or directories to scan (default: src/python_bindings)",
    )
    args = ap.parse_args()
    targets = [Path(p).resolve() for p in args.paths] if args.paths else [BINDINGS]
    files: list[Path] = []
    for t in targets:
        if t.is_dir():
            files.extend(sorted(t.rglob("*.rs")))
        else:
            files.append(t)

    result = run_examples(files)
    if result is None:
        return 1
    total, failures, missing = result
    print(f"docstring-examples: ran {total} Example blocks across {len(files)} files")
    for label, err in failures:
        print(f"FAIL {label}: {err}")
    if missing:
        print(
            f"docstring-examples: {len(missing)} documented pymethods lack an Example:"
        )
        for name in missing:
            print(f"  MISSING {name}")
    status = 0
    if failures:
        print(f"ERROR: {len(failures)} failing docstring example(s)")
        status = 1
    if total == 0:
        print("ERROR: no Example blocks ran")
        status = 1
    if not args.paths:
        if len(missing) > MISSING_EXAMPLE_FLOOR:
            print(
                f"ERROR: {len(missing)} documented pymethods lack an Example, above "
                f"the ratchet floor of {MISSING_EXAMPLE_FLOOR}; add Examples to the "
                "new methods"
            )
            status = 1
        elif len(missing) < MISSING_EXAMPLE_FLOOR:
            print(
                f"note: missing-Example count {len(missing)} is below the floor "
                f"{MISSING_EXAMPLE_FLOOR}; lower MISSING_EXAMPLE_FLOOR in "
                f"{Path(__file__).name} to {len(missing)}"
            )
    return status


if __name__ == "__main__":
    sys.exit(main())
