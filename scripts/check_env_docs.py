#!/usr/bin/env python3
"""ENH-055: every env var production code reads is in docs/CONFIG_REFERENCE.md's
table (or MUX.md's pane env contract, or STREAMING.md for the clap `env =` set);
every row names a variable something reads.

Scans the Rust crates (literal and const-resolved `env::var`/`env::var_os`
reads), the shipped agent-integration extensions/hooks (TS/sh/ps1), the Python
package, and the streamer's clap `env =` attributes. Fails closed on anything
it cannot resolve. `--self-test` copies the inputs into a temp tree, injects
one drift per contract, and requires each to be reported (and the baseline to
pass).
"""

from __future__ import annotations

import argparse
import contextlib
import io
import re
import shutil
import sys
from pathlib import Path
from typing import NoReturn

sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_mux_docs import strip_test_modules

ROOT = Path(__file__).resolve().parent.parent

RUST_ROOTS = ("src", "crates/par-term-emu-core/src", "crates/par-mux/src", "derive/src")
TS_GLOB = "crates/par-mux/integrations/extensions/*.ts"
SH_GLOB = "crates/par-mux/integrations/hooks/*.sh"
PS1_GLOB = "crates/par-mux/integrations/hooks/*.ps1"
PY_GLOB = "python/par_term_emu_core_rust/*.py"
TEST_PATH_RE = re.compile(
    r"(^|/)(tests|benches|fuzz|examples)/|(^|/)tests\.rs$|_tests\.rs$|(^|/)build\.rs$"
)

LIT_RE = re.compile(r'\benv::var(?:_os)?\(\s*"([A-Z][A-Z0-9_]*)"')
IDENT_RE = re.compile(
    r"\b(?:env::var(?:_os)?|env_var_nonempty|agent_dir_from_env_or_home)\(\s*([A-Za-z_]\w*)\s*[,)]"
)
CONST_RE = re.compile(
    r'const\s+([A-Z_][A-Z0-9_]*)\s*:\s*&str\s*=\s*"([A-Z][A-Z0-9_]*)"'
)
CLAP_ENV_RE = re.compile(r'\benv\s*=\s*"([A-Z][A-Z0-9_]*)"')

TS_PATTERNS = (
    re.compile(r"process\.env\.([A-Z][A-Z0-9_]*)"),
    re.compile(r'process\.env\[\s*"([A-Z][A-Z0-9_]*)"'),
    re.compile(r'parseDurationEnv\(\s*"([A-Z][A-Z0-9_]*)"'),
)
SH_PATTERNS = (
    re.compile(r'os\.environ(?:\.get)?[\(\[]\s*"([A-Z][A-Z0-9_]*)"'),
    re.compile(r"\$\{([A-Z][A-Z0-9_]*)"),
)
PS1_PATTERNS = (re.compile(r"\$env:([A-Za-z_]\w*)"),)
PY_PATTERNS = (re.compile(r'os\.environ(?:\.get)?[\(\[]\s*"([A-Z][A-Z0-9_]*)"'),)

# Non-`env::var` reads: each entry must still appear as the string literal
# "<NAME>" in its file (after test stripping) or the entry is stale.
EXTRA_READS: dict[str, str] = {
    "PAR_MUX_CONTROL": "crates/par-mux/src/mux/pane.rs",  # session-env grant (ENH-039), compared by name
}

# Reads that are not this project's knobs, each with a reason. An entry
# nothing reads any more is a stale-allowlist problem.
NOT_OURS: dict[str, str] = {
    "PAR_MUX_HOOK_ACTION": "set by par-mux-claude-session-hook.sh for its own python child",
    "PAR_MUX_HOOK_INPUT_FILE": "set by the codex/grok hook .sh for its own python child",
    "CODEX_THREAD_ID": "codex's own session identity (MUX.md identity-var drop)",
    "GROK_SESSION_ID": "grok's own session identity",
    "OMPCODE": "omp's nested-session marker (MUX.md)",
    "OS": "Windows built-in, PowerShell platform test",
}

# Documented rows read through a std/dirs API instead of `env::var`; checked
# only in the doc-to-code direction.
STD_INDIRECT: dict[str, str] = {
    "HOME": "dirs::home_dir() (mux/persist/restore.rs, skill_installer.rs)",
    "TMPDIR": "std::env::temp_dir() (debug.rs, mux/ipc.rs)",
}

