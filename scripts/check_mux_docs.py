#!/usr/bin/env python3
"""ENH-033 — par-mux doc-contract gate.

Diffs four code-owned lists against their documentation, so the next mux
command or notification added without its doc row fails the gate:

  1. `const COMMANDS` (src/mux/command/)        ↔ MUX.md "Command Reference" rows
  2. `MuxCommand::mutates()` true arms           ↔ MUX.md "When it saves" list
  3. `"%…` strings in `emit()` (src/mux/emit.rs) ↔ MUX.md "Notifications" rows
  4. `notification_type()` strings (crates/par-term-emu-core/src/tmux_control.rs)
                                                 ↔ the `notification_type` bullet
                                                   in docs/API_REFERENCE.md

`%begin`/`%end`/`%error` are reply framing: the gate requires them (backticked)
in MUX.md's "Protocol Overview" section instead of the Notifications table.
`%unlinked-window-close` and `%pane-mode-changed` have emit arms kept for
parser round-trip tests but are never constructed by the daemon, so they live
in the guarded NEVER_SENT allowlist below instead of the table.

Every extraction fails closed: a regex that matched nothing is a broken gate,
not a clean pass (the check_ffi_surface.py rule). DOC-101 and DOC-102 item 3
are prose meaning a list diff cannot see; this gate does not claim them.
"""

from __future__ import annotations

import argparse
import re
import shutil
import sys
import tempfile
from collections.abc import Callable
from pathlib import Path
from typing import NoReturn

# Reply framing (%begin/%end/%error): gated against MUX.md "Protocol
# Overview" — each must appear backticked there, never as a table row.
FRAMING: set[str] = {"begin", "end", "error"}

# Emit arms kept for parser round-trip tests (only emit.rs's own tests
# construct them): never sent by the daemon, so not Notifications rows.
# Guarded both ways: a name losing its emit arm, or a variant that starts
# being constructed in src/mux/ production code, fails the gate — the latter
# means the daemon now sends it and the row belongs in the table.
NEVER_SENT: set[str] = {"unlinked-window-close", "pane-mode-changed"}

# Files the gate reads (also the set --self-test copies and drifts).
SELF_TEST_FILES: tuple[str, ...] = (
    "src/mux/command/",
    "src/mux/emit.rs",
    "crates/par-term-emu-core/src/tmux_control.rs",
    "docs/MUX.md",
    "docs/API_REFERENCE.md",
)

COMMANDS_ROW_RE = re.compile(r'\("([a-z-]+)",\s*(\w+)')
VARIANT_RE = re.compile(r"MuxCommand::(\w+)")
EMIT_STRING_RE = re.compile(r'"%([a-z-]+)')
DOC_CMD_ROW_RE = re.compile(r"^\| `([a-z-]+)` \|", re.MULTILINE)
DOC_NOTIF_ROW_RE = re.compile(r"^\| `%([a-z-]+)", re.MULTILINE)


def fail(msg: str) -> NoReturn:
    raise SystemExit(f"error: {msg}")


def read(root: Path, rel: str) -> str:
    """One file's text, or a module directory's production files concatenated.

    A directory (`rel` ends in `/`) reads `mod.rs` first, then every other
    `*.rs` except `tests.rs`, so items can move between the module's files
    (ARC-003) without the gate losing them.
    """
    try:
        if rel.endswith("/"):
            d = root / rel
            files = [d / "mod.rs"] + sorted(
                f for f in d.glob("*.rs") if f.name not in ("mod.rs", "tests.rs")
            )
            return "\n".join(f.read_text() for f in files)
        return (root / rel).read_text()
    except OSError as exc:
        fail(f"parsed nothing from {rel}: cannot read ({exc})")


def fn_block(text: str, marker: str, where: str) -> str:
    """Brace-matched body of the function whose signature contains marker."""
    start = text.find(marker)
    if start < 0:
        fail(f"parsed nothing from {where}: `{marker}` not found")
    open_brace = text.find("{", start)
    if open_brace < 0:
        fail(f"parsed nothing from {where}: no body after `{marker}`")
    depth = 0
    for i in range(open_brace, len(text)):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                return text[open_brace : i + 1]
    fail(f"parsed nothing from {where}: unbalanced braces after `{marker}`")


def md_section(doc: str, title: str, where: str) -> str:
    """Body of a `## <title>` section, up to the next `## ` heading."""
    m = re.search(
        rf"^## {re.escape(title)}\s*$(.*?)(?=^## |\Z)", doc, re.MULTILINE | re.DOTALL
    )
    if not m:
        fail(f"parsed nothing from {where}: no `## {title}` section")
    return m.group(1)


