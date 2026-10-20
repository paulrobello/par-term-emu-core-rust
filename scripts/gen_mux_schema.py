#!/usr/bin/env python3
"""Generate the par-mux command schema (H-API parity) from the code.

The schema is a machine-readable description of the control-command surface,
written to crates/par-mux/schema/par-mux-commands.json and embedded in the
par-mux crate (`par_mux::mux::command::COMMAND_SCHEMA_JSON`), so the daemon
serves it (`list-commands -J`) and the CLI prints it (`par-mux --print-schema`)
without a second hand-kept table. Inputs:

  * `const COMMANDS` (crates/par-mux/src/mux/command/) — every command name and
    its feature tokens (the table's third column). The single source of truth
    for WHICH commands exist and WHAT they advertise.
  * `list_commands_body()` — the daemon-level `features …` trailing line.
  * docs/MUX.md "Command Reference" — each command's Arguments cell, which
    `make mux-docs-check` already ties 1:1 to the COMMANDS table. It supplies
    the human-readable `synopsis` and the `flags` names spelled in it.

Format (version 1): additive keys keep `version`; removing or renaming a key
bumps it. Consumers must ignore unknown keys.

`--check` exits non-zero when the committed file differs from what the code
generates (the `make mux-schema-check` gate); `--self-test` proves the gate
fails on injected drift. Every extraction fails closed: a regex that matched
nothing is a broken gate, not a clean pass.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import sys
import tempfile
from pathlib import Path
from typing import NoReturn

SCHEMA_VERSION = 1
FORMAT = "par-mux-command-schema"
COMMAND_DIR = "crates/par-mux/src/mux/command/"
MUX_MD = "docs/MUX.md"
OUTPUT = "crates/par-mux/schema/par-mux-commands.json"

# One COMMANDS row: `("name", parser, &["tok", …])`, possibly split across
# lines by rustfmt.
ROW_RE = re.compile(r'\(\s*"([a-z][a-z0-9-]*)",\s*(\w+),\s*&\[([^\]]*)\]\s*,?\s*\)')
TOKEN_RE = re.compile(r'"([^"]*)"')
FEATURE_TOKEN_RE = re.compile(r"[a-z0-9-]+")
DAEMON_FEATURES_RE = re.compile(r'"features ([a-z0-9 -]+)"')
# A flag inside an Arguments code span: `-t`, `-L`, `--enter`. Not `--`
# alone (the end-of-flags separator) and not a dash inside a word (`1-99`).
FLAG_RE = re.compile(r"(?<![\w-])(--?[A-Za-z][A-Za-z0-9-]*)")
CODE_SPAN_RE = re.compile(r"`([^`]*)`")


def fail(msg: str) -> NoReturn:
    raise SystemExit(f"error: {msg}")


def read_command_rs(root: Path) -> str:
    d = root / COMMAND_DIR
    try:
        files = [d / "mod.rs"] + sorted(
            f for f in d.glob("*.rs") if f.name not in ("mod.rs", "tests.rs")
        )
        return "\n".join(f.read_text(encoding="utf-8") for f in files)
    except OSError as exc:
        fail(f"parsed nothing from {COMMAND_DIR}: cannot read ({exc})")


def fn_block(text: str, marker: str) -> str:
    start = text.find(marker)
    if start < 0:
        fail(f"parsed nothing from {COMMAND_DIR}: `{marker}` not found")
    open_brace = text.find("{", start)
    depth = 0
    for i in range(open_brace, len(text)):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                return text[open_brace : i + 1]
    fail(f"parsed nothing from {COMMAND_DIR}: unbalanced braces after `{marker}`")


def parse_commands(command_rs: str) -> list[tuple[str, list[str]]]:
    start = command_rs.find("const COMMANDS")
    if start < 0:
        fail(f"parsed nothing from {COMMAND_DIR}: `const COMMANDS` not found")
    end = command_rs.find("];", start)
    if end < 0:
        fail(f"parsed nothing from {COMMAND_DIR}: unterminated `const COMMANDS`")
    table = command_rs[start:end]
    rows = ROW_RE.findall(table)
    if not rows:
        fail(f"parsed nothing from {COMMAND_DIR}: no rows inside `const COMMANDS`")
    # Every `("name",` opener must have parsed as a full row, or a row shape
    # this regex does not know would silently drop a command.
    openers = re.findall(r'\(\s*"([a-z][a-z0-9-]*)",', table)
    if len(openers) != len(rows):
        missing = sorted(set(openers) - {name for name, _, _ in rows})
        fail(f"COMMANDS rows not parsed as (name, parser, &[tokens]): {missing}")
    commands: list[tuple[str, list[str]]] = []
    for name, _parser, tokens_src in rows:
        tokens = TOKEN_RE.findall(tokens_src)
        for token in tokens:
            if not FEATURE_TOKEN_RE.fullmatch(token):
                fail(f"command `{name}`: feature token {token!r} is not [a-z0-9-]+")
        commands.append((name, tokens))
    names = [name for name, _ in commands]
    dups = sorted({n for n in names if names.count(n) > 1})
    if dups:
        fail(f"duplicate COMMANDS rows: {dups}")
    return commands


def parse_daemon_features(command_rs: str) -> list[str]:
    body = fn_block(command_rs, "fn list_commands_body(")
    m = DAEMON_FEATURES_RE.search(body)
    if not m:
        fail('list_commands_body() no longer pushes a `"features …"` line')
    return m.group(1).split()


def split_row(line: str) -> list[str]:
    """Cells of a Markdown table row, honoring `\\|` escapes."""
    cells = re.split(r"(?<!\\)\|", line.strip())
    return [c.strip() for c in cells[1:-1]]


def doc_arguments(mux_md: str) -> dict[str, str]:
    m = re.search(
        r"^## Command Reference\s*$(.*?)(?=^## |\Z)", mux_md, re.MULTILINE | re.DOTALL
    )
    if not m:
        fail(f"parsed nothing from {MUX_MD}: no `## Command Reference` section")
    args: dict[str, str] = {}
    for line in m.group(1).splitlines():
        if not re.match(r"^\| `[a-z][a-z0-9-]*` \|", line):
            continue
        cells = split_row(line)
        if len(cells) < 2:
            fail(f"{MUX_MD}: malformed Command Reference row: {line[:60]}")
        name = cells[0].strip("`")
        args[name] = cells[1]
    if not args:
        fail(f"parsed nothing from {MUX_MD}: no rows in the Command Reference table")
    return args


def synopsis(cell: str) -> str:
    text = cell.replace("\\|", "|").replace("`", "")
    return "" if text in ("—", "-") else text


def flags(cell: str, command_names: set[str]) -> list[str]:
    found: list[str] = []
    for span in CODE_SPAN_RE.findall(cell.replace("\\|", "|")):
        first = span.split(maxsplit=1)[0] if span.strip() else ""
        # A span quoting another command (`send-keys -H`) names that
        # command's flags, not this one's.
        if first in command_names:
            continue
        for flag in FLAG_RE.findall(span):
            if flag not in found:
                found.append(flag)
    return sorted(found, key=lambda f: (f.lstrip("-").lower(), f))


def build_schema(root: Path) -> dict:
    command_rs = read_command_rs(root)
    commands = parse_commands(command_rs)
    daemon_features = parse_daemon_features(command_rs)
    try:
        mux_md = (root / MUX_MD).read_text(encoding="utf-8")
    except OSError as exc:
        fail(f"parsed nothing from {MUX_MD}: cannot read ({exc})")
    doc_args = doc_arguments(mux_md)
    names = {name for name, _ in commands}
    undocumented = sorted(names - doc_args.keys())
    if undocumented:
        fail(
            f"commands without a {MUX_MD} Command Reference row: {undocumented} "
            "(add the row, then run `make mux-schema`)"
        )
    return {
        "version": SCHEMA_VERSION,
        "format": FORMAT,
        "source": "crates/par-mux/src/mux/command/mod.rs COMMANDS + docs/MUX.md Command Reference",
        "daemon_features": daemon_features,
        "commands": [
            {
                "name": name,
                "features": tokens,
                "flags": flags(doc_args[name], names),
                "synopsis": synopsis(doc_args[name]),
            }
            for name, tokens in sorted(commands)
        ],
    }


def render(schema: dict) -> str:
    return json.dumps(schema, indent=2, ensure_ascii=False) + "\n"


def check(root: Path) -> list[str]:
    """Problems with the committed schema; empty when it is current."""
    expected = render(build_schema(root))
    path = root / OUTPUT
    try:
        actual = path.read_text(encoding="utf-8")
    except OSError:
        return [f"{OUTPUT} is missing — run: make mux-schema"]
    if actual != expected:
        return [f"{OUTPUT} is stale against the COMMANDS table — run: make mux-schema"]
    return []


def run_self_test(root: Path) -> int:
    inputs = (COMMAND_DIR, MUX_MD, OUTPUT)

    def copy(dst: Path) -> None:
        for rel in inputs:
            target = dst / rel
            if rel.endswith("/"):
                shutil.copytree(root / rel, target)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(root / rel, target)

    with tempfile.TemporaryDirectory() as td:
        dst = Path(td)
        copy(dst)
        problems = check(dst)
        if problems:
            print(
                "self-test: baseline copy failed:",
                *problems,
                sep="\n  ",
                file=sys.stderr,
            )
            return 1
    print("self-test: baseline copy passes")

    mod_rs = COMMAND_DIR + "mod.rs"
    drifts = [
        (
            "add a feature token to the `version` row",
            lambda t: t.replace(
                '("version", parse_version, &[]),',
                '("version", parse_version, &["drift"]),',
                1,
            ),
            "stale",
        ),
        (
            "add an undocumented fake-cmd row",
            lambda t: t.replace(
                '("version", parse_version, &[]),',
                '("version", parse_version, &[]),\n    ("fake-cmd", parse_version, &[]),',
                1,
            ),
            "fake-cmd",
        ),
        (
            "rename the COMMANDS table",
            lambda t: t.replace("const COMMANDS", "const COMMAND_ROWS", 1),
            "const COMMANDS",
        ),
    ]
    for label, mutate, expect in drifts:
        with tempfile.TemporaryDirectory() as td:
            dst = Path(td)
            copy(dst)
            target = dst / mod_rs
            before = target.read_text(encoding="utf-8")
            after = mutate(before)
            if after == before:
                fail(f"self-test: mutation '{label}' did not apply (fixture moved?)")
            target.write_text(after, encoding="utf-8")
            try:
                report = " ".join(check(dst))
            except SystemExit as exc:
                report = str(exc)
            if expect not in report:
                fail(f"self-test: '{label}' not reported (got: {report or 'clean'})")
        print(f"self-test: '{label}' reported")
    print(f"self-test ok: baseline clean, all {len(drifts)} injected drifts reported")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="par-mux command schema generator")
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
        help="repository root (default: the checkout holding this script)",
    )
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--check", action="store_true", help="fail when the committed schema is stale"
    )
    mode.add_argument(
        "--self-test", action="store_true", help="assert injected drift fails the check"
    )
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test(args.root)
    if args.check:
        problems = check(args.root)
        if problems:
            print("mux schema gate failed:", *problems, sep="\n  ", file=sys.stderr)
            return 1
        count = len(build_schema(args.root)["commands"])
        print(f"mux schema ok: {count} commands")
        return 0
    schema = build_schema(args.root)
    out = args.root / OUTPUT
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(render(schema), encoding="utf-8")
    print(f"wrote {len(schema['commands'])} commands to {OUTPUT}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