# The integration helpers' own parameters; their call sites are scanned via
# IDENT_RE, the parameters themselves never resolve.
HELPER_PARAMS = {
    ("crates/par-mux/src/mux/integrations/extensions.rs", "name"),
    ("crates/par-mux/src/mux/integrations/extensions.rs", "env_var"),
}


def fail(msg: str) -> NoReturn:
    print(msg)
    raise SystemExit(1)


def rust_files(root: Path) -> list[Path]:
    out: list[Path] = []
    for rel in RUST_ROOTS:
        base = root / rel
        if not base.is_dir():
            fail(f"missing scan root {rel}")
        out.extend(
            p for p in base.rglob("*.rs") if not TEST_PATH_RE.search(p.as_posix())
        )
    return sorted(out)


def read(root: Path, rel: str) -> str:
    path = root / rel
    if not path.is_file():
        fail(f"missing file {rel}")
    return path.read_text(encoding="utf-8")


def glob_one(root: Path, pattern: str) -> list[Path]:
    return sorted(root.glob(pattern))


def collect_rust_reads(root: Path) -> tuple[set[str], set[str]]:
    """(reads, clap streamer vars) from the production Rust trees."""
    reads: set[str] = set()
    clap: set[str] = set()
    for path in rust_files(root):
        rel = path.relative_to(root).as_posix()
        code = strip_test_modules(path.read_text(encoding="utf-8"))
        consts = {name: value for name, value in CONST_RE.findall(code)}
        reads.update(LIT_RE.findall(code))
        for ident in IDENT_RE.findall(code):
            value = consts.get(ident)
            if value is None:
                if (rel, ident) not in HELPER_PARAMS:
                    fail(f"unresolvable env read {ident} in {rel}")
            else:
                reads.add(value)
        clap.update(CLAP_ENV_RE.findall(code))
    for name, rel in EXTRA_READS.items():
        code = strip_test_modules(read(root, rel))
        if f'"{name}"' not in code:
            fail(f"stale EXTRA_READS entry {name}: the literal is gone from {rel}")
        reads.add(name)
    return reads, clap


def collect_script_reads(root: Path) -> set[str]:
    reads: set[str] = set()
    for pattern in TS_PATTERNS:
        for path in glob_one(root, TS_GLOB):
            reads.update(pattern.findall(path.read_text(encoding="utf-8")))
    for pattern in SH_PATTERNS:
        for path in glob_one(root, SH_GLOB):
            reads.update(pattern.findall(path.read_text(encoding="utf-8")))
    for pattern in PS1_PATTERNS:
        for path in glob_one(root, PS1_GLOB):
            reads.update(pattern.findall(path.read_text(encoding="utf-8")))
    for pattern in PY_PATTERNS:
        for path in glob_one(root, PY_GLOB):
            reads.update(pattern.findall(path.read_text(encoding="utf-8")))
    # The integration helpers resolve their own directory env vars through
    # the Rust side; the TS/SH files the hooks read at runtime are scanned
    # above, so an empty result is a fail-closed condition.
    if not reads:
        fail("parsed nothing from the integration/Python script globs")
    return reads


def config_reference_rows(root: Path) -> set[str]:
    doc = read(root, "docs/CONFIG_REFERENCE.md")
    m = re.search(
        r"^## Environment Variables\s*\n(.*?)(?=^## |\Z)", doc, re.MULTILINE | re.DOTALL
    )
    if not m:
        fail(
            "parsed nothing from docs/CONFIG_REFERENCE.md: no Environment Variables section"
        )
    rows = set(re.findall(r"^\| `([A-Z][A-Z0-9_]*\*?)`", m.group(1), re.MULTILINE))
    if not rows:
        fail("parsed nothing from docs/CONFIG_REFERENCE.md: no table rows")
    return rows


def mux_pane_env_rows(root: Path) -> set[str]:
    doc = read(root, "docs/MUX.md")
    m = re.search(
        r"^## Agent Hook Reports\s*\n(.*?)(?=^### )", doc, re.MULTILINE | re.DOTALL
    )
    if not m:
        fail("parsed nothing from docs/MUX.md: no Agent Hook Reports section")
    return set(re.findall(r"^\| `([A-Z][A-Z0-9_]*)`", m.group(1), re.MULTILINE))


def streaming_doc_names(root: Path) -> set[str]:
    doc = read(root, "docs/STREAMING.md")
    return set(re.findall(r"`(PAR_TERM_[A-Z0-9_]*)`", doc))


