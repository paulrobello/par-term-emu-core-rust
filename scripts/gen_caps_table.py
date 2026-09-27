#!/usr/bin/env python3
"""Generate the resource-limits table in docs/SECURITY.md from source.

Every security-relevant size constant in `src/` carries a `/// cap: <what it
bounds>` doc comment on (or in the doc block of) its declaration:

    /// cap: Max decompressed payload accepted from one client message.
    const MAX_DECOMPRESSED_SIZE: usize = 1024 * 1024;

This script scans `src/**/*.rs` for those comments, evaluates the constant's
value with a restricted arithmetic evaluator (no `eval`), renders byte-sized
caps in human units, and writes a Markdown table between the markers in
docs/SECURITY.md. `--check` exits non-zero when the file would change, so
`make caps-table-check` gates drift between the doc and the code.
"""

from __future__ import annotations

import argparse
import ast
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DOC = ROOT / "docs" / "SECURITY.md"
START_MARKER = "<!-- caps-table:start -->"
END_MARKER = "<!-- caps-table:end -->"

# A `/// cap:` line, then any further doc lines/attributes, then the const.
CAP_DECL = re.compile(
    r"^[ \t]*/// cap: (?P<desc>.+?)\s*$\n"
    r"(?:(?!^[ \t]*/// cap: )[ \t]*///.*\n|[ \t]*#!?\[.*\]\n)*"
    r"^[ \t]*(?:(?P<vis>pub(?:\([^)]*\))?\s+)?)const\s+(?P<name>[A-Z0-9_]+)"
    r"\s*:\s*(?P<ty>\w+)\s*=\s*(?P<value>[^;\n]+);",
    re.MULTILINE,
)

INT_TYPES = {"usize", "u64", "u32", "u16", "u8", "isize", "i64", "i32", "i16", "i8"}
BYTEISH = re.compile(r"BYTES|SIZE|LENGTH|WIDTH|HEIGHT|DIMENSION|CELLS", re.IGNORECASE)


def eval_int(expr: str) -> int | None:
    """Evaluate a Rust integer literal expression (digits, _, + - * / parens,
    shifts) without eval(). Returns None for anything else."""
    if not re.fullmatch(r"[0-9_+\-*/()<>\s]+", expr):
        return None

    def walk(node: ast.AST) -> int | None:
        if isinstance(node, ast.Expression):
            return walk(node.body)
        if isinstance(node, ast.Constant) and isinstance(node.value, int):
            return node.value
        if isinstance(node, ast.UnaryOp) and isinstance(node.op, (ast.UAdd, ast.USub)):
            val = walk(node.operand)
            if val is None:
                return None
            return -val if isinstance(node.op, ast.USub) else val
        if isinstance(node, ast.BinOp):
            left, right = walk(node.left), walk(node.right)
            if left is None or right is None:
                return None
            if isinstance(node.op, ast.Add):
                return left + right
            if isinstance(node.op, ast.Sub):
                return left - right
            if isinstance(node.op, ast.Mult):
                return left * right
            if isinstance(node.op, ast.FloorDiv):
                return left // right if right else None
            if isinstance(node.op, ast.LShift):
                return left << right
            return None
        return None

    try:
        return walk(ast.parse(expr, mode="eval"))
    except SyntaxError:
        return None


def humanize(name: str, value: int) -> str:
    """Byte-ish caps render in KiB/MiB (raw value alongside when it rounds);
    counts and non-divisible byte values stay plain."""
    if BYTEISH.search(name) and value >= 1024:
        mib, kib = value / (1 << 20), value / (1 << 10)
        if mib >= 1 and value % (1 << 20) == 0:
            return f"{mib:g} MiB"
        if kib >= 1 and value % (1 << 10) == 0:
            return f"{kib:g} KiB"
    return f"{value:,}"


def collect_caps() -> list[dict]:
    caps: list[dict] = []
    for path in sorted((ROOT / "src").rglob("*.rs")):
        text = path.read_text(encoding="utf-8")
        for m in CAP_DECL.finditer(text):
            value = eval_int(m.group("value"))
            if m.group("ty") not in INT_TYPES or value is None:
                # A cap: comment whose value is not a plain integer literal
                # still documents the bound — render the source expression.
                value_repr = m.group("value").strip()
                render = f"`{value_repr}`"
            else:
                render = humanize(m.group("name"), value)
            line = text[: m.start()].count("\n") + 1
            rel = path.relative_to(ROOT)
            caps.append(
                {
                    "name": m.group("name"),
                    "value": render,
                    "loc": f"{rel}:{line}",
                    "desc": m.group("desc").strip(),
                }
            )
    return caps


def render_table(caps: list[dict]) -> str:
    lines = [
        "| Constant | Value | Location | Bounds |",
        "|----------|-------|----------|--------|",
    ]
    for cap in caps:
        desc = cap["desc"].replace("|", "\\|")
        lines.append(f"| `{cap['name']}` | {cap['value']} | `{cap['loc']}` | {desc} |")
    return "\n".join(lines)


def splice(doc: str, table: str) -> str:
    start = doc.index(START_MARKER) + len(START_MARKER)
    end = doc.index(END_MARKER)
    return doc[:start] + "\n\n" + table + "\n\n" + doc[end:]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="exit non-zero (and print a diff summary) if SECURITY.md would change",
    )
    args = parser.parse_args()

    caps = collect_caps()
    doc = DOC.read_text(encoding="utf-8")
    if START_MARKER not in doc or END_MARKER not in doc:
        print(
            f"error: {DOC} lacks the {START_MARKER}/{END_MARKER} markers",
            file=sys.stderr,
        )
        return 2
    updated = splice(doc, render_table(caps))
    if updated == doc:
        print(f"caps table up to date ({len(caps)} caps)")
        return 0
    if args.check:
        print(
            f"error: docs/SECURITY.md caps table is stale ({len(caps)} caps found in src/); "
            "run `make caps-table`",
            file=sys.stderr,
        )
        return 1
    DOC.write_text(updated, encoding="utf-8")
    print(f"wrote {len(caps)} caps to {DOC}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
