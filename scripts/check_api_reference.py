#!/usr/bin/env python3
"""Check ``docs/API_REFERENCE.md`` parameter lists against ``_native.pyi``.

DOC-039: the reference drifted from the bindings — wrong parameter names,
phantom arguments, missing required ones. The stub is generated from the
compiled module (ARC-002), so it is ground truth for names and arity; this
checker keeps the prose honest.

For every ``- `name(args)``` list item, the parameter names (order-sensitive,
defaults and annotations stripped) must match the stub's signature for the
enclosing class — or, failing that, a module-level function, since some
sections under a class heading document free functions.

Usage:

    uv run python scripts/check_api_reference.py

Exits 1 with one line per mismatch when any drift is found.
"""

from __future__ import annotations

import ast
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
STUB_PATH = REPO_ROOT / "python" / "par_term_emu_core_rust" / "_native.pyi"
DOC_PATH = REPO_ROOT / "docs" / "API_REFERENCE.md"

# ``## X`` sections that document pyi classes. Anything not listed here is
# either prose (Table of Contents, See Also), pure-Python modules the stub
# does not cover (Observer Convenience Functions -> observers.py), or Rust-
# level reference material (Instant Replay, C-Compatible FFI).
CLASS_SECTIONS = {
    "Terminal Class": "Terminal",
    "PtyTerminal Class": "PtyTerminal",
    "StreamingServer Class": "StreamingServer",
    "StreamingConfig Class": "StreamingConfig",
    # A ### subsection that drifted under ## Enumerations but documents
    # Terminal methods; resolved explicitly so member-mode does not eat it.
    "Named Progress Bars (OSC 934)": "Terminal",
}
MODULE_SECTIONS = {"Color Utilities", "Streaming Functions"}
# Under these sections each ``### Name`` subsection is itself a class.
MEMBER_SECTION_HEADINGS = {"Data Classes", "Enumerations"}
# Documented Rust-only accessors; no Python counterpart exists in the stub.
RUST_ONLY_MARK = "(Rust API only)"

# Matches both doc styles: `- `name(args)`: …` and `- `name(args) -> ret`: …`.
METHOD_LINE = re.compile(r"^- `\**([A-Za-z_]\w*)\(([^)]*)\)")
HEADING2 = re.compile(r"^## (.+)$")
HEADING3 = re.compile(r"^### (.+)$")


def stub_param_names(
    func: ast.FunctionDef | ast.AsyncFunctionDef, is_method: bool
) -> list[str]:
    """Parameter names in definition order, self/cls dropped for bound methods."""
    a = func.args
    names = [arg.arg for arg in a.posonlyargs + a.args]
    if is_method and names:
        decorators = {
            d.id if isinstance(d, ast.Name) else getattr(d, "attr", "")
            for d in func.decorator_list
        }
        if not decorators & {"staticmethod"}:
            # Bound methods carry self (instance) or cls (classmethod) first.
            names = names[1:]
    if a.vararg:
        names.append(a.vararg.arg)
    names.extend(arg.arg for arg in a.kwonlyargs)
    if a.kwarg:
        names.append(a.kwarg.arg)
    return names


def parse_stub(
    path: Path,
) -> tuple[dict[str, dict[str, list[str]]], dict[str, set[str]], dict[str, list[str]]]:
    """Return ({class: {method: params}}, {class: property names}, {module function: params})."""
    tree = ast.parse(path.read_text())
    classes: dict[str, dict[str, list[str]]] = {}
    properties: dict[str, set[str]] = {}
    functions: dict[str, list[str]] = {}
    for node in tree.body:
        if isinstance(node, ast.ClassDef):
            methods: dict[str, list[str]] = {}
            props: set[str] = set()
            for n in node.body:
                if not isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef)):
                    continue
                decorators = {
                    d.id if isinstance(d, ast.Name) else getattr(d, "attr", "")
                    for d in n.decorator_list
                }
                if decorators & {"property", "setter", "getter"}:
                    # @property plus its generated @name.setter/@name.getter
                    # pairs; neither is callable documentation surface.
                    props.add(n.name)
                    continue
                if not n.name.startswith("__"):
                    methods[n.name] = stub_param_names(n, is_method=True)
            classes[node.name] = methods
            properties[node.name] = props
        elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            functions[node.name] = stub_param_names(node, is_method=False)
    return classes, properties, functions