def expect_failure(root: Path, needle: str, what: str) -> None:
    """Run check(root), require SystemExit, and require `needle` in the report."""
    buf = io.StringIO()
    try:
        with contextlib.redirect_stdout(buf):
            check(root)
    except SystemExit:
        pass
    else:
        fail(f"self-test: {what} was not reported")
    if needle not in buf.getvalue():
        fail(f"self-test: {what} was reported without naming it ({needle})")


def expect_ok(root: Path, what: str) -> None:
    """Run check(root) and require it to pass."""
    buf = io.StringIO()
    try:
        with contextlib.redirect_stdout(buf):
            check(root)
    except SystemExit as e:
        fail(f"self-test: {what} unexpectedly failed: {e} {buf.getvalue()}")


def check(root: Path) -> None:
    reads, clap = collect_rust_reads(root)
    reads |= collect_script_reads(root)
    config_rows = config_reference_rows(root)
    mux_rows = mux_pane_env_rows(root)
    stream_names = streaming_doc_names(root)

    wildcard = {row for row in config_rows if row.endswith("*")}
    if len(wildcard) > 1:
        fail(f"expected one PAR_TERM_* wildcard row, found {sorted(wildcard)}")
    wildcard_row = next(iter(wildcard), None)

    documented = config_rows | mux_rows

    problems: list[str] = []

    # Reads must be documented (or delegated to the streamer doc / allowlists).
    for name in sorted(reads - NOT_OURS.keys() - documented - clap):
        problems.append(
            f"env var {name} read in production code is not in docs/CONFIG_REFERENCE.md's "
            "Environment Variables table"
        )

    # Every documented row must be read (allowing the std/dirs indirect set).
    for row in sorted(config_rows - reads - STD_INDIRECT.keys() - {"PAR_TERM_*"}):
        problems.append(
            f"CONFIG_REFERENCE row {row}: nothing in production code reads it"
        )

    # The wildcard row covers exactly the clap set, and STREAMING.md names them.
    if wildcard_row is not None:
        m = re.search(r"\((\d+) variables\)", read(root, "docs/CONFIG_REFERENCE.md"))
        claimed = int(m.group(1)) if m else -1
        if claimed != len(clap):
            problems.append(
                f"CONFIG_REFERENCE wildcard row claims {claimed} PAR_TERM_* variables; "
                f"the clap set has {len(clap)}"
            )
    for name in sorted(clap - stream_names):
        problems.append(
            f"streamer env var {name} is not documented in docs/STREAMING.md"
        )

    if not clap:
        fail("parsed nothing from the clap env attributes")

    # Stale allowlist entries.
    for name in NOT_OURS:
        if name not in reads:
            fail(f"stale NOT_OURS entry {name}: nothing reads it any more")
    for name in STD_INDIRECT:
        if name not in config_rows:
            fail(
                f"stale STD_INDIRECT entry {name}: not documented in CONFIG_REFERENCE.md"
            )
    for name, rel in EXTRA_READS.items():
        if name in LIT_RE.findall(strip_test_modules(read(root, rel))):
            fail(
                f"EXTRA_READS entry {name} is now a literal read in {rel}; drop the entry"
            )

    if problems:
        for problem in problems:
            print(problem)
        raise SystemExit(1)
    print(
        f"env docs ok: {len(reads)} reads, {len(config_rows)} documented rows, "
        f"{len(clap)} streamer vars"
    )


def copy_inputs(src_root: Path, dst_root: Path) -> None:
    for rel in RUST_ROOTS:
        (dst_root / rel).mkdir(parents=True, exist_ok=True)
        for path in (src_root / rel).rglob("*.rs"):
            dst = dst_root / path.relative_to(src_root)
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, dst)
    for pattern in (TS_GLOB, SH_GLOB, PS1_GLOB, PY_GLOB):
        for path in src_root.glob(pattern):
            dst = dst_root / path.relative_to(src_root)
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, dst)
    for rel in (
        "docs/CONFIG_REFERENCE.md",
        "docs/MUX.md",
        "docs/STREAMING.md",
    ):
        dst = dst_root / rel
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(src_root / rel, dst)