def strip_line_comments(code: str) -> str:
    return re.sub(r"//[^\n]*", "", code)


def skip_rust_string(code: str, i: int) -> int:
    """Index just past the string literal whose opening `"` is at `i`:
    ordinary and byte strings honor backslash escapes (`\\"` never ends the
    literal); a raw-hash prefix (`r#"…"#`, `br#"…"#`) ends only at the
    matching `"#` run. An unterminated literal consumes to end of text."""
    n = len(code)
    hashes = 0
    while i - 1 - hashes >= 0 and code[i - 1 - hashes] == "#":
        hashes += 1
    r = i - 1 - hashes
    if hashes > 0 and r >= 0 and code[r] == "r":
        before = code[r - 1] if r > 0 else ""
        if not (before.isalnum() or before == "_") or before in "bc":
            closer = '"' + "#" * hashes
            end = code.find(closer, i + 1)
            return n if end < 0 else end + len(closer)
    j = i + 1
    while j < n:
        c = code[j]
        if c == "\\":
            j += 2
        elif c == '"':
            return j + 1
        else:
            j += 1
    return n


def skip_block_comment(code: str, i: int) -> int:
    """Index past the `/* … */` starting at `i` (Rust block comments nest);
    an unterminated comment consumes to end of text."""
    depth = 0
    n = len(code)
    while i < n:
        if code.startswith("/*", i):
            depth += 1
            i += 2
        elif code.startswith("*/", i):
            depth -= 1
            i += 2
            if depth == 0:
                return i
        else:
            i += 1
    return n


def char_literal_end(code: str, i: int) -> int | None:
    """Index just past the closing `'` of the char literal starting at `i`,
    or None when the shape is not a literal (lifetime `'a`, `'static`) —
    the braced form of `\\u{…}` keeps its braces out of the count."""
    n = len(code)
    if i + 1 >= n:
        return None
    j = i + 1
    if code[j] == "\\":
        j += 2
        if code[j - 1] == "u" and j < n and code[j] == "{":
            close = code.find("}", j)
            if close < 0:
                return None
            j = close + 1
    else:
        j += 1
    if j < n and code[j] == "'":
        return j + 1
    return None


def strip_test_modules(code: str) -> str:
    """Remove `#[cfg(test)] mod <name> { … }` blocks (brace-matched; QA-229:
    braces inside string and char literals do not count, escapes honored)."""
    out = code
    while True:
        m = re.search(r"#\[cfg\(test\)\]\s*mod\s+\w+\s*\{", out)
        if not m:
            return out
        open_brace = out.find("{", m.start())
        depth = 0
        end = None
        i = open_brace
        n = len(out)
        while i < n:
            c = out[i]
            if c == "/" and out[i + 1 : i + 2] == "/":
                nl = out.find("\n", i)
                i = n if nl < 0 else nl + 1
                continue
            if c == "/" and out[i + 1 : i + 2] == "*":
                i = skip_block_comment(out, i)
                continue
            if c == '"':
                i = skip_rust_string(out, i)
                continue
            if c == "'":
                literal_end = char_literal_end(out, i)
                if literal_end is not None:
                    i = literal_end
                    continue
            if c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
                if depth == 0:
                    end = i + 1
                    break
            i += 1
        if end is None:
            fail("parsed nothing: unterminated #[cfg(test)] module")
        out = out[: m.start()] + out[end:]


def parse_commands(command_rs: str) -> list[tuple[str, str]]:
    start = command_rs.find("const COMMANDS")
    if start < 0:
        fail(
            "parsed nothing from src/mux/command/: `const COMMANDS` not found (identifier renamed?)"
        )
    end = command_rs.find("];", start)
    if end < 0:
        fail(
            "parsed nothing from src/mux/command/: unterminated `const COMMANDS` table"
        )
    rows = COMMANDS_ROW_RE.findall(command_rs[start:end])
    if not rows:
        fail(
            "parsed nothing from src/mux/command/: no (name, parser, …) rows inside `const COMMANDS`"
        )
    return rows


def parse_mutates_true_arms(command_rs: str) -> set[str]:
    body = fn_block(command_rs, "pub fn mutates(", "src/mux/command/")
    idx = body.find("=> true")
    if idx < 0:
        fail("parsed nothing from src/mux/command/: no `=> true` arm in mutates()")
    variants = set(VARIANT_RE.findall(body[:idx]))
    if not variants:
        fail(
            "parsed nothing from src/mux/command/: no MuxCommand variants before `=> true` in mutates()"
        )
    if not re.search(
        r"MuxCommand::RefreshClient\s*\{[^}]*\}\s*=>\s*size\.is_some\(\)", body
    ):
        fail(
            "mutates() no longer carries the conditional "
            "`MuxCommand::RefreshClient { size, .. } => size.is_some()` arm — "
            "update this gate and the MUX.md save list together"
        )
    return variants


