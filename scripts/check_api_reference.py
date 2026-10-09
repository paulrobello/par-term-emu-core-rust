#!/usr/bin/env python3
"""Check ``docs/API_REFERENCE.md`` parameter lists against ``_native.pyi``.

DOC-039: the reference drifted from the bindings — wrong parameter names,
phantom arguments, missing required ones. The stub is generated from the
compiled module (ARC-002), so it is ground truth for names and arity; this
checker keeps the prose honest.

DOC-064: names-only comparison let parameter-default drift through
(``record_cwd_change`` documented ``hostname=None`` against a required stub
param). Default PRESENCE is now compared too — never the values, which the
prose routinely paraphrases.

ENH-030: constructors and properties are checked too. The first
``ClassName(args)`` form — fenced code block or bullet — under a class
section is compared against the stub's ``__init__`` (now that the stub
generator renders real constructor signatures). ``### Properties``
subsections are checked bidirectionally against the stub's properties:
every documented name must be a stub property, and every stub property of
a class that has a Properties section must be documented.

DOC-134: a ``**Properties:**`` bold label (the Data Classes section's format)
opens a property section the same way; the next bold label or heading ends it.

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
# Constructor form inside a fenced ```python block: `Name(args)` on one
# line, or `Name(` opening a parameter-per-line block closed by `)`.
CTOR_LINE = re.compile(r"^([A-Za-z_]\w*)\((.*)$")
# Property bullet under a `### Properties` section: the backticked name up
# to `:` or the closing backtick (`- `max_clients: int` - …`).
PROPERTY_LINE = re.compile(r"^- `([A-Za-z_]\w*)")
# Table first column naming a property (``| `name` | … ``).
PROPERTY_TABLE_LINE = re.compile(r"^\|\s*`([A-Za-z_]\w*)`\s*\|")
# DOC-134: the Data Classes section labels property lists with a bold
# `**Properties:**` line (also `**Properties (all get/set):**` and
# `**Additional properties:**`) rather than a `### Properties` heading.
PROPERTY_LABEL = re.compile(r"^\*\*(?:Additional )?[Pp]roperties\b[^*]*:\*\*\s*$")
# Any bold label (`**Methods:**`, `**Constructor:** `X(...)``) ends a
# property list that a PROPERTY_LABEL opened.
BOLD_LABEL = re.compile(r"^\*\*[^*]+:\*\*")
FENCE = re.compile(r"^```")

# Stub properties intentionally absent from a `### Properties` section.
# Start empty; an entry needs a reason.
UNDOCUMENTED_PROPERTIES: dict[str, set[str]] = {}


def stub_param_names(
    func: ast.FunctionDef | ast.AsyncFunctionDef, is_method: bool
) -> list[tuple[str, bool]]:
    """(name, has_default) per parameter in definition order, self/cls dropped
    for bound methods. DOC-064: has_default drives the default-presence check;
    values are deliberately not compared."""
    a = func.args
    pos = a.posonlyargs + a.args
    # Defaults align to the tail of the positional parameters.
    has_default = [False] * len(pos)
    for i in range(len(a.defaults)):
        has_default[len(pos) - len(a.defaults) + i] = True
    pairs = [(arg.arg, has_default[i]) for i, arg in enumerate(pos)]
    if is_method and pairs:
        decorators = {
            d.id if isinstance(d, ast.Name) else getattr(d, "attr", "")
            for d in func.decorator_list
        }
        if not decorators & {"staticmethod"}:
            # Bound methods carry self (instance) or cls (classmethod) first.
            pairs = pairs[1:]
    if a.vararg:
        pairs.append((a.vararg.arg, False))
    pairs.extend(
        (arg.arg, d is not None) for arg, d in zip(a.kwonlyargs, a.kw_defaults)
    )
    if a.kwarg:
        pairs.append((a.kwarg.arg, False))
    return pairs


def parse_stub(
    path: Path,
) -> tuple[
    dict[str, dict[str, list[tuple[str, bool]]]],
    dict[str, set[str]],
    dict[str, list[tuple[str, bool]]],
]:
    """Return ({class: {method: params}}, {class: property names}, {module function: params})."""
    tree = ast.parse(path.read_text())
    classes: dict[str, dict[str, list[tuple[str, bool]]]] = {}
    properties: dict[str, set[str]] = {}
    functions: dict[str, list[tuple[str, bool]]] = {}
    for node in tree.body:
        if isinstance(node, ast.ClassDef):
            methods: dict[str, list[tuple[str, bool]]] = {}
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
                if n.name == "__init__":
                    # ENH-030: real constructor params (the stub generator
                    # renders them from the class-level __text_signature__).
                    methods["__init__"] = stub_param_names(n, is_method=True)
                    continue
                if not n.name.startswith("__"):
                    methods[n.name] = stub_param_names(n, is_method=True)
            classes[node.name] = methods
            properties[node.name] = props
        elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            functions[node.name] = stub_param_names(node, is_method=False)
    return classes, properties, functions


def doc_param_names(arg_text: str) -> list[tuple[str, bool]]:
    """Split a doc signature's argument list into (name, has_default) pairs,
    stripping default values and annotations."""
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

    pairs: list[tuple[str, bool]] = []
    for part in parts:
        # Strip a default value, then the annotation, each at bracket depth 0.
        # An `=` cut means the doc line shows a default (DOC-064 presence flag).
        has_default = False
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
                if sep == "=":
                    has_default = True
                part = part[:cut]
        name = part.strip().lstrip("*")
        if name:
            pairs.append((name, has_default))
    return pairs


def iter_doc_lines(path: Path):
    """Walk the doc tracking section scope and fenced-code state.

    Yields ``(lineno, scope, line, in_fence)`` where scope is the class
    name the current section documents (or ``"module"``/``None``) and
    ``in_fence`` says the line sits inside a ``` fence.
    """
    scope: str | None = None  # class name, "module", or None (skip)
    member_mode = False
    in_fence = False
    for lineno, line in enumerate(path.read_text().splitlines(), start=1):
        if FENCE.match(line):
            in_fence = not in_fence
            continue
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
            yield lineno, scope, line, in_fence
            continue
        h3 = HEADING3.match(line)
        if h3:
            heading = h3.group(1).strip()
            if heading in CLASS_SECTIONS:
                scope = CLASS_SECTIONS[heading]
            elif member_mode:
                scope = heading
            # Otherwise ### headings are topic groupings; keep the current scope.
            yield lineno, scope, line, in_fence
            continue
        yield lineno, scope, line, in_fence


def iter_doc_signatures(path: Path):
    """Yield (line_number, class_or_None, name, [doc param names]) per signature.

    Method bullets (``- `name(args)```), and — ENH-030 — constructor forms:
    the first fenced ``ClassName(args)`` block (single- or multi-line) per
    class section, plus ``- `ClassName(args)``` bullets.
    """
    lines = list(iter_doc_lines(path))
    seen_ctors: set[str] = set()
    for i, (lineno, scope, line, in_fence) in enumerate(lines):
        if scope is None or RUST_ONLY_MARK in line:
            continue
        if in_fence and scope != "module":
            m = CTOR_LINE.match(line)
            if m and m.group(1) == scope and scope not in seen_ctors:
                seen_ctors.add(scope)
                rest = m.group(2)
                # The name's opening '(' is consumed by the regex, so the
                # call starts out unbalanced by exactly one.
                depth = 1 + rest.count("(") - rest.count(")")
                arg_text = rest
                j = i
                while depth > 0 and j + 1 < len(lines):
                    j += 1
                    continuation = lines[j][2]
                    arg_text += " " + continuation
                    depth += continuation.count("(") - continuation.count(")")
                # Drop the trailing ')' that closes the call.
                arg_text = (
                    arg_text[: arg_text.rfind(")")] if ")" in arg_text else arg_text
                )
                yield (lineno, scope, scope, doc_param_names(arg_text))
                continue
        m = METHOD_LINE.match(line)
        if m:
            yield (
                lineno,
                None if scope == "module" else scope,
                m.group(1),
                doc_param_names(m.group(2)),
            )


def iter_doc_properties(path: Path):
    """Yield (section_lineno, line_number, class, property_name) under
    ``### Properties`` subsections (ENH-030) and ``**Properties:**`` bold
    labels (DOC-134). Bullets and table first-columns count."""
    in_props = False
    prop_cls: str | None = None
    heading_lineno = 0
    for lineno, scope, line, in_fence in iter_doc_lines(path):
        h3 = HEADING3.match(line)
        if h3:
            in_props = h3.group(1).strip().startswith("Properties")
            if in_props and scope not in (None, "module"):
                prop_cls = scope
                heading_lineno = lineno
            else:
                prop_cls = None
            continue
        h2 = HEADING2.match(line)
        if h2:
            in_props = False
            continue
        if not in_fence and PROPERTY_LABEL.match(line):
            in_props = scope not in (None, "module")
            if in_props:
                # A second label (`**Additional properties:**`) in the same
                # class keeps reporting against the first label's line.
                if prop_cls != scope:
                    heading_lineno = lineno
                prop_cls = scope
            else:
                prop_cls = None
            continue
        if not in_fence and BOLD_LABEL.match(line):
            in_props = False
            continue
        if not in_props or prop_cls is None:
            continue
        m = PROPERTY_LINE.match(line) or PROPERTY_TABLE_LINE.match(line)
        if m:
            yield heading_lineno, lineno, prop_cls, m.group(1)


def main() -> int:
    classes, properties, functions = parse_stub(STUB_PATH)
    problems: list[str] = []
    checked = 0
    for lineno, cls, name, doc_params in iter_doc_signatures(DOC_PATH):
        if cls and name == cls:
            # Constructor line (``- `ProgressBar(...)``` bullet or the
            # fenced ``ClassName(args)`` block). ENH-030: the stub's
            # __init__ now carries the real signature, so compare it —
            # except for the *args/**kwargs fallback classes, where the
            # stub is deliberately opaque.
            ctor = classes.get(cls, {}).get("__init__")
            if ctor is None:
                problems.append(
                    f"{DOC_PATH}:{lineno}: `{cls}` constructor documented "
                    f"but {STUB_PATH.name} has no __init__"
                )
                checked += 1
                continue
            if [n for n, _ in ctor][-2:] != ["args", "kwargs"]:
                checked += 1
                doc_names = [n for n, _ in doc_params]
                stub_names = [n for n, _ in ctor]
                if doc_names != stub_names:
                    problems.append(
                        f"{DOC_PATH}:{lineno}: `{cls}.__init__` params "
                        f"doc=({', '.join(doc_names)}) "
                        f"stub=({', '.join(stub_names)})"
                    )
                    continue
                for (dname, doc_has), (_, stub_has) in zip(doc_params, ctor):
                    if doc_has != stub_has:
                        drift = (
                            "doc shows a default, stub param has none"
                            if doc_has
                            else "stub param has a default, doc shows none"
                        )
                        problems.append(
                            f"{DOC_PATH}:{lineno}: `{cls}.__init__` param "
                            f"`{dname}` default drift: {drift}"
                        )
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
        doc_names = [n for n, _ in doc_params]
        stub_names = [n for n, _ in stub_params]
        if doc_names != stub_names:
            problems.append(
                f"{DOC_PATH}:{lineno}: `{where}` params doc=({', '.join(doc_names)}) "
                f"stub=({', '.join(stub_names)})"
            )
            continue
        for (dname, doc_has), (_, stub_has) in zip(doc_params, stub_params):
            if doc_has != stub_has:
                drift = (
                    "doc shows a default, stub param has none"
                    if doc_has
                    else "stub param has a default, doc shows none"
                )
                problems.append(
                    f"{DOC_PATH}:{lineno}: `{where}` param `{dname}` default drift: {drift}"
                )

    # ENH-030: `### Properties` sections, checked bidirectionally against
    # the stub — every documented name must be a stub property, and every
    # stub property of a class that documents a Properties section must be
    # listed (UNDOCUMENTED_PROPERTIES is the escape hatch).
    doc_props: dict[str, dict[str, int]] = {}
    section_lns: dict[str, int] = {}
    for section_ln, lineno, cls, pname in iter_doc_properties(DOC_PATH):
        doc_props.setdefault(cls, {})[pname] = lineno
        section_lns.setdefault(cls, section_ln)
    for cls, members in sorted(doc_props.items()):
        stub_props = properties.get(cls, set())
        for pname, lineno in sorted(members.items()):
            if pname not in stub_props:
                problems.append(
                    f"{DOC_PATH}:{lineno}: `{cls}.{pname}` documented under "
                    f"Properties but is not a property in {STUB_PATH.name}"
                )
                checked += 1
        allowed = UNDOCUMENTED_PROPERTIES.get(cls, set())
        for pname in sorted(stub_props - set(members) - allowed):
            problems.append(
                f"{DOC_PATH}:{section_lns[cls]}: stub property `{cls}.{pname}` "
                f"missing from this Properties section"
            )
            checked += 1

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
