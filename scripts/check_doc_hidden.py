#!/usr/bin/env python3
"""ARC-005: the `par-term-emu-core` member's `#[doc(hidden)] pub` set is
recorded in `crates/par-term-emu-core/DESIGN.md` and may not drift.

`#[doc(hidden)]` is documentation, not access control: every hidden `pub`
item is reachable by any external embedder of the published crate, so the
set is a public-API commitment that only prose used to freeze. This gate
extracts the current set from `crates/par-term-emu-core/src/**/*.rs` and
fails when it differs from the list between the `doc-hidden-baseline`
markers in DESIGN.md. An addition fails, so does a stale entry.

Policy: promote methods, never fields. A promoted field lets an embedder
write core state directly and bypass every invariant; a promoted method
keeps the invariant inside the core.

Item identity is `<top-level module>::<owner>::<name>` (for example
`terminal::Terminal::grid`), not a file path or line, so moving an item
between files of one module (`terminal/mod.rs` -> `terminal/state.rs`)
leaves the baseline valid. Every `.rs` file is scanned regardless of `cfg`.

`--self-test` builds a synthetic tree, injects one drift per contract, and
also requires the real tree to pass.
"""

from __future__ import annotations

import argparse
import re
import tempfile
from pathlib import Path
from typing import NoReturn

ROOT = Path(__file__).resolve().parent.parent
SRC_REL = "crates/par-term-emu-core/src"
DESIGN_REL = "crates/par-term-emu-core/DESIGN.md"
BEGIN = "<!-- doc-hidden-baseline:begin -->"
END = "<!-- doc-hidden-baseline:end -->"

# `#[doc(hidden)]` alone on its line, or followed by its item on the same line.
HIDDEN_RE = re.compile(r"^(?P<indent>\s*)#\[doc\(hidden\)\]\s*(?P<rest>.*)$")
# Lines whose item is deliberately not a leak: restricted visibility or no `pub`.
NOT_EXPORTED_RE = re.compile(r"^\s*(pub\(|(?!pub\b))")
# Lines that may sit between `#[doc(hidden)]` and the item it marks.
ATTR_OR_COMMENT_RE = re.compile(r"^\s*(#\[|///|//)")
# `pub` exactly: `pub(crate)` / `pub(super)` items are not a leak.
ITEM_RE = re.compile(
    r"^(?P<indent>\s*)pub\s+(?:(?:const|async|unsafe)\s+)*"
    r"(?:(?P<kind>fn|struct|enum|mod|trait|type|static|const)\s+(?P<name>\w+)"
    r"|use\s+(?P<use>[\w:]+)\s*;"
    r"|(?P<field>\w+)\s*:)"
)
OWNER_RE = re.compile(
    r"^(?P<indent>\s*)(?:pub(?:\([^)]*\))?\s+)?"
    r"(?:impl(?:<[^>]*>)?\s+(?:[\w:<>, ]+\s+for\s+)?(?P<impl>\w+)"
    r"|struct\s+(?P<struct>\w+)|enum\s+(?P<enum>\w+)|trait\s+(?P<trait>\w+))"
)


def fail(msg: str) -> NoReturn:
    print(msg)
    raise SystemExit(1)


def top_module(rel: Path) -> str:
    """`terminal/state.rs` -> `terminal`; `cell.rs` -> `cell`; `lib.rs` -> `crate`."""
    first = rel.parts[0]
    if len(rel.parts) == 1:
        stem = Path(first).stem
        return "crate" if stem == "lib" else stem
    return first


def owner_of(lines: list[str], idx: int, indent: int) -> str | None:
    """Name of the nearest enclosing impl/struct/enum/trait of line `idx`."""
    for j in range(idx - 1, -1, -1):
        m = OWNER_RE.match(lines[j])
        if m and len(m.group("indent")) < indent:
            return (
                m.group("impl")
                or m.group("struct")
                or m.group("enum")
                or m.group("trait")
            )
    return None