def variant_map(command_rs: str, commands: list[tuple[str, str]]) -> dict[str, str]:
    """command name → the one MuxCommand variant its parser constructs."""
    result: dict[str, str] = {}
    for name, parser in commands:
        body = strip_line_comments(
            fn_block(command_rs, f"fn {parser}(", "src/mux/command/")
        )
        variants = set(VARIANT_RE.findall(body))
        if len(variants) != 1:
            fail(
                f"parser `{parser}` constructs {sorted(variants) or 'no'} MuxCommand "
                "variant(s), expected exactly one — the variant→command map is broken"
            )
        result[name] = variants.pop()
    return result


def save_list(mux_md: str) -> list[str]:
    m = re.search(r"^- \*\*When it saves:\*\*(.*)$", mux_md, re.MULTILINE)
    if not m:
        fail("parsed nothing from docs/MUX.md: no `**When it saves:**` bullet")
    names = [
        t
        for t in re.findall(r"`([^`]+)`", m.group(1))
        if re.fullmatch(r"[a-z][a-z0-9-]*( -C)?", t)
    ]
    if not names:
        fail("parsed nothing from docs/MUX.md: empty `When it saves` list")
    return names


def notification_type_map(tmux_rs: str) -> dict[str, str]:
    """notification_type string → TmuxNotification variant."""
    body = fn_block(
        tmux_rs,
        "pub fn notification_type(",
        "crates/par-term-emu-core/src/tmux_control.rs",
    )
    arms = re.findall(r'Self::(\w+)(?:\s*\{[^}]*\})?\s*=>\s*"([a-z-]+)"', body)
    if not arms:
        fail(
            "parsed nothing from crates/par-term-emu-core/src/tmux_control.rs: no arms in notification_type()"
        )
    return {name: variant for variant, name in arms}


def doc_notification_types(api_md: str) -> list[str]:
    m = re.search(r"^- `notification_type: str`:.*$", api_md, re.MULTILINE)
    if not m:
        fail(
            "parsed nothing from docs/API_REFERENCE.md: no `- `notification_type: str`` bullet"
        )
    tokens = re.findall(r"`([a-z-]+)`", m.group(0))
    if not tokens:
        fail("parsed nothing from docs/API_REFERENCE.md: empty notification_type list")
    return tokens


def never_sent_production_hits(root: Path, type_map: dict[str, str]) -> list[str]:
    """Files under src/mux/ (production code) constructing a NEVER_SENT variant."""
    hits: list[str] = []
    sources = sorted(
        p
        for p in (root / "src" / "mux").rglob("*.rs")
        if p.name not in ("emit.rs", "tests.rs")
    )
    if not sources:
        fail("parsed nothing from src/mux/: no production .rs files to scan")
    for name in sorted(NEVER_SENT):
        variant = type_map.get(name)
        if variant is None:
            fail(
                f"parsed nothing from crates/par-term-emu-core/src/tmux_control.rs: notification_type() has no "
                f"`{name}` arm — NEVER_SENT no longer maps to a variant"
            )
        needle = f"TmuxNotification::{variant}"
        for path in sources:
            if needle in strip_test_modules(path.read_text()):
                hits.append(f"{path.relative_to(root)} ({needle})")
    return hits