def self_test() -> None:
    import tempfile

    with tempfile.TemporaryDirectory(prefix="enh055-selftest-") as tmp:
        root = Path(tmp)

        copy_inputs(ROOT, root)
        expect_ok(root, "the baseline copy")

        # Drift 1: drop a documented row that is read.
        cfg = root / "docs/CONFIG_REFERENCE.md"
        cfg.write_text(
            cfg.read_text(encoding="utf-8").replace(
                "| `DEBUG_LEVEL` |", "| `DEBUG_LEVEL_X` |", 1
            )
        )
        expect_failure(root, "DEBUG_LEVEL_X", "dropping the DEBUG_LEVEL row")
        cfg.write_text(
            cfg.read_text(encoding="utf-8").replace(
                "| `DEBUG_LEVEL_X` |", "| `DEBUG_LEVEL` |", 1
            )
        )

        # Drift 2: a production read of an undocumented variable.
        mod = root / "crates/par-mux/src/mux/mod.rs"
        probe = '\n#[allow(dead_code)] fn enh055_probe() { let _ = std::env::var("PAR_MUX_FAKE_KNOB"); }\n'
        mod.write_text(mod.read_text(encoding="utf-8") + probe)
        expect_failure(root, "PAR_MUX_FAKE_KNOB", "an undocumented production read")
        mod.write_text(mod.read_text(encoding="utf-8").replace(probe, ""))

        # Drift 3: an unresolvable identifier read fails closed.
        probe2 = "\n#[allow(dead_code)] fn enh055_probe2(x: &str) { let _ = std::env::var(unknown_ident); }\n"
        mod.write_text(mod.read_text(encoding="utf-8") + probe2)
        expect_failure(root, "unresolvable", "an unresolvable env read")
        mod.write_text(mod.read_text(encoding="utf-8").replace(probe2, ""))

        # Drift 4: a fake documented row nothing reads.
        ghost = "| `PAR_MUX_GHOST` | fake | unset | fake row |\n"
        cfg.write_text(
            cfg.read_text(encoding="utf-8").replace(
                "| `DEBUG_LEVEL` |", ghost + "| `DEBUG_LEVEL` |", 1
            )
        )
        expect_failure(root, "PAR_MUX_GHOST", "a ghost CONFIG_REFERENCE row")
        cfg.write_text(cfg.read_text(encoding="utf-8").replace(ghost, "", 1))

        # Drift 5: an extension read of an undocumented variable.
        omp = next(iter(glob_one(root, TS_GLOB)))
        omp.write_text(
            omp.read_text(encoding="utf-8")
            + "\nconst PROBE = process.env.PAR_MUX_FAKE_MS;\n"
        )
        expect_failure(root, "PAR_MUX_FAKE_MS", "an undocumented extension read")
        omp.write_text(
            omp.read_text(encoding="utf-8").replace(
                "\nconst PROBE = process.env.PAR_MUX_FAKE_MS;\n", ""
            )
        )

        # Exclusion holds: a #[cfg(test)] read and a tests/ read are not
        # reported.
        testmod = '\n#[cfg(test)]\nmod enh055 {\n    fn f() { let _ = std::env::var("PAR_MUX_FAKE_KNOB"); }\n}\n'
        mod.write_text(mod.read_text(encoding="utf-8") + testmod)
        tests_dir = root / "crates/par-mux/tests"
        tests_dir.mkdir(exist_ok=True)
        (tests_dir / "enh055.rs").write_text(
            '#[test]\nfn probe() { let _ = std::env::var("PAR_MUX_FAKE_KNOB"); }\n'
        )
        expect_ok(root, "test-only reads stay excluded")

    # Fail closed: an empty CONFIG table.
    with tempfile.TemporaryDirectory(prefix="enh055-selftest2-") as tmp:
        root = Path(tmp)
        copy_inputs(ROOT, root)
        cfg = root / "docs/CONFIG_REFERENCE.md"
        section = re.search(
            r"^## Environment Variables\s*\n.*?(?=^## )",
            cfg.read_text(encoding="utf-8"),
            re.MULTILINE | re.DOTALL,
        )
        assert section is not None
        cfg.write_text(
            cfg.read_text(encoding="utf-8").replace(
                section.group(0), "## Environment Variables\n\nEmpty.\n\n"
            )
        )
        expect_failure(root, "parsed nothing", "an empty CONFIG table")

    print(
        "env docs self-test ok: every injected drift was reported and the exclusions hold"
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="copy the scan inputs into a temp tree and inject one drift per contract",
    )
    args = parser.parse_args()
    if args.self_test:
        self_test()
    else:
        check(ROOT)


if __name__ == "__main__":
    main()