def doc_param_names(arg_text: str) -> list[str]:
    """Split a doc signature's argument list, stripping defaults and annotations."""
    arg_text = arg_text.strip()
    if not arg_text:
        return []
    parts: list[str] = []
    depth = 0
    current = ""
    for ch in arg_text:
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth -= 1
        if ch == "," and depth == 0:
            parts.append(current)
            current = ""
        else:
            current += ch
    parts.append(current)

    names = []
    for part in parts:
        # Strip a default value, then the annotation, each at bracket depth 0.
        for sep in ("=", ":"):
            depth = 0
            cut = None
            for i, ch in enumerate(part):
                if ch in "([{":
                    depth += 1
                elif ch in ")]}":
                    depth -= 1
                elif ch == sep and depth == 0:
                    cut = i
                    break
            if cut is not None:
                part = part[:cut]
        name = part.strip().lstrip("*")
        if name:
            names.append(name)
    return names


def iter_doc_signatures(path: Path):
    """Yield (line_number, class_or_None, name, [doc param names]) per signature."""
    scope: str | None = None  # class name, "module", or None (skip)
    member_mode = False
    for lineno, line in enumerate(path.read_text().splitlines(), start=1):
        h2 = HEADING2.match(line)
        if h2:
            section = h2.group(1).strip()
            member_mode = section in MEMBER_SECTION_HEADINGS
            if section in CLASS_SECTIONS:
                scope = CLASS_SECTIONS[section]
            elif section in MODULE_SECTIONS:
                scope = "module"
            else:
                scope = None
            continue
        h3 = HEADING3.match(line)
        if h3:
            heading = h3.group(1).strip()
            if heading in CLASS_SECTIONS:
                scope = CLASS_SECTIONS[heading]
            elif member_mode:
                scope = heading
            # Otherwise ### headings are topic groupings; keep the current scope.
            continue
        if scope is None or RUST_ONLY_MARK in line:
            continue
        m = METHOD_LINE.match(line)
        if m:
            yield (
                lineno,
                None if scope == "module" else scope,
                m.group(1),
                doc_param_names(m.group(2)),
            )


def main() -> int:
    classes, properties, functions = parse_stub(STUB_PATH)
    problems: list[str] = []
    checked = 0
    for lineno, cls, name, doc_params in iter_doc_signatures(DOC_PATH):
        if cls and name == cls:
            # Constructor line (``- `ProgressBar(...)``); the stub's __init__
            # is opaque (*args/**kwargs, ARC-002), so it cannot be validated.
            continue
        if cls and name in properties.get(cls, set()):
            problems.append(
                f"{DOC_PATH}:{lineno}: `{cls}.{name}` documented with call syntax "
                f"but is a property in {STUB_PATH.name}"
            )
            checked += 1
            continue
        if cls and name in classes.get(cls, {}):
            stub_params = classes[cls][name]
            where = f"{cls}.{name}"
        elif name in functions:
            stub_params = functions[name]
            where = name
        else:
            problems.append(
                f"{DOC_PATH}:{lineno}: `{name}` documented under "
                f"{cls or 'module scope'} but not found in {STUB_PATH.name}"
            )
            checked += 1
            continue
        checked += 1
        if doc_params != stub_params:
            problems.append(
                f"{DOC_PATH}:{lineno}: `{where}` params doc=({', '.join(doc_params)}) "
                f"stub=({', '.join(stub_params)})"
            )

    if problems:
        print(
            f"API_REFERENCE.md drift: {len(problems)} of {checked} signatures mismatch "
            f"against {STUB_PATH.name}:"
        )
        for p in problems:
            print(f"  {p}")
        return 1
    print(f"OK: {checked} documented signatures match {STUB_PATH.name}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