def collect_problems(root: Path) -> tuple[list[str], dict[str, int]]:
    command_rs = read(root, "src/mux/command/")
    emit_rs = read(root, "src/mux/emit.rs")
    tmux_rs = read(root, "crates/par-term-emu-core/src/tmux_control.rs")
    mux_md = read(root, "docs/MUX.md")
    api_md = read(root, "docs/API_REFERENCE.md")

    problems: list[str] = []

    # Check 1: COMMANDS table ↔ Command Reference rows, both directions.
    commands = parse_commands(command_rs)
    command_names = [name for name, _ in commands]
    doc_commands = DOC_CMD_ROW_RE.findall(
        md_section(mux_md, "Command Reference", "docs/MUX.md")
    )
    if not doc_commands:
        fail("parsed nothing from docs/MUX.md: no rows in the Command Reference table")
    for name in command_names:
        if name not in doc_commands:
            problems.append(
                f"command `{name}` missing from MUX.md Command Reference table"
            )
    for name in doc_commands:
        if name not in command_names:
            problems.append(
                f"MUX.md Command Reference row `{name}` has no `const COMMANDS` entry in src/mux/command/"
            )

    # Check 2: mutates() true arms ↔ the "When it saves" list.
    mut_variants = parse_mutates_true_arms(command_rs)
    variant_of = variant_map(command_rs, commands)
    saves = save_list(mux_md)
    mutating = {name for name in command_names if variant_of[name] in mut_variants}
    for name in sorted(mutating - set(saves)):
        problems.append(
            f"mutating command `{name}` missing from MUX.md 'When it saves' list"
        )
    if "refresh-client -C" not in saves:
        problems.append(
            "MUX.md 'When it saves' list must keep the literal `refresh-client -C` "
            "(the conditional RefreshClient arm)"
        )

    # Check 3: emit() strings ↔ Notifications table rows, both directions,
    # with the framing and never-sent allowlists gated separately.
    emit_names = EMIT_STRING_RE.findall(
        fn_block(emit_rs, "pub fn emit(", "src/mux/emit.rs")
    )
    if not emit_names:
        fail('parsed nothing from src/mux/emit.rs: no "%… strings inside pub fn emit(')
    table_rows = DOC_NOTIF_ROW_RE.findall(
        md_section(mux_md, "Notifications", "docs/MUX.md")
    )
    if not table_rows:
        fail("parsed nothing from docs/MUX.md: no rows in the Notifications table")
    tabled = set(emit_names) - FRAMING - NEVER_SENT
    for name in sorted(tabled - set(table_rows)):
        problems.append(
            f"notification `%{name}` (emit) missing from MUX.md Notifications table"
        )
    for name in sorted(set(table_rows) - tabled):
        problems.append(
            f"MUX.md Notifications row `%{name}` has no emit() arm in src/mux/emit.rs"
        )
    protocol = md_section(mux_md, "Protocol Overview", "docs/MUX.md")
    for name in sorted(FRAMING):
        if f"`%{name}`" not in protocol:
            problems.append(
                f"framing notification `%{name}` must be documented (backticked) in MUX.md Protocol Overview"
            )
    stale = sorted(NEVER_SENT - set(emit_names))
    if stale:
        problems.append(
            "NEVER_SENT allowlist is stale: no emit() arm for "
            + ", ".join(f"`%{n}`" for n in stale)
        )
    for hit in never_sent_production_hits(root, notification_type_map(tmux_rs)):
        problems.append(
            "NEVER_SENT variant is constructed in production code: "
            f"{hit} — the daemon now sends it; give it a Notifications table row "
            "and remove it from NEVER_SENT"
        )

    # Check 4: notification_type() strings ↔ the API_REFERENCE bullet.
    type_names = set(
        re.findall(
            r'=>\s*"([a-z-]+)"',
            fn_block(
                tmux_rs,
                "pub fn notification_type(",
                "crates/par-term-emu-core/src/tmux_control.rs",
            ),
        )
    )
    if not type_names:
        fail(
            "parsed nothing from crates/par-term-emu-core/src/tmux_control.rs: no strings in notification_type()"
        )
    doc_types = set(doc_notification_types(api_md))
    for name in sorted(type_names - doc_types):
        problems.append(
            f"notification type `{name}` missing from the API_REFERENCE notification_type list"
        )
    for name in sorted(doc_types - type_names):
        problems.append(
            f"API_REFERENCE notification_type token `{name}` has no notification_type() arm in crates/par-term-emu-core/src/tmux_control.rs"
        )

    counts = {
        "commands": len(command_names),
        "mutating": len(mutating),
        "notifications": len(tabled),
        "types": len(type_names),
    }
    return problems, counts


def copy_inputs(src_root: Path, dst_root: Path) -> None:
    for rel in SELF_TEST_FILES:
        dst = dst_root / rel
        if rel.endswith("/"):
            shutil.copytree(src_root / rel, dst)
            continue
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(src_root / rel, dst)