def extract(root: Path) -> set[str]:
    """The `#[doc(hidden)] pub` identities under `root/SRC_REL`."""
    src = root / SRC_REL
    found: set[str] = set()
    files = sorted(src.rglob("*.rs"))
    if not files:
        fail(f"parsed nothing: no .rs files under {SRC_REL}")
    for path in files:
        rel = path.relative_to(src)
        lines = path.read_text(encoding="utf-8").splitlines()
        module = top_module(rel)
        for i, line in enumerate(lines):
            hidden = HIDDEN_RE.match(line)
            if not hidden:
                continue
            if hidden.group("rest"):
                # Inline form: `#[doc(hidden)] pub fn x()` on one line.
                j = i
                item_line = hidden.group("indent") + hidden.group("rest")
            else:
                j = i + 1
                while j < len(lines) and ATTR_OR_COMMENT_RE.match(lines[j]):
                    j += 1
                if j >= len(lines):
                    fail(
                        f"cannot classify {SRC_REL}/{rel}:{i + 1}: #[doc(hidden)] at end of file"
                    )
                item_line = lines[j]
            m = ITEM_RE.match(item_line)
            if not m:
                if NOT_EXPORTED_RE.match(item_line):
                    continue  # `pub(crate)` or non-`pub`: hidden but not exported
                fail(
                    f"cannot classify {SRC_REL}/{rel}:{j + 1}: `{item_line.strip()}` "
                    "follows #[doc(hidden)] but matches no known pub item form"
                )
            if m.group("use"):
                name = m.group("use").rsplit("::", 1)[-1]
                found.add(f"{module}::{name} (re-export)")
                continue
            indent = len(m.group("indent"))
            if m.group("field"):
                owner = owner_of(lines, j, indent)
                found.add(f"{module}::{owner}::{m.group('field')} (field)")
                continue
            name = m.group("name")
            kind = m.group("kind")
            if indent:
                owner = owner_of(lines, j, indent)
                found.add(f"{module}::{owner}::{name} ({kind})")
            else:
                found.add(f"{module}::{name} ({kind})")
    return found


def baseline(root: Path) -> set[str]:
    """The recorded set: one backticked identity per bullet between the markers."""
    text = (root / DESIGN_REL).read_text(encoding="utf-8")
    try:
        block = text.split(BEGIN, 1)[1].split(END, 1)[0]
    except IndexError:
        fail(f"{DESIGN_REL}: missing {BEGIN} / {END} markers")
    items = set(re.findall(r"^\s*-\s+`([^`]+)`", block, re.MULTILINE))
    if not items:
        fail(f"parsed nothing: no entries between the baseline markers in {DESIGN_REL}")
    return items


def check(root: Path) -> None:
    current = extract(root)
    if not current:
        fail(f"parsed nothing: no #[doc(hidden)] pub items under {SRC_REL}")
    recorded = baseline(root)
    added = sorted(current - recorded)
    stale = sorted(recorded - current)
    if added or stale:
        for item in added:
            hint = (
                " (policy: promote methods, never fields)"
                if item.endswith("(field)")
                else ""
            )
            print(f"new #[doc(hidden)] pub item not in {DESIGN_REL}: `{item}`{hint}")
        for item in stale:
            print(f"{DESIGN_REL} lists `{item}`, which is no longer #[doc(hidden)] pub")
        print(
            "Update the doc-hidden-baseline block and the Promoted items table "
            "together, or keep the item pub(crate)."
        )
        raise SystemExit(1)
    print(f"doc-hidden ok: {len(current)} promoted items match {DESIGN_REL}")


def build_synthetic_tree(root: Path) -> None:
    src = root / SRC_REL
    (src / "terminal").mkdir(parents=True, exist_ok=True)
    (src / "lib.rs").write_text("pub mod terminal;\n", encoding="utf-8")
    (src / "terminal" / "mod.rs").write_text(
        "pub struct Terminal {\n"
        "    /// grid\n"
        "    #[doc(hidden)]\n"
        "    pub grid: u8,\n"
        "    #[doc(hidden)]\n"
        "    pub(crate) private: u8,\n"
        "}\n"
        "impl Terminal {\n"
        "    #[doc(hidden)]\n"
        '    #[cfg(feature = "mux")]\n'
        "    pub fn resize_deferred(&mut self) {}\n"
        "}\n",
        encoding="utf-8",
    )
    (root / "crates/par-term-emu-core/DESIGN.md").write_text(
        f"# x\n\n{BEGIN}\n"
        "- `terminal::Terminal::grid (field)`\n"
        "- `terminal::Terminal::resize_deferred (fn)`\n"
        f"{END}\n",
        encoding="utf-8",
    )


