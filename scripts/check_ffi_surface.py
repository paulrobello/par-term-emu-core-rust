#!/usr/bin/env python3
"""ENH-027/ENH-034 — FFI surface documentation gate.

Five checks against the C embedding surface, each failing closed (a regex
that matched nothing is a broken gate, not a clean pass):

  1. Every `#[no_mangle] extern "C" fn` exported from src/ffi.rs appears as
     a prototype in include/terminal_core.h (cbindgen-generated — `make
     ffi-header-check` covers regeneration drift, this covers the fn being
     absent entirely, e.g. emitted under a different name) and is documented
     in docs/FFI_GUIDE.md (backticked function name).
  2. Every `typedef struct` name in the header — the `} Name;` records plus
     the opaque `typedef struct X X;` handles — is backticked in the guide.
  3. Every `#define TERM_*` constant in include/terminal_core_layout.h is
     backticked in the guide or covered by a backticked `PREFIX_*` family
     wildcard whose prefix has at least two underscores, so `TERM_CELL_*`
     covers but the bare `TERM_*` on the guide's intro line does not.
  4. The guide's `## ABI Version` section carries a table row for every
     version 1..=N, where N is `TERM_CORE_ABI_VERSION`, and the Rust and
     header values of N agree (so a table that stopped at v2 while the
     shipped ABI is v3 — or a Rust/header version split — fails).
  5. No doc still calls include/terminal_core.h "hand-written" (it is
     cbindgen-generated, `make ffi-header`); only terminal_core_layout.h is
     hand-written. Scans docs/*.md non-recursively (which skips the
     docs/opus/, docs/fable/ and docs/research/ planning directories) plus
     README.md, CONTRIBUTING.md and QUICKSTART.md.

Fails with exit 1 listing every miss. The DELIBERATELY_UNDOCUMENTED /
DELIBERATELY_UNDOCUMENTED_TYPES sets below are the escape hatches; they
should stay near-empty, each entry with a reason.

Wired as `ffi-surface-check` in the Makefile, which also runs --self-test:
the self-test copies the inputs to a temp dir, asserts the unmodified copy
passes, then injects one drift per check and asserts each is reported naming
the injected item.
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

FFI_RS = "src/ffi.rs"
HEADER = "include/terminal_core.h"
LAYOUT_HEADER = "include/terminal_core_layout.h"
GUIDE = "docs/FFI_GUIDE.md"
# Non-recursive docs/*.md plus these top-level files are scanned for the
# stale "hand-written header" claim (check 5).
TOPLEVEL_SCANNED = ("README.md", "CONTRIBUTING.md", "QUICKSTART.md")

# Exported internals that are intentionally absent from the guide.
DELIBERATELY_UNDOCUMENTED: set[str] = set()

# TerminalEventKind is an opaque cbindgen forward declaration; no exported
# function takes or returns it, so there is no C usage to document.
DELIBERATELY_UNDOCUMENTED_TYPES: set[str] = {"TerminalEventKind"}

# Files the gate reads (also the set --self-test copies and drifts).
SELF_TEST_FILES: tuple[str, ...] = (
    FFI_RS,
    HEADER,
    LAYOUT_HEADER,
    GUIDE,
    "docs/RUST_USAGE.md",
) + TOPLEVEL_SCANNED

# #[no_mangle] \n pub (unsafe) extern "C" fn NAME
FN_RE = re.compile(
    r'#\[no_mangle\]\s*pub\s+(?:unsafe\s+)?extern\s+"C"\s+fn\s+(\w+)', re.DOTALL
)
# } Name;  (struct typedef close) and the opaque forward declarations.
STRUCT_CLOSE_RE = re.compile(r"^\} (\w+);", re.MULTILINE)
OPAQUE_TYPEDEF_RE = re.compile(r"^typedef struct (\w+) \1;", re.MULTILINE)
# The include guard contains TERM_, so anchor at ^#define TERM_.
CONST_RE = re.compile(r"^#define (TERM_[A-Z0-9_]+)", re.MULTILINE)
# `TERM_SOMETHING_*` family wildcards in the guide (at least one underscore
# inside the prefix, so the bare `TERM_*` never matches).
WILDCARD_RE = re.compile(r"`(TERM_[A-Z0-9_]*_)\*`")
ABI_RS_RE = re.compile(r"pub const TERM_CORE_ABI_VERSION: u32 = (\d+);")
ABI_HEADER_RE = re.compile(r"^#define TERM_CORE_ABI_VERSION (\d+)", re.MULTILINE)
ABI_SECTION_RE = re.compile(
    r"^## ABI Version\s*$(.*?)(?=^## |\Z)", re.MULTILINE | re.DOTALL
)
# "hand-written header" (the DOC-104 phrasing) and any "hand-written …
# terminal_core.h" wording — terminal_core\.h\b cannot match the
# hand-written terminal_core_layout.h.
STALE_PATTERNS: tuple[re.Pattern[str], ...] = (
    re.compile(r"hand-written header", re.IGNORECASE),
    re.compile(r"hand-written[^.\n]{0,40}terminal_core\.h\b", re.IGNORECASE),
)


def fail(msg: str) -> NoReturn:
    raise SystemExit(f"error: {msg}")


def read(root: Path, rel: str) -> str:
    try:
        return (root / rel).read_text()
    except OSError as exc:
        fail(f"parsed nothing from {rel}: cannot read ({exc})")


def exported_functions(ffi_rs: str) -> list[str]:
    names = FN_RE.findall(ffi_rs)
    if not names:
        fail(f'parsed nothing from {FFI_RS}: no extern "C" fns')
    return names


def header_typedefs(header: str) -> list[str]:
    closed = set(STRUCT_CLOSE_RE.findall(header))
    opaque = set(OPAQUE_TYPEDEF_RE.findall(header))
    names = sorted(closed | opaque)
    if not names:
        fail(f"parsed nothing from {HEADER}: no `typedef struct` names")
    return names


def layout_constants(layout: str) -> list[str]:
    names = CONST_RE.findall(layout)
    if not names:
        fail(f"parsed nothing from {LAYOUT_HEADER}: no `#define TERM_*` lines")
    return names


def abi_version(ffi_rs: str) -> int:
    m = ABI_RS_RE.search(ffi_rs)
    if not m:
        fail(
            f"parsed nothing from {FFI_RS}: "
            "`pub const TERM_CORE_ABI_VERSION: u32 = N;` not found"
        )
    return int(m.group(1))


def guide_abi_section(guide: str) -> str:
    m = ABI_SECTION_RE.search(guide)
    if not m:
        fail(f"parsed nothing from {GUIDE}: no `## ABI Version` section")
    return m.group(1)


def stale_phrase_files(root: Path) -> list[Path]:
    docs = root / "docs"
    if not docs.is_dir():
        fail(f"parsed nothing: {docs} does not exist")
    paths = sorted(docs.glob("*.md")) + [root / name for name in TOPLEVEL_SCANNED]
    if not paths:
        fail("parsed nothing: no docs/*.md files to scan for stale phrases")
    return paths


def collect_problems(root: Path) -> tuple[list[str], dict[str, int]]:
    ffi_rs = read(root, FFI_RS)
    header = read(root, HEADER)
    layout = read(root, LAYOUT_HEADER)
    guide = read(root, GUIDE)

    problems: list[str] = []

    # Check 1: exported fns ↔ header prototypes ↔ guide mentions.
    fns = exported_functions(ffi_rs)
    missing_header = [name for name in fns if not re.search(rf"\b{name}\s*\(", header)]
    for name in missing_header:
        problems.append(
            f"FFI function `{name}` missing from {HEADER} (run: make ffi-header)"
        )
    missing_guide = [
        name
        for name in fns
        if name not in DELIBERATELY_UNDOCUMENTED and f"`{name}`" not in guide
    ]
    for name in missing_guide:
        problems.append(
            f"FFI function `{name}` missing from {GUIDE} "
            "(document it, or add to DELIBERATELY_UNDOCUMENTED with a reason)"
        )

    # Check 2: header typedef names ↔ guide mentions.
    typedefs = header_typedefs(header)
    undocumented_types = [
        name
        for name in typedefs
        if name not in DELIBERATELY_UNDOCUMENTED_TYPES and f"`{name}`" not in guide
    ]
    for name in undocumented_types:
        problems.append(
            f"type `{name}` typedef'd in {HEADER} but not backticked in {GUIDE} "
            "(document it, or add to DELIBERATELY_UNDOCUMENTED_TYPES with a reason)"
        )

    # Check 3: layout-header TERM_* constants ↔ guide literal or family
    # wildcard coverage.
    consts = layout_constants(layout)
    wildcards = [
        prefix for prefix in set(WILDCARD_RE.findall(guide)) if prefix.count("_") >= 2
    ]
    const_misses = [
        name
        for name in consts
        if f"`{name}`" not in guide
        and not any(name.startswith(prefix) for prefix in wildcards)
    ]
    for name in const_misses:
        second = name.find("_", len("TERM_"))
        family = (
            f", or add a `{name[: second + 1]}*` family line" if second >= 0 else ""
        )
        problems.append(
            f"constant `{name}` ({LAYOUT_HEADER}) not covered by {GUIDE} — "
            f"document it as `{name}`{family}"
        )

    # Check 4: ABI version agreement + a complete v1..=N row set in the
    # guide's ABI Version table.
    rust_abi = abi_version(ffi_rs)
    m = ABI_HEADER_RE.search(layout)
    if not m:
        fail(
            f"parsed nothing from {LAYOUT_HEADER}: "
            "`#define TERM_CORE_ABI_VERSION N` not found"
        )
    header_abi = int(m.group(1))
    if rust_abi != header_abi:
        problems.append(
            f"TERM_CORE_ABI_VERSION mismatch: {FFI_RS} has {rust_abi}, "
            f"{LAYOUT_HEADER} has {header_abi} — bump them together"
        )
    section = guide_abi_section(guide)
    for version in range(1, rust_abi + 1):
        if not re.search(rf"^\|\s*v?{version}\s*\|", section, re.MULTILINE):
            problems.append(
                f"{GUIDE} ABI Version table has no row for v{version} "
                f"(rows must cover v1..v{rust_abi} with no gaps)"
            )

    # Check 5: no stale "hand-written header" claim about the generated
    # header in the scanned docs.
    for path in stale_phrase_files(root):
        rel = path.relative_to(root)
        try:
            text = path.read_text()
        except OSError as exc:
            fail(f"parsed nothing from {rel}: cannot read ({exc})")
        for lineno, line in enumerate(text.splitlines(), 1):
            if any(pattern.search(line) for pattern in STALE_PATTERNS):
                problems.append(
                    f"{rel}:{lineno}: {line.strip()} — stale phrase: the header is "
                    "cbindgen-generated (`make ffi-header`); only "
                    "terminal_core_layout.h is hand-written"
                )

    counts = {
        "fns": len(fns),
        "typedefs": len(typedefs),
        "constants": len(consts),
        "abi": rust_abi,
    }
    return problems, counts


def copy_inputs(src_root: Path, dst_root: Path) -> None:
    # The stale-phrase check reads all docs/*.md non-recursively, so copy
    # the whole (flat) docs/ markdown set for a faithful baseline.
    for src in sorted((src_root / "docs").glob("*.md")) + [
        src_root / rel for rel in (FFI_RS, HEADER, LAYOUT_HEADER) + TOPLEVEL_SCANNED
    ]:
        dst = dst_root / src.relative_to(src_root)
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(src, dst)


def drop_abi_row(text: str, version: int) -> str:
    new, count = re.subn(
        rf"^\|\s*v{version}\s*\|[^\n]*\n", "", text, flags=re.MULTILINE
    )
    if count != 1:
        fail(f"self-test setup: could not drop the v{version} ABI table row")
    return new


def run_self_test(root: Path) -> int:
    # The baseline copy must pass — the self-test runs after the DOC-103/104
    # fixes, and FFI_GUIDE.md's "hand-written companion terminal_core_layout.h"
    # sentence must be tolerated by check 5.
    with tempfile.TemporaryDirectory() as td:
        dst_root = Path(td)
        copy_inputs(root, dst_root)
        guide = read(dst_root, GUIDE)
        if "hand-written companion `terminal_core_layout.h`" not in guide:
            fail(
                "self-test setup: FFI_GUIDE.md no longer carries the "
                "'hand-written companion `terminal_core_layout.h`' sentence "
                "the stale-phrase check must tolerate"
            )
        problems, counts = collect_problems(dst_root)
        if problems:
            print(
                "self-test: baseline copy failed:",
                *problems,
                sep="\n  ",
                file=sys.stderr,
            )
            return 1
    print(
        "self-test: baseline copy passes "
        f"({counts['fns']} fns, {counts['typedefs']} typedefs, "
        f"{counts['constants']} constants, ABI v{counts['abi']})"
    )

    # One injected drift per check; each must be reported naming its item.
    rust_abi = abi_version(read(root, FFI_RS))
    drifts: list[tuple[str, str, Callable[[str], str], str]] = [
        (
            "delete every `TermKeyOptions` mention from the guide",
            GUIDE,
            lambda t: t.replace("`TermKeyOptions`", "TermKeyOptions"),
            "TermKeyOptions",
        ),
        (
            "add `#define TERM_FAKE_THING 1` to the layout header",
            LAYOUT_HEADER,
            lambda t: t + "\n#define TERM_FAKE_THING 1\n",
            "TERM_FAKE_THING",
        ),
        (
            f"delete the v{rust_abi} row from the ABI Version table",
            GUIDE,
            lambda t: drop_abi_row(t, rust_abi),
            f"v{rust_abi}",
        ),
        (
            "append a stale 'hand-written header' line to RUST_USAGE.md",
            "docs/RUST_USAGE.md",
            lambda t: (
                t
                + "\nThe hand-written header `include/terminal_core.h` declares the surface.\n"
            ),
            "RUST_USAGE.md",
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
    print(
        f"self-test ok: baseline clean and all {len(drifts)} injected drifts reported"
    )
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="ENH-027/ENH-034 FFI doc gate")
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
        help="repository root to check (default: the checkout holding this script)",
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="copy the inputs to a temp dir, inject one drift per check, assert each is reported",
    )
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test(args.root)
    problems, counts = collect_problems(args.root)
    if problems:
        print("FFI doc gate failed:")
        for problem in problems:
            print(f"  {problem}")
        return 1
    print(
        f"ffi surface ok: {counts['fns']} exported fns present in header and guide, "
        f"{counts['typedefs']} header typedefs documented, "
        f"{counts['constants']} TERM_* constants covered, "
        f"ABI v{counts['abi']} table complete (v1..v{counts['abi']})"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