def run_self_test(root: Path) -> int:
    # The baseline copy must pass — the self-test runs after the doc fixes.
    with tempfile.TemporaryDirectory() as td:
        dst_root = Path(td)
        copy_inputs(root, dst_root)
        problems, _ = collect_problems(dst_root)
        if problems:
            print(
                "self-test: baseline copy failed:",
                *problems,
                sep="\n  ",
                file=sys.stderr,
            )
            return 1
    print("self-test: baseline copy passes")

    # One injected drift per check; each must be reported naming its item.
    # (label, file to mutate, mutation, expected name in the report)
    drifts: list[tuple[str, str, Callable[[str], str], str]] = [
        (
            "delete the `version` Command Reference row",
            "docs/MUX.md",
            lambda t: re.sub(r"^\| `version` \|[^\n]*\n", "", t, flags=re.MULTILINE),
            "version",
        ),
        (
            "remove `set-buffer` from the save list",
            "docs/MUX.md",
            lambda t: t.replace("`set-buffer`, ", "", 1),
            "set-buffer",
        ),
        (
            "delete the `%window-add` Notifications row",
            "docs/MUX.md",
            lambda t: re.sub(r"^\| `%window-add [^\n]*\n", "", t, flags=re.MULTILINE),
            "window-add",
        ),
        (
            "remove `exit` from the notification_type list",
            "docs/API_REFERENCE.md",
            lambda t: re.sub(
                r"(- `notification_type: str`:[^\n]*?)`exit`, ", r"\1", t, count=1
            ),
            "exit",
        ),
        (
            'add a fake ("fake-cmd", parse_version, &[]) entry to COMMANDS',
            "src/mux/command/mod.rs",
            lambda t: t.replace(
                '("version", parse_version, &[]),',
                '("version", parse_version, &[]),\n    ("fake-cmd", parse_version, &[]),',
                1,
            ),
            "fake-cmd",
        ),
    ]
    for label, rel, mutate, needle in drifts:
        with tempfile.TemporaryDirectory() as td:
            dst_root = Path(td)
            copy_inputs(root, dst_root)
            target = dst_root / rel
            target.write_text(mutate(target.read_text()))
            problems, _ = collect_problems(dst_root)
        if not problems:
            fail(f"self-test: {label} passed — the gate missed the injected drift")
        if not any(needle in problem for problem in problems):
            fail(f"self-test: {label} was not reported naming `{needle}`: {problems}")
        print(f"self-test: drift reported as required — {label}")

    # QA-229: a cfg(test) module whose string/char literals carry unbalanced
    # braces must be stripped whole — the test-only NEVER_SENT construction
    # inside must not read as production code, and a real production
    # construction must still be reported with the fixture present.
    qa229_mod = (
        "#[cfg(test)]\n"
        "mod qa229_fixture {\n"
        "    #[test]\n"
        "    fn braces_stay_inside_literals() {\n"
        '        let s = "{not json";\n'
        '        let t = "escaped \\" quote }} still json";\n'
        "        let c = '}';\n"
        "        let _ = (s, t, c);\n"
        "        let _ = TmuxNotification::PaneModeChanged;\n"
        "    }\n"
        "}\n"
    )

    def command_rs_with(append: str) -> list[str]:
        with tempfile.TemporaryDirectory() as td:
            dst_root = Path(td)
            copy_inputs(root, dst_root)
            target = dst_root / "src/mux/command/mod.rs"
            target.write_text(target.read_text() + append)
            problems, _ = collect_problems(dst_root)
        return problems

    problems = command_rs_with("\n" + qa229_mod)
    if problems:
        fail(
            "self-test: QA-229 brace-bearing cfg(test) fixture was not ignored: "
            + "; ".join(problems)
        )
    print("self-test: QA-229 brace-bearing cfg(test) fixture ignored")

    problems = command_rs_with(
        "\n"
        + qa229_mod
        + "fn qa229_production_hit() {\n    let _ = TmuxNotification::PaneModeChanged;\n}\n"
    )
    if not any("TmuxNotification::PaneModeChanged" in problem for problem in problems):
        fail(
            "self-test: QA-229 production NEVER_SENT construction was not reported: "
            + "; ".join(problems)
        )
    print("self-test: QA-229 production construction after the fixture still reported")

    print(
        "self-test ok: baseline clean, all 5 injected drifts reported, "
        "QA-229 brace-in-string fixture pinned"
    )
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="ENH-033 mux doc-contract gate")
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
        help="repository root to check (default: the checkout holding this script)",
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="copy the inputs to a temp dir, inject five drifts, assert each is reported",
    )
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test(args.root)
    problems, counts = collect_problems(args.root)
    if problems:
        print("MUX doc-contract gate failed:")
        for problem in problems:
            print(f"  {problem}")
        return 1
    print(
        f"mux docs ok: {counts['commands']} commands, {counts['mutating']} mutating "
        f"(plus conditional `refresh-client -C`), {counts['notifications']} notification rows, "
        f"{counts['types']} notification types"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