def _expect_failure(root: Path, needle: str) -> None:
    import contextlib
    import io

    buf = io.StringIO()
    try:
        with contextlib.redirect_stdout(buf):
            check(root)
    except SystemExit:
        if needle not in buf.getvalue():
            fail(f"self-test: failure output lacks {needle!r}: {buf.getvalue()}")
        return
    fail(f"self-test: expected a failure mentioning {needle!r}, got a pass")


def self_test() -> None:
    with tempfile.TemporaryDirectory(prefix="arc005-selftest-") as tmp:
        root = Path(tmp)
        build_synthetic_tree(root)
        check(root)  # Baseline passes; pub(crate) hidden items are ignored.

        # Moving an item to another file of the same module keeps it valid.
        mod_rs = root / SRC_REL / "terminal" / "mod.rs"
        text = mod_rs.read_text(encoding="utf-8")
        impl_block = text[text.index("impl Terminal {") :]
        mod_rs.write_text(text.replace(impl_block, ""), encoding="utf-8")
        (root / SRC_REL / "terminal" / "state.rs").write_text(
            impl_block, encoding="utf-8"
        )
        check(root)

        # A newly promoted field fails and cites the policy.
        build_synthetic_tree(root)
        mod_rs.write_text(
            mod_rs.read_text(encoding="utf-8").replace(
                "    pub(crate) private: u8,", "    pub leaked: u8,"
            ),
            encoding="utf-8",
        )
        _expect_failure(root, "promote methods, never fields")

        # A promotion reverted to pub(crate) leaves a stale baseline entry.
        build_synthetic_tree(root)
        (root / SRC_REL / "terminal" / "state.rs").unlink()
        mod_rs.write_text(
            mod_rs.read_text(encoding="utf-8").replace(
                "pub fn resize_deferred", "pub(crate) fn resize_deferred"
            ),
            encoding="utf-8",
        )
        _expect_failure(root, "no longer #[doc(hidden)] pub")

        # The inline form `#[doc(hidden)] pub fn x()` is extracted, not skipped.
        build_synthetic_tree(root)
        with mod_rs.open("a", encoding="utf-8") as fh:
            fh.write("#[doc(hidden)] pub fn inline_leak() {}\n")
        _expect_failure(root, "terminal::inline_leak (fn)")

        # An unparseable pub item after #[doc(hidden)] fails closed.
        build_synthetic_tree(root)
        with mod_rs.open("a", encoding="utf-8") as fh:
            fh.write("#[doc(hidden)]\npub macro_rules_like! odd\n")
        _expect_failure(root, "cannot classify")

        # Missing markers fail closed.
        build_synthetic_tree(root)
        (root / DESIGN_REL).write_text("# x\n", encoding="utf-8")
        _expect_failure(root, "missing")

    # Fail closed on an empty source tree.
    with tempfile.TemporaryDirectory(prefix="arc005-selftest2-") as tmp:
        root = Path(tmp)
        (root / SRC_REL).mkdir(parents=True)
        _expect_failure(root, "parsed nothing")

    # The real tree must pass and must not regress to a trivially small scan.
    real = extract(ROOT)
    if len(real) < 20:
        fail(
            f"self-test: the real tree yields only {len(real)} items; the extractor regressed"
        )
    check(ROOT)
    print(
        "self-test ok: drift, stale entries, field policy, and empty trees reported; file moves tolerated"
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root", type=Path, default=ROOT, help="repository root to check"
    )
    parser.add_argument(
        "--print", action="store_true", help="print the current extracted set"
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="build a synthetic tree, inject one drift per contract, and check the real tree",
    )
    args = parser.parse_args()
    if args.print:
        for item in sorted(extract(args.root)):
            print(f"- `{item}`")
    elif args.self_test:
        self_test()
    else:
        check(args.root)


if __name__ == "__main__":
    main()
